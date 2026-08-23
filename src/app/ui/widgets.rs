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
//! [`section!`] with or without a hint) and the variadic one ([`grid!`]).
//! Everything else is a plain function, which reads better than a macro would.

use std::ops::RangeInclusive;

use crate::app::SafeArea;
use crate::config::UiSettings;

use super::theme::{control_height, em, gap, page_margin, GAP_ITEM};

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
    let margin = page_margin(screen) as i8;
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
                    gap(ui, GAP_ITEM);
                    add(ui);
                });
        });
}

/// A floating popup `Frame` in its own `Area`, used for the transient overlays
/// (selection hint, download confirm/progress, marker info bubble, manual
/// position bar).
pub(super) fn floating(
    ctx: &egui::Context,
    id: &str,
    order: egui::Order,
    pos: egui::Pos2,
    pivot: egui::Align2,
    constrain: bool,
    add: impl FnOnce(&mut egui::Ui),
) {
    egui::Area::new(egui::Id::new(id))
        .order(order)
        .fixed_pos(pos)
        .pivot(pivot)
        .movable(false)
        .constrain(constrain)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| add(ui));
        });
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

/// A wrapping row of controls behind a leading label ("Units:", "Every (s):").
///
/// Wrapping rather than plain horizontal because the labels here are sentences
/// more often than words: on a phone-width screen a plain row pushes its last
/// control off the right edge instead of dropping it to the next line.
pub(super) fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        add(ui);
    });
}

/// A number the user drags to change, over the range the loader will accept.
pub(super) fn drag<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut N,
    speed: f64,
    range: RangeInclusive<N>,
) -> egui::Response {
    ui.add(egui::DragValue::new(value).speed(speed).range(range))
}

/// Side padding inside a text input, in body-text heights. The vertical
/// padding is not a constant: it is whatever brings the field up to the height
/// of the button beside it (see below).
const FIELD_PAD_X_EM: f32 = 0.3;

/// A single-line text input `width` points wide, with `hint` shown while it is
/// empty. Pair with [`submitted`] where Enter should act as the button beside
/// it.
///
/// A `TextEdit` is the one control egui sizes to its *text* rather than to
/// `interact_size`, so left alone it comes out around half the height of the
/// button next to it - which looks broken and is half as easy to hit. The
/// vertical margin here is the difference, so a field and its button are one
/// row of one height.
pub(super) fn text_field(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    width: f32,
) -> egui::Response {
    let row = em(ui);
    let pad_y = ((control_height(ui) - row) / 2.0).max(0.0) as i8;
    let pad_x = (row * FIELD_PAD_X_EM) as i8;
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(width)
            .margin(egui::Margin::symmetric(pad_x, pad_y)),
    )
}

/// Whether a text field was just committed with Enter, so a page can treat it
/// as a press of the button next to the field.
///
/// egui reports the key on the frame the field loses focus, so both halves
/// have to be tested together.
pub(super) fn submitted(ui: &egui::Ui, resp: &egui::Response) -> bool {
    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// Explanatory prose beneath a heading or a control: the same text as a label,
/// dimmed so it reads as commentary rather than as another setting.
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

/// A page's title, optionally followed by a line saying what the page is for.
///
/// No trailing gap: what comes next decides its own leading space, which for a
/// [`section!`] is already part of the section.
macro_rules! heading {
    ($ui:expr, $title:expr $(,)?) => {
        $ui.heading($title)
    };
    ($ui:expr, $title:expr, $hint:expr $(,)?) => {{
        $ui.heading($title);
        $crate::app::ui::theme::gap($ui, $crate::app::ui::theme::GAP_TIGHT);
        $crate::app::ui::widgets::hint!($ui, $hint);
    }};
}

/// A group of related controls: the space that sets it apart, its title, and
/// optionally a line explaining the group.
///
/// `section!(ui, sep "Title")` rules a line above the title as well, for the
/// pages whose groups are big enough that space alone stops separating them.
macro_rules! section {
    ($ui:expr, sep $title:expr $(, $hint:expr)? $(,)?) => {{
        $crate::app::ui::theme::gap($ui, $crate::app::ui::theme::GAP_SECTION);
        $ui.separator();
        $crate::app::ui::theme::gap($ui, $crate::app::ui::theme::GAP_ITEM);
        $ui.strong($title);
        $(
            $crate::app::ui::theme::gap($ui, $crate::app::ui::theme::GAP_TIGHT);
            $crate::app::ui::widgets::hint!($ui, $hint);
        )?
    }};
    ($ui:expr, $title:expr $(, $hint:expr)? $(,)?) => {{
        $crate::app::ui::theme::gap($ui, $crate::app::ui::theme::GAP_SECTION);
        $ui.strong($title);
        $(
            $crate::app::ui::theme::gap($ui, $crate::app::ui::theme::GAP_TIGHT);
            $crate::app::ui::widgets::hint!($ui, $hint);
        )?
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
