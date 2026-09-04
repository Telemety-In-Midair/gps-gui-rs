//! The measures every page is written in, and the functions that turn them
//! into points for the current screen.
//!
//! Nothing here draws, and nothing here decides a size either: the sizes are
//! the look sheet's ([`crate::look`]), and a page reads as `gap(ui,
//! Key::GapSection)` rather than as a point count. The sheet is published
//! into the egui context once per frame by [`publish`], and every function
//! below reads it back from there, so the pages need no handle to it.
//!
//! The one thing that is decided here rather than in the sheet is what the
//! sheet's units are *of*: the screen, the body text height, and the icon
//! side. They are worked out once per frame into a [`Scale`], with the two
//! physical bounds ([`TOUCH_MIN`] and the icon cap) applied on the way.
//!
//! While the adjuster is up, the same functions also record where the things
//! they measured landed - a [`Probe`] per drawn element - which is what the
//! adjuster's element picker searches.

use std::sync::Arc;

pub(super) use crate::look::Key;
use crate::look::{Look, Scale, TOUCH_MIN};

/// The scroll bar's floor, in points: wide enough to catch a finger. Physical,
/// like [`TOUCH_MIN`], and so not in the sheet.
const SCROLL_BAR_MIN: f32 = 8.0;

/// egui's own body font size. Only a fallback: the style always has a `Body`
/// entry, [`crate::app::MyApp::apply_ui_style`] writing the whole set before
/// any of this runs.
const DEFAULT_BODY_PT: f32 = 12.5;

/// What this frame is drawn with: the look, its scale for this screen, and
/// whether the adjuster wants to know where things land.
#[derive(Clone)]
struct Published {
    look: Arc<Look>,
    scale: Scale,
    probing: bool,
}

/// One drawn thing the adjuster can pick: where it landed, what to call it,
/// and the keys that shaped it.
#[derive(Clone, Debug)]
pub(super) struct Probe {
    pub(super) rect: egui::Rect,
    pub(super) name: &'static str,
    pub(super) keys: Vec<Key>,
}

fn published_id() -> egui::Id {
    egui::Id::new("look_published")
}

fn probes_id() -> egui::Id {
    egui::Id::new("look_probes")
}

/// Put the look up for this frame, before anything is drawn. The app loop
/// calls it once per frame; the pages read it back through [`px`] and the
/// functions built on it.
///
/// `probing` turns on the recording of [`Probe`]s, which the adjuster reads
/// at the end of the frame with [`probes`]. Off, the recording costs nothing.
pub(in crate::app) fn publish(ctx: &egui::Context, look: Arc<Look>, probing: bool) {
    let screen = ctx.input(|i| i.viewport_rect());
    let font = ctx
        .global_style()
        .text_styles
        .get(&egui::TextStyle::Body)
        .cloned()
        .unwrap_or_else(|| egui::FontId::proportional(DEFAULT_BODY_PT));
    let em = ctx.fonts_mut(|f| f.row_height(&font));
    let scale = look.scale(screen, em);
    ctx.data_mut(|d| {
        d.insert_temp(
            published_id(),
            Published {
                look,
                scale,
                probing,
            },
        );
        if probing {
            d.insert_temp(probes_id(), Vec::<Probe>::new());
        } else {
            d.remove_temp::<Vec<Probe>>(probes_id());
        }
    });
}

/// This frame's look and scale. Before the first [`publish`] (a test, or a
/// widget drawn outside the app loop) it is the defaults against whatever
/// screen the context reports.
fn published(ctx: &egui::Context) -> Published {
    ctx.data(|d| d.get_temp::<Published>(published_id()))
        .unwrap_or_else(|| {
            let look = Arc::new(Look::default());
            let screen = ctx.input(|i| i.viewport_rect());
            Published {
                scale: look.scale(screen, DEFAULT_BODY_PT),
                look,
                probing: false,
            }
        })
}

/// The scale this frame is measured against.
pub(super) fn scale(ctx: &egui::Context) -> Scale {
    published(ctx).scale
}

/// A key of the sheet in points for this frame.
pub(super) fn px(ctx: &egui::Context, key: Key) -> f32 {
    let p = published(ctx);
    p.look.px(key, &p.scale)
}

/// Square icon side length in points for the current screen: the sheet's
/// `icon.size` held between a fingertip and the cap.
pub(super) fn icon_size(ctx: &egui::Context) -> f32 {
    published(ctx).scale.icon
}

/// The body text height: the unit the sheet's `em` is.
pub(super) fn em(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body)
}

/// Vertical space of one of the sheet's gaps.
pub(super) fn gap(ui: &mut egui::Ui, key: Key) {
    let space = px(ui.ctx(), key);
    let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), space));
    probe(ui.ctx(), rect, "Gap", &[key]);
    ui.add_space(space);
}

/// Margin between a page's body and the screen edge, in points.
pub(super) fn page_margin(ctx: &egui::Context) -> f32 {
    px(ctx, Key::PageMargin)
}

/// Inset of a floating corner control from the screen edge, in points.
pub(super) fn corner_margin(ctx: &egui::Context) -> f32 {
    px(ctx, Key::CornerMargin)
}

/// The inner margins of a bar spanning the screen, in whole points, rounded
/// once so a frame and the width budget inside it agree to the point.
pub(super) fn bar_margin(ctx: &egui::Context) -> (i8, i8) {
    (
        px(ctx, Key::BarMarginX) as i8,
        px(ctx, Key::BarMarginY) as i8,
    )
}

/// Largest icon side that keeps a row of `count` icon buttons within `avail`
/// points, so no button may claim more than its equal share of the width.
///
/// A button is wider than its icon by `bar.button.pad.x` on each side, and
/// the buttons are separated by `spacing`; `avail` is expected to already
/// have the enclosing frame's margin taken off. The result is capped at the
/// usual [`icon_size`], so the row only shrinks below it on a screen too
/// narrow to hold the whole set - the share of the width is a ceiling, not a
/// target. The fingertip floor does not apply here: a button that overflows
/// the screen is worse than a small one.
pub(super) fn icon_size_for_row(
    ctx: &egui::Context,
    avail: f32,
    spacing: f32,
    count: usize,
) -> f32 {
    let base = icon_size(ctx);
    if count == 0 {
        return base;
    }
    // The padding as a share of the icon, so a smaller icon carries a
    // smaller pad: exact when the pad is written in icons, as it is by
    // default, and a fair approximation in any other unit.
    let pad = px(ctx, Key::BarButtonPadX) / base;
    let count = count as f32;
    let per_button = (avail - (count - 1.0) * spacing) / count;
    let fit = per_button / (1.0 + 2.0 * pad);
    base.min(fit.max(1.0))
}

/// The height every interactive control is laid out at, in points.
pub(super) fn control_height(ui: &egui::Ui) -> f32 {
    ui.spacing().interact_size.y
}

/// Whether the adjuster is recording where things land this frame.
pub(super) fn probing(ctx: &egui::Context) -> bool {
    published(ctx).probing
}

/// Record that something shaped by `keys` was drawn at `rect`, for the
/// adjuster's picker. Free while the adjuster is closed.
pub(super) fn probe(ctx: &egui::Context, rect: egui::Rect, name: &'static str, keys: &[Key]) {
    if !probing(ctx) {
        return;
    }
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<Probe>>(probes_id())
            .push(Probe {
                rect,
                name,
                keys: keys.to_vec(),
            });
    });
}

/// Everything recorded so far this frame, in drawing order.
pub(super) fn probes(ctx: &egui::Context) -> Vec<Probe> {
    ctx.data(|d| d.get_temp::<Vec<Probe>>(probes_id()))
        .unwrap_or_default()
}

/// Size every interactive control off the body text, with a touch-target
/// floor under the lot.
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
/// The measures are the sheet's `control` block, taken off the body font size
/// rather than the row height so the controls follow the font exactly.
/// Applied to the whole style rather than per page, so it reaches the dropdown
/// popups, the color pickers and the map's own popups as well as the pages.
/// The map's toolbar overrides the button padding locally, its buttons being
/// measured off the icon rather than off any text.
pub(in crate::app) fn apply_spacing(style: &mut egui::Style, look: &Look, screen: egui::Rect) {
    let font = style
        .text_styles
        .get(&egui::TextStyle::Body)
        .map_or(DEFAULT_BODY_PT, |id| id.size);
    let scale = look.scale(screen, font);
    let px = |key: Key| look.px(key, &scale);
    let spacing = &mut style.spacing;
    spacing.button_padding = egui::vec2(px(Key::ControlPadX), px(Key::ControlPadY));
    spacing.item_spacing = egui::vec2(px(Key::ControlSpacingX), px(Key::ControlSpacingY));
    // `interact_size.y` is the floor egui puts under a button, a checkbox, a
    // radio, a drag value, a slider and a dropdown, so this one line is most
    // of what makes them all tappable.
    spacing.interact_size.y = px(Key::ControlHeight).max(TOUCH_MIN);
    // The width floor matters to the widgets that show a number in a box - a
    // drag value, a color swatch - which would otherwise stay 40 points wide
    // however large the digits in them got.
    spacing.interact_size.x = px(Key::ControlWidth).max(TOUCH_MIN);
    spacing.icon_width = px(Key::ControlCheckSize);
    spacing.icon_width_inner = px(Key::ControlCheckMark);
    spacing.icon_spacing = px(Key::ControlSpacingX);
    spacing.indent = px(Key::ControlIndent);
    spacing.combo_width = px(Key::ControlCombo);
    spacing.scroll.bar_width = px(Key::ControlScrollbar).max(SCROLL_BAR_MIN);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::{Measure, Unit};

    fn screen(w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))
    }

    #[test]
    fn a_row_shrinks_only_when_it_has_to() {
        let ctx = egui::Context::default();
        let look = Arc::new(Look::default());
        let wide = screen(1200.0, 800.0);
        ctx.data_mut(|d| {
            d.insert_temp(
                published_id(),
                Published {
                    scale: look.scale(wide, 12.5),
                    look: look.clone(),
                    probing: false,
                },
            )
        });
        let base = icon_size(&ctx);
        assert_eq!(base, 40.0);
        // Plenty of room: the cap is the usual size.
        assert_eq!(icon_size_for_row(&ctx, 1200.0, 6.0, 7), base);
        // Seven buttons in 300 points, each 2.4 icons wide with 6 between:
        // (300 - 36) / 7 / 2.4.
        let fit = icon_size_for_row(&ctx, 300.0, 6.0, 7);
        assert!((fit - 15.714).abs() < 0.01, "{fit}");
        assert_eq!(icon_size_for_row(&ctx, 300.0, 6.0, 0), base);
    }

    #[test]
    fn spacing_follows_the_font_with_a_fingertip_floor() {
        let mut style = egui::Style::default();
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(10.0));
        let mut look = Look::default();
        apply_spacing(&mut style, &look, screen(400.0, 800.0));
        assert_eq!(style.spacing.button_padding, egui::vec2(6.0, 3.0));
        // 2.6em of 10 is 26, under the floor.
        assert_eq!(style.spacing.interact_size.y, TOUCH_MIN);
        assert_eq!(style.spacing.scroll.bar_width, SCROLL_BAR_MIN);

        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(20.0));
        look.set(Key::ControlPadX, Measure::new(1.0, Unit::Em));
        apply_spacing(&mut style, &look, screen(400.0, 800.0));
        assert_eq!(style.spacing.button_padding, egui::vec2(20.0, 6.0));
        assert_eq!(style.spacing.interact_size.y, 52.0);
        assert_eq!(style.spacing.scroll.bar_width, 11.0);
    }

    #[test]
    fn probes_are_recorded_only_while_probing() {
        let ctx = egui::Context::default();
        let rect = screen(10.0, 10.0);
        probe(&ctx, rect, "Thing", &[Key::PageMargin]);
        assert!(probes(&ctx).is_empty());

        // Inside a frame, as the app loop is: the text height `publish`
        // measures needs the fonts, which egui only sets up there.
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let ctx = ui.ctx();
            publish(ctx, Arc::new(Look::default()), true);
            probe(ctx, rect, "Thing", &[Key::PageMargin]);
            probe(ctx, rect, "Other", &[Key::GapItem, Key::GapHair]);
            let found = probes(ctx);
            assert_eq!(found.len(), 2);
            assert_eq!(found[1].name, "Other");
            assert_eq!(found[1].keys, vec![Key::GapItem, Key::GapHair]);

            // The next frame starts empty, and a frame without the adjuster
            // records nothing at all.
            publish(ctx, Arc::new(Look::default()), true);
            assert!(probes(ctx).is_empty());
            publish(ctx, Arc::new(Look::default()), false);
            probe(ctx, rect, "Thing", &[Key::PageMargin]);
            assert!(probes(ctx).is_empty());
        });
    }
}
