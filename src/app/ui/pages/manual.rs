//! Entering a position by hand where no live GPS source is wired up: the
//! desktop's row in the Settings page's Phone section.

use crate::app::ui::text::manual as text;
use crate::app::ui::theme::Key;
use crate::app::ui::widgets::{row, submitted, text_field};
use crate::app::{LocationSource, MyApp};
use crate::gps::GpsFix;
use crate::points::parse_lat_lon;

impl MyApp {
    /// A row for typing a position in when no live GPS source is wired up
    /// (desktop). Accepts "lat, lon" or "lat lon"; a valid entry feeds the
    /// same pipeline a real fix would and recenters the map. Nothing on a
    /// phone, which has a receiver of its own.
    pub(crate) fn manual_position_row(&mut self, ui: &mut egui::Ui) {
        if self.gps.is_some() {
            return;
        }
        row(ui, "Position:", |ui| {
            let resp = text_field(ui, &mut self.manual_gps_text, "lat, lon", Key::ManualField);
            let entered = submitted(ui, &resp);
            if ui.button("Set").clicked() || entered {
                match parse_lat_lon(&self.manual_gps_text) {
                    Some((lat, lon)) => {
                        self.manual_gps_bad = false;
                        self.apply_gps_fix(
                            GpsFix {
                                lat,
                                lon,
                                bearing: None,
                                speed: None,
                            },
                            LocationSource::Manual,
                        );
                        self.map_memory.follow_my_position();
                    }
                    None => self.manual_gps_bad = true,
                }
            }
        });
        if self.manual_gps_bad {
            ui.colored_label(self.config.ui.error, text::BAD_COORD);
        }
    }
}
