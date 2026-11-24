//! Cell quality (UI spec, Colour by: Quality): one value per cell for
//! each metric, computed once per open mesh, for colouring, the threshold
//! that hides good cells, and `inspect.quality`.
//!
//! Non-orthogonality and skewness follow Jasak ("Error analysis and
//! estimation for the finite volume method", PhD thesis, Imperial College
//! 1996, ch. 3) and Ferziger and Perić ("Computational methods for fluid
//! dynamics", sec. 8.6), per face as in [`crate::inspect::face_quality`];
//! a cell takes the worst of its faces, as the Properties panel shows.
//! The volume ratio is the largest ratio of the cell's volume to a
//! neighbour's (or back), a measure of how smoothly cell size changes,
//! which matters across prism layers.

use murmel_mesh::{Mesh, NO_CELL, par};
use serde::{Deserialize, Serialize};

use crate::inspect::{cell_centroid_in, cell_volume, face_centre_area, face_quality};

/// Faces measured together before their values go to the cells: enough
/// to keep every thread busy, few enough that the block stays small
/// (about 16 MiB).
const FACE_BLOCK: usize = 1 << 20;

/// What one distinct face gives its owner and neighbour.
struct FaceValues {
    owner: u32,
    neighbour: Option<u32>,
    non_orthogonality: f32,
    skewness: Option<f32>,
    volume_ratio: Option<f32>,
}

/// A per-cell quality metric. Larger is worse for each.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// Worst face non-orthogonality, degrees.
    #[default]
    NonOrthogonality,
    /// Worst face skewness (interior faces), dimensionless.
    Skewness,
    /// Largest volume ratio to a neighbour, at least 1.
    VolumeRatio,
    /// Number of faces.
    FaceCount,
}

impl Metric {
    pub const ALL: [Metric; 4] = [
        Metric::NonOrthogonality,
        Metric::Skewness,
        Metric::VolumeRatio,
        Metric::FaceCount,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Metric::NonOrthogonality => "non_orthogonality",
            Metric::Skewness => "skewness",
            Metric::VolumeRatio => "volume_ratio",
            Metric::FaceCount => "face_count",
        }
    }

    pub fn unit(self) -> Option<&'static str> {
        (self == Metric::NonOrthogonality).then_some("°")
    }
}

/// The quality values of every cell of a mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Quality {
    pub non_orthogonality: Vec<f32>,
    pub skewness: Vec<f32>,
    pub volume_ratio: Vec<f32>,
    pub face_count: Vec<f32>,
    /// Smallest and largest value of each metric, in `Metric::ALL` order
    /// (`None` without cells).
    ranges: [Option<(f64, f64)>; 4],
}

/// Summary of one metric over the cells, for the legend and
/// `inspect.quality`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Stats {
    pub metric: Metric,
    pub cells: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    /// Equal-width bins from `min` to `max` (one bin when they are equal).
    pub histogram: Vec<usize>,
}

impl Quality {
    /// Compute every metric: centroids and volumes once per cell, then
    /// each distinct face once. Cells and faces are split over the
    /// threads ([`murmel_mesh::par`]); each block of faces then gives its
    /// values to the cells in face order, so the result does not depend
    /// on the number of threads.
    pub fn compute(mesh: &Mesh) -> Quality {
        Self::compute_in_blocks(mesh, FACE_BLOCK)
    }

    /// [`Quality::compute`] with `block` faces measured together.
    fn compute_in_blocks(mesh: &Mesh, block: usize) -> Quality {
        let n = mesh.cell_count();
        let cells: Vec<([f64; 3], f64)> = par::collect(n, 1 << 12, |r| {
            let mut faces = Vec::new();
            r.map(|c| (cell_centroid_in(mesh, c, &mut faces), cell_volume(mesh, c)))
                .collect()
        });
        let mut q = Quality {
            non_orthogonality: vec![0.0; n],
            skewness: vec![0.0; n],
            volume_ratio: vec![1.0; n],
            face_count: par::collect(n, 1 << 16, |r| {
                r.map(|c| mesh.cell_face_range(c).len() as f32).collect()
            }),
            ranges: [None; 4],
        };
        let worst = |v: &mut Vec<f32>, c: u32, x: f32| {
            if let Some(slot) = v.get_mut(c as usize) {
                *slot = slot.max(x);
            }
        };
        let faces = mesh.unique_faces.min(mesh.face_owner.len());
        for start in (0..faces).step_by(block.max(1)) {
            let end = (start + block.max(1)).min(faces);
            let values = par::collect(end - start, 1 << 12, |r| {
                r.filter_map(|i| face_values(mesh, &cells, start + i))
                    .collect()
            });
            for f in values {
                worst(&mut q.non_orthogonality, f.owner, f.non_orthogonality);
                if let Some(c) = f.neighbour {
                    worst(&mut q.non_orthogonality, c, f.non_orthogonality);
                    if let Some(sk) = f.skewness {
                        worst(&mut q.skewness, f.owner, sk);
                        worst(&mut q.skewness, c, sk);
                    }
                    if let Some(r) = f.volume_ratio {
                        worst(&mut q.volume_ratio, f.owner, r);
                        worst(&mut q.volume_ratio, c, r);
                    }
                }
            }
        }
        for (i, m) in Metric::ALL.into_iter().enumerate() {
            let s = q.stats(m, 1);
            q.ranges[i] = (s.cells > 0).then_some((s.min, s.max));
        }
        q
    }

    /// Smallest and largest value of metric `m` (`None` without cells).
    pub fn range(&self, m: Metric) -> Option<(f64, f64)> {
        let i = Metric::ALL.iter().position(|&x| x == m)?;
        self.ranges[i]
    }

    pub fn values(&self, m: Metric) -> &[f32] {
        match m {
            Metric::NonOrthogonality => &self.non_orthogonality,
            Metric::Skewness => &self.skewness,
            Metric::VolumeRatio => &self.volume_ratio,
            Metric::FaceCount => &self.face_count,
        }
    }

    /// Statistics of metric `m` over the cells, with `bins` histogram bins.
    pub fn stats(&self, m: Metric, bins: usize) -> Stats {
        let v = self.values(m);
        let (mut min, mut max, mut sum) = (f64::INFINITY, f64::NEG_INFINITY, 0.0);
        for &x in v {
            let x = x as f64;
            min = min.min(x);
            max = max.max(x);
            sum += x;
        }
        if v.is_empty() {
            (min, max) = (0.0, 0.0);
        }
        let bins = if max > min { bins.max(1) } else { 1 };
        let mut histogram = vec![0; if v.is_empty() { 0 } else { bins }];
        for &x in v {
            let t = if max > min {
                (x as f64 - min) / (max - min)
            } else {
                0.0
            };
            let i = ((t * bins as f64) as usize).min(bins - 1);
            histogram[i] += 1;
        }
        Stats {
            metric: m,
            cells: v.len(),
            min,
            max,
            mean: if v.is_empty() {
                0.0
            } else {
                sum / v.len() as f64
            },
            histogram,
        }
    }
}

/// The values distinct face `u` gives its cells (`None` without a valid
/// owner), from the centroid and volume of each cell.
fn face_values(mesh: &Mesh, cells: &[([f64; 3], f64)], u: usize) -> Option<FaceValues> {
    let o = *mesh.face_owner.get(u)?;
    let (oc, ov) = *cells.get(o as usize).filter(|_| o != NO_CELL)?;
    let nb = mesh
        .face_neighbour
        .get(u)
        .copied()
        .filter(|&c| c != NO_CELL && (c as usize) < cells.len());
    let (fc, s) = face_centre_area(mesh, mesh.face_nodes(u));
    // The area vector as the owner sees it; face_quality takes the
    // absolute cosine, so the side does not matter.
    let (no, skew) = face_quality(fc, s, oc, nb.map(|c| cells[c as usize].0));
    let ratio = nb.and_then(|c| {
        let (a, b) = (ov, cells[c as usize].1);
        (a > 0.0 && b > 0.0).then(|| (a / b).max(b / a) as f32)
    });
    Some(FaceValues {
        owner: o,
        neighbour: nb,
        non_orthogonality: no as f32,
        skewness: skew.map(|x| x as f32),
        volume_ratio: ratio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn channel() -> Mesh {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        murmel_mesh::read(&p).unwrap()
    }

    /// Invariant: the per-cell values equal what the Properties panel
    /// shows for that cell (the worst face of the cell), so the colour
    /// and the panel never disagree. Breaks when the field skips faces
    /// seen from the neighbour side or the boundary; the inspect tests
    /// check only the panel.
    #[test]
    fn values_match_the_cell_properties() {
        let m = channel();
        let q = Quality::compute(&m);
        assert_eq!(q.non_orthogonality.len(), m.cell_count());
        for c in [0, 7, m.cell_count() / 2, m.cell_count() - 1] {
            let rows = crate::inspect::cell_quality(&m, c);
            assert!(
                (rows.0 - q.non_orthogonality[c] as f64).abs() < 1e-3,
                "cell {c}"
            );
            assert!((rows.1 - q.skewness[c] as f64).abs() < 1e-4, "cell {c}");
            assert_eq!(q.face_count[c] as usize, m.cell_face_range(c).len());
        }
    }

    /// Invariant: the values do not depend on how many threads compute
    /// them, nor on the block size (every face still reaches both of its
    /// cells). Breaks when faces at the edge of a thread's range or a
    /// block are dropped or counted for the wrong cell.
    #[test]
    fn threads_give_the_same_values() {
        let m = channel();
        let one = murmel_mesh::par::with_threads(1, || Quality::compute(&m));
        for threads in [2, 3, 8] {
            let q = murmel_mesh::par::with_threads(threads, || Quality::compute(&m));
            assert_eq!(q, one, "{threads} threads");
        }
        let honeycomb = murmel_mesh::synth::honeycomb(6);
        let one = murmel_mesh::par::with_threads(1, || Quality::compute(&honeycomb));
        let many = murmel_mesh::par::with_threads(5, || Quality::compute(&honeycomb));
        assert_eq!(one, many);
        let small_blocks =
            murmel_mesh::par::with_threads(3, || Quality::compute_in_blocks(&honeycomb, 7));
        assert_eq!(one, small_blocks);
        assert!(one.stats(Metric::NonOrthogonality, 1).max > 0.0);
    }

    /// Invariant: statistics cover every cell, the histogram sums to the
    /// cell count, the volume ratio is never below 1 and a mesh with
    /// prisms and polyhedra has more than one face count.
    #[test]
    fn stats_cover_the_cells() {
        let m = channel();
        let q = Quality::compute(&m);
        for metric in Metric::ALL {
            let s = q.stats(metric, 16);
            assert_eq!(s.cells, m.cell_count());
            assert_eq!(s.histogram.iter().sum::<usize>(), m.cell_count());
            assert!(s.min <= s.mean && s.mean <= s.max, "{metric:?}");
        }
        assert!(q.stats(Metric::VolumeRatio, 4).min >= 1.0);
        let f = q.stats(Metric::FaceCount, 4);
        assert!(f.max > f.min);
    }
}
