//! The Bluetooth page: the BLE link to a node, the app-side settings that
//! decide how it connects, and the node's own power and sleep settings.
//!
//! Split from [`MyApp::settings_page`] by who owns each setting. The two
//! groups here read alike but are not: the connection settings are the app's,
//! saved to its settings file with the button beside them, while everything
//! under "Node power and sleep" lives in the node's flash and is only ever
//! reported by the node (see [`MyApp::board_power_ui`]).

use std::time::Duration;

use midair_proto::ble;

use crate::app::ui::text::bluetooth as text;
use crate::app::ui::theme::{gap, Key};
use crate::app::ui::widgets::{
    busy_dots, button, check, content_page, feedback_label, heading, hint, preset_text, row,
    scroll_body, secs_presets, section, text_field,
};
use crate::app::{secs_text, BleIntent, MyApp};
use crate::ble::ConfigWrite;

/// How often the elapsed counts refresh, and how often a running scan's signal
/// readings do. Both move by themselves; the scan is the faster of the two
/// because a node answering is the thing being waited for.
const ELAPSED_TICK: Duration = Duration::from_secs(1);
const SCAN_TICK: Duration = Duration::from_millis(500);

/// The presets each number box offers. Every list is a spread over the
/// range the node clamps to, so a preset is never a value the node will
/// quietly change.
const NOTIFY_MS: [u32; 5] = [250, 500, 1000, 2000, 5000];
const WAKE_CHECK_S: [u32; 5] = [5, 30, 60, 120, 300];
const ADV_WINDOW_S: [u32; 5] = [1, 5, 15, 30, 60];
const BLE_ON_S: [u32; 4] = [5, 15, 30, 60];
const BLE_OFF_S: [u32; 5] = [5, 30, 60, 120, 300];
const IDLE_TIMEOUT_S: [u32; 5] = [60, 300, 600, 1800, 3600];
const SLEEP_NOW_S: [u32; 4] = [5, 30, 60, 300];

impl MyApp {
    pub(crate) fn bluetooth_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let safe = self.safe_area(ctx);
        content_page(ctx, "bluetooth", screen, safe, |ui| {
            scroll_body(ui, |ui| {
                heading!(ui, "Bluetooth");

                // Which node first, then what to do about the link to it.
                section!(ui, "Node");
                gap(ui, Key::GapTight);
                self.device_picker_ui(ui);

                section!(ui, sep "Link");
                gap(ui, Key::GapTight);
                self.ble_link_ui(ui);

                section!(ui, sep "Connection");
                gap(ui, Key::GapTight);
                self.connection_ui(ui);

                section!(ui, sep "Node power and sleep");
                gap(ui, Key::GapItem);
                self.board_power_ui(ui);

                // Last: the one thing here that is neither the link nor a
                // power setting, and the least often changed.
                section!(ui, sep "Node name");
                gap(ui, Key::GapTight);
                self.board_name_ui(ui);
            });
        });
    }

    /// Pick which node to talk to. Only one is connected at a time, so this is
    /// a single-choice list rather than a set of toggles.
    ///
    /// A node is told apart by its address; the name box on each row is the
    /// app's own, kept in the settings file, and a node that carries a name
    /// of its own overwrites it. The address-derived name an unnamed node
    /// advertises under is not shown: it says nothing the address beside it
    /// does not.
    fn device_picker_ui(&mut self, ui: &mut egui::Ui) {
        let scanning = self.ble_intent == BleIntent::Scanning;
        ui.horizontal_wrapped(|ui| {
            let scan = button!(
                ui,
                if scanning { "Stop scanning" } else { "Scan for nodes" },
                hover: if scanning { text::SCAN_STOP_HOVER } else { text::SCAN_START_HOVER },
            );
            if scan.clicked() {
                self.set_ble_intent(if scanning {
                    BleIntent::Idle
                } else {
                    BleIntent::Scanning
                });
            }
            if scanning {
                ui.colored_label(
                    self.config.ui.busy,
                    format!("{}{}", text::SCANNING, busy_dots(ui.ctx())),
                );
            }
        });
        gap(ui, Key::GapTight);

        let rows = self.device_rows();
        // Named nodes are remembered, so an empty list means nothing has ever
        // been named and nothing is on the air right now.
        if rows.is_empty() {
            hint!(ui, text::NO_BOARDS);
        }

        let any_selected = self.config.ble.mac.is_none();
        if ui
            .radio(any_selected, "Any node")
            .on_hover_text(text::ANY_BOARD_HOVER)
            .clicked()
            && !any_selected
        {
            self.select_device(None);
        }

        for device in rows {
            ui.horizontal_wrapped(|ui| {
                if ui.radio(device.selected, "").clicked() && !device.selected {
                    self.select_device(Some(&device.mac));
                }
                // Committing on blur rather than per keystroke: an empty name
                // forgets the node, and that must not happen mid-edit just
                // because the box was cleared before retyping.
                let name = self.name_edit(&device.mac);
                let typed = text_field(ui, name, "name this node", Key::BluetoothName)
                    .on_hover_text(text::NAME_HOVER);
                if typed.lost_focus() {
                    self.commit_name(&device.mac);
                }
                // A name stored on the node is what every page calls it,
                // over the box to its left, so it is drawn as a label.
                if let Some(own) = &device.own_name {
                    ui.label(own.as_str());
                }
                hint!(ui, device.mac.as_str());
                match device.rssi {
                    // Only a running scan measures this, so its absence during
                    // a scan is the useful signal: that node is not answering.
                    Some(rssi) => {
                        ui.colored_label(self.config.ui.ok, format!("{rssi} dBm"));
                    }
                    None if scanning => {
                        hint!(ui, "not answering");
                    }
                    None => {}
                };
            });
        }

        // The signal readings move by themselves while a scan runs.
        if scanning {
            ui.ctx().request_repaint_after(SCAN_TICK);
        }
    }

    /// The link controls: one button per thing you can ask for, plus what the
    /// app and the worker each say is happening.
    ///
    /// Three buttons rather than a toggle because the three requests are
    /// genuinely different - one of them (Disconnect) is the only way to let
    /// the node sleep. Every one of them takes effect on the press. Connect
    /// stays live while connected because pressing it then is a real request
    /// - start over from a scan - and it is the way out of a link that is up
    /// but not working.
    fn ble_link_ui(&mut self, ui: &mut egui::Ui) {
        let connected = self.ble_connected;
        let idle = self.ble_intent == BleIntent::Idle;
        ui.horizontal_wrapped(|ui| {
            let connect = button!(
                ui,
                if connected { "Reconnect" } else { "Connect" },
                hover: if connected { text::RECONNECT_HOVER } else { text::CONNECT_HOVER },
            );
            if connect.clicked() {
                self.set_ble_intent(BleIntent::Connect);
            }
            if button!(ui, "Connect to sleeping", hover: text::CONNECT_SLEEPING_HOVER).clicked() {
                self.set_ble_intent(BleIntent::ConnectSleeping);
            }
            let stop = button!(
                ui,
                "Disconnect",
                enabled: !idle,
                hover: text::DISCONNECT_HOVER,
            );
            if stop.clicked() {
                self.set_ble_intent(BleIntent::Idle);
            }
        });

        gap(ui, Key::GapTight);
        if connected {
            // Which node the link is to: pinned to "any node", the picker
            // above cannot say.
            ui.label(format!("Node: {}", self.selected_device_label()));
            // A link that has gone quiet is not shown in the all-well color:
            // the text is doubting the connection, so the color must not vouch
            // for it. Its elapsed count also needs the one-second tick.
            if self.board_silence().is_some() {
                ui.label(self.ble_intent_text());
                ui.ctx().request_repaint_after(ELAPSED_TICK);
            } else {
                ui.colored_label(self.config.ui.ok, self.ble_intent_text());
            }
        } else if idle {
            hint!(ui, self.ble_intent_text());
        } else {
            // In progress: its own color, and dots that move so a count
            // that is not changing still reads as alive.
            ui.colored_label(
                self.config.ui.busy,
                format!("{}{}", self.ble_intent_text(), busy_dots(ui.ctx())),
            );
        }
        // The worker's own commentary: scanning, connecting, why it retried.
        // Distinct from the line above, which is what was *asked* for.
        hint!(ui, "BLE: {}", self.ble_status);
        // The elapsed count is the only thing here that moves by itself; a
        // one-second tick keeps it honest without pinning the frame rate.
        if !connected && !idle {
            ui.ctx().request_repaint_after(ELAPSED_TICK);
        }
    }

    /// The app's own connection settings, and the node's notify interval.
    ///
    /// The node names, the selected node and the auto-connect checkbox are
    /// the app's settings, not the node's, so they need the same Save the
    /// Settings page has rather than a trip back to it. Same file, same
    /// feedback line.
    fn connection_ui(&mut self, ui: &mut egui::Ui) {
        check!(
            ui,
            self.config.ble.enabled,
            "Connect at launch",
            hover: text::AUTO_CONNECT_HOVER,
        );
        gap(ui, Key::GapTight);
        if button!(ui, "Save settings", hover: text::SAVE_HOVER).clicked() {
            self.save_config();
        }
        feedback_label(ui, self.config.ui, &self.config_feedback);

        gap(ui, Key::GapItem);
        let ready = self.ble_connected && !self.ble_ack_pending;
        let presets: Vec<(String, String)> = NOTIFY_MS
            .iter()
            .map(|ms| (ms.to_string(), format!("{ms} ms")))
            .collect();
        row(ui, "Notify interval:", |ui| {
            preset_text(
                ui,
                "notify_interval",
                &mut self.ble_interval_text,
                &presets,
                Key::BluetoothNumber,
            );
            if button!(ui, "Apply", enabled: ready).clicked() {
                self.apply_notify_interval();
            }
        });
        self.ack_line_ui(ui);
    }

    /// The last config write's answer, or that one is still on its way.
    fn ack_line_ui(&self, ui: &mut egui::Ui) {
        if self.ble_ack.is_none() && self.ble_ack_pending {
            ui.colored_label(
                self.config.ui.busy,
                format!("{}{}", text::AWAITING_ACK, busy_dots(ui.ctx())),
            );
        } else {
            feedback_label(ui, self.config.ui, &self.ble_ack);
        }
    }

    /// The name stored on the node.
    ///
    /// It rides the same config characteristic and the same one-write-at-a-time
    /// rule as the power settings, but it is read off its own characteristic,
    /// so a node whose settings blob is too new to decode can still be named.
    /// The line under the box is the node's own report of what it is called,
    /// which is the confirmation that matters - the ack only says a length
    /// was stored.
    fn board_name_ui(&mut self, ui: &mut egui::Ui) {
        if !self.ble_connected {
            ui.label(text::BOARD_NEED_LINK);
            return;
        }
        let busy = self.ble_ack_pending;
        let named = self.board_own_name().is_some();
        row(ui, "Name:", |ui| {
            text_field(
                ui,
                &mut self.board_name_text,
                "name this node",
                Key::BluetoothName,
            )
            .on_hover_text(text::board_name_hover(ble::NAME_LABEL_MAX));
            if button!(ui, "Apply", enabled: !busy).clicked() {
                self.apply_board_name();
            }
            let clear = button!(
                ui,
                "Clear",
                enabled: !busy && named,
                hover: text::NAME_CLEAR_HOVER,
            );
            if clear.clicked() {
                self.clear_board_name();
            }
        });
        ui.label(match (self.board_own_name(), &self.board_name) {
            (Some(label), _) => format!("Called {label}."),
            (None, Some(advertised)) => format!("Unnamed ({advertised})."),
            (None, None) => text::BOARD_NO_NAME.to_string(),
        });
        self.ack_line_ui(ui);
    }

    /// The node's mode, its sleep switches and its intervals.
    ///
    /// Every control reads the node's own settings blob rather than a local
    /// copy: the node is the authority, and it changes these by itself
    /// (clamping an interval). A control therefore only moves once the node
    /// reports that it moved.
    fn board_power_ui(&mut self, ui: &mut egui::Ui) {
        if !self.ble_connected {
            ui.label(text::BOARD_NEED_LINK);
            return;
        }
        if self.settings_unsupported {
            ui.colored_label(self.config.ui.error, text::BOARD_TOO_NEW);
            return;
        }
        let Some(s) = self.board_settings else {
            ui.colored_label(
                self.config.ui.busy,
                format!("{}{}", text::BOARD_READING, busy_dots(ui.ctx())),
            );
            return;
        };

        // One write at a time: while an ack is outstanding the node has not
        // yet said what it applied, and these controls show only what it has.
        let busy = self.ble_ack_pending;

        // The mode comes first because it decides which of the settings
        // below the node even reads. Selection is drawn from the node's
        // reported mode rather than from the last button pressed, like
        // every other control here - so a press that the node refuses, or
        // that has not landed yet, leaves the highlight where it was.
        ui.strong("Mode");
        let mut pick = None;
        ui.horizontal_wrapped(|ui| {
            for (mode, label, hover) in [
                (ble::Mode::Stored, "Stored", text::MODE_STORED_HOVER),
                (ble::Mode::Idle, "Idle", text::MODE_IDLE_HOVER),
                (ble::Mode::Tracking, "Tracking", text::MODE_TRACKING_HOVER),
                (ble::Mode::Listening, "Listening", text::MODE_LISTENING_HOVER),
            ] {
                let resp = ui
                    .add_enabled_ui(!busy, |ui| ui.selectable_label(s.mode == mode, label))
                    .inner
                    .on_hover_text(hover);
                if resp.clicked() {
                    pick = Some(mode);
                }
            }
        });
        // Applied outside the closure: `apply_mode` takes `&mut self` and
        // the row above already borrows it through `s`.
        if let Some(mode) = pick {
            self.apply_mode(mode);
        }
        self.ack_line_ui(ui);

        gap(ui, Key::GapBlock);
        ui.add_enabled_ui(!busy, |ui| {
            for (mut on, id, label, hover) in [
                (
                    s.wio_sleep,
                    ble::CFG_WIO_SLEEP,
                    "LoRa radio in standby",
                    text::RADIO_STANDBY_HOVER,
                ),
                (
                    s.gps_sleep,
                    ble::CFG_GPS_SLEEP,
                    "GPS in backup mode",
                    text::GPS_SLEEP_HOVER,
                ),
            ] {
                if check!(ui, on, label, hover: hover).changed() {
                    self.send_config(ConfigWrite::Flag { id, on });
                }
            }
        });

        // Stored: the wake cadence and its window. Each title below quotes
        // the node's own value, the row under it is what is asked for next,
        // and the range the node clamps to is the row's hover.
        gap(ui, Key::GapBlock);
        ui.strong(format!("Wake check: {}", secs_text(s.sleep_interval_s)));
        let presets = secs_presets(&WAKE_CHECK_S);
        ui.horizontal_wrapped(|ui| {
            preset_text(
                ui,
                "wake_check",
                &mut self.sleep_interval_text,
                &presets,
                Key::BluetoothNumber,
            );
            if button!(ui, "Apply", enabled: !busy).clicked() {
                self.apply_sleep_interval(None);
            }
            let can_disable = !busy && s.sleep_interval_s > 0;
            let disable = button!(
                ui,
                "Disable",
                enabled: can_disable,
                hover: text::SLEEP_DISABLE_HOVER,
            );
            if disable.clicked() {
                self.apply_sleep_interval(Some(0));
            }
        })
        .response
        .on_hover_text(text::range(ble::ESP_SLEEP_MIN_S, ble::ESP_SLEEP_MAX_S));

        gap(ui, Key::GapBlock);
        ui.strong(format!(
            "Advertising window: {}",
            secs_text(s.adv_window_s)
        ));
        let presets = secs_presets(&ADV_WINDOW_S);
        // No Disable here, unlike the wake check: a zero-length window would
        // leave a sleeping node unreachable by anything short of a physical
        // reset, so the node clamps 0 up to the floor rather than storing it.
        ui.horizontal_wrapped(|ui| {
            preset_text(
                ui,
                "adv_window",
                &mut self.adv_window_text,
                &presets,
                Key::BluetoothNumber,
            );
            if button!(ui, "Apply", enabled: !busy).clicked() {
                self.apply_adv_window();
            }
        })
        .response
        .on_hover_text(text::range(ble::ESP_ADV_MIN_S, ble::ESP_ADV_MAX_S));

        // Idle: how long it lasts, if it ends at all.
        gap(ui, Key::GapBlock);
        ui.strong(format!("Idle timeout: {}", secs_text(s.idle_timeout_s)));
        let presets = secs_presets(&IDLE_TIMEOUT_S);
        ui.horizontal_wrapped(|ui| {
            preset_text(
                ui,
                "idle_timeout",
                &mut self.idle_timeout_text,
                &presets,
                Key::BluetoothNumber,
            );
            if button!(ui, "Apply", enabled: !busy).clicked() {
                self.apply_idle_timeout(None);
            }
            // Zero is the timeout off, which is the firmware default and the
            // safe direction: a node that stays idle can still be reached.
            let can_disable = !busy && s.idle_timeout_s > 0;
            let disable = button!(
                ui,
                "Disable",
                enabled: can_disable,
                hover: text::IDLE_DISABLE_HOVER,
            );
            if disable.clicked() {
                self.apply_idle_timeout(Some(0));
            }
        })
        .response
        .on_hover_text(text::range(ble::IDLE_TIMEOUT_MIN_S, ble::IDLE_TIMEOUT_MAX_S));
        // A timeout with no wake check ends in nothing - the node stays
        // reachable - which is the one case worth a line.
        if s.idle_timeout_s > 0 && s.sleep_interval_s == 0 {
            hint!(ui, text::IDLE_NEEDS_WAKE);
        }

        // Tracking: the modem duty cycle, both halves.
        gap(ui, Key::GapBlock);
        ui.strong(format!("BLE on period: {}", secs_text(s.ble_on_s)));
        let presets = secs_presets(&BLE_ON_S);
        ui.horizontal_wrapped(|ui| {
            preset_text(
                ui,
                "ble_on",
                &mut self.ble_on_text,
                &presets,
                Key::BluetoothNumber,
            );
            if button!(ui, "Apply", enabled: !busy).clicked() {
                self.apply_ble_on();
            }
        })
        .response
        .on_hover_text(text::range(ble::BLE_ON_MIN_S, ble::BLE_ON_MAX_S));

        gap(ui, Key::GapBlock);
        ui.strong(format!("BLE off period: {}", secs_text(s.ble_off_s)));
        let presets = secs_presets(&BLE_OFF_S);
        ui.horizontal_wrapped(|ui| {
            preset_text(
                ui,
                "ble_off",
                &mut self.ble_off_text,
                &presets,
                Key::BluetoothNumber,
            );
            if button!(ui, "Apply", enabled: !busy).clicked() {
                self.apply_ble_off(None);
            }
            // A Disable here, unlike the on period: zero is the firmware
            // default and the safe direction - it makes the node
            // continuously reachable rather than unreachable.
            let can_disable = !busy && s.ble_off_s > 0;
            let disable = button!(
                ui,
                "Disable",
                enabled: can_disable,
                hover: text::BLE_OFF_DISABLE_HOVER,
            );
            if disable.clicked() {
                self.apply_ble_off(Some(0));
            }
        })
        .response
        .on_hover_text(text::range(ble::BLE_OFF_MIN_S, ble::BLE_OFF_MAX_S));

        // Separated from the settings above because it is not one. Every
        // other control on this page changes what the node will do; this
        // one makes it do something, once, and then the link goes away.
        gap(ui, Key::GapBlock);
        ui.strong("Sleep now");
        // The blank entry is labelled with what a blank box will actually
        // do, worked out with the firmware's own resolver rather than
        // restated here - the node is the authority on this number as much
        // as on the settings above.
        let blank = ble::resolve_sleep_now(0, s.sleep_interval_s);
        let mut presets = vec![(String::new(), text::sleep_now_blank(blank))];
        presets.extend(secs_presets(&SLEEP_NOW_S));
        ui.horizontal_wrapped(|ui| {
            preset_text(
                ui,
                "sleep_now",
                &mut self.sleep_now_text,
                &presets,
                Key::BluetoothNumber,
            );
            if button!(ui, "Sleep now", enabled: !busy, hover: text::SLEEP_NOW_HOVER).clicked() {
                self.apply_sleep_now();
            }
        })
        .response
        .on_hover_text(text::range(ble::ESP_SLEEP_MIN_S, ble::ESP_SLEEP_MAX_S));
    }
}
