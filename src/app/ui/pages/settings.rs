//! The Settings page: the app's own settings - what it draws, what it
//! records, where its position comes from, and the settings file itself.
//!
//! The node's own settings are a separate page ([`MyApp::bluetooth_page`]);
//! the split is by who owns the setting, since only the ones here are the
//! app's to keep.
//!
//! Every widget is bound straight to the live [`crate::config::AppConfig`], so
//! a change takes effect on the map immediately; Save is what makes it outlast
//! the session. Most numbers are picked from a short list of presets, with a
//! custom entry for anything else.

use crate::app::ui::adjust::Adjust;
use crate::app::ui::text::settings as text;
use crate::app::ui::theme::{gap, probe, px, Key};
use crate::app::ui::widgets::{
    busy_dots, button, check, content_page, drag, feedback_label, grid, heading, hint,
    preset_pick, row, section, submitted, text_field,
};
use crate::app::{MyApp, Page, RegionSelect};
use crate::config::{
    DistanceUnits, ThemeChoice, COMPASS_HZ_MAX, COMPASS_HZ_MIN, RSSI_DBM_MAX, RSSI_DBM_MIN,
    STATUS_CYCLE_MAX, STATUS_CYCLE_MIN, TEXT_SCALE_MAX, TEXT_SCALE_MIN,
};
use crate::tiles::TileProvider;

/// The step the text-size slider moves in.
const TEXT_SCALE_STEP: f64 = 0.05;

/// The step the bar-opacity slider moves in.
const OPACITY_STEP: f64 = 0.05;

/// Range and drag speed for the overlay sizes, in points. The loader rejects a
/// size of 0 or less, so the drag stops short of one rather than writing a
/// file that will not load.
const SIZE_SPEED: f64 = 0.1;
const SIZE_RANGE: std::ops::RangeInclusive<f32> = 0.5..=64.0;

/// Drag speed for the status bar's per-node dwell, in seconds. Slower than a
/// whole second per point of travel: the useful range is a handful of seconds,
/// and the loader's own range is what stops the drag at either end.
const CYCLE_SPEED: f64 = 0.05;

/// The presets each number offers, each a spread over the range the loader
/// accepts.
const MARKER_SIZES: [f32; 4] = [6.0, 8.0, 12.0, 16.0];
const BEACON_SIZES: [f32; 4] = [4.0, 6.0, 9.0, 12.0];
const LINE_WIDTHS: [f32; 4] = [2.0, 3.0, 5.0, 8.0];
const TEXT_SIZES: [f32; 4] = [12.0, 14.0, 18.0, 24.0];
const PULSE_SECS: [f32; 5] = [0.0, 5.0, 10.0, 30.0, 60.0];
const ARROW_HZ: [f32; 5] = [1.0, 2.0, 4.0, 8.0, 15.0];
const CYCLE_SECS: [f32; 4] = [2.0, 5.0, 10.0, 30.0];
const RSSI_TOPS: [i16; 4] = [-10, -20, -30, -40];
const RSSI_BOTTOMS: [i16; 4] = [-100, -110, -120, -130];
const MIN_DISTANCES: [f64; 6] = [0.0, 1.0, 3.0, 5.0, 10.0, 25.0];

/// A number without trailing zeros, for a preset label: "3", "0.5".
fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A preset dropdown over an `f32` setting, the custom entry a drag over
/// `range`. `unit` follows each preset's number.
fn f32_pick(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut f32,
    presets: &[f32],
    unit: &str,
    speed: f64,
    range: std::ops::RangeInclusive<f32>,
) {
    let labels: Vec<(f32, String)> = presets
        .iter()
        .map(|&p| (p, format!("{}{unit}", num(f64::from(p)))))
        .collect();
    let labelled: Vec<(f32, &str)> = labels.iter().map(|(v, l)| (*v, l.as_str())).collect();
    preset_pick(ui, id, value, &labelled, |ui, v| {
        drag(ui, v, speed, range);
    });
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
        if ui
            .checkbox(&mut on, label)
            .on_hover_text(text::THEME_COLOR_HOVER)
            .changed()
        {
            *value = on.then_some(theme);
        }
        let mut color = value.unwrap_or(theme);
        ui.add_enabled_ui(on, |ui| ui.color_edit_button_srgba(&mut color));
        if on {
            *value = Some(color);
        }
    });
}

impl MyApp {
    pub(crate) fn settings_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let safe = self.safe_area(ctx);
        content_page(ctx, "settings", screen, safe, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading!(ui, "Settings");
                gap(ui, Key::GapBlock);

                ui.label("File:");
                ui.horizontal_wrapped(|ui| {
                    let resp = text_field(
                        ui,
                        &mut self.config_path,
                        "/path/to/app-settings.toml",
                        Key::SettingsConfigPath,
                    );
                    if ui.button("Load").clicked() || submitted(ui, &resp) {
                        self.load_config();
                    }
                    let path = self.config_path.clone();
                    self.copy_button(ui, &path);
                });
                gap(ui, Key::GapItem);
                ui.horizontal_wrapped(|ui| {
                    if button!(ui, "Save", hover: text::SAVE_HOVER).clicked() {
                        self.save_config();
                    }
                    if button!(ui, "Defaults", hover: text::RESET_HOVER).clicked() {
                        self.reset_config();
                    }
                });
                gap(ui, Key::GapItem);
                feedback_label(ui, self.config.ui, &self.config_feedback);

                self.phone_ui(ui);
                self.text_size_ui(ui);
                self.look_ui(ui);
                self.theme_ui(ui);
                self.colors_ui(ui);
                self.overlays_ui(ui);
                self.map_tiles_ui(ui);
                self.map_bars_ui(ui);
                self.compass_ui(ui);
                self.status_bar_ui(ui);
                self.track_ui(ui);
                self.offline_ui(ui);
            });
        });
    }

    /// This device: its name, and which receiver the current position comes
    /// from - its own, the connected node's, or both with the node's
    /// winning.
    fn phone_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Phone");
        gap(ui, Key::GapTight);
        row(ui, "Name:", |ui| {
            let resp = text_field(ui, &mut self.config.phone.name, "Phone", Key::BluetoothName);
            // A blank name is no name, as the loader reads it.
            if resp.lost_focus() && self.config.phone.name.trim().is_empty() {
                self.config.phone.name = crate::config::DEFAULT_PHONE_NAME.to_string();
            }
        });
        check!(
            ui,
            self.config.phone.location,
            "Position from the phone",
            hover: text::PHONE_LOCATION_HOVER,
        );
        check!(
            ui,
            self.config.ble.location,
            "Position from the node",
            hover: text::NODE_LOCATION_HOVER,
        );
    }

    fn text_size_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Text size");
        gap(ui, Key::GapTight);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().slider_width = px(ui.ctx(), Key::SettingsSlider);
            let slider = ui.add(
                egui::Slider::new(
                    &mut self.config.ui.text_scale,
                    TEXT_SCALE_MIN..=TEXT_SCALE_MAX,
                )
                .step_by(TEXT_SCALE_STEP)
                .fixed_decimals(2)
                .suffix("x"),
            );
            probe(ui.ctx(), slider.rect, "Slider", &[Key::SettingsSlider]);
            // Compared with a tolerance of half a step: the slider lands on
            // multiples of the step, which need not be exactly 1.0.
            let scaled = (self.config.ui.text_scale - 1.0).abs() > TEXT_SCALE_STEP as f32 / 2.0;
            let reset = button!(
                ui,
                "Reset",
                enabled: scaled,
                hover: text::TEXT_SCALE_RESET_HOVER,
            );
            if reset.clicked() {
                self.config.ui.text_scale = 1.0;
            }
        });
    }

    /// The look sheet: every size and spacing on the pages, and the adjuster
    /// that edits it live.
    fn look_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Look");
        gap(ui, Key::GapTight);
        ui.horizontal_wrapped(|ui| {
            let resp = text_field(
                ui,
                &mut self.look_path,
                "/path/to/gps-gui.look",
                Key::SettingsLookPath,
            );
            if ui.button("Load").clicked() || submitted(ui, &resp) {
                self.load_look();
            }
            let path = self.look_path.clone();
            self.copy_button(ui, &path);
        });
        gap(ui, Key::GapItem);
        ui.horizontal_wrapped(|ui| {
            if button!(ui, "Save", hover: text::SAVE_HOVER).clicked() {
                self.save_look();
            }
            if button!(ui, "Defaults", hover: text::RESET_HOVER).clicked() {
                self.reset_look();
            }
            let open = self.adjust.is_some();
            let adjust = button!(
                ui,
                "Adjust",
                enabled: !open,
                hover: text::ADJUST_HOVER,
                disabled: text::ADJUST_OPEN,
            );
            if adjust.clicked() {
                self.adjust = Some(Adjust::new());
            }
        });
        gap(ui, Key::GapItem);
        feedback_label(ui, self.config.ui, &self.look_feedback);
    }

    /// Which theme the pages are drawn in. The three overrides under "Page
    /// colors" are laid over whichever one this picks, so the order on the page
    /// is the order they are applied in.
    fn theme_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Theme");
        gap(ui, Key::GapTight);
        ui.horizontal_wrapped(|ui| {
            for choice in ThemeChoice::ALL {
                ui.selectable_value(&mut self.config.ui.theme, choice, choice.label());
            }
        });
    }

    /// The marker colors, the few page colors that carry meaning, and the
    /// three theme surfaces that can be overridden outright.
    fn colors_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Marker colors");
        grid!(ui, "cfg_colors", |ui| {
            "you" => ui.color_edit_button_srgba(&mut self.config.colors.track),
            "connected node" => ui.color_edit_button_srgba(&mut self.config.colors.fixed),
            "marker outline" => ui.color_edit_button_srgba(&mut self.config.colors.outline),
        });

        section!(ui, "Page colors");
        grid!(ui, "cfg_ui_colors", |ui| {
            "ok" => ui.color_edit_button_srgba(&mut self.config.ui.ok),
            "error" => ui.color_edit_button_srgba(&mut self.config.ui.error),
            "busy" => ui.color_edit_button_srgba(&mut self.config.ui.busy),
            "no-target pulse" => ui.color_edit_button_srgba(&mut self.config.ui.pulse),
        });

        gap(ui, Key::GapItem);
        // The surfaces and the text everything else is drawn with. Read out of
        // the live visuals, so an unticked row shows what the theme is
        // actually using and ticking it starts from there.
        let bg = ui.visuals().panel_fill;
        let button = ui.visuals().widgets.inactive.weak_bg_fill;
        let fg = ui.visuals().text_color();
        theme_color(ui, "Background", &mut self.config.ui.background, bg);
        theme_color(ui, "Buttons", &mut self.config.ui.button, button);
        theme_color(ui, "Text", &mut self.config.ui.text, fg);
    }

    /// What the map draws over the tiles, and how big.
    fn overlays_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Overlay sizes");
        let s = &mut self.config.sizes;
        grid!(ui, "cfg_sizes", |ui| {
            "you" => f32_pick(ui, "size_marker", &mut s.marker, &MARKER_SIZES, "", SIZE_SPEED, SIZE_RANGE),
            "nodes" => f32_pick(ui, "size_beacon", &mut s.beacon, &BEACON_SIZES, "", SIZE_SPEED, SIZE_RANGE),
            "track" => f32_pick(ui, "size_track", &mut s.track, &LINE_WIDTHS, "", SIZE_SPEED, SIZE_RANGE),
            "distance line" => f32_pick(ui, "size_dline", &mut s.distance_line, &LINE_WIDTHS, "", SIZE_SPEED, SIZE_RANGE),
            "distance text" => f32_pick(ui, "size_dtext", &mut s.distance_text, &TEXT_SIZES, "", SIZE_SPEED, SIZE_RANGE),
        });

        section!(ui, "Map overlays");
        check!(
            ui,
            self.config.track.show_path,
            "Your path",
            hover: text::CENTRAL_PATH_HOVER,
        );
        check!(
            ui,
            self.config.ble.show_on_map,
            "Connected node",
            hover: text::BOARD_ON_MAP_HOVER,
        );
        // The path is part of what the switch above takes off the map, so
        // its own box has nothing to say while the node is hidden.
        let board_drawn = self.config.ble.show_on_map;
        ui.add_enabled_ui(board_drawn, |ui| {
            check!(ui, self.config.ble.show_path, "Connected node's path");
        });
        check!(
            ui,
            self.config.lora.show_path,
            "Remote node paths",
            hover: text::REMOTE_PATHS_HOVER,
        );
        row(ui, "Remote node pulse:", |ui| {
            let labels: Vec<(f32, String)> = PULSE_SECS
                .iter()
                .map(|&p| {
                    (
                        p,
                        if p == 0.0 {
                            "never".to_string()
                        } else {
                            format!("{} s", num(f64::from(p)))
                        },
                    )
                })
                .collect();
            let labelled: Vec<(f32, &str)> =
                labels.iter().map(|(v, l)| (*v, l.as_str())).collect();
            preset_pick(ui, "pulse_secs", &mut self.config.lora.pulse_secs, &labelled, |ui, v| {
                drag(ui, v, 0.1, 0.0..=3600.0);
            });
        })
        .response
        .on_hover_text(text::PULSE_HOVER);

        gap(ui, Key::GapHair);
        check!(ui, self.config.distance.show, "Distance label");
        check!(ui, self.config.distance.dotted, "Dotted distance line");
        row(ui, "Units:", |ui| {
            for (units, label) in [
                (DistanceUnits::Metric, "km/m"),
                (DistanceUnits::Imperial, "mi/ft"),
            ] {
                ui.selectable_value(&mut self.config.distance.units, units, label);
            }
        });
    }

    /// Who serves the map's tiles. ArcGIS is the provider with satellite
    /// imagery, which the map's layer button then cycles to; its key field
    /// is only shown while it is picked, an OpenStreetMap map having no use
    /// for one.
    fn map_tiles_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Map tiles");
        gap(ui, Key::GapTight);
        ui.horizontal_wrapped(|ui| {
            for provider in TileProvider::ALL {
                ui.selectable_value(&mut self.config.map.tiles, provider, provider.label());
            }
        });
        if self.config.map.tiles == TileProvider::ArcGis {
            gap(ui, Key::GapHair);
            row(ui, "API key:", |ui| {
                text_field(
                    ui,
                    &mut self.config.map.arcgis_key,
                    "built-in key",
                    Key::SettingsArcGisKey,
                );
            });
        }
    }

    /// The two bars over the map: how see-through they are, and the key
    /// under the top one.
    fn map_bars_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Map bars");
        gap(ui, Key::GapTight);
        row(ui, "Opacity:", |ui| {
            ui.spacing_mut().slider_width = px(ui.ctx(), Key::SettingsSlider);
            let slider = ui.add(
                egui::Slider::new(&mut self.config.map.bar_opacity, 0.0..=1.0)
                    .step_by(OPACITY_STEP)
                    .fixed_decimals(2),
            );
            probe(ui.ctx(), slider.rect, "Slider", &[Key::SettingsSlider]);
        });
        check!(ui, self.config.map.show_key, "Color key");
    }

    fn compass_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Compass");
        check!(ui, self.config.compass.marker_arrow, "Marker arrow");
        let on = self.config.compass.marker_arrow;
        row(ui, "Rate:", |ui| {
            ui.add_enabled_ui(on, |ui| {
                f32_pick(
                    ui,
                    "arrow_hz",
                    &mut self.config.compass.arrow_hz,
                    &ARROW_HZ,
                    " Hz",
                    0.1,
                    COMPASS_HZ_MIN..=COMPASS_HZ_MAX,
                );
            });
        })
        .response
        .on_hover_text(text::ARROW_HZ_HOVER);
    }

    /// The map's bottom status bar: whether it is drawn, how long each node
    /// holds it, and the span its bars are drawn against.
    fn status_bar_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Map status bar");
        check!(ui, self.config.status_bar.show, "Show");
        let on = self.config.status_bar.show;
        ui.add_enabled_ui(on, |ui| {
            row(ui, "Per node:", |ui| {
                f32_pick(
                    ui,
                    "cycle_secs",
                    &mut self.config.status_bar.cycle_secs,
                    &CYCLE_SECS,
                    " s",
                    CYCLE_SPEED,
                    STATUS_CYCLE_MIN..=STATUS_CYCLE_MAX,
                );
            })
            .response
            .on_hover_text(text::STATUS_CYCLE_HOVER);
            gap(ui, Key::GapHair);
            let bar = &mut self.config.status_bar;
            grid!(ui, "cfg_rssi", |ui| {
                "full at" => {
                    let labels: Vec<(i16, String)> =
                        RSSI_TOPS.iter().map(|&v| (v, format!("{v} dBm"))).collect();
                    let labelled: Vec<(i16, &str)> =
                        labels.iter().map(|(v, l)| (*v, l.as_str())).collect();
                    preset_pick(ui, "rssi_top", &mut bar.rssi_top_dbm, &labelled, |ui, v| {
                        drag(ui, v, 1.0, RSSI_DBM_MIN..=RSSI_DBM_MAX);
                    })
                },
                "empty at" => {
                    let labels: Vec<(i16, String)> =
                        RSSI_BOTTOMS.iter().map(|&v| (v, format!("{v} dBm"))).collect();
                    let labelled: Vec<(i16, &str)> =
                        labels.iter().map(|(v, l)| (*v, l.as_str())).collect();
                    preset_pick(ui, "rssi_bottom", &mut bar.rssi_bottom_dbm, &labelled, |ui, v| {
                        drag(ui, v, 1.0, RSSI_DBM_MIN..=RSSI_DBM_MAX);
                    })
                },
            });
        });
    }

    fn track_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Track recording");
        row(ui, "Min move:", |ui| {
            let labels: Vec<(f64, String)> = MIN_DISTANCES
                .iter()
                .map(|&v| (v, format!("{} m", num(v))))
                .collect();
            let labelled: Vec<(f64, &str)> =
                labels.iter().map(|(v, l)| (*v, l.as_str())).collect();
            preset_pick(
                ui,
                "min_distance",
                &mut self.config.track.min_distance,
                &labelled,
                |ui, v| {
                    drag(ui, v, 0.1, 0.0..=1000.0);
                },
            );
        });
        gap(ui, Key::GapItem);
        // The map bar's old Clear button is a path toggle now, and discarding
        // the recorded points is not something to leave a finger's width from
        // the buttons used while moving.
        let points = self.recorded_points();
        let discard = button!(
            ui,
            "Discard points",
            enabled: points > 0,
            hover: text::DISCARD_HOVER,
        );
        if discard.clicked() {
            self.discard_tracks();
        }
        hint!(ui, "{points} recorded");
    }

    /// Starting a region download. Only when tiles are cached to disk; it
    /// jumps to the map and begins the box selection there.
    fn offline_ui(&mut self, ui: &mut egui::Ui) {
        if self.cache_dir.is_none() {
            return;
        }
        section!(ui, sep "Offline maps");
        gap(ui, Key::GapItem);
        let downloading = self.download.is_some();
        let start = button!(
            ui,
            "Download region",
            enabled: !downloading,
            hover: text::DOWNLOAD_HOVER,
        );
        if start.clicked() {
            self.page = Page::Map;
            self.select = RegionSelect::Picking {
                start: None,
                current: None,
            };
        }
        if downloading {
            ui.colored_label(
                self.config.ui.busy,
                format!("{}{}", text::DOWNLOAD_BUSY, busy_dots(ui.ctx())),
            );
        }
    }
}
