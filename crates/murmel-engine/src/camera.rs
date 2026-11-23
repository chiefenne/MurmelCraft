//! The viewer camera: a turntable about the up axis (Z up), so the horizon
//! never tilts, with the three projection modes of the UI spec.
//!
//! The camera looks at `target` from `distance` along the direction given
//! by `yaw` (about the up axis, from +X) and `pitch` (elevation). Matrices
//! are built relative to a scene origin so large coordinates keep their
//! precision in `f32`.

use glam::{DMat4, DVec3, DVec4, Mat4};
use murmel_mesh::Bounds;
use serde::{Deserialize, Serialize};

/// Fusion's three modes (UI spec, Projection).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    Orthographic,
    Perspective,
    /// Perspective while orbiting; orthographic after a standard view
    /// until the next orbit.
    #[default]
    PerspectiveOrthoFaces,
}

impl Projection {
    pub fn next(self) -> Projection {
        match self {
            Projection::Orthographic => Projection::Perspective,
            Projection::Perspective => Projection::PerspectiveOrthoFaces,
            Projection::PerspectiveOrthoFaces => Projection::Orthographic,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Projection::Orthographic => "orthographic",
            Projection::Perspective => "perspective",
            Projection::PerspectiveOrthoFaces => "perspective_ortho_faces",
        }
    }
}

/// The standard views: the 26 views of the view cube (6 faces, 12 edges,
/// 8 corners) and home (iso, fitted).
///
/// A cube view is the direction from the target to the eye, one of
/// -1, 0, 1 per axis: x right (+) or left (-), y back (+) or front (-),
/// z top (+) or bottom (-). Its name lists the parts front/back,
/// left/right, top/bottom, joined by `_`: `front`, `front_top`,
/// `front_right_top`; `iso` is `front_right_top`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StandardView {
    Home,
    Cube([i8; 3]),
}

impl StandardView {
    pub const FRONT: StandardView = StandardView::Cube([0, -1, 0]);
    pub const BACK: StandardView = StandardView::Cube([0, 1, 0]);
    pub const LEFT: StandardView = StandardView::Cube([-1, 0, 0]);
    pub const RIGHT: StandardView = StandardView::Cube([1, 0, 0]);
    pub const TOP: StandardView = StandardView::Cube([0, 0, 1]);
    pub const BOTTOM: StandardView = StandardView::Cube([0, 0, -1]);
    pub const ISO: StandardView = StandardView::Cube([1, -1, 1]);

    /// The 26 cube views: faces, then edges, then corners.
    pub fn cube() -> Vec<StandardView> {
        let mut v = Vec::with_capacity(26);
        for parts in 1..=3 {
            for z in [0i8, 1, -1] {
                for y in [-1i8, 1, 0] {
                    for x in [1i8, -1, 0] {
                        let n = [x, y, z].iter().filter(|c| **c != 0).count();
                        if n == parts {
                            v.push(StandardView::Cube([x, y, z]));
                        }
                    }
                }
            }
        }
        v
    }

    /// Every name a tool accepts: the cube views, `iso` and `home`.
    pub fn names() -> Vec<String> {
        let mut v: Vec<String> = StandardView::cube().iter().map(|s| s.name()).collect();
        v.push("iso".into());
        v.push("home".into());
        v
    }

    pub fn name(self) -> String {
        let StandardView::Cube([x, y, z]) = self else {
            return "home".into();
        };
        let mut parts = Vec::new();
        match y {
            -1 => parts.push("front"),
            1 => parts.push("back"),
            _ => {}
        }
        match x {
            -1 => parts.push("left"),
            1 => parts.push("right"),
            _ => {}
        }
        match z {
            1 => parts.push("top"),
            -1 => parts.push("bottom"),
            _ => {}
        }
        parts.join("_")
    }

    /// The view a name gives, its parts in any order; `None` for an
    /// unknown part, a repeated axis or no part.
    pub fn parse(name: &str) -> Option<StandardView> {
        match name {
            "home" => return Some(StandardView::Home),
            "iso" => return Some(StandardView::ISO),
            _ => {}
        }
        let mut d = [0i8; 3];
        for part in name.split('_') {
            let (axis, sign) = match part {
                "right" => (0, 1),
                "left" => (0, -1),
                "back" => (1, 1),
                "front" => (1, -1),
                "top" => (2, 1),
                "bottom" => (2, -1),
                _ => return None,
            };
            if d[axis] != 0 {
                return None;
            }
            d[axis] = sign;
        }
        (d != [0; 3]).then_some(StandardView::Cube(d))
    }

    /// Unit vector from the target to the eye.
    pub fn direction(self) -> DVec3 {
        match self {
            StandardView::Home => StandardView::ISO.direction(),
            StandardView::Cube(d) => {
                DVec3::new(d[0] as f64, d[1] as f64, d[2] as f64).normalize_or(DVec3::Z)
            }
        }
    }

    /// Yaw and pitch in degrees. Front looks along +Y, right along -X,
    /// top down -Z with the front at the bottom of the screen.
    pub fn angles(self) -> (f64, f64) {
        let d = self.direction();
        let yaw = if d.x.abs() < 1e-12 && d.y.abs() < 1e-12 {
            -90.0
        } else {
            d.y.atan2(d.x).to_degrees()
        };
        (yaw, d.z.atan2(d.x.hypot(d.y)).to_degrees())
    }

    /// Face views snap to orthographic in `PerspectiveOrthoFaces`.
    pub fn is_face(self) -> bool {
        matches!(self, StandardView::Cube(d) if d.iter().filter(|c| **c != 0).count() == 1)
    }
}

impl Serialize for StandardView {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.name())
    }
}

impl<'de> Deserialize<'de> for StandardView {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        StandardView::parse(&s).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown view {s}; a view joins front/back, left/right and top/bottom with _ (front_right_top), or is iso or home"
            ))
        })
    }
}

/// A screen axis of [`Camera::rotate`]: x to the right, y up, z out of the
/// screen towards the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenAxis {
    X,
    Y,
    Z,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub target: [f64; 3],
    pub distance: f64,
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    pub projection: Projection,
    /// Vertical field of view of the perspective projection.
    pub fov_deg: f64,
    /// In `PerspectiveOrthoFaces`: the view is snapped to a face.
    #[serde(default)]
    pub snapped: bool,
    /// Turn of the screen about the view direction, counter-clockwise on
    /// screen, in degrees (0 keeps the up axis up).
    #[serde(default)]
    pub roll_deg: f64,
}

impl Default for Camera {
    fn default() -> Self {
        let (yaw, pitch) = StandardView::ISO.angles();
        Self {
            target: [0.0; 3],
            distance: 5.0,
            yaw_deg: yaw,
            pitch_deg: pitch,
            projection: Projection::default(),
            fov_deg: 30.0,
            snapped: false,
            roll_deg: 0.0,
        }
    }
}

/// The matrices of one frame.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub view_proj: Mat4,
    /// Eye position relative to the origin.
    pub eye: [f32; 3],
    /// Unit vector from the target to the eye.
    pub to_eye: [f32; 3],
    /// Unit vector towards the key light: from the eye, a little above and
    /// to the left, so the faces of a box read in three tones.
    pub light: [f32; 3],
    pub orthographic: bool,
}

impl Camera {
    /// Unit vector from the target to the eye.
    pub fn to_eye(&self) -> DVec3 {
        let (y, p) = (self.yaw_deg.to_radians(), self.pitch_deg.to_radians());
        DVec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin())
    }

    /// Screen right and up as world vectors, before the roll.
    fn level_basis(&self) -> (DVec3, DVec3) {
        let y = self.yaw_deg.to_radians();
        let right = DVec3::new(-y.sin(), y.cos(), 0.0);
        let up = self.to_eye().cross(right).normalize_or(DVec3::Z);
        (right, up)
    }

    /// Screen right and up as world vectors.
    pub fn basis(&self) -> (DVec3, DVec3) {
        let (r0, u0) = self.level_basis();
        if self.roll_deg == 0.0 {
            return (r0, u0);
        }
        let (s, c) = self.roll_deg.to_radians().sin_cos();
        // Counter-clockwise on screen: the scene turns, so the camera's
        // axes turn the other way.
        (r0 * c - u0 * s, r0 * s + u0 * c)
    }

    /// Rotate the scene by `deg` about a screen axis (right-hand rule:
    /// positive about z turns it counter-clockwise on screen). Any
    /// orientation results; yaw, pitch and roll are recovered from it, so
    /// mouse orbiting stays a turntable afterwards.
    pub fn rotate(&mut self, axis: ScreenAxis, deg: f64) {
        let (right, up) = self.basis();
        let back = self.to_eye();
        let a = match axis {
            ScreenAxis::X => right,
            ScreenAxis::Y => up,
            ScreenAxis::Z => back,
        };
        // The scene turns by +deg, so the camera frame turns by -deg.
        let q = glam::DQuat::from_axis_angle(a, -deg.to_radians());
        let (right, back) = (q * right, q * back);
        let face = back.abs().max_element() > 1.0 - 1e-9;
        self.set_frame(right, back);
        if axis != ScreenAxis::Z {
            self.snapped = face;
        }
    }

    /// Yaw, pitch and roll from the screen right vector and the direction
    /// to the eye.
    fn set_frame(&mut self, right: DVec3, back: DVec3) {
        let back = back.normalize_or(DVec3::Z);
        if back.x.hypot(back.y) > 1e-9 {
            self.yaw_deg = back.y.atan2(back.x).to_degrees();
        }
        self.pitch_deg = back.z.clamp(-1.0, 1.0).asin().to_degrees();
        let (r0, u0) = self.level_basis();
        let roll = (-right.dot(u0)).atan2(right.dot(r0)).to_degrees();
        self.roll_deg = if roll.abs() < 1e-9 { 0.0 } else { roll };
    }

    /// Turn the screen about the view direction by `deg` (the view cube's
    /// arrows send ±90).
    pub fn roll(&mut self, deg: f64) {
        self.rotate(ScreenAxis::Z, deg);
    }

    /// Interpolate from `a` to `b` at `t` (0..1) for the 250 ms flight to
    /// a view: the eye direction and the screen right turn on the shortest
    /// arc, target and distance move linearly; the projection is `b`'s.
    pub fn lerp(a: &Camera, b: &Camera, t: f64) -> Camera {
        let t = t.clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let frame = |c: &Camera| {
            let (r, u) = c.basis();
            glam::DQuat::from_mat3(&glam::DMat3::from_cols(r, u, c.to_eye()))
        };
        let q = frame(a).slerp(frame(b), t);
        let mut c = b.clone();
        let ta = DVec3::from(a.target);
        c.target = (ta + (DVec3::from(b.target) - ta) * t).into();
        c.distance = a.distance + (b.distance - a.distance) * t;
        c.yaw_deg = a.yaw_deg;
        c.set_frame(q * DVec3::X, q * DVec3::Z);
        // Snap to orthographic on arrival, not on take-off.
        c.snapped = a.snapped && b.snapped;
        if t >= 1.0 {
            return b.clone();
        }
        c
    }

    pub fn eye(&self) -> DVec3 {
        DVec3::from(self.target) + self.to_eye() * self.distance
    }

    pub fn is_orthographic(&self) -> bool {
        match self.projection {
            Projection::Orthographic => true,
            Projection::Perspective => false,
            Projection::PerspectiveOrthoFaces => self.snapped,
        }
    }

    /// Half height of the view at the target distance.
    pub fn half_height(&self) -> f64 {
        self.distance * (0.5 * self.fov_deg.to_radians()).tan()
    }

    /// Frame `bounds` in a viewport of `aspect` (width / height): the box
    /// corners, seen from the current direction, fill the view with a
    /// margin. In perspective the camera also backs off by the depth of the
    /// box towards it, so near corners stay inside.
    pub fn fit(&mut self, bounds: &Bounds, aspect: f64) {
        if bounds.is_empty() {
            return;
        }
        let c = DVec3::from(bounds.center());
        self.target = c.into();
        let (right, up) = self.basis();
        let to_eye = self.to_eye();
        let (mut sx, mut sy, mut sz) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..8 {
            let p = DVec3::new(
                if i & 1 == 0 {
                    bounds.min[0]
                } else {
                    bounds.max[0]
                },
                if i & 2 == 0 {
                    bounds.min[1]
                } else {
                    bounds.max[1]
                },
                if i & 4 == 0 {
                    bounds.min[2]
                } else {
                    bounds.max[2]
                },
            ) - c;
            sx = sx.max(p.dot(right).abs());
            sy = sy.max(p.dot(up).abs());
            sz = sz.max(p.dot(to_eye));
        }
        let half = (sy.max(sx / aspect.max(1e-3)) * 1.1).max(1e-9);
        let tan = (0.5 * self.fov_deg.to_radians()).tan();
        // The orthographic size follows the distance, so both projections
        // share one distance; perspective adds the depth towards the eye.
        self.distance = half / tan + if self.is_orthographic() { 0.0 } else { sz };
    }

    pub fn set_standard(&mut self, view: StandardView) {
        let (y, p) = view.angles();
        self.yaw_deg = y;
        self.pitch_deg = p;
        self.roll_deg = 0.0;
        self.snapped = view.is_face();
    }

    /// The cube view the camera looks along, if it looks along one.
    pub fn standard(&self) -> Option<StandardView> {
        let d = self.to_eye();
        StandardView::cube()
            .into_iter()
            .find(|v| (v.direction() - d).length() < 1e-6)
    }

    /// Turn about the up axis by `dyaw` and tilt by `dpitch` degrees.
    pub fn orbit(&mut self, dyaw: f64, dpitch: f64) {
        self.yaw_deg = (self.yaw_deg + dyaw).rem_euclid(360.0);
        if self.yaw_deg > 180.0 {
            self.yaw_deg -= 360.0;
        }
        self.pitch_deg = (self.pitch_deg + dpitch).clamp(-90.0, 90.0);
        self.snapped = false;
    }

    /// Move the view by a drag of `dx`, `dy` pixels in a viewport `height`
    /// pixels high: the scene follows the pointer.
    pub fn pan(&mut self, dx: f64, dy: f64, height: f64) {
        let per_px = 2.0 * self.half_height() / height.max(1.0);
        let (right, up) = self.basis();
        let t = DVec3::from(self.target) - right * dx * per_px + up * dy * per_px;
        self.target = t.into();
    }

    /// Zoom by `factor` (below 1 moves closer). With `cursor` in normalised
    /// device coordinates (-1..1, y up), the point under it stays put.
    pub fn zoom(&mut self, factor: f64, cursor: Option<[f64; 2]>, aspect: f64) {
        let factor = factor.clamp(1e-3, 1e3);
        if let Some([x, y]) = cursor {
            let (right, up) = self.basis();
            let hh = self.half_height();
            let p = DVec3::from(self.target) + right * x * hh * aspect + up * y * hh;
            let t = p + (DVec3::from(self.target) - p) * factor;
            self.target = t.into();
        }
        self.distance = (self.distance * factor).max(1e-9);
    }

    /// View-projection relative to `origin`, for a scene of `radius` about
    /// it, in a viewport of `aspect`. Depth runs 0..1 (wgpu).
    pub fn frame(&self, origin: [f64; 3], radius: f64, aspect: f64) -> Frame {
        let o = DVec3::from(origin);
        let target = DVec3::from(self.target) - o;
        let eye = target + self.to_eye() * self.distance;
        let (_, up) = self.basis();
        let view = look_at_rh(eye, target, up);
        let span = radius.max(1e-9) + (target.length());
        let far = self.distance + 2.0 * span;
        let aspect = aspect.max(1e-3);
        let ortho = self.is_orthographic();
        let proj = if ortho {
            let hh = self.half_height();
            orthographic_rh(
                -hh * aspect,
                hh * aspect,
                -hh,
                hh,
                self.distance - 2.0 * span,
                far,
            )
        } else {
            let near = (self.distance - 2.0 * span).max(far * 1e-5);
            perspective_rh(self.fov_deg.to_radians(), aspect, near, far)
        };
        let to_eye = self.to_eye();
        let (right, up) = self.basis();
        let light = (to_eye + up * 0.55 - right * 0.35).normalize_or(to_eye);
        Frame {
            view_proj: (proj * view).as_mat4(),
            eye: eye.as_vec3().into(),
            to_eye: to_eye.as_vec3().into(),
            light: light.as_vec3().into(),
            orthographic: ortho,
        }
    }
}

/// Right-handed view matrix (camera looks down -Z).
fn look_at_rh(eye: DVec3, target: DVec3, up: DVec3) -> DMat4 {
    let f = (target - eye).normalize_or(DVec3::NEG_Z);
    let s = f.cross(up).normalize_or(DVec3::X);
    let u = s.cross(f);
    DMat4::from_cols(
        DVec4::new(s.x, u.x, -f.x, 0.0),
        DVec4::new(s.y, u.y, -f.y, 0.0),
        DVec4::new(s.z, u.z, -f.z, 0.0),
        DVec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
    )
}

/// Right-handed perspective with depth 0..1.
fn perspective_rh(fov_y: f64, aspect: f64, near: f64, far: f64) -> DMat4 {
    let h = 1.0 / (0.5 * fov_y).tan();
    let r = far / (near - far);
    DMat4::from_cols(
        DVec4::new(h / aspect, 0.0, 0.0, 0.0),
        DVec4::new(0.0, h, 0.0, 0.0),
        DVec4::new(0.0, 0.0, r, -1.0),
        DVec4::new(0.0, 0.0, r * near, 0.0),
    )
}

/// Right-handed orthographic projection with depth 0..1.
fn orthographic_rh(l: f64, r: f64, b: f64, t: f64, near: f64, far: f64) -> DMat4 {
    let d = 1.0 / (near - far);
    DMat4::from_cols(
        DVec4::new(2.0 / (r - l), 0.0, 0.0, 0.0),
        DVec4::new(0.0, 2.0 / (t - b), 0.0, 0.0),
        DVec4::new(0.0, 0.0, d, 0.0),
        DVec4::new(-(r + l) / (r - l), -(t + b) / (t - b), d * near, 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> Bounds {
        Bounds {
            min: [0.0; 3],
            max: [1.0; 3],
        }
    }

    /// Invariant: the default mode snaps to orthographic on a face view and
    /// returns to perspective on the next orbit (decided 2026-11-23).
    #[test]
    fn perspective_with_orthographic_faces() {
        let mut c = Camera::default();
        assert!(!c.is_orthographic());
        c.set_standard(StandardView::FRONT);
        assert!(c.is_orthographic());
        c.orbit(5.0, 0.0);
        assert!(!c.is_orthographic());
        c.set_standard(StandardView::ISO);
        assert!(!c.is_orthographic());
    }

    fn close(a: DVec3, b: DVec3) -> bool {
        (a - b).length() < 1e-9
    }

    /// Invariant: the 26 cube views have 26 distinct names that parse
    /// back (parts in any order), each looks along its own direction, and
    /// only the six faces snap. Unknown parts and repeated axes fail.
    #[test]
    fn the_26_cube_views() {
        let all = StandardView::cube();
        assert_eq!(all.len(), 26);
        let mut names: Vec<String> = all.iter().map(|v| v.name()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 26);
        assert_eq!(all.iter().filter(|v| v.is_face()).count(), 6);
        for v in all {
            assert_eq!(StandardView::parse(&v.name()), Some(v));
            let mut c = Camera::default();
            c.set_standard(v);
            assert!(close(c.to_eye(), v.direction()), "{}", v.name());
            assert_eq!(c.standard(), Some(v));
            let (right, up) = c.basis();
            assert!(right.is_finite() && up.is_finite());
            assert!(right.z.abs() < 1e-12, "{}: horizon tilted", v.name());
        }
        assert_eq!(
            StandardView::parse("top_right_front"),
            StandardView::parse("front_right_top")
        );
        assert_eq!(StandardView::parse("iso"), Some(StandardView::ISO));
        assert_eq!(StandardView::ISO.name(), "front_right_top");
        for bad in ["", "up", "front_back", "top_top", "front__top"] {
            assert_eq!(StandardView::parse(bad), None, "{bad}");
        }
        let (yaw, pitch) = StandardView::TOP.angles();
        assert_eq!((yaw, pitch), (-90.0, 90.0));
    }

    /// Invariant: view.rotate turns the scene about the screen axes by the
    /// right-hand rule. From the front, +90 about x shows the top (and
    /// stays a snapped face); +90 about z puts the world X axis up on the
    /// screen; four quarter turns, or a turn and its inverse, come back.
    #[test]
    fn rotate_about_screen_axes() {
        let mut c = Camera::default();
        c.set_standard(StandardView::FRONT);
        c.rotate(ScreenAxis::X, 90.0);
        assert!(close(c.to_eye(), DVec3::Z));
        assert!(c.is_orthographic());
        let (right, up) = c.basis();
        assert!(close(right, DVec3::X) && close(up, DVec3::Y));

        c.set_standard(StandardView::FRONT);
        c.rotate(ScreenAxis::Z, 90.0);
        let (right, up) = c.basis();
        assert!(
            close(right, DVec3::NEG_Z) && close(up, DVec3::X),
            "{right} {up}"
        );
        assert!((c.roll_deg - 90.0).abs() < 1e-9);
        assert!(c.is_orthographic(), "a roll keeps the face");

        c.set_standard(StandardView::ISO);
        let start = (c.to_eye(), c.basis());
        for axis in [ScreenAxis::X, ScreenAxis::Y, ScreenAxis::Z] {
            for _ in 0..4 {
                c.rotate(axis, 90.0);
            }
            c.rotate(axis, 37.0);
            c.rotate(axis, -37.0);
            assert!(close(c.to_eye(), start.0), "{axis:?}");
            assert!(close(c.basis().0, start.1.0) && close(c.basis().1, start.1.1));
        }
        c.rotate(ScreenAxis::Y, 30.0);
        assert!(!c.is_orthographic());
    }

    /// Invariant: the flight starts at `a`, ends exactly at `b`, keeps the
    /// screen axes orthonormal on the way, and snaps to orthographic only
    /// on arrival.
    #[test]
    fn flight_between_views() {
        let mut a = Camera::default();
        a.set_standard(StandardView::ISO);
        a.distance = 3.0;
        let mut b = a.clone();
        b.set_standard(StandardView::TOP);
        b.roll(90.0);
        b.distance = 5.0;
        let s = Camera::lerp(&a, &b, 0.0);
        assert!(close(s.to_eye(), a.to_eye()) && (s.distance - 3.0).abs() < 1e-9);
        assert_eq!(Camera::lerp(&a, &b, 1.0), b);
        let m = Camera::lerp(&a, &b, 0.5);
        let (r, u) = m.basis();
        assert!((r.length() - 1.0).abs() < 1e-9 && r.dot(u).abs() < 1e-9);
        assert!(r.dot(m.to_eye()).abs() < 1e-9);
        assert!(!m.is_orthographic());
        assert!(b.is_orthographic());
    }

    /// Invariant: after a fit every corner of the box projects inside the
    /// viewport, in both projections and for wide and tall viewports.
    #[test]
    fn fit_keeps_the_box_in_view() {
        for aspect in [0.5, 1.0, 2.0] {
            for proj in [Projection::Orthographic, Projection::Perspective] {
                let mut c = Camera {
                    projection: proj,
                    ..Camera::default()
                };
                c.fit(&unit_box(), aspect);
                let f = c.frame([0.5; 3], 0.87, aspect);
                for i in 0..8 {
                    let p = glam::Vec3::new(
                        (i & 1) as f32 - 0.5,
                        ((i >> 1) & 1) as f32 - 0.5,
                        ((i >> 2) & 1) as f32 - 0.5,
                    );
                    let q = f.view_proj.project_point3(p);
                    assert!(q.x.abs() < 1.0 && q.y.abs() < 1.0, "{proj:?} {aspect}: {q}");
                    assert!(q.z > 0.0 && q.z < 1.0);
                }
            }
        }
    }

    /// Invariant: a zoom towards the cursor keeps the point under the
    /// cursor fixed on screen.
    #[test]
    fn zoom_to_cursor_keeps_the_point() {
        let mut c = Camera::default();
        c.fit(&unit_box(), 1.5);
        let (right, up) = c.basis();
        let hh = c.half_height();
        let p = DVec3::from(c.target) + right * 0.4 * hh * 1.5 + up * -0.3 * hh;
        c.zoom(0.5, Some([0.4, -0.3]), 1.5);
        let f = c.frame([0.0; 3], 1.0, 1.5);
        let q = f.view_proj.project_point3(p.as_vec3());
        assert!((q.x - 0.4).abs() < 1e-4 && (q.y + 0.3).abs() < 1e-4, "{q}");
    }

    /// Invariant: dragging right moves the scene right (the target moves
    /// left), and the top view looks straight down with a valid up.
    #[test]
    fn pan_follows_the_pointer_and_top_is_stable() {
        let mut c = Camera::default();
        c.set_standard(StandardView::FRONT);
        c.pan(10.0, 0.0, 100.0);
        assert!(c.target[0] < 0.0);
        c.set_standard(StandardView::TOP);
        let (right, up) = c.basis();
        assert!(right.is_finite() && up.is_finite());
        assert!((c.to_eye() - DVec3::Z).length() < 1e-9);
    }
}
