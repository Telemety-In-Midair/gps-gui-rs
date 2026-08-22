//! The Logging page's graph, painted by hand.
//!
//! What it has to do is narrow - a handful of series, autoscaled, in the
//! config's own colors, at a size measured off the screen like every other
//! page here - and a plotting crate that did it would arrive with its own
//! sizing, theming and gesture handling to fight.
//!
//! Nothing here reaches into the app: [`Series::collect`] turns log rows into
//! points and [`draw`] paints them, so the page above decides what to plot and
//! this decides how it looks.

use std::collections::BTreeMap;

use crate::config::MarkerColors;
use crate::logging::{iso8601, LogAxis, LogRow, LogSource, LogStat};

use super::text::logging as text;
use super::theme::em;

/// Plot height as a fraction of the screen height. Written against the screen
/// rather than the text because it is a picture: it should keep its shape when
/// the page text is scaled, not grow into the whole page.
const PLOT_H_FRAC: f32 = 0.34;

/// Padding inside the plot frame for the axis labels, in text heights - that
/// side of it *is* text, so it follows the text size.
const PAD_LEFT_EM: f32 = 3.4;
const PAD_BOTTOM_EM: f32 = 1.6;
const PAD_TOP_EM: f32 = 0.6;
const PAD_RIGHT_EM: f32 = 1.2;

/// Grid divisions on each axis. Few enough that the labels never collide on a
/// phone, which is the narrowest the plot ever gets.
const X_TICKS: usize = 4;
const Y_TICKS: usize = 4;

/// Scatter dot radius and line width, as fractions of a text height, so the
/// marks stay in proportion with the labels around them.
const DOT_R_EM: f32 = 0.16;
const LINE_W_EM: f32 = 0.12;

/// One drawn series: a source, its color, and the points that had both axes.
pub(super) struct Series {
    pub(super) source: LogSource,
    pub(super) color: egui::Color32,
    pub(super) points: Vec<[f64; 2]>,
}

impl Series {
    /// The points to draw, one list per source, in source order so the legend
    /// and the colors are stable as nodes come and go.
    ///
    /// A row only makes a point when *both* axes have a value on it. That is
    /// what a scatter of signal against distance is: the rows that carry both,
    /// which for the nodes is every position report and for telemetry (no
    /// position, so no distance) is none.
    pub(super) fn collect(
        rows: &[LogRow],
        x_axis: LogAxis,
        y_axis: LogStat,
        hidden: &std::collections::BTreeSet<LogSource>,
        colors: MarkerColors,
    ) -> Vec<Series> {
        let mut by_source: BTreeMap<LogSource, Vec<[f64; 2]>> = BTreeMap::new();
        for row in rows {
            if hidden.contains(&row.source) {
                continue;
            }
            let (Some(x), Some(y)) = (x_axis.value(row), y_axis.value(row)) else {
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
                color: source.color(colors),
                points,
            })
            .collect()
    }
}

/// Paint the plot into a rect allocated for it.
///
/// `anything_recorded` decides which of the two empty messages is shown: no
/// rows at all is a different state from rows that carry only one of the two
/// axes, and only saying so tells the reader whether to wait or to pick
/// different axes.
pub(super) fn draw(
    ui: &mut egui::Ui,
    screen: egui::Rect,
    x_axis: LogAxis,
    y_axis: LogStat,
    series: &[Series],
    anything_recorded: bool,
) {
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
        rect.min + egui::vec2(em * PAD_LEFT_EM, em * PAD_TOP_EM),
        rect.max - egui::vec2(em * PAD_RIGHT_EM, em * PAD_BOTTOM_EM),
    );
    if plot.width() <= 0.0 || plot.height() <= 0.0 {
        return;
    }

    let Some((x_min, x_max, y_min, y_max)) = bounds(series) else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            if anything_recorded {
                text::PLOT_NO_PAIRS
            } else {
                text::PLOT_EMPTY
            },
            egui::FontId::proportional(em),
            visuals.weak_text_color(),
        );
        return;
    };

    // Time is plotted from the window's own start, so the axis reads as
    // elapsed rather than as a ten-digit epoch count.
    let x_origin = x_origin(x_axis, series);
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
        let text = match x_axis {
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
        match x_axis {
            // Against time the rows are already in order, so the series is a
            // line: the gaps in it are as much of the story as the values.
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
            // Against another stat the order is meaningless, so joining them
            // up would draw a shape that is not in the data.
            LogAxis::Stat(_) => {
                for p in points {
                    painter.circle_filled(p, em * DOT_R_EM, s.color);
                }
            }
        }
    }

    // The axis names, and the wall-clock start the elapsed axis counts from.
    painter.text(
        egui::pos2(rect.left() + em * 0.2, rect.top() + em * 0.1),
        egui::Align2::LEFT_TOP,
        axis_caption(y_axis),
        font.clone(),
        label_color,
    );
    let x_label = match x_axis {
        LogAxis::Time => format!("Time from {}", iso8601(unix_time(x_origin))),
        LogAxis::Stat(s) => axis_caption(s),
    };
    painter.text(
        egui::pos2(rect.right() - em * 0.2, rect.top() + em * 0.1),
        egui::Align2::RIGHT_TOP,
        x_label,
        font,
        label_color,
    );
}

/// An axis name with its unit, where it has one.
fn axis_caption(stat: LogStat) -> String {
    match stat.unit() {
        "" => stat.label().to_string(),
        unit => format!("{} ({unit})", stat.label()),
    }
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
