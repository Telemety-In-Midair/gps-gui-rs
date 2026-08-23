//! The Radio page: load the WIO-E5's RADIO.TOML, edit each setting with a
//! type-specific input behind a per-field edit lock, and save it back -
//! keeping the file's comments and a timestamped backup of the previous
//! version.

use crate::app::ui::icons;
use crate::app::ui::text::radio as text;
use crate::app::ui::theme::{
    control_height, em, field_width, gap, GAP_BLOCK, GAP_ITEM, GAP_TIGHT,
};
use crate::app::ui::widgets::{
    button, confirm_popup, content_page, feedback_label, heading, hint, icon_button, submitted,
    text_field,
};
use crate::app::{MyApp, RadioEdit};
use crate::radio::{self, EditVal, FieldType};

/// The path field is half the screen, leaving room for the four buttons the
/// row carries beside it.
const PATH_FRAC: f32 = 0.5;

/// The glyph in a field-row action button (the pencil, the check, the x), as a
/// fraction of the height the button is laid out at.
///
/// Written against the button rather than the text so the two agree: the
/// button's height is the touch-target floor, and a glyph sized off the text
/// alone left a tall thin key with a small mark adrift in it.
const ACTION_GLYPH_FRAC: f32 = 0.55;

/// Width of a free-text field in a radio row, in text heights, so it scales
/// with the font rather than being a raw pixel count.
const ENUM_FIELD_EM: f32 = 12.0;

/// Width of the push-confirm popup, in text heights - enough for its two lines
/// to wrap into a block rather than one long line across the screen.
const CONFIRM_EM: f32 = 18.0;

/// Render the type-specific input for an unlocked radio field, bound to `val`.
/// The kind of widget follows the field's type: a draggable number, a checkbox,
/// a dropdown for an enum, or a text field.
fn radio_input(ui: &mut egui::Ui, key: &str, ty: &FieldType, val: &mut EditVal) {
    match val {
        EditVal::Int(i) => {
            ui.add(egui::DragValue::new(i));
        }
        EditVal::Float(f) => {
            ui.add(egui::DragValue::new(f));
        }
        EditVal::Bool(b) => {
            ui.checkbox(b, "");
        }
        EditVal::Str(s) => {
            if let FieldType::Enum(opts) = ty {
                egui::ComboBox::from_id_salt(("radio_enum", key))
                    .selected_text(s.clone())
                    .show_ui(ui, |ui| {
                        for opt in opts {
                            ui.selectable_value(s, opt.clone(), opt.as_str());
                        }
                    });
            } else {
                let width = em(ui) * ENUM_FIELD_EM;
                ui.add(egui::TextEdit::singleline(s).desired_width(width));
            }
        }
    }
}

impl MyApp {
    pub(crate) fn radio_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let safe = self.safe_area(ctx);
        content_page(ctx, "radio", screen, safe, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading!(ui, "Radio config", text::INTRO);
                gap(ui, GAP_BLOCK);

                ui.label("File:");
                self.radio_file_ui(ui, screen);
                gap(ui, GAP_TIGHT);
                feedback_label(ui, self.config.ui, &self.radio_feedback);

                if self.radio.is_some() {
                    gap(ui, GAP_ITEM);
                    self.radio_fields_ui(ui);
                    self.radio_estimate_ui(ui);
                    self.radio_backups_ui(ui);
                } else {
                    gap(ui, GAP_BLOCK);
                    ui.label(text::EMPTY);
                    gap(ui, GAP_ITEM);
                    // With no file to load (a fresh SD card), start from the
                    // firmware defaults instead. It fills the editor only; Save
                    // is what writes the file.
                    if button!(ui, "Generate default config", hover: text::GENERATE_HOVER)
                        .clicked()
                    {
                        self.default_radio();
                    }
                }
            });
        });

        // The edit-confirm popup floats above the page; a nested Area inside the
        // page's own Area misbehaves, so it is drawn here at the top level.
        self.radio_confirm_popup(ctx, screen);
        self.radio_push_popup(ctx, screen);
    }

    /// The file path and the four things that can be done with it: read the
    /// file, write it, send the editor to the board, and read the board back
    /// into the editor.
    fn radio_file_ui(&mut self, ui: &mut egui::Ui, screen: egui::Rect) {
        ui.horizontal_wrapped(|ui| {
            let width = field_width(ui, screen, PATH_FRAC);
            let resp = text_field(ui, &mut self.radio_path, "/path/to/RADIO.toml", width);
            if ui.button("Load").clicked() || submitted(ui, &resp) {
                self.load_radio();
            }

            let dirty = self.radio.as_ref().is_some_and(|r| r.dirty);
            if button!(
                ui,
                if dirty { "Save *" } else { "Save" },
                enabled: self.radio.is_some(),
            )
            .clicked()
            {
                self.save_radio();
            }

            // Push the editor's config to the board over BLE. Behind a confirm
            // popup: it replaces the board's whole config.
            let can_send = self.radio.is_some() && self.ble_connected && !self.radio_push_pending;
            let why = if self.radio.is_none() {
                text::SEND_NEEDS_CONFIG
            } else if !self.ble_connected {
                text::SEND_NEEDS_LINK
            } else {
                text::SEND_WAITING
            };
            let send = button!(
                ui,
                if self.radio_push_pending { "Sending..." } else { "Send to board" },
                enabled: can_send,
                hover: text::SEND_HOVER,
                disabled: why,
            );
            if send.clicked() {
                self.radio_push_confirm = true;
            }

            // Fill the editor with the connected board's own settings, the
            // read-back counterpart to Send. Enabled once the board has
            // reported a config it can decode.
            let fetch_why = if self.radio_config_unsupported {
                text::FETCH_TOO_NEW
            } else {
                text::FETCH_NEEDS_LINK
            };
            let fetch = button!(
                ui,
                "Load from board",
                enabled: self.board_radio_config.is_some(),
                hover: text::FETCH_HOVER,
                disabled: fetch_why,
            );
            if fetch.clicked() {
                self.load_radio_from_board();
            }
        });
    }

    /// The editable settings, grouped by their `[section]`. Each row is a
    /// read-only value with an edit lock, or - while unlocked - the typed input.
    fn radio_fields_ui(&mut self, ui: &mut egui::Ui) {
        let n = match &self.radio {
            Some(r) => r.fields.len(),
            None => return,
        };
        // A sentinel no real section equals, so the first field emits a heading.
        let mut section_shown = String::from("\u{0}");
        for i in 0..n {
            let (section, key, ty, desc) = {
                let f = &self.radio.as_ref().unwrap().fields[i];
                (
                    f.section.clone(),
                    f.key.clone(),
                    f.ty.clone(),
                    f.description.clone(),
                )
            };
            if section != section_shown {
                gap(ui, GAP_BLOCK);
                ui.strong(if section.is_empty() {
                    "general"
                } else {
                    section.as_str()
                });
                ui.separator();
                section_shown = section.clone();
            }
            self.radio_field_row(ui, &section, &key, &ty, desc.as_deref());
        }
    }

    /// The airtime estimate: exact time-on-air for one beacon at the settings
    /// currently in the editor, the duty cycle the beacon interval sets, and
    /// whether one transmission stays under the dwell limit the US
    /// 902-928 MHz band imposes.
    ///
    /// The values are read from the editor the same way a push reads them - the
    /// wire bytes parsed back into a `RadioConfig` - so the estimate tracks
    /// edits the moment they are set, and reflects the same clamping the board
    /// would apply. An editor holding a value the firmware would reject parses
    /// to nothing, and the panel simply stays hidden until it is valid again.
    fn radio_estimate_ui(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.radio.as_ref() else {
            return;
        };
        let Ok(cfg) = midair_proto::radiocfg::parse_bytes(&doc.wire_bytes()) else {
            return;
        };
        let colors = self.config.ui;
        let est = radio::airtime(&cfg);

        gap(ui, GAP_BLOCK);
        ui.strong("Airtime estimate");
        ui.separator();

        ui.label(format!(
            "Time on air: {:.1} ms per beacon (SF{}, BW{} kHz, CR 4/{}, {}-byte frame)",
            est.toa_ms,
            cfg.spreading_factor,
            cfg.bandwidth_khz,
            cfg.coding_rate,
            est.payload_len,
        ));
        match est.duty_pct {
            Some(duty) => {
                ui.label(format!(
                    "Beacon interval: {} s  ->  duty cycle {duty:.2}%",
                    est.interval_s,
                ));
            }
            None => {
                hint!(ui, text::BEACON_OFF);
            }
        }

        // The dwell rule only means anything in-band, so out of band there is
        // nothing to report.
        let Some(limit) = &est.limit else {
            return;
        };
        let (verdict, color) = match limit.overrun_ms(est.toa_ms) {
            None => (
                format!("Under the {:.0} ms limit (902-928 MHz)", limit.budget_ms),
                colors.ok,
            ),
            Some(over) => (
                format!(
                    "Over the {:.0} ms limit by {over:.0} ms (902-928 MHz)",
                    limit.budget_ms
                ),
                colors.error,
            ),
        };
        ui.colored_label(color, verdict);
        // Say which of the two limits is binding, so the number is not a bare
        // figure the reader has to reverse-engineer.
        if limit.dwell_bound {
            hint!(ui, "Limit: 400 ms channel dwell per 20 s.");
        } else {
            hint!(
                ui,
                "Limit: 2% duty cycle over the {} s interval.",
                est.interval_s
            );
        }
    }

    /// One field row: the key, then either the read-only value with a pencil
    /// (edit) button, or - while this field is unlocked - the typed input with a
    /// check (set) and an x (cancel). The description, if any, follows beneath.
    fn radio_field_row(
        &mut self,
        ui: &mut egui::Ui,
        section: &str,
        key: &str,
        ty: &FieldType,
        desc: Option<&str>,
    ) {
        let active = matches!(
            &self.radio_edit,
            RadioEdit::Active { section: s, key: k, .. }
                if s.as_str() == section && k.as_str() == key
        );
        // Sized off the button, which is sized off the text: nothing here is a
        // raw pixel constant.
        let bsz = control_height(ui) * ACTION_GLYPH_FRAC;
        // Wrapped so a long key or value drops its input to the next line
        // rather than pushing the edit buttons past the screen edge.
        ui.horizontal_wrapped(|ui| {
            ui.monospace(key);
            if active {
                if let RadioEdit::Active { val, .. } = &mut self.radio_edit {
                    radio_input(ui, key, ty, val);
                }
                if icon_button(ui, bsz, icons::check()).on_hover_text("Set").clicked() {
                    if let RadioEdit::Active { val, .. } = &self.radio_edit {
                        let val = val.clone();
                        if let Some(doc) = self.radio.as_mut() {
                            doc.apply(section, key, &val);
                        }
                    }
                    self.radio_edit = RadioEdit::None;
                }
                if icon_button(ui, bsz, icons::close())
                    .on_hover_text("Cancel")
                    .clicked()
                {
                    self.radio_edit = RadioEdit::None;
                }
            } else {
                let display = self.radio.as_ref().unwrap().display_at(section, key);
                ui.monospace(display);
                // While any field is mid-edit, lock the other pencils so only
                // one field is edited at a time.
                let busy = !matches!(self.radio_edit, RadioEdit::None);
                let edit = ui
                    .add_enabled_ui(!busy, |ui| {
                        icon_button(ui, bsz, icons::edit()).on_hover_text("Edit")
                    })
                    .inner;
                if edit.clicked() {
                    self.radio_edit = RadioEdit::Confirm {
                        section: section.to_string(),
                        key: key.to_string(),
                    };
                }
            }
        });
        if let Some(d) = desc {
            hint!(ui, small d);
        }
        gap(ui, GAP_TIGHT);
    }

    /// The floating Edit / Cancel popup shown when a field's pencil is pressed.
    /// Confirming unlocks the field for editing; cancelling clears the flow.
    fn radio_confirm_popup(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let (section, key) = match &self.radio_edit {
            RadioEdit::Confirm { section, key } => (section.clone(), key.clone()),
            _ => return,
        };
        confirm_popup(ctx, "radio_confirm", screen, |ui| {
            ui.label(format!("Edit \"{key}\"?"));
            gap(ui, GAP_ITEM);
            ui.horizontal(|ui| {
                if ui.button("Edit").clicked() {
                    let val = self
                        .radio
                        .as_ref()
                        .map(|r| r.edit_val_at(&section, &key))
                        .unwrap_or(EditVal::Str(String::new()));
                    self.radio_edit = RadioEdit::Active {
                        section: section.clone(),
                        key: key.clone(),
                        val,
                    };
                }
                if ui.button("Cancel").clicked() {
                    self.radio_edit = RadioEdit::None;
                }
            });
        });
    }

    /// The floating Send / Cancel popup behind the Send-to-board button. A
    /// confirm because a push replaces the board's whole config - a key absent
    /// from the file reverts to its firmware default, not to what the board
    /// had - and takes effect immediately.
    fn radio_push_popup(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        if !self.radio_push_confirm {
            return;
        }
        confirm_popup(ctx, "radio_push_confirm", screen, |ui| {
            ui.set_max_width(em(ui) * CONFIRM_EM);
            ui.label(text::PUSH_CONFIRM);
            hint!(ui, small text::PUSH_CONFIRM_MORE);
            gap(ui, GAP_ITEM);
            ui.horizontal(|ui| {
                if ui.button("Send").clicked() {
                    self.radio_push_confirm = false;
                    self.push_radio();
                }
                if ui.button("Cancel").clicked() {
                    self.radio_push_confirm = false;
                }
            });
        });
    }

    /// A collapsible list of kept backups, newest first, each restorable into
    /// the editor (a restored file is unsaved until Save writes it as current).
    fn radio_backups_ui(&mut self, ui: &mut egui::Ui) {
        let backups = match &self.radio {
            Some(r) => r.backups(),
            None => return,
        };
        gap(ui, GAP_BLOCK);
        ui.separator();
        egui::CollapsingHeader::new(format!("Backups ({})", backups.len()))
            .id_salt("radio_backups")
            .show(ui, |ui| {
                if backups.is_empty() {
                    ui.label(text::NO_BACKUPS);
                }
                for b in &backups {
                    ui.horizontal(|ui| {
                        let name = b.file_name().and_then(|s| s.to_str()).unwrap_or("");
                        ui.monospace(name);
                        if ui.button("Restore").clicked() {
                            if let Some(doc) = self.radio.as_mut() {
                                let res = doc.restore(b);
                                self.radio_feedback = Some(match res {
                                    Ok(()) => Ok(format!("Restored {name} (unsaved - press Save)")),
                                    Err(e) => Err(e),
                                });
                            }
                            self.radio_edit = RadioEdit::None;
                        }
                    });
                }
            });
    }
}
