//! The clip plane (UI spec, Display: "Clip plane"). One plane cuts the
//! mesh; the side its normal points to is cut away, the other side stays.
//!
//! - **Crinkle** keeps whole cells: every cell with a node on the kept side
//!   stays, so the cut shows the cells themselves (the prism layers at the
//!   walls, the polyhedral core). This is the "crinkle clip" of
//!   visualisation tools, a cell selection by a half-space.
//! - **Flat** slices: the faces are clipped to the half-space
//!   (Sutherland and Hodgman, "Reentrant polygon clipping", CACM 17(1),
//!   1974) and every cell the plane crosses adds its section polygon, so
//!   the cut face shows the cell outlines in the plane.
//!
//! A node exactly on the plane counts as kept. A point where an edge
//! crosses the plane is named by the edge (its two node ids), so the cuts
//! of neighbouring faces and cells meet at the same named points and the
//! section of a cell closes, even when nodes lie on the plane (symbolic
//! perturbation: such a node behaves as if a little inside).

use murmel_mesh::{Bounds, Mesh};
use serde::{Deserialize, Serialize};

/// How the plane cuts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipMode {
    /// Whole cells (the default: what a CFD engineer needs to see the
    /// layers).
    #[default]
    Crinkle,
    /// Cells sliced by the plane.
    Flat,
}

impl ClipMode {
    pub const ALL: [ClipMode; 2] = [ClipMode::Crinkle, ClipMode::Flat];

    pub fn key(self) -> &'static str {
        match self {
            ClipMode::Crinkle => "crinkle",
            ClipMode::Flat => "flat",
        }
    }
}

/// The clip plane.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    /// A point of the plane.
    pub origin: [f64; 3],
    /// Unit normal, towards the part that is cut away.
    pub normal: [f64; 3],
    pub mode: ClipMode,
    /// Off keeps the plane but draws the whole mesh (the tree's eye).
    pub enabled: bool,
}

impl Clip {
    /// Signed distance of `p` from the plane (positive: cut away).
    pub fn distance(&self, p: [f64; 3]) -> f64 {
        (0..3)
            .map(|k| (p[k] - self.origin[k]) * self.normal[k])
            .sum()
    }

    /// Whether point `p` is on the kept side.
    pub fn keeps(&self, p: [f64; 3]) -> bool {
        self.distance(p) <= 0.0
    }

    /// The offset of the plane along its normal from the centre of
    /// `bounds`, and the range of offsets over which the plane crosses
    /// the box (the slider's ends).
    pub fn offset(&self, bounds: &Bounds) -> (f64, [f64; 2]) {
        let c = bounds.center();
        let off = -self.distance(c);
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for i in 0..8 {
            let p = [0, 1, 2].map(|k| {
                if i >> k & 1 == 1 {
                    bounds.max[k]
                } else {
                    bounds.min[k]
                }
            });
            let d: f64 = (0..3).map(|k| (p[k] - c[k]) * self.normal[k]).sum();
            lo = lo.min(d);
            hi = hi.max(d);
        }
        if bounds.is_empty() {
            (off, [0.0, 0.0])
        } else {
            (off, [lo, hi])
        }
    }

    /// Put the plane at `offset` along its normal from the centre of
    /// `bounds`.
    pub fn set_offset(&mut self, bounds: &Bounds, offset: f64) {
        let c = bounds.center();
        self.origin = [0, 1, 2].map(|k| c[k] + offset * self.normal[k]);
    }

    /// The corners of the plane inside `bounds` (the polygon where the
    /// plane cuts the box), in order around the plane; empty when it
    /// misses the box. For the gizmo of the window.
    pub fn box_section(&self, bounds: &Bounds) -> Vec<[f64; 3]> {
        if bounds.is_empty() {
            return Vec::new();
        }
        let corner = |i: usize| {
            [0, 1, 2].map(|k| {
                if i >> k & 1 == 1 {
                    bounds.max[k]
                } else {
                    bounds.min[k]
                }
            })
        };
        let mut pts = Vec::new();
        for a in 0..8usize {
            for k in 0..3 {
                let b = a | (1 << k);
                if b == a {
                    continue;
                }
                let (pa, pb) = (corner(a), corner(b));
                let (da, db) = (self.distance(pa), self.distance(pb));
                if (da <= 0.0) != (db <= 0.0) {
                    let t = da / (da - db);
                    pts.push([0, 1, 2].map(|j| pa[j] + t * (pb[j] - pa[j])));
                }
            }
        }
        if pts.len() < 3 {
            return Vec::new();
        }
        let n = pts.len() as f64;
        let c = [0, 1, 2].map(|k| pts.iter().map(|p| p[k]).sum::<f64>() / n);
        let (u, v) = plane_axes(self.normal);
        let angle = |p: &[f64; 3]| {
            let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
            let x: f64 = (0..3).map(|k| d[k] * u[k]).sum();
            let y: f64 = (0..3).map(|k| d[k] * v[k]).sum();
            y.atan2(x)
        };
        pts.sort_by(|a, b| angle(a).total_cmp(&angle(b)));
        pts
    }
}

/// Two unit vectors that span the plane with normal `n`.
pub fn plane_axes(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let a = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let u = normalize(cross(n, a));
    let v = cross(n, u);
    (u, v)
}

pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `v` scaled to unit length; `None` for a zero or non-finite vector.
pub fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (l.is_finite() && l > 1e-300).then(|| v.map(|x| x / l))
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    unit(v).unwrap_or([0.0, 0.0, 1.0])
}

/// Which cells a clip keeps: a cell stays when one of its nodes is on the
/// kept side (crinkle and flat alike; flat then slices the cells it
/// crosses).
pub fn kept_cells(mesh: &Mesh, keeps_point: &[bool]) -> Vec<bool> {
    (0..mesh.cell_count())
        .map(|c| {
            mesh.cell(c)
                .any(|f| f.iter().any(|&i| keeps_point[i as usize]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the plane through the centre of a unit box normal to X
    /// cuts it in a square of four corners, the offset range is the half
    /// width either way, and moving the plane by an offset moves its
    /// origin along the normal. The window's slider and gizmo rely on
    /// these.
    #[test]
    fn box_section_and_offsets() {
        let b = Bounds {
            min: [0.0; 3],
            max: [1.0, 2.0, 3.0],
        };
        let mut c = Clip {
            origin: b.center(),
            normal: [1.0, 0.0, 0.0],
            mode: ClipMode::Crinkle,
            enabled: true,
        };
        assert_eq!(c.box_section(&b).len(), 4);
        let (off, range) = c.offset(&b);
        assert!(off.abs() < 1e-12);
        assert_eq!(range, [-0.5, 0.5]);
        c.set_offset(&b, 0.25);
        assert!((c.origin[0] - 0.75).abs() < 1e-12);
        assert!(c.keeps([0.7, 0.0, 0.0]) && !c.keeps([0.8, 0.0, 0.0]));
        c.set_offset(&b, 2.0);
        assert!(c.box_section(&b).is_empty());
    }
}
