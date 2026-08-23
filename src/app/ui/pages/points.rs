//! The Points page: a searchable, filterable list of every recorded GPS point.

use std::time::SystemTime;

use walkers::Position;

use crate::app::ui::text::points as text;
use crate::app::ui::theme::{
    control_height, em, field_width, gap, page_margin, GAP_BLOCK, GAP_HAIR, GAP_ITEM,
};
use crate::app::ui::widgets::{content_page, heading, text_field};
use crate::app::{MyApp, Page, PointFilter};
use crate::points::{age_text, PointSource};

/// The search box is most of its row, leaving the Clear button beside it.
const SEARCH_FRAC: f32 = 0.6;

/// The shortest the list may be squeezed to, in text heights: below this the
/// page would show filters over nothing.
const LIST_MIN_EM: f32 = 4.0;

impl MyApp {
    pub(crate) fn points_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let safe = self.safe_area(ctx);
        content_page(ctx, "points", screen, safe, |ui| {
            heading!(ui, "GPS points");
            gap(ui, GAP_BLOCK);

            ui.horizontal_wrapped(|ui| {
                let width = field_width(ui, screen, SEARCH_FRAC);
                text_field(ui, &mut self.points_search, text::SEARCH_HINT, width);
                if ui.button("Clear").clicked() {
                    self.points_search.clear();
                }
            });
            gap(ui, GAP_HAIR);

            ui.horizontal_wrapped(|ui| {
                ui.label("Source:");
                // The source names come from `PointSource::label`, so the
                // filter and the rows below it always read the same. The
                // exception is the one entry standing for every remote node: a
                // node's own address shows in its rows rather than as its own
                // filter button.
                for (filter, label) in [
                    (PointFilter::All, "all".to_string()),
                    (PointFilter::Phone, PointSource::Phone.label()),
                    (PointFilter::Esp, PointSource::Esp.label()),
                    (PointFilter::Remote, "nodes".to_string()),
                ] {
                    ui.selectable_value(&mut self.points_filter, filter, label);
                }
            });
            gap(ui, GAP_ITEM);

            let query = self.points_search.trim().to_lowercase();
            let rows = self.visible_points(self.points_filter, &query);
            ui.label(format!("{} of {} points", rows.len(), self.recorded_points()));
            gap(ui, GAP_HAIR);

            let now = SystemTime::now();
            // A row is a `selectable_label`, which egui floors at the control
            // height, so that is what `show_rows` has to be told: it places
            // rows by arithmetic, and a height that disagrees with the one
            // actually drawn scrolls the list out of step with itself.
            let row_height = ui
                .text_style_height(&egui::TextStyle::Monospace)
                .max(control_height(ui));
            // Everything left below the filters, less the page's own bottom
            // margin, and never so short that no row fits.
            let floor = screen.bottom() - safe.bottom - page_margin(screen);
            let list_height = (floor - ui.cursor().min.y).max(em(ui) * LIST_MIN_EM);
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
}
