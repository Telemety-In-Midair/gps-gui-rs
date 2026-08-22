//! Painting the map: the tile layer with its overlays, the heading-up rotation
//! pass, and the distance label that follows the line to the beacon.
//!
//! Split from the map page ([`super::pages::map`]) by what the code is: the
//! page declares the controls and the popups, and this draws the picture they
//! sit over. The rotation helpers are here because the rotation is the reason
//! the painting cannot simply be handed to the tile crate.

use egui::emath::Rot2;
use egui::{epaint::TextShape, Pos2, Shape};
use walkers::{lat_lon, Map, Position, Projector, Tiles};

use crate::app::{ease_heading, MarkerKind, MyApp, RegionSelect, ARROW_TAU};
use crate::config::remote_color;
use crate::marker::{GpsLayer, RemoteDraw};
use crate::points::TrackPoint;
use crate::tiles::MapLayer;

/// Where the map looks before the first GPS fix arrives.
pub(super) fn default_position() -> Position {
    lat_lon(44.5, -123.0)
}

/// Seconds per beat of the connected-beacon heartbeat, and how often that
/// animation asks for a repaint. A beat a second reads as a pulse rather than a
/// flicker, and ~20 fps is smooth enough for one expanding ring while leaving
/// an otherwise idle map mostly asleep.
const PULSE_PERIOD: f64 = 1.0;
const PULSE_FRAME: f32 = 0.05;

impl MyApp {
    /// Paint the map into `map_rect` (which may overscan past `clip`). When
    /// `rotation` is set, the painted shapes are rotated about the center of
    /// `clip` and then clipped back to `clip`, so the visible map spins with the
    /// heading while its corners stay filled by the overscan.
    pub(super) fn map(
        &mut self,
        ui: &mut egui::Ui,
        map_rect: egui::Rect,
        rotation: Option<Rot2>,
        clip: egui::Rect,
    ) {
        let my_position = self.current.unwrap_or_else(default_position);

        // Heartbeat phase for the beacon marker while the BLE link is up. The
        // animation is driven by repainting on a timer rather than every frame,
        // so an idle map costs a handful of frames a second instead of sixty.
        let beacon_pulse = self.ble_connected.then(|| {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f32(PULSE_FRAME));
            (ui.input(|i| i.time).rem_euclid(PULSE_PERIOD) / PULSE_PERIOD) as f32
        });

        // The arrow is eased toward the live heading rather than snapping to it:
        // outside heading-up the compass runs at a few Hz, so the raw readings
        // would step the arrow round in visible jumps.
        let arrow = self.effective_heading().map(|target| {
            let dt = ui.input(|i| i.stable_dt).clamp(0.0, 0.1);
            let current = self.smoothed_arrow.unwrap_or(target);
            let (next, remaining) = ease_heading(current, target, dt, ARROW_TAU);
            self.smoothed_arrow = Some(next);
            if remaining > 0.05 {
                ui.ctx().request_repaint();
            }
            next
        });
        if arrow.is_none() {
            self.smoothed_arrow = None;
        }

        // A hidden path is an empty one: the plugin draws whatever it is given,
        // so the map page is the only place that decides what is visible. The
        // bar's toggle is a master switch over both per-path settings.
        let paths = |shown: bool, points: &[TrackPoint]| -> Vec<Position> {
            if self.show_paths && shown {
                points.iter().map(|t| t.pos).collect()
            } else {
                Vec::new()
            }
        };
        // Each remote node draws in its address palette color; its path follows
        // the single `[lora] show_path` toggle under the same master switch.
        // The marker falls back to the last recorded point when the live view
        // is gone (a board switch), so a known node never draws as a bare path.
        let remotes: Vec<RemoteDraw> = self
            .remotes
            .iter()
            .map(|(&addr, node)| RemoteDraw {
                pos: node.last_pos(),
                track: paths(self.config.lora.show_path, &node.track),
                color: remote_color(addr),
            })
            .collect();
        // The user->target line goes to the tracked board (or the connected
        // board when not tracking), drawn in that board's color.
        let distance_line = self.distance_target().map(|(kind, pos)| {
            let color = match kind {
                MarkerKind::Remote(addr) => remote_color(addr),
                _ => self.config.colors.fixed,
            };
            (pos, color)
        });
        let layer = GpsLayer {
            current: self.current,
            track: paths(self.config.track.show_path, &self.track),
            heading: arrow,
            beacon: self.beacon,
            beacon_track: paths(self.config.ble.show_path, &self.beacon_track),
            beacon_pulse,
            remotes,
            distance_line,
            colors: self.config.colors,
            sizes: self.config.sizes,
            distance_dotted: self.config.distance.dotted,
        };

        // walkers sizes itself to the child's available space, so give it the
        // (possibly overscanned) map rect.
        let layer_id = ui.layer_id();
        let start = ui.ctx().graphics_mut(|g| g.entry(layer_id).next_idx());

        let android = cfg!(target_os = "android");
        // Tracking mode owns the center and zoom (recomputed each frame), so it
        // locks out manual pan and zoom on every platform. Heading-up on mobile
        // also locks the view. North-up keeps normal pan.
        let tracking = self.tracking_beacon.is_some();
        let locked = tracking || (rotation.is_some() && android);

        // The map is drawn inside a background Area (rotation overscan on
        // Android, full-bleed on desktop). walkers' built-in zoom only fires
        // when the map is the top interactable layer under the pointer, which a
        // background Area never is, so its scroll/pinch zoom silently no-ops. We
        // drive zoom ourselves here and turn walkers' own gesture off below.
        let pinching;

        #[cfg(target_os = "android")]
        {
            // Pinch: mirror walkers' own `zoom_by((delta - 1) * zoom_speed)`.
            let zoom_delta = ui.ctx().input(|i| i.zoom_delta());
            pinching = (zoom_delta - 1.0).abs() > 0.001;
            if pinching && !tracking {
                let zoom = self.map_memory.zoom() + (zoom_delta as f64 - 1.0) * 2.0;
                let _ = self.map_memory.set_zoom(zoom);
            }
        }

        #[cfg(not(target_os = "android"))]
        {
            pinching = false;
            // Bare mouse-wheel zoom about the map center (like the +/- buttons),
            // gated on the pointer being over the map rect rather than walkers'
            // layer-topmost check (which the background Area fails).
            let (scroll_y, hover) =
                ui.ctx().input(|i| (i.smooth_scroll_delta.y, i.pointer.hover_pos()));
            if scroll_y != 0.0 && !tracking && hover.is_some_and(|p| clip.contains(p)) {
                let zoom = self.map_memory.zoom() + scroll_y as f64 * 0.005;
                let _ = self.map_memory.set_zoom(zoom);
                ui.ctx().request_repaint();
            }
        }

        // Suppress pan while pinching so the two-finger gesture zooms instead of
        // dragging (walkers normally keeps zoom and pan mutually exclusive).
        // While a download box is being picked, the drag draws the box instead.
        let picking = matches!(self.select, RegionSelect::Picking { .. });
        let allow_pan = !locked && !pinching && !picking;

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(map_rect));
        // Draw whichever base layer is selected; both share `map_memory` (so the
        // view is unchanged by the switch) and the on-disk cache.
        let tiles: &mut dyn Tiles = match self.layer {
            MapLayer::Standard => &mut self.tiles,
            MapLayer::Topo => &mut self.topo_tiles,
        };
        let map = Map::new(Some(tiles), &mut self.map_memory, my_position)
            .with_plugin(layer)
            // We drive zoom manually above on both platforms (walkers' own zoom
            // gate does not fire for a background Area), so turn its gesture off.
            .zoom_gesture(false)
            // Keep walkers off bare-scroll entirely: with no ctrl-zoom and no
            // touches it also stops scroll-panning, so our wheel handler is the
            // only thing acting on the wheel. We pan by primary-button drag.
            .zoom_with_ctrl(false)
            .panning(allow_pan)
            .drag_pan_buttons(if allow_pan {
                egui::DragPanButtons::PRIMARY
            } else {
                egui::DragPanButtons::empty()
            });
        child.add(map);

        if let Some(rot) = rotation {
            let pivot = clip.center();
            let end = ui.ctx().graphics_mut(|g| g.entry(layer_id).next_idx());
            ui.ctx().graphics_mut(|g| {
                let list = g.entry(layer_id);
                for i in start.0..end.0 {
                    list.mutate_shape(egui::layers::ShapeIdx(i), |cs| {
                        rotate_shape(&mut cs.shape, rot, pivot);
                        cs.clip_rect = clip;
                    });
                }
            });
        }

        // Painted last, so it is outside the rotation pass above and sits over
        // the markers.
        self.distance_label(ui, map_rect, rotation, clip);
    }
    /// Paint the beacon-distance label: the distance to the beacon, centered
    /// just above the midpoint of the user->beacon line, turning with the map.
    ///
    /// The [`GpsLayer`] plugin draws the line but not this label: text needs an
    /// angle as well as a position, and leaving that to the rotation pass in
    /// [`Self::map`] left the glyphs level. So the label is placed here, after
    /// that pass, with both set outright - positions projected exactly as the
    /// plugin projects them and turned about the same pivot, and the map's angle
    /// handed straight to the text shapes.
    fn distance_label(
        &self,
        ui: &egui::Ui,
        map_rect: egui::Rect,
        rotation: Option<Rot2>,
        clip: egui::Rect,
    ) {
        if !self.config.distance.show {
            return;
        }
        let (Some(user), Some((_, target)), Some(meters)) =
            (self.current, self.distance_target(), self.distance_to_target())
        else {
            return;
        };

        // Same projector the plugin draws with: walkers builds it from the rect
        // it was given (the overscanned one when the map is rotated).
        let projector = Projector::new(map_rect, &self.map_memory, user);
        let user_px = projector.project(user).to_pos2();
        let target_px = projector.project(target).to_pos2();

        // Follow the line: turn its midpoint about the pivot the map turned
        // about, and turn the "above the line" offset with it.
        let rot = rotation.unwrap_or(Rot2::IDENTITY);
        let mid = user_px + (target_px - user_px) * 0.5;
        let size = self.config.sizes.distance_text;
        // Lift off the line by a fraction of the label's own font size, so the
        // gap stays the same to the eye at any configured text size.
        let pad = size * 0.7;
        let anchor = rotate_pos(mid, rot, clip.center()) + rot * egui::Vec2::new(0.0, -pad);

        // The label reads in the theme's text color lightened a little (the
        // outline carries the contrast, so the glyphs need not be full
        // strength), outlined in the opposite color so it stays legible over
        // either base map.
        let text_color = ui
            .visuals()
            .text_color()
            .lerp_to_gamma(egui::Color32::WHITE, 0.35);
        let outline_color = if ui.visuals().dark_mode {
            egui::Color32::BLACK
        } else {
            egui::Color32::WHITE
        };

        // Laid out once and shared by every copy below, so the outline costs
        // nine shapes but only one layout.
        let painter = ui.painter().with_clip_rect(clip);
        let galley = painter.layout_no_wrap(
            self.config.distance.units.format(meters),
            egui::FontId::proportional(size),
            text_color,
        );
        let top_left = anchor - egui::Vec2::new(galley.size().x * 0.5, galley.size().y);
        let angle = rot.angle();

        // Outline width scales with the font so it stays a hair around the
        // glyphs at any size. Diagonals are pulled in so the ring is round
        // rather than square-cornered. The offsets are applied after rotation,
        // which a symmetric ring is free to ignore.
        let w = (size * 0.1).max(1.0);
        let d = w * std::f32::consts::FRAC_1_SQRT_2;
        for off in [
            egui::Vec2::new(w, 0.0),
            egui::Vec2::new(-w, 0.0),
            egui::Vec2::new(0.0, w),
            egui::Vec2::new(0.0, -w),
            egui::Vec2::new(d, d),
            egui::Vec2::new(d, -d),
            egui::Vec2::new(-d, d),
            egui::Vec2::new(-d, -d),
        ] {
            painter.add(
                TextShape::new(top_left + off, galley.clone(), outline_color)
                    .with_override_text_color(outline_color)
                    .with_angle_and_anchor(angle, egui::Align2::CENTER_BOTTOM),
            );
        }
        painter.add(
            TextShape::new(top_left, galley, text_color)
                .with_angle_and_anchor(angle, egui::Align2::CENTER_BOTTOM),
        );
    }
}

/// Rotate `p` by `rot` about `origin` (screen-space points).
pub(super) fn rotate_pos(p: Pos2, rot: Rot2, origin: Pos2) -> Pos2 {
    origin + rot * (p - origin)
}

/// Rotate a painted [`Shape`] in place about `origin`. Mirrors the point-moving
/// arm of `Shape::transform`, but applies a rotation instead of a scale/offset.
/// Axis-aligned rects and callbacks can only follow their center; everything
/// else (meshes, paths, text) rotates faithfully.
fn rotate_shape(shape: &mut Shape, rot: Rot2, origin: Pos2) {
    match shape {
        Shape::Noop => {}
        Shape::Vec(shapes) => {
            for s in shapes {
                rotate_shape(s, rot, origin);
            }
        }
        Shape::Circle(c) => c.center = rotate_pos(c.center, rot, origin),
        Shape::Ellipse(e) => e.center = rotate_pos(e.center, rot, origin),
        Shape::LineSegment { points, .. } => {
            for p in points {
                *p = rotate_pos(*p, rot, origin);
            }
        }
        Shape::Path(path) => {
            for p in &mut path.points {
                *p = rotate_pos(*p, rot, origin);
            }
        }
        Shape::Rect(r) => {
            let center = rotate_pos(r.rect.center(), rot, origin);
            r.rect = egui::Rect::from_center_size(center, r.rect.size());
        }
        Shape::Text(t) => {
            t.pos = rotate_pos(t.pos, rot, origin);
            t.angle += rot.angle();
        }
        Shape::Mesh(mesh) => std::sync::Arc::make_mut(mesh).rotate(rot, origin),
        Shape::QuadraticBezier(b) => {
            for p in &mut b.points {
                *p = rotate_pos(*p, rot, origin);
            }
        }
        Shape::CubicBezier(b) => {
            for p in &mut b.points {
                *p = rotate_pos(*p, rot, origin);
            }
        }
        Shape::Callback(cb) => {
            let center = rotate_pos(cb.rect.center(), rot, origin);
            cb.rect = egui::Rect::from_center_size(center, cb.rect.size());
        }
    }
}
