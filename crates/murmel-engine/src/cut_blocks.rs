//! Exact slices and iso-surfaces on an overview (large-mesh
//! post-processing; the maintainer's card 8 of 2026-11-24, "stream
//! slices and iso-surfaces"). An overview holds no volume cells, so a
//! slice or an iso-surface shows on its preview grid at once
//! ([`crate::preview`]); its exact faces are then cut through the blocks
//! of the file, one block at a time, and take the grid's place. The
//! next blocks are read on a thread of their own while one is cut.
//!
//! - **Which blocks.** A slice cuts the blocks whose box ([`BlockBounds`]
//!   of the file) one of its planes crosses. An iso-surface cuts the
//!   blocks whose values, from the smallest to the largest of their own
//!   range and those of the blocks whose boxes touch theirs, hold its
//!   value: a cell's node values lie between the values of the cells
//!   around the node, owned by the block or by a block beside it, and
//!   the faces inside a cell take the values between its nodes' and its
//!   own, so the other blocks hold no piece of it (the span-space idea
//!   of Livnat, Shen and Johnson, 1996). A field the file
//!   has no ranges for, or one made on the overview, cuts every block.
//!   A block every enabled clip plane takes away whole is not read.
//! - **Each block** is read with its halo ([`crate::probe_blocks::load`],
//!   the blocks kept for probes and streamlines taken without reading)
//!   and cut as the mesh read whole is cut ([`crate::extract::append`]),
//!   its owned cells shown and the halo hidden, so each face is cut once,
//!   in the block owning its cell. The cell values are the whole mesh's;
//!   node values next to a block's border take the halo cells'
//!   gradients, so faces there can differ a little from those of the
//!   mesh read whole, as for the regions' blocks. Only the faces stay,
//!   and the last blocks gone through, as many as an eighth of the
//!   memory budget holds, for the next cut (a slice moved a little cuts
//!   mostly the same blocks): a cut of a 100 M cell file holds one block
//!   more than those at a time, and its faces at most a quarter of the
//!   budget (BLOCK006 beyond).
//! - **Kept.** The faces stay with the display ([`Cuts`]) until what they
//!   depend on changes ([`CutKey`]: the item, the field shown, colours,
//!   clip planes, hidden zones and the surface's frame). Selecting an
//!   item colours its faces again without cutting ([`append`]), as
//!   [`crate::extract`] colours a selected item.
//! - **In the window** the cut runs on a thread of its own once the
//!   items have rested [`REST`], the preview grid showing meanwhile; a
//!   change while it runs starts it again. A cut stopped (Cancel, Esc) or
//!   beyond the budget keeps the item on the grid and is not made again
//!   until the item or the budget changes.
//!
//! [`BlockBounds`]: murmel_mesh::vtkhdf::Overview

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use murmel_mesh::profile::{self, Progress};
use murmel_mesh::vtkhdf::Overview;
use murmel_mesh::{Component, MeshError, NO_CELL};

use crate::derive::Made;
use crate::display::{Display, Item};
use crate::probe_blocks::{Kept, Source};
use crate::surface::{IsoPart, SlicePart, Surface};
use crate::theme::{Rgb, Tokens};

/// How long the slices and iso-surfaces of an overview rest in the window
/// before their blocks are read.
pub const REST: Duration = Duration::from_millis(300);

/// Why a cut has no faces when it was stopped.
pub const STOPPED: &str = "cutting through the blocks was stopped";

/// The bytes a surface takes for each vertex (position, normal, colour,
/// values), index, triangle (its cell and face) and edge vertex.
const VERTEX_BYTES: u64 = 52;
const INDEX_BYTES: u64 = 4;
const TRIANGLE_BYTES: u64 = 8;
const EDGE_BYTES: u64 = 24;

/// What the exact faces of one slice or iso-surface depend on.
#[derive(Clone, Debug, PartialEq)]
pub struct CutKey {
    /// The slice or iso-surface.
    pub item: Item,
    /// The display with this item alone shown and nothing selected
    /// ([`alone`]).
    pub look: Display,
    /// The frame of the surface its faces are written in: its origin and
    /// value origin.
    pub origin: [f64; 3],
    pub value_origin: f64,
}

/// A cut wanted: its key, the fields made on the overview and the
/// colours it is drawn in.
#[derive(Clone, Debug, PartialEq)]
pub struct Want {
    pub key: CutKey,
    pub made: Vec<Made>,
    pub tokens: Tokens,
}

/// The exact faces of a slice or an iso-surface, cut through the blocks.
#[derive(Debug)]
pub struct Cut {
    pub want: Want,
    /// The memory budget it was cut within.
    pub budget: u64,
    /// Its faces, unselected, in the frame of its key; `None` when the
    /// cut did not end (`problem` says why).
    pub faces: Option<Surface>,
    pub problem: Option<MeshError>,
    /// The blocks it went through.
    pub blocks: usize,
}

impl Cut {
    /// Whether this cut is the one `w` wants within `budget`: its key,
    /// fields made and colours are those, and it ended or was cut within
    /// the same budget (a cut that did not end is not made again until
    /// the budget changes).
    pub fn serves(&self, w: &Want, budget: u64) -> bool {
        self.want == *w && (self.faces.is_some() || self.budget == budget)
    }
}

/// The cuts of an overview's slices and iso-surfaces, kept with the
/// display.
#[derive(Debug, Default)]
pub struct Cuts(pub Vec<Arc<Cut>>);

impl PartialEq for Cuts {
    /// The same cuts (not equal faces).
    fn eq(&self, other: &Cuts) -> bool {
        self.0.len() == other.0.len() && self.0.iter().zip(&other.0).all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

/// Whether `i` belongs to the mesh (it hides, isolates or colours cells),
/// not to what is drawn in it or picked.
fn of_mesh(i: &Item) -> bool {
    matches!(
        i,
        Item::Mesh | Item::Zone(_) | Item::Kind(_) | Item::Patch(_)
    )
}

/// `display` with only the enabled slice or iso-surface `item` shown and
/// nothing selected: what its faces are cut with. `None` for another
/// item, or one not enabled.
pub fn alone(display: &Display, item: Item) -> Option<Display> {
    let mut look = Display {
        color_by: display.color_by,
        render_mode: display.render_mode,
        hidden: display
            .hidden
            .iter()
            .filter(|i| of_mesh(i))
            .copied()
            .collect(),
        isolated: display
            .isolated
            .iter()
            .filter(|i| of_mesh(i))
            .copied()
            .collect(),
        colors: display
            .colors
            .iter()
            .filter(|(i, _)| of_mesh(i) || **i == item)
            .map(|(i, c)| (*i, *c))
            .collect(),
        clips: display.clips.clone(),
        field: (display.color_by == crate::display::ColorBy::Field)
            .then(|| display.field.clone())
            .flatten(),
        mesh_look: display.mesh_look,
        ..Display::default()
    };
    match item {
        Item::Slice(k) => {
            let s = *display.slices.get(k as usize).filter(|s| s.enabled)?;
            let off = crate::extract::Slice {
                enabled: false,
                ..s
            };
            look.slices = vec![off; k as usize];
            look.slices.push(s);
        }
        Item::Iso(k) => {
            let s = display.isos.get(k as usize).filter(|s| s.enabled)?.clone();
            let off = crate::extract::Iso {
                enabled: false,
                ..s.clone()
            };
            look.isos = vec![off; k as usize];
            look.isos.push(s);
        }
        _ => return None,
    }
    Some(look)
}

/// The enabled slices, then the enabled iso-surfaces, of `display`, in
/// the order [`crate::extract::append`] draws them.
pub fn items(display: &Display) -> Vec<Item> {
    let slices = (display.slices.iter().enumerate())
        .filter(|(_, s)| s.enabled)
        .map(|(k, _)| Item::Slice(k as u32));
    let isos = (display.isos.iter().enumerate())
        .filter(|(_, s)| s.enabled)
        .map(|(k, _)| Item::Iso(k as u32));
    slices.chain(isos).collect()
}

/// The key of `item` of `display` in a surface with origin `origin` and
/// value origin `value_origin`.
pub fn key_of(
    display: &Display,
    item: Item,
    origin: [f64; 3],
    value_origin: f64,
) -> Option<CutKey> {
    Some(CutKey {
        item,
        look: alone(display, item)?,
        origin,
        value_origin,
    })
}

/// The cuts the shown slices and iso-surfaces of `display` want, in a
/// surface with `origin` and `value_origin`, with the fields of `made`
/// and the colours of `tokens`.
pub fn wanted(
    display: &Display,
    origin: [f64; 3],
    value_origin: f64,
    made: &[Made],
    tokens: &Tokens,
) -> Vec<Want> {
    items(display)
        .into_iter()
        .filter_map(|i| key_of(display, i, origin, value_origin))
        .map(|key| Want {
            key,
            made: made.to_vec(),
            tokens: tokens.clone(),
        })
        .collect()
}

/// The cut of `display` that draws `item` in a surface with `origin` and
/// `value_origin`, ended or not.
pub fn of(display: &Display, item: Item, origin: [f64; 3], value_origin: f64) -> Option<&Cut> {
    let cuts = display.block_cuts.as_deref()?;
    let key = key_of(display, item, origin, value_origin)?;
    cuts.0.iter().map(|c| &**c).find(|c| c.want.key == key)
}

/// Whether the exact faces of `item` take the preview grid's place in
/// surface `s`.
pub(crate) fn covers(display: &Display, item: Item, s: &Surface) -> bool {
    of(display, item, s.origin, s.value_origin).is_some_and(|c| c.faces.is_some())
}

/// The largest side of the blocks' boxes of `o`, times 1e-9: how near a
/// plane or another box counts as touching.
fn tolerance(o: &Overview) -> f64 {
    let span = o
        .blocks
        .iter()
        .filter(|b| !b.bounds.is_empty())
        .flat_map(|b| (0..3).map(move |k| b.bounds.max[k] - b.bounds.min[k]))
        .fold(0.0f64, f64::max);
    1e-9 * span.max(f64::MIN_POSITIVE)
}

/// `a` scaled to unit length, as [`crate::extract`] takes a slice's
/// normal (+z when it has none).
fn unit(a: [f64; 3]) -> [f64; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if l > 0.0 && l.is_finite() {
        a.map(|x| x / l)
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// The corners of a box.
fn corners(lo: [f64; 3], hi: [f64; 3]) -> [[f64; 3]; 8] {
    std::array::from_fn(|i| {
        [
            if i & 1 == 0 { lo[0] } else { hi[0] },
            if i & 2 == 0 { lo[1] } else { hi[1] },
            if i & 4 == 0 { lo[2] } else { hi[2] },
        ]
    })
}

/// The range of field `name` (`component`) over block `b` of `o`, when
/// the file has one: a vector's range is of its magnitude, so a component
/// lies within ± its largest.
fn range_of(o: &Overview, b: usize, name: &str, component: Component) -> Option<[f64; 2]> {
    let f = o.fields.iter().position(|f| f.name == name)?;
    let [lo, hi] = (*o.blocks.get(b)?.ranges.get(f)?)?;
    if o.fields[f].components == 1 || component == Component::Magnitude {
        Some([lo, hi])
    } else {
        let m = lo.abs().max(hi.abs());
        Some([-m, m])
    }
}

/// The blocks of `o` whose owned cells the item of `key` may cut,
/// ascending.
pub fn crossed(o: &Overview, key: &CutKey) -> Vec<usize> {
    let tol = tolerance(o);
    let boxes: Vec<Option<([f64; 3], [f64; 3])>> = o
        .blocks
        .iter()
        .map(|b| (!b.bounds.is_empty()).then_some((b.bounds.min, b.bounds.max)))
        .collect();
    // A block every corner of which an enabled clip plane takes away.
    let gone = |(lo, hi): ([f64; 3], [f64; 3])| {
        key.look
            .active_clips()
            .any(|c| corners(lo, hi).iter().all(|&p| c.distance(p) > tol))
    };
    let live = |b: usize| boxes[b].filter(|&x| !gone(x));
    match key.item {
        Item::Slice(k) => {
            let Some(s) = key.look.slices.get(k as usize) else {
                return Vec::new();
            };
            let n = unit(s.normal);
            let planes = s.plane_origins();
            (0..o.blocks.len())
                .filter(|&b| {
                    live(b).is_some_and(|(lo, hi)| {
                        planes.iter().any(|q| {
                            let d = corners(lo, hi)
                                .map(|p| (0..3).map(|a| (p[a] - q[a]) * n[a]).sum::<f64>());
                            let (dl, dh) = d
                                .iter()
                                .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), &x| {
                                    (l.min(x), h.max(x))
                                });
                            dl <= tol && dh >= -tol
                        })
                    })
                })
                .collect()
        }
        Item::Iso(k) => {
            let Some(iso) = key.look.isos.get(k as usize) else {
                return Vec::new();
            };
            let (name, comp, v) = (&iso.field.name, iso.field.component, iso.value);
            let touch = |a: ([f64; 3], [f64; 3]), b: ([f64; 3], [f64; 3])| {
                (0..3).all(|x| a.0[x] <= b.1[x] + tol && b.0[x] <= a.1[x] + tol)
            };
            let ranges: Vec<Option<[f64; 2]>> = (0..o.blocks.len())
                .map(|b| range_of(o, b, name, comp))
                .collect();
            (0..o.blocks.len())
                .filter(|&b| {
                    live(b).is_some_and(|bx| {
                        // The values of the block's cells and of the
                        // cells beside them: a node's value lies between
                        // those, and a face between its corners'.
                        let mut span = [f64::INFINITY, f64::NEG_INFINITY];
                        let beside = (0..o.blocks.len())
                            .filter(|&c| c == b || boxes[c].is_some_and(|cx| touch(bx, cx)));
                        for c in beside {
                            let Some([lo, hi]) = ranges[c] else {
                                return true;
                            };
                            span = [span[0].min(lo), span[1].max(hi)];
                        }
                        span[0] <= v && v <= span[1]
                    })
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The bytes the faces of `s` take.
fn bytes(s: &Surface) -> u64 {
    s.positions.len() as u64 * VERTEX_BYTES
        + s.indices.len() as u64 * INDEX_BYTES
        + s.tri_cell.len() as u64 * TRIANGLE_BYTES
        + s.edge_positions.len() as u64 * EDGE_BYTES
}

/// Cut the item of `look` through `block` into `faces`: its owned cells
/// shown, its halo hidden. The faces name no cell, as the cells they come
/// from are the block's.
fn cut_block(faces: &mut Surface, block: &Kept, look: &Display, tokens: &Tokens) {
    let mut d = look.clone();
    crate::field::ensure_field(&mut d, &block.mesh, &block.fields);
    let mut shown = d.shown_cells(&block.mesh);
    for (s, &h) in shown.iter_mut().zip(block.subset.halo.iter()) {
        *s &= !h;
    }
    let before = faces.tri_cell.len();
    crate::extract::append(faces, &block.mesh, Some(&block.fields), tokens, &d, &shown);
    faces.tri_cell[before..].fill(NO_CELL);
}

/// Neighbouring parts of the same slice plane, one after the other in
/// the buffers, merged.
fn merged(parts: Vec<SlicePart>) -> Vec<SlicePart> {
    let mut v: Vec<SlicePart> = Vec::with_capacity(parts.len());
    for p in parts {
        match v.last_mut() {
            Some(q)
                if q.slice == p.slice
                    && q.faces.end == p.faces.start
                    && q.edges.end == p.edges.start =>
            {
                q.faces.end = p.faces.end;
                q.edges.end = p.edges.end;
            }
            _ => v.push(p),
        }
    }
    v
}

/// Neighbouring parts of the same iso-surface, one after the other in
/// the buffers, merged.
fn merged_isos(parts: Vec<IsoPart>) -> Vec<IsoPart> {
    let mut v: Vec<IsoPart> = Vec::with_capacity(parts.len());
    for p in parts {
        match v.last_mut() {
            Some(q) if q.iso == p.iso && q.faces.end == p.faces.start => q.faces.end = p.faces.end,
            _ => v.push(p),
        }
    }
    v
}

/// What cutting gives: the cuts, and the blocks it went through last,
/// kept for the next cut.
#[derive(Debug, Default)]
pub struct Outcome {
    pub cuts: Vec<Cut>,
    pub kept: Vec<Arc<Kept>>,
}

/// How many blocks a cut keeps for the next one: as many as an eighth of
/// the memory budget holds (the streamlines' and probes' blocks take
/// their own share).
pub(crate) fn keep_for(src: &Source) -> usize {
    let o = &*src.overview;
    let largest = (0..o.blocks.len())
        .map(|k| crate::blocks::bytes_for(o, &[k]))
        .max()
        .unwrap_or(0)
        .max(1);
    (src.budget / 8 / largest) as usize
}

/// Cut the items of `todo` through the blocks of `src`, taking those of
/// `kept` without reading them; `done` counts the blocks gone through.
/// The last [`keep_for`] blocks gone through are kept.
pub(crate) fn cut(src: &Source, todo: &[Want], kept: &[Arc<Kept>], done: &AtomicUsize) -> Outcome {
    let _t = profile::span("read.cut");
    let o = &*src.overview;
    let plans: Vec<Vec<usize>> = todo.iter().map(|w| crossed(o, &w.key)).collect();
    let blocks: BTreeSet<usize> = plans.iter().flatten().copied().collect();
    let mut faces: Vec<Surface> = todo
        .iter()
        .map(|w| Surface {
            origin: w.key.origin,
            value_origin: w.key.value_origin,
            ..Surface::default()
        })
        .collect();
    let limit = src.budget / 4;
    let keep = keep_for(src);
    let mut used: Vec<Arc<Kept>> = Vec::new();
    let mut problem = None;
    let order: Vec<usize> = blocks.iter().copied().collect();
    let watched = profile::watched();
    std::thread::scope(|scope| {
        // The next blocks are read on a thread of their own while this
        // one cuts (one waiting, one being read), so a file over the
        // network is read while the CPU cuts.
        let (tx, rx) = std::sync::mpsc::sync_channel::<(usize, Result<Arc<Kept>, MeshError>)>(1);
        scope.spawn(move || {
            let read = || {
                for &b in &order {
                    if profile::check().is_err() {
                        return;
                    }
                    let block = crate::probe_blocks::load(src, b, kept);
                    let failed = block.is_err();
                    if tx.send((b, block)).is_err() || failed {
                        return;
                    }
                }
            };
            match watched {
                Some(p) => profile::watch(p, read),
                None => read(),
            }
        });
        for (b, block) in rx.iter() {
            if let Err(e) = profile::check() {
                problem = Some(MeshError::new(e.code, STOPPED));
                break;
            }
            let block = match block {
                Ok(x) => x,
                Err(e) if e.code == "MESH019" => {
                    problem = Some(MeshError::new(e.code, STOPPED));
                    break;
                }
                Err(e) => {
                    problem = Some(e);
                    break;
                }
            };
            let _c = profile::span("read.cut.faces");
            for (i, w) in todo.iter().enumerate() {
                if plans[i].binary_search(&b).is_ok() {
                    cut_block(&mut faces[i], &block, &w.key.look, &w.tokens);
                }
            }
            used.push(block);
            let over = used.len().saturating_sub(keep);
            used.drain(..over);
            done.fetch_add(1, Ordering::Relaxed);
            let taken: u64 = faces.iter().map(bytes).sum();
            if taken > limit {
                problem = Some(MeshError::new(
                    "BLOCK006",
                    format!(
                        "the exact faces of the slices and iso-surfaces would take more than a quarter of the memory budget ({:.1} GB)",
                        src.budget as f64 / 1e9
                    ),
                ));
                break;
            }
        }
        // A cut that ended early stops the reading after its block.
        drop(rx);
    });
    // Stopped before the reading ended: as the loop above saw it.
    if problem.is_none() && profile::check().is_err() {
        problem = Some(MeshError::new("MESH019", STOPPED));
    }
    let cuts = todo
        .iter()
        .zip(faces)
        .zip(plans)
        .map(|((w, mut f), plan)| {
            f.slice_parts = merged(std::mem::take(&mut f.slice_parts));
            f.iso_parts = merged_isos(std::mem::take(&mut f.iso_parts));
            Cut {
                want: w.clone(),
                budget: src.budget,
                faces: problem.is_none().then_some(f),
                problem: problem.clone(),
                blocks: plan.len(),
            }
        })
        .collect();
    Outcome { cuts, kept: used }
}

/// The sRGB colour whose linear colour is `c` ([`Rgb::linear`] turned
/// round: exact for a colour it gave).
fn srgb(c: [f32; 3]) -> Rgb {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    let t = TABLE.get_or_init(|| std::array::from_fn(|i| Rgb([i as u8; 3]).linear()[0]));
    Rgb(c.map(|x| {
        let i = t.partition_point(|&y| y < x).min(255);
        // the nearer of the two table values round x
        if i > 0 && (x - t[i - 1]).abs() < (t[i] - x).abs() {
            (i - 1) as u8
        } else {
            i as u8
        }
    }))
}

/// Append the faces `f` to `s`; when selected, in the colours a selected
/// item takes (the selection colour `sel` mixed in by `mix`, its outline
/// in `sel`).
fn put(s: &mut Surface, f: &Surface, selected: Option<(Rgb, f32)>) {
    let base = s.positions.len() as u32;
    let (i0, e0) = (s.indices.len() as u32, s.edge_positions.len() as u32);
    s.positions.extend_from_slice(&f.positions);
    s.normals.extend_from_slice(&f.normals);
    match selected {
        None => {
            s.colors.extend_from_slice(&f.colors);
            s.values.extend_from_slice(&f.values);
            s.edge_colors.extend_from_slice(&f.edge_colors);
        }
        Some((sel, mix)) => {
            s.colors.extend(
                f.colors
                    .iter()
                    .map(|&c| crate::extract::mixed(srgb(c), sel, mix).linear()),
            );
            s.values
                .extend(f.values.iter().map(|v| [v[0], v[1], mix, v[3]]));
            s.edge_colors
                .extend(std::iter::repeat_n(sel.linear(), f.edge_colors.len()));
        }
    }
    s.indices.extend(f.indices.iter().map(|&i| i + base));
    s.tri_cell.extend_from_slice(&f.tri_cell);
    s.tri_face.extend_from_slice(&f.tri_face);
    s.edge_positions.extend_from_slice(&f.edge_positions);
    s.slice_parts
        .extend(f.slice_parts.iter().map(|p| SlicePart {
            slice: p.slice,
            faces: p.faces.start + i0..p.faces.end + i0,
            edges: p.edges.start + e0..p.edges.end + e0,
        }));
    s.iso_parts.extend(f.iso_parts.iter().map(|p| IsoPart {
        iso: p.iso,
        faces: p.faces.start + i0..p.faces.end + i0,
    }));
    if let Some(b) = f.visible_bounds {
        let mut vb = s.visible_bounds.unwrap_or_else(murmel_mesh::Bounds::empty);
        vb.add(b.min);
        vb.add(b.max);
        s.visible_bounds = Some(vb);
    }
    if let Some((lo, hi)) = f.field_range {
        s.field_range = Some(
            s.field_range
                .map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))),
        );
    }
}

/// Draw the exact faces of the slices and iso-surfaces of `display` cut
/// through the blocks so far into `s`, in the order
/// [`crate::extract::append`] draws them; a selected item takes the
/// selection's colours.
pub(crate) fn append(s: &mut Surface, tokens: &Tokens, display: &Display) {
    if display.block_cuts.is_none() {
        return;
    }
    let sel = (
        tokens.mesh.selection,
        tokens.mesh.selection_mix.clamp(0.0, 1.0),
    );
    for item in items(display) {
        let Some(f) = of(display, item, s.origin, s.value_origin).and_then(|c| c.faces.as_ref())
        else {
            continue;
        };
        let picked = display.selection.contains(&item);
        put(s, f, picked.then_some(sel));
    }
}

/// What a cut on a thread of its own gives.
type Slot = Arc<(Mutex<Option<Outcome>>, Condvar)>;

/// Slices and iso-surfaces cut through the blocks on a thread of their
/// own.
#[derive(Clone, Debug)]
pub struct CutJob {
    /// The cuts being made.
    pub todo: Vec<Want>,
    /// The memory budget they are cut within.
    pub budget: u64,
    pub progress: Arc<Progress>,
    pub started: Instant,
    /// The blocks gone through so far, of `blocks`.
    pub done: Arc<AtomicUsize>,
    pub blocks: usize,
    slot: Slot,
}

impl CutJob {
    /// Start cutting `todo` through the blocks of `src`, taking those of
    /// `kept` without reading them.
    pub(crate) fn start(src: Source, todo: Vec<Want>, kept: Vec<Arc<Kept>>) -> CutJob {
        let slot: Slot = Arc::new((Mutex::new(None), Condvar::new()));
        let progress = Arc::new(Progress::default());
        let done = Arc::new(AtomicUsize::new(0));
        let blocks = {
            let o = &*src.overview;
            let all: BTreeSet<usize> = todo.iter().flat_map(|w| crossed(o, &w.key)).collect();
            all.len()
        };
        let budget = src.budget;
        let (out, watched, count, wants) =
            (slot.clone(), progress.clone(), done.clone(), todo.clone());
        std::thread::spawn(move || {
            let r = profile::watch(watched, || cut(&src, &wants, &kept, &count));
            let (lock, ready) = &*out;
            if let Ok(mut v) = lock.lock() {
                *v = Some(r);
                ready.notify_all();
            }
        });
        CutJob {
            todo,
            budget,
            progress,
            started: Instant::now(),
            done,
            blocks,
            slot,
        }
    }

    pub fn is_done(&self) -> bool {
        self.slot.0.lock().map_or(true, |v| v.is_some())
    }

    /// Ask it to stop at the next block: the items stay on the preview
    /// grid.
    pub fn stop(&self) {
        self.progress.cancel();
    }

    /// The cuts once made, and the blocks kept (waiting for them with
    /// `wait`).
    pub(crate) fn take(&self, wait: bool) -> Option<Outcome> {
        let (lock, ready) = &*self.slot;
        let mut v = lock.lock().ok()?;
        while wait && v.is_none() {
            v = ready.wait(v).ok()?;
        }
        v.take()
    }
}
