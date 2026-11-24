//! Picking (UI spec, Display: "Picking"): the cell or face under a screen
//! point. A ray from the camera through the point is tested against the
//! triangles the viewport draws (Möller and Trumbore, "Fast, minimum
//! storage ray-triangle intersection", J. Graphics Tools 2(1), 1997); the
//! nearest hit names the cell and the face that triangle shows, so a
//! pick always agrees with the picture, clip and hidden items included.
//!
//! The test walks every triangle once (no acceleration structure yet),
//! which is fine for a click; the hover tooltip asks only when the pointer
//! rests.

use glam::DVec3;
use murmel_mesh::NO_CELL;
use serde::Serialize;

use crate::camera::Camera;
use crate::surface::Surface;

/// What lies under a screen point.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Hit {
    /// The cell the triangle shows ([`NO_CELL`] when unknown).
    pub cell: u32,
    /// The distinct face ([`NO_CELL`] on a flat cut's section).
    pub face: u32,
    /// The point hit, in world coordinates.
    pub point: [f64; 3],
    /// Distance along the ray from its start.
    pub distance: f64,
}

/// The nearest triangle of `surface` under `ndc` (-1..1, y up) seen
/// through `camera` in a viewport of `aspect`.
pub fn pick(surface: &Surface, camera: &Camera, ndc: [f64; 2], aspect: f64) -> Option<Hit> {
    let (start, dir) = camera.ray(ndc, aspect);
    let o = DVec3::from(surface.origin);
    // Relative to the surface origin, and from far enough back that an
    // orthographic ray starts in front of everything drawn.
    let back = 4.0 * (surface.radius + (start - o).length());
    let start = start
        - o
        - if camera.is_orthographic() {
            dir * back
        } else {
            DVec3::ZERO
        };
    let mut best: Option<(f64, usize)> = None;
    let p = |i: u32| DVec3::from(surface.positions[i as usize].map(f64::from));
    for (t, tri) in surface.indices.as_chunks::<3>().0.iter().enumerate() {
        let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
        let (e1, e2) = (b - a, c - a);
        let h = dir.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1e-30 {
            continue;
        }
        let inv = 1.0 / det;
        let s = start - a;
        let u = s.dot(h) * inv;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = dir.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let d = e2.dot(q) * inv;
        if d > 0.0 && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, t));
        }
    }
    let (d, t) = best?;
    let point = start + dir * d + o;
    Some(Hit {
        cell: surface.tri_cell.get(t).copied().unwrap_or(NO_CELL),
        face: surface.tri_face.get(t).copied().unwrap_or(NO_CELL),
        point: point.to_array(),
        distance: d,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::StandardView;
    use crate::display::Display;
    use crate::theme::Tokens;

    /// Invariant: looking at the channel from the front, the centre of
    /// the view hits the front face (the nearest), on the boundary, at
    /// the y of the box's front side; the point lies on the ray; a point
    /// outside the mesh hits nothing. In both projections.
    #[test]
    fn picks_the_nearest_face() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        let mesh = murmel_mesh::read(&p).unwrap();
        let s = Surface::build(&mesh, &Tokens::builtin().unwrap(), &Display::default());
        let b = s.bounds.unwrap();
        for proj in [
            crate::Projection::Perspective,
            crate::Projection::Orthographic,
        ] {
            let mut cam = Camera {
                projection: proj,
                ..Camera::default()
            };
            cam.set_standard(StandardView::FRONT);
            cam.fit(&b, 1.5);
            let hit = pick(&s, &cam, [0.0, 0.0], 1.5).unwrap();
            assert!((hit.point[1] - b.min[1]).abs() < 1e-6, "{proj:?} {hit:?}");
            assert!((hit.cell as usize) < mesh.cell_count());
            let u = hit.face as usize;
            assert_eq!(mesh.face_neighbour[u], NO_CELL, "a boundary face");
            assert!(pick(&s, &cam, [0.99, 0.99], 1.5).is_none());
        }
    }

    /// Invariant: every triangle drawn can be picked. With a crinkle clip
    /// towards the camera, the ray through the projected centre of each
    /// triangle that faces the eye hits a triangle no farther than that
    /// centre. A ray built differently from the drawing projection, or a
    /// triangle the picker skips, misses the faces the picture shows.
    #[test]
    fn every_drawn_triangle_can_be_picked() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        let mesh = murmel_mesh::read(&p).unwrap();
        let t = Tokens::builtin().unwrap();
        let mut cam = Camera::default();
        cam.set_standard(StandardView::ISO);
        let b = mesh.bounds();
        let aspect = 811.0 / 800.0;
        cam.fit(&b, aspect);
        let eye_dir = cam.to_eye();
        let d = Display {
            clips: vec![crate::Clip {
                origin: b.center(),
                normal: [eye_dir.x.signum(), 0.0, 0.0],
                mode: crate::ClipMode::Crinkle,
                enabled: true,
            }],
            ..Display::default()
        };
        let s = Surface::build(&mesh, &t, &d);
        let mut missed = Vec::new();
        for (i, tri) in s.indices.as_chunks::<3>().0.iter().enumerate() {
            let q: [DVec3; 3] =
                std::array::from_fn(|j| DVec3::from(s.positions[tri[j] as usize].map(f64::from)));
            let n = (q[1] - q[0]).cross(q[2] - q[0]);
            let c = (q[0] + q[1] + q[2]) / 3.0 + DVec3::from(s.origin);
            let eye = cam.ray([0.0, 0.0], aspect).0;
            if n.length() < 1e-12 || n.dot(eye - c).abs() < 1e-3 * n.length() * (eye - c).length() {
                continue;
            }
            let Some(ndc) = cam.project(s.origin, s.radius, aspect, c.to_array()) else {
                continue;
            };
            let hit = pick(&s, &cam, ndc, aspect);
            let far = (c - eye).length();
            if hit.is_none_or(|h| (DVec3::from(h.point) - eye).length() > far + 1e-6 * s.radius) {
                missed.push((i, ndc, s.tri_cell[i], s.tri_face[i]));
            }
        }
        assert!(
            missed.is_empty(),
            "{} missed, e.g. {:?}",
            missed.len(),
            &missed[..missed.len().min(4)]
        );
    }
}
