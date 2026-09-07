//! The interactive map page: the controls bar along the foot of the screen
//! and the strip that unfolds its buttons, the zoom column and the color key
//! that go with them, the marker info popups, and the offline region-download
//! selection and progress.
//!
//! The map picture itself is painted in [`crate::app::ui::mapdraw`] and the
//! status bar that stacks on the controls in [`crate::app::ui::statusbar`];
//! what is declared here is everything else laid over it.

use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use egui::emath::Rot2;
use walkers::sources::TileSource;
use walkers::{Position, Projector};

use crate::app::ui::icons;
use crate::app::ui::mapdraw::{default_position, rotate_pos};
use crate::app::ui::text::map as text;
use crate::app::ui::theme::{
    bar_margin, corner_margin, em, gap, icon_size, icon_size_for_row, probe, px, Key,
};
use crate::app::ui::widgets::{button, floating, icon_button, icon_button_pulse};
use crate::app::{ease_heading, ping_reason, MarkerKind, MyApp, RegionSelect, ROTATE_TAU};
use crate::config::remote_color;
use crate::offline;
use crate::points::age_text;
use crate::tiles::MapLayer;

/// Refuse offline downloads bigger than this many tiles (tile-server
/// courtesy; shrink the box or lower the max zoom instead).
const MAX_REGION_TILES: u64 = 10_000;

/// Zoom levels past the current view a region download offers, and the highest
/// it will ever reach.
const REGION_ZOOM_HEADROOM: u8 = 2;
const REGION_ZOOM_MAX: u8 = 17;

/// How long the button row takes to rise out of the strip, or sink back
/// into it. The same beat as the menu glyph's crossfade.
const BAR_FOLD_S: f32 = 0.15;

/// What shapes the bar's button row, for the adjuster.
const TOOLBAR_KEYS: [Key; 4] = [
    Key::IconSize,
    Key::BarButtonPadX,
    Key::BarButtonPadY,
    Key::BarGap,
];

/// How close a tap must land to a marker to select it: one icon side, so
/// the reach is the same as a toolbar button's touch target.
fn marker_hit_radius(ctx: &egui::Context) -> f32 {
    icon_size(ctx)
}

/// The padding of a button in one of the map's popups, which are measured off
/// the icon so a popup stays a touch target on a phone and does not look lost
/// on a desktop.
fn popup_padding(ctx: &egui::Context) -> egui::Vec2 {
    egui::vec2(px(ctx, Key::MapPopupPadX), px(ctx, Key::MapPopupPadY))
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
        let mut tapped = None;
        egui::Area::new(egui::Id::new("map"))
            .order(egui::Order::Background)
            .fixed_pos(map_rect.min)
            .movable(false)
            .constrain(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(map_rect);
                tapped = self.map(ui, map_rect, rotation, screen);
            });

        // Tap a marker to show its name and time since last update. Skipped
        // while a region box is being drawn (taps belong to it).
        if !selecting {
            self.marker_info(ctx, screen, map_rect, rotation, tapped);
        }

        // The box-selection layer sits between the map and the controls.
        self.select_overlay(ctx, screen);

        // The bars stack at the foot of the screen, in the foreground layer so
        // they keep pointer priority over the (interactive) map behind them:
        // the controls bar on the edge, and the signal read-out on top of it
        // when that is turned on. The controls go first because the read-out
        // is placed off the height they measure. Both are drawn before the
        // transient popups so a confirmation still lands on top of them.
        self.map_controls_bar(ctx, screen);
        self.map_status_bar(ctx, screen);
        // The zoom column and the key go with the buttons: folded away, the
        // map is left to itself.
        if self.map_bar_open {
            self.zoom_column(ctx, screen);
            if self.config.map.show_key {
                self.map_key(ctx, screen);
            }
        }
        self.attribution_ui(ctx, screen);

        // Selection hint / download confirmation, floating over everything.
        self.select_ui(ctx, screen);

        // The center button's marker list, when it has been held open.
        self.center_menu_ui(ctx, screen);
    }

    /// The fill behind the map's bars, at the opacity the settings ask for.
    /// The status bar and the key share it, so the three read as one surface.
    pub(in crate::app::ui) fn bar_fill(&self, ctx: &egui::Context) -> egui::Color32 {
        ctx.global_style()
            .visuals
            .panel_fill
            .gamma_multiply(self.config.map.bar_opacity)
    }

    /// The controls bar along the foot of the screen: a strip with a chevron
    /// at its right end, and above it, once the strip has been pressed, the
    /// row of buttons. Folded, the strip is all there is.
    ///
    /// The row rises out of the strip rather than appearing: it is laid out
    /// at full size every frame it is on the way and clipped to the share of
    /// its height the animation has reached, with the strip covering the
    /// rest. An `Area` places itself by the size it had *last* frame, which
    /// would paint a growing bar one frame late at the bottom edge, so this
    /// one is placed by its top edge instead, worked out from the two heights
    /// measured last frame and this frame's share of the row - which is the
    /// height it is about to lay out, so the edge lands where it is meant to.
    /// The gesture-bar inset is part of the frame, so the fill reaches the
    /// screen edge and the strip still clears it.
    pub(crate) fn map_controls_bar(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let openness = ctx.animate_bool_with_time(
            egui::Id::new("map_bar_openness"),
            self.map_bar_open,
            BAR_FOLD_S,
        );
        let shown = (openness * self.bar_row_height).round();
        let top = screen.bottom() - (self.bar_foot_height + shown);
        let bottom = self.bottom_inset(ctx);
        let (margin_x, margin_y) = bar_margin(ctx);
        let fill = self.bar_fill(ctx);
        let bar = egui::Area::new(egui::Id::new("controls"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(screen.left(), top))
            .movable(false)
            .constrain(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(fill)
                    .inner_margin(egui::Margin {
                        left: margin_x,
                        right: margin_x,
                        top: margin_y,
                        bottom: margin_y.saturating_add(bottom as i8),
                    })
                    .show(ui, |ui| {
                        // The frame's margin is part of the screen width, so the
                        // content gets what is left of it. Setting the full width
                        // here would push the bar (and the button row it sizes)
                        // past the right edge by the margin.
                        ui.set_width(screen.width() - 2.0 * f32::from(margin_x));
                        // The row sits straight on the strip; the buttons'
                        // own padding is the space between the two.
                        ui.spacing_mut().item_spacing.y = 0.0;
                        if openness > 0.0 {
                            self.controls_block(ui, shown);
                        }
                        self.bar_strip(ui, openness);
                    });
            });
        probe(
            ctx,
            bar.response.rect,
            "Controls bar",
            &[Key::BarMarginX, Key::BarMarginY],
        );
        self.controls_height = bar.response.rect.height();
        self.bar_foot_height = self.controls_height - shown;
    }

    /// The button row's place in the bar: `shown` points of it, clipped from
    /// the top, with the buttons laid out at their full size behind the clip
    /// so their height can be measured whatever share of it is on show. A
    /// button clipped away cannot be pressed: egui interacts with what is
    /// inside the clip and nothing else.
    fn controls_block(&mut self, ui: &mut egui::Ui, shown: f32) {
        let width = ui.available_width();
        let origin = ui.next_widget_position();
        let block = egui::Rect::from_min_size(origin, egui::vec2(width, shown));
        let mut row = ui.new_child(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(
                origin,
                egui::pos2(origin.x + width, f32::INFINITY),
            )),
        );
        row.set_clip_rect(block.intersect(ui.clip_rect()));
        self.controls(&mut row);
        self.bar_row_height = row.min_rect().height();
        ui.advance_cursor_after_rect(block);
    }

    /// The strip at the foot of the bar. The whole width is the button, so a
    /// thumb anywhere along it will do, and the chevron at its right end
    /// points the way a press moves the buttons: up while they are folded
    /// away, turning over as they rise.
    fn bar_strip(&mut self, ui: &mut egui::Ui, openness: f32) {
        let ctx = ui.ctx().clone();
        let icon = icon_size(&ctx);
        let pad = px(&ctx, Key::MapTabPad);
        ui.spacing_mut().button_padding = egui::Vec2::splat(pad);
        let tint = ui.visuals().text_color();
        let chevron = egui::Image::new(icons::chevron_up())
            .fit_to_exact_size(egui::Vec2::splat(icon))
            .tint(tint)
            .rotate(openness * PI, egui::Vec2::splat(0.5));
        let hint = if self.map_bar_open {
            text::FOLD_BAR
        } else {
            text::UNFOLD_BAR
        };
        let strip = ui
            .add(
                // The grow atom takes the width, which puts the glyph at the
                // right end. No frame at rest: the bar's own fill is the strip.
                egui::Button::new((egui::Atom::grow(), chevron))
                    .frame_when_inactive(false)
                    .min_size(egui::vec2(ui.available_width(), 0.0)),
            )
            .on_hover_text(hint);
        if strip.clicked() {
            self.map_bar_open = !self.map_bar_open;
        }
        probe(
            &ctx,
            strip.rect,
            "Bar strip",
            &[Key::MapTabPad, Key::IconSize],
        );
    }

    /// The zoom buttons, stacked in the bottom right corner above the bars:
    /// in on top, out below. On every platform now that they are out of the
    /// bar; pinching still works where there is a touch screen.
    fn zoom_column(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let icon = icon_size(ctx);
        let margin = corner_margin(ctx);
        let pad = px(ctx, Key::MapTabPad);
        let between = px(ctx, Key::MapZoomGap);
        let foot = self.bottom_overlay_inset(ctx);
        let area = egui::Area::new(egui::Id::new("map_zoom"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(
                screen.right() - margin,
                screen.bottom() - foot - margin,
            ))
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .movable(false)
            .constrain(false)
            .show(ctx, |ui| {
                ui.spacing_mut().button_padding = egui::Vec2::splat(pad);
                ui.spacing_mut().item_spacing.y = between;
                ui.vertical(|ui| {
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
                });
            });
        probe(
            ctx,
            area.response.rect,
            "Zoom column",
            &[Key::MapZoomGap, Key::MapTabPad, Key::IconSize, Key::CornerMargin],
        );
    }

    /// The color key, in the top left corner: one row per marker on the map,
    /// in the color it is drawn in. Only the markers that are there, so the
    /// key is never longer than the map is busy. Up there rather than on the
    /// bars, whose left end the credit line and the download read-out
    /// already float over.
    fn map_key(&self, ctx: &egui::Context, screen: egui::Rect) {
        let mut entries: Vec<(egui::Color32, String)> = Vec::new();
        if self.current.is_some() {
            entries.push((self.config.colors.track, self.phone_label()));
        }
        if self.beacon_on_map().is_some() {
            entries.push((self.config.colors.fixed, self.beacon_label()));
        }
        for (&addr, node) in &self.remotes {
            if node.last_pos().is_some() || node.heard.is_some() {
                entries.push((remote_color(addr), self.config.lora.label_of(addr)));
            }
        }
        if entries.is_empty() {
            return;
        }
        let margin = corner_margin(ctx);
        let top = self.top_inset(ctx) + margin;
        let radius = px(ctx, Key::MapKeyDot);
        let fill = self.bar_fill(ctx);
        let outline = self.config.colors.outline;
        let area = egui::Area::new(egui::Id::new("map_key"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(screen.left() + margin, top))
            .pivot(egui::Align2::LEFT_TOP)
            .movable(false)
            .constrain(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(fill).show(ui, |ui| {
                    for (color, name) in &entries {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(2.0 * radius, em(ui)),
                                egui::Sense::hover(),
                            );
                            ui.painter().circle_filled(rect.center(), radius, *color);
                            ui.painter()
                                .circle_stroke(rect.center(), radius, egui::Stroke::new(1.0, outline));
                            ui.label(name);
                        });
                    }
                });
            });
        probe(
            ctx,
            area.response.rect,
            "Map key",
            &[Key::MapKeyDot, Key::CornerMargin],
        );
    }

    /// The floating controls bar: one icon button per thing the map can do.
    ///
    /// Every button whose glyph changes shows the state the press switches
    /// *to*, which is what a toolbar icon without a label has to do to be
    /// readable.
    fn controls(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // Taken off the icon rather than left on the style's text-derived
        // spacing, so enlarging the page text does not shrink the toolbar.
        // Derived from the *uncapped* icon size, which is the ceiling
        // `icon_size_for_row` starts from, so the two are not circular.
        let spacing = px(&ctx, Key::BarGap);
        ui.spacing_mut().item_spacing.x = spacing;
        // Which of the optional buttons are in the row this frame. Decided up
        // front, before anything is laid out, because the button count is what
        // sizes the row - and reused where the buttons are drawn, so the count
        // cannot disagree with what ends up in the bar.
        let show_rotate = self.has_direction() && self.tracking_beacon.is_none();
        // Center, track, layer, paths and the page menu are always there; the
        // zoom buttons live in the corner column now.
        let buttons = 5 + usize::from(show_rotate);
        let avail = ui.available_width().max(1.0);
        // No button may take more than a 1/buttons share of the bar, padding and
        // spacing included, so a full row always fits the screen instead of
        // running off the right edge when the set grows.
        let icon = icon_size_for_row(&ctx, avail, spacing, buttons);
        // The padding follows the row's icon rather than the usual one, so a
        // row squeezed to fit keeps its proportions.
        let squeeze = icon / icon_size(&ctx);
        ui.spacing_mut().button_padding = egui::vec2(
            px(&ctx, Key::BarButtonPadX) * squeeze,
            px(&ctx, Key::BarButtonPadY) * squeeze,
        );
        // egui lays a horizontal row out left-to-right and can't center it in a
        // single pass: its `main_align` is ignored and the row just fills the
        // width of any centering parent. So pad the left by half the leftover
        // space, using the row width measured last frame (it stays constant once
        // the button set is fixed). `add_space` counts as an item, so drop one
        // item spacing to keep the gap even on both sides.
        let pad = if self.controls_width > 0.0 {
            ((avail - self.controls_width) * 0.5 - spacing).max(0.0)
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
                self.paths_button(ui, icon);
                // The region download is started from the Settings page, which
                // jumps back here with the box selection already active.

                // The page menu sits inline, right after the other buttons.
                self.page_menu(ui, icon);
            });
            // Remember the row's own width (the inner group, excluding the pad)
            // so the next frame can center it.
            self.controls_width = row.response.rect.width();
            probe(&ctx, row.response.rect, "Toolbar buttons", &TOOLBAR_KEYS);
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

    /// Base-layer button: cycles the layers the tile provider has (standard
    /// and topographic; satellite too under ArcGIS), showing the one a press
    /// goes to.
    fn layer_button(&mut self, ui: &mut egui::Ui, icon: f32) {
        let next = self.layer.next(self.config.map.tiles);
        let (glyph, hint) = match next {
            MapLayer::Standard => (icons::map(), text::STANDARD_MAP),
            MapLayer::Topo => (icons::topo(), text::TOPO_MAP),
            MapLayer::Satellite => (icons::satellite(), text::SATELLITE_MAP),
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

        let foot = self.bottom_overlay_inset(ctx);
        let padding = popup_padding(ctx);
        let entry_gap = px(ctx, Key::MapCenterGap);
        let min_width = px(ctx, Key::MapCenterWidth);
        // Resolve each entry's name up front (a remote's comes from the config),
        // so the popup closure need not reach back into `self`.
        let labelled: Vec<(MarkerKind, Position, String)> = targets
            .iter()
            .map(|&(kind, pos)| (kind, pos, self.marker_label(kind)))
            .collect();
        let mut chosen: Option<(MarkerKind, Position)> = None;
        let mut close = false;
        let rect = floating(
            ctx,
            "center_menu",
            egui::Order::Foreground,
            // Just above the bars, the button that opened it being in them.
            egui::Pos2::new(screen.center().x, screen.bottom() - foot - px(ctx, Key::MapAboveBar)),
            egui::Align2::CENTER_BOTTOM,
            false,
            |ui| {
                ui.spacing_mut().button_padding = padding;
                ui.spacing_mut().item_spacing.y = entry_gap;
                ui.set_min_width(min_width);
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
        probe(
            ctx,
            rect,
            "Center menu",
            &[
                Key::MapAboveBar,
                Key::MapPopupPadX,
                Key::MapPopupPadY,
                Key::MapCenterGap,
                Key::MapCenterWidth,
            ],
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

    /// Handle tap selection of a map marker and draw the info popup (name +
    /// time since last update) for the selected one.
    ///
    /// Marker screen positions are computed the same way the `GpsLayer` plugin
    /// draws them: project with the map's projector, then apply the heading-up
    /// rotation (about the screen center) when the map is rotated. `tapped` is
    /// where the map itself was tapped this frame, and a tap that misses
    /// every marker dismisses the popup.
    fn marker_info(
        &mut self,
        ctx: &egui::Context,
        screen: egui::Rect,
        map_rect: egui::Rect,
        rotation: Option<Rot2>,
        tapped: Option<egui::Pos2>,
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
        // the connected board (when it is drawn), then every remote node.
        let mut markers: Vec<(MarkerKind, Option<Position>)> = vec![
            (MarkerKind::You, self.current),
            (MarkerKind::Beacon, self.beacon_on_map()),
        ];
        markers.extend(
            self.remotes
                .iter()
                .map(|(&addr, node)| (MarkerKind::Remote(addr), node.last_pos())),
        );

        // On a tap, pick the closest marker within the hit radius; a miss
        // clears the current selection.
        if let Some(click) = tapped {
            let reach = marker_hit_radius(ctx);
            self.selected_marker = markers
                .iter()
                .filter_map(|(kind, pos)| {
                    pos.as_ref().map(|p| (*kind, to_screen(*p).distance(click)))
                })
                .filter(|(_, dist)| *dist <= reach)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(kind, _)| kind);
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
            MarkerKind::Beacon => (self.beacon_on_map(), self.beacon_time),
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

        let rect = floating(
            ctx,
            "marker_info",
            egui::Order::Foreground,
            // Clear of the marker it points at.
            egui::pos2(anchor.x, anchor.y - px(ctx, Key::MapMarkerLift)),
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
        probe(ctx, rect, "Marker info", &[Key::MapMarkerLift]);
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
        // Taps and hairline drags are not a region.
        let min_drag = px(ctx, Key::MapDragMin);
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
        // The hint sits at the top, where nothing else is now that the bars
        // are at the foot of the screen, inset as far as the corner toggle.
        let top = self.top_inset(ctx) + corner_margin(ctx);
        // The confirm panel's buttons are measured off the icon size, which
        // is itself a fraction of the screen, so they stay a touch target.
        let padding = popup_padding(ctx);
        match self.select {
            RegionSelect::Inactive => {}
            RegionSelect::Picking { .. } => {
                let mut cancel = false;
                let rect = floating(
                    ctx,
                    "select_hint",
                    egui::Order::Foreground,
                    egui::Pos2::new(screen.center().x, top),
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
                probe(ctx, rect, "Select hint", &[Key::CornerMargin]);
                if cancel {
                    self.select = RegionSelect::Inactive;
                }
            }
            RegionSelect::Confirm { a, b, mut max_zoom } => {
                let mut close = false;
                let error_color = self.config.ui.error;
                // The stepper counts map zooms, which is what the zoom buttons
                // show; the source turns them into the tile levels it fetches
                // (one lower on a 512 px source). Topo tiles stop at zoom 17;
                // don't offer levels the server 404s.
                let source = self.map_source();
                let layer_max = source.download_max_zoom();
                let rect = floating(
                    ctx,
                    "select_confirm",
                    egui::Order::Foreground,
                    screen.center(),
                    egui::Align2::CENTER_CENTER,
                    false,
                    |ui| {
                        ui.spacing_mut().button_padding = padding;
                        ui.label(egui::RichText::new(text::DOWNLOAD_TITLE).strong());
                        gap(ui, Key::GapItem);
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
                        let max_level = source.tile_level(max_zoom);
                        let count = offline::tile_count(a, b, max_level);
                        ui.label(format!(
                            "{count} tiles, ~{} MB",
                            (count * source.tile_kb()).div_ceil(1024).max(1)
                        ));
                        if count > MAX_REGION_TILES {
                            ui.colored_label(error_color, text::TOO_MANY_TILES);
                        }
                        gap(ui, Key::GapItem);
                        ui.horizontal(|ui| {
                            let can_download =
                                count <= MAX_REGION_TILES && self.cache_dir.is_some();
                            if button!(ui, "Download", enabled: can_download).clicked() {
                                if let Some(dir) = &self.cache_dir {
                                    self.download = Some(offline::spawn_download(
                                        dir.clone(),
                                        source.clone(),
                                        offline::region_tiles(a, b, max_level),
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
                probe(
                    ctx,
                    rect,
                    "Download confirm",
                    &[Key::MapPopupPadX, Key::MapPopupPadY],
                );
                self.select = if close {
                    RegionSelect::Inactive
                } else {
                    RegionSelect::Confirm { a, b, max_zoom }
                };
            }
        }
    }

    /// The tile source's credit line, bottom left over the map, as every
    /// source's terms ask for. Small, on the bar fill so it reads over any
    /// tiles, and not interactable so the map under it still pans. It gives
    /// way to the download read-out, which floats in the same corner and
    /// matters more for as long as it is there.
    fn attribution_ui(&self, ctx: &egui::Context, screen: egui::Rect) {
        if self.download.is_some() {
            return;
        }
        let credit = self.map_source().attribution().text;
        let bottom = self.bottom_overlay_inset(ctx);
        let margin = corner_margin(ctx);
        let fill = self.bar_fill(ctx);
        let pad_x = px(ctx, Key::FieldPadX) as i8;
        let pad_y = px(ctx, Key::GapHair) as i8;
        egui::Area::new(egui::Id::new("attribution"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.left_bottom() + egui::vec2(margin, -(margin + bottom)))
            .pivot(egui::Align2::LEFT_BOTTOM)
            .movable(false)
            .interactable(false)
            .constrain(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(fill)
                    .inner_margin(egui::Margin::symmetric(pad_x, pad_y))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(credit).small().weak());
                    });
            });
    }

    /// Progress readout for the offline tile download, floating bottom-left
    /// on every page.
    pub(crate) fn download_ui(&mut self, ctx: &egui::Context, screen: egui::Rect) {
        let Some(progress) = self.download.clone() else {
            return;
        };
        // Clears the gesture bar, and the map's status bar when that is what
        // is at the foot of the screen - this floats over every page, so the
        // progress of a download is never hidden behind either.
        let bottom = self.bottom_overlay_inset(ctx);
        // Inset from the corner by the same fraction the floating page toggle
        // uses, so the two read as the same distance from the edge.
        let margin = corner_margin(ctx);
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

#[cfg(test)]
mod tests {
    use crate::app::tests::test_app;
    use crate::app::MyApp;

    /// One frame of the two bottom bars, at a given clock, so the fold
    /// animation can be run through.
    fn frame(ctx: &egui::Context, app: &mut MyApp, screen: egui::Rect, time: f64) {
        let input = egui::RawInput {
            time: Some(time),
            screen_rect: Some(screen),
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ctx| {
            app.map_controls_bar(ctx, screen);
            app.map_status_bar(ctx, screen);
        });
    }

    fn area(ctx: &egui::Context, id: &str) -> egui::Rect {
        ctx.memory(|m| m.area_rect(egui::Id::new(id)))
            .unwrap_or_else(|| panic!("{id} was not laid out"))
    }

    /// Folded, the controls bar is a strip on the bottom edge and the status
    /// bar stands on it; opened, the button row rises out of the strip, the
    /// bar stays on the edge, and the status bar rides up with its top.
    #[test]
    fn the_buttons_rise_out_of_the_strip_and_the_status_bar_stands_on_it() {
        let (mut app, _cmds, _events) = test_app();
        app.config.status_bar.show = true;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 800.0));
        let ctx = egui::Context::default();

        // The first frame measures the strip; the second places the bar by it.
        frame(&ctx, &mut app, screen, 0.0);
        frame(&ctx, &mut app, screen, 0.02);
        let folded = app.controls_height;
        assert!(folded > 0.0);
        assert!(folded < screen.height() / 8.0, "a strip, not a panel: {folded}");
        let bar = area(&ctx, "controls");
        assert!((bar.bottom() - screen.bottom()).abs() < 1.0, "on the edge: {bar:?}");
        assert!((bar.height() - folded).abs() < 1.0);
        let status = area(&ctx, "map_status_bar");
        assert!(
            (status.bottom() - bar.top()).abs() < 1.0,
            "the status bar stands on the strip: {status:?} over {bar:?}"
        );

        app.map_bar_open = true;
        for i in 1..=30 {
            frame(&ctx, &mut app, screen, 0.02 + f64::from(i) * 0.02);
            // Every frame of the rise, the bar's bottom edge stays put.
            let bar = area(&ctx, "controls");
            assert!(
                (bar.bottom() - screen.bottom()).abs() < 1.0,
                "frame {i}: the edge moved: {bar:?}"
            );
        }
        let open = app.controls_height;
        assert!(open > folded + 1.0, "the row adds height: {open} over {folded}");
        assert!(app.bar_row_height > 0.0);
        let bar = area(&ctx, "controls");
        assert!((bar.height() - open).abs() < 1.0);
        let status = area(&ctx, "map_status_bar");
        assert!(
            (status.bottom() - bar.top()).abs() < 1.0,
            "the status bar rode up: {status:?} over {bar:?}"
        );
    }
}
