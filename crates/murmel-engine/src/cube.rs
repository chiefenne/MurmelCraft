//! The view cube (UI spec, "Viewport, navigation and view cube"): a cube
//! seen from the camera's direction, each face split into a 3 × 3 grid
//! whose centre is the face view, whose border strips are the edge views
//! and whose corner squares are the corner views, so a click picks one of
//! the 26 standard views.
//!
//! The geometry lives here, without a window, so the window draws and
//! hit-tests the same patches that the tests check. Coordinates are those
//! of a cube with half size 1 about the origin, projected orthographically
//! onto the screen axes of the camera (x right, y up).

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::camera::{Camera, StandardView};

/// Width of the edge and corner strips, as a share of the half size.
pub const BORDER: f64 = 0.28;

/// One visible patch of a face of the cube.
#[derive(Clone, Debug, PartialEq)]
pub struct Patch {
    /// The view a click on the patch flies to.
    pub view: StandardView,
    /// The face the patch lies on (its outward normal).
    pub face: StandardView,
    /// Screen corners (x right, y up), counter-clockwise.
    pub corners: [[f64; 2]; 4],
    /// Cosine between the face normal and the direction to the eye
    /// (0 edge-on, 1 head-on).
    pub facing: f64,
}

/// A visible face, for its label.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub face: StandardView,
    pub center: [f64; 2],
    /// Screen direction of the label's baseline (unit length when the face
    /// is head-on, shorter when foreshortened).
    pub right: [f64; 2],
    /// Screen direction of the label's up.
    pub up: [f64; 2],
    pub facing: f64,
}

/// The six faces: outward normal, label right and label up, as world
/// vectors. Labels read upright in the home view and on the side faces.
fn faces() -> [(StandardView, DVec3, DVec3); 6] {
    [
        (StandardView::FRONT, DVec3::X, DVec3::Z),
        (StandardView::RIGHT, DVec3::Y, DVec3::Z),
        (StandardView::BACK, DVec3::NEG_X, DVec3::Z),
        (StandardView::LEFT, DVec3::NEG_Y, DVec3::Z),
        (StandardView::TOP, DVec3::X, DVec3::Y),
        (StandardView::BOTTOM, DVec3::X, DVec3::NEG_Y),
    ]
}

fn project(c: &Camera, p: DVec3) -> [f64; 2] {
    let (right, up) = c.basis();
    [p.dot(right), p.dot(up)]
}

/// Faces turned towards the eye by at least this cosine are drawn.
const MIN_FACING: f64 = 1e-3;

/// The patches of the faces turned towards the eye, back to front.
pub fn patches(c: &Camera) -> Vec<Patch> {
    let to_eye = c.to_eye();
    let mut out = Vec::new();
    for (face, u, v) in faces() {
        let n = face.direction();
        let facing = n.dot(to_eye);
        if facing < MIN_FACING {
            continue;
        }
        let StandardView::Cube(fd) = face else {
            continue;
        };
        let span = |i: i8| match i {
            -1 => (-1.0, -1.0 + BORDER),
            0 => (-1.0 + BORDER, 1.0 - BORDER),
            _ => (1.0 - BORDER, 1.0),
        };
        for i in [-1i8, 0, 1] {
            for j in [-1i8, 0, 1] {
                let (u0, u1) = span(i);
                let (v0, v1) = span(j);
                let at = |a: f64, b: f64| project(c, n + u * a + v * b);
                let mut d = fd;
                for (axis, k) in [(u, i), (v, j)] {
                    for (m, comp) in [axis.x, axis.y, axis.z].into_iter().enumerate() {
                        if comp != 0.0 {
                            d[m] = (comp as i8) * k;
                        }
                    }
                }
                out.push(Patch {
                    view: StandardView::Cube(d),
                    face,
                    corners: [at(u0, v0), at(u1, v0), at(u1, v1), at(u0, v1)],
                    facing,
                });
            }
        }
    }
    out.sort_by(|a, b| a.facing.total_cmp(&b.facing));
    out
}

/// The labels of the faces turned towards the eye.
pub fn labels(c: &Camera) -> Vec<Face> {
    let to_eye = c.to_eye();
    faces()
        .into_iter()
        .filter_map(|(face, u, v)| {
            let n = face.direction();
            let facing = n.dot(to_eye);
            (facing >= MIN_FACING).then(|| Face {
                face,
                center: project(c, n),
                right: project(c, u),
                up: project(c, v),
                facing,
            })
        })
        .collect()
}

/// Whether a label fits in the centre patch of face `f` with `gap` to
/// spare on every side, so it never touches the grid lines around it.
/// The label is a box of half size `half` (along and across its
/// baseline, cube units), drawn upright on the screen (not
/// foreshortened) with its baseline along `f.right` about `f.center`.
pub fn label_fits(f: &Face, half: [f64; 2], gap: f64) -> bool {
    let (rx, ry) = (f.right[0], f.right[1]);
    let (ux, uy) = (f.up[0], f.up[1]);
    let len = rx.hypot(ry);
    let det = rx * uy - ry * ux;
    if len < 1e-9 || det.abs() < 1e-9 {
        return false;
    }
    // Baseline and its normal on the screen, unit length.
    let (bx, by) = (rx / len, ry / len);
    let (nx, ny) = (-by, bx);
    let limit = 1.0 - BORDER;
    let (w, h) = (half[0] + gap, half[1] + gap);
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .into_iter()
        .all(|(sa, sb)| {
            let dx = bx * w * sa + nx * h * sb;
            let dy = by * w * sa + ny * h * sb;
            // The corner in face coordinates: d = right·a + up·b.
            let a = (dx * uy - dy * ux) / det;
            let b = (rx * dy - ry * dx) / det;
            a.abs() <= limit && b.abs() <= limit
        })
}

/// Whether `p` lies in the convex quad `q` (either winding), edges
/// included.
fn inside(q: &[[f64; 2]; 4], p: [f64; 2]) -> bool {
    let mut sign = 0.0f64;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if cross.abs() < 1e-9 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// The view under screen point `p` (cube coordinates, y up), if the
/// point is on the cube. Patches more head-on win on shared borders.
pub fn hit(c: &Camera, p: [f64; 2]) -> Option<StandardView> {
    patches(c)
        .into_iter()
        .rev()
        .find(|pa| inside(&pa.corners, p))
        .map(|pa| pa.view)
}

/// The face most turned towards the eye: the one the window tints.
pub fn front_face(c: &Camera) -> StandardView {
    let to_eye = c.to_eye();
    faces()
        .into_iter()
        .map(|(f, _, _)| f)
        .max_by(|a, b| {
            a.direction()
                .dot(to_eye)
                .total_cmp(&b.direction().dot(to_eye))
        })
        .unwrap_or(StandardView::FRONT)
}

// ---- placement ----------------------------------------------------------

/// Where the window shows the view cube (decided 2026-11-23: any corner of
/// the viewport, a floating panel the user drags anywhere over it, or
/// hidden).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    TopRight,
    TopLeft,
    BottomLeft,
    BottomRight,
    Floating,
    Hidden,
}

impl Placement {
    pub const ALL: [Placement; 6] = [
        Placement::TopRight,
        Placement::TopLeft,
        Placement::BottomLeft,
        Placement::BottomRight,
        Placement::Floating,
        Placement::Hidden,
    ];

    /// The name in tool inputs, settings and translation keys.
    pub fn key(self) -> &'static str {
        match self {
            Placement::TopRight => "top_right",
            Placement::TopLeft => "top_left",
            Placement::BottomLeft => "bottom_left",
            Placement::BottomRight => "bottom_right",
            Placement::Floating => "floating",
            Placement::Hidden => "hidden",
        }
    }
}

/// Room between the viewport's sides and a cube in a corner, in pixels.
pub const MARGIN: [f64; 2] = [16.0, 12.0];
/// Pixels over which an auto-hidden cube fades out beyond its distance.
pub const FADE: f64 = 40.0;
/// The auto-hide distance unless the user sets one, in pixels.
pub const DEFAULT_DISTANCE: f64 = 120.0;
/// The largest auto-hide distance a setting may give, in pixels.
pub const MAX_DISTANCE: f64 = 2000.0;

/// The view cube's placement settings, kept in `settings.toml` and set by
/// `settings.set_view_cube`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CubeSettings {
    pub placement: Placement,
    /// Hide the cube while the pointer is farther from it than
    /// `distance_px`.
    pub auto_hide: bool,
    pub distance_px: f64,
    /// Top left of the floating panel, from the viewport's top left, in
    /// pixels; `None` starts it where the top-right cube is.
    pub position: Option<[f64; 2]>,
}

impl Default for CubeSettings {
    fn default() -> Self {
        Self {
            placement: Placement::default(),
            auto_hide: false,
            distance_px: DEFAULT_DISTANCE,
            position: None,
        }
    }
}

impl CubeSettings {
    /// Top left of the cube's block (`block` wide and high) in a viewport
    /// of `viewport` pixels, from the viewport's top left; `None` when the
    /// cube is hidden. A floating block is kept inside the viewport, so a
    /// smaller window never loses it.
    pub fn origin(&self, viewport: [f64; 2], block: [f64; 2]) -> Option<[f64; 2]> {
        let right = viewport[0] - MARGIN[0] - block[0];
        let bottom = viewport[1] - MARGIN[1] - block[1];
        let at = match self.placement {
            Placement::Hidden => return None,
            Placement::TopRight => [right, MARGIN[1]],
            Placement::TopLeft => [MARGIN[0], MARGIN[1]],
            Placement::BottomLeft => [MARGIN[0], bottom],
            Placement::BottomRight => [right, bottom],
            Placement::Floating => {
                let p = self.position.unwrap_or([right, MARGIN[1]]);
                let clamp = |v: f64, hi: f64| v.min(hi).max(0.0);
                [
                    clamp(p[0], viewport[0] - block[0]),
                    clamp(p[1], viewport[1] - block[1]),
                ]
            }
        };
        Some(at)
    }

    /// How opaque the cube is drawn, 0 to 1, for a pointer `distance`
    /// pixels from its block (`None`: the pointer is not over the
    /// window). Without auto-hide, or while the cube is dragged, it is
    /// always 1; with it, 1 up to the distance, then fading to 0 over
    /// `FADE` pixels.
    pub fn opacity(&self, distance: Option<f64>, dragging: bool) -> f64 {
        if !self.auto_hide || dragging {
            return 1.0;
        }
        match distance {
            None => 0.0,
            Some(d) => (1.0 - (d - self.distance_px) / FADE).clamp(0.0, 1.0),
        }
    }
}

/// Distance in pixels from point `p` to the rectangle `min`..`max`
/// (0 inside).
pub fn distance_to_rect(p: [f64; 2], min: [f64; 2], max: [f64; 2]) -> f64 {
    let dx = (min[0] - p[0]).max(p[0] - max[0]).max(0.0);
    let dy = (min[1] - p[1]).max(p[1] - max[1]).max(0.0);
    dx.hypot(dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: looking straight at any of the 26 views, the centre of
    /// the cube hits that view, so a click on a patch and the view it flies
    /// to always agree. A wrong edge or corner mapping fails here.
    #[test]
    fn the_centre_hits_the_view_looked_along() {
        for v in StandardView::cube() {
            let mut c = Camera::default();
            c.set_standard(v);
            assert_eq!(hit(&c, [0.0, 0.0]), Some(v), "{}", v.name());
            c.roll(90.0);
            assert_eq!(hit(&c, [0.0, 0.0]), Some(v), "{} rolled", v.name());
        }
    }

    /// Invariant: from any direction the visible patches cover the cube's
    /// silhouette, so every point on the cube picks a view, and points off
    /// it pick none. In the iso view all three visible faces and their
    /// labels show, and the top face's label reads with world +X to the
    /// right.
    #[test]
    fn patches_cover_the_cube() {
        let mut c = Camera::default();
        c.set_standard(StandardView::ISO);
        c.orbit(13.0, -7.0);
        let mut hits = 0;
        for i in -20..=20 {
            for j in -20..=20 {
                let p = [i as f64 * 0.09, j as f64 * 0.09];
                if hit(&c, p).is_some() {
                    hits += 1;
                }
            }
        }
        assert!(hits > 600, "{hits}");
        assert_eq!(hit(&c, [5.0, 0.0]), None);
        c.set_standard(StandardView::ISO);
        let l = labels(&c);
        assert_eq!(l.len(), 3);
        assert_eq!(patches(&c).len(), 27);
        let top = l.iter().find(|f| f.face == StandardView::TOP).unwrap();
        assert!(top.right[0] > 0.0);
        let seen = [StandardView::FRONT, StandardView::RIGHT, StandardView::TOP];
        assert!(seen.contains(&front_face(&c)));
    }

    /// Invariant: a label that fits keeps `gap` from every grid line of
    /// the centre patch, head-on and foreshortened. Breaks when the fit
    /// lets labels overflow the patch (the window allowed 15 % before,
    /// and "Front" touched the lines); the cube geometry tests above do
    /// not look at labels.
    #[test]
    fn labels_keep_a_gap_to_the_lines() {
        let mut c = Camera::default();
        c.set_standard(StandardView::FRONT);
        let front = labels(&c)
            .into_iter()
            .find(|f| f.face == StandardView::FRONT)
            .unwrap();
        let inner = 1.0 - BORDER;
        assert!(label_fits(&front, [0.5, 0.2], 0.1));
        // As wide as the patch: touches the lines, so it does not fit.
        assert!(!label_fits(&front, [inner, 0.2], 0.0001));
        assert!(!label_fits(&front, [inner - 0.05, 0.2], 0.1));
        assert!(!label_fits(&front, [0.3, inner - 0.05], 0.1));
        // Foreshortened faces leave less room than head-on.
        c.set_standard(StandardView::ISO);
        for f in labels(&c) {
            assert!(!label_fits(&f, [0.6, 0.2], 0.1), "{}", f.face.name());
            assert!(label_fits(&f, [0.3, 0.12], 0.05), "{}", f.face.name());
        }
    }

    /// Invariant: the tinted face is the one head-on in a face view.
    #[test]
    fn front_face_is_the_view() {
        for v in StandardView::cube().into_iter().filter(|v| v.is_face()) {
            let mut c = Camera::default();
            c.set_standard(v);
            assert_eq!(front_face(&c), v);
        }
    }

    /// Invariant: each corner places the block inside the viewport at the
    /// margins, hidden places none, and a floating block saved outside a
    /// now smaller viewport comes back inside. Breaks when a placement
    /// puts the cube off screen (a restored floating position from a
    /// larger window); the window tests do not check positions.
    #[test]
    fn placements_stay_in_the_viewport() {
        let vp = [800.0, 600.0];
        let block = [196.0, 192.0];
        let mut s = CubeSettings::default();
        assert_eq!(s.origin(vp, block), Some([588.0, 12.0]));
        s.placement = Placement::BottomLeft;
        assert_eq!(s.origin(vp, block), Some([16.0, 396.0]));
        s.placement = Placement::Hidden;
        assert_eq!(s.origin(vp, block), None);
        s.placement = Placement::Floating;
        assert_eq!(s.origin(vp, block), Some([588.0, 12.0]));
        s.position = Some([1500.0, -40.0]);
        assert_eq!(s.origin(vp, block), Some([604.0, 0.0]));
        for p in Placement::ALL {
            s.placement = p;
            if let Some(o) = s.origin(vp, block) {
                assert!(o[0] >= 0.0 && o[0] + block[0] <= vp[0], "{}", p.key());
                assert!(o[1] >= 0.0 && o[1] + block[1] <= vp[1], "{}", p.key());
            }
        }
    }

    /// Invariant: auto-hide shows the cube fully within the distance,
    /// fades it over FADE pixels beyond, hides it with the pointer outside
    /// the window, and never hides it while it is dragged or with
    /// auto-hide off. Breaks when the cube vanishes under a drag that
    /// leaves the distance, or stays when the pointer leaves the window.
    #[test]
    fn auto_hide_fades_by_distance() {
        let mut s = CubeSettings::default();
        assert_eq!(s.opacity(None, false), 1.0);
        s.auto_hide = true;
        s.distance_px = 100.0;
        assert_eq!(s.opacity(Some(0.0), false), 1.0);
        assert_eq!(s.opacity(Some(100.0), false), 1.0);
        assert!((s.opacity(Some(100.0 + FADE / 2.0), false) - 0.5).abs() < 1e-9);
        assert_eq!(s.opacity(Some(100.0 + FADE), false), 0.0);
        assert_eq!(s.opacity(None, false), 0.0);
        assert_eq!(s.opacity(Some(900.0), true), 1.0);
        assert_eq!(distance_to_rect([5.0, 5.0], [0.0, 0.0], [10.0, 10.0]), 0.0);
        assert_eq!(
            distance_to_rect([13.0, 14.0], [0.0, 0.0], [10.0, 10.0]),
            5.0
        );
    }

    /// Invariant: the settings file form round-trips and an empty table
    /// gives the defaults (an old settings.toml without the cube loads).
    #[test]
    fn cube_settings_round_trip() {
        let s = CubeSettings {
            placement: Placement::Floating,
            auto_hide: true,
            distance_px: 80.0,
            position: Some([10.0, 20.0]),
        };
        let t = toml::to_string(&s).unwrap();
        assert_eq!(toml::from_str::<CubeSettings>(&t).unwrap(), s);
        assert_eq!(
            toml::from_str::<CubeSettings>("").unwrap(),
            CubeSettings::default()
        );
        assert!(toml::from_str::<CubeSettings>("placement = \"middle\"").is_err());
    }
}
