//! The Beacon page: the BLE link to the board, the app-side settings that
//! decide how it connects, and the board's own power and sleep settings.
//!
//! Split from [`MyApp::settings_page`] by who owns each setting. The two
//! groups here read alike but are not: the connection settings are the app's,
//! saved to its TOML with the button beside them, while everything under
//! "Board power and sleep" lives in the board's flash and is only ever
//! reported by the board (see [`MyApp::board_power_ui`]).

use std::time::Duration;

use midair_proto::{ble, session};

use crate::app::ui::text::beacon as text;
use crate::app::ui::theme::{em, gap, GAP_BLOCK, GAP_HAIR, GAP_ITEM, GAP_TIGHT};
use crate::app::ui::widgets::{
    button, check, content_page, feedback_label, heading, hint, row, section, text_field,
};
use crate::app::{secs_text, BleIntent, MyApp};
use crate::ble::ConfigWrite;

/// How often the elapsed counts refresh, and how often a running scan's signal
/// readings do. Both move by themselves; the scan is the faster of the two
/// because a board answering is the thing being waited for.
const ELAPSED_TICK: Duration = Duration::from_secs(1);
const SCAN_TICK: Duration = Duration::from_millis(500);

/// Widths in text heights, so a box holds roughly the same number of
/// characters whatever the font scale is: a board's nickname, and a number of
/// milliseconds or seconds.
const NAME_EM: f32 = 7.0;
const NUMBER_EM: f32 = 5.0;

impl MyApp {
    pub(crate) fn beacon_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let safe = self.safe_area(ctx);
        content_page(ctx, "beacon", screen, safe, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading!(ui, "Beacon", text::INTRO);

                // Which board first, then what to do about the link to it.
                section!(ui, "Device");
                gap(ui, GAP_TIGHT);
                self.device_picker_ui(ui);

                section!(ui, sep "Link");
                gap(ui, GAP_TIGHT);
                self.ble_link_ui(ui);

                section!(ui, sep "Connection");
                gap(ui, GAP_TIGHT);
                self.connection_ui(ui);

                section!(ui, sep "Board power and sleep", text::BOARD_INTRO);
                gap(ui, GAP_ITEM);
                self.board_power_ui(ui);
            });
        });
    }

    /// Pick which board to talk to. Only one is connected at a time, so this is
    /// a single-choice list rather than a set of toggles.
    ///
    /// Every board runs the same firmware and advertises the same name, so a
    /// raw scan is a list of identical entries told apart only by MAC. The
    /// nickname box on each row is what makes the list readable, and it is
    /// stored in the app's config file rather than on the board.
    fn device_picker_ui(&mut self, ui: &mut egui::Ui) {
        let scanning = self.ble_intent == BleIntent::Scanning;
        ui.horizontal_wrapped(|ui| {
            let scan = button!(
                ui,
                if scanning { "Stop scanning" } else { "Scan for boards" },
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
                ui.spinner();
            }
        });
        gap(ui, GAP_TIGHT);

        let rows = self.device_rows();
        // Named boards are remembered, so an empty list means nothing has ever
        // been named and nothing is on the air right now.
        if rows.is_empty() {
            hint!(
                ui,
                if scanning {
                    text::NO_BOARDS_SCANNING
                } else {
                    text::NO_BOARDS_IDLE
                }
            );
        }

        let any_selected = self.config.ble.mac.is_none();
        if ui
            .radio(any_selected, "Any board")
            .on_hover_text(text::ANY_BOARD_HOVER)
            .clicked()
            && !any_selected
        {
            self.select_device(None);
        }

        let name_width = em(ui) * NAME_EM;
        for device in rows {
            ui.horizontal_wrapped(|ui| {
                if ui.radio(device.selected, "").clicked() && !device.selected {
                    self.select_device(Some(&device.mac));
                }
                // Committing on blur rather than per keystroke: an empty name
                // forgets the board, and that must not happen mid-edit just
                // because the box was cleared before retyping.
                let name = self.name_edit(&device.mac);
                if text_field(ui, name, "name this board", name_width).lost_focus() {
                    self.commit_name(&device.mac);
                }
                hint!(ui, device.mac.as_str());
                match device.rssi {
                    // Only a running scan measures this, so its absence during
                    // a scan is the useful signal: that board is not answering.
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

        gap(ui, GAP_TIGHT);
        hint!(ui, text::NAMES_NOTE);
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
    /// the board sleep, and that was impossible to express when connecting was
    /// a checkbox the app re-applied on its own.
    ///
    /// Every one of them takes effect on the press. Connect stays live while
    /// connected because pressing it then is a real request - start over from
    /// a scan - and it is the way out of a link that is up but not working.
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

        gap(ui, GAP_TIGHT);
        // Which board these buttons act on. With several boards around, the
        // link state means little without knowing whose it is.
        ui.label(format!("Board: {}", self.selected_device_label()));
        if connected {
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
            ui.label(self.ble_intent_text());
        }
        // The worker's own commentary: scanning, connecting, why it retried.
        // Distinct from the line above, which is what was *asked* for.
        hint!(ui, "BLE: {}", self.ble_status);

        // The single most useful thing to know while debugging sleep: a
        // connected board never sleeps, so a sleep interval that "does
        // nothing" is usually just the app holding the link open.
        if let (true, Some(s)) = (connected, self.board_settings) {
            gap(ui, GAP_HAIR);
            match s.sleep_interval_s {
                0 => hint!(ui, text::SLEEP_DISABLED),
                secs => hint!(ui, text::will_sleep(secs, s.adv_window_s)),
            };
        }
        if !connected && !idle {
            gap(ui, GAP_HAIR);
            hint!(ui, text::ONLY_ON_WINDOW);
            // The elapsed count is the only thing here that moves by itself; a
            // one-second tick keeps it honest without pinning the frame rate.
            ui.ctx().request_repaint_after(ELAPSED_TICK);
        }
    }

    /// The app's own connection settings, and the board's notify interval.
    ///
    /// The board names, the selected board and the auto-connect checkbox are
    /// the app's settings, not the board's, so they need the same Save the
    /// Settings page has rather than a trip back to it. Same file, same
    /// feedback line.
    fn connection_ui(&mut self, ui: &mut egui::Ui) {
        check!(
            ui,
            self.config.ble.enabled,
            "Connect automatically at startup",
            hover: text::AUTO_CONNECT_HOVER,
        );
        gap(ui, GAP_TIGHT);
        if button!(ui, "Save to config file", hover: text::SAVE_HOVER).clicked() {
            self.save_config();
        }
        feedback_label(ui, self.config.ui, &self.config_feedback);

        gap(ui, GAP_ITEM);
        let ready = self.ble_connected && !self.ble_ack_pending;
        let width = em(ui) * NUMBER_EM;
        row(ui, "Notify interval (ms):", |ui| {
            text_field(ui, &mut self.ble_interval_text, "", width);
            if button!(ui, "Apply", enabled: ready).clicked() {
                self.apply_notify_interval();
            }
        });
        if self.ble_ack.is_none() && self.ble_ack_pending {
            ui.label(text::AWAITING_ACK);
        } else {
            feedback_label(ui, self.config.ui, &self.ble_ack);
        }
    }

    /// The board's sleep switches and the wake-check interval.
    ///
    /// Every control reads the board's own settings blob rather than a local
    /// copy: the board is the authority, and it changes these by itself
    /// (clamping an interval). A control therefore only moves once the board
    /// reports that it moved.
    fn board_power_ui(&mut self, ui: &mut egui::Ui) {
        if !self.ble_connected {
            ui.label(text::BOARD_NEED_LINK);
            return;
        }
        if self.settings_unsupported {
            ui.colored_label(self.config.ui.error, text::BOARD_TOO_NEW);
            ui.label(text::BOARD_TOO_NEW_MORE);
            return;
        }
        let Some(s) = self.board_settings else {
            ui.label(text::BOARD_READING);
            return;
        };

        // One write at a time: while an ack is outstanding the board has not
        // yet said what it applied, and these controls show only what it has.
        let busy = self.ble_ack_pending;
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

        let width = em(ui) * NUMBER_EM;
        gap(ui, GAP_BLOCK);
        ui.strong("Wake check");
        hint!(
            ui,
            text::wake_check(ble::ESP_SLEEP_MIN_S, ble::ESP_SLEEP_MAX_S)
        );
        row(ui, "Every (s):", |ui| {
            text_field(ui, &mut self.sleep_interval_text, "", width);
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
        });
        ui.label(match s.sleep_interval_s {
            0 => "Board: sleep disabled.".to_string(),
            secs => format!("Board: waking every {}.", secs_text(secs)),
        });

        gap(ui, GAP_BLOCK);
        ui.strong("Advertising window");
        hint!(
            ui,
            text::adv_window(ble::ESP_ADV_MIN_S, ble::ESP_ADV_MAX_S)
        );
        hint!(ui, text::adv_window_note(session::LINGER_S as u32));
        row(ui, "Window (s):", |ui| {
            text_field(ui, &mut self.adv_window_text, "", width);
            if button!(ui, "Apply", enabled: !busy).clicked() {
                self.apply_adv_window();
            }
        });
        // No Disable here, unlike the wake check: a zero-length window would
        // leave a sleeping board unreachable by anything short of a physical
        // reset, so the board clamps 0 up to the floor rather than storing it.
        ui.label(format!(
            "Board: advertising {} per wake.",
            secs_text(s.adv_window_s)
        ));

        // Separated from the settings above because it is not one. Every
        // other control on this page changes what the board will do; this
        // one makes it do something, once, and then the link goes away.
        gap(ui, GAP_BLOCK);
        ui.strong("Sleep now");
        hint!(
            ui,
            text::sleep_now(ble::ESP_SLEEP_MIN_S, ble::ESP_SLEEP_MAX_S)
        );
        row(ui, "For (s):", |ui| {
            text_field(ui, &mut self.sleep_now_text, "", width)
                .on_hover_text(text::SLEEP_NOW_BLANK_HOVER);
            if button!(ui, "Sleep now", enabled: !busy, hover: text::SLEEP_NOW_HOVER).clicked() {
                self.apply_sleep_now();
            }
        });
        // What a blank box will actually do, worked out with the firmware's
        // own resolver rather than restated here - the board is the
        // authority on this number as much as on the settings above.
        if self.sleep_now_text.trim().is_empty() {
            ui.label(format!(
                "Blank: sleeps for {}.",
                secs_text(ble::resolve_sleep_now(0, s.sleep_interval_s))
            ));
        }
    }
}
