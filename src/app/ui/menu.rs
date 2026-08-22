//! The menu page and the floating corner toggle that opens it.
//!
//! The menu is a page rather than a dropdown because it is the app's only
//! navigation: on a phone a list of full-width touch targets is what that has
//! to be, and a page is the one thing that always has room for one.

use super::icons;
use super::text::map as text;
use super::theme::{corner_margin, em, icon_size_for, page_margin, GAP_ITEM, TOGGLE_PAD_FRAC};
use super::widgets::content_page;
use crate::app::{MyApp, Page};

/// Menu-page measures, as fractions of the icon size (which is itself a
/// fraction of the screen, see [`icon_size_for`]): the height of a button, its
/// width, the gap between two of them, and the text size inside one.
///
/// Written against the icon rather than the body text because these are touch
/// targets first - the point of the page is that they are comfortable to hit.
const ROW_H_FRAC: f32 = 1.3;
const ROW_W_FRAC: f32 = 5.0;
const ROW_GAP_FRAC: f32 = 0.3;
const TEXT_FRAC: f32 = 0.45;

/// How long the hamburger takes to cross-fade into the X, in seconds.
const TOGGLE_FADE_S: f32 = 0.15;

/// Every page in menu order, each with its label and icon. Drives the menu
/// page. [`Page::Menu`] is deliberately absent: it is the page doing the
/// listing, so a button back to it would go nowhere.
fn page_items() -> [(Page, &'static str, egui::ImageSource<'static>); 7] {
    [
        (Page::Map, "Map", icons::map()),
        (Page::Points, "Points", icons::points()),
        (Page::Status, "Status", icons::status()),
        (Page::Beacon, "Beacon", icons::beacon()),
        (Page::Logging, "Logging", icons::log()),
        (Page::Settings, "Settings", icons::settings()),
        (Page::Radio, "Radio", icons::radio()),
    ]
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
            if self.page == Page::Menu {
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
        let open = self.page == Page::Menu;
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

    /// The menu, as a page of its own: one large button per entry of
    /// [`page_items`], centered on an otherwise empty screen. The page it was
    /// opened from is marked, and the corner toggle floating over it (an X by
    /// now) is what returns there.
    pub(crate) fn menu_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let top = self.top_inset(ctx);
        let bottom = self.bottom_inset(ctx);
        let margin = page_margin(screen);
        let icon = icon_size_for(screen);
        let row = egui::vec2(icon * ROW_W_FRAC, icon * ROW_H_FRAC);
        let row_gap = icon * ROW_GAP_FRAC;
        let text_size = icon * TEXT_FRAC;
        let items = page_items();
        content_page(ctx, "menu", screen, top, |ui| {
            // Center the column vertically by hand: the page lives in an `Area`
            // and so has no height of its own to align against. What is left to
            // share out is the screen less the frame's two margins, the
            // safe-area insets, and the gap `content_page` has already laid
            // down under the top one.
            let count = items.len() as f32;
            let content = count * row.y + (count - 1.0) * row_gap;
            let used = 2.0 * margin + top + bottom + em(ui) * GAP_ITEM;
            ui.add_space(((screen.height() - used - content) / 2.0).max(0.0));

            // The button font, which the glyph beside it is sized to as well,
            // so label and icon keep their proportions on any screen.
            ui.style_mut().text_styles.insert(
                egui::TextStyle::Button,
                egui::FontId::proportional(text_size),
            );
            ui.spacing_mut().item_spacing.y = row_gap;
            // A top-down centered layout both centers each button in the page
            // and centers the glyph and label inside the button, which is what
            // makes `min_size` alone enough to size a row.
            ui.vertical_centered(|ui| {
                for (page, label, src) in items {
                    let image = egui::Image::new(src)
                        .fit_to_exact_size(egui::vec2(text_size, text_size))
                        .tint(ui.visuals().text_color());
                    let selected = self.menu_from == page;
                    let button = egui::Button::image_and_text(image, label)
                        .selected(selected)
                        .min_size(row);
                    if ui.add(button).clicked() {
                        self.page = page;
                    }
                }
            });
        });
    }

    /// Floating menu button in the top-right corner. Used on every page but the
    /// map, where it lives at the right end of the controls bar instead.
    pub(crate) fn page_toggle(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let size = icon_size_for(screen);
        let top = self.top_inset(ctx);
        // Corner inset as a fraction of the screen, so the button stays clear
        // of the edge on any size (a fixed few points crowds a dense screen).
        let margin = corner_margin(screen);
        egui::Area::new(egui::Id::new("page_toggle"))
            // Float above the (Background) page content it sits over.
            .order(egui::Order::Tooltip)
            .fixed_pos(egui::Pos2::new(screen.right() - margin, top + margin))
            .pivot(egui::Align2::RIGHT_TOP)
            .movable(false)
            .constrain(false)
            .show(ctx, |ui| {
                // Square padding, not the toolbar's wide-and-short pair: this
                // button is on its own, so there is nothing for the extra width
                // to space it from.
                ui.spacing_mut().button_padding = egui::Vec2::splat(size * TOGGLE_PAD_FRAC);
                self.page_menu(ui, size);
            });
    }
}
