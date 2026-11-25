//! Probes on an overview (large-mesh post-processing; the maintainer's
//! card 8 of 2026-11-24, "block cache for streamlines and probes"): a
//! probe whose point the cells shown do not hold (an overview shows none;
//! the blocks read for the regions hold some) reads the block of the file
//! that holds it, and shows the values, cell and quality the whole mesh
//! shows there.
//!
//! - **Which block.** The blocks whose bounds hold the point, those kept
//!   first, then the one the point lies deepest inside first. A cell lies
//!   in the box of its block's owned points, so the block owning the
//!   point's cell is among them; on the honeycombs of 1 M and 4 M cells a
//!   point lies in one to four boxes, two at the median
//!   (docs/performance.md). Each is read with its halo
//!   ([`crate::blocks::read`]) and the point looked for among its owned
//!   cells ([`crate::measure::locate_where`]); the first block holding it
//!   is its block, and a point in none is outside the volume.
//! - **The values** are read in the block as on the whole mesh
//!   ([`FieldSet::probe_values`]): the halo holds every cell sharing a
//!   node with an owned cell, so the cell's value and gradient of a file
//!   field are the whole mesh's. The derived and calculated fields of the
//!   mesh shown are made again on the block, in order; next to the halo
//!   their gradient reads halo cells whose own neighbours are partly
//!   missing, so they can differ a little there, as on the blocks read for
//!   regions. A probe snapped to a wall of the overview adds the wall's
//!   boundary values ([`values`]).
//! - **Kept.** The last [`KEEP`] blocks read stay (fewer when they would
//!   take more than a quarter of the memory budget), and each probe's
//!   finding stays with its point (`Display::probe_found`), so drawing
//!   again reads nothing; a probe moved, or a field made, is found again.
//! - **In the window** a probe's blocks are read on a thread of their own,
//!   one probe at a time, and its label says so meanwhile.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

use murmel_mesh::field::Location;
use murmel_mesh::profile::{self, Progress};
use murmel_mesh::vtkhdf::{Overview, Subset};
use murmel_mesh::{Evidence, Mesh, MeshError, NO_CELL};

use crate::derive::Made;
use crate::field::{FieldSet, FieldValue};
use crate::inspect::Row;
use crate::measure::Anchor;
use crate::session::OpenMesh;

/// The blocks read for probes that stay, at most.
pub const KEEP: usize = 4;

/// A block read for probes.
#[derive(Debug)]
pub struct Kept {
    pub block: usize,
    pub mesh: Arc<Mesh>,
    /// The file's fields of the block.
    pub base: Arc<FieldSet>,
    /// `base` with the fields of `made` made on it.
    pub fields: Arc<FieldSet>,
    pub made: Vec<Made>,
    pub subset: Arc<Subset>,
}

impl Kept {
    /// This block with the fields of `made` made on it (itself when they
    /// are the ones made).
    pub(crate) fn with_made(self: &Arc<Kept>, made: &[Made]) -> Arc<Kept> {
        if self.made == made {
            return self.clone();
        }
        let mut fields = (*self.base).clone();
        for m in made {
            if let Ok((set, _)) = fields.remake(&self.mesh, m.clone()) {
                fields = set;
            }
        }
        Arc::new(Kept {
            block: self.block,
            mesh: self.mesh.clone(),
            base: self.base.clone(),
            fields: Arc::new(fields),
            made: made.to_vec(),
            subset: self.subset.clone(),
        })
    }
}

/// A probe's point as found in the blocks of the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub point: [f64; 3],
    /// The fields made on the mesh shown when it was found.
    pub made: Vec<Made>,
    /// The memory budget it was looked for within (bytes).
    pub budget: u64,
    /// Its cell, `None` outside the volume or when its block could not
    /// be read.
    pub cell: Option<FoundCell>,
    /// Why a block holding it could not be read.
    pub problem: Option<MeshError>,
}

/// The cell holding a probe's point, in a block read for it.
#[derive(Clone, Debug, PartialEq)]
pub struct FoundCell {
    pub block: usize,
    /// The cell's id in the file.
    pub file_cell: u64,
    /// What the Properties panel shows of the cell
    /// ([`crate::inspect::probe_rows`], from its type on).
    pub rows: Vec<Row>,
    /// The cell and node fields at the point, read as
    /// [`FieldSet::probe_values`] reads them.
    pub values: Vec<FieldValue>,
}

/// The probes' points found so far, kept in the display.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Findings(pub Vec<Found>);

impl Findings {
    /// The finding of `point`.
    pub fn get(&self, point: [f64; 3]) -> Option<&Found> {
        self.0.iter().find(|f| same(f.point, point))
    }
}

/// Whether two points are the same, bit for bit.
fn same(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|k| a[k].to_bits() == b[k].to_bits())
}

/// The values a probe at `at` shows when its point lies in `cell` of a
/// block: the cell's and, for a probe snapped to a wall of the overview
/// shown, the wall's boundary values (not the copies of the volume's
/// cell fields the overview colours its surface with).
pub fn values(open: &OpenMesh, cell: &FoundCell, at: &Anchor) -> Vec<FieldValue> {
    let mut v = cell.values.clone();
    let (Some(o), None, Some(u)) = (open.overview.as_deref(), &open.blocks, at.face) else {
        return v;
    };
    let Some(&b) = open.mesh.face_boundary.get(u as usize) else {
        return v;
    };
    if b == NO_CELL {
        return v;
    }
    let copied = |name: &str| o.fields.iter().any(|f| f.on_surface && f.name == name);
    v.extend(
        open.fields
            .face_values(&open.mesh, b as usize)
            .into_iter()
            .filter(|f| f.location == Location::Boundary && !copied(&f.name)),
    );
    v
}

/// What finding a point needs of the open file.
#[derive(Clone, Debug)]
pub struct Source {
    pub path: PathBuf,
    /// The file, read block by block ([`murmel_mesh::vtkhdf::BlockFile`]).
    pub file: Arc<murmel_mesh::vtkhdf::BlockFile>,
    pub overview: Arc<Overview>,
    pub evidence: Option<Evidence>,
    /// The fields made on the mesh shown, in order.
    pub made: Vec<Made>,
    pub budget: u64,
}

/// How many blocks read for probes stay: [`KEEP`], fewer when they would
/// take more than a quarter of `budget` (one at least).
fn keep_for(o: &Overview, budget: u64) -> usize {
    let largest = (0..o.blocks.len())
        .map(|k| crate::blocks::bytes_for(o, &[k]))
        .max()
        .unwrap_or(0)
        .max(1);
    ((budget / 4 / largest) as usize).clamp(1, KEEP)
}

/// The blocks whose bounds hold `p`, those in `kept` first, then the one
/// `p` lies deepest inside first (its distance to the box's nearest face,
/// over the box's size along that axis).
pub(crate) fn candidates(o: &Overview, p: [f64; 3], kept: &[usize]) -> Vec<usize> {
    let span = o
        .blocks
        .iter()
        .filter(|b| !b.bounds.is_empty())
        .flat_map(|b| (0..3).map(move |k| b.bounds.max[k] - b.bounds.min[k]))
        .fold(0.0f64, f64::max);
    let tol = 1e-9 * span.max(f64::MIN_POSITIVE);
    let mut c: Vec<(bool, f64, usize)> = o
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.bounds.is_empty())
        .filter_map(|(k, b)| {
            let (lo, hi) = (b.bounds.min, b.bounds.max);
            if (0..3).any(|a| p[a] < lo[a] - tol || p[a] > hi[a] + tol) {
                return None;
            }
            let depth = (0..3)
                .map(|a| {
                    let size = (hi[a] - lo[a]).max(f64::MIN_POSITIVE);
                    (p[a] - lo[a]).min(hi[a] - p[a]) / size
                })
                .fold(f64::INFINITY, f64::min);
            let is_kept = kept.contains(&k);
            Some((!is_kept, -depth, k))
        })
        .collect();
    c.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    c.into_iter().map(|(_, _, k)| k).collect()
}

/// Find `point` in the blocks of `src`: read the blocks that may hold it
/// (those in `kept` are not read again) until one of their owned cells
/// holds it. The finding, and the blocks read or made again for it.
pub fn find(
    src: &Source,
    point: [f64; 3],
    kept: &[Arc<Kept>],
) -> Result<(Found, Vec<Arc<Kept>>), MeshError> {
    let _t = profile::span("read.probe");
    let o = &*src.overview;
    let mut used = Vec::new();
    let mut found = Found {
        point,
        made: src.made.clone(),
        budget: src.budget,
        cell: None,
        problem: None,
    };
    if !point.iter().all(|v| v.is_finite()) {
        return Ok((found, used));
    }
    let ids: Vec<usize> = kept.iter().map(|x| x.block).collect();
    for k in candidates(o, point, &ids) {
        profile::check()?;
        let block = load(src, k, kept)?;
        let halo = &block.subset.halo;
        let hit = crate::measure::locate_where(&block.mesh, point, |c| {
            !halo.get(c).copied().unwrap_or(true)
        });
        used.push(block.clone());
        let hit = hit.and_then(|c| Some((c as usize, *block.subset.file_cells.get(c as usize)?)));
        if let Some((c, file_cell)) = hit {
            let mut rows = vec![
                crate::inspect::row_of("block", k),
                crate::inspect::row_of("file_cell", file_cell),
            ];
            rows.extend(crate::inspect::probe_cell_rows(&block.mesh, point, c));
            found.cell = Some(FoundCell {
                block: k,
                file_cell,
                rows,
                values: block.fields.probe_values(&block.mesh, c, point, None),
            });
            break;
        }
    }
    Ok((found, used))
}

/// Block `k` of `src` with the fields of `src.made` made on it: from
/// `kept`, else read with its halo, BLOCK006 beyond the memory budget.
pub(crate) fn load(src: &Source, k: usize, kept: &[Arc<Kept>]) -> Result<Arc<Kept>, MeshError> {
    if let Some(x) = kept.iter().find(|x| x.block == k) {
        return Ok(x.with_made(&src.made));
    }
    let o = &*src.overview;
    crate::blocks::fits(o, &[k], src.budget)?;
    let ranges = crate::blocks::volume_ranges(o);
    let r = crate::blocks::read(&src.file, &[k], src.evidence.as_ref(), ranges)?;
    let base = Arc::new(r.fields);
    Ok(Arc::new(Kept {
        block: k,
        mesh: Arc::new(r.mesh),
        fields: base.clone(),
        base,
        made: Vec::new(),
        subset: Arc::new(r.subset),
    })
    .with_made(&src.made))
}

/// The blocks read for probes and the probe being found.
#[derive(Clone, Debug, Default)]
pub struct Cache {
    /// The blocks kept, the one used last at the end.
    pub kept: Vec<Arc<Kept>>,
    /// A probe's point being found on a thread of its own.
    pub job: Option<ProbeJob>,
    /// The blocks the streamlines last went through
    /// ([`crate::stream_blocks`]).
    pub stream_kept: Vec<Arc<Kept>>,
    /// Streamlines being traced through the blocks on a thread of their
    /// own.
    pub stream_job: Option<crate::stream_blocks::StreamJob>,
    /// A trace stopped because its sets changed, ending: the blocks it
    /// read are kept once it ends.
    pub stream_ending: Option<crate::stream_blocks::StreamJob>,
    /// Why a block could not be read for the streamlines last traced.
    pub stream_problem: Option<MeshError>,
    /// Slices and iso-surfaces being cut through the blocks on a thread
    /// of their own ([`crate::cut_blocks`]).
    pub cut_job: Option<crate::cut_blocks::CutJob>,
    /// The cuts wanted in the window and since when, until they have
    /// rested [`crate::cut_blocks::REST`].
    pub cut_rest: Option<(Vec<crate::cut_blocks::Want>, std::time::Instant)>,
    /// Why the cut ended last without faces (not when stopped).
    pub cut_problem: Option<MeshError>,
    /// The blocks the last cut went through last.
    pub cut_kept: Vec<Arc<Kept>>,
}

impl Cache {
    /// Keep the blocks of `used`, the last used last, within `keep`.
    pub(crate) fn keep(&mut self, used: Vec<Arc<Kept>>, keep: usize) {
        for b in used {
            self.kept.retain(|x| x.block != b.block);
            self.kept.push(b);
        }
        let over = self.kept.len().saturating_sub(keep);
        self.kept.drain(..over);
    }

    /// Stop finding and forget the blocks kept.
    pub(crate) fn clear(&mut self) {
        if let Some(j) = self.job.take() {
            j.stop();
        }
        for j in [self.stream_job.take(), self.stream_ending.take()]
            .into_iter()
            .flatten()
        {
            j.stop();
        }
        if let Some(j) = self.cut_job.take() {
            j.stop();
        }
        self.cut_rest = None;
        self.cut_problem = None;
        self.cut_kept.clear();
        self.kept.clear();
        self.stream_kept.clear();
        self.stream_problem = None;
    }
}

type Outcome = Result<(Found, Vec<Arc<Kept>>), MeshError>;
type Slot = Arc<(Mutex<Option<Outcome>>, Condvar)>;

/// A probe's point found on a thread of its own.
#[derive(Clone, Debug)]
pub struct ProbeJob {
    pub point: [f64; 3],
    pub made: Vec<Made>,
    pub budget: u64,
    progress: Arc<Progress>,
    done: Slot,
}

impl ProbeJob {
    /// Start finding `point` in the blocks of `src` (see [`find`]).
    pub fn start(src: Source, point: [f64; 3], kept: Vec<Arc<Kept>>) -> ProbeJob {
        let done: Slot = Arc::new((Mutex::new(None), Condvar::new()));
        let progress = Arc::new(Progress::default());
        let (made, budget) = (src.made.clone(), src.budget);
        let (out, watched) = (done.clone(), progress.clone());
        std::thread::spawn(move || {
            let r = profile::watch(watched, || find(&src, point, &kept));
            let (lock, ready) = &*out;
            if let Ok(mut v) = lock.lock() {
                *v = Some(r);
                ready.notify_all();
            }
        });
        ProbeJob {
            point,
            made,
            budget,
            progress,
            done,
        }
    }

    /// Whether it has ended (found, failed or stopped).
    pub fn is_done(&self) -> bool {
        self.done.0.lock().map_or(true, |v| v.is_some())
    }

    /// Ask it to stop at its next check.
    pub fn stop(&self) {
        self.progress.cancel();
    }

    /// What it gave, once it ended (waiting for it with `wait`).
    pub(crate) fn take(&self, wait: bool) -> Option<Outcome> {
        let (lock, ready) = &*self.done;
        let mut v = lock.lock().ok()?;
        while wait && v.is_none() {
            v = ready.wait(v).ok()?;
        }
        v.take()
    }
}

/// The source of the open overview, `None` when the open mesh is not
/// one.
pub(crate) fn source(open: &OpenMesh, budget: u64) -> Option<Source> {
    Some(Source {
        path: open.path.clone(),
        file: open.file(),
        overview: open.overview.clone()?,
        evidence: open.evidence.clone(),
        made: open.fields.made().to_vec(),
        budget,
    })
}

/// The points of the probes of `probes` the cells shown do not hold,
/// each once, in the probes' order.
pub(crate) fn wanted(probes: &[crate::measure::Probe]) -> Vec<[f64; 3]> {
    let mut v: Vec<[f64; 3]> = Vec::new();
    for p in probes.iter().filter(|p| p.at.cell.is_none()) {
        if !v.iter().any(|&q| same(q, p.at.point)) {
            v.push(p.at.point);
        }
    }
    v
}

/// Whether finding `f` need not be looked for again with the fields of
/// `made` made and within `budget`: it was found with them, or a block
/// could not be read for it within the same budget (it is not read again
/// until the budget changes).
pub(crate) fn is_current(f: &Found, made: &[Made], budget: u64) -> bool {
    match f.problem {
        None => f.made == made,
        Some(_) => f.budget == budget,
    }
}

/// How many blocks stay for the open overview and `budget`.
pub(crate) fn keep_count(src: &Source) -> usize {
    keep_for(&src.overview, src.budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use murmel_mesh::Bounds;
    use murmel_mesh::vtkhdf::{Block, Size};

    /// Invariant: a point is looked for in the blocks whose boxes hold
    /// it, borders included, those kept first, then the one it lies
    /// deepest inside; at most [`KEEP`] blocks stay, fewer within a
    /// quarter of the budget, one at least. Breaks when a block holding
    /// the point is left out (its probe would read as outside), when the
    /// order reads a far block first (a read too many), or when the
    /// blocks kept outgrow the budget. The session test reads files.
    #[test]
    fn a_point_is_looked_for_in_the_blocks_whose_boxes_hold_it() {
        let block = |min: [f64; 3], max: [f64; 3]| Block {
            cells: 10,
            bounds: Bounds { min, max },
            ranges: Vec::new(),
        };
        let o = Overview {
            cells: 30,
            size: Size {
                cells: 30,
                field_values: 0,
                blocked: true,
            },
            blocks: vec![
                block([0.0, 0.0, 0.0], [2.0, 1.0, 1.0]),
                block([1.0, 0.0, 0.0], [3.0, 1.0, 1.0]),
                block([5.0, 0.0, 0.0], [6.0, 1.0, 1.0]),
                Block {
                    cells: 0,
                    bounds: Bounds::empty(),
                    ranges: Vec::new(),
                },
            ],
            fields: Vec::new(),
        };
        // x = 1.2 lies 0.4 of block 0's length inside it, 0.1 of block 1's
        assert_eq!(candidates(&o, [1.2, 0.5, 0.5], &[]), vec![0, 1]);
        assert_eq!(candidates(&o, [1.8, 0.5, 0.5], &[]), vec![1, 0]);
        // on a border, and a block kept first
        assert_eq!(candidates(&o, [2.0, 0.5, 0.5], &[]), vec![1, 0]);
        assert_eq!(candidates(&o, [1.2, 0.5, 0.5], &[1]), vec![1, 0]);
        assert_eq!(candidates(&o, [4.0, 0.5, 0.5], &[]), Vec::<usize>::new());
        assert_eq!(candidates(&o, [6.0, 1.0, 1.0], &[]), vec![2]);
        let whole = o.size.open_bytes();
        let one = crate::blocks::bytes_for(&o, &[0]);
        assert_eq!(keep_for(&o, u64::MAX), KEEP);
        assert_eq!(keep_for(&o, 8 * one), 2);
        assert_eq!(keep_for(&o, 0), 1);
        assert!(whole > one);
    }
}
