//! Picking (UI spec, Display: "Picking"): the cell or face under a screen
//! point. A ray from the camera through the point is tested against the
//! triangles the viewport draws (Möller and Trumbore, "Fast, minimum
//! storage ray-triangle intersection", J. Graphics Tools 2(1), 1997); the
//! nearest hit names the cell and the face that triangle shows, so a
//! pick always agrees with the picture, clip and hidden items included.
//!
//! The surface keeps a box around each run of triangles
//! ([`Surface::tri_boxes`]); the ray is tested against the boxes first
//! (the slab test: Kay and Kajiya, "Ray tracing complex scenes",
//! SIGGRAPH 1986) and against the triangles of the boxes it passes
//! through, nearer than the best hit so far. The hit is the one a walk
//! over every triangle finds, the first of equally near ones included.

use glam::DVec3;
use murmel_mesh::NO_CELL;
use serde::Serialize;

use crate::camera::Camera;
use crate::surface::{PICK_RUN, Surface};

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
    let tris = surface.indices.as_chunks::<3>().0;
    let mut best: Option<(f64, usize)> = None;
    let p = |i: u32| DVec3::from(surface.positions[i as usize].map(f64::from));
    let test = |best: &mut Option<(f64, usize)>, t: usize, tri: &[u32; 3]| {
        if let Some(d) = hit_triangle(start, dir, p(tri[0]), p(tri[1]), p(tri[2]))
            && best.is_none_or(|(bd, _)| d < bd)
        {
            *best = Some((d, t));
        }
    };
    if surface.tri_boxes.len() == tris.len().div_ceil(PICK_RUN) {
        // A box is widened by a hair, so rounding in the slab test never
        // drops a triangle on its border.
        let pad = 1e-6 * surface.radius.max(f64::MIN_POSITIVE);
        for (k, b) in surface.tri_boxes.iter().enumerate() {
            let Some(near) = hit_box(start, dir, b, pad) else {
                continue;
            };
            if best.is_some_and(|(bd, _)| near > bd) {
                continue;
            }
            let end = ((k + 1) * PICK_RUN).min(tris.len());
            for (t, tri) in tris.iter().enumerate().take(end).skip(k * PICK_RUN) {
                test(&mut best, t, tri);
            }
        }
    } else {
        for (t, tri) in tris.iter().enumerate() {
            test(&mut best, t, tri);
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

/// The distance along the ray (`dir` of unit length) at which it
/// crosses triangle `a b c` in front of `start` (Möller and Trumbore).
fn hit_triangle(start: DVec3, dir: DVec3, a: DVec3, b: DVec3, c: DVec3) -> Option<f64> {
    let (e1, e2) = (b - a, c - a);
    let h = dir.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-30 {
        return None;
    }
    let inv = 1.0 / det;
    let s = start - a;
    let u = s.dot(h) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let d = e2.dot(q) * inv;
    (d > 0.0).then_some(d)
}

/// Where the ray enters box `b` widened by `pad` (0 when it starts
/// inside), or `None` when it misses it or the box lies behind.
fn hit_box(start: DVec3, dir: DVec3, b: &[[f32; 3]; 2], pad: f64) -> Option<f64> {
    let (mut near, mut far) = (f64::NEG_INFINITY, f64::INFINITY);
    for k in 0..3 {
        let lo = f64::from(b[0][k]) - pad;
        let hi = f64::from(b[1][k]) + pad;
        if dir[k] == 0.0 {
            if start[k] < lo || start[k] > hi {
                return None;
            }
            continue;
        }
        let (t1, t2) = ((lo - start[k]) / dir[k], (hi - start[k]) / dir[k]);
        near = near.max(t1.min(t2));
        far = far.min(t1.max(t2));
    }
    (near <= far && far >= 0.0).then_some(near.max(0.0))
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

    /// Invariant: the boxes change nothing but the speed. Over a grid of
    /// screen points, several views and clip modes, the pick with the
    /// boxes equals the walk over every triangle (the same triangle, the
    /// first of equally near ones). Breaks when a box misses part of its
    /// run, the slab test drops a ray grazing a box, or a box is skipped
    /// while it still holds a nearer triangle.
    #[test]
    fn boxes_pick_what_the_full_walk_picks() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        let t = Tokens::builtin().unwrap();
        for mesh in [
            murmel_mesh::read(&p).unwrap(),
            murmel_mesh::synth::honeycomb(5),
        ] {
            let b = mesh.bounds();
            let clip = |mode| Display {
                clips: vec![crate::Clip {
                    origin: b.center(),
                    normal: [0.6, 0.0, 0.8],
                    mode,
                    enabled: true,
                }],
                ..Display::default()
            };
            for d in [
                Display::default(),
                clip(crate::ClipMode::Crinkle),
                clip(crate::ClipMode::Flat),
            ] {
                let s = Surface::build(&mesh, &t, &d);
                assert_eq!(s.tri_boxes.len(), s.triangles().div_ceil(PICK_RUN));
                let walk = Surface {
                    tri_boxes: Vec::new(),
                    ..s.clone()
                };
                for view in [StandardView::ISO, StandardView::TOP] {
                    let mut cam = Camera::default();
                    cam.set_standard(view);
                    cam.fit(&b, 1.3);
                    let mut hits = 0;
                    for i in 0..=24 {
                        for j in 0..=24 {
                            let ndc = [f64::from(i) / 12.0 - 1.0, f64::from(j) / 12.0 - 1.0];
                            let a = pick(&s, &cam, ndc, 1.3);
                            assert_eq!(a, pick(&walk, &cam, ndc, 1.3), "{view:?} {ndc:?}");
                            hits += usize::from(a.is_some());
                        }
                    }
                    assert!(hits > 50, "{view:?}: {hits} hits");
                }
            }
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
