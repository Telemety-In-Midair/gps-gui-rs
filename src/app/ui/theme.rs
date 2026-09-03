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

/// The smallest a control may be, in points: the floor under an icon's side
/// length and under the height of every button, field, checkbox and dropdown
/// on a page.
///
/// The one measure in the UI that stays absolute. Everything else here is a
/// fraction of the screen or of the text, but this is a touch target, and a
/// fingertip is the same size whatever the screen is.
pub(super) const TOUCH_MIN: f32 = 40.0;

/// Icon side length as a fraction of the smaller screen dimension, held
/// between [`TOUCH_MIN`] and this ceiling. Keeps the toolbar proportional
/// across phone and desktop.
const ICON_SIZE_FRAC: f32 = 0.05;
const ICON_SIZE_MAX: f32 = 70.0;

/// Padding around an icon inside a toolbar button, as a fraction of the icon
/// side. The horizontal one is what makes a toolbar button wider than its
/// glyph, so it has to be part of any width budget (see [`icon_size_for_row`]).
///
/// It is generous because in the bar it doubles as the spacing that keeps the
/// buttons apart, and the bar's own fill sits behind all of it.
pub(super) const BUTTON_PAD_X_FRAC: f32 = 0.7;
pub(super) const BUTTON_PAD_Y_FRAC: f32 = 0.45;

/// Space between two buttons in the controls bar, as a fraction of the icon
/// side.
///
/// The bar is measured off the icon throughout, and this is why: the style's
/// own `item_spacing` follows the page text, and a toolbar spaced by that
/// would *shrink* when the text was enlarged - [`icon_size_for_row`] divides
/// what is left after the gaps, so wider gaps mean smaller buttons.
pub(super) const CONTROLS_GAP_FRAC: f32 = 0.15;

/// Inner margin of a bar spanning the screen - the map's controls at the top,
/// its status read-out at the bottom - as a fraction of the smaller screen
/// dimension. Wider than it is tall: a bar spans the screen, so the side gaps
/// are what keep its end content off the edges.
const BAR_MARGIN_X_FRAC: f32 = 0.02;
const BAR_MARGIN_Y_FRAC: f32 = 0.01;

/// Those margins in whole points, rounded once so a frame and the width budget
/// inside it agree to the point.
pub(super) fn bar_margin(screen: egui::Rect) -> (i8, i8) {
    let min = screen.size().min_elem();
    (
        (min * BAR_MARGIN_X_FRAC) as i8,
        (min * BAR_MARGIN_Y_FRAC) as i8,
    )
}

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
    (screen.size().min_elem() * ICON_SIZE_FRAC).clamp(TOUCH_MIN, ICON_SIZE_MAX)
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

/// Measures for egui's own `Spacing`, in body-font sizes. These are the
/// insides of a control - what [`gap`] and [`field_width`] are to the space
/// between them.
///
/// The button padding is wider than it is tall because a label needs air at
/// its ends more than above and below; the height is carried by
/// [`CONTROL_HEIGHT_EM`] and its floor instead.
///
/// These are the knobs for how chunky the app feels. Every one of them is a
/// multiple of the body font, so they hold their proportions at any
/// `text_scale`; only [`TOUCH_MIN`] is absolute.
const BUTTON_PAD_X_EM: f32 = 0.6;
const BUTTON_PAD_Y_EM: f32 = 0.3;
const ITEM_SPACING_X_EM: f32 = 0.55;
const ITEM_SPACING_Y_EM: f32 = 0.35;
const CONTROL_HEIGHT_EM: f32 = 2.6;
const CONTROL_WIDTH_EM: f32 = 3.2;
const CHECK_EM: f32 = 1.2;
const CHECK_INNER_EM: f32 = 0.7;
const INDENT_EM: f32 = 1.4;
const COMBO_EM: f32 = 8.0;

/// The scroll bar, in body-font sizes, with a floor so it stays wide enough to
/// catch a finger.
const SCROLL_BAR_EM: f32 = 0.55;
const SCROLL_BAR_MIN: f32 = 8.0;

/// egui's own body font size. Only a fallback: the style always has a `Body`
/// entry, [`crate::app::MyApp::apply_ui_style`] writing the whole set before
/// this runs.
const DEFAULT_BODY_PT: f32 = 12.5;

/// Size every interactive control off the body text, with a touch-target floor
/// under the lot.
///
/// egui's stock spacing is a set of absolute point counts, which leaves two
/// problems that this fixes together:
///
/// - A button is 18 points tall before padding. That is under half a
///   fingertip, and every page here is a page of buttons.
/// - `[ui] text_scale` scales the *font*. The padding, the checkbox glyph and
///   the minimum control height are not fonts, so they stayed where they were:
///   doubling the text size gave big letters in the same cramped rows, next to
///   a checkbox that had not moved.
///
/// Applied to the whole style rather than per page, so it reaches the dropdown
/// popups, the color pickers and the map's own popups as well as the pages.
/// The map's toolbar overrides the button padding locally, its buttons being
/// measured off the icon rather than off any text.
pub(in crate::app) fn apply_spacing(style: &mut egui::Style) {
    let font = style
        .text_styles
        .get(&egui::TextStyle::Body)
        .map_or(DEFAULT_BODY_PT, |id| id.size);
    let spacing = &mut style.spacing;
    spacing.button_padding = egui::vec2(font * BUTTON_PAD_X_EM, font * BUTTON_PAD_Y_EM);
    spacing.item_spacing = egui::vec2(font * ITEM_SPACING_X_EM, font * ITEM_SPACING_Y_EM);
    // `interact_size.y` is the floor egui puts under a button, a checkbox, a
    // radio, a drag value, a slider and a dropdown, so this one line is most
    // of what makes them all tappable.
    spacing.interact_size.y = (font * CONTROL_HEIGHT_EM).max(TOUCH_MIN);
    // The width floor matters to the widgets that show a number in a box - a
    // drag value, a color swatch - which would otherwise stay 40 points wide
    // however large the digits in them got.
    spacing.interact_size.x = (font * CONTROL_WIDTH_EM).max(TOUCH_MIN);
    spacing.icon_width = font * CHECK_EM;
    spacing.icon_width_inner = font * CHECK_INNER_EM;
    spacing.icon_spacing = font * ITEM_SPACING_X_EM;
    spacing.indent = font * INDENT_EM;
    spacing.combo_width = font * COMBO_EM;
    spacing.scroll.bar_width = (font * SCROLL_BAR_EM).max(SCROLL_BAR_MIN);
}

/// The height every interactive control is laid out at, in points.
pub(super) fn control_height(ui: &egui::Ui) -> f32 {
    ui.spacing().interact_size.y
}
