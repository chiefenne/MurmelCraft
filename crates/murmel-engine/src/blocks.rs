//! Regions on an overview (large-mesh post-processing, stage 4; the
//! maintainer's card 1 of 2026-11-24, "overview plus regions beyond the
//! memory budget"): a box, sphere or slab region drawn on a mesh open as
//! an overview reads the blocks whose bounds it touches, with their halo,
//! and the regions show on those cells as on the mesh read whole. Once no
//! enabled region needs a block, the overview shows again.
//!
//! A block's bounds hold the points of its owned cells, so every cell
//! whose box touches a region lies in a block whose bounds touch it: the
//! regions take the same cells as on the whole mesh. The halo (the ghost
//! cells of the blocks read, owned by blocks not read) is read so node
//! values at the border are the whole mesh's, and it is never drawn
//! ([`crate::region::ensure_on`]).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use murmel_mesh::profile::{self, Progress};
use murmel_mesh::vtkhdf::{Overview, Subset};
use murmel_mesh::{Evidence, Mesh, MeshError};

use crate::field::FieldSet;
use crate::region::Region;

/// The halo a block read adds, as a share of the cells of its blocks,
/// for the memory estimate: about the most it was measured at (one block
/// of 65,536 cells of the 4 M file holds 11,134 halo cells, 17 %; four
/// hold 27,166, 10 %).
pub const HALO_SHARE: f64 = 0.25;

/// The blocks of an overview read for its regions.
#[derive(Clone, Debug)]
pub struct Blocks {
    /// The blocks read, each cell's id in the file and which cells are
    /// the halo.
    pub subset: Arc<Subset>,
    /// The overview's boundary and its fields, shown again once no
    /// region needs a block.
    pub overview: (Arc<Mesh>, Arc<FieldSet>),
}

impl Blocks {
    /// The cells of the halo.
    pub fn halo_cells(&self) -> usize {
        self.subset.halo.iter().filter(|&&h| h).count()
    }
}

/// The blocks whose bounds an enabled box, sphere or slab of `regions`
/// touches, ascending.
pub fn wanted(overview: &Overview, regions: &[Region]) -> Vec<usize> {
    let shapes: Vec<_> = regions
        .iter()
        .filter(|r| r.enabled && r.shape.is_geometric())
        .map(|r| &r.shape)
        .collect();
    if shapes.is_empty() {
        return Vec::new();
    }
    overview
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| {
            !b.bounds.is_empty()
                && shapes
                    .iter()
                    .any(|s| s.touches_box(b.bounds.min, b.bounds.max))
        })
        .map(|(k, _)| k)
        .collect()
}

/// The owned cells of `blocks`.
pub fn cells_of(overview: &Overview, blocks: &[usize]) -> u64 {
    blocks
        .iter()
        .filter_map(|&k| overview.blocks.get(k))
        .map(|b| b.cells)
        .sum()
}

/// About the memory reading `blocks` with their halo needs: the whole
/// read's ([`murmel_mesh::vtkhdf::Size::open_bytes`]) share of their
/// cells, and [`HALO_SHARE`] more.
pub fn bytes_for(overview: &Overview, blocks: &[usize]) -> u64 {
    if overview.cells == 0 {
        return 0;
    }
    let share = cells_of(overview, blocks) as f64 / overview.cells as f64 * (1.0 + HALO_SHARE);
    (overview.size.open_bytes() as f64 * share).ceil() as u64
}

/// BLOCK006 when reading `blocks` needs more than `budget` bytes.
pub fn fits(overview: &Overview, blocks: &[usize], budget: u64) -> Result<(), MeshError> {
    let needs = bytes_for(overview, blocks);
    if needs <= budget {
        return Ok(());
    }
    use crate::memory::gib;
    Err(MeshError::new(
        "BLOCK006",
        format!(
            "the regions touch {} of the {} blocks ({} cells), which need about {} GB with their halo, more than the memory budget of {} GB",
            blocks.len(),
            overview.blocks.len(),
            cells_of(overview, blocks),
            gib(needs),
            gib(budget),
        ),
    ))
}

/// The range of each field of the volume over all blocks, from the block
/// index, which an overview and the blocks read from it colour over.
pub fn volume_ranges(overview: &Overview) -> Vec<(String, (f64, f64))> {
    overview
        .fields
        .iter()
        .filter_map(|f| {
            overview
                .range(&f.name)
                .map(|[a, b]| (f.name.clone(), (a, b)))
        })
        .collect()
}

/// Whether every block of `a` is in `b` (both ascending).
pub fn within(a: &[usize], b: &[usize]) -> bool {
    a.iter().all(|k| b.binary_search(k).is_ok())
}

/// Blocks read, ready to show.
#[derive(Debug)]
pub struct Read {
    pub mesh: Mesh,
    pub fields: FieldSet,
    pub subset: Subset,
}

/// Read `blocks` of the blocked VTKHDF file at `path` with their halo: the
/// evidence's patch roles applied, the fields coloured over the volume's
/// `ranges`.
pub fn read(
    path: &Path,
    blocks: &[usize],
    evidence: Option<&Evidence>,
    ranges: Vec<(String, (f64, f64))>,
) -> Result<Read, MeshError> {
    let (mut mesh, results, subset) = murmel_mesh::vtkhdf::read_blocks(path, blocks, true)?;
    if let Some(e) = evidence {
        mesh.apply_evidence_roles(e);
    }
    let fields = FieldSet::new(results).with_volume_ranges(ranges);
    Ok(Read {
        mesh,
        fields,
        subset,
    })
}

/// What a block read on a thread of its own gives.
type Slot = Arc<(Mutex<Option<Result<Read, MeshError>>>, Condvar)>;

/// Blocks read on a thread of their own.
#[derive(Clone, Debug)]
pub struct BlockJob {
    /// The blocks being read, ascending.
    pub blocks: Vec<usize>,
    /// Their owned cells.
    pub cells: u64,
    /// The reader's phase, and Cancel.
    pub progress: Arc<Progress>,
    pub started: Instant,
    done: Slot,
}

impl BlockJob {
    /// Start reading `blocks` of the file at `path` (see [`read`]).
    pub fn start(
        path: PathBuf,
        blocks: Vec<usize>,
        cells: u64,
        evidence: Option<Evidence>,
        ranges: Vec<(String, (f64, f64))>,
    ) -> BlockJob {
        let done: Slot = Arc::new((Mutex::new(None), Condvar::new()));
        let progress = Arc::new(Progress::default());
        let (out, watched, which) = (done.clone(), progress.clone(), blocks.clone());
        std::thread::spawn(move || {
            let r = profile::watch(watched, || {
                let r = read(&path, &which, evidence.as_ref(), ranges)?;
                profile::check()?;
                Ok(r)
            });
            let (lock, ready) = &*out;
            if let Ok(mut v) = lock.lock() {
                *v = Some(r);
                ready.notify_all();
            }
        });
        BlockJob {
            blocks,
            cells,
            progress,
            started: Instant::now(),
            done,
        }
    }

    /// Whether the read has ended (read, failed or stopped).
    pub fn is_done(&self) -> bool {
        self.done.0.lock().map_or(true, |v| v.is_some())
    }

    /// Ask the reader to stop at its next check.
    pub fn stop(&self) {
        self.progress.cancel();
    }

    /// What the read gave, once it ended (waiting for it with `wait`).
    pub(crate) fn take(&self, wait: bool) -> Option<Result<Read, MeshError>> {
        let (lock, ready) = &*self.done;
        let mut v = lock.lock().ok()?;
        while wait && v.is_none() {
            v = ready.wait(v).ok()?;
        }
        v.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::region::Shape;
    use murmel_mesh::Bounds;
    use murmel_mesh::vtkhdf::{Block, Size};

    fn block(min: [f64; 3], max: [f64; 3], cells: u64) -> Block {
        Block {
            cells,
            bounds: Bounds { min, max },
            ranges: Vec::new(),
        }
    }

    /// Four blocks in a row along x, of 10 cells each, 1 wide.
    fn overview() -> Overview {
        Overview {
            cells: 40,
            size: Size {
                cells: 40,
                field_values: 0,
                blocked: true,
            },
            blocks: (0..4)
                .map(|k| {
                    let x = k as f64;
                    block([x, 0.0, 0.0], [x + 1.0, 1.0, 1.0], 10)
                })
                .collect(),
            fields: Vec::new(),
        }
    }

    fn boxed(lo: f64, hi: f64) -> Region {
        Region::new(Shape::Box {
            center: [0.5 * (lo + hi), 0.5, 0.5],
            size: [hi - lo, 0.5, 0.5],
        })
    }

    /// Invariant: the blocks a region reads are those whose bounds an
    /// enabled box, sphere or slab touches, borders included; a disabled
    /// region and a set of cells read none. Breaks when the test takes
    /// the block's centre (a box over the border of two blocks would miss
    /// the cells of one), when it ignores `enabled`, or when a zone
    /// region reads every block. No other test reads the block choice
    /// apart from a file.
    #[test]
    fn regions_want_the_blocks_their_shapes_touch() {
        let o = overview();
        assert_eq!(wanted(&o, &[boxed(1.2, 1.8)]), vec![1]);
        // touching the border of blocks 1 and 2 at x = 2
        assert_eq!(wanted(&o, &[boxed(1.5, 2.0)]), vec![1, 2]);
        assert_eq!(wanted(&o, &[boxed(0.2, 0.4), boxed(3.2, 3.4)]), vec![0, 3]);
        let mut off = boxed(1.2, 1.8);
        off.enabled = false;
        assert!(wanted(&o, &[off]).is_empty());
        let zone = Region::new(Shape::Zone { zone: "a".into() });
        assert!(wanted(&o, &[zone]).is_empty());
        let slab = Region::new(Shape::Slab {
            origin: [2.5, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            thickness: 0.2,
        });
        assert_eq!(wanted(&o, &[slab]), vec![2]);
    }

    /// Invariant: the memory a block read is estimated at is the whole
    /// read's share of its cells and a quarter more for the halo, and
    /// BLOCK006 refuses it beyond the budget. Breaks when the estimate
    /// leaves out the halo or counts the blocks rather than their cells.
    #[test]
    fn a_block_read_beyond_the_budget_is_refused() {
        let o = overview();
        let whole = o.size.open_bytes();
        assert_eq!(
            bytes_for(&o, &[1]),
            (whole as f64 * 0.25 * 1.25).ceil() as u64
        );
        assert!(fits(&o, &[1], whole).is_ok());
        let e = fits(&o, &[0, 1, 2, 3], whole).unwrap_err();
        assert_eq!(e.code, "BLOCK006");
        assert!(within(&[1, 3], &[0, 1, 3]));
        assert!(!within(&[1, 2], &[0, 1, 3]));
        assert!(within(&[], &[]));
    }
}
