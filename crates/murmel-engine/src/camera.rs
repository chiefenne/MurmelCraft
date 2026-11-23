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

/// The standard views (the six faces of the view cube and home).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardView {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    Iso,
    Home,
}

impl StandardView {
    /// Yaw and pitch in degrees. Front looks along +Y, right along -X,
    /// top down -Z; iso sees front, right and top.
    pub fn angles(self) -> (f64, f64) {
        let iso_pitch = (1.0f64 / 2.0f64.sqrt()).atan().to_degrees();
        match self {
            StandardView::Front => (-90.0, 0.0),
            StandardView::Back => (90.0, 0.0),
            StandardView::Right => (0.0, 0.0),
            StandardView::Left => (180.0, 0.0),
            StandardView::Top => (-90.0, 90.0),
            StandardView::Bottom => (-90.0, -90.0),
            StandardView::Iso | StandardView::Home => (-45.0, iso_pitch),
        }
    }

    /// Face views snap to orthographic in `PerspectiveOrthoFaces`.
    pub fn is_face(self) -> bool {
        !matches!(self, StandardView::Iso | StandardView::Home)
    }
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
}

impl Default for Camera {
    fn default() -> Self {
        let (yaw, pitch) = StandardView::Iso.angles();
        Self {
            target: [0.0; 3],
            distance: 5.0,
            yaw_deg: yaw,
            pitch_deg: pitch,
            projection: Projection::default(),
            fov_deg: 30.0,
            snapped: false,
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

    /// Screen right and up as world vectors.
    pub fn basis(&self) -> (DVec3, DVec3) {
        let y = self.yaw_deg.to_radians();
        let right = DVec3::new(-y.sin(), y.cos(), 0.0);
        let up = self.to_eye().cross(right).normalize_or(DVec3::Z);
        (right, up)
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
        self.snapped = view.is_face();
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
        c.set_standard(StandardView::Front);
        assert!(c.is_orthographic());
        c.orbit(5.0, 0.0);
        assert!(!c.is_orthographic());
        c.set_standard(StandardView::Iso);
        assert!(!c.is_orthographic());
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
        c.set_standard(StandardView::Front);
        c.pan(10.0, 0.0, 100.0);
        assert!(c.target[0] < 0.0);
        c.set_standard(StandardView::Top);
        let (right, up) = c.basis();
        assert!(right.is_finite() && up.is_finite());
        assert!((c.to_eye() - DVec3::Z).length() < 1e-9);
    }
}
