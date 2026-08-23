//! The Logging page: arming the CSV recorder, the graph over what it has
//! recorded, and getting the file off the device.
//!
//! The graph itself is painted in [`crate::app::ui::plot`]; this file is the
//! controls around it.

use std::collections::BTreeMap;

use walkers::Position;

use crate::app::ui::plot::{self, Series};
use crate::app::ui::text::logging as text;
use crate::app::ui::theme::{
    field_width, gap, GAP_BLOCK, GAP_HAIR, GAP_ITEM, GAP_SECTION, GAP_TIGHT,
};
use crate::app::ui::widgets::{
    button, check, content_page, feedback_label, heading, hint, text_field,
};
use crate::app::MyApp;
use crate::logging::{LogAxis, LogSource, LogStat};
use crate::points::parse_lat_lon;

/// The file path takes half the row, the reference coordinate a little less:
/// the reference sits ahead of three buttons rather than two.
const PATH_FRAC: f32 = 0.5;
const REFERENCE_FRAC: f32 = 0.45;

impl MyApp {
    pub(crate) fn logging_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let safe = self.safe_area(ctx);
        content_page(ctx, "logging", screen, safe, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading!(ui, "Logging", text::INTRO);
                gap(ui, GAP_BLOCK);
                self.log_controls_ui(ui, screen);
                gap(ui, GAP_SECTION);
                self.log_reference_ui(ui, screen);
                gap(ui, GAP_SECTION);
                self.log_graph_ui(ui, screen);
            });
        });
    }

    /// The file, the Start/Stop switch, and what the run has written so far.
    fn log_controls_ui(&mut self, ui: &mut egui::Ui, screen: egui::Rect) {
        let recording = self.logger.is_recording();
        let path_width = field_width(ui, screen, PATH_FRAC);

        ui.strong("Log file (CSV)");
        gap(ui, GAP_HAIR);
        ui.horizontal_wrapped(|ui| {
            // Locked while recording: the open file is what the path names,
            // and letting it be retyped mid-run would leave the two disagreeing
            // about where the rows are going.
            ui.add_enabled_ui(!recording, |ui| {
                text_field(ui, &mut self.log_path, "/path/to/log.csv", path_width);
            });
            if recording {
                if button!(ui, "Stop", hover: text::STOP_HOVER).clicked() {
                    self.stop_log();
                }
            } else if button!(ui, "Start", hover: text::START_HOVER).clicked() {
                self.start_log();
            }
        });

        gap(ui, GAP_ITEM);
        // An existing file is appended to, which is what makes a stop a pause
        // rather than the end of a run - and is worth saying, since the usual
        // meaning of picking a file to write is that it gets replaced.
        hint!(ui, text::APPEND_NOTE);

        gap(ui, GAP_ITEM);
        ui.horizontal_wrapped(|ui| {
            check!(ui, self.config.log.auto_start, "Start recording at launch");
            if button!(ui, "Save settings", hover: text::SAVE_HOVER).clicked() {
                // The path only becomes a setting when it is saved by hand: a
                // generated timestamped name is for this run, and writing it
                // back would pin every later run to the same file.
                self.config.log.file = Some(self.log_path.clone());
                self.save_config();
            }
        });
        gap(ui, GAP_HAIR);
        feedback_label(ui, self.config.ui, &self.config_feedback);

        gap(ui, GAP_ITEM);
        let rows = self.logger.rows().len();
        ui.label(
            egui::RichText::new(text::state(
                recording,
                self.logger.started(),
                self.logger.written(),
            ))
            .color(if recording {
                self.config.ui.ok
            } else {
                ui.visuals().text_color()
            }),
        );
        ui.label(format!("{rows} rows on the graph"));
        if self.logger.dropped() > 0 {
            hint!(ui, text::dropped(self.logger.dropped()));
        }

        gap(ui, GAP_ITEM);
        ui.horizontal_wrapped(|ui| {
            let hover = if self.export.is_some() {
                text::EXPORT_HOVER_PHONE
            } else {
                text::EXPORT_HOVER_DESKTOP
            };
            if button!(ui, "Export a copy", hover: hover).clicked() {
                self.export_log();
            }
            let clear = button!(
                ui,
                "Clear graph",
                enabled: rows > 0,
                hover: text::CLEAR_HOVER,
            );
            if clear.clicked() {
                self.logger.clear_rows();
            }
        });
        gap(ui, GAP_HAIR);
        feedback_label(ui, self.config.ui, &self.log_feedback);
    }

    /// The fixed coordinate the `dist_ref_m` column is measured against.
    fn log_reference_ui(&mut self, ui: &mut egui::Ui, screen: egui::Rect) {
        ui.strong("Reference point");
        gap(ui, GAP_HAIR);
        hint!(ui, text::REFERENCE);
        gap(ui, GAP_TIGHT);

        let width = field_width(ui, screen, REFERENCE_FRAC);
        // Every button here sets the same thing, so they all report through
        // one slot applied after the row: `Some(point)` sets it, `Some(None)`
        // clears it, `None` means nothing was pressed.
        let mut commit: Option<Option<(f64, f64)>> = None;
        ui.horizontal_wrapped(|ui| {
            let resp = text_field(ui, &mut self.log_ref_text, "lat, lon", width);
            // Committed on blur, like the board nicknames: a coordinate is not
            // valid until it is fully typed, and parsing per keystroke would
            // flag every half of one as bad.
            if resp.lost_focus() {
                let text = self.log_ref_text.trim().to_string();
                if text.is_empty() {
                    commit = Some(None);
                } else {
                    match parse_lat_lon(&text) {
                        Some(point) => commit = Some(Some(point)),
                        None => self.log_ref_bad = true,
                    }
                }
            }
            if button!(ui, "Use my position", enabled: self.current.is_some()).clicked() {
                commit = self.current.map(|p| Some((p.y(), p.x())));
            }
            // The beacon, when there is one: setting the reference to where the
            // board is standing is the usual way a range test starts.
            let beacon: Option<Position> = self.beacon;
            if button!(ui, "Use beacon", enabled: beacon.is_some()).clicked() {
                commit = beacon.map(|p| Some((p.y(), p.x())));
            }
            let has_ref = self.config.log.reference().is_some();
            if button!(ui, "Clear", enabled: has_ref).clicked() {
                commit = Some(None);
            }
        });

        if let Some(point) = commit {
            self.config.log.set_reference(point);
            self.log_ref_text = match point {
                Some((lat, lon)) => format!("{lat:.5}, {lon:.5}"),
                None => String::new(),
            };
            self.log_ref_bad = false;
        }
        if self.log_ref_bad {
            ui.colored_label(self.config.ui.error, text::BAD_COORD);
        }
    }

    /// The graph: the two axis pickers, the plot, and the legend that doubles
    /// as the per-source filter.
    fn log_graph_ui(&mut self, ui: &mut egui::Ui, screen: egui::Rect) {
        ui.strong("Graph");
        gap(ui, GAP_HAIR);
        ui.horizontal_wrapped(|ui| {
            ui.label("Plot");
            egui::ComboBox::from_id_salt("log_y")
                .selected_text(self.log_y.label())
                .show_ui(ui, |ui| {
                    for stat in LogStat::ALL {
                        ui.selectable_value(&mut self.log_y, stat, stat.label());
                    }
                });
            ui.label("against");
            egui::ComboBox::from_id_salt("log_x")
                .selected_text(self.log_x.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.log_x, LogAxis::Time, LogAxis::Time.label());
                    for stat in LogStat::ALL {
                        ui.selectable_value(&mut self.log_x, LogAxis::Stat(stat), stat.label());
                    }
                });
        });

        gap(ui, GAP_TIGHT);
        let series = Series::collect(
            self.logger.rows(),
            self.log_x,
            self.log_y,
            &self.log_hidden,
            self.config.colors,
        );
        plot::draw(
            ui,
            screen,
            self.log_x,
            self.log_y,
            &series,
            !self.logger.rows().is_empty(),
        );
        gap(ui, GAP_TIGHT);
        self.log_legend_ui(ui, &series);
    }

    /// The legend, whose entries are also the per-source filter: what is drawn
    /// and what it is drawn in are the same question, so they are one control.
    fn log_legend_ui(&mut self, ui: &mut egui::Ui, series: &[Series]) {
        // Every source seen, not just the drawn ones - a hidden source has to
        // stay in the legend or there is no way to bring it back.
        let sources: Vec<LogSource> = self
            .logger
            .rows()
            .iter()
            .map(|r| r.source)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if sources.is_empty() {
            return;
        }
        let counts: BTreeMap<LogSource, usize> =
            series.iter().map(|s| (s.source, s.points.len())).collect();
        ui.horizontal_wrapped(|ui| {
            for source in sources {
                let hidden = self.log_hidden.contains(&source);
                let label = match source {
                    LogSource::Node(addr) => self.config.lora.label_of(addr),
                    LogSource::Phone => "This device".to_string(),
                    LogSource::Board => "Board".to_string(),
                    LogSource::Telemetry => "Board link".to_string(),
                };
                let points = counts.get(&source).copied().unwrap_or(0);
                let text = if hidden {
                    label
                } else {
                    format!("{label} ({points})")
                };
                let color = if hidden {
                    ui.visuals().weak_text_color()
                } else {
                    source.color(self.config.colors)
                };
                // Not `.small()`: that is the one thing that opts a button
                // out of the touch-target floor, and a legend entry is a
                // filter switch, not a caption.
                let entry =
                    egui::Button::new(egui::RichText::new(text).color(color)).selected(!hidden);
                if ui.add(entry).on_hover_text(text::LEGEND_HOVER).clicked() {
                    if hidden {
                        self.log_hidden.remove(&source);
                    } else {
                        self.log_hidden.insert(source);
                    }
                }
            }
        });
    }
}
