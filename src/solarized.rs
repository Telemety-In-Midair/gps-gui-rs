//! Solarized, as the app's two themes.
//!
//! Ethan Schoonover's palette (<https://github.com/altercation/solarized>) is
//! sixteen fixed colors: eight accents, and eight monotones running from the
//! darkest ground to the lightest. The two themes are that run read in
//! opposite directions - a light page is `base3` down to `base00`, a dark one
//! `base03` up to `base0` - which is why both are built here from one set of
//! roles (`Roles`) rather than written out twice. The accents keep their
//! values in both: the palette is picked so they read against either end.
//!
//! [`visuals`] is a whole theme in the form egui wants it.
//! `MyApp::apply_ui_style` pushes it in whenever the theme
//! changes and lays the `[ui]` color overrides on top, so an override is a
//! change *to* solarized rather than a replacement of it.
//!
//! Which of the two is drawn is `[ui] theme` ([`crate::config::ThemeChoice`]),
//! and the default is the light one.

use egui::{Color32, Stroke, Theme, Visuals};

/// Darkest ground: the dark theme's page.
pub const BASE03: Color32 = Color32::from_rgb(0x00, 0x2b, 0x36);
/// The dark theme's raised surfaces.
pub const BASE02: Color32 = Color32::from_rgb(0x07, 0x36, 0x42);
/// Secondary content on dark; frames and separators on light.
pub const BASE01: Color32 = Color32::from_rgb(0x58, 0x6e, 0x75);
/// The light theme's body text.
pub const BASE00: Color32 = Color32::from_rgb(0x65, 0x7b, 0x83);
/// The dark theme's body text.
pub const BASE0: Color32 = Color32::from_rgb(0x83, 0x94, 0x96);
/// Emphasized content on dark; frames and separators on dark's opposite.
pub const BASE1: Color32 = Color32::from_rgb(0x93, 0xa1, 0xa1);
/// The light theme's raised surfaces.
pub const BASE2: Color32 = Color32::from_rgb(0xee, 0xe8, 0xd5);
/// Lightest ground: the light theme's page.
pub const BASE3: Color32 = Color32::from_rgb(0xfd, 0xf6, 0xe3);

/// Accent: yellow.
pub const YELLOW: Color32 = Color32::from_rgb(0xb5, 0x89, 0x00);
/// Accent: orange. Warnings.
pub const ORANGE: Color32 = Color32::from_rgb(0xcb, 0x4b, 0x16);
/// Accent: red. Errors, and the "no" on the Status page.
pub const RED: Color32 = Color32::from_rgb(0xdc, 0x32, 0x2f);
/// Accent: magenta.
pub const MAGENTA: Color32 = Color32::from_rgb(0xd3, 0x36, 0x82);
/// Accent: violet.
pub const VIOLET: Color32 = Color32::from_rgb(0x6c, 0x71, 0xc4);
/// Accent: blue. Links, the text cursor and the selection.
pub const BLUE: Color32 = Color32::from_rgb(0x26, 0x8b, 0xd2);
/// Accent: cyan.
pub const CYAN: Color32 = Color32::from_rgb(0x2a, 0xa1, 0x98);
/// Accent: green. The "yes" on the Status page.
pub const GREEN: Color32 = Color32::from_rgb(0x85, 0x99, 0x00);

/// How far the selection fill is pulled back from [`BLUE`] toward the page, so
/// selected text stays readable in it - most of the way, the accent being far
/// stronger than the monotones it has to sit between. One figure for both
/// themes: the pull is toward the ground, which is what differs.
const SELECTION_MIX: f32 = 0.8;

/// How far a control's fill is pulled from [`Roles::surface`] toward
/// [`Roles::line`] when it is hovered, and when it is held down. Toward the
/// frame color rather than toward black or white, so one pair of figures
/// lightens on the dark theme and darkens on the light one.
const HOVER_MIX: f32 = 0.35;
const ACTIVE_MIX: f32 = 0.7;

/// What the monotone run is used *for*. The same five jobs in both themes;
/// only which end of the run fills them changes.
struct Roles {
    /// The page itself, and the popups over it.
    ground: Color32,
    /// Anything raised off the page: a button at rest, a text field, a striped
    /// row.
    surface: Color32,
    /// Frames, separators, and the shading of a control being used.
    line: Color32,
    /// Body text.
    text: Color32,
    /// A step past the body text: headings, and the label of a pressed
    /// control.
    strong: Color32,
}

impl Roles {
    /// The run read light-to-dark.
    const LIGHT: Self = Self {
        ground: BASE3,
        surface: BASE2,
        line: BASE1,
        text: BASE00,
        strong: BASE01,
    };

    /// The same run read dark-to-light.
    const DARK: Self = Self {
        ground: BASE03,
        surface: BASE02,
        line: BASE01,
        text: BASE0,
        strong: BASE1,
    };

    fn of(theme: Theme) -> Self {
        match theme {
            Theme::Light => Self::LIGHT,
            Theme::Dark => Self::DARK,
        }
    }
}

/// One widget state: its fill, its frame, and the weight and color of its
/// text. The corner radius and expansion are left as egui had them - those are
/// shape, not palette.
fn paint(w: &mut egui::style::WidgetVisuals, fill: Color32, frame: Color32, fg: Stroke) {
    w.bg_fill = fill;
    w.weak_bg_fill = fill;
    w.bg_stroke = Stroke::new(1.0, frame);
    w.fg_stroke = fg;
}

/// The solarized theme for `theme`, as egui visuals.
///
/// Built on top of egui's own light or dark visuals, so everything that is not
/// a color - corner radii, shadows, the handle shapes, the transfer function
/// text is rendered with - stays what egui chose for that side.
pub fn visuals(theme: Theme) -> Visuals {
    let r = Roles::of(theme);
    let mut v = theme.default_visuals();

    v.panel_fill = r.ground;
    v.window_fill = r.ground;
    v.window_stroke = Stroke::new(1.0, r.line);
    // The striped rows of a grid, a text field's well, and the ground behind
    // code: all of them the one step off the page.
    v.faint_bg_color = r.surface;
    v.extreme_bg_color = r.surface;
    v.code_bg_color = r.surface;

    v.hyperlink_color = BLUE;
    v.warn_fg_color = ORANGE;
    v.error_fg_color = RED;
    v.selection = egui::style::Selection {
        bg_fill: BLUE.lerp_to_gamma(r.ground, SELECTION_MIX),
        stroke: Stroke::new(1.0, BLUE),
    };
    v.text_cursor.stroke = Stroke::new(2.0, BLUE);

    // Not a control: the page's own text, its separators and its indent lines.
    paint(
        &mut v.widgets.noninteractive,
        r.ground,
        r.line,
        Stroke::new(1.0, r.text),
    );
    // A frame on the resting state as well as the used ones: solarized holds
    // its grounds close together on purpose, and without one a button is hard
    // to find on the page it sits on.
    paint(
        &mut v.widgets.inactive,
        r.surface,
        r.line,
        Stroke::new(1.0, r.text),
    );
    paint(
        &mut v.widgets.hovered,
        r.surface.lerp_to_gamma(r.line, HOVER_MIX),
        r.line,
        Stroke::new(1.5, r.strong),
    );
    paint(
        &mut v.widgets.active,
        r.surface.lerp_to_gamma(r.line, ACTIVE_MIX),
        r.strong,
        Stroke::new(2.0, r.strong),
    );
    // An open dropdown reads as hovered rather than as pressed: it is waiting
    // on the pointer, not under it.
    paint(
        &mut v.widgets.open,
        r.surface.lerp_to_gamma(r.line, HOVER_MIX),
        r.line,
        Stroke::new(1.0, r.text),
    );

    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One channel of a gamma sRGB color, linearized for [`luminance`].
    fn linear(c: u8) -> f32 {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// WCAG relative luminance.
    fn luminance(c: Color32) -> f32 {
        0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
    }

    /// WCAG contrast ratio, 1.0 (identical) to 21.0 (black on white).
    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// Solarized is a low-contrast palette by design, so this is not the 4.5
    /// of WCAG AA: it is a floor that the *roles* have not been swapped or
    /// collapsed - reading body text off its own ground, for instance.
    const MIN_CONTRAST: f32 = 3.0;

    #[test]
    fn each_theme_reads_its_own_way_round() {
        let light = visuals(Theme::Light);
        let dark = visuals(Theme::Dark);
        assert!(!light.dark_mode);
        assert!(dark.dark_mode);
        assert_eq!(light.panel_fill, BASE3);
        assert_eq!(dark.panel_fill, BASE03);
        // The light page is the lighter of the two, and its text the darker.
        assert!(luminance(light.panel_fill) > luminance(dark.panel_fill));
        assert!(luminance(light.text_color()) < luminance(dark.text_color()));
    }

    #[test]
    fn text_reads_on_the_page_and_on_a_button() {
        for theme in [Theme::Light, Theme::Dark] {
            let v = visuals(theme);
            let text = v.text_color();
            for ground in [v.panel_fill, v.widgets.inactive.weak_bg_fill] {
                let ratio = contrast(text, ground);
                assert!(
                    ratio >= MIN_CONTRAST,
                    "{theme:?}: text on {ground:?} is only {ratio:.2}:1"
                );
            }
        }
    }

    #[test]
    fn a_button_stands_off_the_page_and_moves_when_used() {
        for theme in [Theme::Light, Theme::Dark] {
            let v = visuals(theme);
            let w = &v.widgets;
            // The fill alone is a small step - hence the frame, which is the
            // part that actually finds the button.
            assert_ne!(w.inactive.weak_bg_fill, v.panel_fill, "{theme:?}");
            assert_ne!(w.inactive.bg_stroke.color, v.panel_fill, "{theme:?}");
            // Rest, hover and press are three distinguishable fills.
            assert_ne!(w.hovered.weak_bg_fill, w.inactive.weak_bg_fill, "{theme:?}");
            assert_ne!(w.active.weak_bg_fill, w.hovered.weak_bg_fill, "{theme:?}");
        }
    }

    #[test]
    fn the_selection_is_visible_and_still_has_text_in_it() {
        for theme in [Theme::Light, Theme::Dark] {
            let v = visuals(theme);
            let text = contrast(v.text_color(), v.selection.bg_fill);
            assert!(text >= MIN_CONTRAST, "{theme:?}: text on it is {text:.2}:1");
            // Pulled that far toward the page, it still has to be a mark on it.
            let page = contrast(v.selection.bg_fill, v.panel_fill);
            assert!(page >= 1.15, "{theme:?}: it is only {page:.2}:1 off the page");
        }
    }
}
