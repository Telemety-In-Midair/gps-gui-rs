//! The Settings page: the app's own TOML settings - what it draws, what it
//! records, and the config file itself.
//!
//! The beacon and the board's own settings are a separate page
//! ([`MyApp::beacon_page`]); the split is by who owns the setting, since only
//! the ones here are the app's to keep.
//!
//! Every widget is bound straight to the live [`crate::config::AppConfig`], so
//! a change takes effect on the map immediately; Save is what makes it outlast
//! the session.

use crate::app::ui::adjust::Adjust;
use crate::app::ui::text::settings as text;
use crate::app::ui::theme::{gap, probe, px, Key};
use crate::app::ui::widgets::{
    button, check, content_page, drag, feedback_label, grid, heading, hint, row, section,
    submitted, text_field,
};
use crate::app::{MyApp, Page, RegionSelect};
use crate::config::{
    DistanceUnits, COMPASS_HZ_MAX, COMPASS_HZ_MIN, STATUS_CYCLE_MAX, STATUS_CYCLE_MIN,
    TEXT_SCALE_MAX, TEXT_SCALE_MIN,
};

/// The step the text-size slider moves in.
const TEXT_SCALE_STEP: f64 = 0.05;

/// Range and drag speed for the overlay sizes, in points. The loader rejects a
/// size of 0 or less, so the drag stops short of one rather than writing a
/// file that will not load.
const SIZE_SPEED: f64 = 0.1;
const SIZE_RANGE: std::ops::RangeInclusive<f32> = 0.5..=64.0;

/// Drag speed for the status bar's per-node dwell, in seconds. Slower than a
/// whole second per point of travel: the useful range is a handful of seconds,
/// and the loader's own range is what stops the drag at either end.
const CYCLE_SPEED: f64 = 0.05;

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

impl MyApp {
    pub(crate) fn settings_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let safe = self.safe_area(ctx);
        content_page(ctx, "settings", screen, safe, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading!(ui, "App settings", text::INTRO);
                gap(ui, Key::GapBlock);

                ui.label("Config file (TOML):");
                ui.horizontal_wrapped(|ui| {
                    let resp = text_field(
                        ui,
                        &mut self.config_path,
                        "/path/to/config.toml",
                        Key::SettingsPath,
                    );
                    if ui.button("Load").clicked() || submitted(ui, &resp) {
                        self.load_config();
                    }
                });
                gap(ui, Key::GapItem);
                ui.horizontal_wrapped(|ui| {
                    if button!(ui, "Save", hover: text::SAVE_HOVER).clicked() {
                        self.save_config();
                    }
                    if button!(ui, "Reset to defaults", hover: text::RESET_HOVER).clicked() {
                        self.reset_config();
                    }
                });
                gap(ui, Key::GapItem);
                feedback_label(ui, self.config.ui, &self.config_feedback);

                self.text_size_ui(ui);
                self.look_ui(ui);
                self.colors_ui(ui);
                self.overlays_ui(ui);
                self.compass_ui(ui);
                self.status_bar_ui(ui);
                self.track_ui(ui);
                self.offline_ui(ui);
            });
        });
    }

    fn text_size_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Text size", text::TEXT_SCALE);
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
        section!(ui, "Look", text::LOOK);
        gap(ui, Key::GapTight);
        ui.horizontal_wrapped(|ui| {
            let resp = text_field(
                ui,
                &mut self.look_path,
                "/path/to/gps-gui.look",
                Key::SettingsPath,
            );
            if ui.button("Load").clicked() || submitted(ui, &resp) {
                self.load_look();
            }
        });
        gap(ui, Key::GapItem);
        ui.horizontal_wrapped(|ui| {
            if button!(ui, "Save", hover: text::LOOK_SAVE_HOVER).clicked() {
                self.save_look();
            }
            if button!(ui, "Reset to defaults", hover: text::LOOK_RESET_HOVER).clicked() {
                self.reset_look();
            }
            let open = self.adjust.is_some();
            let adjust = button!(
                ui,
                "Adjust the look",
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

    /// The marker colors, the few page colors that carry meaning, and the
    /// three theme surfaces that can be overridden outright.
    fn colors_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Marker colors");
        grid!(ui, "cfg_colors", |ui| {
            "track" => ui.color_edit_button_srgba(&mut self.config.colors.track),
            "beacon" => ui.color_edit_button_srgba(&mut self.config.colors.fixed),
            "marker outline" => ui.color_edit_button_srgba(&mut self.config.colors.outline),
        });

        section!(ui, "Page colors", text::PAGE_COLORS);
        grid!(ui, "cfg_ui_colors", |ui| {
            "ok" => ui.color_edit_button_srgba(&mut self.config.ui.ok),
            "error" => ui.color_edit_button_srgba(&mut self.config.ui.error),
            "no-target pulse" => ui.color_edit_button_srgba(&mut self.config.ui.pulse),
        });

        gap(ui, Key::GapItem);
        // The surfaces and the text everything else is drawn with. Read out of
        // the live visuals, so an unticked row shows what the theme is
        // actually using and ticking it starts from there.
        let bg = ui.visuals().panel_fill;
        let button = ui.visuals().widgets.inactive.weak_bg_fill;
        let fg = ui.visuals().text_color();
        theme_color(ui, "Set the background", &mut self.config.ui.background, bg);
        theme_color(ui, "Set the buttons", &mut self.config.ui.button, button);
        theme_color(ui, "Set the text", &mut self.config.ui.text, fg);
        gap(ui, Key::GapHair);
        hint!(ui, text::THEME_COLORS);
    }

    /// What the map draws over the tiles, and how big.
    fn overlays_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Overlay sizes (points)");
        let s = &mut self.config.sizes;
        grid!(ui, "cfg_sizes", |ui| {
            "marker" => drag(ui, &mut s.marker, SIZE_SPEED, SIZE_RANGE),
            "beacon" => drag(ui, &mut s.beacon, SIZE_SPEED, SIZE_RANGE),
            "track" => drag(ui, &mut s.track, SIZE_SPEED, SIZE_RANGE),
            "distance line" => drag(ui, &mut s.distance_line, SIZE_SPEED, SIZE_RANGE),
            "distance text" => drag(ui, &mut s.distance_text, SIZE_SPEED, SIZE_RANGE),
        });

        section!(ui, "Map overlays");
        // "Central" is what the Points page calls this device's own fixes, so
        // the two pages name the same track the same way.
        check!(
            ui,
            self.config.track.show_path,
            "Show central path on map",
            hover: text::CENTRAL_PATH_HOVER,
        );
        check!(
            ui,
            self.config.ble.show_on_map,
            "Show the connected board on map",
            hover: text::BOARD_ON_MAP_HOVER,
        );
        // The path is part of what the switch above takes off the map, so
        // its own box has nothing to say while the board is hidden.
        let board_drawn = self.config.ble.show_on_map;
        ui.add_enabled_ui(board_drawn, |ui| {
            check!(ui, self.config.ble.show_path, "Show beacon path on map");
        });
        check!(
            ui,
            self.config.lora.show_path,
            "Show remote node paths on map",
            hover: text::REMOTE_PATHS_HOVER,
        );
        gap(ui, Key::GapHair);
        hint!(ui, text::PATHS_NOTE);

        gap(ui, Key::GapHair);
        check!(
            ui,
            self.config.distance.show,
            "Show distance on the line to the beacon"
        );
        check!(
            ui,
            self.config.distance.dotted,
            "Draw distance line dotted rather than solid"
        );
        row(ui, "Units:", |ui| {
            for (units, label) in [
                (DistanceUnits::Metric, "km/m"),
                (DistanceUnits::Imperial, "mi/ft"),
            ] {
                ui.selectable_value(&mut self.config.distance.units, units, label);
            }
        });
    }

    fn compass_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Compass", text::COMPASS);
        check!(
            ui,
            self.config.compass.marker_arrow,
            "Point the marker arrow with the compass"
        );
        let on = self.config.compass.marker_arrow;
        row(ui, "Compass rate for the arrow (Hz):", |ui| {
            ui.add_enabled_ui(on, |ui| {
                drag(
                    ui,
                    &mut self.config.compass.arrow_hz,
                    0.1,
                    COMPASS_HZ_MIN..=COMPASS_HZ_MAX,
                )
                .on_hover_text(text::ARROW_HZ_HOVER);
            });
        });
    }

    /// The map's bottom status bar: whether it is drawn, and how long each
    /// node holds it.
    fn status_bar_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Map status bar", text::STATUS_BAR);
        check!(
            ui,
            self.config.status_bar.show,
            "Show the status bar at the bottom of the map",
            hover: text::STATUS_BAR_SHOW_HOVER,
        );
        let on = self.config.status_bar.show;
        row(ui, "Seconds per node:", |ui| {
            ui.add_enabled_ui(on, |ui| {
                drag(
                    ui,
                    &mut self.config.status_bar.cycle_secs,
                    CYCLE_SPEED,
                    STATUS_CYCLE_MIN..=STATUS_CYCLE_MAX,
                )
                .on_hover_text(text::STATUS_CYCLE_HOVER);
            });
        });
    }

    fn track_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Track recording");
        row(ui, "Minimum move between points (m):", |ui| {
            drag(ui, &mut self.config.track.min_distance, 0.1, 0.0..=1000.0);
        });
        gap(ui, Key::GapItem);
        // The map bar's old Clear button is a path toggle now, and discarding
        // the recorded points is not something to leave a finger's width from
        // the buttons used while moving.
        let points = self.recorded_points();
        let discard = button!(
            ui,
            "Discard recorded points",
            enabled: points > 0,
            hover: text::DISCARD_HOVER,
        );
        if discard.clicked() {
            self.discard_tracks();
        }
        hint!(ui, "{points} points recorded");
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
            ui.label(text::DOWNLOAD_BUSY);
        }
    }
}
