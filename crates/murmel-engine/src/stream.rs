//! Streamlines (field results, stage 4; the maintainer's cards of
//! 2026-11-24).
//!
//! - **Seeds.** A streamline starts at each seed of a seed shape: evenly
//!   along a line (its two ends included), spread through a ball (its
//!   centre, then Halton points: J. H. Halton, "On the efficiency of
//!   certain quasi-random sequences of points in evaluating
//!   multi-dimensional integrals", Numerische Mathematik 2, 1960), on a
//!   boundary patch (its faces thinned to one per cube of an even grid,
//!   the face nearest the cube's centre, the cube's side set so about
//!   `count` faces remain) or on an even square grid of a plane through
//!   the mesh (anchored at the plane's origin, its spacing set so about
//!   `count` points fall inside the mesh). Seeds outside the mesh are left
//!   out.
//! - **Tracing.** The vector field inside a cell is the piecewise linear
//!   field on the cell's tetrahedra ([`crate::tets`]), the one slices
//!   and iso-surfaces show. A streamline follows the field's direction
//!   with the classical fourth-order Runge–Kutta method in arc length
//!   (E. Hairer, S. P. Nørsett and G. Wanner, "Solving Ordinary
//!   Differential Equations I", section II.1), a quarter of the cell's
//!   size per step, forward, backward or both ways (the default) from
//!   its seed. A point is found by walking from the cell of the last
//!   one towards it, face by face (the face its plane crosses first on
//!   the way from the cell's centroid); a seed starts the walk from the
//!   cell with the nearest centroid, found on a bucket grid. It stops
//!   when it leaves the mesh (or the regions of interest shown), where
//!   the speed falls below 0.1 % of the field's largest, at the length
//!   limit (three mesh diagonals unless the user sets one), or after
//!   [`MAX_STEPS`] steps.
//! - **Look.** Thin lines by default; on request shaded tubes of a radius
//!   the user sets, their frames carried along the line by the double
//!   reflection method (W. Wang, B. Jüttler, D. Zheng and Y. Liu,
//!   "Computation of rotation minimizing frames", ACM Transactions on
//!   Graphics 27(1), 2008), and arrowheads along the line pointing
//!   downstream.
//! - **Colour.** Solid, or by a field of their own with a colour bar of
//!   their own (the default: the traced field's magnitude), in that
//!   field's style ([`crate::field::FieldStyle`]: map, range, flat,
//!   smooth or banded).
//!
//! Traces are made by [`ensure`] when a streamline's seeds, field or
//! limits change (`Session::redraw`) and kept in the display, so moving
//! a slice or a clip plane only draws them again.

use std::collections::HashMap;
use std::sync::Arc;

use murmel_mesh::{Bounds, Component, Mesh, NO_CELL, PatchRole};
use serde::{Deserialize, Serialize};

use crate::clip::{Clip, plane_axes};
use crate::colormap::Colormap;
use crate::display::{Display, Item};
use crate::field::{FieldChoice, FieldMode, FieldSet, FieldValues, RangeMode};
use crate::region::RegionCells;
use crate::surface::Surface;
use crate::tets::{barycentric, cell_value, centroid, face_centre, face_value, node_value};
use crate::theme::{Rgb, Tokens};

/// Most streamline sets one display holds.
pub const MAX_STREAMS: usize = 8;
/// Seeds of one set: the least, the default and the most.
pub const MIN_COUNT: u32 = 1;
pub const DEFAULT_COUNT: u32 = 50;
pub const MAX_COUNT: u32 = 1000;
/// The length limit unless the user sets one, in mesh diagonals.
pub const DEFAULT_LENGTH: f64 = 3.0;
/// The tube radius unless the user sets one, in mesh diagonals.
pub const DEFAULT_RADIUS: f64 = 1.0 / 400.0;
/// Most steps one way from a seed.
pub const MAX_STEPS: usize = 10_000;
/// Most points of one set; each line gets its share.
pub const MAX_POINTS: usize = 500_000;
/// A streamline ends where the speed falls below this share of the
/// field's largest.
pub const SLOW: f64 = 1e-3;
/// A step, in sizes of the cell it starts in.
const STEP: f64 = 0.25;
/// Most faces one walk crosses before it gives up.
const MAX_HOPS: usize = 256;
/// Sides of a tube and of an arrowhead.
const SIDES: usize = 8;
/// Most tube rings of one set; the rings thin out beyond.
const MAX_RINGS: usize = 200_000;
/// Arrowheads per mesh diagonal of streamline.
const HEADS_PER_DIAGONAL: f64 = 8.0;

fn default_count() -> u32 {
    DEFAULT_COUNT
}

fn is_default_count(n: &u32) -> bool {
    *n == DEFAULT_COUNT
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Where the seeds of a streamline set lie.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
pub enum Seed {
    /// Evenly along the segment `from`–`to`, its ends included.
    Line { from: [f64; 3], to: [f64; 3] },
    /// Spread through a ball: its centre first.
    Sphere { centre: [f64; 3], radius: f64 },
    /// On the faces of a boundary patch, by its name.
    Patch { patch: String },
    /// On an even grid of a plane through the mesh.
    Plane { origin: [f64; 3], normal: [f64; 3] },
}

impl Seed {
    pub const KINDS: [&'static str; 4] = ["line", "sphere", "patch", "plane"];

    pub fn key(&self) -> &'static str {
        match self {
            Seed::Line { .. } => "line",
            Seed::Sphere { .. } => "sphere",
            Seed::Patch { .. } => "patch",
            Seed::Plane { .. } => "plane",
        }
    }

    /// A seed shape of kind `kind` placed for `mesh`: a line across the
    /// mesh near the low end of its longest side (where a duct's inlet
    /// usually is), a ball at its centre, the first inlet patch (else the
    /// first patch), a plane across the longest side near its low end.
    pub fn default_for(kind: &str, mesh: &Mesh) -> Option<Seed> {
        let b = mesh.bounds();
        if b.is_empty() {
            return None;
        }
        let ext = [0, 1, 2].map(|k| b.max[k] - b.min[k]);
        let mut axes = [0usize, 1, 2];
        axes.sort_by(|&i, &j| ext[j].total_cmp(&ext[i]));
        let (long, across) = (axes[0], axes[1]);
        let c = b.center();
        let mut near_low = c;
        near_low[long] = b.min[long] + 0.05 * ext[long];
        Some(match kind {
            "line" => {
                let (mut from, mut to) = (near_low, near_low);
                from[across] = b.min[across] + 0.05 * ext[across];
                to[across] = b.max[across] - 0.05 * ext[across];
                Seed::Line { from, to }
            }
            "sphere" => Seed::Sphere {
                centre: c,
                radius: 0.1 * ext[long].max(f64::MIN_POSITIVE),
            },
            "patch" => {
                let p = mesh
                    .patches
                    .iter()
                    .find(|p| p.role == PatchRole::Inlet)
                    .or_else(|| mesh.patches.first())?;
                Seed::Patch {
                    patch: p.name.clone(),
                }
            }
            "plane" => {
                let mut normal = [0.0; 3];
                normal[long] = 1.0;
                Seed::Plane {
                    origin: near_low,
                    normal,
                }
            }
            _ => return None,
        })
    }

    /// Why the shape cannot seed on `mesh`, or `None`.
    pub fn problem(&self, mesh: &Mesh) -> Option<String> {
        if let Seed::Patch { patch } = self
            && !mesh.patches.iter().any(|p| &p.name == patch)
        {
            return Some(format!("no patch {patch}"));
        }
        self.problem_alone()
    }

    /// Why the shape cannot seed on any mesh, or `None`.
    pub fn problem_alone(&self) -> Option<String> {
        let finite = |p: &[f64; 3]| p.iter().all(|v| v.is_finite());
        match self {
            Seed::Line { from, to } => {
                if !finite(from) || !finite(to) {
                    return Some("the line's ends must be finite".into());
                }
                if from == to {
                    return Some("the line's ends must differ".into());
                }
            }
            Seed::Sphere { centre, radius } => {
                if !finite(centre) {
                    return Some("the centre must be finite".into());
                }
                if !(radius.is_finite() && *radius > 0.0) {
                    return Some(format!("radius {radius} must be above 0"));
                }
            }
            Seed::Patch { patch } => {
                if patch.is_empty() {
                    return Some("the patch needs a name".into());
                }
            }
            Seed::Plane { origin, normal } => {
                if !finite(origin) || !finite(normal) {
                    return Some("the plane must be finite".into());
                }
                if dot(*normal, *normal) == 0.0 {
                    return Some("the normal must not be zero".into());
                }
            }
        }
        None
    }
}

/// Which way from the seeds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Forward,
    Backward,
    #[default]
    Both,
}

impl Direction {
    pub const ALL: [Direction; 3] = [Direction::Forward, Direction::Backward, Direction::Both];

    pub fn key(self) -> &'static str {
        match self {
            Direction::Forward => "forward",
            Direction::Backward => "backward",
            Direction::Both => "both",
        }
    }
}

fn is_both(d: &Direction) -> bool {
    *d == Direction::Both
}

/// How the streamlines are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Look {
    #[default]
    Lines,
    Tubes,
}

impl Look {
    pub fn key(self) -> &'static str {
        match self {
            Look::Lines => "lines",
            Look::Tubes => "tubes",
        }
    }
}

fn is_lines(l: &Look) -> bool {
    *l == Look::Lines
}

/// A set of streamlines (`stream.set`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stream {
    /// The vector field traced (three components).
    pub field: String,
    pub seed: Seed,
    /// Off keeps the settings but draws nothing (the tree's eye).
    pub enabled: bool,
    /// Seeds wanted (the shape may place fewer: seeds outside the mesh
    /// are left out, a patch has so many faces).
    #[serde(default = "default_count", skip_serializing_if = "is_default_count")]
    pub count: u32,
    #[serde(default, skip_serializing_if = "is_both")]
    pub direction: Direction,
    /// The longest a streamline grows one way; `None`: [`DEFAULT_LENGTH`]
    /// mesh diagonals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<f64>,
    #[serde(default, skip_serializing_if = "is_lines")]
    pub look: Look,
    /// The tubes' radius; `None`: [`DEFAULT_RADIUS`] mesh diagonals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub arrowheads: bool,
    /// The field the lines are coloured by, with a colour bar of its own;
    /// `None`: solid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<FieldChoice>,
}

impl Stream {
    /// Streamlines of `field` from `seed`, coloured by the field's
    /// magnitude.
    pub fn new(field: impl Into<String>, seed: Seed) -> Self {
        let field = field.into();
        Self {
            color: Some(FieldChoice {
                name: field.clone(),
                component: Component::Magnitude,
            }),
            field,
            seed,
            enabled: true,
            count: DEFAULT_COUNT,
            direction: Direction::Both,
            max_length: None,
            look: Look::Lines,
            radius: None,
            arrowheads: false,
        }
    }

    /// Why the settings are out of range on any mesh, or `None`.
    pub fn problem_alone(&self) -> Option<String> {
        if self.field.is_empty() {
            return Some("a vector field is needed".into());
        }
        if !(MIN_COUNT..=MAX_COUNT).contains(&self.count) {
            return Some(format!(
                "count {} is outside {MIN_COUNT} to {MAX_COUNT}",
                self.count
            ));
        }
        if let Some(l) = self.max_length
            && !(l.is_finite() && l > 0.0)
        {
            return Some(format!("max_length {l} must be above 0"));
        }
        if let Some(r) = self.radius
            && !(r.is_finite() && r > 0.0)
        {
            return Some(format!("radius {r} must be above 0"));
        }
        self.seed.problem_alone()
    }

    /// The length limit on `mesh`.
    pub fn length_limit(&self, mesh: &Mesh) -> f64 {
        self.max_length
            .unwrap_or_else(|| DEFAULT_LENGTH * 2.0 * mesh.bounds().radius())
    }

    /// The tube radius on `mesh`.
    pub fn tube_radius(&self, mesh: &Mesh) -> f64 {
        self.radius
            .unwrap_or_else(|| DEFAULT_RADIUS * 2.0 * mesh.bounds().radius())
    }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|x| x * s)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let l = norm(a);
    if l > 0.0 { scale(a, 1.0 / l) } else { a }
}

/// Cells by their centroids on an even grid of buckets (about two cells
/// a bucket), so a point's nearest centroid is found near it. Built once
/// per mesh ([`FieldSet::cell_grid`]).
#[derive(Debug)]
pub(crate) struct CellGrid {
    lo: [f64; 3],
    side: [f64; 3],
    dims: [usize; 3],
    start: Vec<u32>,
    cells: Vec<u32>,
    centres: Vec<[f64; 3]>,
}

impl CellGrid {
    pub(crate) fn build(mesh: &Mesh, fields: &FieldSet) -> CellGrid {
        let n = mesh.cell_count();
        let centres: Vec<[f64; 3]> = (0..n).map(|c| fields.centroid(mesh, c)).collect();
        let mut b = Bounds::empty();
        for p in centres.iter().filter(|p| p.iter().all(|v| v.is_finite())) {
            b.add(*p);
        }
        if b.is_empty() {
            b.add([0.0; 3]);
        }
        let ext = [0, 1, 2].map(|k| b.max[k] - b.min[k]);
        let big = ext
            .iter()
            .copied()
            .fold(0.0, f64::max)
            .max(f64::MIN_POSITIVE);
        let ext = ext.map(|e| e.max(1e-9 * big));
        let target = (n / 2).max(1) as f64;
        let side = (ext[0] * ext[1] * ext[2] / target).cbrt();
        let dims = ext.map(|e| ((e / side).ceil() as usize).clamp(1, 1024));
        let side = [0, 1, 2].map(|k| ext[k] / dims[k] as f64);
        let mut g = CellGrid {
            lo: b.min,
            side,
            dims,
            start: Vec::new(),
            cells: Vec::new(),
            centres,
        };
        let buckets = dims[0] * dims[1] * dims[2];
        let mut at = vec![0u32; buckets + 1];
        let of: Vec<usize> = g.centres.iter().map(|&p| g.flat(g.bucket(p))).collect();
        for &k in &of {
            at[k + 1] += 1;
        }
        for k in 0..buckets {
            at[k + 1] += at[k];
        }
        let mut cursor = at.clone();
        let mut cells = vec![0u32; n];
        for (c, &k) in of.iter().enumerate() {
            cells[cursor[k] as usize] = c as u32;
            cursor[k] += 1;
        }
        g.start = at;
        g.cells = cells;
        g
    }

    fn bucket(&self, p: [f64; 3]) -> [usize; 3] {
        [0, 1, 2].map(|k| {
            let i = ((p[k] - self.lo[k]) / self.side[k]).floor();
            if i.is_finite() {
                (i.max(0.0) as usize).min(self.dims[k] - 1)
            } else {
                0
            }
        })
    }

    fn flat(&self, b: [usize; 3]) -> usize {
        (b[2] * self.dims[1] + b[1]) * self.dims[0] + b[0]
    }

    /// The cell whose centroid is nearest `p`.
    pub(crate) fn nearest(&self, p: [f64; 3]) -> Option<u32> {
        let b = self.bucket(p);
        let step = self.side.iter().copied().fold(f64::INFINITY, f64::min);
        let most = self.dims.iter().copied().max().unwrap_or(1);
        let mut best: Option<(f64, u32)> = None;
        for r in 0..=most {
            let lo = b.map(|x| x.saturating_sub(r));
            let hi = [0, 1, 2].map(|k| (b[k] + r).min(self.dims[k] - 1));
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        let shell = [x, y, z]
                            .iter()
                            .zip(&b)
                            .map(|(&i, &j)| i.abs_diff(j))
                            .max()
                            .unwrap_or(0);
                        if shell != r {
                            continue;
                        }
                        let k = self.flat([x, y, z]);
                        for &c in &self.cells[self.start[k] as usize..self.start[k + 1] as usize] {
                            let d = norm(sub(self.centres[c as usize], p));
                            if d.is_finite() && best.is_none_or(|(bd, _)| d < bd) {
                                best = Some((d, c));
                            }
                        }
                    }
                }
            }
            // Buckets further out are at least `r` sides away.
            if best.is_some_and(|(d, _)| d <= r as f64 * step) {
                break;
            }
        }
        best.map(|(_, c)| c)
    }
}

/// A cell of a part ([`Parts`]): of the mesh shown (part 0), or of a
/// block of an overview's file (the part is the block).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct At {
    pub part: u32,
    pub cell: u32,
}

impl At {
    /// Cell `cell` of the mesh shown.
    fn whole(cell: u32) -> At {
        At { part: 0, cell }
    }
}

/// A mesh streamlines are traced through: the mesh shown, or a block of
/// an overview's file read with its halo ([`crate::stream_blocks`]).
#[derive(Clone, Copy)]
pub(crate) struct Part<'a> {
    pub mesh: &'a Mesh,
    pub fields: &'a FieldSet,
    /// The three components of the traced vector field.
    pub v: [&'a FieldValues; 3],
    /// The values the lines are coloured by.
    pub color: Option<&'a FieldValues>,
    /// Which cells are the halo: a walk entering one goes on in the block
    /// that owns it, so the field is always read in an owned cell.
    pub halo: Option<&'a [bool]>,
}

/// The meshes streamlines are traced through.
pub(crate) trait Parts: Sync {
    /// Part `i`, when it is read.
    fn part(&self, i: u32) -> Option<Part<'_>>;
    /// The cell owning halo cell `c` of part `i`, in the part of its
    /// block (read when needed); `None` when that block cannot be read.
    fn owner(&self, i: u32, c: u32) -> Option<At>;
}

/// The mesh shown, the one part.
struct Whole<'a>(Part<'a>);

impl Parts for Whole<'_> {
    fn part(&self, i: u32) -> Option<Part<'_>> {
        (i == 0).then_some(self.0)
    }

    fn owner(&self, _: u32, _: u32) -> Option<At> {
        None
    }
}

/// Where a point lies: in one tetrahedron of a cell, with its weights.
#[derive(Clone, Copy, Debug)]
struct Hit<'m> {
    at: At,
    face: u32,
    nodes: &'m [u32],
    fc: [f64; 3],
    a: u32,
    b: u32,
    w: [f64; 4],
}

/// Why a streamline ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    /// It left the mesh (or the regions shown).
    Left,
    /// The speed fell below [`SLOW`] of the largest.
    Slow,
    /// The length limit.
    Length,
    /// [`MAX_STEPS`] or its share of [`MAX_POINTS`].
    Steps,
    /// The field has no value there, or the walk got lost.
    Lost,
    /// On an overview: the block it went on into could not be read
    /// (beyond the memory budget, or the file failed).
    Unread,
}

/// How many streamlines ended each way (each direction counts once).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Ends {
    pub left: usize,
    pub slow: usize,
    pub length: usize,
    pub steps: usize,
    pub lost: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub unread: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Ends {
    fn add(&mut self, e: End) {
        *match e {
            End::Left => &mut self.left,
            End::Slow => &mut self.slow,
            End::Length => &mut self.length,
            End::Steps => &mut self.steps,
            End::Lost => &mut self.lost,
            End::Unread => &mut self.unread,
        } += 1;
    }
}

/// One streamline: its points upstream to downstream, the cell of each
/// (of the mesh shown; none on an overview), and the colour field's value
/// at each (empty when solid).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub points: Vec<[f64; 3]>,
    pub cells: Vec<u32>,
    pub values: Vec<f64>,
    /// On an overview: the colour field's value of each point's cell
    /// (the flat style), as `cells` give it on the mesh shown.
    pub cell_values: Vec<f64>,
}

/// What a trace was made for.
#[derive(Clone, Debug)]
struct Key {
    field: String,
    seed: Seed,
    count: u32,
    direction: Direction,
    max_length: Option<f64>,
    region: Option<Arc<RegionCells>>,
}

impl Key {
    fn of(s: &Stream, region: Option<Arc<RegionCells>>) -> Key {
        Key {
            field: s.field.clone(),
            seed: s.seed.clone(),
            count: s.count,
            direction: s.direction,
            max_length: s.max_length,
            region,
        }
    }
}

impl PartialEq for Key {
    fn eq(&self, o: &Key) -> bool {
        let region = match (&self.region, &o.region) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        region
            && self.field == o.field
            && self.seed == o.seed
            && self.count == o.count
            && self.direction == o.direction
            && self.max_length == o.max_length
    }
}

/// The streamlines of one set, traced.
#[derive(Clone)]
pub struct Traced {
    key: Key,
    /// The field `Line::values` hold.
    color: Option<FieldChoice>,
    pub lines: Vec<Line>,
    /// Seeds placed in the mesh, and seeds the shape gave outside it.
    pub seeds: usize,
    pub outside: usize,
    pub ends: Ends,
    pub points: usize,
    /// The smallest and largest colour value on the lines.
    pub color_range: Option<(f64, f64)>,
    /// Why nothing was traced (the field is missing or not a vector).
    pub problem: Option<String>,
    /// The memory budget it was traced through an overview's blocks
    /// within ([`crate::stream_blocks`]); `None` on the cells shown.
    pub(crate) budget: Option<u64>,
    /// How many blocks the lines went through (an overview's).
    pub blocks: Option<usize>,
}

impl std::fmt::Debug for Traced {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Traced")
            .field("lines", &self.lines.len())
            .field("seeds", &self.seeds)
            .field("outside", &self.outside)
            .field("ends", &self.ends)
            .field("points", &self.points)
            .finish()
    }
}

impl Traced {
    /// Whether it was made for `s` as it is now (the region aside).
    pub fn fits(&self, s: &Stream) -> bool {
        self.key.field == s.field
            && self.key.seed == s.seed
            && self.key.count == s.count
            && self.key.direction == s.direction
            && self.key.max_length == s.max_length
            && self.color == s.color
    }
}

/// Whether sets `a` and `b` trace the same lines, coloured alike (their
/// looks aside).
pub(crate) fn same_trace(a: &Stream, b: &Stream) -> bool {
    a.field == b.field
        && a.seed == b.seed
        && a.count == b.count
        && a.direction == b.direction
        && a.max_length == b.max_length
        && a.color == b.color
}

/// The traces of the display's streamline sets, by set (`None` for a set
/// not traced: off since it was added).
#[derive(Clone, Debug, Default)]
pub struct Traces(pub Vec<Option<Arc<Traced>>>);

impl PartialEq for Traces {
    fn eq(&self, o: &Traces) -> bool {
        self.0.len() == o.0.len()
            && self.0.iter().zip(&o.0).all(|(a, b)| match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            })
    }
}

/// Trace the enabled streamline sets of `display` whose seeds, field or
/// limits changed, and colour those whose colour field changed
/// (`Session::redraw` calls it before the surface is built).
pub fn ensure(display: &mut Display, mesh: &Mesh, fields: &FieldSet) {
    if display.streams.is_empty() {
        display.stream_traces = None;
        return;
    }
    let region = display
        .region_cells
        .clone()
        .filter(|rc| rc.filtering() && rc.regions == display.regions);
    let old = display.stream_traces.clone();
    let mut out = Vec::with_capacity(display.streams.len());
    for (k, s) in display.streams.iter().enumerate() {
        let prev = old.as_ref().and_then(|t| t.0.get(k).cloned().flatten());
        if !s.enabled {
            out.push(prev);
            continue;
        }
        let key = Key::of(s, region.clone());
        out.push(Some(match prev {
            Some(p) if p.key == key && p.color == s.color => p,
            Some(p) if p.key == key => Arc::new(recolor((*p).clone(), mesh, fields, &s.color)),
            _ => Arc::new(trace(mesh, fields, s, key)),
        }));
    }
    let new = Traces(out);
    if old.as_deref() != Some(&new) {
        display.stream_traces = Some(Arc::new(new));
    }
}

/// Finds points in the cells of the parts and the vector field there.
struct Tracer<'a> {
    parts: &'a dyn Parts,
    /// The regions shown (cells of part 0).
    region: Option<&'a RegionCells>,
}

/// Which values of a part [`Tracer::value`] reads.
#[derive(Clone, Copy)]
enum Of {
    /// A component of the traced vector.
    V(usize),
    /// The colour field.
    Color,
}

impl<'a> Tracer<'a> {
    fn part(&self, i: u32) -> Result<Part<'a>, End> {
        self.parts.part(i).ok_or(End::Lost)
    }

    fn allowed(&self, at: At) -> bool {
        self.region.is_none_or(|r| r.contains(at.cell as usize))
    }

    /// The tetrahedron of cell `at` holding `p`, or the one it is nearest
    /// to (its weights clamped), and whether `p` lies inside.
    fn in_cell(&self, at: At, p: [f64; 3]) -> Option<(Hit<'a>, bool)> {
        let part = self.parts.part(at.part)?;
        let mesh = part.mesh;
        let c = at.cell;
        if c as usize >= mesh.cell_count() {
            return None;
        }
        let centre = centroid(mesh, Some(part.fields), c as usize);
        let mut best: Option<(f64, Hit<'a>)> = None;
        for f in mesh.cell(c as usize) {
            if f.len() < 3 {
                continue;
            }
            let fc = face_centre(mesh, f.nodes);
            for j in 0..f.len() {
                let (a, b) = (f.at(j), f.at((j + 1) % f.len()));
                let t = [centre, fc, mesh.points[a as usize], mesh.points[b as usize]];
                let Some(w) = barycentric(&t, p) else {
                    continue;
                };
                let low = w.iter().copied().fold(f64::INFINITY, f64::min);
                if best.as_ref().is_none_or(|(l, _)| low > *l) {
                    best = Some((
                        low,
                        Hit {
                            at,
                            face: f.id,
                            nodes: f.nodes,
                            fc,
                            a,
                            b,
                            w,
                        },
                    ));
                }
                if low >= -1e-9 {
                    return best.map(|(_, h)| (h, true));
                }
            }
        }
        best.map(|(_, mut h)| {
            let w = h.w.map(|x| x.max(0.0));
            let s: f64 = w.iter().sum();
            h.w = if s > 0.0 {
                w.map(|x| x / s)
            } else {
                [1.0, 0.0, 0.0, 0.0]
            };
            (h, false)
        })
    }

    /// Walk from cell `from` to the cell holding `p`. A walk entering a
    /// halo cell goes on from the same cell in the block owning it.
    fn locate_from(&self, p: [f64; 3], from: At) -> Result<Hit<'a>, End> {
        if !p.iter().all(|v| v.is_finite()) {
            return Err(End::Lost);
        }
        let (mut at, mut prev) = (from, NO_CELL);
        for _ in 0..MAX_HOPS {
            let part = self.part(at.part)?;
            if part
                .halo
                .is_some_and(|h| h.get(at.cell as usize).copied().unwrap_or(false))
            {
                at = self.parts.owner(at.part, at.cell).ok_or(End::Unread)?;
                prev = NO_CELL;
                continue;
            }
            if !self.allowed(at) {
                return Err(End::Left);
            }
            let mesh = part.mesh;
            let c = at.cell;
            if c as usize >= mesh.cell_count() {
                return Err(End::Lost);
            }
            let cc = centroid(mesh, Some(part.fields), c as usize);
            // The face whose plane the way from the centroid to `p`
            // crosses first.
            let mut exit: Option<(f64, u32)> = None;
            for f in mesh.cell(c as usize) {
                if f.len() < 3 {
                    continue;
                }
                let fc = face_centre(mesh, f.nodes);
                let mut n = [0.0; 3];
                for i in 0..f.len() {
                    let (a, b) = (
                        mesh.points[f.at(i) as usize],
                        mesh.points[f.at((i + 1) % f.len()) as usize],
                    );
                    n = add(n, cross(a, b));
                }
                if dot(n, sub(fc, cc)) < 0.0 {
                    n = scale(n, -1.0);
                }
                let sp = dot(sub(p, fc), n);
                if sp.is_nan() || sp <= 0.0 {
                    continue;
                }
                let sc = dot(sub(cc, fc), n);
                let t = if sc < 0.0 { sc / (sc - sp) } else { 0.0 };
                if exit.is_none_or(|(bt, _)| t < bt) {
                    exit = Some((t, f.id));
                }
            }
            let Some((_, u)) = exit else {
                return self.in_cell(at, p).map(|(h, _)| h).ok_or(End::Lost);
            };
            let next = mesh.across(u as usize, c);
            if next == NO_CELL {
                // Past a boundary face's plane, which a warped face may
                // not bound exactly.
                return match self.in_cell(at, p) {
                    Some((h, true)) => Ok(h),
                    _ => Err(End::Left),
                };
            }
            if next == prev {
                // Between two cells whose faces are not flat: the nearer.
                return self.in_cell(at, p).map(|(h, _)| h).ok_or(End::Lost);
            }
            prev = c;
            at.cell = next;
        }
        Err(End::Lost)
    }

    /// The value of `of` where `h` lies.
    fn value(&self, of: Of, h: &Hit<'_>) -> f64 {
        let Some(part) = self.parts.part(h.at.part) else {
            return f64::NAN;
        };
        let f = match of {
            Of::V(k) => part.v[k],
            Of::Color => match part.color {
                Some(f) => f,
                None => return f64::NAN,
            },
        };
        let fv = face_value(part.mesh, Some(part.fields), f, h.face, h.nodes, h.fc);
        h.w[0] * cell_value(f, h.at.cell as usize)
            + h.w[1] * fv
            + h.w[2] * node_value(f, h.a)
            + h.w[3] * node_value(f, h.b)
    }

    fn vector(&self, h: &Hit<'_>) -> Result<[f64; 3], End> {
        let v = [0, 1, 2].map(|k| self.value(Of::V(k), h));
        if v.iter().all(|x| x.is_finite()) {
            Ok(v)
        } else {
            Err(End::Lost)
        }
    }

    /// The size of cell `at`: the cube root of its box's volume (its
    /// largest side when the box is flat).
    fn size(&self, at: At) -> f64 {
        let Some(part) = self.parts.part(at.part) else {
            return f64::NAN;
        };
        let mut b = Bounds::empty();
        for i in part.mesh.cell_points(at.cell as usize) {
            if let Some(&p) = part.mesh.points.get(i as usize) {
                b.add(p);
            }
        }
        let e = [0, 1, 2].map(|k| b.max[k] - b.min[k]);
        let big = e.iter().copied().fold(0.0, f64::max);
        let v = (e[0] * e[1] * e[2]).cbrt();
        if v > 1e-3 * big { v } else { big }
    }

    /// The unit direction (times `sign`) at `p`, found from cell `from`:
    /// its cell, the direction and the vector.
    fn direction(&self, p: [f64; 3], from: At, sign: f64) -> Result<(At, [f64; 3]), End> {
        let h = self.locate_from(p, from)?;
        let v = self.vector(&h)?;
        let s = norm(v);
        let d = if s > 0.0 {
            scale(v, sign / s)
        } else {
            [0.0; 3]
        };
        Ok((h.at, d))
    }

    /// One step of `h` from `x` (in cell `c`, direction `k1`).
    fn rk4(
        &self,
        x: [f64; 3],
        c: At,
        k1: [f64; 3],
        h: f64,
        sign: f64,
    ) -> Result<([f64; 3], At), End> {
        let (c2, k2) = self.direction(add(x, scale(k1, 0.5 * h)), c, sign)?;
        let (c3, k3) = self.direction(add(x, scale(k2, 0.5 * h)), c2, sign)?;
        let (_, k4) = self.direction(add(x, scale(k3, h)), c3, sign)?;
        let d = [0, 1, 2].map(|i| (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]) / 6.0);
        let xn = add(x, scale(d, h));
        let hit = self.locate_from(xn, c3)?;
        Ok((xn, hit.at))
    }

    /// Follow the field from `p0` (in cell `c0`) one way: the points
    /// after `p0`, their cells, and why it ended.
    fn march(
        &self,
        p0: [f64; 3],
        c0: At,
        sign: f64,
        limit: f64,
        steps: usize,
        slow: f64,
    ) -> (Vec<[f64; 3]>, Vec<At>, End) {
        let (mut pts, mut cells) = (Vec::new(), Vec::new());
        let (mut x, mut c, mut len) = (p0, c0, 0.0);
        loop {
            if pts.len() >= steps {
                return (pts, cells, End::Steps);
            }
            let hit = match self.locate_from(x, c) {
                Ok(h) => h,
                Err(e) => return (pts, cells, e),
            };
            // (on an overview the cell may now be of another block)
            c = hit.at;
            let v = match self.vector(&hit) {
                Ok(v) => v,
                Err(e) => return (pts, cells, e),
            };
            let speed = norm(v);
            if speed.is_nan() || speed <= slow {
                return (pts, cells, End::Slow);
            }
            let size = self.size(c);
            let rest = limit - len;
            if rest.is_nan() || rest <= 1e-9 * size {
                return (pts, cells, End::Length);
            }
            let mut h = (STEP * size).min(rest);
            let k1 = scale(v, sign / speed);
            let mut tries = 0;
            let (xn, cn) = loop {
                match self.rk4(x, c, k1, h, sign) {
                    Ok(n) => break n,
                    Err(End::Left) if tries < 3 => {
                        tries += 1;
                        h *= 0.5;
                    }
                    Err(e) => return (pts, cells, e),
                }
            };
            let moved = norm(sub(xn, x));
            if moved.is_nan() || moved <= 1e-9 * size {
                return (pts, cells, End::Slow);
            }
            // The step's arc length (its chord is a little shorter).
            len += h;
            x = xn;
            c = cn;
            pts.push(x);
            cells.push(c);
        }
    }
}

/// The colour field's values at the lines' points (`ats`: the cell of
/// each), and their range; `own` keeps each point's cell's own value too
/// (an overview's lines, which name no cell of the mesh shown).
fn color_values(
    lines: &mut [Line],
    ats: &[Vec<At>],
    tracer: &Tracer<'_>,
    colored: bool,
    own: bool,
) -> Option<(f64, f64)> {
    let mut range: Option<(f64, f64)> = None;
    for (l, ats) in lines.iter_mut().zip(ats) {
        l.values.clear();
        l.cell_values.clear();
        if !colored {
            continue;
        }
        l.values = l
            .points
            .iter()
            .zip(ats)
            .map(|(&p, &c)| {
                tracer
                    .in_cell(c, p)
                    .map_or(f64::NAN, |(h, _)| tracer.value(Of::Color, &h))
            })
            .collect();
        if own {
            l.cell_values = ats
                .iter()
                .map(|a| {
                    tracer
                        .parts
                        .part(a.part)
                        .and_then(|p| p.color)
                        .map_or(f64::NAN, |f| cell_value(f, a.cell as usize))
                })
                .collect();
        }
        for &v in l.values.iter().filter(|v| v.is_finite()) {
            range = Some(range.map_or((v, v), |(a, b)| (a.min(v), b.max(v))));
        }
    }
    range
}

/// The three components of vector field `name`, and its largest
/// magnitude.
fn vector_values(
    mesh: &Mesh,
    fields: &FieldSet,
    name: &str,
) -> Result<([Arc<FieldValues>; 3], f64), String> {
    let f = fields.get(name).ok_or_else(|| format!("no field {name}"))?;
    if !f.is_vector() {
        return Err(format!("{name} is not a vector field"));
    }
    let comp = |component| {
        fields
            .values_cached(
                mesh,
                &FieldChoice {
                    name: name.into(),
                    component,
                },
            )
            .ok_or_else(|| format!("{name} has no values on this mesh"))
    };
    let top = comp(Component::Magnitude)?
        .range
        .map_or(0.0, |(a, b)| a.abs().max(b.abs()));
    Ok((
        [
            comp(Component::X)?,
            comp(Component::Y)?,
            comp(Component::Z)?,
        ],
        top,
    ))
}

fn empty(key: Key, color: Option<FieldChoice>, problem: Option<String>) -> Traced {
    Traced {
        key,
        color,
        lines: Vec::new(),
        seeds: 0,
        outside: 0,
        ends: Ends::default(),
        points: 0,
        color_range: None,
        problem,
        budget: None,
        blocks: None,
    }
}

/// Colour `t`'s lines by `color` (when the colour field changed).
fn recolor(mut t: Traced, mesh: &Mesh, fields: &FieldSet, color: &Option<FieldChoice>) -> Traced {
    t.color = color.clone();
    let Ok((v, _)) = vector_values(mesh, fields, &t.key.field) else {
        t.color_range = None;
        return t;
    };
    let values = color.as_ref().and_then(|c| fields.values_cached(mesh, c));
    let whole = Whole(Part {
        mesh,
        fields,
        v: [&*v[0], &*v[1], &*v[2]],
        color: values.as_deref(),
        halo: None,
    });
    let tracer = Tracer {
        parts: &whole,
        region: t.key.region.as_deref(),
    };
    let ats: Vec<Vec<At>> = t
        .lines
        .iter()
        .map(|l| l.cells.iter().map(|&c| At::whole(c)).collect())
        .collect();
    t.color_range = color_values(&mut t.lines, &ats, &tracer, values.is_some(), false);
    t
}

/// Trace the streamlines of `s` on the mesh shown.
fn trace(mesh: &Mesh, fields: &FieldSet, s: &Stream, key: Key) -> Traced {
    let (v, top) = match vector_values(mesh, fields, &s.field) {
        Ok(v) => v,
        Err(e) => return empty(key, s.color.clone(), Some(e)),
    };
    if let Some(e) = s.seed.problem(mesh) {
        return empty(key, s.color.clone(), Some(e));
    }
    let values = s.color.as_ref().and_then(|c| fields.values_cached(mesh, c));
    let region = key.region.clone();
    let whole = Whole(Part {
        mesh,
        fields,
        v: [&*v[0], &*v[1], &*v[2]],
        color: values.as_deref(),
        halo: None,
    });
    let tracer = Tracer {
        parts: &whole,
        region: region.as_deref(),
    };
    let grid = fields.cell_grid(mesh);
    let locate = |p: [f64; 3]| -> Option<At> {
        let start = grid.nearest(p)?;
        tracer.locate_from(p, At::whole(start)).ok().map(|h| h.at)
    };
    let (mut t, ats) = trace_lines(&tracer, mesh, s, key, top, &locate);
    t.color_range = color_values(&mut t.lines, &ats, &tracer, values.is_some(), false);
    for (l, a) in t.lines.iter_mut().zip(&ats) {
        l.cells = a.iter().map(|a| a.cell).collect();
    }
    t
}

/// Trace the streamlines of `s` through the parts of `tracer`, seeded on
/// `domain` (the mesh shown, or an overview's boundary) and placed in the
/// parts by `locate`; `top` is the traced field's largest magnitude. The
/// lines name the cells of the mesh shown unless `blocks` (then none),
/// and are coloured by the parts' colour values (their range too).
pub(crate) fn trace_parts(
    parts: &dyn Parts,
    domain: &Mesh,
    s: &Stream,
    top: f64,
    locate: &(dyn Fn([f64; 3]) -> Option<At> + Sync),
) -> Traced {
    let key = Key::of(s, None);
    if let Some(e) = s.seed.problem(domain) {
        return empty(key, s.color.clone(), Some(e));
    }
    let tracer = Tracer {
        parts,
        region: None,
    };
    let (mut t, ats) = trace_lines(&tracer, domain, s, key, top, locate);
    t.color_range = color_values(&mut t.lines, &ats, &tracer, s.color.is_some(), true);
    for l in &mut t.lines {
        l.cells = vec![NO_CELL; l.points.len()];
    }
    t
}

/// The cell holding `p`, walking through `parts` from cell `start`;
/// `None` outside the mesh or when the walk gets lost.
pub(crate) fn locate_in(parts: &dyn Parts, p: [f64; 3], start: At) -> Option<At> {
    let tracer = Tracer {
        parts,
        region: None,
    };
    tracer.locate_from(p, start).ok().map(|h| h.at)
}

/// The streamlines of `s` not traced, and why.
pub(crate) fn failed(s: &Stream, problem: String) -> Traced {
    empty(Key::of(s, None), s.color.clone(), Some(problem))
}

/// Seed and trace the lines of `s`: the trace (not coloured, its lines
/// naming no cell) and the cell of each point of each line.
fn trace_lines(
    tracer: &Tracer<'_>,
    domain: &Mesh,
    s: &Stream,
    key: Key,
    top: f64,
    locate: &(dyn Fn([f64; 3]) -> Option<At> + Sync),
) -> (Traced, Vec<Vec<At>>) {
    let (seeds, outside) = seeds(
        tracer,
        domain,
        locate,
        &s.seed,
        s.count.clamp(MIN_COUNT, MAX_COUNT),
    );
    let limit = s.length_limit(domain);
    let ways = match s.direction {
        Direction::Forward => vec![1.0],
        Direction::Backward => vec![-1.0],
        Direction::Both => vec![-1.0, 1.0],
    };
    let steps = (MAX_POINTS / (seeds.len().max(1) * ways.len())).clamp(1, MAX_STEPS);
    let slow = SLOW * top;
    let traced: Vec<(Line, Vec<At>, Vec<End>)> = murmel_mesh::par::collect(seeds.len(), 1, |r| {
        r.map(|i| {
            let (p, c) = seeds[i];
            let mut line = Line::default();
            let mut ats: Vec<At> = Vec::new();
            let mut ends = Vec::new();
            for &sign in &ways {
                let (pts, cells, end) = tracer.march(p, c, sign, limit, steps, slow);
                ends.push(end);
                if sign < 0.0 {
                    line.points.extend(pts.into_iter().rev());
                    ats.extend(cells.into_iter().rev());
                    line.points.push(p);
                    ats.push(c);
                } else {
                    if line.points.is_empty() {
                        line.points.push(p);
                        ats.push(c);
                    }
                    line.points.extend(pts);
                    ats.extend(cells);
                }
            }
            (line, ats, ends)
        })
        .collect()
    });
    let mut t = empty(key, s.color.clone(), None);
    t.seeds = seeds.len();
    t.outside = outside;
    let mut all = Vec::new();
    for (line, ats, ends) in traced {
        for e in ends {
            t.ends.add(e);
        }
        if line.points.len() >= 2 {
            t.points += line.points.len();
            t.lines.push(line);
            all.push(ats);
        }
    }
    (t, all)
}

/// Van der Corput's radical inverse of `i` in `base` (Halton's sequence
/// takes one prime base per dimension).
fn radical_inverse(mut i: usize, base: usize) -> f64 {
    let (mut f, mut r) = (1.0, 0.0);
    while i > 0 {
        f /= base as f64;
        r += f * (i % base) as f64;
        i /= base;
    }
    r
}

/// The seeds of `seed`, at most about `count`, each with its cell, and
/// how many points of the shape fell outside the mesh.
fn seeds(
    tracer: &Tracer<'_>,
    mesh: &Mesh,
    locate: &(dyn Fn([f64; 3]) -> Option<At> + Sync),
    seed: &Seed,
    count: u32,
) -> (Vec<([f64; 3], At)>, usize) {
    let n = count as usize;
    let place = |pts: Vec<[f64; 3]>| -> (Vec<([f64; 3], At)>, usize) {
        let mut out = Vec::with_capacity(pts.len());
        let mut outside = 0;
        for p in pts {
            match locate(p) {
                Some(c) => out.push((p, c)),
                None => outside += 1,
            }
        }
        (out, outside)
    };
    match seed {
        Seed::Line { from, to } => place(
            (0..n)
                .map(|i| {
                    let t = if n > 1 {
                        i as f64 / (n - 1) as f64
                    } else {
                        0.5
                    };
                    add(*from, scale(sub(*to, *from), t))
                })
                .collect(),
        ),
        Seed::Sphere { centre, radius } => {
            let mut pts = vec![*centre];
            let mut i = 1;
            while pts.len() < n && i < 4 * n + 64 {
                let q = [
                    2.0 * radical_inverse(i, 2) - 1.0,
                    2.0 * radical_inverse(i, 3) - 1.0,
                    2.0 * radical_inverse(i, 5) - 1.0,
                ];
                if dot(q, q) <= 1.0 {
                    pts.push(add(*centre, scale(q, *radius)));
                }
                i += 1;
            }
            place(pts)
        }
        Seed::Patch { patch } => {
            let Some(pi) = mesh.patches.iter().position(|p| &p.name == patch) else {
                return (Vec::new(), 0);
            };
            // The patch's faces: centre, area, owner (none on an
            // overview's boundary, whose faces point out of the volume).
            let cells = mesh.cell_count() > 0;
            let faces: Vec<([f64; 3], f64, u32, [f64; 3])> = (0..mesh.boundary.len())
                .filter(|&b| mesh.boundary_patch.get(b) == Some(&(pi as u32)))
                .map(|b| {
                    let nodes = mesh.boundary.get(b);
                    let fc = face_centre(mesh, nodes);
                    let mut a = [0.0; 3];
                    for i in 0..nodes.len() {
                        a = add(
                            a,
                            cross(
                                mesh.points[nodes[i] as usize],
                                mesh.points[nodes[(i + 1) % nodes.len()] as usize],
                            ),
                        );
                    }
                    let owner = mesh.boundary_owner.get(b).copied().unwrap_or(NO_CELL);
                    (fc, 0.5 * norm(a), owner, a)
                })
                .filter(|f| (f.2 != NO_CELL || !cells) && f.0.iter().all(|v| v.is_finite()))
                .collect();
            let total: f64 = faces.iter().map(|f| f.1).sum();
            if faces.is_empty() {
                return (Vec::new(), 0);
            }
            let lo = mesh.bounds().min;
            let mut side = (total / n as f64).sqrt();
            if side.is_nan() || side <= 0.0 {
                side = mesh.bounds().radius().max(f64::MIN_POSITIVE);
            }
            let mut picks: Vec<usize> = Vec::new();
            for _ in 0..40 {
                let mut cubes: HashMap<[i64; 3], (f64, usize)> = HashMap::new();
                for (k, f) in faces.iter().enumerate() {
                    let cube = [0, 1, 2].map(|d| ((f.0[d] - lo[d]) / side).floor() as i64);
                    let mid = [0, 1, 2].map(|d| lo[d] + (cube[d] as f64 + 0.5) * side);
                    let dist = norm(sub(f.0, mid));
                    let e = cubes.entry(cube).or_insert((dist, k));
                    if dist < e.0 || (dist == e.0 && k < e.1) {
                        *e = (dist, k);
                    }
                }
                picks = cubes.into_values().map(|(_, k)| k).collect();
                if picks.len() <= n {
                    break;
                }
                side *= 1.15;
            }
            picks.sort_unstable();
            let mut out = Vec::with_capacity(picks.len());
            let mut outside = 0;
            let inward = 1e-6 * mesh.bounds().radius();
            for k in picks {
                let (fc, _, owner, normal) = faces[k];
                let found = if owner == NO_CELL {
                    // Just inside the volume, against the face's normal.
                    let p = sub(fc, scale(unit(normal), inward));
                    locate(p).map(|c| (p, c))
                } else {
                    // Just inside the owner, so the walk starts in it.
                    let fields = tracer.parts.part(0).map(|p| p.fields);
                    let cc = centroid(mesh, fields, owner as usize);
                    let p = add(fc, scale(sub(cc, fc), 1e-3));
                    tracer
                        .locate_from(p, At::whole(owner))
                        .ok()
                        .map(|h| (p, h.at))
                };
                match found {
                    Some(f) => out.push(f),
                    None => outside += 1,
                }
            }
            (out, outside)
        }
        Seed::Plane { origin, normal } => {
            let nrm = unit(*normal);
            let (u, v) = plane_axes(nrm);
            let b = mesh.bounds();
            let (mut ulo, mut uhi, mut vlo, mut vhi) = (
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
            );
            for k in 0..8 {
                let corner = [0, 1, 2].map(|d| if k >> d & 1 == 0 { b.min[d] } else { b.max[d] });
                let r = sub(corner, *origin);
                ulo = ulo.min(dot(r, u));
                uhi = uhi.max(dot(r, u));
                vlo = vlo.min(dot(r, v));
                vhi = vhi.max(dot(r, v));
            }
            let area = (uhi - ulo) * (vhi - vlo);
            if area.is_nan() || area <= 0.0 {
                return (Vec::new(), 0);
            }
            let mut h = (area / n as f64).sqrt();
            let mut best = (Vec::new(), 0);
            for round in 0..4 {
                let (i0, i1) = ((ulo / h).ceil() as i64, (uhi / h).floor() as i64);
                let (j0, j1) = ((vlo / h).ceil() as i64, (vhi / h).floor() as i64);
                let total = (i1 - i0 + 1).max(0) * (j1 - j0 + 1).max(0);
                if total as usize > 20 * n + 64 {
                    break;
                }
                let mut pts = Vec::with_capacity(total.max(0) as usize);
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        pts.push(add(
                            *origin,
                            add(scale(u, i as f64 * h), scale(v, j as f64 * h)),
                        ));
                    }
                }
                let placed = place(pts);
                let found = placed.0.len();
                if found <= n || best.0.is_empty() {
                    best = placed;
                }
                if found >= n * 4 / 5 || round == 3 {
                    break;
                }
                h *= if found == 0 {
                    0.5
                } else {
                    (found as f64 / n as f64).sqrt().max(0.25)
                };
            }
            best
        }
    }
}

/// A point of a drawn piece of a line: where, its colour, its line.
type Piece = ([f64; 3], [f32; 3], u32);

/// How the streamlines of one set are coloured: solid, or by their
/// field in its style.
enum Paint {
    Solid([f32; 3]),
    Field {
        map: &'static Colormap,
        reverse: bool,
        lo: f64,
        hi: f64,
        mode: FieldMode,
        bands: u32,
        /// The mesh's values (none on an overview without the field).
        values: Option<Arc<FieldValues>>,
        none: [f32; 3],
        mix: Option<(Rgb, f32)>,
    },
}

impl Paint {
    fn color(&self, line: &Line, i: usize) -> [f32; 3] {
        match self {
            Paint::Solid(c) => *c,
            Paint::Field {
                map,
                reverse,
                lo,
                hi,
                mode,
                bands,
                values,
                none,
                mix,
            } => {
                let v = match mode {
                    // (an overview's lines keep their cells' values)
                    FieldMode::Flat if !line.cell_values.is_empty() => {
                        line.cell_values.get(i).copied().unwrap_or(f64::NAN)
                    }
                    FieldMode::Flat => match (values, line.cells.get(i)) {
                        (Some(v), Some(&c)) => cell_value(v, c as usize),
                        _ => f64::NAN,
                    },
                    _ => line.values.get(i).copied().unwrap_or(f64::NAN),
                };
                if !v.is_finite() {
                    return *none;
                }
                let mut t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
                if *mode == FieldMode::Banded && *bands > 0 {
                    let b = *bands as f64;
                    t = ((t * b).floor().min(b - 1.0) + 0.5) / b;
                }
                let rgb = map.sample(t, *reverse);
                match mix {
                    Some((sel, m)) => mixed(rgb, *sel, *m).linear(),
                    None => rgb.linear(),
                }
            }
        }
    }
}

fn mixed(a: Rgb, b: Rgb, mix: f32) -> Rgb {
    Rgb(std::array::from_fn(|k| {
        (a.0[k] as f32 * (1.0 - mix) + b.0[k] as f32 * mix).round() as u8
    }))
}

/// The colour range of set `k`'s colour field, in that field's style
/// (as the faces use it for the mesh's field), or `None` when solid or
/// not traced.
pub fn color_range(
    display: &Display,
    mesh: &Mesh,
    fields: Option<&FieldSet>,
    k: usize,
) -> Option<(f64, f64)> {
    let s = display.streams.get(k)?;
    let choice = s.color.as_ref()?;
    let t = display.stream_traces.as_ref()?.0.get(k)?.as_ref()?;
    let style = display.style_of(&choice.name);
    let whole = fields
        .and_then(|f| f.values_cached(mesh, choice))
        .and_then(|v| v.range)
        .or(t.color_range);
    let r = match style.range {
        // Streamlines keep the step's range for now (see glyph.rs).
        RangeMode::Field | RangeMode::Steps => whole?,
        RangeMode::Shown => t.color_range?,
        RangeMode::Symmetric => {
            let (a, b) = whole?;
            let m = a.abs().max(b.abs());
            (-m, m)
        }
        RangeMode::Fixed { min, max } => (min, max),
    };
    Some(crate::field::widen(r))
}

/// The solid colour of set `k`.
pub fn solid_color(display: &Display, tokens: &Tokens, k: usize) -> Rgb {
    display
        .colors
        .get(&Item::Stream(k as u32))
        .copied()
        .unwrap_or(tokens.mesh.field.stream)
}

/// Clip the segment `a`–`b` by the planes: the part every plane keeps,
/// as fractions along it.
fn clip_segment(planes: &[Clip], a: [f64; 3], b: [f64; 3]) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for p in planes {
        let (da, db) = (p.distance(a), p.distance(b));
        match (da <= 0.0, db <= 0.0) {
            (true, true) => {}
            (false, false) => return None,
            (true, false) => t1 = t1.min(da / (da - db)),
            (false, true) => t0 = t0.max(da / (da - db)),
        }
    }
    (t0 < t1).then_some((t0, t1))
}

fn lerp(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    add(a, scale(sub(b, a), t))
}

/// Draw the traced streamline sets of `display` into `s`: lines into its
/// stream line buffer, tubes and arrowheads as triangles.
pub fn append(
    s: &mut Surface,
    mesh: &Mesh,
    fields: Option<&FieldSet>,
    tokens: &Tokens,
    display: &Display,
) {
    let Some(traces) = display.stream_traces.as_deref() else {
        return;
    };
    let planes: Vec<Clip> = display.active_clips().copied().collect();
    let keeps = |p: [f64; 3]| planes.iter().all(|c| c.keeps(p));
    let diag = 2.0 * mesh.bounds().radius();
    let sel = tokens.mesh.selection;
    let mix = tokens.mesh.selection_mix.clamp(0.0, 1.0);
    let o = s.origin;
    let rel = |p: [f64; 3]| [0, 1, 2].map(|k| (p[k] - o[k]) as f32);
    let mut vb = s.visible_bounds.unwrap_or_else(Bounds::empty);
    for (k, st) in display.streams.iter().enumerate() {
        if !st.enabled {
            continue;
        }
        let Some(t) = traces.0.get(k).and_then(Option::as_ref) else {
            continue;
        };
        if !t.fits(st) {
            continue;
        }
        let picked = display.selection.contains(&Item::Stream(k as u32));
        let solid = || {
            let rgb = solid_color(display, tokens, k);
            Paint::Solid(if picked { mixed(rgb, sel, mix) } else { rgb }.linear())
        };
        let paint = match (&st.color, color_range(display, mesh, fields, k)) {
            (Some(choice), Some((lo, hi))) => {
                let style = display.style_of(&choice.name);
                match crate::field::map_for(&style, (lo, hi)) {
                    Some(map) => Paint::Field {
                        map,
                        reverse: style.reverse,
                        lo,
                        hi,
                        mode: style.mode,
                        bands: style
                            .bands
                            .clamp(crate::field::MIN_BANDS, crate::field::MAX_BANDS),
                        values: fields.and_then(|f| f.values_cached(mesh, choice)),
                        none: tokens.mesh.field.none.linear(),
                        mix: picked.then_some((sel, mix)),
                    },
                    None => solid(),
                }
            }
            _ => solid(),
        };
        let radius = st.tube_radius(mesh);
        for line in &t.lines {
            // The kept pieces of the line: runs of (point, colour).
            let mut pieces: Vec<Vec<Piece>> = Vec::new();
            let mut run: Vec<([f64; 3], [f32; 3], u32)> = Vec::new();
            for i in 0..line.points.len().saturating_sub(1) {
                let (a, b) = (line.points[i], line.points[i + 1]);
                let (ca, cb) = (paint.color(line, i), paint.color(line, i + 1));
                let cell = line.cells.get(i).copied().unwrap_or(NO_CELL);
                let Some((t0, t1)) = clip_segment(&planes, a, b) else {
                    if !run.is_empty() {
                        pieces.push(std::mem::take(&mut run));
                    }
                    continue;
                };
                let mixc = |t: f64| -> [f32; 3] {
                    std::array::from_fn(|d| ca[d] + (cb[d] - ca[d]) * t as f32)
                };
                let (pa, pb) = (lerp(a, b, t0), lerp(a, b, t1));
                if t0 > 0.0 && !run.is_empty() {
                    pieces.push(std::mem::take(&mut run));
                }
                if run.is_empty() {
                    run.push((pa, mixc(t0), cell));
                }
                run.push((pb, mixc(t1), cell));
                if t1 < 1.0 {
                    pieces.push(std::mem::take(&mut run));
                }
            }
            if !run.is_empty() {
                pieces.push(run);
            }
            for piece in &pieces {
                for &(p, _, _) in piece {
                    vb.add(p);
                }
                match st.look {
                    Look::Lines => {
                        for w in piece.windows(2) {
                            s.stream_positions.push(rel(w[0].0));
                            s.stream_positions.push(rel(w[1].0));
                            s.stream_colors.push(w[0].1);
                            s.stream_colors.push(w[1].1);
                        }
                    }
                    Look::Tubes => tube(s, piece, radius, ring_spacing(t, radius, diag)),
                }
            }
            if st.arrowheads {
                let size = match st.look {
                    Look::Tubes => 2.5 * radius,
                    Look::Lines => 2.5 * DEFAULT_RADIUS * diag,
                };
                let every = diag / HEADS_PER_DIAGONAL;
                let mut walked = 0.5 * every;
                for i in 0..line.points.len().saturating_sub(1) {
                    let (a, b) = (line.points[i], line.points[i + 1]);
                    let d = norm(sub(b, a));
                    while walked <= d && d > 0.0 {
                        let p = lerp(a, b, walked / d);
                        if keeps(p) {
                            let c = paint.color(line, i);
                            head(s, p, unit(sub(b, a)), size, c, line.cells[i]);
                            vb.add(p);
                        }
                        walked += every;
                    }
                    walked -= d;
                }
            }
        }
    }
    if !vb.is_empty() {
        s.visible_bounds = Some(vb);
    }
}

/// The spacing of tube rings that keeps a set within [`MAX_RINGS`].
fn ring_spacing(t: &Traced, radius: f64, diag: f64) -> f64 {
    let length: f64 = t
        .lines
        .iter()
        .map(|l| {
            l.points
                .windows(2)
                .map(|w| norm(sub(w[1], w[0])))
                .sum::<f64>()
        })
        .sum();
    (length / MAX_RINGS as f64)
        .max(0.5 * radius)
        .min(0.05 * diag.max(radius))
}

/// Push one vertex of a shaded triangle mesh (value: none, so the
/// colour shows).
fn vertex(s: &mut Surface, p: [f64; 3], n: [f64; 3], c: [f32; 3]) -> u32 {
    let o = s.origin;
    s.positions.push([0, 1, 2].map(|k| (p[k] - o[k]) as f32));
    s.normals.push(n.map(|x| x as f32));
    s.colors.push(c);
    s.values.push([0.0; 4]);
    (s.positions.len() - 1) as u32
}

fn triangle(s: &mut Surface, t: [u32; 3], cell: u32) {
    s.indices.extend(t);
    s.tri_cell.push(cell);
    s.tri_face.push(NO_CELL);
}

/// A tube of radius `r` along `piece`, its rings at least `spacing`
/// apart, its frame carried along by double reflection.
fn tube(s: &mut Surface, piece: &[([f64; 3], [f32; 3], u32)], r: f64, spacing: f64) {
    // The rings: the first point, then points `spacing` on, and the last.
    let mut keep: Vec<usize> = vec![0];
    for i in 1..piece.len() {
        let last = piece[*keep.last().unwrap_or(&0)].0;
        if norm(sub(piece[i].0, last)) >= spacing || i + 1 == piece.len() {
            keep.push(i);
        }
    }
    if keep.len() < 2 {
        return;
    }
    let pts: Vec<[f64; 3]> = keep.iter().map(|&i| piece[i].0).collect();
    let tangent = |i: usize| -> [f64; 3] {
        let (a, b) = if i == 0 {
            (pts[0], pts[1])
        } else if i + 1 == pts.len() {
            (pts[i - 1], pts[i])
        } else {
            (pts[i - 1], pts[i + 1])
        };
        unit(sub(b, a))
    };
    let mut t = tangent(0);
    let (mut nrm, _) = plane_axes(t);
    let mut prev: Option<u32> = None;
    for (j, &i) in keep.iter().enumerate() {
        if j > 0 {
            // Double reflection (Wang et al. 2008).
            let v1 = sub(pts[j], pts[j - 1]);
            let c1 = dot(v1, v1);
            let tn = tangent(j);
            if c1 > 0.0 {
                let rl = sub(nrm, scale(v1, 2.0 / c1 * dot(v1, nrm)));
                let tl = sub(t, scale(v1, 2.0 / c1 * dot(v1, t)));
                let v2 = sub(tn, tl);
                let c2 = dot(v2, v2);
                nrm = if c2 > 0.0 {
                    sub(rl, scale(v2, 2.0 / c2 * dot(v2, rl)))
                } else {
                    rl
                };
            }
            t = tn;
        }
        let b = cross(t, nrm);
        let (p, c, cell) = piece[i];
        let base = s.positions.len() as u32;
        for k in 0..SIDES {
            let a = k as f64 * std::f64::consts::TAU / SIDES as f64;
            let n = add(scale(nrm, a.cos()), scale(b, a.sin()));
            vertex(s, add(p, scale(n, r)), n, c);
        }
        if let Some(pb) = prev {
            let k = SIDES as u32;
            for q in 0..k {
                let (a0, a1) = (pb + q, pb + (q + 1) % k);
                let (b0, b1) = (base + q, base + (q + 1) % k);
                triangle(s, [a0, a1, b1], cell);
                triangle(s, [a0, b1, b0], cell);
            }
        }
        prev = Some(base);
    }
}

/// An arrowhead (a cone `size` wide, twice as long) at `p` pointing
/// along the unit `dir`.
fn head(s: &mut Surface, p: [f64; 3], dir: [f64; 3], size: f64, c: [f32; 3], cell: u32) {
    let (u, v) = plane_axes(dir);
    let len = 2.0 * size;
    let back = sub(p, scale(dir, 0.5 * len));
    let tip = add(p, scale(dir, 0.5 * len));
    let slope = size / len;
    let k = SIDES as u32;
    let ring: Vec<[f64; 3]> = (0..SIDES)
        .map(|i| {
            let a = i as f64 * std::f64::consts::TAU / SIDES as f64;
            add(scale(u, a.cos()), scale(v, a.sin()))
        })
        .collect();
    // The side: a ring of vertices and one tip vertex per side, so the
    // normals stay those of the cone.
    let base = s.positions.len() as u32;
    for r in &ring {
        let n = unit(add(*r, scale(dir, slope)));
        vertex(s, add(back, scale(*r, size)), n, c);
    }
    for i in 0..SIDES {
        let mid = unit(add(ring[i], ring[(i + 1) % SIDES]));
        vertex(s, tip, unit(add(mid, scale(dir, slope))), c);
    }
    for q in 0..k {
        triangle(s, [base + q, base + (q + 1) % k, base + k + q], cell);
    }
    // The back: a disc facing upstream.
    let back_n = scale(dir, -1.0);
    let centre = vertex(s, back, back_n, c);
    let rim = s.positions.len() as u32;
    for r in &ring {
        vertex(s, add(back, scale(*r, size)), back_n, c);
    }
    for q in 0..k {
        triangle(s, [centre, rim + (q + 1) % k, rim + q], cell);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> (Mesh, FieldSet) {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
        let (m, r) = murmel_mesh::read_results(&p).unwrap();
        (m, FieldSet::new(r))
    }

    fn traced(m: &Mesh, f: &FieldSet, s: Stream) -> Arc<Traced> {
        let mut d = Display {
            streams: vec![s],
            ..Display::default()
        };
        ensure(&mut d, m, f);
        d.stream_traces.unwrap().0[0].clone().unwrap()
    }

    /// Invariant: a seed's cell holds it, and walking from any cell to a
    /// centroid ends in that centroid's cell; a point outside the mesh is
    /// not found.
    #[test]
    fn walks_find_the_cells() {
        let (m, f) = channel();
        let v = vector_values(&m, &f, "velocity").unwrap().0;
        let whole = Whole(Part {
            mesh: &m,
            fields: &f,
            v: [&*v[0], &*v[1], &*v[2]],
            color: None,
            halo: None,
        });
        let tracer = Tracer {
            parts: &whole,
            region: None,
        };
        let grid = f.cell_grid(&m);
        for c in (0..m.cell_count()).step_by(5) {
            let p = f.centroid(&m, c);
            assert_eq!(grid.nearest(p), Some(c as u32));
            let from = ((c * 7919) % m.cell_count()) as u32;
            let h = tracer.locate_from(p, At::whole(from)).unwrap();
            assert_eq!(h.at, At::whole(c as u32), "from {from}");
            assert!(h.w.iter().all(|&w| w >= -1e-9));
        }
        let b = m.bounds();
        let out = [b.max[0] + 1.0, b.center()[1], b.center()[2]];
        let start = grid.nearest(out).unwrap();
        assert_eq!(
            tracer.locate_from(out, At::whole(start)).unwrap_err(),
            End::Left
        );
    }

    /// Invariant: the field inside a cell is the tetrahedra's linear
    /// field: at the centroid the cell's own value, at a node its node
    /// value.
    #[test]
    fn values_follow_the_tetrahedra() {
        let (m, f) = channel();
        let v = vector_values(&m, &f, "velocity").unwrap().0;
        let whole = Whole(Part {
            mesh: &m,
            fields: &f,
            v: [&*v[0], &*v[1], &*v[2]],
            color: None,
            halo: None,
        });
        let tracer = Tracer {
            parts: &whole,
            region: None,
        };
        for c in (0..m.cell_count()).step_by(11) {
            let p = f.centroid(&m, c);
            let (h, inside) = tracer.in_cell(At::whole(c as u32), p).unwrap();
            assert!(inside);
            let u = tracer.vector(&h).unwrap();
            for k in 0..3 {
                assert!((u[k] - v[k].cell[c]).abs() < 1e-9, "cell {c}");
            }
        }
    }

    /// Invariant: in the channel (flow along +x) a seed line across the
    /// inlet traced both ways gives one line per seed, each running
    /// downstream (x grows along it) from near the inlet to the outlet,
    /// where it leaves the mesh; backward it leaves through the inlet.
    #[test]
    fn channel_lines_run_downstream() {
        let (m, f) = channel();
        let b = m.bounds();
        let seed = Seed::default_for("line", &m).unwrap();
        let mut s = Stream::new("velocity", seed);
        s.count = 9;
        let t = traced(&m, &f, s);
        assert_eq!((t.seeds, t.outside), (9, 0), "{t:?}");
        assert_eq!(t.lines.len(), 9);
        let len = b.max[0] - b.min[0];
        for l in &t.lines {
            for w in l.points.windows(2) {
                assert!(w[1][0] >= w[0][0] - 1e-9 * len, "x falls: {w:?}");
            }
            let first = l.points[0];
            assert!(first[0] < b.min[0] + 0.06 * len, "first {first:?}");
            assert_eq!(l.values.len(), l.points.len());
        }
        // The middle line runs to the outlet; the cross flow (+y) takes
        // the others into the walls, where they leave or slow down.
        let middle = &t.lines[4].points;
        assert!(middle[middle.len() - 1][0] > b.max[0] - 0.01 * len);
        assert_eq!(t.ends.left + t.ends.slow, 18, "{:?}", t.ends);
        assert!(t.ends.left >= 10, "{:?}", t.ends);
        let (lo, hi) = t.color_range.unwrap();
        assert!(lo >= 0.0 && hi > lo);
    }

    /// Invariant: the length limit ends every line at that length (one
    /// way); forward only gives no point upstream of the seed.
    #[test]
    fn length_limit_and_direction() {
        let (m, f) = channel();
        let mut s = Stream::new("velocity", Seed::default_for("line", &m).unwrap());
        s.count = 3;
        s.direction = Direction::Forward;
        s.max_length = Some(0.1 * 2.0 * m.bounds().radius());
        let t = traced(&m, &f, s.clone());
        assert_eq!(t.ends.length, 3, "{:?}", t.ends);
        for l in &t.lines {
            // Chords of arcs: a little shorter than the arc length.
            let length: f64 = l.points.windows(2).map(|w| norm(sub(w[1], w[0]))).sum();
            let limit = s.max_length.unwrap();
            assert!(length <= limit * (1.0 + 1e-9) && length > 0.999 * limit);
        }
        s.direction = Direction::Backward;
        let t = traced(&m, &f, s);
        for l in &t.lines {
            let seed = l.points[l.points.len() - 1];
            assert!(l.points.iter().all(|p| p[0] <= seed[0] + 1e-12));
        }
    }

    /// Invariant: every seed shape places seeds inside the mesh, about
    /// as many as asked (a line or a ball inside the mesh exactly as
    /// many), each traced both ways to the length limit, and a
    /// shape outside the mesh places none and counts them outside.
    #[test]
    fn seed_shapes_place_seeds() {
        let (m, f) = channel();
        for kind in Seed::KINDS {
            let mut s = Stream::new("velocity", Seed::default_for(kind, &m).unwrap());
            s.count = 20;
            s.max_length = Some(1e-6);
            let t = traced(&m, &f, s);
            assert!(t.seeds >= 8 && t.seeds <= 25, "{kind}: {t:?}");
            assert_eq!(t.ends.length, 2 * t.seeds, "{kind}: {t:?}");
            if kind == "sphere" || kind == "line" {
                assert_eq!((t.seeds, t.outside), (20, 0), "{kind}");
            }
        }
        let b = m.bounds();
        let mut s = Stream::new(
            "velocity",
            Seed::Sphere {
                centre: [b.max[0] + 10.0, 0.0, 0.0],
                radius: 1.0,
            },
        );
        s.count = 5;
        let t = traced(&m, &f, s);
        assert_eq!((t.seeds, t.outside, t.lines.len()), (0, 5, 0));
    }

    /// Invariant: a trace is kept while only its look or colour changes
    /// (a new colour only recolours it), and made again when its seeds
    /// change.
    #[test]
    fn traces_are_kept_until_the_seeds_change() {
        let (m, f) = channel();
        let mut d = Display {
            streams: vec![Stream::new(
                "velocity",
                Seed::default_for("line", &m).unwrap(),
            )],
            ..Display::default()
        };
        d.streams[0].count = 4;
        ensure(&mut d, &m, &f);
        let first = d.stream_traces.clone().unwrap();
        d.streams[0].look = Look::Tubes;
        ensure(&mut d, &m, &f);
        assert!(Arc::ptr_eq(&first, d.stream_traces.as_ref().unwrap()));
        d.streams[0].color = Some(FieldChoice {
            name: "pressure".into(),
            component: Component::Magnitude,
        });
        ensure(&mut d, &m, &f);
        let again = d.stream_traces.clone().unwrap();
        assert!(!Arc::ptr_eq(&first, &again));
        let (a, b) = (first.0[0].as_ref().unwrap(), again.0[0].as_ref().unwrap());
        assert_eq!(a.lines.len(), b.lines.len());
        assert_eq!(a.lines[0].points, b.lines[0].points);
        assert_ne!(a.lines[0].values, b.lines[0].values);
        d.streams[0].count = 5;
        ensure(&mut d, &m, &f);
        assert_eq!(d.stream_traces.unwrap().0[0].as_ref().unwrap().seeds, 5);
    }

    /// Invariant: a missing or scalar field traces nothing and says why.
    #[test]
    fn wrong_fields_say_why() {
        let (m, f) = channel();
        let t = traced(
            &m,
            &f,
            Stream::new("pressure", Seed::default_for("line", &m).unwrap()),
        );
        assert!(t.lines.is_empty());
        assert!(t.problem.as_deref().unwrap().contains("not a vector"));
        let t = traced(
            &m,
            &f,
            Stream::new("nope", Seed::default_for("line", &m).unwrap()),
        );
        assert!(t.problem.as_deref().unwrap().contains("no field"));
    }

    /// Invariant: lines go into the stream line buffer in pairs, tubes
    /// and arrowheads into the triangles (closed rings of the tube's
    /// sides, normals of unit length); a clip plane drops the part it
    /// cuts away.
    #[test]
    fn geometry_of_lines_tubes_and_heads() {
        let (m, f) = channel();
        let t = Tokens::builtin().unwrap();
        let mut d = Display {
            streams: vec![Stream::new(
                "velocity",
                Seed::default_for("line", &m).unwrap(),
            )],
            ..Display::default()
        };
        d.streams[0].count = 3;
        ensure(&mut d, &m, &f);
        let mut s = Surface::default();
        append(&mut s, &m, Some(&f), &t, &d);
        assert!(s.stream_positions.len() >= 6);
        assert_eq!(s.stream_positions.len() % 2, 0);
        assert_eq!(s.stream_positions.len(), s.stream_colors.len());
        assert!(s.indices.is_empty());
        let whole = s.stream_positions.len();
        // Clipped at the middle of the channel: about half the lines.
        let b = m.bounds();
        d.clips = vec![Clip {
            origin: b.center(),
            normal: [1.0, 0.0, 0.0],
            mode: crate::clip::ClipMode::Crinkle,
            enabled: true,
        }];
        let mut s = Surface::default();
        append(&mut s, &m, Some(&f), &t, &d);
        assert!(s.stream_positions.len() < whole && !s.stream_positions.is_empty());
        let c = b.center()[0] as f32;
        assert!(s.stream_positions.iter().all(|p| p[0] <= c + 1e-4));
        d.clips.clear();
        d.streams[0].look = Look::Tubes;
        d.streams[0].arrowheads = true;
        let mut s = Surface::default();
        append(&mut s, &m, Some(&f), &t, &d);
        assert!(s.stream_positions.is_empty());
        assert!(!s.indices.is_empty());
        assert_eq!(s.indices.len() % 3, 0);
        assert_eq!(s.tri_cell.len(), s.indices.len() / 3);
        for n in &s.normals {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((l - 1.0).abs() < 1e-3);
        }
    }
}
