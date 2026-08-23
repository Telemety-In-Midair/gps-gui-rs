//! The interactive map page: the floating controls bar, the marker info
//! popups, and the offline region-download selection and progress.
//!
//! The map picture itself is painted in [`crate::app::ui::mapdraw`]; what is
//! declared here is everything laid over it.

use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use egui::emath::Rot2;
use walkers::{Position, Projector};

use crate::app::ui::icons;
use crate::app::ui::mapdraw::{default_position, rotate_pos};
use crate::app::ui::text::map as text;
use crate::app::ui::theme::{
    corner_margin, gap, icon_size_for, icon_size_for_row, BUTTON_PAD_X_FRAC, BUTTON_PAD_Y_FRAC,
    CONTROLS_GAP_FRAC, GAP_ITEM,
};
use crate::app::ui::widgets::{button, floating, icon_button, icon_button_pulse};
use crate::app::{ease_heading, ping_reason, MarkerKind, MyApp, RegionSelect, ROTATE_TAU};
use crate::offline;
use crate::points::age_text;
use crate::tiles::MapLayer;

/// Refuse offline downloads bigger than this many tiles (tile-server
/// courtesy; shrink the box or lower the max zoom instead).
const MAX_REGION_TILES: u64 = 10_000;

/// Rough average size of a cached OSM tile, for the download estimate.
const TILE_SIZE_ESTIMATE_KB: u64 = 15;

/// Smallest box drag that counts as a region selection rather than a tap, as a
/// fraction of the smaller screen dimension.
const MIN_DRAG_FRAC: f32 = 0.025;

/// Inner margin of the controls bar frame, as a fraction of the smaller screen
/// dimension. Wider than it is tall: the bar spans the screen, so the side gaps
/// are what keep the end buttons off the edges.
const CONTROLS_MARGIN_X_FRAC: f32 = 0.02;
const CONTROLS_MARGIN_Y_FRAC: f32 = 0.01;

/// Zoom levels past the current view a region download offers, and the highest
/// it will ever reach.
const REGION_ZOOM_HEADROOM: u8 = 2;
const REGION_ZOOM_MAX: u8 = 17;

/// Popup positions and paddings, as fractions of the icon size (itself a
/// fraction of the screen), so a popup stays a touch target on a phone and
/// does not look lost on a desktop.
const POPUP_PAD_X_FRAC: f32 = 0.35;
const POPUP_PAD_Y_FRAC: f32 = 0.25;
const CENTER_MENU_GAP_FRAC: f32 = 0.12;
const CENTER_MENU_WIDTH_FRAC: f32 = 3.5;
/// How far below the controls bar the center menu and the select hint hang.
const UNDER_BAR_FRAC: f32 = 1.8;
const HINT_UNDER_BAR_FRAC: f32 = 1.6;
/// How far above the marker its info bubble floats.
const MARKER_INFO_LIFT_FRAC: f32 = 0.35;

/// How close a double-click must land to a marker to select it: one icon side,
/// so the reach is the same as a toolbar button's touch target.
fn marker_hit_radius(screen: egui::Rect) -> f32 {
    icon_size_for(screen)
}

/// The controls bar margins in whole points, rounded once so the frame and the
/// width budget inside it agree to the point.
fn controls_margin(screen: egui::Rect) -> (i8, i8) {
    let min = screen.size().min_elem();
    (
        (min * CONTROLS_MARGIN_X_FRAC) as i8,
        (min * CONTROLS_MARGIN_Y_FRAC) as i8,
    )
}

impl MyApp {
    /// The interactive map page: full-bleed map with the floating controls.
    pub(crate) fn map_page(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        // Box selection needs the screen to map 1:1 onto the north-up tile
        // space (the projector knows nothing about our post-rotation), so
        // heading-up rotation pauses while a region is being selected.
        let selecting = !matches!(self.select, RegionSelect::Inactive);

        // Tracking mode reframes the view between the user and the beacon and
        // returns the bearing to turn the map to (beacon up). It centers and
        // zooms as a side effect. Paused while a region box is being drawn.
        let track_bearing = if selecting {
            None
        } else {
            self.tracking_orientation(ctx, screen)
        };

        // The angle the map should be turned to: the tracking bearing wins;
        // otherwise heading-up uses the live heading. Anything else leaves the
        // map north-up.
        let target_heading = track_bearing.or(if self.heading_up && !selecting {
            self.effective_heading()
        } else {
            None
        });

        // Ease the drawn angle toward the target each frame (shortest way round
        // the circle), so the map glides rather than stepping between updates. We
        // keep requesting repaints until it settles.
        let rotation = match target_heading {
            Some(target) => {
                let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
                let current = self.smoothed_heading.unwrap_or(target);
                let (next, remaining) = ease_heading(current, target, dt, ROTATE_TAU);
                self.smoothed_heading = Some(next);
                if remaining > 0.05 {
                    ctx.request_repaint();
                }
                Some(Rot2::from_angle(-next.to_radians()))
            }
            None => {
                self.smoothed_heading = None;
                None
            }
        };

        // Heading-up (without tracking, which already centered on the midpoint)
        // locks the map to the current position: it stays centered on you
        // (re-following each frame), which also makes dragging a no-op so the
        // rotated view can't be panned off. Zoom (buttons) still works.
        if track_bearing.is_none() && self.heading_up && self.current.is_some() {
            self.map_memory.follow_my_position();
        }

        // A rotated map needs to paint past the screen edges, otherwise the
        // corners rotate away to nothing. Overscan to a square whose side is the
        // screen diagonal - large enough to cover the screen at any angle.
        let map_rect = if rotation.is_some() {
            egui::Rect::from_center_size(screen.center(), egui::Vec2::splat(screen.size().length()))
        } else {
            screen
        };

        // Full-bleed map in the background layer. It lives in its own Area (not a
        // CentralPanel) so its clip rect can extend past the screen for overscan.
        egui::Area::new(egui::Id::new("map"))
            .order(egui::Order::Background)
            .fixed_pos(map_rect.min)
            .movable(false)
            .constrain(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(map_rect);
                self.map(ui, map_rect, rotation, screen);
            });

        // Double-click/tap a marker to show its name and time since last update.
        // Skipped while a region box is being drawn (double-clicks belong to it).
        if !selecting {
            self.marker_info(ctx, screen, map_rect, rotation);
        }

        // The box-selection layer sits between the map and the controls.
        self.select_overlay(ctx, screen);

        // Controls float on top in the foreground layer, so they keep pointer
        // priority over the (interactive) map behind them. The fill spans the
        // status-bar area; the top inset pushes the buttons clear of it.
        let top = self.top_inset(ctx);
        egui::Area::new(egui::Id::new("controls"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::Pos2::ZERO)
            .movable(false)
            .constrain(false)
            .show(ctx, |ui| {
                let (margin_x, margin_y) = controls_margin(screen);
                egui::Frame::NONE
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(egui::Margin::symmetric(margin_x, margin_y))
                    .show(ui, |ui| {
                        // The frame's margin is part of the screen width, so the
                        // content gets what is left of it. Setting the full width
                        // here would push the bar (and the button row it sizes)
                        // past the right edge by the margin.
                        ui.set_width(screen.width() - 2.0 * f32::from(margin_x));
                        ui.add_space(top);
                        self.controls(ui, screen);
                    });
            });

        // Selection hint / download confirmation, floating over everything.
        self.select_ui(ctx, screen);

        // The center button's marker list, when it has been held open.
        self.center_menu_ui(ctx, screen);
    }

    /// The floating controls bar: one icon button per thing the map can do.
    ///
    /// Every button whose glyph changes shows the state the press switches
    /// *to*, which is what a toolbar icon without a label has to do to be
    /// readable.
    fn controls(&mut self, ui: &mut egui::Ui, screen: egui::Rect) {
        // Taken off the icon rather than left on the style's text-derived
        // spacing, so enlarging the page text does not shrink the toolbar.
        // Derived from the *uncapped* icon size, which is the ceiling
        // `icon_size_for_row` starts from, so the two are not circular.
        let spacing = icon_size_for(screen) * CONTROLS_GAP_FRAC;
        ui.spacing_mut().item_spacing.x = spacing;
        // Which of the optional buttons are in the row this frame. Decided up
        // front, before anything is laid out, because the button count is what
        // sizes the row - and reused where the buttons are drawn, so the count
        // cannot disagree with what ends up in the bar.
        let show_rotate = self.has_direction() && self.tracking_beacon.is_none();
        let zoom_buttons = if cfg!(target_os = "android") { 0 } else { 2 };
        // Center, track, layer, paths and the page menu are always there.
        let buttons = 5 + usize::from(show_rotate) + zoom_buttons;
        // No button may take more than a 1/buttons share of the bar, padding and
        // spacing included, so a full row always fits the screen instead of
        // running off the right edge when the set grows.
        let icon = icon_size_for_row(screen, ui.available_width(), spacing, buttons);
        ui.spacing_mut().button_padding =
            egui::vec2(icon * BUTTON_PAD_X_FRAC, icon * BUTTON_PAD_Y_FRAC);
        // egui lays a horizontal row out left-to-right and can't center it in a
        // single pass: its `main_align` is ignored and the row just fills the
        // width of any centering parent. So pad the left by half the leftover
        // space, using the row width measured last frame (it stays constant once
        // the button set is fixed). `add_space` counts as an item, so drop one
        // item spacing to keep the gap even on both sides.
        let pad = if self.controls_width > 0.0 {
            ((ui.available_width() - self.controls_width) * 0.5 - spacing).max(0.0)
        } else {
            0.0
        };
        ui.horizontal(|ui| {
            ui.add_space(pad);
            let row = ui.horizontal(|ui| {
                self.center_button(ui, icon);
                if show_rotate {
                    self.rotate_button(ui, icon);
                } else if !self.has_direction() {
                    // No orientation available: nothing to toggle, so stay
                    // north-up and drop any stale heading-up flag.
                    self.heading_up = false;
                }
                self.track_button(ui, icon);
                self.layer_button(ui, icon);
                // Zoom buttons are desktop-only; on mobile pinch-zoom handles
                // it, so the buttons would only crowd the small toolbar.
                if !cfg!(target_os = "android") {
                    if icon_button(ui, icon, icons::zoom_in())
                        .on_hover_text(text::ZOOM_IN)
                        .clicked()
                    {
                        let _ = self.map_memory.zoom_in();
                    }
                    if icon_button(ui, icon, icons::zoom_out())
                        .on_hover_text(text::ZOOM_OUT)
                        .clicked()
                    {
                        let _ = self.map_memory.zoom_out();
                    }
                }
                self.paths_button(ui, icon);
                // The region download is started from the Settings page, which
                // jumps back here with the box selection already active.

                // The page menu sits inline, right after the other buttons.
                self.page_menu(ui, icon);
            });
            // Remember the row's own width (the inner group, excluding the pad)
            // so the next frame can center it.
            self.controls_width = row.response.rect.width();
        });
    }

    /// Center on the user marker if we have a fix; otherwise fall back to the
    /// next available marker (the beacon). With no marker at all the button
    /// pulses and does nothing when clicked. Holding it (or right-clicking on
    /// desktop) opens the list of markers instead, so any of them can be
    /// picked.
    fn center_button(&mut self, ui: &mut egui::Ui, icon: f32) {
        let targets = self.center_targets();
        let center = icon_button_pulse(
            ui,
            icon,
            icons::center(),
            targets.is_empty().then_some(self.config.ui.pulse),
        )
        .on_hover_text(text::CENTER_HOVER);
        if center.clicked() {
            // A plain tap always goes to you, falling back to the first beacon
            // when there is no fix yet.
            if let Some(&(kind, pos)) = targets.first() {
                self.center_on(ui.ctx(), pos, kind == MarkerKind::You);
            }
        }
        // A long touch does not also register as a click, so the two paths
        // cannot both fire from one press.
        if center.secondary_clicked() && !targets.is_empty() {
            self.center_menu = true;
        }
    }

    /// Heading-up on/off. Only shown with a direction source (compass, or GPS
    /// course over ground) and while not tracking, which owns the map's
    /// orientation - the track button is the way out of that mode.
    fn rotate_button(&mut self, ui: &mut egui::Ui, icon: f32) {
        let (glyph, hint) = if self.heading_up {
            (icons::north(), text::NORTH_UP)
        } else {
            (icons::heading(), text::HEADING_UP)
        };
        if icon_button(ui, icon, glyph).on_hover_text(hint).clicked() {
            self.heading_up = !self.heading_up;
        }
    }

    /// Tracking mode: keep the user and a beacon framed together. Tapping
    /// enters the mode on the first beacon, then walks along the beacon list,
    /// and the press after the last one leaves the mode - this button is the
    /// only way in and out.
    ///
    /// It frames the two together, so it needs BOTH a live user position and
    /// at least one beacon; with either missing the button pulses and does
    /// nothing, entering with a piece missing being a lock on a view it cannot
    /// frame.
    fn track_button(&mut self, ui: &mut egui::Ui, icon: f32) {
        let can_track = self.can_track();
        let pulse = (!can_track).then_some(self.config.ui.pulse);
        if icon_button_pulse(ui, icon, icons::track(), pulse)
            .on_hover_text(self.tracking_hint())
            .clicked()
            && can_track
        {
            self.cycle_tracking();
        }
    }

    /// Base-layer toggle between the standard map and the topographic one.
    fn layer_button(&mut self, ui: &mut egui::Ui, icon: f32) {
        let (glyph, hint, next) = match self.layer {
            MapLayer::Standard => (icons::topo(), text::TOPO_MAP, MapLayer::Topo),
            MapLayer::Topo => (icons::map(), text::STANDARD_MAP, MapLayer::Standard),
        };
        if icon_button(ui, icon, glyph).on_hover_text(hint).clicked() {
            self.layer = next;
        }
    }

    /// Paths on/off: a master switch over both recorded paths, which only ever
    /// hides - the per-path settings decide which of the two a switched-on map
    /// draws. It leaves the line to the beacon and its distance label alone,
    /// those being what is worth keeping when the map is too busy to read.
    /// Session state, so a glance at a clear map does not overwrite either
    /// setting.
    fn paths_button(&mut self, ui: &mut egui::Ui, icon: f32) {
        let (glyph, hint) = if self.show_paths {
            (icons::path_off(), text::HIDE_PATHS)
        } else {
            (icons::path(), text::SHOW_PATHS)
        };
        if icon_button(ui, icon, glyph).on_hover_text(hint).clicked() {
            self.show_paths = !self.show_paths;
        }
    }

    /// The center button's marker list, opened by holding the button (or
    /// right-clicking it on desktop). Picking an entry centers on that marker;
    /// a plain tap of the button never opens this and just goes to you.
    fn center_menu_ui(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        if !self.center_menu {
            return;
        }
        let targets = self.center_targets();
        // Every listed marker had a position when the list was opened; if the
        // last one has since gone (beacon disconnected), there is nothing left
        // to offer.
        if targets.is_empty() {
            self.center_menu = false;
            return;
        }

        let icon = icon_size_for(screen);
        let top = self.top_inset(ctx);
        // Resolve each entry's name up front (a remote's comes from the config),
        // so the popup closure need not reach back into `self`.
        let labelled: Vec<(MarkerKind, Position, String)> = targets
            .iter()
            .map(|&(kind, pos)| (kind, pos, self.marker_label(kind)))
            .collect();
        let mut chosen: Option<(MarkerKind, Position)> = None;
        let mut close = false;
        floating(
            ctx,
            "center_menu",
            egui::Order::Foreground,
            // Just under the controls bar the button sits in.
            egui::Pos2::new(screen.center().x, top + icon * UNDER_BAR_FRAC),
            egui::Align2::CENTER_TOP,
            false,
            |ui| {
                ui.spacing_mut().button_padding =
                    egui::vec2(icon * POPUP_PAD_X_FRAC, icon * POPUP_PAD_Y_FRAC);
                ui.spacing_mut().item_spacing.y = icon * CENTER_MENU_GAP_FRAC;
                ui.set_min_width(icon * CENTER_MENU_WIDTH_FRAC);
                ui.label(egui::RichText::new("Center on").strong());
                for (kind, pos, label) in &labelled {
                    if ui.button(label).clicked() {
                        chosen = Some((*kind, *pos));
                    }
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            },
        );

        if let Some((kind, pos)) = chosen {
            // Centering on yourself follows the live position; a beacon is a
            // one-off recenter.
            self.center_on(ctx, pos, kind == MarkerKind::You);
            close = true;
        }
        if close {
            self.center_menu = false;
        }
    }

    /// Handle double-click/tap selection of a map marker and draw the info
    /// popup (name + time since last update) for the selected one.
    ///
    /// Marker screen positions are computed the same way the `GpsLayer` plugin
    /// draws them: project with the map's projector, then apply the heading-up
    /// rotation (about the screen center) when the map is rotated. A double-click
    /// that misses every marker dismisses the popup.
    fn marker_info(
        &mut self,
        ctx: &egui::Context,
        screen: egui::Rect,
        map_rect: egui::Rect,
        rotation: Option<Rot2>,
    ) {
        let my_position = self.current.unwrap_or_else(default_position);
        let projector = Projector::new(map_rect, &self.map_memory, my_position);
        let origin = screen.center();
        let to_screen = |pos: Position| {
            let p = projector.project(pos).to_pos2();
            match rotation {
                Some(rot) => rotate_pos(p, rot, origin),
                None => p,
            }
        };

        // Present markers, nearest-first is resolved below by distance: you,
        // the connected board, then every remote node.
        let mut markers: Vec<(MarkerKind, Option<Position>)> = vec![
            (MarkerKind::You, self.current),
            (MarkerKind::Beacon, self.beacon),
        ];
        markers.extend(
            self.remotes
                .iter()
                .map(|(&addr, node)| (MarkerKind::Remote(addr), node.last_pos())),
        );

        // On a double-click, pick the closest marker within the hit radius; a
        // miss clears the current selection.
        let double = ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
        if double {
            if let Some(click) = ctx.input(|i| i.pointer.interact_pos()) {
                self.selected_marker = markers
                    .iter()
                    .filter_map(|(kind, pos)| {
                        pos.as_ref().map(|p| (*kind, to_screen(*p).distance(click)))
                    })
                    .filter(|(_, dist)| *dist <= marker_hit_radius(screen))
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(kind, _)| kind);
            }
        }

        let Some(kind) = self.selected_marker else {
            return;
        };
        // A remote node also carries whether it is currently without a fix,
        // and when it was last heard at all. Without that, a marker left at
        // the last position a node managed reads as its current one.
        let mut no_fix = None;
        let (pos, time) = match kind {
            MarkerKind::You => (self.current, self.current_time),
            MarkerKind::Beacon => (self.beacon, self.beacon_time),
            MarkerKind::Remote(addr) => match self.remotes.get(&addr) {
                Some(node) => {
                    no_fix = node.no_fix.map(|ping| (ping, node.heard));
                    (node.last_pos(), node.last_time())
                }
                None => (None, None),
            },
        };
        // The marker may have vanished (e.g. beacon disconnected) since it was
        // selected; drop the popup if so.
        let Some(pos) = pos else {
            self.selected_marker = None;
            return;
        };
        let anchor = to_screen(pos);
        let label = self.marker_label(kind);

        let now = SystemTime::now();
        let age = match time {
            Some(t) => format!("Updated {} ago", age_text(now, t)),
            None => text::NO_UPDATE.to_string(),
        };
        // The node is still on the air, just without a position: say so, and
        // say when it last spoke, so the marker above is read as where it was
        // rather than where it is.
        let no_fix = no_fix.map(|(ping, heard)| {
            let when = match heard {
                Some(t) => format!(", heard {} ago", age_text(now, t)),
                None => String::new(),
            };
            format!("No fix now: {}{when}", ping_reason(ping))
        });

        floating(
            ctx,
            "marker_info",
            egui::Order::Foreground,
            // Clear of the marker it points at, by about a third of an icon side.
            egui::pos2(
                anchor.x,
                anchor.y - icon_size_for(screen) * MARKER_INFO_LIFT_FRAC,
            ),
            egui::Align2::CENTER_BOTTOM,
            true,
            |ui| {
                ui.label(egui::RichText::new(&label).strong());
                ui.label(age);
                if let Some(note) = &no_fix {
                    ui.label(note);
                }
            },
        );
        // Keep the elapsed-time text live even without new fixes.
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    /// The box-drag layer for the offline region download. It sits between the
    /// map (Background) and the floating controls (Foreground): drags land here
    /// instead of panning the map, while the buttons above stay clickable.
    fn select_overlay(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        if matches!(self.select, RegionSelect::Inactive) {
            return;
        }
        let my_position = self.current.unwrap_or_else(default_position);
        let color = self.config.colors.track;
        let fill = color.gamma_multiply(0.15);
        let stroke = egui::Stroke::new(2.0, color);
        let paint_box = |ui: &egui::Ui, rect: egui::Rect| {
            ui.painter().rect(
                rect,
                egui::CornerRadius::ZERO,
                fill,
                stroke,
                egui::StrokeKind::Middle,
            );
        };

        egui::Area::new(egui::Id::new("region_select"))
            .order(egui::Order::Middle)
            .fixed_pos(egui::Pos2::ZERO)
            .movable(false)
            .constrain(false)
            .show(ctx, |ui| match self.select {
                RegionSelect::Inactive => {}
                RegionSelect::Picking {
                    mut start,
                    mut current,
                } => {
                    let resp = ui.allocate_rect(screen, egui::Sense::drag());
                    if resp.drag_started() {
                        start = resp.interact_pointer_pos();
                    }
                    if let Some(p) = resp.interact_pointer_pos() {
                        current = Some(p);
                    }
                    if let (Some(s), Some(c)) = (start, current) {
                        paint_box(ui, egui::Rect::from_two_pos(s, c));
                    }

                    self.select = match (resp.drag_stopped(), start, current) {
                        (true, Some(s), Some(c)) => {
                            let rect = egui::Rect::from_two_pos(s, c);
                            // Ignore taps and hairline drags.
                            let min_drag = screen.size().min_elem() * MIN_DRAG_FRAC;
                            if rect.width() >= min_drag && rect.height() >= min_drag {
                                // Same clip rect and position the map was
                                // drawn with (selection forces north-up, so
                                // the map rect is exactly the screen).
                                let projector =
                                    Projector::new(screen, &self.map_memory, my_position);
                                // Offer two zoom levels past the current view.
                                let max_zoom = (self.map_memory.zoom().ceil() as u8)
                                    .saturating_add(REGION_ZOOM_HEADROOM)
                                    .min(REGION_ZOOM_MAX);
                                RegionSelect::Confirm {
                                    a: projector.unproject(rect.min.to_vec2()),
                                    b: projector.unproject(rect.max.to_vec2()),
                                    max_zoom,
                                }
                            } else {
                                RegionSelect::Picking {
                                    start: None,
                                    current: None,
                                }
                            }
                        }
                        (true, ..) => RegionSelect::Picking {
                            start: None,
                            current: None,
                        },
                        (false, ..) => RegionSelect::Picking { start, current },
                    };
                }
                RegionSelect::Confirm { a, b, .. } => {
                    let projector = Projector::new(screen, &self.map_memory, my_position);
                    paint_box(
                        ui,
                        egui::Rect::from_two_pos(
                            projector.project(a).to_pos2(),
                            projector.project(b).to_pos2(),
                        ),
                    );
                }
            });
    }

    /// The floating hint while picking a box, and the confirm panel (tile
    /// count, max-zoom stepper) once one is chosen.
    fn select_ui(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let top = self.top_inset(ctx);
        // Both panels are measured off the icon size, which is itself a
        // fraction of the screen: the hint clears the controls bar it sits
        // under, and the confirm panel's buttons stay a touch target.
        let icon = icon_size_for(screen);
        match self.select {
            RegionSelect::Inactive => {}
            RegionSelect::Picking { .. } => {
                let mut cancel = false;
                floating(
                    ctx,
                    "select_hint",
                    egui::Order::Foreground,
                    egui::Pos2::new(screen.center().x, top + icon * HINT_UNDER_BAR_FRAC),
                    egui::Align2::CENTER_TOP,
                    false,
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.label(text::SELECT_HINT);
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    },
                );
                if cancel {
                    self.select = RegionSelect::Inactive;
                }
            }
            RegionSelect::Confirm { a, b, mut max_zoom } => {
                let mut close = false;
                let error_color = self.config.ui.error;
                // Topo tiles stop at zoom 17; don't offer levels the server 404s.
                let layer_max = self.layer.max_zoom();
                floating(
                    ctx,
                    "select_confirm",
                    egui::Order::Foreground,
                    screen.center(),
                    egui::Align2::CENTER_CENTER,
                    false,
                    |ui| {
                        ui.spacing_mut().button_padding =
                            egui::vec2(icon * POPUP_PAD_X_FRAC, icon * POPUP_PAD_Y_FRAC);
                        ui.label(egui::RichText::new(text::DOWNLOAD_TITLE).strong());
                        gap(ui, GAP_ITEM);
                        ui.horizontal(|ui| {
                            ui.label("Max zoom:");
                            if button!(ui, "-", enabled: max_zoom > 1).clicked() {
                                max_zoom -= 1;
                            }
                            ui.label(format!("{max_zoom}"));
                            if button!(ui, "+", enabled: max_zoom < layer_max).clicked() {
                                max_zoom += 1;
                            }
                        });
                        let count = offline::tile_count(a, b, max_zoom);
                        ui.label(format!(
                            "{count} tiles, ~{} MB",
                            (count * TILE_SIZE_ESTIMATE_KB).div_ceil(1024).max(1)
                        ));
                        if count > MAX_REGION_TILES {
                            ui.colored_label(error_color, text::TOO_MANY_TILES);
                        }
                        gap(ui, GAP_ITEM);
                        ui.horizontal(|ui| {
                            let can_download =
                                count <= MAX_REGION_TILES && self.cache_dir.is_some();
                            if button!(ui, "Download", enabled: can_download).clicked() {
                                if let Some(dir) = &self.cache_dir {
                                    self.download = Some(offline::spawn_download(
                                        dir.clone(),
                                        self.layer,
                                        offline::region_tiles(a, b, max_zoom),
                                        ctx.clone(),
                                    ));
                                }
                                close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    },
                );
                self.select = if close {
                    RegionSelect::Inactive
                } else {
                    RegionSelect::Confirm { a, b, max_zoom }
                };
            }
        }
    }

    /// Progress readout for the offline tile download, floating bottom-left
    /// on every page.
    pub(crate) fn download_ui(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let Some(progress) = self.download.clone() else {
            return;
        };
        let bottom = self.bottom_inset(ctx);
        // Inset from the corner by the same fraction the floating page toggle
        // uses, so the two read as the same distance from the edge.
        let margin = corner_margin(screen);
        floating(
            ctx,
            "download_progress",
            egui::Order::Foreground,
            screen.left_bottom() + egui::vec2(margin, -(margin + bottom)),
            egui::Align2::LEFT_BOTTOM,
            false,
            |ui| {
                let done = progress.done.load(Ordering::Relaxed);
                let failed = progress.failed.load(Ordering::Relaxed);
                let status = if progress.finished() {
                    if failed > 0 {
                        format!("Offline tiles: done, {failed} of {} failed", progress.total)
                    } else {
                        format!("Offline tiles: all {} done", progress.total)
                    }
                } else if failed > 0 {
                    format!("Offline tiles: {done}/{} ({failed} failed)", progress.total)
                } else {
                    format!("Offline tiles: {done}/{}", progress.total)
                };
                ui.horizontal(|ui| {
                    ui.label(status);
                    let label = if progress.finished() { "OK" } else { "Cancel" };
                    if ui.button(label).clicked() {
                        progress.cancel.store(true, Ordering::Relaxed);
                        self.download = None;
                    }
                });
            },
        );
    }
}
