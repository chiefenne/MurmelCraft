//! The cells of a mesh in bins of a regular grid, so a slice or an
//! iso-surface tests only the cells near it instead of every cell
//! (fields on large meshes; docs/performance.md, "Slices and
//! iso-surfaces test only the cells near them").
//!
//! Each cell goes to the bin of the mean of its nodes, about
//! [`PER_BIN`] cells to a bin. A bin keeps the sphere about its centre
//! that holds every node of its cells, so a plane farther from the
//! centre than that crosses none of them; and, for each field, the
//! smallest and largest value the iso-surface test of its cells can
//! meet, so an iso value outside them crosses none. The cells of the
//! bins left are tested as before, in cell order, so what is drawn is
//! the same, bit for bit, as when every cell is tested.

use std::sync::{Arc, OnceLock};

use murmel_mesh::Mesh;

#[cfg(test)]
thread_local! {
    static OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether slices and iso-surfaces use the bins (tests compare them with
/// testing every cell).
pub(crate) fn on() -> bool {
    #[cfg(test)]
    {
        !OFF.with(std::cell::Cell::get)
    }
    #[cfg(not(test))]
    {
        true
    }
}

/// Run `f` with every cell tested, the bins left out (tests).
#[cfg(test)]
pub(crate) fn without<R>(f: impl FnOnce() -> R) -> R {
    let before = OFF.with(|o| o.replace(true));
    let r = f();
    OFF.with(|o| o.set(before));
    r
}

/// About how many cells share a bin.
pub(crate) const PER_BIN: usize = 8;

/// The cells of a mesh by bin.
#[derive(Debug)]
pub(crate) struct CellBins {
    /// Where bin (0, 0, 0) starts, the size of a bin and the bins along
    /// each axis.
    lo: [f64; 3],
    side: [f64; 3],
    dims: [usize; 3],
    /// The cells of bin `b` are `cells[start[b]..start[b + 1]]`, in
    /// cell order.
    start: Vec<u32>,
    cells: Vec<u32>,
    /// The radius about each bin's centre that holds every node of its
    /// cells (NaN for an empty bin).
    reach: Vec<f64>,
}

impl CellBins {
    /// The bins of the cells of `mesh`, whose nodes (each once) are
    /// `nodes(c)`.
    pub(crate) fn build<'a>(mesh: &'a Mesh, nodes: impl Fn(usize) -> &'a [u32] + Sync) -> Self {
        let n = mesh.cell_count();
        let np = mesh.points.len();
        let b = mesh.bounds();
        let lo = b.min;
        let ext = [0, 1, 2].map(|k| (b.max[k] - b.min[k]).max(0.0));
        let big = ext
            .iter()
            .copied()
            .fold(0.0, f64::max)
            .max(f64::MIN_POSITIVE);
        let ext = ext.map(|e| e.max(1e-9 * big));
        let target = (n / PER_BIN).max(1) as f64;
        let side = (ext[0] * ext[1] * ext[2] / target).cbrt();
        let dims = ext.map(|e| ((e / side).ceil() as usize).clamp(1, 512));
        let side = [0, 1, 2].map(|k| ext[k] / dims[k] as f64);
        let bin_of = |p: [f64; 3]| -> usize {
            let i = [0, 1, 2].map(|k| {
                let x = ((p[k] - lo[k]) / side[k]).floor();
                if x.is_finite() {
                    (x.max(0.0) as usize).min(dims[k] - 1)
                } else {
                    0
                }
            });
            (i[2] * dims[1] + i[1]) * dims[0] + i[0]
        };
        // The bin of each cell: the mean of its nodes.
        let bin: Vec<u32> = murmel_mesh::par::collect(n, 1 << 14, |r| {
            r.map(|c| {
                let (mut k, mut m) = (0.0, [0.0; 3]);
                for &i in nodes(c) {
                    if let Some(p) = mesh.points.get(i as usize) {
                        m = [m[0] + p[0], m[1] + p[1], m[2] + p[2]];
                        k += 1.0;
                    }
                }
                if k > 0.0 {
                    bin_of(m.map(|x| x / k)) as u32
                } else {
                    0
                }
            })
            .collect()
        });
        let count = dims[0] * dims[1] * dims[2];
        let mut start = vec![0u32; count + 1];
        for &b in &bin {
            start[b as usize + 1] += 1;
        }
        for i in 0..count {
            start[i + 1] += start[i];
        }
        let mut cursor = start.clone();
        let mut cells = vec![0u32; n];
        for (c, &b) in bin.iter().enumerate() {
            let at = &mut cursor[b as usize];
            cells[*at as usize] = c as u32;
            *at += 1;
        }
        let centre = |b: usize| {
            let i = [
                b % dims[0],
                (b / dims[0]) % dims[1],
                b / (dims[0] * dims[1]),
            ];
            [0, 1, 2].map(|k| lo[k] + (i[k] as f64 + 0.5) * side[k])
        };
        let reach = murmel_mesh::par::collect(count, 1 << 10, |r| {
            r.map(|b| {
                let o = centre(b);
                let mut d2 = f64::NAN;
                for &c in &cells[start[b] as usize..start[b + 1] as usize] {
                    for &i in nodes(c as usize) {
                        if (i as usize) < np {
                            let p = mesh.points[i as usize];
                            let x = (p[0] - o[0]).powi(2)
                                + (p[1] - o[1]).powi(2)
                                + (p[2] - o[2]).powi(2);
                            d2 = if d2.is_nan() { x } else { d2.max(x) };
                        }
                    }
                }
                d2.sqrt()
            })
            .collect()
        });
        CellBins {
            lo,
            side,
            dims,
            start,
            cells,
            reach,
        }
    }

    fn bins(&self) -> usize {
        self.reach.len()
    }

    fn centre(&self, b: usize) -> [f64; 3] {
        let d = self.dims;
        let i = [b % d[0], (b / d[0]) % d[1], b / (d[0] * d[1])];
        [0, 1, 2].map(|k| self.lo[k] + (i[k] as f64 + 0.5) * self.side[k])
    }

    fn of(&self, b: usize) -> &[u32] {
        &self.cells[self.start[b] as usize..self.start[b + 1] as usize]
    }

    /// The cells, in cell order, of the bins `keep` keeps.
    fn cells_where(&self, keep: impl Fn(usize) -> bool) -> Vec<u32> {
        let mut out: Vec<u32> = (0..self.bins())
            .filter(|&b| keep(b))
            .flat_map(|b| self.of(b).iter().copied())
            .collect();
        out.sort_unstable();
        out
    }

    /// The cells, in cell order, that may have nodes on both sides of the
    /// plane whose signed distance is `distance`: those of the bins whose
    /// sphere the plane meets.
    pub(crate) fn near_plane(&self, distance: impl Fn([f64; 3]) -> f64) -> Vec<u32> {
        self.cells_where(|b| {
            let r = self.reach[b];
            // NaN: an empty bin. A little room for rounding.
            r.is_finite() && distance(self.centre(b)).abs() <= r * (1.0 + 1e-9) + 1e-300
        })
    }

    /// The smallest and largest of `bounds(c)` (`None`: a cell no level
    /// crosses) over the cells of each bin, for [`CellBins::near_value`].
    pub(crate) fn spans(&self, bounds: impl Fn(usize) -> Option<(f64, f64)> + Sync) -> Spans {
        Spans(murmel_mesh::par::collect(self.bins(), 1 << 10, |r| {
            r.map(|b| {
                let mut span: Option<(f64, f64)> = None;
                for &c in self.of(b) {
                    if let Some((lo, hi)) = bounds(c as usize) {
                        span = Some(span.map_or((lo, hi), |(a, z)| (a.min(lo), z.max(hi))));
                    }
                }
                span.map_or([f64::NAN; 2], |(a, z)| [a, z])
            })
            .collect()
        }))
    }

    /// The cells, in cell order, of the bins whose span (`spans`, made by
    /// [`CellBins::spans`]) holds `value`.
    pub(crate) fn near_value(&self, spans: &Spans, value: f64) -> Vec<u32> {
        self.cells_where(|b| {
            spans
                .0
                .get(b)
                .is_some_and(|s| s[0] <= value && value <= s[1])
        })
    }
}

/// The smallest and largest value each bin's cells can meet (NaN: none).
#[derive(Debug)]
pub(crate) struct Spans(Vec<[f64; 2]>);

/// Something made once and kept, which never makes two values differ
/// (it is made from them): the bins' spans of a field's values.
pub(crate) struct Kept<T>(OnceLock<Arc<T>>);

impl<T> Default for Kept<T> {
    fn default() -> Self {
        Kept(OnceLock::new())
    }
}

impl<T> Clone for Kept<T> {
    fn clone(&self) -> Self {
        Kept(self.0.clone())
    }
}

impl<T> Kept<T> {
    pub(crate) fn get_or_init(&self, make: impl FnOnce() -> T) -> &T {
        self.0.get_or_init(|| Arc::new(make()))
    }
}

impl<T> PartialEq for Kept<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T> std::fmt::Debug for Kept<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.get().is_some() {
            "Kept(made)"
        } else {
            "Kept(not made)"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: every cell is in exactly one bin, each bin's cells in
    /// cell order, and every node of a cell lies within its bin's
    /// sphere; a plane through the mesh keeps every cell it crosses
    /// (nodes on both sides) and leaves most others out. Breaks when the
    /// counting sort drops or repeats a cell, when the reach misses a
    /// node (a slice would lose the faces of that cell), or when the
    /// test keeps everything (no gain). The slice and iso-surface tests
    /// compare whole surfaces, not the candidates.
    #[test]
    fn bins_hold_every_cell_and_keep_the_crossed_ones() {
        let mesh = murmel_mesh::synth::honeycomb(20);
        let nodes: Vec<Vec<u32>> = (0..mesh.cell_count())
            .map(|c| {
                let mut v: Vec<u32> = mesh.cell_points(c).collect();
                v.sort_unstable();
                v.dedup();
                v
            })
            .collect();
        let bins = CellBins::build(&mesh, |c| &nodes[c]);
        let mut all: Vec<u32> = bins.cells.clone();
        all.sort_unstable();
        assert_eq!(all, (0..mesh.cell_count() as u32).collect::<Vec<_>>());
        for b in 0..bins.bins() {
            assert!(bins.of(b).is_sorted());
            let o = bins.centre(b);
            for &c in bins.of(b) {
                for &i in &nodes[c as usize] {
                    let p = mesh.points[i as usize];
                    let d = (0..3).map(|k| (p[k] - o[k]).powi(2)).sum::<f64>().sqrt();
                    assert!(d <= bins.reach[b] * (1.0 + 1e-12));
                }
            }
        }
        let mid = mesh.bounds().center();
        let plane = |p: [f64; 3]| (p[0] - mid[0]) * 0.6 + (p[1] - mid[1]) * 0.8;
        let near = bins.near_plane(plane);
        assert!(near.is_sorted());
        let crossed: Vec<u32> = (0..mesh.cell_count() as u32)
            .filter(|&c| {
                let d: Vec<f64> = nodes[c as usize]
                    .iter()
                    .map(|&i| plane(mesh.points[i as usize]))
                    .collect();
                d.iter().any(|&x| x <= 0.0) && d.iter().any(|&x| x > 0.0)
            })
            .collect();
        assert!(!crossed.is_empty());
        assert!(crossed.iter().all(|c| near.binary_search(c).is_ok()));
        assert!(
            near.len() < mesh.cell_count() / 4,
            "{} of {}",
            near.len(),
            mesh.cell_count()
        );
    }
}
