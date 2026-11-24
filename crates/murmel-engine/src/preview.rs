//! Slices, iso-surfaces and arrows on an overview (large-mesh
//! post-processing; the maintainer's card 7 of 2026-11-24: the overview
//! is the surface, the block ranges and a preview grid). A mesh open as
//! an overview has no volume cells, so what cuts through the volume draws
//! on its preview grid ([`murmel_mesh::vtkhdf::read_preview`]): the cell
//! fields of the file on a regular grid of about
//! [`murmel_mesh::vtkhdf::PREVIEW_CELLS`] cells along the longest side,
//! each grid cell showing the file cell its centre lies in. Once a region
//! reads the cells of its blocks, they draw on those cells instead, as on
//! the mesh read whole.
//!
//! The grid is read once, the first time a slice, an iso-surface or the
//! arrows show on the overview: in the window on a thread of its own, the
//! overview showing meanwhile, by the tools before they return. It is
//! kept with the overview's fields ([`crate::field::FieldSet::preview`]),
//! so it stays while a region's blocks show and comes back with the
//! overview, and in the cache folder ([`crate::preview_cache`]), so the
//! file opened again unchanged does not sample its volume again.
//!
//! Fields made on the overview (derived and calculated) and the
//! streamlines do not draw on the grid; they need a region's cells.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use murmel_mesh::profile::{self, Progress};
#[cfg(doc)]
use murmel_mesh::vtkhdf::PREVIEW_CELLS;
use murmel_mesh::vtkhdf::PreviewGrid;
use murmel_mesh::{Mesh, MeshError, NO_CELL};

use crate::display::Display;
use crate::field::FieldSet;
use crate::surface::Surface;
use crate::theme::Tokens;

/// The preview grid of an overview's volume.
#[derive(Debug)]
pub struct Preview {
    /// One hexahedron for each grid cell inside the volume.
    pub mesh: Mesh,
    /// The cell fields of the file on those cells, coloured over the
    /// volume's ranges as the overview is.
    pub fields: FieldSet,
    /// Where the grid lies and the file cell each of its cells shows.
    pub grid: PreviewGrid,
}

/// Read the preview grid of the blocked VTKHDF file at `path` with
/// `cells` grid cells along the longest side ([`PREVIEW_CELLS`] in the
/// window and the tools), its fields coloured over the volume's
/// `ranges`: the grid kept in `cache` while the file is unchanged, else
/// sampled from the file (and kept there).
pub fn read(
    path: &Path,
    cells: usize,
    ranges: Vec<(String, (f64, f64))>,
    cache: Option<&Path>,
) -> Result<Preview, MeshError> {
    let kept = cache.and_then(|dir| {
        let _t = profile::span("read.preview.cache");
        crate::preview_cache::load(dir, path, cells)
    });
    let (mesh, results, grid) = match kept {
        Some(k) => k,
        None => {
            let before = crate::checked::Stamp::of(path);
            let k = murmel_mesh::vtkhdf::read_preview(path, cells)?;
            if let (Some(dir), Some(before)) = (cache, before) {
                let _t = profile::span("read.preview.keep");
                crate::preview_cache::store(dir, path, &before, cells, &k);
            }
            k
        }
    };
    let fields = FieldSet::new(results).with_volume_ranges(ranges);
    Ok(Preview { mesh, fields, grid })
}

/// Draw the slices, iso-surfaces and arrows of `display` on the preview
/// grid `p` into `s`: the field shown takes the grid's values, the cells
/// shown are those that no crinkle clip plane takes away and no hidden
/// zone holds. Their triangles name no cell, as the cells they come from
/// are the grid's.
pub(crate) fn append(s: &mut Surface, p: &Preview, tokens: &Tokens, display: &Display) {
    // The block boxes alone draw nothing on the grid.
    let cuts = display.slices.iter().any(|s| s.enabled) || display.isos.iter().any(|i| i.enabled);
    if !cuts && !display.arrows_shown() {
        return;
    }
    let mut d = display.clone();
    d.field_values = None;
    crate::field::ensure_field(&mut d, &p.mesh, &p.fields);
    d.quality = None;
    d.regions.clear();
    d.region_cells = None;
    d.stream_traces = None;
    d.isolated
        .retain(|i| !matches!(i, crate::display::Item::Cell(_)));
    let shown = d.shown_cells(&p.mesh);
    let before = s.tri_cell.len();
    crate::extract::append(s, &p.mesh, Some(&p.fields), tokens, &d, &shown);
    s.tri_cell[before..].fill(NO_CELL);
}

/// What a preview read on a thread of its own gives.
type Slot = Arc<(Mutex<Option<Result<Preview, MeshError>>>, Condvar)>;

/// A preview grid read on a thread of its own.
#[derive(Clone, Debug)]
pub struct PreviewJob {
    /// The reader's phase, and Cancel.
    pub progress: Arc<Progress>,
    pub started: Instant,
    done: Slot,
}

impl PreviewJob {
    /// Start reading the preview grid of the file at `path` (see
    /// [`read`]).
    pub fn start(
        path: PathBuf,
        cells: usize,
        ranges: Vec<(String, (f64, f64))>,
        cache: Option<PathBuf>,
    ) -> PreviewJob {
        let done: Slot = Arc::new((Mutex::new(None), Condvar::new()));
        let progress = Arc::new(Progress::default());
        let (out, watched) = (done.clone(), progress.clone());
        std::thread::spawn(move || {
            let r = profile::watch(watched, || {
                let p = read(&path, cells, ranges, cache.as_deref())?;
                profile::check()?;
                Ok(p)
            });
            let (lock, ready) = &*out;
            if let Ok(mut v) = lock.lock() {
                *v = Some(r);
                ready.notify_all();
            }
        });
        PreviewJob {
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
    pub(crate) fn take(&self, wait: bool) -> Option<Result<Preview, MeshError>> {
        let (lock, ready) = &*self.done;
        let mut v = lock.lock().ok()?;
        while wait && v.is_none() {
            v = ready.wait(v).ok()?;
        }
        v.take()
    }
}
