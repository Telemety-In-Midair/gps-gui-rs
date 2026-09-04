//! The desktop manual-position bar: entering a position by hand where no live
//! GPS source is wired up.

use crate::app::ui::text::manual as text;
use crate::app::ui::theme::{corner_margin, Key};
use crate::app::ui::widgets::{floating, submitted, text_field};
use crate::app::MyApp;
use crate::gps::GpsFix;
use crate::points::parse_lat_lon;

impl MyApp {
    /// Bottom-anchored bar for entering a position by hand when no live GPS
    /// source is wired up (desktop). Accepts "lat, lon" or "lat lon"; a valid
    /// entry feeds the same pipeline a real fix would and recenters the map.
    pub(crate) fn manual_gps_bar(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let margin = corner_margin(ctx);
        // The status bar owns the foot of the screen when it is on, so this
        // sits on top of it rather than under it.
        let foot = self.bottom_overlay_inset(ctx);
        floating(
            ctx,
            "manual_gps",
            egui::Order::Foreground,
            egui::Pos2::new(screen.center().x, screen.bottom() - foot - margin),
            egui::Align2::CENTER_BOTTOM,
            false,
            |ui| {
                ui.horizontal(|ui| {
                    ui.label("Position:");
                    let resp =
                        text_field(ui, &mut self.manual_gps_text, "lat, lon", Key::ManualField);
                    let entered = submitted(ui, &resp);
                    if ui.button("Set").clicked() || entered {
                        match parse_lat_lon(&self.manual_gps_text) {
                            Some((lat, lon)) => {
                                self.manual_gps_bad = false;
                                self.apply_gps_fix(GpsFix {
                                    lat,
                                    lon,
                                    bearing: None,
                                    speed: None,
                                });
                                self.map_memory.follow_my_position();
                            }
                            None => self.manual_gps_bad = true,
                        }
                    }
                });
                if self.manual_gps_bad {
                    ui.colored_label(self.config.ui.error, text::BAD_COORD);
                }
            },
        );
    }
}
