//! The Status page: where we are, and how the board is doing.
//!
//! The position read-out is the app's own; the BLE link state comes from the
//! connection itself; the GPS/LoRa figures come from the board's telemetry
//! characteristic, and the last line from its log characteristic.

use std::time::Duration;

use midair_proto::hop::{STRATUM_GPS, STRATUM_MAX};
use midair_proto::link::{TELEM_FLAG_CFG_LOADED, TELEM_FLAG_GPS_FIX, TELEM_FLAG_SD_OK};

use crate::app::ui::text::status as text;
use crate::app::ui::theme::{gap, Key};
use crate::app::ui::widgets::{content_page, heading, hint, section, status_bool};
use crate::app::{secs_text, BleIntent, MyApp};

/// How often the elapsed counts (connecting-for, board-silent-for) are
/// refreshed. They move by themselves, so a tick keeps them honest without
/// pinning the frame rate.
const ELAPSED_TICK: Duration = Duration::from_secs(1);

/// Where the board's hop clock comes from, in words. Stratum 0 is its own
/// GPS; the ceiling is a clock nothing has set, which is a board that has
/// not heard the network yet.
fn hop_clock_text(stratum: u8) -> String {
    match stratum {
        STRATUM_GPS => "clock from the board's GPS".to_string(),
        STRATUM_MAX => "not synced yet (own clock)".to_string(),
        s => format!("synced, stratum {s}"),
    }
}

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

                // A board that has just connected may still be on its first
                // fix, so an empty read-out at that point is the GPS working,
                // not a fault.
                let warming = self.board_warming();
                if warming {
                    gap(ui, Key::GapItem);
                    ui.label(text::WARMING);
                }

                let Some(t) = self.telemetry else {
                    if !warming {
                        gap(ui, Key::GapSection);
                        ui.label(text::NO_TELEMETRY);
                    }
                    self.board_log_ui(ui);
                    return;
                };

                // GPS (the board's own MAX-M10).
                section!(ui, "GPS");
                status_bool(ui, colors, "Fix", t.flags & TELEM_FLAG_GPS_FIX != 0);
                ui.label(format!("Satellites: {}", t.sats));

                // LoRa mesh link (the board's SX1262).
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
                if let Some(stratum) = t.hop_stratum() {
                    ui.label(format!(
                        "Hopping: channel {}, {}",
                        t.hop_channel,
                        hop_clock_text(stratum)
                    ));
                }

                // Board housekeeping.
                section!(ui, "Board");
                status_bool(ui, colors, "SD logging", t.flags & TELEM_FLAG_SD_OK != 0);
                status_bool(
                    ui,
                    colors,
                    "Radio config",
                    t.flags & TELEM_FLAG_CFG_LOADED != 0,
                );

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
                // Velocity, from the fix rather than the compass: the compass
                // says where the device is pointed, which is not where it is
                // going. A receiver reports neither while stationary and a
                // hand-entered position reports neither ever, so the line is
                // drawn only once there is a measurement behind it.
                if let Some(speed) = self.speed {
                    let course = match self.heading {
                        Some(deg) => format!("   Course: {deg:.0} deg"),
                        None => String::new(),
                    };
                    ui.label(format!("Speed: {speed:.1} m/s{course}"));
                }
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

        // The board's own position, when it is streaming, under the name the
        // map uses for it. Read whether or not the map draws the board: this
        // page is where a board taken off the map is still accounted for.
        if let (Some(b), Some(p)) = (self.beacon, self.beacon_packet) {
            let name = self.beacon_label();
            gap(ui, Key::GapItem);
            ui.monospace(format!("{name}: {:.5}, {:.5}", b.y(), b.x()));
            ui.label(format!("{name} speed: {:.1} m/s", p.speed_mps()));
            // Satellite count from the packet only when there is no telemetry
            // to report it below, so it is never on screen twice from two
            // different sources.
            if self.telemetry.is_none() {
                ui.label(format!("{name} satellites: {}", p.sats));
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
        section!(ui, "Wio-S3 (BLE)");
        status_bool(ui, self.config.ui, "Link", self.ble_connected);
        // Which board, by the same name the Beacon page and the map use.
        ui.label(format!("Board: {}", self.selected_device_label()));
        ui.label(self.ble_intent_text());
        hint!(ui, "BLE: {}", self.ble_status);
        if let Some(secs) = self
            .board_settings
            .map(|s| s.sleep_interval_s)
            .filter(|_| self.ble_connected)
        {
            match secs {
                0 => hint!(ui, "Board sleep: disabled."),
                secs => hint!(
                    ui,
                    "Board sleep: every {} once disconnected.",
                    secs_text(secs)
                ),
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
