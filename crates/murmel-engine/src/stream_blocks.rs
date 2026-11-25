//! Streamlines on an overview (large-mesh post-processing; the
//! maintainer's card 8 of 2026-11-24, "block cache for streamlines and
//! probes"): an overview shows no volume cells, so its streamlines are
//! traced through the blocks of the file, each read with its halo the
//! first time a line reaches it.
//!
//! - **Seeds** are placed as on the mesh read whole ([`crate::stream`]):
//!   each point is looked for in the blocks whose bounds hold it
//!   ([`crate::probe_blocks::candidates`]), walking from the nearest cell
//!   centre of the first; a patch's seeds start just inside its faces.
//! - **Tracing.** A walk that enters a halo cell goes on from the same
//!   cell in the block owning it (read then), so the field is always read
//!   in an owned cell, whose neighbours the halo holds. The cell values
//!   are the whole mesh's; node values on a block's border take the halo
//!   cells' gradients, so lines there can differ a little from those of
//!   the mesh read whole.
//! - **Limits.** A trace reads at most as many blocks as a quarter of the
//!   memory budget holds; a line that would need another ends there
//!   (counted as `unread`, BLOCK006 says why), and the set is traced
//!   again once the budget changes. The blocks it went through stay for
//!   the next trace, so moving a seed reads only new blocks.
//! - **In the window** the lines are traced on a thread of their own;
//!   the overview shows meanwhile, and the lines appear once traced. A
//!   trace stopped (Cancel, Esc) keeps the lines up to the blocks read
//!   and is not made again until the set or the budget changes; a set
//!   changed while traced stops that trace, whose blocks are kept.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use murmel_mesh::profile::Progress;
use murmel_mesh::{Component, Mesh, MeshError};

use crate::field::{FieldChoice, FieldValues};
use crate::probe_blocks::{Kept, Source};
use crate::stream::{At, Part, Parts, Stream, Traced};

/// A block read for the lines, with the values they read in it.
struct Loaded {
    kept: Arc<Kept>,
    v: [Arc<FieldValues>; 3],
    color: Option<Arc<FieldValues>>,
    /// The owned cells by their id in the file, ascending: (id, cell).
    owned: Vec<(u64, u32)>,
}

/// The blocks of an overview's file as parts of a trace, read when a
/// line first reaches them.
pub(crate) struct Blocks<'s> {
    src: &'s Source,
    field: String,
    color: Option<FieldChoice>,
    /// Blocks read before, taken again without reading.
    kept: &'s [Arc<Kept>],
    slots: Vec<OnceLock<Option<Loaded>>>,
    /// Where each block's owned cells start among the file's.
    starts: Vec<u64>,
    /// The most blocks one trace reads, and how many it has.
    most: usize,
    taken: AtomicUsize,
    /// How many blocks the trace has read (or taken kept), for the
    /// window to say.
    read: Arc<AtomicUsize>,
    stop: Arc<Progress>,
    problem: Mutex<Option<MeshError>>,
}

/// The most blocks a trace of `src` reads: as many as a quarter of the
/// memory budget holds, one at least.
pub(crate) fn most_blocks(src: &Source) -> usize {
    let o = &*src.overview;
    let largest = (0..o.blocks.len())
        .map(|k| crate::blocks::bytes_for(o, &[k]))
        .max()
        .unwrap_or(0)
        .max(1);
    ((src.budget / 4 / largest) as usize).max(1)
}

impl<'s> Blocks<'s> {
    pub(crate) fn new(
        src: &'s Source,
        s: &Stream,
        kept: &'s [Arc<Kept>],
        stop: Arc<Progress>,
        read: Arc<AtomicUsize>,
    ) -> Blocks<'s> {
        let o = &*src.overview;
        let mut starts = Vec::with_capacity(o.blocks.len() + 1);
        let mut at = 0u64;
        for b in &o.blocks {
            starts.push(at);
            at += b.cells;
        }
        starts.push(at);
        Blocks {
            src,
            field: s.field.clone(),
            color: s.color.clone(),
            kept,
            slots: (0..o.blocks.len()).map(|_| OnceLock::new()).collect(),
            starts,
            most: most_blocks(src),
            taken: AtomicUsize::new(0),
            read,
            stop,
            problem: Mutex::new(None),
        }
    }

    /// The blocks read or taken for this trace.
    pub(crate) fn used(&self) -> Vec<Arc<Kept>> {
        self.slots
            .iter()
            .filter_map(|s| s.get().and_then(Option::as_ref))
            .map(|l| l.kept.clone())
            .collect()
    }

    /// Why a block could not be read, the first time.
    pub(crate) fn problem(&self) -> Option<MeshError> {
        self.problem.lock().ok().and_then(|p| p.clone())
    }

    fn note(&self, e: MeshError) {
        if let Ok(mut p) = self.problem.lock()
            && p.is_none()
        {
            *p = Some(e);
        }
    }

    /// Block `b`, read the first time it is asked for.
    fn load(&self, b: usize) -> Option<&Loaded> {
        self.slots.get(b)?.get_or_init(|| self.read(b)).as_ref()
    }

    fn read(&self, b: usize) -> Option<Loaded> {
        if self.stop.cancelled() {
            return None;
        }
        if self.taken.fetch_add(1, Ordering::SeqCst) >= self.most {
            self.note(MeshError::new(
                "BLOCK006",
                format!(
                    "the streamlines reach more than the {} blocks a quarter of the memory budget holds",
                    self.most
                ),
            ));
            return None;
        }
        let kept = match crate::probe_blocks::load(self.src, b, self.kept) {
            Ok(k) => k,
            Err(e) => {
                self.note(e);
                return None;
            }
        };
        let comp = |component| {
            kept.fields.values_cached(
                &kept.mesh,
                &FieldChoice {
                    name: self.field.clone(),
                    component,
                },
            )
        };
        let v = [
            comp(Component::X)?,
            comp(Component::Y)?,
            comp(Component::Z)?,
        ];
        let color = self
            .color
            .as_ref()
            .and_then(|c| kept.fields.values_cached(&kept.mesh, c));
        self.read.fetch_add(1, Ordering::Relaxed);
        let (halo, ids) = (&kept.subset.halo, &kept.subset.file_cells);
        let mut owned: Vec<(u64, u32)> = (0..kept.mesh.cell_count())
            .filter(|&c| !halo.get(c).copied().unwrap_or(true))
            .filter_map(|c| Some((*ids.get(c)?, c as u32)))
            .collect();
        owned.sort_unstable();
        Some(Loaded {
            kept,
            v,
            color,
            owned,
        })
    }

    /// The block owning file cell `id`.
    fn block_of(&self, id: u64) -> Option<usize> {
        let k = self.starts.partition_point(|&s| s <= id);
        (k >= 1 && k < self.starts.len()).then(|| k - 1)
    }

    /// The cell holding `p`: looked for from the nearest cell centre of
    /// each block whose bounds hold it, until a walk ends in a cell.
    pub(crate) fn locate(&self, p: [f64; 3]) -> Option<At> {
        let kept: Vec<usize> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.get().is_some_and(Option::is_some))
            .map(|(k, _)| k)
            .collect();
        for k in crate::probe_blocks::candidates(&self.src.overview, p, &kept) {
            let Some(l) = self.load(k) else {
                continue;
            };
            let Some(start) = l.kept.fields.cell_grid(&l.kept.mesh).nearest(p) else {
                continue;
            };
            let start = At {
                part: k as u32,
                cell: start,
            };
            if let Some(at) = crate::stream::locate_in(self, p, start) {
                return Some(at);
            }
        }
        None
    }
}

impl Parts for Blocks<'_> {
    fn part(&self, i: u32) -> Option<Part<'_>> {
        let l = self.slots.get(i as usize)?.get()?.as_ref()?;
        Some(Part {
            mesh: &l.kept.mesh,
            fields: &l.kept.fields,
            v: [&*l.v[0], &*l.v[1], &*l.v[2]],
            color: l.color.as_deref(),
            halo: Some(&l.kept.subset.halo),
        })
    }

    fn owner(&self, i: u32, c: u32) -> Option<At> {
        let l = self.slots.get(i as usize)?.get()?.as_ref()?;
        let id = *l.kept.subset.file_cells.get(c as usize)?;
        let b = self.block_of(id)?;
        let o = self.load(b)?;
        let k = o.owned.binary_search_by_key(&id, |x| x.0).ok()?;
        Some(At {
            part: b as u32,
            cell: o.owned.get(k)?.1,
        })
    }
}

/// A trace through the blocks: the lines, the blocks they went through
/// and why a block could not be read, the first time.
pub(crate) struct Through {
    pub traced: Traced,
    pub used: Vec<Arc<Kept>>,
    pub problem: Option<MeshError>,
}

/// Trace `s` through the blocks of `src` (the overview's boundary
/// `domain` seeds it), taking those of `kept` without reading them.
pub(crate) fn trace(
    src: &Source,
    domain: &Mesh,
    s: &Stream,
    kept: &[Arc<Kept>],
    stop: Arc<Progress>,
    read: Arc<AtomicUsize>,
) -> Through {
    let o = &*src.overview;
    let failed = |why: String| {
        let mut traced = crate::stream::failed(s, why);
        traced.budget = Some(src.budget);
        Through {
            traced,
            used: Vec::new(),
            problem: None,
        }
    };
    let Some(f) = o.fields.iter().find(|f| f.name == s.field) else {
        return failed(format!("no field {}", s.field));
    };
    if f.components != 3 {
        return failed(format!("{} is not a vector field", s.field));
    }
    let top = o.range(&s.field).map_or(0.0, |[a, b]| a.abs().max(b.abs()));
    let blocks = Blocks::new(src, s, kept, stop.clone(), read);
    let locate = |p: [f64; 3]| blocks.locate(p);
    let mut t = crate::stream::trace_parts(&blocks, domain, s, top, &locate);
    t.budget = Some(src.budget);
    let problem = blocks.problem();
    if t.problem.is_none() {
        if stop.cancelled() {
            t.problem = Some(STOPPED.to_string());
        } else if t.lines.is_empty()
            && let Some(e) = &problem
        {
            t.problem = Some(format!("{}: {}", e.code, e.message));
        }
    }
    let used = blocks.used();
    t.blocks = Some(used.len());
    Through {
        traced: t,
        used,
        problem,
    }
}

/// What a trace stopped before its blocks were read says.
pub const STOPPED: &str = "stopped before the blocks it reached were read";

/// Whether trace `t` need not be made again for set `s` within `budget`:
/// traced through the blocks for it as it is, and either whole or
/// within the same budget (a trace cut short by the budget is made
/// again once it changes; one stopped, only then too).
pub(crate) fn is_current(t: &Traced, s: &Stream, budget: u64) -> bool {
    let whole = t.ends.unread == 0 && t.problem.is_none();
    t.fits(s) && t.budget.is_some_and(|b| whole || b == budget)
}

/// The enabled sets of `streams` the traces of `traces` do not hold as
/// they are now (within `budget`), by index.
pub(crate) fn todo(
    streams: &[Stream],
    traces: Option<&crate::stream::Traces>,
    budget: u64,
) -> Vec<(usize, Stream)> {
    streams
        .iter()
        .enumerate()
        .filter(|(_, s)| s.enabled)
        .filter(|&(k, s)| {
            !traces
                .and_then(|t| t.0.get(k))
                .and_then(Option::as_ref)
                .is_some_and(|t| is_current(t, s, budget))
        })
        .map(|(k, s)| (k, s.clone()))
        .collect()
}

/// Whether two lists of sets to trace trace the same lines.
pub(crate) fn same_sets(a: &[(usize, Stream)], b: &[(usize, Stream)]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|((i, s), (j, t))| i == j && crate::stream::same_trace(s, t))
}

/// Put in place the traces of `traced` whose set is still as it was
/// traced for; true when one was.
pub(crate) fn install(
    display: &mut crate::display::Display,
    traced: Vec<(usize, Stream, Arc<Traced>)>,
) -> bool {
    let n = display.streams.len();
    let mut v = display
        .stream_traces
        .as_deref()
        .map(|t| t.0.clone())
        .unwrap_or_default();
    v.resize(n, None);
    let mut any = false;
    for (k, s, t) in traced {
        let now = display.streams.get(k);
        if now.is_some_and(|now| crate::stream::same_trace(now, &s) && t.fits(now)) {
            v[k] = Some(t);
            any = true;
        }
    }
    if any {
        display.stream_traces = Some(Arc::new(crate::stream::Traces(v)));
    }
    any
}

/// What a trace on a thread of its own gives: each set traced (its index
/// in the display, as it was, and its trace), the blocks to keep and why
/// a block could not be read, the first time.
#[derive(Debug)]
pub struct Outcome {
    pub traced: Vec<(usize, Stream, Arc<Traced>)>,
    pub kept: Vec<Arc<Kept>>,
    pub problem: Option<MeshError>,
}

type Slot = Arc<(Mutex<Option<Outcome>>, Condvar)>;

/// Streamline sets traced through the blocks on a thread of their own.
#[derive(Clone, Debug)]
pub struct StreamJob {
    /// The sets being traced (their index in the display, as they were).
    pub sets: Vec<(usize, Stream)>,
    /// The memory budget they are traced within.
    pub budget: u64,
    pub progress: Arc<Progress>,
    pub started: std::time::Instant,
    /// How many blocks the trace has read so far.
    pub read: Arc<AtomicUsize>,
    done: Slot,
}

impl StreamJob {
    /// Start tracing `sets` through the blocks of `src`, taking those of
    /// `kept` without reading them.
    pub(crate) fn start(
        src: Source,
        domain: Arc<Mesh>,
        sets: Vec<(usize, Stream)>,
        kept: Vec<Arc<Kept>>,
    ) -> StreamJob {
        let done: Slot = Arc::new((Mutex::new(None), Condvar::new()));
        let progress = Arc::new(Progress::default());
        let read = Arc::new(AtomicUsize::new(0));
        let budget = src.budget;
        let (out, stop, todo, count) = (done.clone(), progress.clone(), sets.clone(), read.clone());
        std::thread::spawn(move || {
            let mut kept = kept;
            let mut traced = Vec::new();
            let mut problem = None;
            for (k, s) in todo {
                let t = trace(&src, &domain, &s, &kept, stop.clone(), count.clone());
                traced.push((k, s, Arc::new(t.traced)));
                problem = problem.or(t.problem);
                for b in t.used {
                    kept.retain(|x| x.block != b.block);
                    kept.push(b);
                }
            }
            let over = kept.len().saturating_sub(most_blocks(&src));
            kept.drain(..over);
            let (lock, ready) = &*out;
            if let Ok(mut v) = lock.lock() {
                *v = Some(Outcome {
                    traced,
                    kept,
                    problem,
                });
                ready.notify_all();
            }
        });
        StreamJob {
            sets,
            budget,
            progress,
            started: std::time::Instant::now(),
            read,
            done,
        }
    }

    pub fn is_done(&self) -> bool {
        self.done.0.lock().map_or(true, |v| v.is_some())
    }

    /// Ask it to stop: the blocks not yet read are not read, and the
    /// lines that reach them end there.
    pub fn stop(&self) {
        self.progress.cancel();
    }

    /// Its outcome once traced (waiting for it with `wait`).
    pub(crate) fn take(&self, wait: bool) -> Option<Outcome> {
        let (lock, ready) = &*self.done;
        let mut v = lock.lock().ok()?;
        while wait && v.is_none() {
            v = ready.wait(v).ok()?;
        }
        v.take()
    }
}
