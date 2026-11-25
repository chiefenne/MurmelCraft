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
//! Fields made on the overview (derived and calculated) do not draw on
//! the grid; they need a region's cells, or the exact faces cut through
//! the blocks of the file ([`crate::cut_blocks`]), which take the grid's
//! place once cut. Streamlines are not traced on the grid but through
//! the blocks of the file ([`crate::stream_blocks`]).

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use murmel_mesh::profile::{self, Progress};
#[cfg(doc)]
use murmel_mesh::vtkhdf::PREVIEW_CELLS;
use murmel_mesh::vtkhdf::PreviewGrid;
use murmel_mesh::{Mesh, MeshError, NO_CELL};

use crate::display::{Display, Item};
use crate::field::FieldSet;
use crate::surface::{SlicePart, Surface};
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
/// are the grid's. A slice or an iso-surface whose exact faces are cut
/// through the blocks ([`crate::cut_blocks`]) draws those instead (the
/// arrows on a slice stay the grid's).
pub(crate) fn append(s: &mut Surface, p: &Preview, tokens: &Tokens, display: &Display) {
    let mut d = display.clone();
    let arrows = d.arrows_shown();
    // Slices with arrows are cut on the grid for their arrows; their
    // faces are taken out again after.
    let mut exact = Vec::new();
    for (k, sl) in d.slices.iter_mut().enumerate() {
        if sl.enabled && crate::cut_blocks::covers(display, Item::Slice(k as u32), s) {
            if arrows {
                exact.push(k as u32);
            } else {
                sl.enabled = false;
            }
        }
    }
    for (k, iso) in d.isos.iter_mut().enumerate() {
        if iso.enabled && crate::cut_blocks::covers(display, Item::Iso(k as u32), s) {
            iso.enabled = false;
        }
    }
    // The block boxes alone draw nothing on the grid.
    let cuts = d.slices.iter().any(|s| s.enabled) || d.isos.iter().any(|i| i.enabled);
    if !cuts && !arrows {
        return;
    }
    d.field_values = None;
    crate::field::ensure_field(&mut d, &p.mesh, &p.fields);
    d.quality = None;
    d.regions.clear();
    d.region_cells = None;
    d.stream_traces = None;
    d.isolated.retain(|i| !matches!(i, Item::Cell(_)));
    let shown = d.shown_cells(&p.mesh);
    let before = s.tri_cell.len();
    let parts = s.slice_parts.len();
    crate::extract::append(s, &p.mesh, Some(&p.fields), tokens, &d, &shown);
    s.tri_cell[before..].fill(NO_CELL);
    if !exact.is_empty() {
        drop_slices(s, parts, &exact);
    }
}

/// `v` without the entries in `gone` (ascending, apart).
fn without<T: Copy>(v: &mut Vec<T>, gone: &[Range<usize>]) {
    let mut out = Vec::with_capacity(v.len());
    let mut at = 0;
    for r in gone {
        let start = r.start.clamp(at, v.len());
        out.extend_from_slice(&v[at..start]);
        at = r.end.clamp(start, v.len());
    }
    out.extend_from_slice(&v[at..]);
    *v = out;
}

/// Take the faces and outlines of the slices `gone` out of `s` again,
/// as drawn by the call that added `s.slice_parts[from..]` (their arrows
/// stay); the vertices stay, unused.
fn drop_slices(s: &mut Surface, from: usize, gone: &[u32]) {
    let parts = s.slice_parts.split_off(from.min(s.slice_parts.len()));
    let (out, keep): (Vec<SlicePart>, Vec<SlicePart>) =
        parts.into_iter().partition(|p| gone.contains(&p.slice));
    let faces: Vec<Range<usize>> = out
        .iter()
        .map(|p| p.faces.start as usize..p.faces.end as usize)
        .collect();
    let tris: Vec<Range<usize>> = faces.iter().map(|r| r.start / 3..r.end / 3).collect();
    let edges: Vec<Range<usize>> = out
        .iter()
        .map(|p| p.edges.start as usize..p.edges.end as usize)
        .collect();
    without(&mut s.indices, &faces);
    without(&mut s.tri_cell, &tris);
    without(&mut s.tri_face, &tris);
    without(&mut s.edge_positions, &edges);
    without(&mut s.edge_colors, &edges);
    // Where an entry after them is now.
    let moved = |x: u32, gone: &[Range<usize>]| -> u32 {
        let before: usize = gone
            .iter()
            .filter(|r| r.end <= x as usize)
            .map(|r| r.end - r.start)
            .sum();
        x - before as u32
    };
    s.slice_parts.extend(keep.into_iter().map(|p| SlicePart {
        slice: p.slice,
        faces: moved(p.faces.start, &faces)..moved(p.faces.end, &faces),
        edges: moved(p.edges.start, &edges)..moved(p.edges.end, &edges),
    }));
    for p in &mut s.iso_parts {
        p.faces = moved(p.faces.start, &faces)..moved(p.faces.end, &faces);
    }
    let a = s.arrow_faces.clone();
    if !a.is_empty() {
        s.arrow_faces = moved(a.start, &faces)..moved(a.end, &faces);
    }
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
