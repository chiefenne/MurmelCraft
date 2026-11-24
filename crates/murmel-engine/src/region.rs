//! Regions of interest (large-mesh post-processing, stage 1): shapes and
//! sets of cells that limit what is shown and what the filters compute.
//!
//! A cell belongs to a region when its bounding box touches the shape
//! (Arvo's box-sphere test for a sphere, the overlap of intervals along
//! the normal for a slab; Arvo, "A simple method for box-sphere
//! intersection testing", Graphics Gems, 1990). Every cell with a point
//! inside belongs, so the shape is always fully covered, and so does a
//! cell larger than the shape, such as the one cell across a 2-D mesh
//! extruded one layer thick; the border draws as whole cells. Several
//! enabled regions show their union; with none enabled the whole mesh
//! shows.
//!
//! A box, sphere or slab region can cut flat instead (stage 1b, the
//! maintainer's card 6): its border cells are clipped to the shape and
//! closed by their sections, as a flat clip plane cuts (Sutherland and
//! Hodgman 1974). A box is six half-spaces and a slab two, so their cut
//! is exact, also where the shape is thinner than a cell; a sphere cuts
//! along its signed distance, the crossing on each edge found by linear
//! interpolation, so its cut is the sphere to within a cell; a sphere
//! that holds no node of its cells (smaller than them, between their
//! nodes) shows whole cells instead, as no edge would cross it. Each
//! cell is cut by one region: a cell a whole-cell region takes stays
//! whole, and where flat regions overlap a cell is cut by the one its
//! centre lies deepest in.
//!
//! The halo of a set of cells is the cells outside it that share a point
//! with it. On a mesh read in pieces the halo is loaded but never drawn,
//! so node values (inverse distance over the cells around a point) and
//! face interpolation at the border equal those of the whole mesh. On a
//! mesh in memory it only enters the estimates.

use std::sync::Arc;

use murmel_mesh::{Bounds, Mesh, NO_CELL, par};
use serde::{Deserialize, Serialize};

use crate::clip::ClipMode;

/// Most regions one mesh keeps.
pub const MAX_REGIONS: usize = 16;
/// Most rings of neighbours a cell region grows by, and most cell layers
/// along a patch.
pub const MAX_RINGS: u32 = 64;
/// Most cells a cell region names by id.
pub const MAX_CELL_IDS: usize = 100_000;

/// The shape of a region.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    /// An axis-aligned box: its centre and its full size along x, y, z.
    Box {
        center: [f64; 3],
        size: [f64; 3],
    },
    Sphere {
        center: [f64; 3],
        radius: f64,
    },
    /// The cells between two parallel planes `thickness` apart, the
    /// middle plane through `origin` with unit normal `normal`.
    Slab {
        origin: [f64; 3],
        normal: [f64; 3],
        thickness: f64,
    },
    /// The cells of a cell zone, by name.
    Zone {
        zone: String,
    },
    /// The cells along a patch (the owners of its faces), and `layers - 1`
    /// rings of face neighbours further in.
    Patch {
        patch: String,
        layers: u32,
    },
    /// Cells by id, grown by `rings` rings of face neighbours.
    Cells {
        cells: Vec<u32>,
        rings: u32,
    },
}

impl Shape {
    pub const KINDS: [&'static str; 6] = ["box", "sphere", "slab", "zone", "patch", "cells"];

    pub fn kind(&self) -> &'static str {
        match self {
            Shape::Box { .. } => "box",
            Shape::Sphere { .. } => "sphere",
            Shape::Slab { .. } => "slab",
            Shape::Zone { .. } => "zone",
            Shape::Patch { .. } => "patch",
            Shape::Cells { .. } => "cells",
        }
    }

    /// A box, sphere or slab: a shape in space rather than a set of cells.
    pub fn is_geometric(&self) -> bool {
        matches!(
            self,
            Shape::Box { .. } | Shape::Sphere { .. } | Shape::Slab { .. }
        )
    }

    /// Whether point `p` is inside a box, sphere or slab (borders
    /// included); always false for the other shapes.
    pub fn contains(&self, p: [f64; 3]) -> bool {
        match self {
            Shape::Box { center, size } => {
                (0..3).all(|k| (p[k] - center[k]).abs() <= 0.5 * size[k])
            }
            Shape::Sphere { center, radius } => {
                (0..3).map(|k| (p[k] - center[k]).powi(2)).sum::<f64>() <= radius * radius
            }
            Shape::Slab {
                origin,
                normal,
                thickness,
            } => {
                let d: f64 = (0..3).map(|k| (p[k] - origin[k]) * normal[k]).sum();
                d.abs() <= 0.5 * thickness
            }
            _ => false,
        }
    }

    /// Whether the box from `lo` to `hi` touches the shape (geometric
    /// shapes; false for the sets of cells).
    pub fn touches_box(&self, lo: [f64; 3], hi: [f64; 3]) -> bool {
        match self {
            Shape::Box { center, size } => (0..3)
                .all(|k| lo[k] <= center[k] + 0.5 * size[k] && hi[k] >= center[k] - 0.5 * size[k]),
            Shape::Sphere { center, radius } => {
                let d2: f64 = (0..3)
                    .map(|k| {
                        let d = (lo[k] - center[k]).max(0.0) + (center[k] - hi[k]).max(0.0);
                        d * d
                    })
                    .sum();
                d2 <= radius * radius
            }
            Shape::Slab {
                origin,
                normal,
                thickness,
            } => {
                let (mut d, mut r) = (0.0, 0.0);
                for k in 0..3 {
                    d += (0.5 * (lo[k] + hi[k]) - origin[k]) * normal[k];
                    r += 0.5 * (hi[k] - lo[k]) * normal[k].abs();
                }
                d.abs() <= r + 0.5 * thickness
            }
            _ => false,
        }
    }

    /// Whether the shape can cut flat (box, sphere, slab).
    pub fn cuts_flat(&self) -> bool {
        self.is_geometric()
    }

    /// The borders a flat cut clips to: a box's six faces, a slab's two
    /// planes, a sphere; none for the sets of cells.
    pub fn borders(&self) -> Vec<Bound> {
        match self {
            Shape::Box { center, size } => {
                let mut v = Vec::with_capacity(6);
                for k in 0..3 {
                    for s in [1.0, -1.0] {
                        let mut normal = [0.0; 3];
                        normal[k] = s;
                        let mut origin = *center;
                        origin[k] += s * 0.5 * size[k];
                        v.push(Bound::Plane { origin, normal });
                    }
                }
                v
            }
            Shape::Sphere { center, radius } => vec![Bound::Sphere {
                center: *center,
                radius: *radius,
            }],
            Shape::Slab {
                origin,
                normal,
                thickness,
            } => [1.0, -1.0]
                .into_iter()
                .map(|s| Bound::Plane {
                    origin: [0, 1, 2].map(|k| origin[k] + s * 0.5 * thickness * normal[k]),
                    normal: normal.map(|n| s * n),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// How deep `p` lies in the shape: negative inside, the deeper the
    /// smaller (the largest of the borders' distances; for a sphere its
    /// signed distance). Infinite for the sets of cells.
    pub fn depth(&self, p: [f64; 3]) -> f64 {
        depth(&self.borders(), p)
    }

    /// The box of a box or sphere; `None` for a slab and the sets of
    /// cells, which have no box of their own.
    pub fn bounds(&self) -> Option<Bounds> {
        match self {
            Shape::Box { center, size } => Some(Bounds {
                min: [0, 1, 2].map(|k| center[k] - 0.5 * size[k]),
                max: [0, 1, 2].map(|k| center[k] + 0.5 * size[k]),
            }),
            Shape::Sphere { center, radius } => Some(Bounds {
                min: center.map(|c| c - radius),
                max: center.map(|c| c + radius),
            }),
            _ => None,
        }
    }

    /// The default box of a new region: centred in the mesh's box, half
    /// its size.
    pub fn default_box(b: &Bounds) -> Shape {
        Shape::Box {
            center: b.center(),
            size: extent(b).map(|e| 0.5 * e),
        }
    }

    /// The default sphere: centred in the mesh's box, a quarter of its
    /// largest extent in radius.
    pub fn default_sphere(b: &Bounds) -> Shape {
        Shape::Sphere {
            center: b.center(),
            radius: 0.25 * largest(b),
        }
    }

    /// The default slab: through the centre, normal to x, a tenth of the
    /// largest extent thick.
    pub fn default_slab(b: &Bounds) -> Shape {
        Shape::Slab {
            origin: b.center(),
            normal: [1.0, 0.0, 0.0],
            thickness: 0.1 * largest(b),
        }
    }

    /// Why the shape cannot be used on `mesh`, if it cannot: what
    /// [`Shape::problem_alone`] finds, or a zone or patch the mesh does
    /// not have, or a cell id out of range.
    pub fn problem(&self, mesh: &Mesh) -> Option<String> {
        if let Some(p) = self.problem_alone() {
            return Some(p);
        }
        match self {
            Shape::Zone { zone } if !mesh.zones.iter().any(|z| z == zone) => {
                Some(format!("the mesh has no zone {zone}"))
            }
            Shape::Patch { patch, .. } if !mesh.patches.iter().any(|p| &p.name == patch) => {
                Some(format!("the mesh has no patch {patch}"))
            }
            Shape::Cells { cells, .. } => cells
                .iter()
                .find(|&&c| c as usize >= mesh.cell_count())
                .map(|c| {
                    format!(
                        "cell {c} is out of range; the mesh has {} cells",
                        mesh.cell_count()
                    )
                }),
            _ => None,
        }
    }

    /// Why the shape is broken whatever the mesh, if it is: numbers that
    /// are not finite, sizes that are not positive, a normal that is not a
    /// unit vector, too many rings or layers, no cells or too many.
    pub fn problem_alone(&self) -> Option<String> {
        let finite = |v: &[f64]| v.iter().all(|x| x.is_finite());
        match self {
            Shape::Box { center, size } => {
                if !finite(center) || !finite(size) {
                    return Some("a box needs a finite centre and size".into());
                }
                if size.iter().any(|&s| s <= 0.0) {
                    return Some(format!("a box needs a positive size, not {size:?}"));
                }
            }
            Shape::Sphere { center, radius } => {
                if !finite(center) || !(radius.is_finite() && *radius > 0.0) {
                    return Some("a sphere needs a finite centre and a positive radius".into());
                }
            }
            Shape::Slab {
                origin,
                normal,
                thickness,
            } => {
                if !finite(origin) || !(thickness.is_finite() && *thickness > 0.0) {
                    return Some("a slab needs a finite origin and a positive thickness".into());
                }
                let n: f64 = normal.iter().map(|v| v * v).sum::<f64>().sqrt();
                if !(n.is_finite() && (n - 1.0).abs() < 1e-6) {
                    return Some("a slab needs a unit normal".into());
                }
            }
            Shape::Zone { zone } => {
                if zone.is_empty() {
                    return Some("a zone region needs a zone name".into());
                }
            }
            Shape::Patch { patch, layers } => {
                if patch.is_empty() {
                    return Some("a patch region needs a patch name".into());
                }
                if !(1..=MAX_RINGS).contains(layers) {
                    return Some(format!("layers {layers} is outside 1 to {MAX_RINGS}"));
                }
            }
            Shape::Cells { cells, rings } => {
                if cells.is_empty() {
                    return Some("a cell region needs at least one cell".into());
                }
                if cells.len() > MAX_CELL_IDS {
                    return Some(format!("at most {MAX_CELL_IDS} cells by id"));
                }
                if *rings > MAX_RINGS {
                    return Some(format!("rings {rings} is more than {MAX_RINGS}"));
                }
            }
        }
        None
    }

    /// The cells of the shape on `mesh`.
    pub fn cells(&self, mesh: &Mesh) -> Vec<bool> {
        let n = mesh.cell_count();
        match self {
            Shape::Box { .. } | Shape::Sphere { .. } | Shape::Slab { .. } => {
                par::collect(n, 1 << 14, |r| {
                    r.map(|c| {
                        let (lo, hi) = cell_box(mesh, c);
                        self.touches_box(lo, hi)
                    })
                    .collect()
                })
            }
            Shape::Zone { zone } => match mesh.zones.iter().position(|z| z == zone) {
                Some(z) => mesh.cell_zone.iter().map(|&c| c == z as u32).collect(),
                None => vec![false; n],
            },
            Shape::Patch { patch, layers } => {
                let mut mask = vec![false; n];
                if let Some(p) = mesh.patches.iter().position(|p| &p.name == patch) {
                    for (f, &q) in mesh.boundary_patch.iter().enumerate() {
                        let c = mesh.boundary_owner.get(f).copied().unwrap_or(NO_CELL);
                        if q == p as u32 && c != NO_CELL && (c as usize) < n {
                            mask[c as usize] = true;
                        }
                    }
                    grow(mesh, &mut mask, layers.saturating_sub(1));
                }
                mask
            }
            Shape::Cells { cells, rings } => {
                let mut mask = vec![false; n];
                for &c in cells {
                    if (c as usize) < n {
                        mask[c as usize] = true;
                    }
                }
                grow(mesh, &mut mask, *rings);
                mask
            }
        }
    }
}

/// A border of a flat region: a plane whose normal points to the side
/// cut away, or a sphere (the outside cut away).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bound {
    Plane { origin: [f64; 3], normal: [f64; 3] },
    Sphere { center: [f64; 3], radius: f64 },
}

impl Bound {
    /// Signed distance of `p` (positive: cut away).
    pub fn distance(&self, p: [f64; 3]) -> f64 {
        match self {
            Bound::Plane { origin, normal } => (0..3).map(|k| (p[k] - origin[k]) * normal[k]).sum(),
            Bound::Sphere { center, radius } => {
                (0..3)
                    .map(|k| (p[k] - center[k]).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    - radius
            }
        }
    }
}

impl From<&crate::Clip> for Bound {
    fn from(c: &crate::Clip) -> Self {
        Bound::Plane {
            origin: c.origin,
            normal: c.normal,
        }
    }
}

/// A region of interest: a shape, on or off (the tree's eye), cutting
/// whole cells (crinkle, the default) or flat along its border.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub shape: Shape,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_crinkle")]
    pub cut: ClipMode,
}

fn is_crinkle(m: &ClipMode) -> bool {
    *m == ClipMode::Crinkle
}

impl Region {
    pub fn new(shape: Shape) -> Self {
        Self {
            shape,
            enabled: true,
            cut: ClipMode::Crinkle,
        }
    }

    /// Whether the region is on and cuts flat.
    pub fn cuts_flat(&self) -> bool {
        self.enabled && self.cut == ClipMode::Flat && self.shape.cuts_flat()
    }

    /// What is wrong with the region on its own: its shape's problem, or
    /// a flat cut on a set of cells.
    pub fn problem_alone(&self) -> Option<String> {
        if self.cut == ClipMode::Flat && !self.shape.cuts_flat() {
            return Some(format!(
                "a {} region has no border to cut flat; only box, sphere and slab regions cut flat",
                self.shape.kind()
            ));
        }
        self.shape.problem_alone()
    }
}

/// The regions of a display as found on the mesh, kept with the regions
/// they were found for (`Display::region_cells`): what each would cost
/// to load, and the cells of the enabled ones.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionCells {
    pub regions: Vec<Region>,
    /// By region, in order.
    pub estimates: Vec<Estimate>,
    /// By cell: in one of the enabled regions; `None` while none is
    /// enabled (the whole mesh shows).
    pub cells: Option<Vec<bool>>,
    /// The union of the enabled regions, with its halo.
    pub union: Estimate,
    /// By cell: the region that cuts it flat, or [`NOT_CUT`]; `None`
    /// while no enabled region cuts flat.
    pub cut_by: Option<Vec<u8>>,
    /// By region: the borders a flat region cuts along (empty for the
    /// others).
    pub borders: Vec<Vec<Bound>>,
    /// By region: a flat sphere that holds no node of the cells it takes
    /// (it lies between the nodes), which shows whole cells instead,
    /// since a cut along its distance would find no crossing.
    pub whole_instead: Vec<bool>,
}

/// A cell no region cuts flat.
pub const NOT_CUT: u8 = u8::MAX;

impl RegionCells {
    /// The regions on `mesh`; `None` when there are none.
    pub fn of(mesh: &Mesh, regions: &[Region]) -> Option<RegionCells> {
        Self::of_on(mesh, regions, None)
    }

    /// As [`RegionCells::of`] on a mesh read in pieces: no region takes a
    /// cell of `halo` (by cell; the halo of an overview's blocks).
    pub fn of_on(mesh: &Mesh, regions: &[Region], halo: Option<&[bool]>) -> Option<RegionCells> {
        if regions.is_empty() {
            return None;
        }
        let mut estimates = Vec::with_capacity(regions.len());
        let mut cells: Option<Vec<bool>> = None;
        // The cells whole-cell regions take, and each flat region's.
        let mut whole: Option<Vec<bool>> = None;
        let mut flat: Vec<(usize, Vec<bool>)> = Vec::new();
        let mut whole_instead = vec![false; regions.len()];
        for (k, r) in regions.iter().enumerate() {
            let mut own = r.shape.cells(mesh);
            if let Some(h) = halo {
                for (o, &h) in own.iter_mut().zip(h) {
                    *o &= !h;
                }
            }
            estimates.push(Estimate::of(mesh, &own));
            if !r.enabled {
                continue;
            }
            match &mut cells {
                Some(all) => or_into(all, &own),
                None => cells = Some(own.clone()),
            }
            whole_instead[k] = r.cuts_flat()
                && matches!(r.shape, Shape::Sphere { .. })
                && !(0..own.len()).any(|c| {
                    own[c]
                        && mesh
                            .cell_points(c)
                            .any(|p| r.shape.contains(mesh.points[p as usize]))
                });
            if r.cuts_flat() && !whole_instead[k] {
                flat.push((k, own));
            } else {
                match &mut whole {
                    Some(all) => or_into(all, &own),
                    None => whole = Some(own),
                }
            }
        }
        let union = match &cells {
            Some(c) => Estimate::of(mesh, c),
            None => Estimate::default(),
        };
        let cut_by = (!flat.is_empty()).then(|| {
            let deep: Vec<Vec<Bound>> = flat
                .iter()
                .map(|(k, _)| regions[*k].shape.borders())
                .collect();
            par::collect(mesh.cell_count(), 1 << 14, |range| {
                range
                    .map(|c| {
                        if whole.as_ref().is_some_and(|w| w[c]) {
                            return NOT_CUT;
                        }
                        let mut takes = flat.iter().enumerate().filter(|(_, (_, own))| own[c]);
                        let Some((i, (k, _))) = takes.next() else {
                            return NOT_CUT;
                        };
                        let Some(next) = takes.next() else {
                            return *k as u8;
                        };
                        // Overlapping flat regions: the one the centre
                        // lies deepest in.
                        let p = cell_centre(mesh, c);
                        [(i, *k), (next.0, next.1.0)]
                            .into_iter()
                            .chain(takes.map(|(i, (k, _))| (i, *k)))
                            .map(|(i, k)| (depth(&deep[i], p), k))
                            .fold((f64::INFINITY, NOT_CUT), |best, (d, k)| {
                                if d < best.0 || best.1 == NOT_CUT {
                                    (d, k as u8)
                                } else {
                                    best
                                }
                            })
                            .1
                    })
                    .collect()
            })
        });
        let borders = regions
            .iter()
            .zip(&whole_instead)
            .map(|(r, &w)| {
                if r.cuts_flat() && !w {
                    r.shape.borders()
                } else {
                    Vec::new()
                }
            })
            .collect();
        Some(RegionCells {
            regions: regions.to_vec(),
            estimates,
            cells,
            union,
            cut_by,
            borders,
            whole_instead,
        })
    }

    /// The borders cell `c` is cut along (none when no region cuts it
    /// flat).
    pub fn borders_of(&self, c: usize) -> &[Bound] {
        match self.cut_by.as_ref().and_then(|v| v.get(c)) {
            Some(&k) if k != NOT_CUT => &self.borders[k as usize],
            _ => &[],
        }
    }

    /// Whether an enabled region cuts flat.
    pub fn cuts_flat(&self) -> bool {
        self.cut_by.is_some()
    }

    /// Whether cell `c` shows: in an enabled region, or none is enabled.
    pub fn contains(&self, c: usize) -> bool {
        self.cells
            .as_ref()
            .is_none_or(|cells| cells.get(c).copied().unwrap_or(false))
    }

    /// Whether the regions limit what shows (one is enabled).
    pub fn filtering(&self) -> bool {
        self.cells.is_some()
    }
}

/// Find the regions' cells when the regions changed since they were last
/// found (`Session::redraw` calls it before the surface is built).
pub fn ensure(display: &mut crate::Display, mesh: &Mesh) {
    ensure_on(display, mesh, None);
}

/// As [`ensure`] on a mesh read in pieces, whose `halo` cells no region
/// takes.
pub fn ensure_on(display: &mut crate::Display, mesh: &Mesh, halo: Option<&[bool]>) {
    let fresh = match &display.region_cells {
        Some(rc) => rc.regions == display.regions,
        None => display.regions.is_empty(),
    };
    if !fresh {
        display.region_cells = RegionCells::of_on(mesh, &display.regions, halo).map(Arc::new);
    }
}

/// What loading a set of cells costs: the cells, their halo, the points
/// they use, and the bytes of the mesh model those take (from the open
/// mesh's bytes per cell).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Estimate {
    pub cells: usize,
    pub halo: usize,
    pub points: usize,
    pub bytes: u64,
}

impl Estimate {
    pub fn of(mesh: &Mesh, cells: &[bool]) -> Estimate {
        let halo = halo(mesh, cells);
        let mut used = vec![false; mesh.points.len()];
        let (mut inside, mut around) = (0, 0);
        for c in 0..mesh.cell_count() {
            let (a, b) = (cells[c], halo[c]);
            inside += a as usize;
            around += b as usize;
            if a || b {
                for p in mesh.cell_points(c) {
                    used[p as usize] = true;
                }
            }
        }
        let points = used.iter().filter(|&&u| u).count();
        let per_cell = mesh.model_bytes() as f64 / mesh.cell_count().max(1) as f64;
        Estimate {
            cells: inside,
            halo: around,
            points,
            bytes: ((inside + around) as f64 * per_cell).round() as u64,
        }
    }
}

/// The cells with one of their points inside (`inside` by point).
pub fn touching(mesh: &Mesh, inside: &[bool]) -> Vec<bool> {
    par::collect(mesh.cell_count(), 1 << 14, |r| {
        r.map(|c| mesh.cell_points(c).any(|p| inside[p as usize]))
            .collect()
    })
}

/// The halo of `cells`: the cells outside them that share a point with
/// one of them.
pub fn halo(mesh: &Mesh, cells: &[bool]) -> Vec<bool> {
    let mut marked = vec![false; mesh.points.len()];
    for c in (0..mesh.cell_count()).filter(|&c| cells[c]) {
        for p in mesh.cell_points(c) {
            marked[p as usize] = true;
        }
    }
    let near = touching(mesh, &marked);
    near.into_iter().zip(cells).map(|(n, &c)| n && !c).collect()
}

/// Add `rings` rings of face neighbours to `mask`.
pub fn grow(mesh: &Mesh, mask: &mut [bool], rings: u32) {
    let mut front: Vec<u32> = (0..mask.len() as u32)
        .filter(|&c| mask[c as usize])
        .collect();
    for _ in 0..rings {
        let mut next = Vec::new();
        for &c in &front {
            for f in mesh.cell(c as usize) {
                let o = mesh.across(f.id as usize, c);
                if o != NO_CELL && !mask[o as usize] {
                    mask[o as usize] = true;
                    next.push(o);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        front = next;
    }
}

/// The largest of the borders' distances of `p` (negative inside all).
fn depth(borders: &[Bound], p: [f64; 3]) -> f64 {
    if borders.is_empty() {
        return f64::INFINITY;
    }
    borders
        .iter()
        .map(|b| b.distance(p))
        .fold(f64::NEG_INFINITY, f64::max)
}

fn or_into(all: &mut [bool], own: &[bool]) {
    for (a, b) in all.iter_mut().zip(own) {
        *a |= *b;
    }
}

/// The mean of cell `c`'s points.
fn cell_centre(mesh: &Mesh, c: usize) -> [f64; 3] {
    let mut sum = [0.0; 3];
    let mut n: f64 = 0.0;
    for p in mesh.cell_points(c) {
        let q = mesh.points[p as usize];
        for k in 0..3 {
            sum[k] += q[k];
        }
        n += 1.0;
    }
    sum.map(|x| x / n.max(1.0))
}

/// The bounding box of cell `c`.
fn cell_box(mesh: &Mesh, c: usize) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in mesh.cell_points(c) {
        let q = mesh.points[p as usize];
        for k in 0..3 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    (lo, hi)
}

/// The bounds of the cells in `cells`.
pub fn cells_bounds(mesh: &Mesh, cells: &[bool]) -> Option<Bounds> {
    let mut b = Bounds::empty();
    for c in (0..mesh.cell_count()).filter(|&c| cells[c]) {
        for p in mesh.cell_points(c) {
            b.add(mesh.points[p as usize]);
        }
    }
    (!b.is_empty()).then_some(b)
}

fn extent(b: &Bounds) -> [f64; 3] {
    if b.is_empty() {
        return [1.0; 3];
    }
    [0, 1, 2].map(|k| (b.max[k] - b.min[k]).max(f64::MIN_POSITIVE))
}

fn largest(b: &Bounds) -> f64 {
    extent(b)
        .into_iter()
        .fold(0.0, f64::max)
        .max(f64::MIN_POSITIVE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use murmel_mesh::synth;

    fn honeycomb() -> Mesh {
        synth::honeycomb(6)
    }

    /// Invariant: a geometric region holds exactly the cells with a point
    /// inside the shape (as a crinkle clip keeps cells), so the shape is
    /// covered and nothing outside its reach is taken.
    #[test]
    fn shapes_take_the_cells_they_touch() {
        let m = honeycomb();
        let b = m.bounds();
        for shape in [
            Shape::default_box(&b),
            Shape::default_sphere(&b),
            Shape::default_slab(&b),
        ] {
            assert!(shape.problem(&m).is_none(), "{shape:?}");
            let cells = shape.cells(&m);
            for (c, &inside) in cells.iter().enumerate() {
                let (lo, hi) = cell_box(&m, c);
                assert_eq!(
                    inside,
                    shape.touches_box(lo, hi),
                    "{} cell {c}",
                    shape.kind()
                );
                if m.cell_points(c)
                    .any(|p| shape.contains(m.points[p as usize]))
                {
                    assert!(inside, "{} cell {c} has a point inside", shape.kind());
                }
            }
            let n = cells.iter().filter(|&&x| x).count();
            assert!(n > 0 && n < m.cell_count(), "{} takes {n}", shape.kind());
        }
    }

    /// Invariant: a shape smaller than a cell, with no point of the mesh
    /// inside, still takes the cells it lies in (the thin box across a
    /// 2-D mesh one cell thick took nothing when points decided).
    #[test]
    fn a_shape_inside_one_cell_takes_it() {
        let m = honeycomb();
        let c = m.cell_count() / 2;
        let (lo, hi) = cell_box(&m, c);
        let mid = [0, 1, 2].map(|k| 0.5 * (lo[k] + hi[k]));
        let tiny = [0, 1, 2].map(|k| 1e-3 * (hi[k] - lo[k]));
        for shape in [
            Shape::Box {
                center: mid,
                size: tiny,
            },
            Shape::Sphere {
                center: mid,
                radius: tiny[0],
            },
        ] {
            assert!(
                !m.points.iter().any(|&p| shape.contains(p)),
                "{} holds no point",
                shape.kind()
            );
            assert!(shape.cells(&m)[c], "{} takes cell {c}", shape.kind());
        }
        // A slab between the cell's bottom and top points.
        let slab = Shape::Slab {
            origin: mid,
            normal: [0.0, 0.0, 1.0],
            thickness: tiny[2],
        };
        assert!(!slab.contains(lo) && !slab.contains(hi));
        assert!(slab.touches_box(lo, hi));
        assert!(slab.cells(&m)[c]);
    }

    /// Invariant (stage 1b): a flat sphere inside one cell, with no node
    /// of the mesh inside it, shows that cell whole instead of cutting it
    /// to nothing; a flat box there cuts it exactly (its six planes cross
    /// the cell's edges). Breaks when a small flat sphere makes its
    /// region vanish.
    #[test]
    fn a_flat_sphere_between_nodes_shows_whole_cells() {
        let m = honeycomb();
        let c = m.cell_count() / 2;
        let (lo, hi) = cell_box(&m, c);
        let mid = [0, 1, 2].map(|k| 0.5 * (lo[k] + hi[k]));
        let small = 1e-3 * (hi[0] - lo[0]);
        let flat = |shape: Shape| Region {
            cut: ClipMode::Flat,
            ..Region::new(shape)
        };
        let sphere = flat(Shape::Sphere {
            center: mid,
            radius: small,
        });
        let rc = RegionCells::of(&m, &[sphere]).unwrap();
        assert_eq!(rc.whole_instead, [true]);
        assert!(rc.contains(c));
        assert!(rc.borders_of(c).is_empty(), "shown whole");
        let cube = flat(Shape::Box {
            center: mid,
            size: [small; 3],
        });
        let rc = RegionCells::of(&m, &[cube]).unwrap();
        assert_eq!(rc.whole_instead, [false]);
        assert_eq!(rc.borders_of(c).len(), 6, "cut by the box's faces");
    }

    /// Invariant: the halo touches the cells and never overlaps them;
    /// every cell sharing a point with the set is in one or the other.
    #[test]
    fn halo_is_the_point_neighbours_outside() {
        let m = honeycomb();
        let cells = Shape::default_sphere(&m.bounds()).cells(&m);
        let h = halo(&m, &cells);
        let mut marked = vec![false; m.points.len()];
        for c in (0..m.cell_count()).filter(|&c| cells[c]) {
            for p in m.cell_points(c) {
                marked[p as usize] = true;
            }
        }
        for c in 0..m.cell_count() {
            let near = m.cell_points(c).any(|p| marked[p as usize]);
            assert_eq!(h[c], near && !cells[c], "cell {c}");
        }
        assert!(h.iter().any(|&x| x));
        let e = Estimate::of(&m, &cells);
        assert_eq!(e.cells, cells.iter().filter(|&&x| x).count());
        assert_eq!(e.halo, h.iter().filter(|&&x| x).count());
        assert!(e.points > 0 && e.bytes > 0);
    }

    /// Invariant: a cell region grows by face neighbours one ring at a
    /// time, and a patch region starts at the owners of the patch's faces.
    #[test]
    fn rings_and_layers_grow_by_faces() {
        let m = honeycomb();
        let one = Shape::Cells {
            cells: vec![0],
            rings: 0,
        }
        .cells(&m);
        assert_eq!(one.iter().filter(|&&x| x).count(), 1);
        let ring = Shape::Cells {
            cells: vec![0],
            rings: 1,
        }
        .cells(&m);
        let neighbours: Vec<u32> = m
            .cell(0)
            .map(|f| m.across(f.id as usize, 0))
            .filter(|&o| o != NO_CELL)
            .collect();
        assert_eq!(
            ring.iter().filter(|&&x| x).count(),
            1 + neighbours.len(),
            "cell 0 and its face neighbours"
        );
        let patch = m.patches[0].name.clone();
        let l1 = Shape::Patch {
            patch: patch.clone(),
            layers: 1,
        }
        .cells(&m);
        let l2 = Shape::Patch { patch, layers: 2 }.cells(&m);
        let (n1, n2) = (
            l1.iter().filter(|&&x| x).count(),
            l2.iter().filter(|&&x| x).count(),
        );
        assert!(n1 > 0 && n2 > n1);
        assert!(l1.iter().zip(&l2).all(|(a, b)| !a || *b));
    }

    /// Invariant: enabled regions show their union; with none enabled
    /// nothing is filtered, and every region keeps its own estimate.
    #[test]
    fn enabled_regions_show_their_union() {
        let m = honeycomb();
        let b = m.bounds();
        let a = Region::new(Shape::default_sphere(&b));
        let mut s = Region::new(Shape::Zone {
            zone: m.zones[0].clone(),
        });
        let rc = RegionCells::of(&m, &[a.clone(), s.clone()]).unwrap();
        let (ca, cs) = (a.shape.cells(&m), s.shape.cells(&m));
        for c in 0..m.cell_count() {
            assert_eq!(rc.contains(c), ca[c] || cs[c]);
        }
        assert_eq!(rc.estimates.len(), 2);
        assert_eq!(rc.estimates[0], Estimate::of(&m, &ca));
        assert!(rc.union.cells >= rc.estimates[0].cells.max(rc.estimates[1].cells));
        s.enabled = false;
        let only = RegionCells::of(&m, &[a.clone(), s.clone()]).unwrap();
        assert_eq!(only.cells.as_deref(), Some(ca.as_slice()));
        let mut off = a;
        off.enabled = false;
        let none = RegionCells::of(&m, &[off, s]).unwrap();
        assert!(!none.filtering() && (0..m.cell_count()).all(|c| none.contains(c)));
        assert!(RegionCells::of(&m, &[]).is_none());
    }

    /// Invariant: on a mesh read in pieces no region takes a cell of the
    /// halo, a shape (a sphere over all of it) as little as a zone, and
    /// the cells it does take are those it takes on the whole mesh. A box
    /// region on an overview never reaches a halo cell (its blocks would
    /// be read), but a zone region a session file brings does, so the
    /// session's test cannot catch a halo that shows; dropping the mask
    /// in `of_on` breaks this one.
    #[test]
    fn regions_leave_out_the_halo() {
        let m = honeycomb();
        let b = m.bounds();
        let halo: Vec<bool> = (0..m.cell_count()).map(|c| c % 3 == 0).collect();
        let regions = [
            Region::new(Shape::Sphere {
                center: b.center(),
                radius: 2.0 * b.radius(),
            }),
            Region::new(Shape::Zone {
                zone: m.zones[0].clone(),
            }),
        ];
        let whole = RegionCells::of(&m, &regions).unwrap();
        let on = RegionCells::of_on(&m, &regions, Some(&halo)).unwrap();
        for (c, &h) in halo.iter().enumerate() {
            assert_eq!(on.contains(c), whole.contains(c) && !h, "cell {c}");
        }
        assert!((0..m.cell_count()).any(|c| on.contains(c)));
        assert!(on.estimates[0].cells < whole.estimates[0].cells);
    }

    #[test]
    fn problems_are_named() {
        let m = honeycomb();
        let bad = [
            Shape::Box {
                center: [0.0; 3],
                size: [1.0, 0.0, 1.0],
            },
            Shape::Sphere {
                center: [f64::NAN, 0.0, 0.0],
                radius: 1.0,
            },
            Shape::Slab {
                origin: [0.0; 3],
                normal: [0.0; 3],
                thickness: 1.0,
            },
            Shape::Zone {
                zone: "nowhere".into(),
            },
            Shape::Patch {
                patch: "nowhere".into(),
                layers: 1,
            },
            Shape::Cells {
                cells: vec![u32::MAX],
                rings: 0,
            },
        ];
        for s in bad {
            assert!(s.problem(&m).is_some(), "{s:?}");
        }
    }
}
