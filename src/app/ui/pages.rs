//! The non-map pages: the searchable Points list, the position and board
//! Status page, the Beacon and Settings pages, and the desktop
//! manual-position bar.

use std::time::{Duration, SystemTime};

use walkers::Position;

use midair_proto::ble;
use midair_proto::link::{TELEM_FLAG_CFG_LOADED, TELEM_FLAG_GPS_FIX, TELEM_FLAG_SD_OK};
use midair_proto::{lora, radiocfg, session};

use crate::app::{secs_text, BleIntent, MyApp, Page, PointFilter, RadioEdit, RegionSelect};
use crate::ble::ConfigWrite;
use crate::config::{
    DistanceUnits, COMPASS_HZ_MAX, COMPASS_HZ_MIN, TEXT_SCALE_MAX, TEXT_SCALE_MIN,
};
use crate::gps::GpsFix;
use crate::points::{age_text, PointSource, TrackPoint};
use crate::radio::{EditVal, FieldType};

use super::{
    content_page, em, feedback_label, field_width, floating, gap, icon_button, page_margin,
    status_bool, CORNER_MARGIN_FRAC, GAP_BLOCK, GAP_HAIR, GAP_ITEM, GAP_SECTION, GAP_TIGHT,
};

/// How long after connecting the board counts as warming up. The GPS/LoRa
/// rail is off through sleep and through each wake window, and comes up only
/// once a central connects, so the WIO has to boot and the GPS has to make a
/// cold fix before there is anything to report.
const BOARD_WARMUP: Duration = Duration::from_secs(45);

/// Width of the text-size slider, as a fraction of the screen width, and the
/// step it moves in.
///
/// A fraction of the screen rather than of the text, unlike every other input on
/// the page: this is the one control whose own text grows while it is dragged,
/// and a width in text heights would walk out from under the finger setting it.
const TEXT_SCALE_SLIDER_FRAC: f32 = 0.45;
const TEXT_SCALE_STEP: f64 = 0.05;

/// Render the type-specific input for an unlocked radio field, bound to `val`.
/// The kind of widget follows the field's type: a draggable number, a checkbox,
/// a dropdown for an enum, or a text field.
fn radio_input(ui: &mut egui::Ui, key: &str, ty: &FieldType, val: &mut EditVal) {
    match val {
        EditVal::Int(i) => {
            ui.add(egui::DragValue::new(i));
        }
        EditVal::Float(f) => {
            ui.add(egui::DragValue::new(f));
        }
        EditVal::Bool(b) => {
            ui.checkbox(b, "");
        }
        EditVal::Str(s) => {
            if let FieldType::Enum(opts) = ty {
                egui::ComboBox::from_id_salt(("radio_enum", key))
                    .selected_text(s.clone())
                    .show_ui(ui, |ui| {
                        for opt in opts {
                            ui.selectable_value(s, opt.clone(), opt.as_str());
                        }
                    });
            } else {
                // Width in text units, not raw pixels, so it scales with the font.
                let width = em(ui) * 12.0;
                ui.add(egui::TextEdit::singleline(s).desired_width(width));
            }
        }
    }
}

/// A color that may be left to the light/dark theme: a checkbox that turns the
/// override on and off, and a picker beside it, enabled only while it is on.
///
/// `theme` is what the theme is drawing with right now, which is both what the
/// disabled picker shows and what ticking the box starts from - so turning an
/// override on changes nothing until the color is moved, and turning it off
/// leaves nothing behind to be saved.
fn theme_color(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<egui::Color32>,
    theme: egui::Color32,
) {
    ui.horizontal(|ui| {
        let mut on = value.is_some();
        if ui.checkbox(&mut on, label).changed() {
            *value = on.then_some(theme);
        }
        let mut color = value.unwrap_or(theme);
        ui.add_enabled_ui(on, |ui| ui.color_edit_button_srgba(&mut color));
        if on {
            *value = Some(color);
        }
    });
}

/// Parse "lat, lon" or "lat lon" into decimal degrees. `None` unless it is
/// exactly two finite numbers within the valid latitude/longitude range.
fn parse_lat_lon(s: &str) -> Option<(f64, f64)> {
    let mut parts = s
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty());
    let lat: f64 = parts.next()?.parse().ok()?;
    let lon: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None; // trailing junk
    }
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some((lat, lon))
}

impl MyApp {
    /// The points page: a searchable, filterable list of every recorded GPS
    /// point from both sources. Tapping a row shows it on the map.
    pub(crate) fn points_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let top = self.top_inset(ctx);
        let bottom = self.bottom_inset(ctx);
        content_page(ctx, "points", screen, top, |ui| {
            ui.heading("GPS points");
            gap(ui, GAP_BLOCK);

            ui.horizontal_wrapped(|ui| {
                // Most of the row, leaving the Clear button beside it.
                let width = field_width(ui, screen, 0.6);
                ui.add(
                    egui::TextEdit::singleline(&mut self.points_search)
                        .hint_text("search (example 51.47 or central)")
                        .desired_width(width),
                );
                if ui.button("Clear").clicked() {
                    self.points_search.clear();
                }
            });
            gap(ui, GAP_HAIR);
            ui.horizontal_wrapped(|ui| {
                ui.label("Source:");
                ui.selectable_value(&mut self.points_filter, PointFilter::All, "all");
                // The source names come from `PointSource::label`, so the
                // filter and the rows below it always read the same.
                ui.selectable_value(
                    &mut self.points_filter,
                    PointFilter::Phone,
                    PointSource::Phone.label(),
                );
                ui.selectable_value(
                    &mut self.points_filter,
                    PointFilter::Esp,
                    PointSource::Esp.label(),
                );
                // One entry for all remote nodes; a node's own address shows in
                // its rows below rather than as its own filter button.
                ui.selectable_value(&mut self.points_filter, PointFilter::Remote, "nodes");
            });
            gap(ui, GAP_ITEM);

            let query = self.points_search.trim().to_lowercase();
            let remote_points = self.remotes.values().flat_map(|n| n.track.iter());
            let mut rows: Vec<TrackPoint> = self
                .track
                .iter()
                .chain(self.beacon_track.iter())
                .chain(remote_points)
                .filter(|p| self.points_filter.admits(p.source))
                .filter(|p| query.is_empty() || p.matches(&query))
                .copied()
                .collect();
            // Newest first; every track interleaves by record time.
            rows.sort_by(|x, y| y.time.cmp(&x.time));

            let remote_total: usize = self.remotes.values().map(|n| n.track.len()).sum();
            let total = self.track.len() + self.beacon_track.len() + remote_total;
            ui.label(format!("{} of {total} points", rows.len()));
            gap(ui, GAP_HAIR);

            let now = SystemTime::now();
            let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
            // Everything left below the filters, less the page's own bottom
            // margin, and never so short that no row fits.
            let list_height = (screen.bottom() - bottom - ui.cursor().min.y - page_margin(screen))
                .max(em(ui) * 4.0);
            let mut goto: Option<Position> = None;
            egui::ScrollArea::vertical()
                .max_height(list_height)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, rows.len(), |ui, range| {
                    for p in &rows[range] {
                        // The source column is wide enough for the longest
                        // label, so the coordinates stay in line.
                        let text = format!(
                            "{:<8} {}  {:>7}",
                            p.source.label(),
                            p.coord_text(),
                            age_text(now, p.time),
                        );
                        if ui
                            .selectable_label(false, egui::RichText::new(text).monospace())
                            .clicked()
                        {
                            goto = Some(p.pos);
                        }
                    }
                });
            if let Some(pos) = goto {
                self.map_memory.center_at(pos);
                self.page = Page::Map;
            }
        });
    }

    /// Bottom-anchored bar for entering a position by hand when no live GPS
    /// source is wired up (desktop). Accepts "lat, lon" or "lat lon"; a valid
    /// entry feeds the same pipeline a real fix would and recenters the map.
    pub(crate) fn manual_gps_bar(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let bottom = self.bottom_inset(ctx);
        let margin = screen.size().min_elem() * CORNER_MARGIN_FRAC;
        floating(
            ctx,
            "manual_gps",
            egui::Order::Foreground,
            egui::Pos2::new(screen.center().x, screen.bottom() - bottom - margin),
            egui::Align2::CENTER_BOTTOM,
            false,
            |ui| {
                ui.horizontal(|ui| {
                    ui.label("Position:");
                    let width = field_width(ui, screen, 0.5);
                    let field = egui::TextEdit::singleline(&mut self.manual_gps_text)
                        .hint_text("lat, lon")
                        .desired_width(width);
                    let resp = ui.add(field);
                    let entered =
                        resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Set").clicked() || entered {
                        match parse_lat_lon(&self.manual_gps_text) {
                            Some((lat, lon)) => {
                                self.manual_gps_bad = false;
                                self.apply_gps_fix(GpsFix {
                                    lat,
                                    lon,
                                    bearing: None,
                                });
                                self.map_memory.follow_my_position();
                            }
                            None => self.manual_gps_bad = true,
                        }
                    }
                });
                if self.manual_gps_bad {
                    ui.colored_label(
                        self.config.ui.error,
                        "Enter latitude and longitude, example 51.4779, -0.0015",
                    );
                }
            },
        );
    }

    /// The status page: where we are, and board health for the esp32c6-gps
    /// board. The position read-out is the app's own; the BLE link state comes
    /// from the connection itself; the WIO/GPS/LoRa figures come from the
    /// board's telemetry characteristic, and the last line from its log
    /// characteristic.
    pub(crate) fn status_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let top = self.top_inset(ctx);
        content_page(ctx, "status", screen, top, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Status");
                gap(ui, GAP_BLOCK);

                // Where we are, and how far off the beacon is. First because it
                // is the one section that needs no board at all.
                ui.strong("Position");
                match self.current {
                    Some(pos) => {
                        ui.label(
                            egui::RichText::new(format!("{:.5}, {:.5}", pos.y(), pos.x()))
                                .monospace(),
                        );
                        // Names the board it is measured to: the tracked one
                        // when tracking, otherwise the connected board.
                        if let (Some((kind, _)), Some(m)) =
                            (self.distance_target(), self.distance_to_target())
                        {
                            ui.label(format!(
                                "Distance to {}: {}",
                                self.marker_label(kind),
                                self.config.distance.units.format(m)
                            ));
                        }
                    }
                    None => {
                        ui.label("Waiting for a GPS fix...");
                    }
                }

                // The beacon's own position, when it is streaming.
                if let (Some(b), Some(p)) = (self.beacon, self.beacon_packet) {
                    gap(ui, GAP_ITEM);
                    ui.label(
                        egui::RichText::new(format!("Beacon: {:.5}, {:.5}", b.y(), b.x()))
                            .monospace(),
                    );
                    ui.label(format!("Beacon speed: {:.1} m/s", p.speed_mps()));
                    // Satellite count from the packet only when there is no
                    // telemetry to report it below, so it is never on screen
                    // twice from two different sources.
                    if self.telemetry.is_none() {
                        ui.label(format!("Beacon satellites: {}", p.sats));
                    }
                }

                // Remote nodes heard over LoRa and relayed by the connected
                // board, each with its last position and the signal it came in
                // at. A node with no position is listed too, with why it has
                // none: a node searching for the sky is a node that is up, and
                // leaving it out makes it indistinguishable from one that is
                // out of range or dead. Collected first so the config lookup
                // below is free of the borrow on `remotes`.
                let heard: Vec<(u8, String, i16)> = self
                    .remotes
                    .iter()
                    .filter(|(_, n)| n.pos.is_some() || n.heard.is_some())
                    .map(|(&addr, n)| (addr, n.state_text(), n.rssi))
                    .collect();
                if !heard.is_empty() {
                    gap(ui, GAP_SECTION);
                    ui.strong("Remote nodes");
                    for (addr, state, rssi) in heard {
                        ui.label(
                            egui::RichText::new(format!(
                                "{}: {state}  (rssi {rssi})",
                                self.config.lora.label_of(addr),
                            ))
                            .monospace(),
                        );
                    }
                }

                // ESP32-C6 / BLE link.
                gap(ui, GAP_SECTION);
                ui.strong("ESP32-C6 (BLE)");
                status_bool(ui, self.config.ui, "Link", self.ble_connected);
                ui.label(self.ble_intent_text());
                ui.label(egui::RichText::new(format!("BLE: {}", self.ble_status)).weak());
                if let Some(secs) = self
                    .board_settings
                    .map(|s| s.sleep_interval_s)
                    .filter(|_| self.ble_connected)
                {
                    ui.label(
                        egui::RichText::new(match secs {
                            0 => "Board sleep: disabled.".to_string(),
                            secs => format!("Board sleep: every {} once disconnected.", secs_text(secs)),
                        })
                        .weak(),
                    );
                }
                // The elapsed counts (connecting-for, board-silent-for) move
                // by themselves; a one-second tick keeps them honest.
                if self.ble_intent != BleIntent::Idle
                    && (!self.ble_connected || self.board_silence().is_some())
                {
                    ui.ctx().request_repaint_after(Duration::from_secs(1));
                }

                // The rail powering the WIO-E5 and the GPS comes up only once
                // a central connects, so an empty read-out just after
                // connecting is the board waking, not a fault.
                let warming = self
                    .connected_at
                    .is_some_and(|t| t.elapsed() < BOARD_WARMUP);
                let rail_off = self.board_settings.is_some_and(|s| !s.pwr_en);
                if rail_off {
                    gap(ui, GAP_ITEM);
                    ui.label(
                        "The GPS/LoRa power rail is switched off, so the WIO-E5 and the GPS \
                         are unpowered and report nothing. Turn it on under Beacon.",
                    );
                } else if warming {
                    gap(ui, GAP_ITEM);
                    ui.label(
                        "Warming up: the rail powers on at connect, so the WIO-E5 is still \
                         booting and the GPS is working on a cold fix.",
                    );
                }

                let Some(t) = self.telemetry else {
                    gap(ui, GAP_SECTION);
                    if !warming && !rail_off {
                        ui.label(
                            "No board telemetry yet.\n\
                             Waiting for the esp32c6-gps board (an esp32c3 \
                             beacon does not report it).",
                        );
                    }
                    if let Some(line) = &self.board_log {
                        gap(ui, GAP_SECTION);
                        ui.strong("Last message");
                        ui.label(egui::RichText::new(line).monospace());
                    }
                    return;
                };

                // GPS (via the WIO's MAX-M10).
                gap(ui, GAP_SECTION);
                ui.strong("GPS");
                status_bool(ui, self.config.ui, "Fix", t.flags & TELEM_FLAG_GPS_FIX != 0);
                ui.label(format!("Satellites: {}", t.sats));

                // LoRa mesh link (WIO-E5 radio).
                gap(ui, GAP_SECTION);
                ui.strong("LoRa");
                let last_rx = match t.secs_since_rx {
                    0xFFFF => "never".to_string(),
                    s => format!("{s} s ago"),
                };
                ui.label(format!("Last RX: {last_rx}"));
                if t.last_rssi != 0 {
                    ui.label(format!(
                        "RSSI: {} dBm   SNR: {:.2} dB",
                        t.last_rssi,
                        t.last_snr_cb as f32 / 100.0
                    ));
                }
                ui.label(format!("RX: {}   TX: {}", t.rx_count, t.tx_count));

                // WIO-E5 housekeeping.
                gap(ui, GAP_SECTION);
                ui.strong("WIO-E5");
                status_bool(
                    ui,
                    self.config.ui,
                    "SD logging",
                    t.flags & TELEM_FLAG_SD_OK != 0,
                );
                status_bool(
                    ui,
                    self.config.ui,
                    "Radio config",
                    t.flags & TELEM_FLAG_CFG_LOADED != 0,
                );

                if let Some(line) = &self.board_log {
                    gap(ui, GAP_SECTION);
                    ui.strong("Last message");
                    ui.label(egui::RichText::new(line).monospace());
                }
            });
        });
    }

    /// The settings page: the app's own TOML settings - what it draws, what it
    /// records, and the config file itself. The beacon and the board's own
    /// settings are a separate page ([`MyApp::beacon_page`]); the split is by
    /// who owns the setting, since only the ones here are the app's to keep.
    ///
    /// Every widget here is bound straight to the live [`crate::config::AppConfig`],
    /// so a change takes effect on the map immediately; Save is what makes it
    /// outlast the session.
    pub(crate) fn settings_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let top = self.top_inset(ctx);
        content_page(ctx, "settings", screen, top, |ui| {
            // The field column is half the screen, leaving room for its label
            // and the buttons beside it.
            let path_width = field_width(ui, screen, 0.5);
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("App settings");
                gap(ui, GAP_TIGHT);
                ui.label(
                    egui::RichText::new(
                        "What the app draws and records, kept in its own TOML file.",
                    )
                    .weak(),
                );
                gap(ui, GAP_BLOCK);

                ui.label("Config file (TOML):");
                ui.horizontal_wrapped(|ui| {
                    let field = egui::TextEdit::singleline(&mut self.config_path)
                        .hint_text("/path/to/config.toml")
                        .desired_width(path_width);
                    let resp = ui.add(field);
                    let entered = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Load").clicked() || entered {
                        self.load_config();
                    }
                });

                gap(ui, GAP_ITEM);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button("Save")
                        .on_hover_text(
                            "Write these settings to the file above, generating it if it is not there",
                        )
                        .clicked()
                    {
                        self.save_config();
                    }
                    if ui
                        .button("Reset to defaults")
                        .on_hover_text("Only in the app until you save")
                        .clicked()
                    {
                        self.reset_config();
                    }
                });

                gap(ui, GAP_ITEM);
                feedback_label(ui, self.config.ui, &self.config_feedback);

                gap(ui, GAP_SECTION);
                ui.strong("Text size");
                gap(ui, GAP_TIGHT);
                ui.label(
                    egui::RichText::new(
                        "Scales the text on every page. The gaps and the input widths are \
                         measured in text heights, so they grow with it; the map's icons and \
                         overlays keep their own sizes.",
                    )
                    .weak(),
                );
                gap(ui, GAP_TIGHT);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().slider_width = screen.width() * TEXT_SCALE_SLIDER_FRAC;
                    ui.add(
                        egui::Slider::new(
                            &mut self.config.ui.text_scale,
                            TEXT_SCALE_MIN..=TEXT_SCALE_MAX,
                        )
                        .step_by(TEXT_SCALE_STEP)
                        .fixed_decimals(2)
                        .suffix("x"),
                    );
                    // Compared with a tolerance of half a step: the slider lands
                    // on multiples of the step, which need not be exactly 1.0.
                    let scaled =
                        (self.config.ui.text_scale - 1.0).abs() > TEXT_SCALE_STEP as f32 / 2.0;
                    if ui
                        .add_enabled(scaled, egui::Button::new("Reset"))
                        .on_hover_text("Back to the default text size")
                        .clicked()
                    {
                        self.config.ui.text_scale = 1.0;
                    }
                });

                gap(ui, GAP_SECTION);
                ui.strong("Marker colors");
                egui::Grid::new("cfg_colors").num_columns(2).show(ui, |ui| {
                    ui.label("track");
                    ui.color_edit_button_srgba(&mut self.config.colors.track);
                    ui.end_row();
                    ui.label("beacon");
                    ui.color_edit_button_srgba(&mut self.config.colors.fixed);
                    ui.end_row();
                    ui.label("marker outline");
                    ui.color_edit_button_srgba(&mut self.config.colors.outline);
                    ui.end_row();
                });

                gap(ui, GAP_SECTION);
                ui.strong("Page colors");
                gap(ui, GAP_TIGHT);
                ui.label(
                    egui::RichText::new(
                        "The few places off the map that carry meaning by color; the rest \
                         follows the theme.",
                    )
                    .weak(),
                );
                egui::Grid::new("cfg_ui_colors").num_columns(2).show(ui, |ui| {
                    ui.label("ok");
                    ui.color_edit_button_srgba(&mut self.config.ui.ok);
                    ui.end_row();
                    ui.label("error");
                    ui.color_edit_button_srgba(&mut self.config.ui.error);
                    ui.end_row();
                    ui.label("no-target pulse");
                    ui.color_edit_button_srgba(&mut self.config.ui.pulse);
                    ui.end_row();
                });

                gap(ui, GAP_ITEM);
                // The surfaces and the text everything else is drawn with. Read
                // out of the live visuals, so an unticked row shows what the
                // theme is actually using and ticking it starts from there.
                let bg = ui.visuals().panel_fill;
                let button = ui.visuals().widgets.inactive.weak_bg_fill;
                let text = ui.visuals().text_color();
                theme_color(ui, "Set the background", &mut self.config.ui.background, bg);
                theme_color(ui, "Set the buttons", &mut self.config.ui.button, button);
                theme_color(ui, "Set the text", &mut self.config.ui.text, text);
                gap(ui, GAP_HAIR);
                ui.label(
                    egui::RichText::new(
                        "Unticked follows the light/dark theme, which is what keeps these \
                         three readable against each other. Setting one is taking that on \
                         yourself.",
                    )
                    .weak(),
                );

                gap(ui, GAP_SECTION);
                ui.strong("Overlay sizes (points)");
                let s = &mut self.config.sizes;
                egui::Grid::new("cfg_sizes").num_columns(2).show(ui, |ui| {
                    for (label, value) in [
                        ("marker", &mut s.marker),
                        ("beacon", &mut s.beacon),
                        ("track", &mut s.track),
                        ("distance line", &mut s.distance_line),
                        ("distance text", &mut s.distance_text),
                    ] {
                        ui.label(label);
                        // The loader rejects a size of 0 or less, so the drag stops
                        // short of one rather than writing a file that won't load.
                        ui.add(egui::DragValue::new(value).speed(0.1).range(0.5..=64.0));
                        ui.end_row();
                    }
                });

                gap(ui, GAP_SECTION);
                ui.strong("Map overlays");
                // "Central" is what the Points page calls this device's own
                // fixes, so the two pages name the same track the same way.
                ui.checkbox(&mut self.config.track.show_path, "Show central path on map")
                    .on_hover_text(
                        "This device's own track. Hiding a path never stops it being recorded",
                    );
                ui.checkbox(&mut self.config.ble.show_path, "Show beacon path on map");
                ui.checkbox(
                    &mut self.config.lora.show_path,
                    "Show remote node paths on map",
                )
                .on_hover_text("The LoRa nodes relayed by the connected board, one color each");
                gap(ui, GAP_HAIR);
                ui.label(
                    egui::RichText::new(
                        "The map's path button hides them all at once without changing these; \
                         the line to the beacon and its distance stay drawn either way.",
                    )
                    .weak(),
                );
                gap(ui, GAP_HAIR);
                ui.checkbox(
                    &mut self.config.distance.show,
                    "Show distance on the line to the beacon",
                );
                ui.checkbox(
                    &mut self.config.distance.dotted,
                    "Draw distance line dotted rather than solid",
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label("Units:");
                    ui.selectable_value(
                        &mut self.config.distance.units,
                        DistanceUnits::Metric,
                        "km/m",
                    );
                    ui.selectable_value(
                        &mut self.config.distance.units,
                        DistanceUnits::Imperial,
                        "mi/ft",
                    );
                });

                gap(ui, GAP_SECTION);
                ui.strong("Compass");
                ui.label(
                    egui::RichText::new(
                        "Heading-up always runs the compass at full rate. These are for the \
                         other modes, where it only points the arrow on your marker.",
                    )
                    .weak(),
                );
                ui.checkbox(
                    &mut self.config.compass.marker_arrow,
                    "Point the marker arrow with the compass",
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label("Compass rate for the arrow (Hz):");
                    ui.add_enabled(
                        self.config.compass.marker_arrow,
                        egui::DragValue::new(&mut self.config.compass.arrow_hz)
                            .speed(0.1)
                            .range(COMPASS_HZ_MIN..=COMPASS_HZ_MAX),
                    )
                    .on_hover_text(
                        "Lower is cheaper: the sensor is fused from the accelerometer, \
                         gyroscope and magnetometer, so it keeps all three awake",
                    );
                });

                gap(ui, GAP_SECTION);
                ui.strong("Track recording");
                // Wrapped, not plain horizontal: the label is long enough to
                // push the drag off a phone-width screen otherwise.
                ui.horizontal_wrapped(|ui| {
                    ui.label("Minimum move between points (m):");
                    ui.add(
                        egui::DragValue::new(&mut self.config.track.min_distance)
                            .speed(0.1)
                            .range(0.0..=1000.0),
                    );
                });
                gap(ui, GAP_ITEM);
                // The map bar's old Clear button is a path toggle now, and
                // discarding the recorded points is not something to leave a
                // finger's width from the buttons used while moving.
                let remote_points: usize = self.remotes.values().map(|n| n.track.len()).sum();
                let points = self.track.len() + self.beacon_track.len() + remote_points;
                if ui
                    .add_enabled(points > 0, egui::Button::new("Discard recorded points"))
                    .on_hover_text("Drops every track: yours, the beacon's and the nodes'. Not undoable")
                    .clicked()
                {
                    self.track.clear();
                    self.beacon_track.clear();
                    for node in self.remotes.values_mut() {
                        node.track.clear();
                    }
                }
                ui.label(egui::RichText::new(format!("{points} points recorded")).weak());

                // Offline maps: start a region download. Only when tiles are cached
                // to disk; jumps to the map and begins the box selection there.
                if self.cache_dir.is_some() {
                    gap(ui, GAP_SECTION);
                    ui.separator();
                    gap(ui, GAP_ITEM);
                    ui.strong("Offline maps");
                    gap(ui, GAP_ITEM);
                    let downloading = self.download.is_some();
                    if ui
                        .add_enabled(!downloading, egui::Button::new("Download region"))
                        .on_hover_text("Pick a box on the map to cache for offline use")
                        .clicked()
                    {
                        self.page = Page::Map;
                        self.select = RegionSelect::Picking {
                            start: None,
                            current: None,
                        };
                    }
                    if downloading {
                        ui.label("A download is already in progress.");
                    }
                }
            });
        });
    }

    /// The beacon page: the BLE link to the board, the app-side settings that
    /// decide how it connects, and the board's own power and sleep settings.
    ///
    /// Split from [`MyApp::settings_page`] by who owns each setting. The two
    /// groups here read alike but are not: the connection settings are the
    /// app's, saved to its TOML with the button beside them, while everything
    /// under "Board power and sleep" lives in the board's flash and is only
    /// ever reported by the board (see `board_power_ui`).
    pub(crate) fn beacon_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let top = self.top_inset(ctx);
        content_page(ctx, "beacon", screen, top, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Beacon");
                gap(ui, GAP_TIGHT);
                ui.label(
                    egui::RichText::new(
                        "The BLE link to a GPS beacon. One board at a time.",
                    )
                    .weak(),
                );
                gap(ui, GAP_BLOCK);

                // Which board first, then what to do about the link to it.
                ui.strong("Device");
                gap(ui, GAP_TIGHT);
                self.device_picker_ui(ui);

                gap(ui, GAP_SECTION);
                ui.separator();
                gap(ui, GAP_ITEM);
                ui.strong("Link");
                gap(ui, GAP_TIGHT);
                self.ble_link_ui(ui);

                gap(ui, GAP_SECTION);
                ui.separator();
                gap(ui, GAP_ITEM);
                ui.strong("Connection");
                gap(ui, GAP_TIGHT);
                ui.checkbox(
                    &mut self.config.ble.enabled,
                    "Connect automatically at startup",
                )
                .on_hover_text("Only read when the app launches; use the buttons above now");
                gap(ui, GAP_TIGHT);
                // The board names, the selected board and the checkbox above are
                // the app's settings, not the board's, so they need the same
                // Save the Settings page has rather than a trip back to it.
                // Same file, same feedback line.
                if ui
                    .button("Save to config file")
                    .on_hover_text(
                        "Write the board names and these settings to the app's config file, \
                         set on the Settings page",
                    )
                    .clicked()
                {
                    self.save_config();
                }
                feedback_label(ui, self.config.ui, &self.config_feedback);

                gap(ui, GAP_ITEM);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Notify interval (ms):");
                    let width = em(ui) * 5.0;
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ble_interval_text)
                            .desired_width(width),
                    );
                    let ready = self.ble_connected && !self.ble_ack_pending;
                    let apply = ui.add_enabled(ready, egui::Button::new("Apply"));
                    if apply.clicked() {
                        match self.ble_interval_text.trim().parse::<u32>() {
                            Ok(ms) => self.send_config(ConfigWrite::Interval(ms)),
                            Err(_) => {
                                self.ble_ack =
                                    Some(Err("Enter a whole number of milliseconds.".to_string()));
                            }
                        }
                    }
                });
                if self.ble_ack.is_none() && self.ble_ack_pending {
                    ui.label("waiting for device ack...");
                } else {
                    feedback_label(ui, self.config.ui, &self.ble_ack);
                }

                gap(ui, GAP_SECTION);
                ui.separator();
                gap(ui, GAP_ITEM);
                ui.strong("Board power and sleep");
                gap(ui, GAP_TIGHT);
                ui.label(
                    egui::RichText::new(
                        "ESP32-C6 settings. The board keeps these in flash, so they outlast a power cycle.",
                    )
                    .weak(),
                );
                gap(ui, GAP_ITEM);
                self.board_power_ui(ui);
            });
        });
    }

    /// Pick which board to talk to. Only one is connected at a time, so this is
    /// a single-choice list rather than a set of toggles.
    ///
    /// Every board runs the same firmware and advertises the same name, so a
    /// raw scan is a list of identical entries told apart only by MAC. The
    /// nickname box on each row is what makes the list readable, and it is
    /// stored in the app's config file rather than on the board.
    fn device_picker_ui(&mut self, ui: &mut egui::Ui) {
        let scanning = self.ble_intent == BleIntent::Scanning;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(if scanning {
                    "Stop scanning"
                } else {
                    "Scan for boards"
                })
                .on_hover_text(if scanning {
                    "Stop looking and leave the list as it stands"
                } else {
                    "Look for boards nearby. This drops the current link, since only one board is connected at a time."
                })
                .clicked()
            {
                self.set_ble_intent(if scanning {
                    BleIntent::Idle
                } else {
                    BleIntent::Scanning
                });
            }
            if scanning {
                ui.spinner();
            }
        });
        gap(ui, GAP_TIGHT);

        let rows = self.device_rows();
        // Named boards are remembered, so an empty list means nothing has ever
        // been named and nothing is on the air right now.
        if rows.is_empty() {
            ui.label(
                egui::RichText::new(if scanning {
                    "No boards yet. A sleeping board only answers during its advertising window."
                } else {
                    "No boards known yet. Scan to find one, then name it so you can tell it apart later."
                })
                .weak(),
            );
        }

        let any_selected = self.config.ble.mac.is_none();
        if ui
            .radio(any_selected, "Any board")
            .on_hover_text("Connect to the first board that answers, whichever it is")
            .clicked()
            && !any_selected
        {
            self.select_device(None);
        }

        // Sized off the text, so the box holds roughly the same number of
        // characters whatever the font scale is.
        let name_width = em(ui) * 7.0;
        for row in rows {
            ui.horizontal_wrapped(|ui| {
                if ui.radio(row.selected, "").clicked() && !row.selected {
                    self.select_device(Some(&row.mac));
                }
                // Committing on blur rather than per keystroke: an empty name
                // forgets the board, and that must not happen mid-edit just
                // because the box was cleared before retyping.
                let field = egui::TextEdit::singleline(self.name_edit(&row.mac))
                    .hint_text("name this board")
                    .desired_width(name_width);
                if ui.add(field).lost_focus() {
                    self.commit_name(&row.mac);
                }
                ui.label(egui::RichText::new(row.mac.as_str()).weak());
                match row.rssi {
                    // Only a running scan measures this, so its absence during
                    // a scan is the useful signal: that board is not answering.
                    Some(rssi) => {
                        let text = egui::RichText::new(format!("{rssi} dBm"));
                        ui.label(text.color(self.config.ui.ok));
                    }
                    None if scanning => {
                        ui.label(egui::RichText::new("not answering").weak());
                    }
                    None => {}
                }
            });
        }

        gap(ui, GAP_TIGHT);
        ui.label(
            egui::RichText::new(
                "Names are the app's own and are saved with the rest of its settings. \
                 Clearing a name forgets the board.",
            )
            .weak(),
        );
        // The signal readings move by themselves while a scan runs.
        if scanning {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
    }

    /// The link controls: one button per thing you can ask for, plus what the
    /// app and the worker each say is happening.
    ///
    /// Three buttons rather than a toggle because the three requests are
    /// genuinely different - one of them (Disconnect) is the only way to let
    /// the board sleep, and that was impossible to express when connecting was
    /// a checkbox the app re-applied on its own.
    ///
    /// Every one of them takes effect on the press. Connect stays live while
    /// connected because pressing it then is a real request - start over from
    /// a scan - and it is the way out of a link that is up but not working.
    fn ble_link_ui(&mut self, ui: &mut egui::Ui) {
        let connected = self.ble_connected;
        let idle = self.ble_intent == BleIntent::Idle;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(if connected { "Reconnect" } else { "Connect" })
                .on_hover_text(if connected {
                    "Drop this link and start over from a scan, on the selected board"
                } else {
                    "Go straight to the selected board, or scan when it is set to any"
                })
                .clicked()
            {
                self.set_ble_intent(BleIntent::Connect);
            }
            if ui
                .button("Connect to sleeping")
                .on_hover_text(
                    "Scan without stopping. A sleeping board advertises for only a window per wake, a plain connect can miss.",
                )
                .clicked()
            {
                self.set_ble_intent(BleIntent::ConnectSleeping);
            }
            if ui
                .add_enabled(!idle, egui::Button::new("Disconnect"))
                .on_hover_text(
                    "Drop the link now, forget what the board reported, and stop trying so it can sleep",
                )
                .clicked()
            {
                self.set_ble_intent(BleIntent::Idle);
            }
        });

        gap(ui, GAP_TIGHT);
        // Which board these buttons act on. With several boards around, the
        // link state means little without knowing whose it is.
        ui.label(format!("Board: {}", self.selected_device_label()));
        if connected {
            // A link that has gone quiet is not shown in the all-well color:
            // the text is doubting the connection, so the color must not vouch
            // for it. Its elapsed count also needs the one-second tick.
            if self.board_silence().is_some() {
                ui.label(self.ble_intent_text());
                ui.ctx().request_repaint_after(Duration::from_secs(1));
            } else {
                ui.colored_label(self.config.ui.ok, self.ble_intent_text());
            }
        } else if idle {
            ui.label(egui::RichText::new(self.ble_intent_text()).weak());
        } else {
            ui.label(self.ble_intent_text());
        }
        // The worker's own commentary: scanning, connecting, why it retried.
        // Distinct from the line above, which is what was *asked* for.
        ui.label(egui::RichText::new(format!("BLE: {}", self.ble_status)).weak());

        // The single most useful thing to know while debugging sleep: a
        // connected board never sleeps, so a sleep interval that "does
        // nothing" is usually just the app holding the link open.
        if connected {
            if let Some(s) = self.board_settings {
                gap(ui, GAP_HAIR);
                ui.label(
                    egui::RichText::new(match s.sleep_interval_s {
                        0 => "Sleep is disabled on the board, so it stays awake after you \
                              disconnect too."
                            .to_string(),
                        secs => format!(
                            "On disconnect it will sleep, waking every {} to advertise for {}.",
                            secs_text(secs),
                            secs_text(s.adv_window_s)
                        ),
                    })
                    .weak(),
                );
            }
        }
        if !connected && !idle {
            gap(ui, GAP_HAIR);
            ui.label(
                egui::RichText::new(
                    "During sleep intervals the board is only reachable on its advertising window.",
                )
                .weak(),
            );
            // The elapsed count is the only thing here that moves by itself; a
            // one-second tick keeps it honest without pinning the frame rate.
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
    }

    /// The board's power rail, sleep switches and the wake-check interval.
    /// Every control reads the board's own settings blob rather than a local
    /// copy: the board is the authority, and it changes these by itself
    /// (clamping an interval). A control therefore only moves once the board
    /// reports that it moved.
    fn board_power_ui(&mut self, ui: &mut egui::Ui) {
        if !self.ble_connected {
            ui.label("Connect to the board to see and change these.");
            return;
        }
        if self.settings_unsupported {
            ui.colored_label(
                self.config.ui.error,
                "This board's firmware is newer than the app.",
            );
            ui.label(
                "Its settings use a layout this build cannot decode. Update the app to change them.",
            );
            return;
        }
        let Some(s) = self.board_settings else {
            ui.label("Reading the board's settings...");
            return;
        };

        // One write at a time: while an ack is outstanding the board has not
        // yet said what it applied, and these controls show only what it has.
        let busy = self.ble_ack_pending;
        ui.add_enabled_ui(!busy, |ui| {
            let mut pwr = s.pwr_en;
            if ui
                .checkbox(&mut pwr, "GPS/LoRa power rail")
                .on_hover_text("The LDO feeding both the WIO-E5 and the GPS")
                .changed()
            {
                self.send_config(ConfigWrite::Flag {
                    id: ble::CFG_PWR_EN,
                    on: pwr,
                });
            }
            let mut wio = s.wio_sleep;
            if ui
                .checkbox(&mut wio, "WIO-E5 asleep")
                .on_hover_text("Soft sleep over the UART link, radio and GPS logging stop")
                .changed()
            {
                self.send_config(ConfigWrite::Flag {
                    id: ble::CFG_WIO_SLEEP,
                    on: wio,
                });
            }
            let mut gps = s.gps_sleep;
            if ui
                .checkbox(&mut gps, "GPS in backup mode")
                .on_hover_text("The next fix after waking is a cold one")
                .changed()
            {
                self.send_config(ConfigWrite::Flag {
                    id: ble::CFG_GPS_SLEEP,
                    on: gps,
                });
            }
        });

        gap(ui, GAP_BLOCK);
        ui.strong("Wake check");
        ui.label(
            egui::RichText::new(format!(
                "When set, board deep-sleeps when nothing is connected. Wakes every interval to advertise for window. The GPS/LoRa stay off. the interval survives a connect. Clamped to {} - {}.",
                secs_text(ble::ESP_SLEEP_MIN_S),
                secs_text(ble::ESP_SLEEP_MAX_S),
            ))
            .weak(),
        );
        let width = em(ui) * 5.0;
        ui.horizontal_wrapped(|ui| {
            ui.label("Every (s):");
            ui.add(egui::TextEdit::singleline(&mut self.sleep_interval_text).desired_width(width));
            if ui.add_enabled(!busy, egui::Button::new("Apply")).clicked() {
                match self.sleep_interval_text.trim().parse::<u32>() {
                    Ok(secs) => self.send_config(ConfigWrite::Seconds {
                        id: ble::CFG_ESP_SLEEP_S,
                        secs,
                    }),
                    Err(_) => {
                        self.ble_ack = Some(Err("Enter a whole number of seconds.".to_string()));
                    }
                }
            }
            let can_disable = !busy && s.sleep_interval_s > 0;
            if ui
                .add_enabled(can_disable, egui::Button::new("Disable"))
                .on_hover_text("Stop the board sleeping at all")
                .clicked()
            {
                self.send_config(ConfigWrite::Seconds {
                    id: ble::CFG_ESP_SLEEP_S,
                    secs: 0,
                });
            }
        });
        ui.label(match s.sleep_interval_s {
            0 => "Board: sleep disabled.".to_string(),
            secs => format!("Board: waking every {}.", secs_text(secs)),
        });

        gap(ui, GAP_BLOCK);
        ui.strong("Advertising window");
        ui.label(
            egui::RichText::new(format!(
                "How long each wake advertises before going back to sleep. Clamped to {} - {}.",
                secs_text(ble::ESP_ADV_MIN_S),
                secs_text(ble::ESP_ADV_MAX_S),
            ))
            .weak(),
        );
        // Two firmware behaviors that otherwise read as the board ignoring the
        // window, and the shorter the window the more they stand out: the
        // budget for a wake is taken when that wake starts, and a disconnect
        // replaces what is left of it with a fixed linger so the app can come
        // straight back.
        ui.label(
            egui::RichText::new(format!(
                "A new window takes effect at the next wake, not the current one, and the \
                 stretch right after you disconnect is always {} however short the window is.",
                secs_text(session::LINGER_S as u32),
            ))
            .weak(),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Window (s):");
            ui.add(egui::TextEdit::singleline(&mut self.adv_window_text).desired_width(width));
            if ui.add_enabled(!busy, egui::Button::new("Apply")).clicked() {
                match self.adv_window_text.trim().parse::<u32>() {
                    Ok(secs) => self.send_config(ConfigWrite::Seconds {
                        id: ble::CFG_ESP_ADV_WINDOW_S,
                        secs,
                    }),
                    Err(_) => {
                        self.ble_ack = Some(Err("Enter a whole number of seconds.".to_string()));
                    }
                }
            }
        });
        // No Disable here, unlike the wake check: a zero-length window would
        // leave a sleeping board unreachable by anything short of a physical
        // reset, so the board clamps 0 up to the floor rather than storing it.
        ui.label(format!(
            "Board: advertising {} per wake.",
            secs_text(s.adv_window_s)
        ));
    }


    /// The radio page: load the WIO-E5 RADIO.TOML, edit each setting with a
    /// type-specific input behind a per-field edit lock, and save it back -
    /// keeping the file's comments and a timestamped backup of the previous
    /// version.
    pub(crate) fn radio_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let top = self.top_inset(ctx);
        content_page(ctx, "radio", screen, top, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Radio config");
                gap(ui, GAP_TIGHT);
                ui.label(egui::RichText::new("WIO-E5 RADIO.TOML for the esp32c6-gps board.").weak());
                gap(ui, GAP_BLOCK);

                ui.label("File:");
                ui.horizontal_wrapped(|ui| {
                    let width = field_width(ui, screen, 0.5);
                    let field = egui::TextEdit::singleline(&mut self.radio_path)
                        .hint_text("/path/to/RADIO.toml")
                        .desired_width(width);
                    let resp = ui.add(field);
                    let entered =
                        resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Load").clicked() || entered {
                        self.load_radio();
                    }
                    let dirty = self.radio.as_ref().is_some_and(|r| r.dirty);
                    let save = egui::Button::new(if dirty { "Save *" } else { "Save" });
                    if ui.add_enabled(self.radio.is_some(), save).clicked() {
                        self.save_radio();
                    }
                    // Push the editor's config to the board over BLE. Behind
                    // a confirm popup: it replaces the board's whole config.
                    let send = egui::Button::new(if self.radio_push_pending {
                        "Sending..."
                    } else {
                        "Send to board"
                    });
                    let can_send =
                        self.radio.is_some() && self.ble_connected && !self.radio_push_pending;
                    let why = if self.radio.is_none() {
                        "Load or generate a config first"
                    } else if !self.ble_connected {
                        "Connect to the board first (Beacon page)"
                    } else {
                        "Waiting for the board to answer"
                    };
                    if ui
                        .add_enabled(can_send, send)
                        .on_hover_text(
                            "Send this config to the connected board. The WIO-E5 applies it \
                             immediately and stores it on the SD card and in flash.",
                        )
                        .on_disabled_hover_text(why)
                        .clicked()
                    {
                        self.radio_push_confirm = true;
                    }

                    // Fill the editor with the connected board's own settings,
                    // the read-back counterpart to Send. Enabled once the board
                    // has reported a config it can decode.
                    let can_fetch = self.board_radio_config.is_some();
                    let fetch_why = if self.radio_config_unsupported {
                        "The board's config format is newer than this app can read"
                    } else {
                        "Connect on the Beacon page; the board's settings load once it reports \
                         them (the GPS/LoRa rail must be on)"
                    };
                    if ui
                        .add_enabled(can_fetch, egui::Button::new("Load from board"))
                        .on_hover_text(
                            "Fill the editor with the settings the connected board is currently \
                             running, ready to edit, save or send back.",
                        )
                        .on_disabled_hover_text(fetch_why)
                        .clicked()
                    {
                        self.load_radio_from_board();
                    }
                });
                gap(ui, GAP_TIGHT);
                feedback_label(ui, self.config.ui, &self.radio_feedback);

                if self.radio.is_some() {
                    gap(ui, GAP_ITEM);
                    self.radio_fields_ui(ui);
                    self.radio_estimate_ui(ui);
                    self.radio_backups_ui(ui);
                } else {
                    gap(ui, GAP_BLOCK);
                    ui.label(
                        "Load a RADIO.TOML to view and edit the radio, mesh, beacon and GPS \
                         settings.",
                    );
                    gap(ui, GAP_ITEM);
                    // With no file to load (a fresh SD card), start from the
                    // firmware defaults instead. It fills the editor only; Save
                    // is what writes the file.
                    if ui
                        .button("Generate default config")
                        .on_hover_text(
                            "Fill the editor with the firmware defaults, ready to edit and \
                             save to the file above",
                        )
                        .clicked()
                    {
                        self.default_radio();
                    }
                }
            });
        });

        // The edit-confirm popup floats above the page; a nested Area inside the
        // page's own Area misbehaves, so it is drawn here at the top level.
        self.radio_confirm_popup(ctx, screen);
        self.radio_push_popup(ctx, screen);
    }

    /// The editable settings, grouped by their `[section]`. Each row is a
    /// read-only value with an edit lock, or - while unlocked - the typed input.
    fn radio_fields_ui(&mut self, ui: &mut egui::Ui) {
        let n = match &self.radio {
            Some(r) => r.fields.len(),
            None => return,
        };
        // A sentinel no real section equals, so the first field emits a heading.
        let mut section_shown = String::from("\u{0}");
        for i in 0..n {
            let (section, key, ty, desc) = {
                let f = &self.radio.as_ref().unwrap().fields[i];
                (
                    f.section.clone(),
                    f.key.clone(),
                    f.ty.clone(),
                    f.description.clone(),
                )
            };
            if section != section_shown {
                gap(ui, GAP_BLOCK);
                ui.strong(if section.is_empty() {
                    "general"
                } else {
                    section.as_str()
                });
                ui.separator();
                section_shown = section.clone();
            }
            self.radio_field_row(ui, &section, &key, &ty, desc.as_deref());
        }
    }

    /// The airtime estimate: exact time-on-air for one beacon at the settings
    /// currently in the editor, the duty cycle the beacon interval sets, and
    /// whether one transmission stays under the 400 ms dwell limit the US
    /// 902-928 MHz band imposes.
    ///
    /// The values are read from the editor the same way a push reads them - the
    /// wire bytes parsed back into a [`RadioConfig`] - so the estimate tracks
    /// edits the moment they are set, and reflects the same clamping the board
    /// would apply. An editor holding a value the firmware would reject parses
    /// to nothing, and the panel simply stays hidden until it is valid again.
    fn radio_estimate_ui(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.radio.as_ref() else {
            return;
        };
        let Ok(cfg) = radiocfg::parse_bytes(&doc.wire_bytes()) else {
            return;
        };
        let colors = self.config.ui;

        gap(ui, GAP_BLOCK);
        ui.strong("Airtime estimate");
        ui.separator();

        let payload = lora::HEADER_LEN + lora::position_msg_len(cfg.beacon_fields);
        let toa_ms = cfg.beacon_airtime_us() as f32 / 1000.0;
        ui.label(format!(
            "Time on air: {toa_ms:.1} ms per beacon \
             (SF{}, BW{} kHz, CR 4/{}, {payload}-byte frame)",
            cfg.spreading_factor, cfg.bandwidth_khz, cfg.coding_rate,
        ));

        // The beacon interval turns airtime into a duty cycle; interval 0 means
        // the beacon is off, so there is no periodic airtime to report.
        if cfg.beacon_interval_s == 0 {
            ui.label(
                egui::RichText::new("Beacon disabled (interval 0): no periodic airtime.").weak(),
            );
        } else {
            let duty = toa_ms / (cfg.beacon_interval_s as f32 * 1000.0) * 100.0;
            ui.label(format!(
                "Beacon interval: {} s  ->  duty cycle {duty:.2}%",
                cfg.beacon_interval_s,
            ));
        }

        // The 902-928 MHz FH rule caps channel dwell at 400 ms per 20 s, which
        // is a 2% duty cycle. At an interval below 20 s more than one beacon
        // lands in a 20 s window, so the per-beacon budget tightens to 2% of
        // the interval (200 ms at the 10 s default); a single transmission can
        // never top the 400 ms ceiling either. The check only means anything
        // in-band, so key it off the frequency.
        const DWELL_MS: f32 = 400.0;
        const DUTY_LIMIT: f32 = 0.02; // 2% = 400 ms per 20 s
        if (902_000_000..=928_000_000).contains(&cfg.frequency_hz) {
            // One beacon's budget: 2% of the interval, never above the 400 ms
            // dwell ceiling. With the beacon off (interval 0) only the ceiling
            // is left to test against.
            let duty_budget_ms = DUTY_LIMIT * cfg.beacon_interval_s as f32 * 1000.0;
            let budget_ms = if cfg.beacon_interval_s == 0 {
                DWELL_MS
            } else {
                duty_budget_ms.min(DWELL_MS)
            };
            let (verdict, color) = if toa_ms <= budget_ms {
                (
                    format!("Under the {budget_ms:.0} ms limit (902-928 MHz)"),
                    colors.ok,
                )
            } else {
                (
                    format!(
                        "Over the {budget_ms:.0} ms limit by {:.0} ms (902-928 MHz)",
                        toa_ms - budget_ms
                    ),
                    colors.error,
                )
            };
            ui.colored_label(color, verdict);
            // Say which of the two limits is binding, so the number is not a
            // bare figure the reader has to reverse-engineer.
            let basis = if budget_ms >= DWELL_MS {
                "400 ms channel dwell per 20 s".to_string()
            } else {
                format!("2% duty cycle over the {} s interval", cfg.beacon_interval_s)
            };
            ui.label(egui::RichText::new(format!("Limit: {basis}.")).weak());
        }
    }

    /// One field row: the key, then either the read-only value with a pencil
    /// (edit) button, or - while this field is unlocked - the typed input with a
    /// check (set) and an x (cancel). The description, if any, follows beneath.
    fn radio_field_row(
        &mut self,
        ui: &mut egui::Ui,
        section: &str,
        key: &str,
        ty: &FieldType,
        desc: Option<&str>,
    ) {
        let active = matches!(
            &self.radio_edit,
            RadioEdit::Active { section: s, key: k, .. }
                if s.as_str() == section && k.as_str() == key
        );
        // Action buttons sized to the text, so nothing is a raw pixel constant.
        let bsz = em(ui) * 1.2;
        // Wrapped so a long key or value drops its input to the next line
        // rather than pushing the edit buttons past the screen edge.
        ui.horizontal_wrapped(|ui| {
            ui.monospace(key);
            if active {
                if let RadioEdit::Active { val, .. } = &mut self.radio_edit {
                    radio_input(ui, key, ty, val);
                }
                let set = icon_button(ui, bsz, egui::include_image!("../../../assets/icons/check.svg"))
                    .on_hover_text("Set");
                if set.clicked() {
                    if let RadioEdit::Active { val, .. } = &self.radio_edit {
                        let val = val.clone();
                        if let Some(doc) = self.radio.as_mut() {
                            doc.apply(section, key, &val);
                        }
                    }
                    self.radio_edit = RadioEdit::None;
                }
                let cancel =
                    icon_button(ui, bsz, egui::include_image!("../../../assets/icons/close.svg"))
                        .on_hover_text("Cancel");
                if cancel.clicked() {
                    self.radio_edit = RadioEdit::None;
                }
            } else {
                let display = self.radio.as_ref().unwrap().display_at(section, key);
                ui.monospace(display);
                // While any field is mid-edit, lock the other pencils so only
                // one field is edited at a time.
                let busy = !matches!(self.radio_edit, RadioEdit::None);
                let edit = ui
                    .add_enabled_ui(!busy, |ui| {
                        icon_button(
                            ui,
                            bsz,
                            egui::include_image!("../../../assets/icons/edit.svg"),
                        )
                        .on_hover_text("Edit")
                    })
                    .inner;
                if edit.clicked() {
                    self.radio_edit = RadioEdit::Confirm {
                        section: section.to_string(),
                        key: key.to_string(),
                    };
                }
            }
        });
        if let Some(d) = desc {
            ui.label(egui::RichText::new(d).weak().small());
        }
        gap(ui, GAP_TIGHT);
    }

    /// The floating Edit / Cancel popup shown when a field's pencil is pressed.
    /// Confirming unlocks the field for editing; cancelling clears the flow.
    fn radio_confirm_popup(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let (section, key) = match &self.radio_edit {
            RadioEdit::Confirm { section, key } => (section.clone(), key.clone()),
            _ => return,
        };
        floating(
            ctx,
            "radio_confirm",
            egui::Order::Foreground,
            screen.center(),
            egui::Align2::CENTER_CENTER,
            false,
            |ui| {
                ui.label(format!("Edit \"{key}\"?"));
                gap(ui, GAP_ITEM);
                ui.horizontal(|ui| {
                    if ui.button("Edit").clicked() {
                        let val = self
                            .radio
                            .as_ref()
                            .map(|r| r.edit_val_at(&section, &key))
                            .unwrap_or(EditVal::Str(String::new()));
                        self.radio_edit = RadioEdit::Active {
                            section: section.clone(),
                            key: key.clone(),
                            val,
                        };
                    }
                    if ui.button("Cancel").clicked() {
                        self.radio_edit = RadioEdit::None;
                    }
                });
            },
        );
    }

    /// The floating Send / Cancel popup behind the Send-to-board button. A
    /// confirm because a push replaces the board's whole config - a key absent
    /// from the file reverts to its firmware default, not to what the board
    /// had - and takes effect immediately.
    fn radio_push_popup(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        if !self.radio_push_confirm {
            return;
        }
        floating(
            ctx,
            "radio_push_confirm",
            egui::Order::Foreground,
            screen.center(),
            egui::Align2::CENTER_CENTER,
            false,
            |ui| {
                ui.set_max_width(em(ui) * 18.0);
                ui.label("Send this config to the board?");
                ui.label(
                    egui::RichText::new(
                        "It replaces the board's whole config, takes effect immediately \
                         and is stored on the board.",
                    )
                    .weak()
                    .small(),
                );
                gap(ui, GAP_ITEM);
                ui.horizontal(|ui| {
                    if ui.button("Send").clicked() {
                        self.radio_push_confirm = false;
                        self.push_radio();
                    }
                    if ui.button("Cancel").clicked() {
                        self.radio_push_confirm = false;
                    }
                });
            },
        );
    }

    /// A collapsible list of kept backups, newest first, each restorable into
    /// the editor (a restored file is unsaved until Save writes it as current).
    fn radio_backups_ui(&mut self, ui: &mut egui::Ui) {
        let backups = match &self.radio {
            Some(r) => r.backups(),
            None => return,
        };
        gap(ui, GAP_BLOCK);
        ui.separator();
        egui::CollapsingHeader::new(format!("Backups ({})", backups.len()))
            .id_salt("radio_backups")
            .show(ui, |ui| {
                if backups.is_empty() {
                    ui.label("No backups yet. Saving keeps the previous version here.");
                }
                for b in &backups {
                    ui.horizontal(|ui| {
                        let name = b.file_name().and_then(|s| s.to_str()).unwrap_or("");
                        ui.monospace(name);
                        if ui.button("Restore").clicked() {
                            if let Some(doc) = self.radio.as_mut() {
                                let res = doc.restore(b);
                                self.radio_feedback = Some(match res {
                                    Ok(()) => Ok(format!("Restored {name} (unsaved - press Save)")),
                                    Err(e) => Err(e),
                                });
                            }
                            self.radio_edit = RadioEdit::None;
                        }
                    });
                }
            });
    }
}
