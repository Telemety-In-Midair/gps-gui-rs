//! The Logging page: arming the CSV recorder, the graph over what it has
//! recorded, and getting the file off the device.
//!
//! The graph is painted by hand rather than pulled in from a plotting crate.
//! What it has to do is narrow - a handful of series, autoscaled, in the
//! config's own colors, at a size measured off the screen like every other
//! page here - and a crate that did it would arrive with its own sizing,
//! theming and gesture handling to fight.

use std::collections::BTreeMap;

use walkers::Position;

use super::{
    em, feedback_label, field_width, gap, GAP_BLOCK, GAP_HAIR, GAP_ITEM, GAP_SECTION, GAP_TIGHT,
};
use crate::app::MyApp;
use crate::logging::{iso8601, LogAxis, LogSource, LogStat};
use crate::points::age_text;

/// Plot height as a fraction of the screen height. Written against the screen
/// rather than the text because it is a picture: it should keep its shape when
/// the page text is scaled, not grow into the whole page.
const PLOT_H_FRAC: f32 = 0.34;

/// Padding inside the plot frame for the axis labels, in text heights - that
/// side of it *is* text, so it follows the text size.
const PLOT_PAD_LEFT_EM: f32 = 3.4;
const PLOT_PAD_BOTTOM_EM: f32 = 1.6;
const PLOT_PAD_TOP_EM: f32 = 0.6;
const PLOT_PAD_RIGHT_EM: f32 = 1.2;

/// Grid divisions on each axis. Few enough that the labels never collide on a
/// phone, which is the narrowest the plot ever gets.
const X_TICKS: usize = 4;
const Y_TICKS: usize = 4;

/// Scatter dot radius and line width, as fractions of a text height, so the
/// marks stay in proportion with the labels around them.
const DOT_R_EM: f32 = 0.16;
const LINE_W_EM: f32 = 0.12;

/// One drawn series: a source, its color, and the points that had both axes.
struct Series {
    source: LogSource,
    color: egui::Color32,
    points: Vec<[f64; 2]>,
}

/// Parse a "lat, lon" entry (also accepting whitespace as the separator), the
/// same shape the desktop manual position bar takes.
fn parse_coord(text: &str) -> Option<(f64, f64)> {
    let cleaned = text.replace(',', " ");
    let mut parts = cleaned.split_whitespace();
    let lat: f64 = parts.next()?.parse().ok()?;
    let lon: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon)
    {
        return None;
    }
    Some((lat, lon))
}

/// A tick label for a value on an axis, with the decimals the range calls for:
/// a span of tens needs none, a span of hundredths needs three.
fn tick_text(value: f64, span: f64) -> String {
    let decimals = if span >= 100.0 {
        0
    } else if span >= 10.0 {
        1
    } else if span >= 1.0 {
        2
    } else {
        3
    };
    format!("{value:.decimals$}")
}

/// A time-axis tick: seconds from the start of the plotted window, as `m:ss`
/// once past a minute. Elapsed rather than wall-clock, because what a run is
/// read against is its own start; the absolute time is under the plot and in
/// the CSV.
fn time_tick_text(secs: f64) -> String {
    let secs = secs.max(0.0);
    if secs < 60.0 {
        format!("{secs:.0}s")
    } else if secs < 3600.0 {
        format!("{}:{:02}", (secs / 60.0) as u64, (secs % 60.0) as u64)
    } else {
        format!(
            "{}:{:02}:{:02}",
            (secs / 3600.0) as u64,
            ((secs % 3600.0) / 60.0) as u64,
            (secs % 60.0) as u64
        )
    }
}

impl MyApp {
    pub(crate) fn logging_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let top = self.top_inset(ctx);
        super::content_page(ctx, "logging", screen, top, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Logging");
                gap(ui, GAP_TIGHT);
                ui.label(
                    egui::RichText::new(
                        "Every report from every source, written to a CSV as it arrives: \
                         where each one was, how strongly it was heard, and how far off it \
                         is. One row per report, so a node's distance and its signal are \
                         always the same instant.",
                    )
                    .weak(),
                );

                gap(ui, GAP_BLOCK);
                self.log_controls_ui(ui, screen);
                gap(ui, GAP_SECTION);
                self.log_reference_ui(ui, screen);
                gap(ui, GAP_SECTION);
                self.log_graph_ui(ui, screen);
            });
        });
    }

    /// Arming the recorder: the file it writes to, the start/stop, what it has
    /// recorded so far, and the export off the device.
    fn log_controls_ui(&mut self, ui: &mut egui::Ui, screen: egui::Rect) {
        let recording = self.logger.is_recording();
        let path_width = field_width(ui, screen, 0.5);

        ui.strong("Log file (CSV)");
        gap(ui, GAP_HAIR);
        ui.horizontal_wrapped(|ui| {
            // Locked while recording: the open file is what the path names,
            // and letting it be retyped mid-run would leave the two disagreeing
            // about where the rows are going.
            ui.add_enabled(
                !recording,
                egui::TextEdit::singleline(&mut self.log_path)
                    .hint_text("/path/to/log.csv")
                    .desired_width(path_width),
            );
            if recording {
                if ui
                    .button("Stop")
                    .on_hover_text("Close the file; what is recorded stays on the graph")
                    .clicked()
                {
                    self.stop_log();
                }
            } else if ui
                .button("Start")
                .on_hover_text("Append to this file, creating it if it is not there")
                .clicked()
            {
                self.start_log();
            }
        });

        gap(ui, GAP_ITEM);
        // An existing file is appended to, which is what makes a stop a pause
        // rather than the end of a run - and is worth saying, since the usual
        // meaning of picking a file to write is that it gets replaced.
        ui.label(
            egui::RichText::new(
                "Starting appends to the file, so stopping and starting again continues the \
                 same log rather than replacing it.",
            )
            .weak(),
        );

        gap(ui, GAP_ITEM);
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.config.log.auto_start, "Start recording at launch");
            if ui
                .button("Save settings")
                .on_hover_text("Write the log file, reference and auto-start to the app config")
                .clicked()
            {
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
        let written = self.logger.written();
        let state = if recording {
            match self.logger.started() {
                Some(t) => format!(
                    "Recording for {} - {written} rows written",
                    age_text(std::time::SystemTime::now(), t)
                ),
                None => format!("Recording - {written} rows written"),
            }
        } else if written > 0 {
            format!("Stopped after {written} rows")
        } else {
            "Not recording".to_string()
        };
        ui.label(egui::RichText::new(state).color(if recording {
            self.config.ui.ok
        } else {
            ui.visuals().text_color()
        }));
        ui.label(format!("{rows} rows on the graph"));
        if self.logger.dropped() > 0 {
            ui.label(
                egui::RichText::new(format!(
                    "The oldest {} rows have scrolled off the graph; they are still in the file.",
                    self.logger.dropped()
                ))
                .weak(),
            );
        }

        gap(ui, GAP_ITEM);
        ui.horizontal_wrapped(|ui| {
            let hover = if self.export.is_some() {
                "Copy the CSV into the phone's Downloads folder"
            } else {
                "Write a timestamped copy of the CSV beside the log file"
            };
            if ui.button("Export a copy").on_hover_text(hover).clicked() {
                self.export_log();
            }
            if ui
                .add_enabled(rows > 0, egui::Button::new("Clear graph"))
                .on_hover_text("Empty the graph; the file on disk is untouched")
                .clicked()
            {
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
        ui.label(
            egui::RichText::new(
                "A fixed coordinate every logged position is also measured against, so a run \
                 can be read against a surveyed point rather than against a control device \
                 that is moving too. Leave it empty to log only the distance to yourself.",
            )
            .weak(),
        );
        gap(ui, GAP_TIGHT);

        let width = field_width(ui, screen, 0.45);
        let mut commit: Option<Option<(f64, f64)>> = None;
        ui.horizontal_wrapped(|ui| {
            let field = egui::TextEdit::singleline(&mut self.log_ref_text)
                .hint_text("lat, lon")
                .desired_width(width);
            let resp = ui.add(field);
            // Committed on blur or Enter, like the board nicknames: a coordinate
            // is not valid until it is fully typed, and parsing per keystroke
            // would flag every half of one as bad.
            let done = resp.lost_focus();
            if done {
                let text = self.log_ref_text.trim().to_string();
                if text.is_empty() {
                    commit = Some(None);
                } else {
                    match parse_coord(&text) {
                        Some(point) => commit = Some(Some(point)),
                        None => self.log_ref_bad = true,
                    }
                }
            }
            if ui
                .add_enabled(self.current.is_some(), egui::Button::new("Use my position"))
                .clicked()
            {
                commit = self.current.map(|p| Some((p.y(), p.x())));
            }
            // The beacon, when there is one: setting the reference to where the
            // board is standing is the usual way a range test starts.
            let beacon: Option<Position> = self.beacon;
            if ui
                .add_enabled(beacon.is_some(), egui::Button::new("Use beacon"))
                .clicked()
            {
                commit = beacon.map(|p| Some((p.y(), p.x())));
            }
            if ui
                .add_enabled(
                    self.config.log.reference().is_some(),
                    egui::Button::new("Clear"),
                )
                .clicked()
            {
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
            ui.colored_label(self.config.ui.error, "Enter a coordinate as \"lat, lon\".");
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
        let series = self.log_series();
        self.plot(ui, screen, &series);
        gap(ui, GAP_TIGHT);
        self.log_legend_ui(ui, &series);
    }

    /// The points to draw, one list per source, in source order so the legend
    /// and the colors are stable as nodes come and go.
    ///
    /// A row only makes a point when *both* axes have a value on it. That is
    /// what a scatter of signal against distance is: the rows that carry both,
    /// which for the nodes is every position report and for telemetry (no
    /// position, so no distance) is none.
    fn log_series(&self) -> Vec<Series> {
        let mut by_source: BTreeMap<LogSource, Vec<[f64; 2]>> = BTreeMap::new();
        for row in self.logger.rows() {
            if self.log_hidden.contains(&row.source) {
                continue;
            }
            let (Some(x), Some(y)) = (self.log_x.value(row), self.log_y.value(row)) else {
                continue;
            };
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            by_source.entry(row.source).or_default().push([x, y]);
        }
        by_source
            .into_iter()
            .map(|(source, points)| Series {
                source,
                color: source.color(self.config.colors),
                points,
            })
            .collect()
    }

    /// Paint the plot into a rect allocated for it.
    fn plot(&self, ui: &mut egui::Ui, screen: egui::Rect, series: &[Series]) {
        let em = em(ui);
        let height = screen.height() * PLOT_H_FRAC;
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        let frame_stroke = visuals.widgets.noninteractive.bg_stroke;
        painter.rect(
            rect,
            egui::CornerRadius::ZERO,
            visuals.extreme_bg_color,
            frame_stroke,
            egui::StrokeKind::Inside,
        );

        // The area the data itself occupies, inside the room the labels need.
        let plot = egui::Rect::from_min_max(
            rect.min + egui::vec2(em * PLOT_PAD_LEFT_EM, em * PLOT_PAD_TOP_EM),
            rect.max - egui::vec2(em * PLOT_PAD_RIGHT_EM, em * PLOT_PAD_BOTTOM_EM),
        );
        if plot.width() <= 0.0 || plot.height() <= 0.0 {
            return;
        }

        let Some((x_min, x_max, y_min, y_max)) = bounds(series) else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if self.logger.rows().is_empty() {
                    "Nothing recorded yet."
                } else {
                    "No rows carry both of these."
                },
                egui::FontId::proportional(em),
                visuals.weak_text_color(),
            );
            return;
        };

        // Time is plotted from the window's own start, so the axis reads as
        // elapsed rather than as a ten-digit epoch count.
        let x_origin = x_origin(self.log_x, series);
        let x_span = (x_max - x_min).max(f64::EPSILON);
        let y_span = (y_max - y_min).max(f64::EPSILON);
        let to_screen = |p: [f64; 2]| {
            egui::pos2(
                plot.left() + ((p[0] - x_min) / x_span) as f32 * plot.width(),
                // Screen y grows downward; the axis does not.
                plot.bottom() - ((p[1] - y_min) / y_span) as f32 * plot.height(),
            )
        };

        let grid = visuals.weak_text_color().gamma_multiply(0.35);
        let label_color = visuals.weak_text_color();
        let font = egui::FontId::proportional(em * 0.8);
        for i in 0..=X_TICKS {
            let t = i as f64 / X_TICKS as f64;
            let value = x_min + t * x_span;
            let x = plot.left() + t as f32 * plot.width();
            painter.line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                egui::Stroke::new(1.0, grid),
            );
            let text = match self.log_x {
                LogAxis::Time => time_tick_text(value - x_origin),
                LogAxis::Stat(_) => tick_text(value, x_span),
            };
            painter.text(
                egui::pos2(x, plot.bottom() + em * 0.15),
                egui::Align2::CENTER_TOP,
                text,
                font.clone(),
                label_color,
            );
        }
        for i in 0..=Y_TICKS {
            let t = i as f64 / Y_TICKS as f64;
            let value = y_min + t * y_span;
            let y = plot.bottom() - t as f32 * plot.height();
            painter.line_segment(
                [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                egui::Stroke::new(1.0, grid),
            );
            painter.text(
                egui::pos2(plot.left() - em * 0.3, y),
                egui::Align2::RIGHT_CENTER,
                tick_text(value, y_span),
                font.clone(),
                label_color,
            );
        }

        for s in series {
            let points: Vec<egui::Pos2> = s.points.iter().map(|&p| to_screen(p)).collect();
            match self.log_x {
                // Against time the rows are already in order, so the series is
                // a line: the gaps in it are as much of the story as the values.
                LogAxis::Time => {
                    if points.len() > 1 {
                        painter.add(egui::Shape::line(
                            points,
                            egui::Stroke::new(em * LINE_W_EM, s.color),
                        ));
                    } else if let Some(&p) = points.first() {
                        painter.circle_filled(p, em * DOT_R_EM, s.color);
                    }
                }
                // Against another stat the order is meaningless, so joining
                // them up would draw a shape that is not in the data.
                LogAxis::Stat(_) => {
                    for p in points {
                        painter.circle_filled(p, em * DOT_R_EM, s.color);
                    }
                }
            }
        }

        // The axis names, and the wall-clock start the elapsed axis counts from.
        let y_label = match self.log_y.unit() {
            "" => self.log_y.label().to_string(),
            unit => format!("{} ({unit})", self.log_y.label()),
        };
        painter.text(
            egui::pos2(rect.left() + em * 0.2, rect.top() + em * 0.1),
            egui::Align2::LEFT_TOP,
            y_label,
            font.clone(),
            label_color,
        );
        let x_label = match self.log_x {
            LogAxis::Time => format!("Time from {}", iso8601(unix_time(x_origin))),
            LogAxis::Stat(s) => match s.unit() {
                "" => s.label().to_string(),
                unit => format!("{} ({unit})", s.label()),
            },
        };
        painter.text(
            egui::pos2(rect.right() - em * 0.2, rect.top() + em * 0.1),
            egui::Align2::RIGHT_TOP,
            x_label,
            font,
            label_color,
        );
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
                let button = egui::Button::new(egui::RichText::new(text).color(color))
                    .selected(!hidden)
                    .small();
                if ui
                    .add(button)
                    .on_hover_text("Show or hide this source")
                    .clicked()
                {
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

/// What the X tick labels count from.
///
/// The time axis reads as elapsed, so it counts from the earliest point that
/// is actually on the plot - not from the padded edge of the frame, which sits
/// a twentieth of the span earlier and would have the first sample read as
/// minutes into a run it starts. It is also not the first recorded row: a row
/// carrying neither axis, or belonging to a hidden source, is not on the plot
/// and cannot be what the axis begins at. A stat axis reads its own values, so
/// it counts from nothing.
///
/// Only called with a non-empty series (the plot leaves early otherwise), so
/// the minimum is a real value.
fn x_origin(axis: LogAxis, series: &[Series]) -> f64 {
    match axis {
        LogAxis::Time => series
            .iter()
            .flat_map(|s| &s.points)
            .map(|p| p[0])
            .fold(f64::INFINITY, f64::min),
        LogAxis::Stat(_) => 0.0,
    }
}

/// Seconds since the epoch back into a timestamp, for the axis caption. A
/// negative can only come from a device whose clock is set before 1970, which
/// [`iso8601`] clamps in the same direction.
fn unix_time(unix_s: f64) -> std::time::SystemTime {
    std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(unix_s.max(0.0))
}

/// The data's extent on both axes, padded so a series never runs along an
/// edge, and widened when every point shares a value (a flat series still
/// needs an axis to sit in the middle of).
fn bounds(series: &[Series]) -> Option<(f64, f64, f64, f64)> {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;
    let mut any = false;
    for s in series {
        for p in &s.points {
            any = true;
            x_min = x_min.min(p[0]);
            x_max = x_max.max(p[0]);
            y_min = y_min.min(p[1]);
            y_max = y_max.max(p[1]);
        }
    }
    if !any {
        return None;
    }
    let (x_min, x_max) = pad(x_min, x_max);
    let (y_min, y_max) = pad(y_min, y_max);
    Some((x_min, x_max, y_min, y_max))
}

/// Widen a range by a twentieth at each end, or to a unit either side when it
/// has no width at all.
fn pad(min: f64, max: f64) -> (f64, f64) {
    let span = max - min;
    if span <= f64::EPSILON {
        let unit = if min.abs() > 1.0 {
            min.abs() * 0.05
        } else {
            1.0
        };
        return (min - unit, max + unit);
    }
    (min - span * 0.05, max + span * 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_parse_with_either_separator() {
        assert_eq!(parse_coord("51.4779, -0.0015"), Some((51.4779, -0.0015)));
        assert_eq!(parse_coord(" 51.4779  -0.0015 "), Some((51.4779, -0.0015)));
        assert_eq!(parse_coord("51.4779"), None);
        assert_eq!(parse_coord("51.4779, -0.0015, 3"), None);
        // Out of range is a typo, not a place.
        assert_eq!(parse_coord("91, 0"), None);
        assert_eq!(parse_coord("0, 181"), None);
        assert_eq!(parse_coord(""), None);
    }

    #[test]
    fn a_flat_series_still_gets_an_axis() {
        let (min, max) = pad(5.0, 5.0);
        assert!(min < 5.0 && max > 5.0);
        // A range with width keeps its shape, padded a little at each end.
        let (min, max) = pad(0.0, 100.0);
        assert_eq!((min, max), (-5.0, 105.0));
    }

    #[test]
    fn tick_decimals_follow_the_span() {
        assert_eq!(tick_text(1234.4, 4000.0), "1234");
        assert_eq!(tick_text(1.25, 20.0), "1.2");
        assert_eq!(tick_text(0.125, 0.5), "0.125");
    }

    #[test]
    fn time_ticks_read_as_elapsed() {
        assert_eq!(time_tick_text(0.0), "0s");
        assert_eq!(time_tick_text(45.0), "45s");
        assert_eq!(time_tick_text(90.0), "1:30");
        assert_eq!(time_tick_text(3725.0), "1:02:05");
        // A negative can only come from float error at the origin.
        assert_eq!(time_tick_text(-0.001), "0s");
    }

    /// The elapsed axis and the wall-clock caption printed beside it have to
    /// agree about when the run started, and the first sample has to read as
    /// zero seconds into it.
    ///
    /// The origin used to be the padded edge of the plot frame, which sits a
    /// twentieth of the span before any data: on an hour-long run the first
    /// sample read as three minutes in, and the caption named a time three
    /// minutes after the axis actually began.
    #[test]
    fn the_time_axis_counts_from_the_first_plotted_point() {
        let start = 1_700_000_000.0;
        let hour = 3600.0;
        let series = vec![
            Series {
                source: LogSource::Phone,
                color: egui::Color32::WHITE,
                points: vec![[start, 1.0], [start + hour, 2.0]],
            },
            // A later source must not move the origin off the earliest point.
            Series {
                source: LogSource::Node(3),
                color: egui::Color32::WHITE,
                points: vec![[start + hour / 2.0, 9.0]],
            },
        ];

        let origin = x_origin(LogAxis::Time, &series);
        assert_eq!(origin, start);
        // What the first sample reads on the axis, and the half-hour one.
        assert_eq!(time_tick_text(start - origin), "0s");
        assert_eq!(time_tick_text(start + hour / 2.0 - origin), "30:00");
        // The caption names that same instant, so the two can be added up.
        assert_eq!(iso8601(unix_time(origin)), "2023-11-14T22:13:20Z");

        // The padding that keeps the series off the frame edge is still there;
        // it just no longer moves what the labels count from.
        let (x_min, x_max, ..) = bounds(&series).unwrap();
        assert!(x_min < start && x_max > start + hour);

        // A stat axis is absolute - it reads its own values, not an offset.
        assert_eq!(x_origin(LogAxis::Stat(LogStat::Rssi), &series), 0.0);
    }

    #[test]
    fn bounds_ignore_nothing_and_cover_every_series() {
        assert!(bounds(&[]).is_none());
        let series = vec![
            Series {
                source: LogSource::Phone,
                color: egui::Color32::WHITE,
                points: vec![[0.0, 1.0], [10.0, 2.0]],
            },
            Series {
                source: LogSource::Node(3),
                color: egui::Color32::WHITE,
                points: vec![[-5.0, 9.0]],
            },
        ];
        let (x_min, x_max, y_min, y_max) = bounds(&series).unwrap();
        assert!(x_min < -5.0 && x_max > 10.0);
        assert!(y_min < 1.0 && y_max > 9.0);
    }
}
