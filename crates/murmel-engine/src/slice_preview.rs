//! The preview of a slice while it is dragged in the window (decided
//! 2026-11-24 on a card: "grid preview, exact slice follows"). Building
//! a slice through a large mesh takes a while (about 0.2 s a million
//! cells), so while the plane moves the renderer draws it at once from a
//! regular grid of the field held on the GPU as a 3D texture, and the
//! exact slice, built on a thread of its own once the plane rests, takes
//! its place ([`crate::Session::poll_surface`]). This is texture-based
//! slicing of a volume (B. Cabral, N. Cam and J. Foran, "Accelerated
//! volume rendering and tomographic reconstruction using texture mapping
//! hardware", 1994 Symposium on Volume Visualization; K. Engel et al.,
//! "Real-Time Volume Graphics", 2006, chapter 3), drawn on one polygon:
//! the plane inside the mesh's box.
//!
//! The grid has about [`VOXELS`] cells over the mesh's box. Each grid
//! cell shows the mesh cell whose centroid is nearest its centre among
//! the cells whose box holds that centre ([`Voxels::build`], made once
//! per mesh): every grid cell inside the mesh finds the cell it lies in
//! among them, and the grid cells outside every cell's box show nothing.
//! It keeps that cell's value (colour mode flat) and, for smooth and
//! banded, a value between the cell's and its nodes' ([`between`]),
//! mixed between grid cells on the GPU. Near a curved or concave boundary a
//! grid cell inside a cell's box but outside the mesh may show that
//! cell; the exact slice follows within a moment.

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use murmel_mesh::{Bounds, Mesh, NO_CELL};

use crate::field::FieldValues;

/// About how many grid cells the preview grid has (2 M: 16 MB on the
/// GPU).
pub const VOXELS: usize = 1 << 21;

/// A grid cell outside every mesh cell's box: drawn as nothing.
pub const OUTSIDE: f32 = 3.0e38;
/// A grid cell of a mesh cell without a value: drawn in the slice's
/// colour.
pub const NO_VALUE: f32 = -3.0e38;

/// The mesh cell each cell of a regular grid over the mesh's box shows.
#[derive(Debug)]
pub(crate) struct Voxels {
    lo: [f64; 3],
    side: [f64; 3],
    dims: [usize; 3],
    /// By grid cell, x fastest; [`NO_CELL`] outside every cell's box.
    cell: Vec<u32>,
}

impl Voxels {
    /// The grid of `mesh` with about `count` cells, its cells' nodes
    /// (each once) `nodes(c)` and their centroids `centre(c)`.
    pub(crate) fn build<'a>(
        mesh: &'a Mesh,
        count: usize,
        nodes: impl Fn(usize) -> &'a [u32] + Sync,
        centre: impl Fn(usize) -> [f64; 3] + Sync,
    ) -> Self {
        let b = mesh.bounds();
        let lo = b.min;
        let ext = [0, 1, 2].map(|k| (b.max[k] - b.min[k]).max(0.0));
        let big = ext
            .iter()
            .copied()
            .fold(0.0, f64::max)
            .max(f64::MIN_POSITIVE);
        let ext = ext.map(|e| e.max(1e-6 * big));
        let side = (ext[0] * ext[1] * ext[2] / count.max(1) as f64).cbrt();
        let dims = ext.map(|e| ((e / side).ceil() as usize).clamp(1, 1024));
        let side = [0, 1, 2].map(|k| ext[k] / dims[k] as f64);
        let n = dims[0] * dims[1] * dims[2];
        // The nearest centroid so far, by grid cell: its squared distance
        // (an f32's bits, which order as the distances do) above the cell.
        let best: Vec<AtomicU64> = (0..n).map(|_| AtomicU64::new(u64::MAX)).collect();
        let cells = mesh.cell_count();
        murmel_mesh::par::map(cells, 1 << 12, |r| {
            for c in r {
                let (mut a, mut z) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                for &i in nodes(c) {
                    if let Some(p) = mesh.points.get(i as usize) {
                        for k in 0..3 {
                            a[k] = a[k].min(p[k]);
                            z[k] = z[k].max(p[k]);
                        }
                    }
                }
                let x = centre(c);
                if !(0..3).all(|k| a[k] <= z[k] && x[k].is_finite()) {
                    continue;
                }
                // The grid cells whose centre lies in the cell's box.
                let span = |k: usize| {
                    let from = ((a[k] - lo[k]) / side[k] - 0.5).ceil().max(0.0) as usize;
                    let to = ((z[k] - lo[k]) / side[k] - 0.5).floor();
                    if to < 0.0 {
                        return 0..0;
                    }
                    from..(to as usize).min(dims[k] - 1) + 1
                };
                let (sx, sy, sz) = (span(0), span(1), span(2));
                for iz in sz {
                    let pz = lo[2] + (iz as f64 + 0.5) * side[2] - x[2];
                    for iy in sy.clone() {
                        let py = lo[1] + (iy as f64 + 0.5) * side[1] - x[1];
                        let row = (iz * dims[1] + iy) * dims[0];
                        for ix in sx.clone() {
                            let px = lo[0] + (ix as f64 + 0.5) * side[0] - x[0];
                            let d2 = (px * px + py * py + pz * pz) as f32;
                            let key = (u64::from(d2.to_bits()) << 32) | c as u64;
                            best[row + ix].fetch_min(key, Ordering::Relaxed);
                        }
                    }
                }
            }
        });
        let cell = murmel_mesh::par::collect(n, 1 << 14, |r| {
            r.map(|i| {
                let k = best[i].load(Ordering::Relaxed);
                if k == u64::MAX { NO_CELL } else { k as u32 }
            })
            .collect()
        });
        Voxels {
            lo,
            side,
            dims,
            cell,
        }
    }

    /// The centre of grid cell `i`.
    fn centre(&self, i: usize) -> [f64; 3] {
        let d = self.dims;
        let at = [i % d[0], (i / d[0]) % d[1], i / (d[0] * d[1])];
        [0, 1, 2].map(|k| self.lo[k] + (at[k] as f64 + 0.5) * self.side[k])
    }
}

/// The preview grid of one field, as the renderer uploads it: by grid
/// cell (x fastest), the cell value and the value smooth colours show
/// at the grid cell's centre, both relative to the surface's value
/// origin ([`OUTSIDE`], [`NO_VALUE`]).
#[derive(Clone, Debug, PartialEq)]
pub struct SliceGrid {
    /// Names this grid's values (a renderer uploads each once).
    pub id: u64,
    pub dims: [u32; 3],
    /// Where grid cell (0, 0, 0) starts and the size of a grid cell,
    /// relative to the surface's origin.
    pub lo: [f32; 3],
    pub side: [f32; 3],
    pub texels: Vec<[f32; 2]>,
    /// The surface origin and value origin it was made for.
    origin: [f64; 3],
    value_origin: f64,
}

impl SliceGrid {
    /// The grid of `values` on `voxels` of `mesh`, its cells' nodes
    /// (each once) `nodes(c)` and centroids `centre(c)`, relative to
    /// `origin` and the value origin `value_origin`.
    pub(crate) fn of<'a>(
        voxels: &Voxels,
        values: &FieldValues,
        mesh: &'a Mesh,
        nodes: impl Fn(usize) -> &'a [u32] + Sync,
        centre: impl Fn(usize) -> [f64; 3] + Sync,
        origin: [f64; 3],
        value_origin: f64,
    ) -> SliceGrid {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let texels = murmel_mesh::par::collect(voxels.cell.len(), 1 << 14, |r| {
            r.map(|i| {
                let c = voxels.cell[i];
                if c == NO_CELL {
                    return [OUTSIDE; 2];
                }
                let v = values.flat(c, NO_CELL);
                if !v.is_finite() {
                    return [NO_VALUE; 2];
                }
                let smooth = between(
                    mesh,
                    values,
                    nodes(c as usize),
                    centre(c as usize),
                    v,
                    voxels.centre(i),
                );
                [(v - value_origin) as f32, (smooth - value_origin) as f32]
            })
            .collect()
        });
        SliceGrid {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            dims: voxels.dims.map(|d| d as u32),
            lo: [0, 1, 2].map(|k| (voxels.lo[k] - origin[k]) as f32),
            side: voxels.side.map(|s| s as f32),
            texels,
            origin,
            value_origin,
        }
    }

    /// Whether it was made for the origins of `s`.
    pub(crate) fn fits(&self, s: &crate::surface::Surface) -> bool {
        self.origin == s.origin && self.value_origin == s.value_origin
    }
}

/// The value at `p` inside a cell (value `v`, centroid `x`, nodes
/// `nodes`) as colour modes smooth and banded show it: between the
/// cell's value at its centroid and its nodes' values, each weighted by
/// the inverse square of its distance (D. Shepard, "A two-dimensional
/// interpolation function for irregularly-spaced data", 1968), so it runs
/// from the cell's value in its middle to the nodes' at its corners as
/// the exact slice does. A node without a value is left out.
fn between(
    mesh: &Mesh,
    values: &FieldValues,
    nodes: &[u32],
    x: [f64; 3],
    v: f64,
    p: [f64; 3],
) -> f64 {
    let d2 = |q: [f64; 3]| (0..3).map(|k| (q[k] - p[k]).powi(2)).sum::<f64>();
    let dx = d2(x);
    if dx.is_nan() || dx <= 0.0 {
        return v;
    }
    let (mut sw, mut sv) = (1.0 / dx, v / dx);
    for &n in nodes {
        let (Some(&q), Some(&vn)) = (mesh.points.get(n as usize), values.node.get(n as usize))
        else {
            continue;
        };
        if !vn.is_finite() {
            continue;
        }
        let d = d2(q);
        if d.is_nan() || d <= 0.0 {
            return vn;
        }
        sw += 1.0 / d;
        sv += vn / d;
    }
    sv / sw
}

/// What the renderer draws for the slices moving in the window: their
/// planes inside the mesh's box, coloured from the grid, and which of
/// the surface's triangles and outline vertices to leave out meanwhile
/// (the moving slices where they were built, and arrows on slices).
#[derive(Clone, Debug, PartialEq)]
pub struct SliceDraw {
    /// Each plane as a convex polygon relative to the surface's origin,
    /// with its unit normal.
    pub polygons: Vec<(Vec<[f32; 3]>, [f32; 3])>,
    /// The field's grid; `None` when the slices are not coloured by a
    /// field, or have no cells to show it on.
    pub grid: Option<Arc<SliceGrid>>,
    /// The linear colour of a plane without a value.
    pub color: [f32; 3],
    /// How much of the selection colour a selected slice mixes in.
    pub selection_mix: f32,
    /// Entries of the surface's `indices` and vertices of its
    /// `edge_positions` not to draw, in order.
    pub hide_faces: Vec<Range<u32>>,
    pub hide_edges: Vec<Range<u32>>,
    /// The clip planes that cut the slices (`[n, d]` relative to the
    /// surface's origin: a point `p` is cut away where `n · p + d > 0`).
    pub clips: Vec<[f32; 4]>,
}

/// The polygon where the plane through `origin` with normal `normal`
/// crosses the box `b`, in order around it; empty when it misses.
pub fn plane_in_box(origin: [f64; 3], normal: [f64; 3], b: &Bounds) -> Vec<[f64; 3]> {
    let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if !(len > 0.0 && len.is_finite()) {
        return Vec::new();
    }
    let n = normal.map(|x| x / len);
    let d = |p: [f64; 3]| (0..3).map(|k| n[k] * (p[k] - origin[k])).sum::<f64>();
    let corner = |i: usize| [0, 1, 2].map(|k| if i >> k & 1 == 1 { b.max[k] } else { b.min[k] });
    let mut pts: Vec<[f64; 3]> = Vec::new();
    for i in 0..8 {
        for k in 0..3 {
            let j = i | (1 << k);
            if j == i {
                continue;
            }
            let (p, q) = (corner(i), corner(j));
            let (dp, dq) = (d(p), d(q));
            if (dp <= 0.0) != (dq <= 0.0) {
                let t = dp / (dp - dq);
                pts.push([0, 1, 2].map(|m| p[m] + t * (q[m] - p[m])));
            }
        }
    }
    if pts.len() < 3 {
        return Vec::new();
    }
    // In order of their angle about their mean, in the plane.
    let m = pts
        .iter()
        .fold([0.0; 3], |a, p| [a[0] + p[0], a[1] + p[1], a[2] + p[2]])
        .map(|x| x / pts.len() as f64);
    let helper = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let u = cross(n, helper);
    let v = cross(n, u);
    let angle = |p: &[f64; 3]| {
        let r = [p[0] - m[0], p[1] - m[1], p[2] - m[2]];
        let (x, y) = (
            (0..3).map(|k| r[k] * u[k]).sum::<f64>(),
            (0..3).map(|k| r[k] * v[k]).sum::<f64>(),
        );
        y.atan2(x)
    };
    pts.sort_by(|a, b| angle(a).total_cmp(&angle(b)));
    pts
}

/// The ranges of `0..len` left once `hide` (sorted, not overlapping) is
/// taken out.
pub fn shown_ranges(len: u32, hide: &[Range<u32>]) -> Vec<Range<u32>> {
    let mut out = Vec::new();
    let mut at = 0;
    for h in hide {
        let (a, z) = (h.start.min(len), h.end.min(len));
        if a > at {
            out.push(at..a);
        }
        at = at.max(z);
    }
    if at < len {
        out.push(at..len);
    }
    out
}

/// `ranges` sorted with the empty ones left out and those that touch
/// merged.
pub(crate) fn merged(mut ranges: Vec<Range<u32>>) -> Vec<Range<u32>> {
    ranges.retain(|r| !r.is_empty());
    ranges.sort_by_key(|r| r.start);
    let mut out: Vec<Range<u32>> = Vec::new();
    for r in ranges {
        match out.last_mut() {
            Some(l) if r.start <= l.end => l.end = l.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the plane through the middle of a box, tilted, meets it
    /// in a convex polygon whose corners lie on the plane and on the box,
    /// in order around it (no two edges cross); a plane beside the box
    /// gives none. Breaks when the corners come out of order (the
    /// preview would draw a bow tie).
    #[test]
    fn a_plane_crosses_a_box_in_a_convex_polygon() {
        let b = Bounds {
            min: [0.0, 0.0, 0.0],
            max: [4.0, 2.0, 1.0],
        };
        let n = [0.3, 0.5, 0.8];
        let o = [2.0, 1.0, 0.5];
        let p = plane_in_box(o, n, &b);
        assert!(p.len() >= 3 && p.len() <= 6, "{p:?}");
        let len = (0.09f64 + 0.25 + 0.64).sqrt();
        for q in &p {
            let d: f64 = (0..3).map(|k| n[k] / len * (q[k] - o[k])).sum();
            assert!(d.abs() < 1e-9);
            assert!((0..3).all(|k| q[k] >= -1e-9 && q[k] <= b.max[k] + 1e-9));
        }
        // Convex and in order: each turn the same way about the normal.
        let k = p.len();
        let turns: Vec<f64> = (0..k)
            .map(|i| {
                let (a, b2, c) = (p[i], p[(i + 1) % k], p[(i + 2) % k]);
                let e = [b2[0] - a[0], b2[1] - a[1], b2[2] - a[2]];
                let f = [c[0] - b2[0], c[1] - b2[1], c[2] - b2[2]];
                let t = [
                    e[1] * f[2] - e[2] * f[1],
                    e[2] * f[0] - e[0] * f[2],
                    e[0] * f[1] - e[1] * f[0],
                ];
                (0..3).map(|m| t[m] * n[m]).sum()
            })
            .collect();
        assert!(
            turns.iter().all(|&s| s > 0.0) || turns.iter().all(|&s| s < 0.0),
            "{turns:?}"
        );
        assert!(plane_in_box([9.0, 0.0, 0.0], [1.0, 0.0, 0.0], &b).is_empty());
    }

    /// Invariant: every grid cell inside a mesh of hexahedra shows the
    /// cell it lies in, and the ranges left between hidden ones cover
    /// the rest exactly. Breaks when the span of grid cells in a cell's
    /// box is off by one (a stripe of grid cells with nothing, or with
    /// the neighbour's value).
    #[test]
    fn grid_cells_show_the_cell_they_lie_in() {
        let mesh = murmel_mesh::synth::honeycomb(6);
        let set = crate::field::FieldSet::default();
        let vox = Voxels::build(
            &mesh,
            4000,
            |c| set.nodes_of_cell(&mesh, c),
            |c| set.centroid(&mesh, c),
        );
        assert!(vox.cell.len() > 1000);
        let inside = vox.cell.iter().filter(|&&c| c != NO_CELL).count();
        assert!(
            inside * 10 >= vox.cell.len() * 9,
            "{inside} of {}",
            vox.cell.len()
        );
        for (i, &c) in vox.cell.iter().enumerate() {
            if c == NO_CELL {
                continue;
            }
            // The cell's box holds the grid cell's centre.
            let p = vox.centre(i);
            let nodes = set.nodes_of_cell(&mesh, c as usize);
            for k in 0..3 {
                let lo = nodes
                    .iter()
                    .map(|&j| mesh.points[j as usize][k])
                    .fold(f64::INFINITY, f64::min);
                let hi = nodes
                    .iter()
                    .map(|&j| mesh.points[j as usize][k])
                    .fold(f64::NEG_INFINITY, f64::max);
                assert!(p[k] >= lo - 1e-9 && p[k] <= hi + 1e-9);
            }
        }
        assert_eq!(shown_ranges(10, &[2..4, 4..5, 8..12]), vec![0..2, 5..8]);
        assert_eq!(merged(vec![5..6, 0..2, 2..3, 7..7]), vec![0..3, 5..6]);
    }
}
