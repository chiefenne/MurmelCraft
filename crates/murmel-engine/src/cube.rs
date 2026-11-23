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

    /// Invariant: the tinted face is the one head-on in a face view.
    #[test]
    fn front_face_is_the_view() {
        for v in StandardView::cube().into_iter().filter(|v| v.is_face()) {
            let mut c = Camera::default();
            c.set_standard(v);
            assert_eq!(front_face(&c), v);
        }
    }
}
