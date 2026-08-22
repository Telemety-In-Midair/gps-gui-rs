//! The measures every page is written in: the icon sizes, the vertical rhythm,
//! the input widths and the page margin, plus the functions that turn them into
//! points for the current screen.
//!
//! Nothing here draws. It is the one place a size is decided, so a page reads
//! as `gap(ui, GAP_SECTION)` rather than as a point count, and changing the
//! rhythm of the whole app is changing a constant here.
//!
//! Almost every measure is a fraction - of the screen for pictures and touch
//! targets, of the body text height for anything sitting next to text - so a
//! page keeps its proportions on a phone and on a desktop. A fixed point count
//! reads as cramped on one and loose on the other.

/// Icon side length as a fraction of the smaller screen dimension, clamped to
/// this point range. Keeps the toolbar proportional across phone and desktop.
const ICON_SIZE_FRAC: f32 = 0.05;
const ICON_SIZE_MIN: f32 = 40.0;
const ICON_SIZE_MAX: f32 = 70.0;

/// Padding around an icon inside a toolbar button, as a fraction of the icon
/// side. The horizontal one is what makes a toolbar button wider than its
/// glyph, so it has to be part of any width budget (see [`icon_size_for_row`]).
///
/// It is generous because in the bar it doubles as the spacing that keeps the
/// buttons apart, and the bar's own fill sits behind all of it.
pub(super) const BUTTON_PAD_X_FRAC: f32 = 0.7;
pub(super) const BUTTON_PAD_Y_FRAC: f32 = 0.45;

/// Padding around the floating corner toggle's glyph, as a fraction of the
/// icon side. Far tighter than the toolbar's, and deliberately so: alone over
/// the page, with nothing to space it from and its own fill showing, the
/// toolbar's padding reads as an oversized slab around a small glyph - and
/// covers that much more of the text underneath.
pub(super) const TOGGLE_PAD_FRAC: f32 = 0.2;

/// Inset of a floating corner control from the screen edge, as a fraction of
/// the smaller screen dimension.
pub(super) const CORNER_MARGIN_FRAC: f32 = 0.03;

/// Margin between a content page's body and the screen edge, as a fraction of
/// the smaller screen dimension.
const PAGE_MARGIN_FRAC: f32 = 0.025;

/// The vertical rhythm of the pages, in body-text heights: a hair, a tight
/// gap, the gap between controls, between blocks, and between sections.
pub(super) const GAP_HAIR: f32 = 0.25;
pub(super) const GAP_TIGHT: f32 = 0.4;
pub(super) const GAP_ITEM: f32 = 0.5;
pub(super) const GAP_BLOCK: f32 = 0.75;
pub(super) const GAP_SECTION: f32 = 1.0;

/// A text input is a fraction of the screen width, held between these two
/// widths in text units: wide enough to type in on a phone, and not sprawling
/// across a desktop window.
const FIELD_MIN_EM: f32 = 8.0;
const FIELD_MAX_EM: f32 = 22.0;

/// Square icon side length in points for the current screen size.
///
/// The clamp is the one measure in the UI that stays absolute: it is a touch
/// target, and a fingertip is the same size whatever the screen is.
pub(super) fn icon_size_for(screen: egui::Rect) -> f32 {
    (screen.size().min_elem() * ICON_SIZE_FRAC).clamp(ICON_SIZE_MIN, ICON_SIZE_MAX)
}

/// The body text height: the unit the page measures are written in.
pub(super) fn em(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body)
}

/// Vertical space of `ems` body-text heights.
pub(super) fn gap(ui: &mut egui::Ui, ems: f32) {
    let space = em(ui) * ems;
    ui.add_space(space);
}

/// Margin between a page's body and the screen edge, in points.
pub(super) fn page_margin(screen: egui::Rect) -> f32 {
    screen.size().min_elem() * PAGE_MARGIN_FRAC
}

/// Inset of a floating corner control from the screen edge, in points.
pub(super) fn corner_margin(screen: egui::Rect) -> f32 {
    screen.size().min_elem() * CORNER_MARGIN_FRAC
}

/// Width for a text input: `frac` of the screen width, kept readable.
pub(super) fn field_width(ui: &egui::Ui, screen: egui::Rect, frac: f32) -> f32 {
    let em = em(ui);
    (screen.width() * frac).clamp(em * FIELD_MIN_EM, em * FIELD_MAX_EM)
}

/// Largest icon side that keeps a row of `count` icon buttons within `avail`
/// points, so no button may claim more than its equal share of the width.
///
/// A button is wider than its icon by [`BUTTON_PAD_X_FRAC`] on each side, and
/// the buttons are separated by `spacing`; `avail` is expected to already have
/// the enclosing frame's margin taken off. The result is capped at the usual
/// [`icon_size_for`] size, so the row only shrinks below it on a screen too
/// narrow to hold the whole set - the share of the width is a ceiling, not a
/// target. [`ICON_SIZE_MIN`] does not apply here: a button that overflows the
/// screen is worse than a small one.
pub(super) fn icon_size_for_row(
    screen: egui::Rect,
    avail: f32,
    spacing: f32,
    count: usize,
) -> f32 {
    let base = icon_size_for(screen);
    if count == 0 {
        return base;
    }
    let count = count as f32;
    let per_button = (avail - (count - 1.0) * spacing) / count;
    let fit = per_button / (1.0 + 2.0 * BUTTON_PAD_X_FRAC);
    base.min(fit.max(1.0))
}
