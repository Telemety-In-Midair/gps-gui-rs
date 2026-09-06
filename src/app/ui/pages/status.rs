//! The Status page: where you are, and how the node is doing.
//!
//! Two things that are easy to confuse, kept apart on the page: the
//! position the app is using as yours (and where it came from), and what the
//! connected node reports of itself - its own fix, its link, its telemetry.

use std::time::Duration;

use midair_proto::hop::{STRATUM_GPS, STRATUM_MAX};
use midair_proto::link::{TELEM_FLAG_CFG_LOADED, TELEM_FLAG_GPS_FIX, TELEM_FLAG_SD_OK};

use crate::app::ui::text::status as text;
use crate::app::ui::theme::{gap, Key};
use crate::app::ui::widgets::{busy_dots, content_page, heading, hint, section, status_bool};
use crate::app::{BleIntent, MyApp};

/// How often the elapsed counts (connecting-for, node-silent-for) are
/// refreshed. They move by themselves, so a tick keeps them honest without
/// pinning the frame rate.
const ELAPSED_TICK: Duration = Duration::from_secs(1);

/// Where the node's hop clock comes from, in words. Stratum 0 is its own
/// GPS; the ceiling is a clock nothing has set, which is a node that has
/// not heard the network yet.
fn hop_clock_text(stratum: u8) -> String {
    match stratum {
        STRATUM_GPS => "GPS clock".to_string(),
        STRATUM_MAX => "not synced".to_string(),
        s => format!("stratum {s}"),
    }
}

impl MyApp {
    pub(crate) fn status_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let safe = self.safe_area(ctx);
        content_page(ctx, "status", screen, safe, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading!(ui, "Status");
                self.position_ui(ui);
                self.node_ui(ui);
                self.remote_nodes_ui(ui);
            });
        });
    }

    /// Your position: what the app is using, and where it came from. First
    /// on the page because it is the one section that needs no node at all.
    fn position_ui(&mut self, ui: &mut egui::Ui) {
        section!(ui, self.phone_label());
        match self.current {
            Some(pos) => {
                ui.monospace(format!("{:.5}, {:.5}", pos.y(), pos.x()));
                if let Some(source) = self.location_source() {
                    hint!(ui, text::source(source));
                }
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
                // Names the node it is measured to: the tracked one when
                // tracking, otherwise the connected node.
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
            None if !self.config.phone.location && !self.config.ble.location => {
                hint!(ui, text::NO_SOURCE);
            }
            None => {
                ui.label(text::WAITING_FIX);
            }
        }
    }

    /// The connected node: its link, its own fix, and its telemetry. One
    /// section under its name, so nothing here can be read as yours.
    fn node_ui(&mut self, ui: &mut egui::Ui) {
        let colors = self.config.ui;
        section!(ui, sep format!("Node: {}", self.selected_device_label()));
        status_bool(ui, colors, "Link", self.ble_connected);
        if self.ble_busy() {
            ui.colored_label(
                colors.busy,
                format!("{}{}", self.ble_intent_text(), busy_dots(ui.ctx())),
            );
        } else {
            ui.label(self.ble_intent_text());
        }
        if let Some(s) = self.board_settings.filter(|_| self.ble_connected) {
            ui.label(format!("Mode: {}", s.mode.as_str()));
        }
        if self.ble_intent != BleIntent::Idle
            && (!self.ble_connected || self.board_silence().is_some())
        {
            ui.ctx().request_repaint_after(ELAPSED_TICK);
        }

        // The node's own position, whether or not it is also yours: this is
        // what the node measured, under its own heading.
        if let (Some(b), Some(p)) = (self.beacon, self.beacon_packet) {
            gap(ui, Key::GapItem);
            ui.monospace(format!("{:.5}, {:.5}", b.y(), b.x()));
            ui.label(format!(
                "Speed: {:.1} m/s   Satellites: {}",
                p.speed_mps(),
                p.sats
            ));
        }

        // A node that has just connected may still be on its first fix, so
        // an empty read-out at that point is the GPS working, not a fault.
        let warming = self.board_warming();
        if warming {
            gap(ui, Key::GapItem);
            ui.label(text::WARMING);
        }
        let Some(t) = self.telemetry else {
            if !warming && self.ble_connected {
                gap(ui, Key::GapItem);
                hint!(ui, text::NO_TELEMETRY);
            }
            self.board_log_ui(ui);
            return;
        };

        gap(ui, Key::GapItem);
        ui.strong("GPS");
        status_bool(ui, colors, "Fix", t.flags & TELEM_FLAG_GPS_FIX != 0);
        ui.label(format!("Satellites: {}", t.sats));

        gap(ui, Key::GapItem);
        ui.strong("LoRa");
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

        gap(ui, Key::GapItem);
        ui.strong("Card and config");
        status_bool(ui, colors, "SD logging", t.flags & TELEM_FLAG_SD_OK != 0);
        status_bool(
            ui,
            colors,
            "Radio config",
            t.flags & TELEM_FLAG_CFG_LOADED != 0,
        );

        self.board_log_ui(ui);
    }

    /// Remote nodes heard over LoRa and relayed by the connected node, each
    /// with its last position and the signal it came in at.
    fn remote_nodes_ui(&mut self, ui: &mut egui::Ui) {
        let heard = self.remote_states();
        if heard.is_empty() {
            return;
        }
        section!(ui, sep "Remote nodes");
        for (addr, state, rssi) in heard {
            ui.monospace(format!(
                "{}: {state}  (rssi {rssi})",
                self.config.lora.label_of(addr),
            ));
        }
    }

    /// The node's last log line, when it has sent one.
    fn board_log_ui(&mut self, ui: &mut egui::Ui) {
        let Some(line) = self.board_log.clone() else {
            return;
        };
        gap(ui, Key::GapItem);
        ui.strong("Last message");
        ui.monospace(line);
    }
}
