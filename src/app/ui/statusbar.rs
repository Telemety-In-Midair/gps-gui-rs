//! The map's bottom status bar: a bar graph of the last few receptions, and
//! beside it the read-out for one remote node at a time.
//!
//! It answers the question a map cannot: not where a node is, but whether it
//! is still being heard, how well, and how recently. The graph is the whole
//! mesh at once - one bar per reception, colored by the node it came from -
//! and the read-out is one node in detail, cycling when several are on the
//! air.
//!
//! Off unless `[status_bar] show` is set: it covers a strip of the map with a
//! read-out that only means anything while a node is being heard.

use std::time::{Duration, SystemTime};

use crate::app::ui::text::statusbar as text;
use crate::app::ui::theme::{bar_margin, em};
use crate::app::{MyApp, NodeStatus, RssiSample, RSSI_HISTORY};
use crate::config::{remote_color, STATUS_CYCLE_MIN};
use crate::points::age_text;

/// The graph's size: a fraction of the screen width, and a height in text
/// heights so it stays in proportion with the read-out beside it.
const GRAPH_W_FRAC: f32 = 0.28;
const GRAPH_H_EM: f32 = 1.7;

/// The dBm range the bars are drawn against.
///
/// Fixed rather than autoscaled over the ten samples. A bar's height has to
/// mean the same thing from one frame to the next for a glance at it to be
/// worth anything, and an autoscale turns a few dB of noise on a steady link
/// into a full-height swing. The ends are the usable span of a LoRa receiver:
/// below the floor is under any sensitivity worth plotting, above the ceiling
/// is a node in the same room.
const RSSI_FLOOR_DBM: f32 = -130.0;
const RSSI_CEIL_DBM: f32 = -30.0;

/// Space between two bars, as a fraction of the slot each one gets.
const BAR_GAP_FRAC: f32 = 0.25;

/// The shortest a bar may be drawn, as a fraction of the graph height. A
/// reception at the very floor still gets a sliver, so a barely-heard node
/// reads as one that was heard rather than as an empty slot.
const BAR_MIN_FRAC: f32 = 0.05;

/// How often the bar is redrawn while it is up, so the "heard N ago" count
/// stays honest without pinning the frame rate.
const AGE_TICK: Duration = Duration::from_secs(1);

/// Which node the read-out is on, and how long is left of its turn.
///
/// Derived from the clock rather than stored: the cycle is a function of the
/// time and the list, so nothing has to be kept in step with a node appearing
/// mid-cycle or with the dwell being changed while it runs.
///
/// `now` is seconds since the app started. A lone node keeps the read-out to
/// itself and never rotates, which is what the `None` says.
fn cycle_slot(now: f64, dwell: f32, count: usize) -> (usize, Option<f32>) {
    if count <= 1 {
        return (0, None);
    }
    // The loader holds the setting to this range, but the bar is drawn from
    // the live config, which a half-dragged control can carry through zero.
    let dwell = f64::from(dwell.max(STATUS_CYCLE_MIN));
    let index = (now / dwell) as usize % count;
    (index, Some((dwell - now % dwell) as f32))
}

/// Paint the receptions as bars, oldest at the left, each in its node's color.
///
/// The slots are always [`RSSI_HISTORY`] wide whatever has arrived, so the
/// bars fill the graph from the left and then slide through it rather than
/// re-spacing themselves on every reception.
fn rssi_graph(ui: &mut egui::Ui, size: egui::Vec2, samples: &[RssiSample]) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let visuals = ui.visuals();
    let painter = ui.painter_at(rect);
    painter.rect(
        rect,
        egui::CornerRadius::ZERO,
        visuals.extreme_bg_color,
        visuals.widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let slot = rect.width() / RSSI_HISTORY as f32;
    let gap = slot * BAR_GAP_FRAC;
    let span = RSSI_CEIL_DBM - RSSI_FLOOR_DBM;
    for (i, sample) in samples.iter().take(RSSI_HISTORY).enumerate() {
        let frac = ((f32::from(sample.dbm) - RSSI_FLOOR_DBM) / span).clamp(0.0, 1.0);
        let height = (rect.height() * frac).max(rect.height() * BAR_MIN_FRAC);
        let left = rect.left() + slot * i as f32 + gap / 2.0;
        let bar = egui::Rect::from_min_max(
            egui::pos2(left, rect.bottom() - height),
            egui::pos2(left + slot - gap, rect.bottom()),
        );
        painter.rect_filled(bar, egui::CornerRadius::ZERO, remote_color(sample.addr));
    }
}

impl MyApp {
    /// The status bar along the bottom of the map, when it is turned on.
    ///
    /// Records its own height in [`MyApp::status_bar_height`] so the other
    /// bottom-anchored overlay - the desktop manual position bar - can sit
    /// above it rather than under it. Measured after the fact because the
    /// read-out wraps: how tall it is depends on the text size and what is
    /// being reported.
    pub(crate) fn map_status_bar(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        if !self.config.status_bar.show {
            self.status_bar_height = 0.0;
            return;
        }
        let bottom = self.bottom_inset(ctx);
        let (margin_x, margin_y) = bar_margin(screen);
        let area = egui::Area::new(egui::Id::new("map_status_bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(screen.left(), screen.bottom()))
            .pivot(egui::Align2::LEFT_BOTTOM)
            .movable(false)
            .constrain(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(egui::Margin {
                        left: margin_x,
                        right: margin_x,
                        top: margin_y,
                        // The gesture bar is part of the frame rather than
                        // space under it, so the fill reaches the screen edge
                        // and the read-out still clears the inset.
                        bottom: margin_y.saturating_add(bottom as i8),
                    })
                    .show(ui, |ui| {
                        ui.set_width(screen.width() - 2.0 * f32::from(margin_x));
                        self.status_bar_row(ui, screen);
                    });
            });
        self.status_bar_height = area.response.rect.height();
    }

    /// The bar's one row: the graph, then the node the cycle is currently on.
    fn status_bar_row(&self, ui: &mut egui::Ui, screen: egui::Rect) {
        let em = em(ui);
        let samples = self.rssi_samples();
        let nodes = self.status_bar_nodes();
        let (slot, remaining) = cycle_slot(
            ui.input(|i| i.time),
            self.config.status_bar.cycle_secs,
            nodes.len(),
        );

        ui.horizontal_wrapped(|ui| {
            let width = (screen.width() * GRAPH_W_FRAC).max(em);
            rssi_graph(ui, egui::vec2(width, em * GRAPH_H_EM), &samples);
            match nodes
                .get(slot)
                .and_then(|&addr| self.status_bar_node(addr).map(|status| (addr, status)))
            {
                Some((addr, status)) => self.status_bar_readout(ui, addr, status),
                // Either no node has been heard yet, or the connected board
                // was just switched and what the last one heard went with it.
                None => {
                    ui.label(egui::RichText::new(text::NO_NODES).weak());
                }
            }
        });

        // The age counts up by itself, and the cycle moves on by itself; both
        // need a frame at the right moment rather than a running frame rate.
        let next = match remaining {
            Some(secs) => AGE_TICK.min(Duration::from_secs_f32(secs.max(0.0))),
            None => AGE_TICK,
        };
        ui.ctx().request_repaint_after(next);
    }

    /// One node's figures: who it is, how strongly and how long ago it was
    /// heard, what its receiver can see, and how fast it is going.
    ///
    /// The name carries the node's map color, so a bar in the graph can be
    /// matched to the node being read out without a legend.
    fn status_bar_readout(&self, ui: &mut egui::Ui, addr: u8, status: NodeStatus) {
        let colors = self.config.ui;
        ui.colored_label(remote_color(addr), self.config.lora.label_of(addr));
        ui.label(format!("{} dBm", status.rssi));
        ui.label(format!("{} ago", age_text(SystemTime::now(), status.heard)));
        // Red on no fix, green with one: the count means nothing on its own -
        // a node can see four satellites and still not have solved a position.
        let sats = if status.fix { colors.ok } else { colors.error };
        ui.colored_label(sats, format!("{} sat", status.sats));
        ui.label(format!("{:.1} m/s", status.speed_mps));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_node_keeps_the_readout_and_never_rotates() {
        assert!(matches!(cycle_slot(0.0, 5.0, 1), (0, None)));
        assert!(matches!(cycle_slot(1_000.0, 5.0, 1), (0, None)));
        // No node at all still answers with a slot, which the caller finds
        // nothing at rather than having to test the count itself.
        assert!(matches!(cycle_slot(1_000.0, 5.0, 0), (0, None)));
    }

    #[test]
    fn the_cycle_walks_the_nodes_and_wraps() {
        let slots: Vec<usize> = (0..7)
            .map(|i| cycle_slot(f64::from(i) * 5.0, 5.0, 3).0)
            .collect();
        assert_eq!(slots, vec![0, 1, 2, 0, 1, 2, 0]);
    }

    #[test]
    fn the_time_left_is_what_is_left_of_this_nodes_turn() {
        let (slot, remaining) = cycle_slot(6.0, 5.0, 2);
        assert_eq!(slot, 1);
        assert!((remaining.unwrap() - 4.0).abs() < 1e-3);
    }

    #[test]
    fn a_dwell_dragged_to_zero_does_not_divide_by_it() {
        // The config loader will not accept a zero, but the bar draws from the
        // live config, which a control being dragged passes through.
        let (slot, remaining) = cycle_slot(3.0, 0.0, 2);
        assert!(slot < 2);
        assert!(remaining.is_some_and(f32::is_finite));
    }
}
