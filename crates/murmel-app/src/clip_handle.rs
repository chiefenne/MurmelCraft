//! The clip plane's handles in the viewport (UI spec, Clip plane: "drag
//! the plane"): an arrow along the normal slides the plane, and two arcs
//! across it tilt it about its own horizontal and vertical axes through
//! its centre in the mesh. Every drag calls the registered tools
//! (`clip.set` with an offset, `clip.rotate` with an axis, an angle and
//! the pivot), so the window, the command line and agents share them, and
//! the window's undo group (open while a mouse button is held) makes one
//! drag one undo step.
//!
//! The handles are our own: tubes and cones built in world space at a
//! constant size on screen, projected with the camera the window draws,
//! shaded per vertex (Lambert diffuse and a Blinn-Phong highlight; Blinn,
//! "Models of light reflection for computer synthesized pictures",
//! SIGGRAPH 1977) in the accent and the `gizmo` tokens, and drawn back to
//! front (painter's algorithm) over a darker halo, so they read on any
//! mesh colour. The tube frames follow the path by parallel transport
//! (Bishop, "There is more than one way to frame a curve", 1975), so the
//! arcs do not twist. The arcs take their own colour (the `tilt` gizmo
//! token, a little more violet than the arrow), and while one is hovered
//! or dragged its rotation plane shows as a faint disc through the pivot,
//! square to the tilt axis, with ticks every 15 degrees, the axis as a
//! dashed line and, during the drag, a wedge of the angle turned so far.
//! Like the view cube they are painted by egui, so screenshots
//! (`view.screenshot`) leave them out.
//!
//! Slices (field results, stage 2) take the same handles, calling
//! `slice.set` and `slice.rotate` (decided 2026-11-24: "placed and moved
//! like clip planes"). One plane carries them at a time: the clip plane
//! or slice selected last, else the slice added last, else the current
//! clip plane; none while streamline seeds selected after them carry
//! their own handles (`stream_handle.rs`).

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, CursorIcon, FontId, Pos2, Rect, Role, Sense, Ui, WidgetInfo};
use murmel_engine::Camera;
use murmel_engine::clip::{Clip, cross, unit};
use murmel_engine::theme::GizmoTokens;
use serde_json::json;

use crate::app::MurmelApp;
use crate::style::color;

/// One of the three handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Handle {
    /// The arrow: slides the plane along its normal.
    Slide,
    /// The arc on the plane's upper side: tilts it about its horizontal
    /// axis (the normal turns up or down).
    TiltU,
    /// The arc on the plane's right side: tilts it about its vertical
    /// axis (the normal turns left or right).
    TiltV,
}

impl Handle {
    fn name_key(self) -> &'static str {
        match self {
            Handle::Slide => "clip-handle-slide",
            Handle::TiltU => "clip-handle-tilt-u",
            Handle::TiltV => "clip-handle-tilt-v",
        }
    }

    fn tip_key(self) -> &'static str {
        match self {
            Handle::Slide => "clip-handle-slide-tip",
            _ => "clip-handle-tilt-tip",
        }
    }
}

/// The plane the handles move (0-based).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Clip(usize),
    Slice(usize),
}

impl Target {
    /// The tool that slides or tilts it, and its input naming it.
    fn tool(self, rotate: bool) -> (&'static str, &'static str, usize) {
        match (self, rotate) {
            (Target::Clip(k), false) => ("clip.set", "plane", k + 1),
            (Target::Clip(k), true) => ("clip.rotate", "plane", k + 1),
            (Target::Slice(k), false) => ("slice.set", "slice", k + 1),
            (Target::Slice(k), true) => ("slice.rotate", "slice", k + 1),
        }
    }
}

/// A drag of a handle under way: what it turns about stays fixed until
/// the button is let go, so the plane turns smoothly about one line.
#[derive(Clone, Copy, Debug)]
pub struct ClipDrag {
    pub handle: Handle,
    pub pivot: [f64; 3],
    /// The tilt axis (world), the plane's own axis when the drag began.
    pub axis: [f64; 3],
    pub start_normal: [f64; 3],
    /// Which side of the pivot each arc sits on (+1 or -1), kept for the
    /// drag so the arcs do not jump.
    pub sides: [f64; 2],
    /// The plane dragged; a drag ends when another plane takes the
    /// handles.
    pub plane: Target,
}

/// Where a handle is on screen this frame: the middle of its centre line
/// and the direction a drag moves it (for the window tests and the
/// pointer).
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(test), allow(dead_code))] // read by the window tests
pub struct Spot {
    pub handle: Handle,
    pub at: Pos2,
    pub dir: egui::Vec2,
}

// Sizes on screen, in points, whatever the zoom.
const ARROW_LEN: f64 = 108.0;
const SHAFT_R: f64 = 3.0;
const HEAD_R: f64 = 9.0;
const HEAD_LEN: f64 = 24.0;
const ARC_R: f64 = 84.0;
const ARC_HALF_DEG: f64 = 26.0;
const ARC_TUBE_R: f64 = 2.6;
const ARC_HEAD_R: f64 = 7.0;
const ARC_HEAD_LEN: f64 = 15.0;
/// The halo around the handles, in points.
const HALO: f64 = 1.2;
/// The ticks of the rotation plane, in degrees (a longer one every 45).
const TICK_DEG: usize = 15;
/// How near the pointer must come to a handle's centre line.
const HIT: f32 = 10.0;
/// The share of the pointer's movement a drag takes while Shift is held.
const FINE: f64 = 0.2;
const SIDES: usize = 16;

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

/// A vertex of the handles' surface: where it is and its normal.
#[derive(Clone, Copy)]
struct Vx {
    p: V3,
    n: V3,
}

/// Triangles of a handle, in world space.
#[derive(Default)]
struct Solid {
    tris: Vec<[Vx; 3]>,
}

impl Solid {
    /// A tube of radius `r` along `path`, its rings framed by parallel
    /// transport, closed at both ends by discs.
    fn tube(&mut self, path: &[V3], r: f64) {
        if path.len() < 2 {
            return;
        }
        let tangent = |i: usize| {
            let (a, b) = (path[i.saturating_sub(1)], path[(i + 1).min(path.len() - 1)]);
            unit(sub(b, a)).unwrap_or([0.0, 0.0, 1.0])
        };
        let t0 = tangent(0);
        let (mut n, _) = murmel_engine::clip::plane_axes(t0);
        let mut rings: Vec<Vec<Vx>> = Vec::with_capacity(path.len());
        for (i, &c) in path.iter().enumerate() {
            let t = tangent(i);
            n = unit(sub(n, mul(t, dot(t, n)))).unwrap_or(n);
            let b = cross(t, n);
            rings.push(
                (0..SIDES)
                    .map(|k| {
                        let a = k as f64 / SIDES as f64 * std::f64::consts::TAU;
                        let d = add(mul(n, a.cos()), mul(b, a.sin()));
                        Vx {
                            p: add(c, mul(d, r)),
                            n: d,
                        }
                    })
                    .collect(),
            );
        }
        for w in rings.windows(2) {
            for k in 0..SIDES {
                let k1 = (k + 1) % SIDES;
                self.tris.push([w[0][k], w[1][k], w[1][k1]]);
                self.tris.push([w[0][k], w[1][k1], w[0][k1]]);
            }
        }
        let last = path.len() - 1;
        self.disc(path[0], &rings[0], mul(tangent(0), -1.0));
        self.disc(path[last], &rings[last], tangent(last));
    }

    /// A flat disc closing a ring, facing `n`.
    fn disc(&mut self, centre: V3, ring: &[Vx], n: V3) {
        let c = Vx { p: centre, n };
        for k in 0..ring.len() {
            let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
            self.tris.push([c, Vx { p: a.p, n }, Vx { p: b.p, n }]);
        }
    }

    /// A cone of radius `r` and length `len` on `base`, pointing along
    /// the unit vector `dir`, with its base closed.
    fn cone(&mut self, base: V3, dir: V3, len: f64, r: f64) {
        let (u, v) = murmel_engine::clip::plane_axes(dir);
        let apex = add(base, mul(dir, len));
        // The surface normal leans back from the radial direction by the
        // half angle of the cone.
        let slope = (r / len).atan();
        let ring: Vec<Vx> = (0..SIDES)
            .map(|k| {
                let a = k as f64 / SIDES as f64 * std::f64::consts::TAU;
                let d = add(mul(u, a.cos()), mul(v, a.sin()));
                Vx {
                    p: add(base, mul(d, r)),
                    n: add(mul(d, slope.cos()), mul(dir, slope.sin())),
                }
            })
            .collect();
        for k in 0..SIDES {
            let (a, b) = (ring[k], ring[(k + 1) % SIDES]);
            let mid = unit(add(a.n, b.n)).unwrap_or(a.n);
            self.tris.push([a, b, Vx { p: apex, n: mid }]);
        }
        self.disc(base, &ring, mul(dir, -1.0));
    }
}

/// The plane's own axes on screen: `u` as near the screen's right as the
/// plane allows, `v = n × u` (so `u × v = n`). A tilt drag keeps its axis.
fn plane_frame(c: &Clip, cam: &Camera, drag: Option<&ClipDrag>) -> (V3, V3) {
    let n = c.normal;
    match drag.map(|d| (d.handle, d.axis)) {
        Some((Handle::TiltU, a)) => (a, cross(n, a)),
        Some((Handle::TiltV, a)) => (cross(a, n), a),
        _ => {
            let (right, up) = cam.basis();
            let (right, up) = (right.to_array(), up.to_array());
            let u = unit(sub(right, mul(n, dot(n, right))))
                .or_else(|| unit(sub(up, mul(n, dot(n, up)))))
                .unwrap_or([1.0, 0.0, 0.0]);
            (u, cross(n, u))
        }
    }
}

/// The geometry of one handle: its centre line (for the pointer) and
/// what a drag along it does.
struct Shape {
    handle: Handle,
    line: Vec<V3>,
    /// For a tilt: the axis, the point the drag follows and the direction
    /// it moves in per radian.
    tilt: Option<(V3, V3, V3)>,
    /// For a tilt: the arc's middle as a unit vector from the pivot (in
    /// the clip plane, square to the axis).
    toward: V3,
}

/// The arc of a tilt handle about axis `a`: in the plane of `b` (in the
/// clip plane) and `t = a × b` (across it), centred on `b`.
fn arc(pivot: V3, b: V3, t: V3, radius: f64, half_deg: f64, steps: usize) -> Vec<V3> {
    (0..=steps)
        .map(|i| {
            let th = (-half_deg + 2.0 * half_deg * i as f64 / steps as f64).to_radians();
            add(pivot, mul(add(mul(b, th.cos()), mul(t, th.sin())), radius))
        })
        .collect()
}

/// The plane a tilt arc turns the clip plane in: through the pivot,
/// square to the tilt axis, drawn while the arc is hovered or dragged.
struct RotationPlane {
    pivot: V3,
    /// The tilt axis (unit).
    axis: V3,
    /// The arc's middle now (unit, square to the axis).
    toward: V3,
    /// The arc's middle when the drag began, during a drag.
    start: Option<V3>,
    /// The arc's radius in world units.
    radius: f64,
}

impl RotationPlane {
    /// The point at `deg` degrees from `from` about the axis, at `r`
    /// times the radius.
    fn at(&self, from: V3, deg: f64, r: f64) -> V3 {
        let t = cross(self.axis, from);
        let a = deg.to_radians();
        add(
            self.pivot,
            mul(add(mul(from, a.cos()), mul(t, a.sin())), r * self.radius),
        )
    }

    /// The signed angle (degrees) from the drag's start to now, about the
    /// axis; 0 without a drag.
    fn turned(&self) -> f64 {
        self.start.map_or(0.0, |s| {
            dot(cross(s, self.toward), self.axis)
                .atan2(dot(s, self.toward))
                .to_degrees()
        })
    }

    /// Paint the plane in `c` over a thin `shade` outline, so it reads
    /// on light and dark meshes alike.
    fn paint(
        &self,
        painter: &egui::Painter,
        to_screen: &dyn Fn(V3) -> Option<Pos2>,
        c: Color32,
        shade: Color32,
    ) {
        let from = self.start.unwrap_or(self.toward);
        let ring: Option<Vec<Pos2>> = (0..72)
            .map(|k| to_screen(self.at(from, k as f64 * 5.0, 1.0)))
            .collect();
        let (Some(ring), Some(centre)) = (ring, to_screen(self.pivot)) else {
            return;
        };
        // The disc (a fan, so it stays right seen nearly edge on).
        painter.add(egui::Shape::mesh(fan(
            centre,
            &ring,
            true,
            c.gamma_multiply(0.24),
        )));
        let mut closed = ring.clone();
        closed.push(ring[0]);
        let outline = shade.gamma_multiply(0.45);
        painter.add(egui::Shape::line(
            closed.clone(),
            egui::Stroke::new(3.2, outline),
        ));
        painter.add(egui::Shape::line(closed, egui::Stroke::new(1.6, c)));
        for k in (0..360).step_by(TICK_DEG) {
            let major = k % 45 == 0;
            let inner = if major { 0.8 } else { 0.89 };
            let (Some(a), Some(b)) = (
                to_screen(self.at(from, k as f64, inner)),
                to_screen(self.at(from, k as f64, 1.0)),
            ) else {
                continue;
            };
            let w = if major { 1.8 } else { 1.2 };
            painter.line_segment([a, b], egui::Stroke::new(w + 1.6, outline));
            painter.line_segment([a, b], egui::Stroke::new(w, c));
        }
        // The axis the plane turns about.
        if let (Some(a), Some(b)) = (
            to_screen(add(self.pivot, mul(self.axis, -1.2 * self.radius))),
            to_screen(add(self.pivot, mul(self.axis, 1.2 * self.radius))),
        ) {
            painter.add(egui::Shape::dashed_line(
                &[a, b],
                egui::Stroke::new(1.2, c.gamma_multiply(0.8)),
                5.0,
                4.0,
            ));
        }
        // The wedge of the turn so far.
        let deg = self.turned();
        if deg.abs() > 0.05 {
            let steps = ((deg.abs() / 3.0).ceil() as usize).max(1);
            let arc: Option<Vec<Pos2>> = (0..=steps)
                .map(|i| to_screen(self.at(from, deg * i as f64 / steps as f64, 0.9)))
                .collect();
            if let Some(arc) = arc {
                painter.add(egui::Shape::mesh(fan(
                    centre,
                    &arc,
                    false,
                    c.gamma_multiply(0.45),
                )));
            }
        }
    }
}

/// A triangle fan from `centre` over `rim` (closed back to the first
/// point when `closed`) in one colour.
fn fan(centre: Pos2, rim: &[Pos2], closed: bool, c: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(centre, c);
    for &p in rim {
        mesh.colored_vertex(p, c);
    }
    let n = rim.len() as u32;
    let segments = if closed { n } else { n.saturating_sub(1) };
    for k in 0..segments {
        mesh.add_triangle(0, 1 + k, 1 + (k + 1) % n);
    }
    mesh
}

/// The shaded handles as one egui mesh: per-vertex colour from the
/// light, triangles back to front; `halo` draws them inflated in the
/// shade colour instead.
struct Painter<'a> {
    cam: &'a Camera,
    to_screen: &'a dyn Fn(V3) -> Option<Pos2>,
    eye: V3,
    light: V3,
    half: V3,
}

impl Painter<'_> {
    fn mesh(&self, solids: &[(Solid, Color32)], g: &GizmoTokens) -> Mesh {
        let shade = color(g.shade);
        let shine = color(g.shine);
        let mut tris: Vec<(f64, [(Pos2, Color32); 3])> = Vec::new();
        for (solid, base) in solids {
            for tri in &solid.tris {
                let mean_n = unit(add(add(tri[0].n, tri[1].n), tri[2].n)).unwrap_or(tri[0].n);
                if dot(mean_n, self.view_dir(tri[0].p)) < -0.35 {
                    continue;
                }
                let mut out = [(Pos2::ZERO, Color32::TRANSPARENT); 3];
                let mut ok = true;
                for (o, v) in out.iter_mut().zip(tri) {
                    let Some(s) = (self.to_screen)(v.p) else {
                        ok = false;
                        break;
                    };
                    let diffuse = dot(v.n, self.light).max(0.0);
                    let spec = dot(v.n, self.half).max(0.0).powi(36);
                    let dark = (1.0 - (0.2 + 0.8 * diffuse)) as f32 * g.shade_mix;
                    let c = lerp(*base, shade, dark);
                    *o = (s, lerp(c, shine, spec as f32 * g.shine_mix));
                }
                if ok {
                    let depth = tri.iter().map(|v| dot(v.p, self.eye)).sum::<f64>();
                    tris.push((depth, out));
                }
            }
        }
        tris.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut mesh = Mesh::default();
        for (_, t) in tris {
            let i = mesh.vertices.len() as u32;
            for (pos, c) in t {
                mesh.vertices.push(Vertex {
                    pos,
                    uv: WHITE_UV,
                    color: c,
                });
            }
            mesh.indices.extend([i, i + 1, i + 2]);
        }
        mesh
    }

    /// From a point towards the viewer.
    fn view_dir(&self, p: V3) -> V3 {
        if self.cam.is_orthographic() {
            self.eye
        } else {
            unit(sub(self.cam.eye().to_array(), p)).unwrap_or(self.eye)
        }
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The distance from `p` to the polyline `line` on screen, and the
/// nearest point on it.
fn nearest(line: &[Pos2], p: Pos2) -> Option<(f32, Pos2)> {
    line.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
            let q = a + ab * t;
            (q.distance(p), q)
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
}

/// A number with about four significant digits.
fn short(v: f64) -> String {
    let mag = if v == 0.0 {
        0
    } else {
        v.abs().log10().floor() as i32
    };
    let decimals = (3 - mag).clamp(0, 6) as usize;
    format!("{v:.decimals$}")
}

impl MurmelApp {
    /// The plane that carries the handles, if it shows: the clip plane or
    /// slice selected last, else the slice added last, else the current
    /// clip plane. A slice series moves by its middle plane.
    pub(crate) fn handle_target(&self) -> Option<(Target, Clip)> {
        let d = &self.session.display;
        let clip = |k: usize| {
            d.clips
                .get(k)
                .filter(|c| c.enabled)
                .map(|c| (Target::Clip(k), *c))
        };
        let slice = |k: usize| {
            d.slices
                .get(k)
                .filter(|c| c.enabled)
                .map(|c| (Target::Slice(k), c.plane()))
        };
        // Streamline seeds selected after any plane take the handles.
        let seeds = self.stream_handle_target().is_some();
        let picked = d.selection.iter().rev().find_map(|&i| match i {
            murmel_engine::Item::Clip(k) => Some(clip(k as usize)),
            murmel_engine::Item::Slice(k) => Some(slice(k as usize)),
            murmel_engine::Item::Stream(_) if seeds => Some(None),
            _ => None,
        });
        match picked {
            Some(t) => t,
            None => d
                .current_slice()
                .and_then(slice)
                .or_else(|| d.current_clip().and_then(clip)),
        }
    }

    /// The outline inside the mesh's box of the clip plane or slice that
    /// carries the handles, and the handles.
    pub(crate) fn clip_handles(&mut self, ui: &Ui, rect: Rect, cam: &Camera, dark: bool) {
        self.clip_spots.clear();
        self.rotation_plane = None;
        // The calls name the plane, so the drag stays on it.
        let Some((current, c)) = self.handle_target() else {
            self.clip_drag = None;
            return;
        };
        if self.clip_drag.is_some_and(|d| d.plane != current) {
            self.clip_drag = None;
        }
        let Some(m) = &self.session.mesh else { return };
        let bounds = m.mesh.bounds();
        let section = c.box_section(&bounds);
        if section.is_empty() {
            self.clip_drag = None;
            return;
        }
        let palette = self.session.tokens.palette(dark).clone();
        let gizmo = self.session.tokens.gizmo.clone();
        let accent = color(palette.accent);
        let colors = *gizmo.colors(dark);
        // Each handle its own colour, and a tilt arc's rotation plane its arc's.
        let own = |h: Handle| {
            color(match h {
                Handle::Slide => colors.slide,
                Handle::TiltU => colors.tilt,
                Handle::TiltV => colors.tilt2,
            })
        };
        let slide_c = own(Handle::Slide);
        let (origin, sradius) = (m.surface.origin, m.surface.radius);
        let aspect = (rect.width() / rect.height().max(1.0)) as f64;
        let to_screen = move |p: V3| {
            let [x, y] = cam.project(origin, sradius, aspect, p)?;
            Some(Pos2::new(
                rect.left() + ((x + 1.0) * 0.5) as f32 * rect.width(),
                rect.top() + ((1.0 - y) * 0.5) as f32 * rect.height(),
            ))
        };
        let Some(pts) = section
            .iter()
            .map(|&p| to_screen(p))
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        let painter = ui.painter_at(rect);
        painter.add(egui::Shape::convex_polygon(
            pts.clone(),
            accent.gamma_multiply(0.10),
            egui::Stroke::NONE,
        ));
        let mut ring = pts.clone();
        ring.push(pts[0]);
        painter.add(egui::Shape::dashed_line(
            &ring,
            egui::Stroke::new(1.5, accent),
            6.0,
            4.0,
        ));

        // Pointer state first: a drag ends when the button is let go.
        if !ui.input(|i| i.pointer.any_down()) {
            self.clip_drag = None;
        }
        let drag = self.clip_drag;
        let pivot = drag.map(|d| d.pivot).unwrap_or_else(|| c.pivot(&bounds));
        let n = c.normal;
        let (u, v) = plane_frame(&c, cam, drag.as_ref());

        // World size of a screen point at the pivot.
        let radius = bounds.radius().max(1e-9);
        let (right, _) = cam.basis();
        let (Some(p0), Some(p1)) = (
            to_screen(pivot),
            to_screen(add(pivot, mul(right.to_array(), radius))),
        ) else {
            return;
        };
        let px = (p1 - p0).length() as f64;
        if px < 1e-6 {
            return;
        }
        let s = radius / px;

        let shaft_end = add(pivot, mul(n, (ARROW_LEN - HEAD_LEN) * s));
        // The arcs sit on the plane at ±v (the tilt about u) and ±u (the
        // tilt about v); each side is chosen so that the arrow and the two arcs point apart on
        // screen, which keeps them from crossing when the plane is seen
        // at a grazing angle.
        let sides = drag.map(|d| d.sides).unwrap_or_else(|| {
            let dir = |w: V3| to_screen(add(pivot, mul(w, ARC_R * s))).map(|q| q - p0);
            let arrow = to_screen(add(pivot, mul(n, ARROW_LEN * s)))
                .map(|q| q - p0)
                .filter(|d| d.length() > 12.0);
            let (Some(dv), Some(du)) = (dir(v), dir(u)) else {
                return [1.0, 1.0];
            };
            let cos =
                |a: egui::Vec2, b: egui::Vec2| a.normalized().dot(b.normalized()).clamp(-1.0, 1.0);
            let mut best = ([1.0, 1.0], f32::INFINITY);
            for sv in [1.0f32, -1.0] {
                for su in [1.0f32, -1.0] {
                    let (a, b) = (dv * sv, du * su);
                    // The worst (largest) cosine between any two handles.
                    let mut worst = cos(a, b);
                    if let Some(w) = arrow {
                        worst = worst.max(cos(a, w)).max(cos(b, w));
                    }
                    // Prefer the plane's upper and right sides on a tie.
                    let score = worst - 0.01 * (sv + su);
                    if score < best.1 {
                        best = ([sv as f64, su as f64], score);
                    }
                }
            }
            best.0
        });
        let (bv, bu) = (mul(v, sides[0]), mul(u, sides[1]));
        let shapes = [
            Shape {
                handle: Handle::Slide,
                line: vec![pivot, add(pivot, mul(n, ARROW_LEN * s))],
                tilt: None,
                toward: n,
            },
            // About u: the arc sits on ±v and moves along u × (±v) = ±n.
            Shape {
                handle: Handle::TiltU,
                line: arc(pivot, bv, cross(u, bv), ARC_R * s, ARC_HALF_DEG, 24),
                tilt: Some((
                    u,
                    add(pivot, mul(bv, ARC_R * s)),
                    mul(cross(u, bv), ARC_R * s),
                )),
                toward: bv,
            },
            // About v: the arc sits on ±u and moves along v × (±u) = ∓n.
            Shape {
                handle: Handle::TiltV,
                line: arc(pivot, bu, cross(v, bu), ARC_R * s, ARC_HALF_DEG, 24),
                tilt: Some((
                    v,
                    add(pivot, mul(bu, ARC_R * s)),
                    mul(cross(v, bu), ARC_R * s),
                )),
                toward: bu,
            },
        ];

        // The handle under the pointer: the nearest within reach.
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let lines: Vec<Option<Vec<Pos2>>> = shapes
            .iter()
            .map(|sh| sh.line.iter().map(|&p| to_screen(p)).collect())
            .collect();
        for (sh, l) in shapes.iter().zip(&lines) {
            let Some(l) = l else { continue };
            let mid = l[l.len() / 2];
            let dir = match sh.tilt {
                None => l[l.len() - 1] - l[0],
                Some(_) => l[l.len() / 2 + 1] - l[l.len() / 2 - 1],
            };
            let at = if sh.handle == Handle::Slide {
                l[0] + (l[1] - l[0]) * 0.6
            } else {
                mid
            };
            self.clip_spots.push(Spot {
                handle: sh.handle,
                at,
                dir: dir.normalized(),
            });
        }
        let near = pointer.and_then(|p| {
            shapes
                .iter()
                .zip(&lines)
                .filter_map(|(sh, l)| {
                    let (d, q) = nearest(l.as_deref()?, p)?;
                    // The arrow's head is wider than its shaft.
                    let reach = if sh.handle == Handle::Slide {
                        HIT + 3.0
                    } else {
                        HIT
                    };
                    (d < reach).then_some((sh.handle, d, q))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
        });

        let fine = ui.input(|i| i.modifiers.shift);
        let mut hot = None;
        for sh in &shapes {
            let id = ui.scope_id().with(("clip-handle", sh.handle));
            let area = match (drag, near) {
                (Some(d), _) if d.handle == sh.handle => rect,
                (None, Some((h, _, q))) if h == sh.handle => {
                    Rect::from_center_size(q, egui::vec2(2.0 * HIT, 2.0 * HIT))
                }
                _ => continue,
            };
            let resp = ui.interact(area, id, Sense::drag());
            let name = self.i18n.tr(sh.handle.name_key());
            resp.widget_info(|| WidgetInfo::labeled(Role::Slider, true, name.clone()));
            if resp.hovered() || resp.dragged() {
                hot = Some(sh.handle);
                ui.ctx().set_cursor_icon(if resp.dragged() {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                });
            }
            if resp.drag_started() {
                self.clip_drag = Some(ClipDrag {
                    handle: sh.handle,
                    pivot,
                    axis: sh.tilt.map(|t| t.0).unwrap_or(n),
                    start_normal: n,
                    sides,
                    plane: current,
                });
            }
            if !resp.dragged() {
                if self.clip_drag.is_none() {
                    resp.on_hover_text(self.i18n.tr(sh.handle.tip_key()));
                }
                continue;
            }
            let d = resp.drag_delta();
            let k = if fine { FINE } else { 1.0 };
            match sh.tilt {
                None => {
                    let (Some(a), Some(b)) = (to_screen(sh.line[0]), to_screen(sh.line[1])) else {
                        continue;
                    };
                    let axis = b - a;
                    let len = axis.length() as f64;
                    if len > 4.0 {
                        let along = (d.x * axis.x + d.y * axis.y) as f64 / len;
                        let metres = k * along * (ARROW_LEN * s) / len;
                        if metres != 0.0 {
                            let (off, [lo, hi]) = c.offset(&bounds);
                            let v = (off + metres).clamp(lo, hi);
                            let (tool, key, n) = current.tool(false);
                            self.call(tool, json!({ key: n, "offset": v }));
                        }
                    }
                }
                Some((axis, at, per_rad)) => {
                    // The screen motion of the arc's middle per radian.
                    let h = 0.05;
                    let (Some(a), Some(b)) = (to_screen(at), to_screen(add(at, mul(per_rad, h))))
                    else {
                        continue;
                    };
                    let t = (b - a) / h as f32;
                    let l2 = t.length_sq() as f64;
                    if l2 > 64.0 {
                        let rad = k * (d.x * t.x + d.y * t.y) as f64 / l2;
                        if rad != 0.0 {
                            let deg = rad.to_degrees();
                            let (tool, key, n) = current.tool(true);
                            self.call(
                                tool,
                                json!({ key: n, "axis": axis, "deg": deg, "pivot": pivot }),
                            );
                        }
                    }
                }
            }
        }

        // The rotation plane of the arc under the pointer or dragged,
        // under the handles.
        let turning = self
            .clip_drag
            .map(|d| d.handle)
            .or(hot)
            .filter(|h| *h != Handle::Slide);
        if let Some(sh) = turning.and_then(|h| shapes.iter().find(|sh| sh.handle == h))
            && let Some((axis, _, _)) = sh.tilt
        {
            let start = self.clip_drag.map(|d| {
                // Where the arc's middle was when the drag began: the
                // same frame built from the normal at the press.
                let w = match d.handle {
                    Handle::TiltU => mul(cross(d.start_normal, axis), d.sides[0]),
                    _ => mul(cross(axis, d.start_normal), d.sides[1]),
                };
                unit(w).unwrap_or(sh.toward)
            });
            let plane = RotationPlane {
                pivot,
                axis,
                toward: sh.toward,
                start,
                radius: ARC_R * s,
            };
            plane.paint(&painter, &to_screen, own(sh.handle), color(gizmo.shade));
            self.rotation_plane = Some(sh.handle);
        }

        // Draw: halo first, then the shaded handles.
        let lift = |c: Color32| lerp(c, color(gizmo.shine), 0.22);
        let base = |h: Handle| {
            if hot == Some(h) { lift(own(h)) } else { own(h) }
        };
        let grow = |h: Handle| if hot == Some(h) { 1.3 } else { 1.0 };
        let build = |extra: f64| {
            let mut out = Vec::new();
            let g = grow(Handle::Slide);
            let mut arrow = Solid::default();
            arrow.tube(&[pivot, shaft_end], (SHAFT_R * g + extra) * s);
            arrow.cone(
                add(shaft_end, mul(n, -extra * s)),
                n,
                (HEAD_LEN + 2.0 * extra) * s,
                (HEAD_R * g + extra) * s,
            );
            out.push((arrow, base(Handle::Slide)));
            for sh in &shapes[1..] {
                let g = grow(sh.handle);
                let mut a = Solid::default();
                a.tube(&sh.line, (ARC_TUBE_R * g + extra) * s);
                for (end, prev) in [(sh.line.len() - 1, sh.line.len() - 2), (0, 1)] {
                    let dir = unit(sub(sh.line[end], sh.line[prev])).unwrap_or(n);
                    a.cone(
                        add(sh.line[end], mul(dir, -extra * s)),
                        dir,
                        (ARC_HEAD_LEN + 2.0 * extra) * s,
                        (ARC_HEAD_R * g + extra) * s,
                    );
                }
                out.push((a, base(sh.handle)));
            }
            out
        };
        let eye = cam.to_eye().to_array();
        let (r, up) = cam.basis();
        let light = unit(add(
            add(mul(r.to_array(), -0.35), mul(up.to_array(), 0.55)),
            mul(eye, 0.75),
        ))
        .unwrap_or(eye);
        let p = Painter {
            cam,
            to_screen: &to_screen,
            eye,
            light,
            half: unit(add(light, eye)).unwrap_or(eye),
        };
        // The halo: the same solids a little larger, in the shade colour.
        let halo: Vec<(Solid, Color32)> = build(HALO)
            .into_iter()
            .map(|(sol, _)| (sol, color(gizmo.shade)))
            .collect();
        let mut halo_mesh = p.mesh(&halo, &gizmo);
        for v in &mut halo_mesh.vertices {
            v.color = color(gizmo.shade);
        }
        painter.add(egui::Shape::mesh(halo_mesh));
        painter.add(egui::Shape::mesh(p.mesh(&build(0.0), &gizmo)));
        painter.circle_filled(p0, 4.0, color(gizmo.shade));
        painter.circle_filled(p0, 2.8, slide_c);

        // While dragging: the value beside the pointer.
        if let (Some(d), Some(at)) = (self.clip_drag, pointer) {
            let text = match d.handle {
                Handle::Slide => {
                    let (off, _) = c.offset(&bounds);
                    self.i18n
                        .tr_args("clip-handle-offset", &[("value", short(off).into())])
                }
                _ => {
                    let cosang = dot(d.start_normal, n).clamp(-1.0, 1.0);
                    let sign = dot(cross(d.start_normal, n), d.axis).signum();
                    let deg = sign * cosang.acos().to_degrees();
                    self.i18n
                        .tr_args("clip-handle-angle", &[("deg", format!("{deg:.1}").into())])
                }
            };
            let galley = painter.layout_no_wrap(
                text,
                FontId::proportional(12.0),
                color(palette.text_primary),
            );
            let pos = at + egui::vec2(16.0, 14.0);
            let bg = Rect::from_min_size(pos, galley.size()).expand2(egui::vec2(6.0, 3.0));
            painter.rect_filled(bg, 4.0, color(palette.bg_raised));
            painter.rect_stroke(
                bg,
                4.0,
                egui::Stroke::new(1.0, color(palette.line)),
                egui::StrokeKind::Inside,
            );
            painter.galley(pos, galley, color(palette.text_primary));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the arc of the tilt about u sits on v and moves towards
    /// the normal, so dragging it along its own direction turns the
    /// normal the way the arc points (u × v = n). Breaks if the frame or
    /// the arc's direction flips; the window test of a drag does not see
    /// which way the arc points.
    #[test]
    fn arc_moves_the_way_it_turns() {
        let c = Clip {
            origin: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            mode: murmel_engine::ClipMode::Crinkle,
            enabled: true,
        };
        let cam = Camera::default();
        let (u, v) = plane_frame(&c, &cam, None);
        assert!((dot(cross(u, v), c.normal) - 1.0).abs() < 1e-12);
        let line = arc([0.0; 3], v, c.normal, 1.0, 10.0, 2);
        assert!((dot(line[1], v) - 1.0).abs() < 1e-12, "the middle is on v");
        assert!(dot(sub(line[2], line[0]), c.normal) > 0.0);
        let mut turned = c;
        turned.rotate(u, 5.0, [0.0; 3]).unwrap();
        // The normal turned away from v: the plane's v side moved
        // towards where the normal was, as the arc's end shows.
        assert!(dot(turned.normal, v) < 0.0);
        assert_eq!(short(0.012345), "0.01235");
        assert_eq!(short(1234.5), "1234");
    }

    /// Invariant: a tube is closed (two discs and the sides) and the
    /// nearest point of a polyline is on it. Breaks if the rings leak.
    #[test]
    fn tube_and_nearest() {
        let mut s = Solid::default();
        s.tube(&[[0.0; 3], [0.0, 0.0, 1.0]], 0.1);
        assert_eq!(s.tris.len(), 2 * SIDES + 2 * SIDES);
        let (d, q) = nearest(
            &[Pos2::new(0.0, 0.0), Pos2::new(10.0, 0.0)],
            Pos2::new(5.0, 3.0),
        )
        .unwrap();
        assert_eq!((d, q), (3.0, Pos2::new(5.0, 0.0)));
    }
}
