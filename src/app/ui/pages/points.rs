//! The Points page: a searchable, filterable list of every recorded GPS point.

use std::time::SystemTime;

use walkers::Position;

use crate::app::ui::text::points as text;
use crate::app::ui::theme::{control_height, gap, page_margin, probe, px, Key};
use crate::app::ui::widgets::{content_page, heading, text_field};
use crate::app::{MyApp, Page, PointFilter};
use crate::points::{age_text, PointSource};

/// The narrowest the source column is drawn, in characters: room for the
/// generic names, so the column does not jump when the first board is named.
const SOURCE_COLUMN_MIN: usize = 8;

impl MyApp {
    pub(crate) fn points_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let safe = self.safe_area(ctx);
        content_page(ctx, "points", screen, safe, |ui| {
            heading!(ui, "GPS points");
            gap(ui, Key::GapBlock);

            ui.horizontal_wrapped(|ui| {
                text_field(
                    ui,
                    &mut self.points_search,
                    text::SEARCH_HINT,
                    Key::PointsSearch,
                );
                if ui.button("Clear").clicked() {
                    self.points_search.clear();
                }
            });
            gap(ui, Key::GapHair);

            ui.horizontal_wrapped(|ui| {
                ui.label("Source:");
                // The source names come from `MyApp::source_label`, so the
                // filter and the rows below it always read the same - and the
                // same as the map's markers. The exception is the one entry
                // standing for every remote node: a node's own name shows in
                // its rows rather than as its own filter button.
                for (filter, label) in [
                    (PointFilter::All, "all".to_string()),
                    (PointFilter::Phone, self.source_label(PointSource::Phone)),
                    (PointFilter::Esp, self.source_label(PointSource::Esp)),
                    (PointFilter::Remote, "nodes".to_string()),
                ] {
                    ui.selectable_value(&mut self.points_filter, filter, label);
                }
            });
            gap(ui, Key::GapItem);

            let query = self.points_search.trim().to_lowercase();
            let rows = self.visible_points(self.points_filter, &query);
            ui.label(format!(
                "{} of {} points",
                rows.len(),
                self.recorded_points()
            ));
            gap(ui, Key::GapHair);

            let now = SystemTime::now();
            // The source column is as wide as the longest name any row could
            // carry, so the coordinates stay in line down the list. Measured
            // over the sources rather than the rows: a name is per source,
            // and there are a few of those against thousands of rows.
            let sources = [PointSource::Phone, PointSource::Esp]
                .into_iter()
                .chain(self.remotes.keys().map(|&addr| PointSource::Remote(addr)));
            let column = sources
                .map(|source| self.source_label(source).chars().count())
                .max()
                .unwrap_or(0)
                .max(SOURCE_COLUMN_MIN);
            // A row is a `selectable_label`, which egui floors at the control
            // height, so that is what `show_rows` has to be told: it places
            // rows by arithmetic, and a height that disagrees with the one
            // actually drawn scrolls the list out of step with itself.
            let row_height = ui
                .text_style_height(&egui::TextStyle::Monospace)
                .max(control_height(ui));
            // Everything left below the filters, less the page's own bottom
            // margin, and never so short that no row fits.
            let floor = screen.bottom() - safe.bottom - page_margin(ctx);
            let list_height = (floor - ui.cursor().min.y).max(px(ctx, Key::PointsListMin));
            let mut goto: Option<Position> = None;
            let list = egui::ScrollArea::vertical()
                .max_height(list_height)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, rows.len(), |ui, range| {
                    for p in &rows[range] {
                        let text = format!(
                            "{:<column$} {}  {:>7}",
                            self.source_label(p.source),
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
            probe(
                ctx,
                list.inner_rect,
                "Points list",
                &[Key::PointsListMin, Key::ControlHeight],
            );
            if let Some(pos) = goto {
                self.map_memory.center_at(pos);
                self.page = Page::Map;
            }
        });
    }
}
