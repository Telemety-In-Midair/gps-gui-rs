//! The menu pages and the floating corner toggle that opens them.
//!
//! The menu is a page rather than a dropdown because it is the app's only
//! navigation: on a phone a list of full-width touch targets is what that has
//! to be, and a page is the one thing that always has room for one.
//!
//! It is two pages rather than one for the same reason. A touch target is a
//! large thing and a phone screen is a short one, so a list of every page runs
//! out of screen before it runs out of pages; the first page holds the three
//! that are looked at while moving, and [`Page::More`] holds the rest. Both are
//! drawn by [`MyApp::menu_list`] and behave alike - the corner toggle is an X
//! on either, and leaves the menu entirely.

use super::icons;
use super::text::map as text;
use super::theme::{corner_margin, icon_size, page_margin, probe, px, Key};
use super::widgets::content_page;
use crate::app::{MyApp, Page};

/// How long the hamburger takes to cross-fade into the X, in seconds.
const TOGGLE_FADE_S: f32 = 0.15;

/// What shapes a button on the menu page, for the adjuster. The sheet writes
/// them against the icon rather than the body text because these are touch
/// targets first - the point of the page is that they are comfortable to hit.
const MENU_BUTTON_KEYS: [Key; 5] = [
    Key::MenuRowWidth,
    Key::MenuMargin,
    Key::MenuRowHeight,
    Key::MenuRowGap,
    Key::MenuText,
];

/// One row on a menu page: where it goes, what it is called, and its glyph.
/// A row that goes to another menu page ([`Page::More`], and the way back) is
/// no different from one that goes to a destination - the whole of navigation
/// is these rows.
type MenuItem = (Page, &'static str, egui::ImageSource<'static>);

/// The main menu, in order: the pages worth a tap while walking, then the way
/// to the rest. [`Page::Menu`] is deliberately absent - it is the page doing
/// the listing, so a button back to it would go nowhere.
fn main_items() -> [MenuItem; 4] {
    [
        (Page::Map, "Map", icons::map()),
        (Page::Status, "Status", icons::status()),
        (Page::Bluetooth, "Bluetooth", icons::bluetooth()),
        (Page::More, "More", icons::more()),
    ]
}

/// The More page: everything the main menu does not list, in the order it used
/// to appear there.
fn more_items() -> [MenuItem; 4] {
    [
        (Page::Points, "Points", icons::points()),
        (Page::Logging, "Logging", icons::log()),
        (Page::Settings, "Settings", icons::settings()),
        (Page::Radio, "Radio", icons::radio()),
    ]
}

/// The row that returns from the More page to the main menu. Last on the page
/// and not a destination, so it is kept apart from [`more_items`]: that list is
/// what More *holds*, which is what the main menu's More row is marked
/// against.
fn back_item() -> MenuItem {
    (Page::Menu, "Back", icons::back())
}

/// Whether a page is one the More page lists. The main menu's More row stands
/// in for all of them, so it is the row marked as current while any of them is
/// the page behind the menu.
fn on_more_page(page: Page) -> bool {
    more_items().iter().any(|(p, ..)| *p == page)
}

/// Whether a page is one of the menu's own, rather than a destination. Both
/// are dismissed by the corner toggle and neither is ever the page the menu
/// was opened from.
fn is_menu(page: Page) -> bool {
    matches!(page, Page::Menu | Page::More)
}

impl MyApp {
    /// The button that opens the menu page, and closes it again. Rendered
    /// inline in the map controls bar and in the floating corner toggle on
    /// every other page - including the menu page itself, where it is what
    /// leaves without picking anything. The glyph crossfades from the
    /// hamburger to an X once the menu is up.
    pub(super) fn page_menu(&mut self, ui: &mut egui::Ui, icon: f32) {
        let text_color = ui.visuals().text_color();
        // Transparent base image: it reserves the icon-sized hit area and owns
        // the click; the visible glyph is painted on top so it can crossfade
        // between the hamburger and the X.
        let base = egui::Image::new(icons::menu())
            .fit_to_exact_size(egui::vec2(icon, icon))
            .tint(egui::Color32::TRANSPARENT);
        let resp = ui.add(egui::Button::image(base));
        if resp.clicked() {
            // From either menu page this leaves the menu altogether rather
            // than stepping back through it: the X says "done here", and the
            // More page has its own Back row for the other reading.
            if is_menu(self.page) {
                self.page = self.menu_from;
            } else {
                self.menu_from = self.page;
                self.page = Page::Menu;
            }
        }

        // Eased open/close crossfade. `animate_bool_with_time` keeps requesting
        // repaints until it settles. The two places this button is drawn share
        // the animation id, so the glyph carries on across the frame where the
        // map's inline copy hands over to the corner one.
        let open = is_menu(self.page);
        let rect = egui::Rect::from_center_size(resp.rect.center(), egui::vec2(icon, icon));
        let t = ui.ctx().animate_bool_with_time(
            egui::Id::new("page_menu_icon_anim"),
            open,
            TOGGLE_FADE_S,
        );
        egui::Image::new(icons::menu())
            .tint(text_color.gamma_multiply(1.0 - t))
            .paint_at(ui, rect);
        egui::Image::new(icons::close())
            .tint(text_color.gamma_multiply(t))
            .paint_at(ui, rect);

        resp.on_hover_text(if open {
            text::MENU_CLOSE
        } else {
            text::MENU_OPEN
        });
    }

    /// The main menu, as a page of its own: [`main_items`] as one large button
    /// each, centered on an otherwise empty screen. The page it was opened
    /// from is marked, and the corner toggle floating over it (an X by now) is
    /// what returns there.
    pub(crate) fn menu_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        self.menu_list(ctx, "menu", screen, &main_items());
    }

    /// The second menu page: [`more_items`], then the row back to the main
    /// menu. Identical to [`MyApp::menu_page`] in everything but its list, so
    /// arriving on it is not a change of mode - the same rows in the same
    /// places, one level down.
    pub(crate) fn more_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let mut items = more_items().to_vec();
        items.push(back_item());
        self.menu_list(ctx, "more", screen, &items);
    }

    /// One menu page's worth of rows. `id` names its `Area`, so the two menu
    /// pages keep their own layout state.
    fn menu_list(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        screen: egui::Rect,
        items: &[MenuItem],
    ) {
        let safe = self.safe_area(ctx);
        let margin = page_margin(ctx);
        // As wide as the screen less the page margin and the menu's own,
        // up to the sheet's ceiling: a row is a touch target, and on a phone
        // the whole width is what makes it one that cannot be missed.
        let width = px(ctx, Key::MenuRowWidth)
            .min(screen.width() - 2.0 * (margin + px(ctx, Key::MenuMargin)))
            .max(px(ctx, Key::MenuText));
        let row = egui::vec2(width, px(ctx, Key::MenuRowHeight));
        let row_gap = px(ctx, Key::MenuRowGap);
        let text_size = px(ctx, Key::MenuText);
        let item_gap = px(ctx, Key::GapItem);
        content_page(ctx, id, screen, safe, |ui| {
            // Center the column vertically by hand: the page lives in an `Area`
            // and so has no height of its own to align against. What is left to
            // share out is the screen less the frame's two margins, the
            // safe-area insets, and the gap `content_page` has already laid
            // down under the top one.
            let count = items.len() as f32;
            let content = count * row.y + (count - 1.0) * row_gap;
            let used = 2.0 * margin + safe.top + safe.bottom + item_gap;
            ui.add_space(((screen.height() - used - content) / 2.0).max(0.0));

            // The button font, which the glyph beside it is sized to as well,
            // so label and icon keep their proportions on any screen.
            ui.style_mut().text_styles.insert(
                egui::TextStyle::Button,
                egui::FontId::proportional(text_size),
            );
            ui.spacing_mut().item_spacing.y = row_gap;
            // Drawn in the theme's strong text rather than its body text,
            // frame included: solarized holds its monotones close together,
            // and a page that is nothing but these rows wants them to read
            // at a glance in sunlight.
            let strong = ui.visuals().strong_text_color();
            let visuals = ui.visuals_mut();
            for w in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered] {
                w.fg_stroke.color = strong;
                w.bg_stroke = egui::Stroke::new(1.5, strong);
            }
            // A top-down centered layout both centers each button in the page
            // and centers the glyph and label inside the button, which is what
            // makes `min_size` alone enough to size a row.
            ui.vertical_centered(|ui| {
                for (page, label, src) in items {
                    let image = egui::Image::new(src.clone())
                        .fit_to_exact_size(egui::vec2(text_size, text_size))
                        .tint(strong);
                    let button = egui::Button::image_and_text(image, *label)
                        .selected(self.menu_marks(*page))
                        .min_size(row);
                    let resp = ui.add(button);
                    probe(ui.ctx(), resp.rect, "Menu button", &MENU_BUTTON_KEYS);
                    if resp.clicked() {
                        self.page = *page;
                    }
                }
            });
        });
    }

    /// Whether a row is marked as the page behind the menu. The More row is
    /// marked for any of the pages it holds, so the menu says where you are
    /// even when the page itself is one level down.
    fn menu_marks(&self, target: Page) -> bool {
        if target == Page::More {
            on_more_page(self.menu_from)
        } else {
            self.menu_from == target
        }
    }

    /// Floating menu button in the top-right corner. Used on every page but the
    /// map, where it lives at the right end of the controls bar instead.
    pub(crate) fn page_toggle(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let size = icon_size(ctx);
        let top = self.top_inset(ctx);
        // Corner inset as a fraction of the screen, so the button stays clear
        // of the edge on any size (a fixed few points crowds a dense screen).
        let margin = corner_margin(ctx);
        // Square padding, not the toolbar's wide-and-short pair: this button is
        // on its own, so there is nothing for the extra width to space it from.
        let pad = px(ctx, Key::CornerPad);
        let area = egui::Area::new(egui::Id::new("page_toggle"))
            // Float above the (Background) page content it sits over.
            .order(egui::Order::Tooltip)
            .fixed_pos(egui::Pos2::new(screen.right() - margin, top + margin))
            .pivot(egui::Align2::RIGHT_TOP)
            .movable(false)
            .constrain(false)
            .show(ctx, |ui| {
                ui.spacing_mut().button_padding = egui::Vec2::splat(pad);
                self.page_menu(ui, size);
            });
        probe(
            ctx,
            area.response.rect,
            "Corner toggle",
            &[Key::CornerMargin, Key::CornerPad, Key::IconSize],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::test_app;

    /// Which menu page a page is reached from.
    #[derive(PartialEq, Debug)]
    enum Home {
        /// The menu itself: not reached from a list, it is the list.
        Navigation,
        Main,
        More,
    }

    /// Said a second time, as a `match`, so that adding a [`Page`] variant
    /// stops compiling here until it has been decided which menu holds it.
    /// An unlisted page is not a small bug: it cannot be reached at all.
    fn home(page: Page) -> Home {
        match page {
            Page::Menu | Page::More => Home::Navigation,
            Page::Map | Page::Status | Page::Bluetooth => Home::Main,
            Page::Points | Page::Logging | Page::Settings | Page::Radio => Home::More,
        }
    }

    /// Every page is on exactly one menu, and no menu lists a page twice.
    #[test]
    fn every_page_is_listed_once() {
        let main = main_items();
        let more = more_items();
        let listed: Vec<Page> = main
            .iter()
            .chain(more.iter())
            .map(|(page, ..)| *page)
            .collect();
        for page in &listed {
            let count = listed.iter().filter(|p| *p == page).count();
            assert_eq!(count, 1, "{page:?} is listed {count} times");
            let expected = if *page == Page::More {
                Home::Main
            } else {
                home(*page)
            };
            let found = if main.iter().any(|(p, ..)| p == page) {
                Home::Main
            } else {
                Home::More
            };
            assert_eq!(found, expected, "{page:?} is on the wrong menu");
        }
        // The two lists plus the More row that joins them: every page but the
        // main menu, which is what does the listing.
        assert_eq!(listed.len(), 8);
        assert!(!listed.contains(&Page::Menu));
    }

    /// The More page ends with the way back, and it is the only row there that
    /// is not a page More holds.
    #[test]
    fn more_page_ends_with_the_way_back() {
        let (page, ..) = back_item();
        assert_eq!(page, Page::Menu);
        assert!(!on_more_page(Page::Menu));
        assert!(on_more_page(Page::Radio));
        assert!(!on_more_page(Page::Map));
    }

    /// The More row is marked while the page behind the menu is one of the
    /// ones it holds, so the menu still says where you are one level down.
    #[test]
    fn more_row_stands_in_for_its_pages() {
        let (mut app, ..) = test_app();

        app.menu_from = Page::Radio;
        assert!(app.menu_marks(Page::More));
        assert!(!app.menu_marks(Page::Map));

        app.menu_from = Page::Map;
        assert!(!app.menu_marks(Page::More));
        assert!(app.menu_marks(Page::Map));
    }

    /// Both menu pages are the menu as far as the corner toggle is concerned.
    #[test]
    fn both_menu_pages_are_navigation() {
        assert!(is_menu(Page::Menu));
        assert!(is_menu(Page::More));
        assert!(!is_menu(Page::Bluetooth));
        assert_eq!(home(Page::More), Home::Navigation);
    }
}
