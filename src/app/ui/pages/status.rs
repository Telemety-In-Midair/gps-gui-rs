//! The Status page: where we are, and how the board is doing.
//!
//! The position read-out is the app's own; the BLE link state comes from the
//! connection itself; the WIO/GPS/LoRa figures come from the board's telemetry
//! characteristic, and the last line from its log characteristic.

use std::time::Duration;

use midair_proto::link::{TELEM_FLAG_CFG_LOADED, TELEM_FLAG_GPS_FIX, TELEM_FLAG_SD_OK};

use crate::app::ui::text::status as text;
use crate::app::ui::theme::{gap, GAP_ITEM, GAP_SECTION};
use crate::app::ui::widgets::{content_page, heading, hint, section, status_bool};
use crate::app::{secs_text, BleIntent, MyApp};

/// How often the elapsed counts (connecting-for, board-silent-for) are
/// refreshed. They move by themselves, so a tick keeps them honest without
/// pinning the frame rate.
const ELAPSED_TICK: Duration = Duration::from_secs(1);

impl MyApp {
    pub(crate) fn status_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let safe = self.safe_area(ctx);
        let colors = self.config.ui;
        content_page(ctx, "status", screen, safe, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading!(ui, "Status");

                self.position_ui(ui);
                self.remote_nodes_ui(ui);
                self.ble_status_ui(ui);

                // The rail powering the WIO-E5 and the GPS comes up only once a
                // central connects, so an empty read-out just after connecting
                // is the board waking, not a fault.
                let warming = self.board_warming();
                let rail_off = self.rail_off();
                if rail_off {
                    gap(ui, GAP_ITEM);
                    ui.label(text::RAIL_OFF);
                } else if warming {
                    gap(ui, GAP_ITEM);
                    ui.label(text::WARMING);
                }

                let Some(t) = self.telemetry else {
                    if !warming && !rail_off {
                        gap(ui, GAP_SECTION);
                        ui.label(text::NO_TELEMETRY);
                    }
                    self.board_log_ui(ui);
                    return;
                };

                // GPS (via the WIO's MAX-M10).
                section!(ui, "GPS");
                status_bool(ui, colors, "Fix", t.flags & TELEM_FLAG_GPS_FIX != 0);
                ui.label(format!("Satellites: {}", t.sats));

                // LoRa mesh link (WIO-E5 radio).
                section!(ui, "LoRa");
                let last_rx = match t.secs_since_rx {
                    0xFFFF => "never".to_string(),
                    s => format!("{s} s ago"),
                };
                ui.label(format!("Last RX: {last_rx}"));
                if t.last_rssi != 0 {
                    ui.label(format!(
                        "RSSI: {} dBm   SNR: {:.2} dB",
                        t.last_rssi,
                        t.last_snr_cb as f32 / 100.0
                    ));
                }
                ui.label(format!("RX: {}   TX: {}", t.rx_count, t.tx_count));

                // WIO-E5 housekeeping.
                section!(ui, "WIO-E5");
                status_bool(ui, colors, "SD logging", t.flags & TELEM_FLAG_SD_OK != 0);
                status_bool(ui, colors, "Radio config", t.flags & TELEM_FLAG_CFG_LOADED != 0);

                self.board_log_ui(ui);
            });
        });
    }

    /// Where we are, how far off the beacon is, and what the beacon itself is
    /// reporting. First on the page because it is the one section that needs
    /// no board at all.
    fn position_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "Position");
        match self.current {
            Some(pos) => {
                ui.monospace(format!("{:.5}, {:.5}", pos.y(), pos.x()));
                // Names the board it is measured to: the tracked one when
                // tracking, otherwise the connected board.
                if let (Some((kind, _)), Some(m)) =
                    (self.distance_target(), self.distance_to_target())
                {
                    ui.label(format!(
                        "Distance to {}: {}",
                        self.marker_label(kind),
                        self.config.distance.units.format(m)
                    ));
                }
            }
            None => {
                ui.label(text::WAITING_FIX);
            }
        }

        // The beacon's own position, when it is streaming.
        if let (Some(b), Some(p)) = (self.beacon, self.beacon_packet) {
            gap(ui, GAP_ITEM);
            ui.monospace(format!("Beacon: {:.5}, {:.5}", b.y(), b.x()));
            ui.label(format!("Beacon speed: {:.1} m/s", p.speed_mps()));
            // Satellite count from the packet only when there is no telemetry
            // to report it below, so it is never on screen twice from two
            // different sources.
            if self.telemetry.is_none() {
                ui.label(format!("Beacon satellites: {}", p.sats));
            }
        }
    }

    /// Remote nodes heard over LoRa and relayed by the connected board, each
    /// with its last position and the signal it came in at.
    fn remote_nodes_ui(&mut self, ui: &mut egui::Ui) {
        let heard = self.remote_states();
        if heard.is_empty() {
            return;
        }
        section!(ui, "Remote nodes");
        for (addr, state, rssi) in heard {
            ui.monospace(format!(
                "{}: {state}  (rssi {rssi})",
                self.config.lora.label_of(addr),
            ));
        }
    }

    /// The BLE link to the board: whether it is up, what was asked for, and
    /// what the worker is saying about it.
    fn ble_status_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, "ESP32-C6 (BLE)");
        status_bool(ui, self.config.ui, "Link", self.ble_connected);
        ui.label(self.ble_intent_text());
        hint!(ui, "BLE: {}", self.ble_status);
        if let Some(secs) = self
            .board_settings
            .map(|s| s.sleep_interval_s)
            .filter(|_| self.ble_connected)
        {
            match secs {
                0 => hint!(ui, "Board sleep: disabled."),
                secs => hint!(ui, "Board sleep: every {} once disconnected.", secs_text(secs)),
            };
        }
        if self.ble_intent != BleIntent::Idle
            && (!self.ble_connected || self.board_silence().is_some())
        {
            ui.ctx().request_repaint_after(ELAPSED_TICK);
        }
    }

    /// The board's last log line, when it has sent one.
    fn board_log_ui(&mut self, ui: &mut egui::Ui) {
        let Some(line) = self.board_log.clone() else {
            return;
        };
        section!(ui, "Last message");
        ui.monospace(line);
    }
}
