//! The handles of the streamline seeds in the viewport (decided
//! 2026-11-24: "drag handles that snap to the mesh, or typed points"): a
//! seed line has a dot at each end and moves whole by its middle, a seed
//! ball has a dot at its centre and one on its rim for the radius, and a
//! seed plane a dot at its centre (its normal comes from the tree's menu
//! or `stream.set`). A dragged point moves in the plane through it square
//! to the view, and snaps to a node, an edge's middle or a face's centre
//! within reach of the pointer (Alt: no snapping), so it can be put on
//! the mesh's corners and inlets or anywhere inside. Every drag calls
//! `stream.set`; the window's undo group (open while a mouse button is
//! held) makes one drag one undo step. Patch seeds have no handles.
//!
//! The handles take the current streamline set (the one selected last,
//! else the one added last) unless a clip plane or slice was selected
//! after it; then the plane keeps its handles and the seeds have none.
//! Like the clip handles they are painted by egui, so screenshots
//! (`view.screenshot`) leave them out.

use egui::{Color32, CursorIcon, FontId, Pos2, Rect, Role, Sense, Stroke, Ui, WidgetInfo};
use murmel_engine::clip::{Clip, ClipMode};
use murmel_engine::measure::{Anchor, SnapKind};
use murmel_engine::{Camera, Item, Seed};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::panels::number;
use crate::style::color;

/// One handle of the seeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SeedHandle {
    /// The seed line's start.
    From,
    /// The seed line's end.
    To,
    /// The seed line itself: moves it whole.
    Mid,
    /// The seed ball's centre.
    Centre,
    /// A point on the seed ball's rim: its radius.
    Radius,
    /// The seed plane's centre.
    Origin,
}

impl SeedHandle {
    fn name_key(self) -> &'static str {
        match self {
            SeedHandle::From => "stream-handle-from",
            SeedHandle::To => "stream-handle-to",
            SeedHandle::Mid => "stream-handle-mid",
            SeedHandle::Centre => "stream-handle-centre",
            SeedHandle::Radius => "stream-handle-radius",
            SeedHandle::Origin => "stream-handle-origin",
        }
    }

    /// Whether a drag of it snaps to the mesh.
    fn snaps(self) -> bool {
        matches!(
            self,
            SeedHandle::From | SeedHandle::To | SeedHandle::Centre | SeedHandle::Origin
        )
    }
}

/// A drag of a seed handle under way.
#[derive(Clone, Copy, Debug)]
pub struct StreamDrag {
    /// The set dragged (0-based); a drag ends when another set takes
    /// the handles.
    pub set: usize,
    pub handle: SeedHandle,
    /// The handle's point when the drag began: it moves in the plane
    /// through this point square to the view.
    pub start: [f64; 3],
    /// Where the pointer met that plane when the drag began (the handle
    /// keeps its offset from the pointer).
    pub grab: [f64; 3],
    /// The seed line's ends when the drag began (a drag of the line).
    pub ends: [[f64; 3]; 2],
}

/// Where a handle was drawn this frame (for the window tests).
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(test), allow(dead_code))] // read by the window tests
pub struct SeedSpot {
    pub handle: SeedHandle,
    pub at: Pos2,
}

/// The features a dragged point snaps to: not any surface point, so that
/// away from them it moves freely, inside the mesh too.
const SNAP: [SnapKind; 3] = [SnapKind::Node, SnapKind::EdgeMidpoint, SnapKind::FaceCentre];
/// Radius of a handle's dot, in points, and of its halo.
const DOT: f32 = 5.5;
const HALO: f32 = 1.6;
/// How near the pointer must come to a handle.
const HIT: f32 = 10.0;

type V3 = [f64; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Where the pointer's ray at `ndc` meets the plane through `through`
/// square to the view.
fn view_plane_hit(cam: &Camera, ndc: [f64; 2], aspect: f64, through: V3) -> Option<V3> {
    let (o, d) = cam.ray(ndc, aspect);
    let (o, d) = (o.to_array(), d.to_array());
    let n = cam.to_eye().to_array();
    let den = dot(d, n);
    if den.abs() < 1e-12 {
        return None;
    }
    let t = dot(sub(through, o), n) / den;
    Some(add(o, mul(d, t)))
}

/// The distance from `p` to the segment `a`–`b` on screen.
fn to_segment(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

impl MurmelApp {
    /// The streamline set whose seeds carry handles: the current set,
    /// when it shows and its seeds are not a patch, unless a clip plane
    /// or slice was selected after it.
    pub(crate) fn stream_handle_target(&self) -> Option<usize> {
        let d = &self.session.display;
        let k = d.current_stream()?;
        let st = d.streams.get(k)?;
        if !st.enabled || matches!(st.seed, Seed::Patch { .. }) {
            return None;
        }
        let plane_after = d
            .selection
            .iter()
            .rev()
            .find_map(|i| match i {
                Item::Stream(_) => Some(false),
                Item::Clip(_) | Item::Slice(_) => Some(true),
                _ => None,
            })
            .unwrap_or(false);
        (!plane_after).then_some(k)
    }

    /// The point a drag of a snapping handle puts at `pos`: a feature of
    /// the mesh within reach, else the view plane's point.
    fn seed_point(&mut self, pos: Pos2, rect: Rect, snap: bool) -> Option<Anchor> {
        if !snap {
            return None;
        }
        let kinds: Vec<&str> = SNAP.iter().map(|k| k.key()).collect();
        let e = murmel_tools::call(
            "measure.snap",
            json!({"ndc": crate::viewport::ndc(pos, rect), "snap": kinds}),
            &mut self.session,
        );
        e.ok()
            .then(|| serde_json::from_value::<Anchor>(e.result["anchor"].clone()).ok())
            .flatten()
    }

    /// The seeds' outline and handles of the set that carries them.
    pub(crate) fn stream_handles(&mut self, ui: &Ui, rect: Rect, cam: &Camera, dark: bool) {
        self.stream_spots.clear();
        let Some(k) = self.stream_handle_target() else {
            self.stream_drag = None;
            return;
        };
        if !ui.input(|i| i.pointer.any_down()) || self.stream_drag.is_some_and(|d| d.set != k) {
            self.stream_drag = None;
        }
        let Some(m) = &self.session.mesh else { return };
        let Some(seed) = self.session.display.streams.get(k).map(|s| s.seed.clone()) else {
            return;
        };
        let bounds = m.mesh.bounds();
        let (origin, sradius) = (m.surface.origin, m.surface.radius);
        let aspect = (rect.width() / rect.height().max(1.0)) as f64;
        let to_screen = move |p: V3| {
            let [x, y] = cam.project(origin, sradius, aspect, p)?;
            Some(Pos2::new(
                rect.left() + ((x + 1.0) * 0.5) as f32 * rect.width(),
                rect.top() + ((1.0 - y) * 0.5) as f32 * rect.height(),
            ))
        };
        let palette = self.session.tokens.palette(dark).clone();
        let gizmo = self.session.tokens.gizmo.clone();
        let colors = *gizmo.colors(dark);
        let (dot_c, outline_c, shade) = (
            color(colors.slide),
            color(palette.accent),
            color(gizmo.shade),
        );
        let painter = ui.painter_at(rect);
        let (right, _) = cam.basis();
        let right = right.to_array();

        // The handles and their points; the seeds' outline.
        let mut handles: Vec<(SeedHandle, V3)> = Vec::new();
        match &seed {
            Seed::Line { from, to } => {
                if let (Some(a), Some(b)) = (to_screen(*from), to_screen(*to)) {
                    painter.line_segment([a, b], Stroke::new(4.0, shade.gamma_multiply(0.6)));
                    painter.line_segment([a, b], Stroke::new(2.0, outline_c));
                }
                handles.push((SeedHandle::Mid, mul(add(*from, *to), 0.5)));
                handles.push((SeedHandle::From, *from));
                handles.push((SeedHandle::To, *to));
            }
            Seed::Sphere { centre, radius } => {
                let (_, up) = cam.basis();
                let up = up.to_array();
                let ring: Option<Vec<Pos2>> = (0..=64)
                    .map(|i| {
                        let a = i as f64 / 64.0 * std::f64::consts::TAU;
                        to_screen(add(
                            *centre,
                            add(mul(right, radius * a.cos()), mul(up, radius * a.sin())),
                        ))
                    })
                    .collect();
                if let Some(ring) = ring {
                    painter.add(egui::Shape::line(
                        ring.clone(),
                        Stroke::new(3.5, shade.gamma_multiply(0.6)),
                    ));
                    painter.add(egui::Shape::dashed_line(
                        &ring,
                        Stroke::new(1.6, outline_c),
                        6.0,
                        4.0,
                    ));
                }
                handles.push((SeedHandle::Centre, *centre));
                handles.push((SeedHandle::Radius, add(*centre, mul(right, *radius))));
            }
            Seed::Plane { origin, normal } => {
                let plane = Clip {
                    origin: *origin,
                    normal: *normal,
                    mode: ClipMode::Crinkle,
                    enabled: true,
                };
                let section: Option<Vec<Pos2>> = plane
                    .box_section(&bounds)
                    .into_iter()
                    .map(&to_screen)
                    .collect();
                if let Some(pts) = section.filter(|p| p.len() >= 3) {
                    painter.add(egui::Shape::convex_polygon(
                        pts.clone(),
                        outline_c.gamma_multiply(0.08),
                        Stroke::NONE,
                    ));
                    let mut ring = pts.clone();
                    ring.push(pts[0]);
                    painter.add(egui::Shape::dashed_line(
                        &ring,
                        Stroke::new(1.5, outline_c),
                        6.0,
                        4.0,
                    ));
                }
                handles.push((SeedHandle::Origin, *origin));
            }
            Seed::Patch { .. } => return,
        }
        let spots: Vec<(SeedHandle, V3, Pos2)> = handles
            .iter()
            .filter_map(|&(h, p)| Some((h, p, to_screen(p)?)))
            .collect();
        for &(handle, _, at) in &spots {
            self.stream_spots.push(SeedSpot { handle, at });
        }

        // The handle under the pointer: a dot within reach, else the line.
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let line_ends = match (&seed, spots.as_slice()) {
            (Seed::Line { .. }, [_, (_, _, a), (_, _, b)]) => Some((*a, *b)),
            _ => None,
        };
        let near = pointer.and_then(|p| {
            spots
                .iter()
                .filter(|(h, _, _)| *h != SeedHandle::Mid)
                .map(|&(h, _, at)| (h, at.distance(p), at))
                .filter(|x| x.1 < HIT)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .or_else(|| {
                    let (a, b) = line_ends?;
                    (to_segment(a, b, p) < HIT).then_some((SeedHandle::Mid, 0.0, p))
                })
        });

        let free = ui.input(|i| i.modifiers.alt);
        let drag = self.stream_drag;
        let mut hot = None;
        let mut call: Option<Value> = None;
        let mut snapped: Option<SnapKind> = None;
        for &(handle, point, _) in &spots {
            let id = ui.scope_id().with(("stream-handle", handle));
            let area = match (drag, near) {
                (Some(d), _) if d.handle == handle => rect,
                (None, Some((h, _, q))) if h == handle => {
                    Rect::from_center_size(q, egui::vec2(2.0 * HIT, 2.0 * HIT))
                }
                _ => continue,
            };
            let resp = ui.interact(area, id, Sense::drag());
            let name = self.i18n.tr(handle.name_key());
            resp.widget_info(|| WidgetInfo::labeled(Role::Slider, true, name.clone()));
            if resp.hovered() || resp.dragged() {
                hot = Some(handle);
                ui.ctx().set_cursor_icon(if resp.dragged() {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                });
            }
            let at = resp.interact_pointer_pos().or(pointer);
            if resp.drag_started()
                && let Some(pos) = at
            {
                let ndc = crate::viewport::ndc(pos, rect);
                let grab = view_plane_hit(cam, ndc, aspect, point).unwrap_or(point);
                let ends = match &seed {
                    Seed::Line { from, to } => [*from, *to],
                    _ => [point, point],
                };
                self.stream_drag = Some(StreamDrag {
                    set: k,
                    handle,
                    start: point,
                    grab,
                    ends,
                });
            }
            if !resp.dragged() {
                if self.stream_drag.is_none() {
                    resp.on_hover_text(self.i18n.tr("stream-handle-tip"));
                }
                continue;
            }
            let (Some(d), Some(pos)) = (self.stream_drag, at) else {
                continue;
            };
            let ndc = crate::viewport::ndc(pos, rect);
            let Some(hit) = view_plane_hit(cam, ndc, aspect, d.start) else {
                continue;
            };
            let moved = add(d.start, sub(hit, d.grab));
            let snap = self.seed_point(pos, rect, handle.snaps() && !free);
            snapped = snap.as_ref().map(|a| a.snap);
            let to = snap.map_or(moved, |a| a.point);
            call = match handle {
                SeedHandle::From => Some(json!({ "from": to })),
                SeedHandle::To => Some(json!({ "to": to })),
                SeedHandle::Centre => Some(json!({ "centre": to })),
                SeedHandle::Origin => Some(json!({ "origin": to })),
                SeedHandle::Mid => {
                    let delta = sub(hit, d.grab);
                    Some(json!({
                        "from": add(d.ends[0], delta),
                        "to": add(d.ends[1], delta),
                    }))
                }
                SeedHandle::Radius => match &seed {
                    Seed::Sphere { centre, .. } => {
                        let r = dot(sub(hit, *centre), sub(hit, *centre)).sqrt();
                        (r > 0.0 && r.is_finite()).then(|| json!({ "radius": r }))
                    }
                    _ => None,
                },
            };
        }
        if let Some(Value::Object(mut input)) = call {
            // Only a change goes to the tool (and the journal).
            let same = match &seed {
                Seed::Line { from, to } => {
                    input.get("from").is_none_or(|v| *v == json!(from))
                        && input.get("to").is_none_or(|v| *v == json!(to))
                }
                Seed::Sphere { centre, radius } => {
                    input.get("centre").is_none_or(|v| *v == json!(centre))
                        && input.get("radius").is_none_or(|v| *v == json!(radius))
                }
                Seed::Plane { origin, .. } => {
                    input.get("origin").is_none_or(|v| *v == json!(origin))
                }
                Seed::Patch { .. } => true,
            };
            if !same {
                input.insert("stream".into(), json!(k + 1));
                self.call("stream.set", Value::Object(input));
            }
        }

        // The dots: halo, then the colour; larger when hovered.
        for &(handle, _, at) in &spots {
            if handle == SeedHandle::Mid {
                continue;
            }
            let r = if hot == Some(handle) { DOT * 1.3 } else { DOT };
            painter.circle_filled(at, r + HALO, shade);
            painter.circle_filled(at, r, dot_c);
        }
        if hot == Some(SeedHandle::Mid)
            && let Some((a, b)) = line_ends
        {
            painter.line_segment([a, b], Stroke::new(3.0, dot_c));
        }

        // While dragging: where the point is, and what it snapped to.
        if let (Some(d), Some(at)) = (self.stream_drag, pointer) {
            let seed = self.session.display.streams.get(k).map(|s| s.seed.clone());
            let p3 = |p: V3| format!("({}, {}, {})", number(p[0]), number(p[1]), number(p[2]));
            let mut text = match (d.handle, seed) {
                (SeedHandle::From, Some(Seed::Line { from, .. })) => p3(from),
                (SeedHandle::To | SeedHandle::Mid, Some(Seed::Line { to, .. })) => p3(to),
                (SeedHandle::Centre, Some(Seed::Sphere { centre, .. })) => p3(centre),
                (SeedHandle::Radius, Some(Seed::Sphere { radius, .. })) => {
                    format!("r = {}", number(radius))
                }
                (SeedHandle::Origin, Some(Seed::Plane { origin, .. })) => p3(origin),
                _ => String::new(),
            };
            if let Some(kind) = snapped {
                let what = self.i18n.tr(&format!("snap-{}", kind.key()));
                text = format!(
                    "{text}\n{}",
                    self.i18n
                        .tr_args("stream-handle-snapped", &[("kind", what.into())])
                );
            }
            if !text.is_empty() {
                let galley = painter.layout(
                    text,
                    FontId::proportional(12.0),
                    color(palette.text_primary),
                    f32::INFINITY,
                );
                let pos = at + egui::vec2(16.0, 14.0);
                let bg = Rect::from_min_size(pos, galley.size()).expand2(egui::vec2(6.0, 3.0));
                painter.rect_filled(bg, 4.0, color(palette.bg_raised));
                painter.rect_stroke(
                    bg,
                    4.0,
                    Stroke::new(1.0, color(palette.line)),
                    egui::StrokeKind::Inside,
                );
                painter.galley(pos, galley, Color32::PLACEHOLDER);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: a dragged point stays in the plane through its start
    /// square to the view, under the pointer: the ray at the centre of an
    /// orthographic or perspective view meets that plane at the point the
    /// camera looks at, shifted along the view to the start's depth.
    #[test]
    fn view_plane_hit_stays_at_the_depth() {
        let mut cam = Camera::default();
        for ortho in [false, true] {
            if ortho != cam.is_orthographic() {
                cam.projection = cam.projection.next();
                if ortho != cam.is_orthographic() {
                    cam.projection = cam.projection.next();
                }
            }
            let target = cam.target;
            let through = add(target, mul(cam.to_eye().to_array(), 0.3));
            let hit = view_plane_hit(&cam, [0.0, 0.0], 1.5, through).unwrap();
            let n = cam.to_eye().to_array();
            assert!((dot(sub(hit, through), n)).abs() < 1e-9);
            for k in 0..3 {
                assert!((hit[k] - through[k]).abs() < 1e-6, "{hit:?} {through:?}");
            }
        }
        assert!(
            to_segment(
                Pos2::new(0.0, 0.0),
                Pos2::new(10.0, 0.0),
                Pos2::new(5.0, 3.0)
            ) == 3.0
        );
    }
}
