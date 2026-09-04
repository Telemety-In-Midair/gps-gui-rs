//! The adjuster: pick a thing on the screen, and move the measures that
//! shaped it, watching the app follow.
//!
//! Opened from the Settings page. It floats over every page, so a button on
//! the map, an entry on the menu page and a field on a settings page are all
//! within reach of the same picker:
//!
//! - **Pick** arms the picker. The next tap picks the smallest thing under
//!   it; a hold (or a right click) lists everything under the finger, the
//!   page included, so a gap or a margin can be picked from behind whatever
//!   sits on it.
//! - The panel then shows the picked element's measures as sliders, each in
//!   its own unit with a way to change the unit, and the app redraws with
//!   every move.
//! - **Save** writes the sheet in place, **Reload** reads it back, and
//!   **Defaults** drops every measure to what the app ships with.
//!
//! What it knows about the screen is the [`Probe`]s the theme functions
//! record while it is open: a rect, a name, and the keys behind it. There is
//! no widget tree to walk in an immediate-mode UI, so the picker is a search
//! of those, smallest first.
//!
//! Everything it draws lives in one `Area` at the top order: the catcher
//! that takes the pointer away from the pages while picking, then the list,
//! then the panel, in that order so the later ones are what the pointer
//! meets. Separate areas would reorder themselves on every click.

use crate::app::ui::text::adjust as text;
use crate::app::ui::theme::{probes, px, scale, Key, Probe};
use crate::app::ui::widgets::{feedback_label, hint};
use crate::app::MyApp;
use crate::look::{Measure, Unit};

/// How long a press is held before it is a hold rather than a tap, in
/// seconds.
const HOLD_S: f64 = 0.5;

/// The outline around a highlighted element, in text heights.
const OUTLINE_EM: f32 = 0.12;

/// The adjuster's state while it is open.
pub(in crate::app) struct Adjust {
    /// The picker is armed: the next tap picks what is under it, and nothing
    /// under the picker sees the pointer.
    picking: bool,
    /// The element being adjusted, matched against each frame's probes by
    /// name and keys so every copy of it is outlined.
    picked: Option<Picked>,
    /// The list a hold brought up: where the finger was, and what was under
    /// it.
    candidates: Option<Candidates>,
    /// The current press has already been treated as a hold, so its release
    /// is not also a tap.
    held: bool,
    /// The panel is folded down to its title row.
    folded: bool,
    /// The panel's height last frame, which is where this frame docks it.
    panel_height: f32,
    /// The hold list's rect last frame: its size is what keeps this frame's
    /// list on the screen, and a test reads it.
    list_rect: egui::Rect,
}

impl Default for Adjust {
    fn default() -> Self {
        Self {
            picking: false,
            picked: None,
            candidates: None,
            held: false,
            folded: false,
            panel_height: 0.0,
            list_rect: egui::Rect::NOTHING,
        }
    }
}

/// An element as the picker names it. Two probes of the same name and keys
/// are the same element drawn twice.
#[derive(Clone, PartialEq)]
struct Picked {
    name: &'static str,
    keys: Vec<Key>,
}

impl From<&Probe> for Picked {
    fn from(p: &Probe) -> Self {
        Picked {
            name: p.name,
            keys: p.keys.clone(),
        }
    }
}

impl Picked {
    fn matches(&self, p: &Probe) -> bool {
        self.name == p.name && self.keys == p.keys
    }
}

struct Candidates {
    at: egui::Pos2,
    list: Vec<Probe>,
}

impl Adjust {
    /// Opened with the picker armed, so the first tap already picks.
    pub(in crate::app) fn new() -> Self {
        Self {
            picking: true,
            ..Default::default()
        }
    }
}

/// The probes under `pos`, smallest first - the one drawn tightest around
/// the point is what a tap means, and the rest are what a hold lists - with
/// each element once however many times it was drawn.
fn under(all: &[Probe], pos: egui::Pos2) -> Vec<Probe> {
    let mut hits: Vec<Probe> = all
        .iter()
        .filter(|p| p.rect.contains(pos))
        .cloned()
        .collect();
    hits.sort_by(|a, b| a.rect.area().total_cmp(&b.rect.area()));
    let mut seen: Vec<Picked> = Vec::new();
    hits.retain(|p| {
        let id = Picked::from(p);
        if seen.contains(&id) {
            false
        } else {
            seen.push(id);
            true
        }
    });
    hits
}

/// What the list calls an element: its name and the keys behind it.
fn describe(p: &Probe) -> String {
    let keys: Vec<&str> = p.keys.iter().map(|k| k.path()).collect();
    format!("{} ({})", p.name, keys.join(", "))
}

/// What the panel asked for this frame, applied once the panel is drawn and
/// the adjuster's own state is no longer borrowed.
#[derive(Default)]
struct Asks {
    edits: Vec<(Key, Measure)>,
    save: bool,
    reload: bool,
    defaults: bool,
    done: bool,
}

impl MyApp {
    /// The adjuster overlay, drawn after every page so this frame's probes
    /// are all in. Nothing when it is closed.
    pub(crate) fn adjust_ui(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let Some(mut adjust) = self.adjust.take() else {
            return;
        };
        let all = probes(ctx);
        let safe = self.safe_area(ctx);
        let hold_slack = px(ctx, Key::MapDragMin);
        let mut asks = Asks::default();
        // The element the pointer is over, outlined so a tap's target is
        // known before the tap.
        let mut hover: Option<Probe> = None;
        // Where the panel sits: docked to the foot of the screen, above the
        // gesture bar, as tall as it was last frame.
        let foot = screen.bottom() - safe.bottom;
        let dock = egui::Rect::from_min_max(
            egui::pos2(screen.left(), foot - adjust.panel_height),
            egui::pos2(screen.right(), foot),
        );

        // While picking, the area itself is the catcher: egui gives a press
        // on an area to the area's own handle ahead of anything the area
        // draws over the same spot, so that handle is what has to sense the
        // tap. The content only has to make the area screen-sized. When not
        // picking the area is as big as the panel and senses nothing, so the
        // pages under it get every press.
        let sense = if adjust.picking {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::hover()
        };
        let area = egui::Area::new(egui::Id::new("adjust"))
            .order(egui::Order::Debug)
            .sense(sense)
            .fixed_pos(egui::Pos2::ZERO)
            .movable(false)
            .constrain(false)
            .show(ctx, |ui| {
                if adjust.picking {
                    ui.allocate_rect(screen, egui::Sense::hover());
                }
                if let Some(c) = &adjust.candidates {
                    // Hung from the finger, but pulled back onto the screen by
                    // the size it had last frame, and its rows wrap so a long
                    // list of keys fits a phone's width.
                    let size = adjust.list_rect.size().max(egui::Vec2::ZERO);
                    let left = c.at.x.min(screen.right() - size.x).max(screen.left());
                    let top = c.at.y.min(foot - size.y).max(screen.top());
                    let rect = egui::Rect::from_min_max(
                        egui::pos2(left, top),
                        egui::pos2(screen.right(), foot),
                    );
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    let mut choice: Option<Picked> = None;
                    let mut close = false;
                    let popup = egui::Frame::popup(ui.style()).show(&mut child, |ui| {
                        ui.set_max_width(ui.available_width());
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.label(egui::RichText::new(text::UNDER_FINGER).strong());
                        for p in &c.list {
                            let resp = ui.selectable_label(false, describe(p));
                            if resp.hovered() {
                                hover = Some(p.clone());
                            }
                            if resp.clicked() {
                                choice = Some(p.into());
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                    ui.expand_to_include_rect(popup.response.rect);
                    adjust.list_rect = popup.response.rect;
                    if let Some(p) = choice {
                        adjust.picked = Some(p);
                        adjust.picking = false;
                        close = true;
                    }
                    if close {
                        adjust.candidates = None;
                    }
                }
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(dock)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                let panel = egui::Frame::popup(ui.style()).show(&mut child, |ui| {
                    ui.set_width(ui.available_width());
                    self.adjust_panel(ui, &mut adjust, &mut asks);
                });
                ui.expand_to_include_rect(panel.response.rect);
                adjust.panel_height = panel.response.rect.height();
            });
        if adjust.picking {
            self.adjust_picker(
                ctx,
                &area.response,
                &mut adjust,
                &all,
                dock,
                hold_slack,
                &mut hover,
            );
        }

        self.adjust_outlines(ctx, &adjust, &all, hover.as_ref());

        for (key, measure) in asks.edits {
            self.set_measure(key, measure);
        }
        if asks.defaults {
            self.reset_look();
        }
        if asks.reload {
            self.load_look();
        }
        if asks.save {
            self.save_look();
        }
        if !asks.done {
            self.adjust = Some(adjust);
        }
    }

    /// The picker: a tap on the screen-sized area picks the smallest thing
    /// under it, and a hold brings up the list.
    #[allow(clippy::too_many_arguments)]
    fn adjust_picker(
        &self,
        ctx: &egui::Context,
        catch: &egui::Response,
        adjust: &mut Adjust,
        all: &[Probe],
        dock: egui::Rect,
        hold_slack: f32,
        hover: &mut Option<Probe>,
    ) {
        // A hold is a press that has lasted without moving, which no widget
        // reports, so it is read straight off the pointer.
        let (down, origin, start, latest, now) = ctx.input(|i| {
            (
                i.pointer.primary_down(),
                i.pointer.press_origin(),
                i.pointer.press_start_time(),
                i.pointer.latest_pos(),
                i.time,
            )
        });
        if let Some(pos) = latest {
            if !dock.contains(pos) && adjust.candidates.is_none() {
                *hover = under(all, pos).into_iter().next();
            }
        }
        if down && !adjust.held && adjust.candidates.is_none() {
            if let (Some(origin), Some(start), Some(latest)) = (origin, start, latest) {
                let elapsed = now - start;
                if (latest - origin).length() <= hold_slack && !dock.contains(origin) {
                    if elapsed >= HOLD_S {
                        adjust.held = true;
                        adjust.candidates = Some(Candidates {
                            at: origin,
                            list: under(all, origin),
                        });
                    } else {
                        // Nothing else asks for a frame while a finger rests
                        // still, so the clock has to.
                        ctx.request_repaint_after_secs((HOLD_S - elapsed) as f32 + 0.01);
                    }
                }
            }
        }
        if catch.clicked() && !adjust.held {
            if let Some(top) = catch
                .interact_pointer_pos()
                .and_then(|pos| under(all, pos).into_iter().next())
            {
                adjust.picked = Some((&top).into());
                adjust.picking = false;
            }
        }
        if catch.secondary_clicked() {
            if let Some(pos) = catch.interact_pointer_pos() {
                adjust.candidates = Some(Candidates {
                    at: pos,
                    list: under(all, pos),
                });
            }
        }
        // A release ends the hold, after the click it would otherwise have
        // been has been looked at.
        if !down {
            adjust.held = false;
        }
    }

    /// Outline every copy of the picked element, and the one under the
    /// pointer with its name, over everything else on the screen.
    fn adjust_outlines(
        &self,
        ctx: &egui::Context,
        adjust: &Adjust,
        all: &[Probe],
        hover: Option<&Probe>,
    ) {
        let painter = ctx.debug_painter();
        let em = scale(ctx).em;
        let width = em * OUTLINE_EM;
        if let Some(picked) = &adjust.picked {
            let stroke = egui::Stroke::new(width, self.config.ui.ok);
            for p in all.iter().filter(|p| picked.matches(p)) {
                painter.rect_stroke(
                    p.rect,
                    egui::CornerRadius::ZERO,
                    stroke,
                    egui::StrokeKind::Outside,
                );
            }
        }
        if let Some(h) = hover {
            let color = ctx.global_style().visuals.text_color();
            painter.rect_stroke(
                h.rect,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(width, color),
                egui::StrokeKind::Outside,
            );
            let galley =
                painter.layout_no_wrap(h.name.to_string(), egui::FontId::proportional(em), color);
            let pos = h.rect.left_top();
            painter.rect_filled(
                egui::Rect::from_min_size(pos, galley.size()),
                egui::CornerRadius::ZERO,
                ctx.global_style().visuals.panel_fill,
            );
            painter.galley(pos, galley, color);
        }
    }

    /// The docked panel: the picker switch and the way out on its title row,
    /// then the picked element's measures, then the sheet's buttons.
    fn adjust_panel(&self, ui: &mut egui::Ui, adjust: &mut Adjust, asks: &mut Asks) {
        ui.horizontal_wrapped(|ui| {
            let pick = ui
                .add(egui::Button::new("Pick").selected(adjust.picking))
                .on_hover_text(text::PICK_HOVER);
            if pick.clicked() {
                adjust.picking = !adjust.picking;
                adjust.candidates = None;
            }
            let status = if adjust.picking {
                text::PICKING.to_string()
            } else {
                match &adjust.picked {
                    Some(p) => p.name.to_string(),
                    None => text::NOTHING_PICKED.to_string(),
                }
            };
            ui.label(status);
            let fold = if adjust.folded { "Show" } else { "Hide" };
            if ui.button(fold).clicked() {
                adjust.folded = !adjust.folded;
            }
            if ui.button("Done").on_hover_text(text::DONE_HOVER).clicked() {
                asks.done = true;
            }
        });
        if adjust.folded {
            return;
        }
        if let Some(picked) = &adjust.picked {
            let scale = scale(ui.ctx());
            let slider_width = px(ui.ctx(), Key::SettingsSlider);
            for &key in &picked.keys {
                let mut m = self.look.get(key);
                let mut changed = false;
                ui.horizontal_wrapped(|ui| {
                    ui.label(key.path()).on_hover_text(key.doc());
                    ui.spacing_mut().slider_width = slider_width;
                    let range = m.q.unit.range();
                    let slider = egui::Slider::new(&mut m.q.value, range.start..=range.end)
                        .step_by(m.q.unit.step())
                        .clamping(egui::SliderClamping::Never)
                        .fixed_decimals(2)
                        .suffix(m.q.unit.suffix());
                    changed |= ui.add(slider).changed();
                    let mut unit = m.q.unit;
                    egui::ComboBox::from_id_salt(("adjust_unit", key))
                        .selected_text(unit.suffix())
                        .show_ui(ui, |ui| {
                            for u in Unit::ALL {
                                // The icon cannot be measured in icons.
                                if key == Key::IconSize && u == Unit::Icon {
                                    continue;
                                }
                                ui.selectable_value(&mut unit, u, u.suffix())
                                    .on_hover_text(u.describe());
                            }
                        });
                    if unit != m.q.unit {
                        m.convert(unit, &scale);
                        changed = true;
                    }
                    let default = key.default_measure();
                    let reset = ui
                        .add_enabled(m != default, egui::Button::new("Reset"))
                        .on_hover_text(text::RESET_HOVER);
                    if reset.clicked() {
                        m = default;
                        changed = true;
                    }
                });
                if changed {
                    asks.edits.push((key, m));
                }
            }
        } else {
            hint!(ui, text::NOTHING_PICKED);
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Save").on_hover_text(text::SAVE_HOVER).clicked() {
                asks.save = true;
            }
            if ui
                .button("Reload")
                .on_hover_text(text::RELOAD_HOVER)
                .clicked()
            {
                asks.reload = true;
            }
            if ui
                .button("Defaults")
                .on_hover_text(text::DEFAULTS_HOVER)
                .clicked()
            {
                asks.defaults = true;
            }
            hint!(ui, small "{}", self.look_path);
        });
        feedback_label(ui, self.config.ui, &self.look_feedback);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(name: &'static str, keys: &[Key], x: f32, y: f32, w: f32, h: f32) -> Probe {
        Probe {
            rect: egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)),
            name,
            keys: keys.to_vec(),
        }
    }

    #[test]
    fn a_tap_means_the_smallest_thing_and_a_hold_lists_the_rest() {
        let all = vec![
            probe("Page", &[Key::PageMargin], 0.0, 0.0, 400.0, 800.0),
            probe("Gap", &[Key::GapItem], 10.0, 100.0, 380.0, 6.0),
            probe("Button", &[Key::ControlHeight], 10.0, 110.0, 80.0, 40.0),
            probe(
                "Text field",
                &[Key::SettingsPath],
                100.0,
                110.0,
                200.0,
                40.0,
            ),
            // The same button drawn again elsewhere lists once.
            probe("Button", &[Key::ControlHeight], 10.0, 200.0, 80.0, 40.0),
        ];
        let hits = under(&all, egui::pos2(20.0, 120.0));
        let names: Vec<&str> = hits.iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["Button", "Page"]);

        let hits = under(&all, egui::pos2(20.0, 102.0));
        let names: Vec<&str> = hits.iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["Gap", "Page"]);

        assert!(under(&all, egui::pos2(500.0, 120.0)).is_empty());
    }

    #[test]
    fn the_list_names_the_keys() {
        let p = probe(
            "Checkbox",
            &[Key::ControlCheckSize, Key::ControlHeight],
            0.0,
            0.0,
            1.0,
            1.0,
        );
        assert_eq!(
            describe(&p),
            "Checkbox (control.check.size, control.height)"
        );
        let picked = Picked::from(&p);
        assert!(picked.matches(&p));
        assert!(!picked.matches(&probe(
            "Checkbox",
            &[Key::ControlHeight],
            0.0,
            0.0,
            1.0,
            1.0
        )));
    }

    /// The picker driven through real frames over the Settings page: what the
    /// page records is what a tap finds, and a finger held still lists
    /// everything under it, the page last.
    #[test]
    fn a_tap_picks_a_button_and_a_hold_lists_the_page_behind_it() {
        use crate::app::tests::test_app;
        use crate::app::ui::theme::publish;

        let (mut app, _cmds, _events) = test_app();
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 800.0));
        app.adjust = Some(Adjust::new());

        // One frame as the app loop runs it, returning what the page recorded
        // before the adjuster searched it.
        let frame = |app: &mut MyApp, events: Vec<egui::Event>, time: f64| -> Vec<Probe> {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut found = Vec::new();
            let _ = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                app.apply_ui_style(&ctx);
                publish(&ctx, app.look.clone(), true);
                app.settings_page(&ctx, screen);
                found = probes(&ctx);
                app.adjust_ui(&ctx, screen);
            });
            found
        };
        let modifiers = egui::Modifiers::default();
        let button = egui::PointerButton::Primary;

        // Nothing pressed yet: the page lays itself out and records its
        // probes, and the picker's catcher takes the screen.
        let recorded = frame(&mut app, vec![], 0.0);
        let target = recorded
            .iter()
            .find(|p| p.name == "Button")
            .expect("the settings page has buttons");
        let at = target.rect.center();
        let press = egui::Event::PointerButton {
            pos: at,
            button,
            pressed: true,
            modifiers,
        };
        let release = egui::Event::PointerButton {
            pos: at,
            button,
            pressed: false,
            modifiers,
        };

        // A tap: press, then release a moment later.
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(at), press.clone()],
            0.1,
        );
        frame(&mut app, vec![release.clone()], 0.2);
        let adjust = app.adjust.as_ref().expect("the adjuster stays open");
        assert!(!adjust.picking, "a pick disarms the picker");
        let picked = adjust.picked.as_ref().expect("the tap picked something");
        assert_eq!(picked.name, "Button");
        assert_eq!(
            picked.keys,
            vec![Key::ControlPadX, Key::ControlPadY, Key::ControlHeight]
        );

        // Armed again, a press held still past the hold time lists what is
        // under it instead, smallest first and the page last, and the release
        // that ends the hold is not also a tap.
        app.adjust.as_mut().unwrap().picking = true;
        frame(&mut app, vec![], 0.9);
        frame(&mut app, vec![egui::Event::PointerMoved(at), press], 1.0);
        frame(&mut app, vec![], 1.2);
        assert!(app.adjust.as_ref().unwrap().candidates.is_none());
        frame(&mut app, vec![], 1.7);
        let listed: Vec<&str> = app
            .adjust
            .as_ref()
            .unwrap()
            .candidates
            .as_ref()
            .expect("a hold brings up the list")
            .list
            .iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(listed.first(), Some(&"Button"));
        assert_eq!(listed.last(), Some(&"Page"));
        // Placed by its own size from the frame before, the list is on the
        // screen however long its rows are.
        frame(&mut app, vec![], 1.75);
        let list_rect = app.adjust.as_ref().unwrap().list_rect;
        assert!(
            screen.contains_rect(list_rect),
            "the hold list runs off the screen: {list_rect:?}"
        );
        frame(&mut app, vec![release], 1.8);
        let adjust = app.adjust.as_ref().unwrap();
        assert!(adjust.candidates.is_some());
        assert!(adjust.picking);
        assert_eq!(adjust.picked.as_ref().map(|p| p.name), Some("Button"));

        // The panel's own buttons stay reachable while the catcher has the
        // screen. Its title row starts with Pick: sweep the row from the
        // left for the first thing egui hovers that is not the area itself,
        // press it, and the picker disarms.
        app.adjust.as_mut().unwrap().candidates = None;
        frame(&mut app, vec![], 2.0);
        let dock_top = screen.bottom() - app.adjust.as_ref().unwrap().panel_height;
        let row_y = dock_top + 26.0;
        let area_ids = [
            egui::Id::new("adjust"),
            egui::Id::new("adjust").with("move"),
        ];
        let mut pick_at = None;
        let mut x = 4.0;
        while x < 120.0 && pick_at.is_none() {
            let pos = egui::pos2(x, row_y);
            frame(&mut app, vec![egui::Event::PointerMoved(pos)], 2.1);
            // Over a button egui hovers the button as the click hit and the
            // area's handle as the drag hit; over the panel's margins only
            // the handle senses a click.
            let hovered: Vec<egui::Id> =
                ctx.interaction_snapshot(|s| s.hovered.iter().copied().collect());
            let on_button = hovered.iter().any(|id| {
                !area_ids.contains(id)
                    && ctx
                        .read_response(*id)
                        .is_some_and(|r| r.sense.senses_click())
            });
            if on_button {
                pick_at = Some(pos);
            }
            x += 4.0;
        }
        let at = pick_at.expect("the title row has a button at its left");
        frame(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: at,
                button,
                pressed: true,
                modifiers,
            }],
            2.2,
        );
        frame(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: at,
                button,
                pressed: false,
                modifiers,
            }],
            2.3,
        );
        let adjust = app.adjust.as_ref().expect("Pick is not Done");
        assert!(!adjust.picking, "the press reached Pick, not the catcher");
        assert_eq!(adjust.picked.as_ref().map(|p| p.name), Some("Button"));
    }
}
