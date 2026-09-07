//! The widget vocabulary the pages are written in.
//!
//! Every page is a list of declarations - this section, that hint, this
//! checkbox with that hover - and this module is where each of those
//! declarations is spelled out once. A page file should read as *what is on
//! the page*; the `Area`/`Frame` scaffolding, the `RichText` incantations and
//! the color feedback lines all live here so they do not have to.
//!
//! Macros carry the declarations that a function cannot: the ones with
//! optional pieces ([`button!`] with its enabled/hover/disabled trio,
//! [`section!`] with or without a rule) and the variadic one ([`grid!`]).
//! Everything else is a plain function, which reads better than a macro would.

use std::hash::Hash;
use std::ops::RangeInclusive;
use std::time::Duration;

use crate::app::{MyApp, SafeArea};
use crate::config::UiSettings;

use super::theme::{control_height, em, gap, page_margin, probe, px, Key};

/// How fast the dots behind a busy line cycle.
const DOTS_PERIOD: Duration = Duration::from_millis(500);

/// The trailing dots of a line describing something in progress - one, two,
/// three, one - cycling on the clock, and a repaint scheduled to move them.
/// Appended to a line rather than built into it, so the words stay the
/// text module's and the animation stays here.
pub(super) fn busy_dots(ctx: &egui::Context) -> &'static str {
    let step = (ctx.input(|i| i.time) / DOTS_PERIOD.as_secs_f64()) as u64 % 3;
    ctx.request_repaint_after(DOTS_PERIOD);
    match step {
        0 => ".",
        1 => "..",
        _ => "...",
    }
}

/// The word "custom" in a preset dropdown: the entry that opens the editor
/// for a value none of the presets has.
const CUSTOM: &str = "custom";

/// A setting picked from a short list of presets, with a custom entry that
/// opens an editor for anything else.
///
/// The dropdown shows the preset the value matches, or "custom" when none
/// does. Picking a preset writes it; picking "custom" opens `edit` beside
/// the dropdown - and keeps it open, remembered per widget, until a preset
/// is picked again, so a value dragged through a preset on its way
/// somewhere else does not snap the editor shut. Returns whether the value
/// changed.
pub(super) fn preset_pick<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    id: impl Hash + std::fmt::Debug,
    value: &mut T,
    presets: &[(T, &str)],
    edit: impl FnOnce(&mut egui::Ui, &mut T),
) -> bool {
    let id = egui::Id::new("preset_pick").with(id);
    let mut custom = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let matched = presets.iter().find(|(v, _)| v == value).map(|(_, l)| *l);
    // With no preset matching there is nothing else to show but the editor.
    if matched.is_none() {
        custom = true;
    }
    let before = value.clone();
    let shown = if custom { CUSTOM } else { matched.unwrap_or(CUSTOM) };
    let combo = egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (preset, label) in presets {
                if ui.selectable_label(!custom && preset == value, *label).clicked() {
                    *value = preset.clone();
                    custom = false;
                }
            }
            if ui.selectable_label(custom, CUSTOM).clicked() {
                custom = true;
            }
        });
    probe(
        ui.ctx(),
        combo.response.rect,
        "Dropdown",
        &[Key::ControlCombo, Key::ControlHeight],
    );
    if custom {
        edit(ui, value);
    }
    ui.data_mut(|d| d.insert_temp(id, custom));
    *value != before
}

/// A row of preset labels for a list of seconds, with `secs_text` naming
/// each, so every page spells "5 min" the same way.
pub(super) fn secs_presets(secs: &[u32]) -> Vec<(String, String)> {
    secs.iter()
        .map(|&s| (s.to_string(), crate::app::secs_text(s)))
        .collect()
}

/// [`preset_pick`] over a text buffer that holds a number: the presets are
/// the buffer's own spellings, and the custom editor is the field itself.
/// For the pages whose numbers are typed and sent rather than bound.
pub(super) fn preset_text(
    ui: &mut egui::Ui,
    id: impl Hash + std::fmt::Debug,
    text: &mut String,
    presets: &[(String, String)],
    width: Key,
) -> bool {
    let labelled: Vec<(String, &str)> = presets
        .iter()
        .map(|(v, l)| (v.clone(), l.as_str()))
        .collect();
    preset_pick(ui, id, text, &labelled, |ui, text| {
        text_field(ui, text, "", width);
    })
}

impl MyApp {
    /// A button that puts `text` on the clipboard, saying so beside it for a
    /// moment. The one place the platform difference shows: the phone goes
    /// through the framework, the desktop through egui.
    pub(super) fn copy_button(&self, ui: &mut egui::Ui, text: &str) {
        let id = ui.make_persistent_id(("copied", text));
        if button!(ui, "Copy", hover: "Put this path on the clipboard").clicked() {
            let result = match &self.copier {
                Some(copy) => copy(text),
                None => {
                    ui.ctx().copy_text(text.to_string());
                    Ok(())
                }
            };
            let until = ui.input(|i| i.time) + 2.0;
            ui.data_mut(|d| d.insert_temp(id, (until, result.err())));
        }
        if let Some((until, err)) = ui.data(|d| d.get_temp::<(f64, Option<String>)>(id)) {
            if ui.input(|i| i.time) < until {
                match err {
                    None => {
                        ui.colored_label(self.config.ui.ok, "copied");
                    }
                    Some(e) => {
                        ui.colored_label(self.config.ui.error, e);
                    }
                }
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            } else {
                ui.data_mut(|d| d.remove_temp::<(f64, Option<String>)>(id));
            }
        }
    }
}

/// A square icon button. The icons are white SVGs tinted to the current text
/// color so they follow the theme.
pub(super) fn icon_button(
    ui: &mut egui::Ui,
    size: f32,
    source: egui::ImageSource<'_>,
) -> egui::Response {
    icon_button_pulse(ui, size, source, None)
}

/// Same as [`icon_button`], but when `pulse` carries a color the button
/// background oscillates in it to flag that the action currently has no target
/// (used by the center button when there is no marker to center on).
pub(super) fn icon_button_pulse(
    ui: &mut egui::Ui,
    size: f32,
    source: egui::ImageSource<'_>,
    pulse: Option<egui::Color32>,
) -> egui::Response {
    let tint = ui.visuals().text_color();
    let mut button = egui::Button::image(
        egui::Image::new(source)
            .fit_to_exact_size(egui::vec2(size, size))
            .tint(tint),
    );
    if let Some(color) = pulse {
        // 0..1 oscillation, one cycle every ~1.6s.
        let t = ui.input(|i| i.time);
        let wave = 0.5 + 0.5 * (t * std::f64::consts::PI * 1.25).sin() as f32;
        let alpha = (60.0 + wave * 150.0) as u8;
        button = button.fill(color.gamma_multiply(f32::from(alpha) / 255.0));
        // Keep the animation running even when nothing else asks for a repaint.
        ui.ctx().request_repaint();
    }
    ui.add(button)
}

/// A full-screen page: a Background `Area` filled with the panel color, a
/// [`page_margin`] margin, sized to the screen, with both safe-area insets
/// already kept clear. The closure supplies the page's heading and body (and
/// its own `ScrollArea` where one is used).
pub(super) fn content_page(
    ctx: &egui::Context,
    id: &str,
    screen: egui::Rect,
    safe: SafeArea,
    add: impl FnOnce(&mut egui::Ui),
) {
    // `Margin` counts in whole points, so the fractions are rounded once here
    // and the layout inside uses those same rounded values.
    let margin = page_margin(ctx) as i8;
    // The whole screen, so a tap on empty page is a tap on the page: the
    // picker takes the smallest thing under a point, and this is the largest.
    probe(ctx, screen, "Page", &[Key::PageMargin]);
    // The bottom inset is part of the frame's margin rather than space added
    // after the content, and that is the whole point of it: a `ScrollArea`
    // sizes its viewport to the height it is given, so the inset has to come
    // off that height to keep the last row of a scrolled page above the
    // gesture bar. Trailing space inside the scroll would just scroll away
    // with everything else. The fill still spans the whole screen, so the
    // reserved strip is page-colored rather than a gap.
    let foot = margin.saturating_add(safe.bottom as i8);
    egui::Area::new(egui::Id::new(id))
        .order(egui::Order::Background)
        .fixed_pos(egui::Pos2::ZERO)
        .movable(false)
        .constrain(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(ui.visuals().panel_fill)
                .inner_margin(egui::Margin {
                    left: margin,
                    right: margin,
                    top: margin,
                    bottom: foot,
                })
                .show(ui, |ui| {
                    // An Area sizes itself to whatever it held last frame, so
                    // its Ui has no width to wrap against until something pins
                    // one: without this a long label lays out as one endless
                    // line and widens the page instead of wrapping. `set_width`
                    // pins both bounds, which is also what makes the frame
                    // (content plus its two margins) exactly screen-wide.
                    let margin = f32::from(margin);
                    ui.set_width(screen.width() - 2.0 * margin);
                    // Content plus the frame's two margins is then exactly
                    // screen-tall, which is what leaves the Area measuring a
                    // full screen for the next frame to lay out against.
                    ui.set_min_height(screen.height() - margin - f32::from(foot));
                    ui.add_space(safe.top);
                    gap(ui, Key::GapItem);
                    add(ui);
                });
        });
}

/// A floating popup `Frame` in its own `Area`, used for the transient overlays
/// (selection hint, download confirm/progress, marker info bubble, manual
/// position bar). Returns where it landed, for a probe.
pub(super) fn floating(
    ctx: &egui::Context,
    id: &str,
    order: egui::Order,
    pos: egui::Pos2,
    pivot: egui::Align2,
    constrain: bool,
    add: impl FnOnce(&mut egui::Ui),
) -> egui::Rect {
    egui::Area::new(egui::Id::new(id))
        .order(order)
        .fixed_pos(pos)
        .pivot(pivot)
        .movable(false)
        .constrain(constrain)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| add(ui));
        })
        .response
        .rect
}

/// A centered confirm popup: a question, then the two buttons that answer it.
/// The closure lays out the body between them.
pub(super) fn confirm_popup(
    ctx: &egui::Context,
    id: &str,
    screen: egui::Rect,
    add: impl FnOnce(&mut egui::Ui),
) {
    floating(
        ctx,
        id,
        egui::Order::Foreground,
        screen.center(),
        egui::Align2::CENTER_CENTER,
        false,
        add,
    );
}

/// Show a stored result as a colored line: the ok color on `Ok`, the error
/// color on `Err`, and nothing on `None`. Used for the config-load and BLE-ack
/// feedback. The colors come from the config, so a theme carries through the
/// pages as well as the map.
pub(super) fn feedback_label(
    ui: &mut egui::Ui,
    colors: UiSettings,
    feedback: &Option<Result<String, String>>,
) {
    match feedback {
        Some(Ok(msg)) => {
            ui.colored_label(colors.ok, msg);
        }
        Some(Err(msg)) => {
            ui.colored_label(colors.error, msg);
        }
        None => {}
    }
}

/// A labeled boolean status row: the label followed by an ok-colored "yes" or
/// an error-colored "no", for the Status page's health indicators.
pub(super) fn status_bool(ui: &mut egui::Ui, colors: UiSettings, label: &str, ok: bool) {
    ui.horizontal(|ui| {
        ui.label(format!("{label}:"));
        let (text, color) = if ok {
            ("yes", colors.ok)
        } else {
            ("no", colors.error)
        };
        ui.colored_label(color, text);
    });
}

/// A wrapping row of controls behind a leading label ("Units:", "Every:").
///
/// Wrapping rather than plain horizontal because the labels here are sentences
/// more often than words: on a phone-width screen a plain row pushes its last
/// control off the right edge instead of dropping it to the next line. The
/// row's own response comes back, for a hover over the whole of it.
pub(super) fn row(
    ui: &mut egui::Ui,
    label: &str,
    add: impl FnOnce(&mut egui::Ui),
) -> egui::InnerResponse<()> {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        add(ui);
    })
}

/// A number the user drags to change, over the range the loader will accept.
pub(super) fn drag<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut N,
    speed: f64,
    range: RangeInclusive<N>,
) -> egui::Response {
    let resp = ui.add(egui::DragValue::new(value).speed(speed).range(range));
    probe(
        ui.ctx(),
        resp.rect,
        "Number",
        &[Key::ControlWidth, Key::ControlHeight],
    );
    resp
}

/// A single-line text input as wide as the sheet's `width` key says, with
/// `hint` shown while it is empty. Pair with [`submitted`] where Enter should
/// act as the button beside it.
///
/// A `TextEdit` is the one control egui sizes to its *text* rather than to
/// `interact_size`, so left alone it comes out around half the height of the
/// button next to it - which looks broken and is half as easy to hit. The
/// vertical margin here is the difference, so a field and its button are one
/// row of one height; the side padding is the sheet's.
pub(super) fn text_field(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    width: Key,
) -> egui::Response {
    let row = em(ui);
    let pad_y = ((control_height(ui) - row) / 2.0).max(0.0) as i8;
    let pad_x = px(ui.ctx(), Key::FieldPadX) as i8;
    let desired = px(ui.ctx(), width);
    let resp = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(desired)
            .margin(egui::Margin::symmetric(pad_x, pad_y)),
    );
    probe(
        ui.ctx(),
        resp.rect,
        "Text field",
        &[width, Key::FieldPadX, Key::ControlHeight],
    );
    keep_above_keyboard(ui, &resp);
    resp
}

/// Where the frame notes that the on-screen keyboard has just risen, for the
/// text fields to read.
fn keyboard_rise_id() -> egui::Id {
    egui::Id::new("keyboard_rise")
}

/// Note for this frame whether the bottom inset has just grown, which is the
/// phone's keyboard coming up under a focused field. The app loop calls it
/// once per frame, before the pages, and it is false every frame the inset
/// holds still.
pub(in crate::app) fn note_keyboard_rise(ctx: &egui::Context, rising: bool) {
    ctx.data_mut(|d| d.insert_temp(keyboard_rise_id(), rising));
}

/// Keep a text field in view when the keyboard rises under it. The page's
/// viewport has just lost the keyboard's height off its foot - the inset is
/// part of the page frame - and the field the keyboard was opened for is
/// likely in what was lost, a field near the foot being the one a thumb
/// reaches for. So on the frame the inset grows, the focused field scrolls to
/// the middle of what is left, once; after that the page is the user's to
/// scroll.
pub(super) fn keep_above_keyboard(ui: &egui::Ui, resp: &egui::Response) {
    let rising = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(keyboard_rise_id()))
        .unwrap_or(false);
    if rising && resp.has_focus() {
        resp.scroll_to_me(Some(egui::Align::Center));
    }
}

/// Whether a text field was just committed with Enter, so a page can treat it
/// as a press of the button next to the field.
///
/// egui reports the key on the frame the field loses focus, so both halves
/// have to be tested together.
pub(super) fn submitted(ui: &egui::Ui, resp: &egui::Response) -> bool {
    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// A dimmed line under a control: what something is doing, or the one thing
/// a control cannot say for itself. The same text as a label, dimmed so it
/// reads as commentary rather than as another setting.
///
/// Takes either a ready string (usually a `super::text` constant) or a
/// `format!` pattern and its arguments - a string literal is always treated as
/// the pattern, so an inline `{capture}` in one is filled in rather than
/// printed. `hint!(ui, small ..)` is the smaller version, for prose attached to
/// one row rather than to a section.
macro_rules! hint {
    ($ui:expr, small $fmt:literal $($arg:tt)*) => {
        $ui.label(egui::RichText::new(format!($fmt $($arg)*)).weak().small())
    };
    ($ui:expr, small $text:expr $(,)?) => {
        $ui.label(egui::RichText::new($text).weak().small())
    };
    ($ui:expr, $fmt:literal $($arg:tt)*) => {
        $ui.label(egui::RichText::new(format!($fmt $($arg)*)).weak())
    };
    ($ui:expr, $text:expr $(,)?) => {
        $ui.label(egui::RichText::new($text).weak())
    };
}

/// A page's title, and nothing under it: a heading names the page, and a
/// line explaining it is what a page cannot spare the room for.
///
/// No trailing gap: what comes next decides its own leading space, which for a
/// [`section!`] is already part of the section.
macro_rules! heading {
    ($ui:expr, $title:expr $(,)?) => {
        $ui.heading($title)
    };
}

/// A group of related controls: the space that sets it apart, and its title.
/// No line under the title explaining the group: a title that needs one is
/// the wrong title, and the explanation belongs in the docs.
///
/// `section!(ui, sep "Title")` rules a line above the title as well, for the
/// pages whose groups are big enough that space alone stops separating them.
macro_rules! section {
    ($ui:expr, sep $title:expr $(,)?) => {{
        $crate::app::ui::theme::gap($ui, $crate::app::ui::theme::Key::GapSection);
        $ui.separator();
        $crate::app::ui::theme::gap($ui, $crate::app::ui::theme::Key::GapItem);
        $ui.strong($title);
    }};
    ($ui:expr, $title:expr $(,)?) => {{
        $crate::app::ui::theme::gap($ui, $crate::app::ui::theme::Key::GapSection);
        $ui.strong($title);
    }};
}

/// A text button, and the three things a page ever wants to say about one:
/// whether it is live, what it does, and - when it is not live - why not.
///
/// Written as one declaration because the alternative is the shape this
/// replaces, an `add_enabled(cond, Button::new(..))` wrapped in enough builder
/// calls that the label is no longer the first thing you read. The pieces are
/// optional but ordered: `enabled`, then `hover`, then `disabled`.
///
/// Evaluates to the `egui::Response`, so a press is still `.clicked()`.
macro_rules! button {
    ($ui:expr, $label:expr
        $(, enabled: $enabled:expr)?
        $(, hover: $hover:expr)?
        $(, disabled: $disabled:expr)?
        $(,)?
    ) => {{
        // `true && cond` is just `cond`, so the optional piece folds away when
        // it is not given rather than needing a mutable default.
        let live = true $( && $enabled )?;
        let resp = $ui.add_enabled(live, egui::Button::new($label));
        $crate::app::ui::theme::probe(
            $ui.ctx(),
            resp.rect,
            "Button",
            &[
                $crate::app::ui::theme::Key::ControlPadX,
                $crate::app::ui::theme::Key::ControlPadY,
                $crate::app::ui::theme::Key::ControlHeight,
            ],
        );
        $( let resp = resp.on_hover_text($hover); )?
        $( let resp = resp.on_disabled_hover_text($disabled); )?
        resp
    }};
}

/// A checkbox bound straight to the field it sets, with an optional hover
/// explaining what turning it on actually does.
macro_rules! check {
    ($ui:expr, $value:expr, $label:expr $(, hover: $hover:expr)? $(,)?) => {{
        let resp = $ui.checkbox(&mut $value, $label);
        $crate::app::ui::theme::probe(
            $ui.ctx(),
            resp.rect,
            "Checkbox",
            &[
                $crate::app::ui::theme::Key::ControlCheckSize,
                $crate::app::ui::theme::Key::ControlCheckMark,
                $crate::app::ui::theme::Key::ControlHeight,
            ],
        );
        $( let resp = resp.on_hover_text($hover); )?
        resp
    }};
}

/// A two-column grid of label-and-control pairs, for the settings that are
/// genuinely a table (the color pickers, the overlay sizes).
///
/// The `ui` the arms are written against is named at the call site, so it is
/// the grid's own `Ui` rather than the surrounding one.
macro_rules! grid {
    ($ui:expr, $id:expr, |$inner:ident| { $($label:expr => $control:expr),* $(,)? }) => {
        egui::Grid::new($id).num_columns(2).show($ui, |$inner| {
            $(
                $inner.label($label);
                $control;
                $inner.end_row();
            )*
        })
    };
}

pub(super) use {button, check, grid, heading, hint, section};
