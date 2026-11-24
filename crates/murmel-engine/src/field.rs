//! Field results in the session: the fields read with the mesh
//! ([`FieldSet`]), the field the view colours by ([`FieldChoice`]), how
//! it is drawn ([`FieldStyle`]: flat, smooth or banded, contour lines,
//! colour map, range) and the values the surface takes to the GPU
//! ([`FieldValues`], [`FieldPaint`]).
//!
//! Smooth colouring needs values at the mesh nodes, and a finite-volume
//! solver writes them per cell. Each cell predicts the node's value from
//! its own value and its least-squares gradient ([`crate::gradient`]),
//! and the node takes the inverse distance weighted mean of its cells'
//! predictions, weights 1 / |node − cell centroid| (D. Shepard, "A
//! two-dimensional interpolation function for irregularly-spaced data",
//! Proc. ACM National Conference 1968, pp. 517–524), kept within the
//! values of those cells so no new extreme appears (as T. J. Barth and
//! D. C. Jespersen bound a reconstruction, "The design and application
//! of upwind schemes on unstructured meshes", AIAA paper 89-0366). A
//! field linear in space is so exact at every inner node, and its
//! iso-surfaces flat; the plain weighted mean of the cell values
//! (decided 2026-11-24 morning, replaced the same day) made them wave
//! cell by cell. Where the file also has the field on the boundary
//! faces, boundary nodes take those face values instead, weighted by
//! the face centres. A vector is interpolated by components and its
//! magnitude taken at the nodes.
//!
//! The values go to the GPU relative to [`FieldValues::origin`] (the
//! middle of the field's range), so they keep their precision in `f32`;
//! the range, the colour map, the bands and the contour lines are
//! uniforms, so changing them draws again without building the surface.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

pub use murmel_mesh::field::{Component, Field, Location, Results, range_of};
use murmel_mesh::{Mesh, NO_CELL};
use serde::{Deserialize, Serialize};

use crate::calc::Formula;
use crate::colormap::{self, Colormap};
use crate::derive::{Made, Recipe, Refusal};
use crate::display::ColorBy;
use crate::session::Session;

/// Bands and contour lines a field shows when first coloured (decided
/// 2026-11-24).
pub const DEFAULT_BANDS: u32 = 12;
pub const DEFAULT_LINES: u32 = 12;
/// The fewest and most bands, and the most contour lines.
pub const MIN_BANDS: u32 = 2;
pub const MAX_BANDS: u32 = 64;
pub const MAX_LINES: u32 = 64;

/// How the faces take the field's colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldMode {
    /// One colour per cell: the solver's own value, no interpolation.
    Flat,
    /// The node values blended across the faces (decided 2026-11-24 as
    /// the first look).
    #[default]
    Smooth,
    /// The range in `bands` steps of even colour, the band edges
    /// following the smooth values.
    Banded,
}

impl FieldMode {
    pub const ALL: [FieldMode; 3] = [FieldMode::Flat, FieldMode::Smooth, FieldMode::Banded];

    pub fn key(self) -> &'static str {
        match self {
            FieldMode::Flat => "flat",
            FieldMode::Smooth => "smooth",
            FieldMode::Banded => "banded",
        }
    }
}

/// What the colour range spans.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeMode {
    /// Smallest to largest value of the whole field (decided
    /// 2026-11-24): it stays put while cells are clipped or hidden.
    #[default]
    Field,
    /// Smallest to largest value drawn now; it follows clipping, hiding
    /// and isolating.
    Shown,
    /// −max|v| to +max|v| of the whole field, centred on zero.
    Symmetric,
    /// Set by the user.
    Fixed { min: f64, max: f64 },
}

impl RangeMode {
    pub fn key(self) -> &'static str {
        match self {
            RangeMode::Field => "field",
            RangeMode::Shown => "shown",
            RangeMode::Symmetric => "symmetric",
            RangeMode::Fixed { .. } => "fixed",
        }
    }
}

/// How one field is drawn; each field keeps its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FieldStyle {
    pub mode: FieldMode,
    /// Bands of colour mode banded.
    pub bands: u32,
    /// Contour lines on or off, in every mode.
    pub lines: bool,
    /// How many contour lines: they split the range into
    /// `line_count + 1` even steps (so `bands − 1` lines lie on the band
    /// edges).
    pub line_count: u32,
    /// A colour map id (`colormap::ids`); `None` takes
    /// rainbow_desaturated, or rainbow_uniform when the range is symmetric
    /// about zero (decided 2026-11-24).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map: Option<String>,
    /// The map turned end for end.
    pub reverse: bool,
    pub range: RangeMode,
}

impl Default for FieldStyle {
    fn default() -> Self {
        Self {
            mode: FieldMode::Smooth,
            bands: DEFAULT_BANDS,
            lines: false,
            line_count: DEFAULT_LINES,
            map: None,
            reverse: false,
            range: RangeMode::Field,
        }
    }
}

impl FieldStyle {
    /// The values the contour lines are drawn at, inside `range`.
    pub fn contours(&self, (lo, hi): (f64, f64)) -> Vec<f64> {
        if !self.lines || self.line_count == 0 {
            return Vec::new();
        }
        let n = self.line_count.min(MAX_LINES);
        (1..=n)
            .map(|i| lo + (hi - lo) * i as f64 / (n + 1) as f64)
            .collect()
    }
}

/// The field the view colours by, and which scalar of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldChoice {
    pub name: String,
    /// The magnitude or one component of a vector; a scalar's value.
    #[serde(default)]
    pub component: Component,
}

impl FieldChoice {
    /// The title of the legend: the name for a scalar, `|velocity|` for
    /// a vector's magnitude, `velocity_x` for a component (as FoehnCraft
    /// names its datasets), so no word needs translating.
    pub fn title(&self, vector: bool) -> String {
        match (vector, self.component) {
            (false, _) => self.name.clone(),
            (true, Component::Magnitude) => format!("|{}|", self.name),
            (true, c) => format!("{}_{}", self.name, c.name()),
        }
    }
}

/// How [`FieldSet::sample`] reads a value at a point of a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interp {
    /// The cell's value.
    Cell,
    /// The inverse distance weighted mean of the cell's node values.
    Nodes,
}

/// One field's value in one cell, for the Properties panel and probes.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FieldValue {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    pub location: Location,
    /// One value, or the three components of a vector.
    pub value: Vec<f64>,
}

impl FieldValue {
    /// The value of a scalar, the length of a vector.
    pub fn magnitude(&self) -> f64 {
        match self.value.as_slice() {
            [v] => *v,
            v => v.iter().map(|x| x * x).sum::<f64>().sqrt(),
        }
    }
}

/// The cells and boundary faces around each node, and their centroids,
/// built once per mesh the first time a field needs them.
#[derive(Debug)]
struct Topology {
    /// Cell `c` has the distinct nodes `cell_nodes[cell_start[c]..cell_start[c + 1]]`.
    cell_start: Vec<usize>,
    cell_nodes: Vec<u32>,
    /// Point `p` lies in the cells `point_cells[point_start[p]..point_start[p + 1]]`.
    point_start: Vec<usize>,
    point_cells: Vec<u32>,
    centre: Vec<[f64; 3]>,
    /// The same for boundary faces.
    point_face_start: Vec<usize>,
    point_faces: Vec<u32>,
    face_centre: Vec<[f64; 3]>,
}

/// Invert `items` (each a list of point ids) into lists per point.
fn invert(points: usize, start: &[usize], ids: &[u32]) -> (Vec<usize>, Vec<u32>) {
    let mut at = vec![0usize; points + 1];
    for &p in ids {
        at[p as usize + 1] += 1;
    }
    for i in 0..points {
        at[i + 1] += at[i];
    }
    let mut out = vec![0u32; ids.len()];
    let mut cursor = at.clone();
    for item in 0..start.len().saturating_sub(1) {
        for &p in &ids[start[item]..start[item + 1]] {
            out[cursor[p as usize]] = item as u32;
            cursor[p as usize] += 1;
        }
    }
    (at, out)
}

impl Topology {
    fn build(mesh: &Mesh) -> Topology {
        let np = mesh.points.len();
        let nc = mesh.cell_count();
        // Each cell's nodes, sorted and each once, run by run on all
        // threads, joined in cell order.
        let runs = murmel_mesh::par::map(nc, 1 << 13, |r| {
            let (mut lens, mut nodes) = (Vec::with_capacity(r.len()), Vec::new());
            let mut scratch: Vec<u32> = Vec::new();
            for c in r {
                scratch.clear();
                scratch.extend(mesh.cell_points(c).filter(|&i| (i as usize) < np));
                scratch.sort_unstable();
                scratch.dedup();
                nodes.extend_from_slice(&scratch);
                lens.push(scratch.len());
            }
            (lens, nodes)
        });
        let mut cell_start = Vec::with_capacity(nc + 1);
        cell_start.push(0);
        let mut cell_nodes = Vec::with_capacity(runs.iter().map(|r| r.1.len()).sum());
        for (lens, nodes) in runs {
            for k in lens {
                cell_start.push(cell_start[cell_start.len() - 1] + k);
            }
            cell_nodes.extend_from_slice(&nodes);
        }
        let (point_start, point_cells) = invert(np, &cell_start, &cell_nodes);
        let centre = murmel_mesh::par::collect(nc, 1 << 13, |r| {
            let mut faces = Vec::new();
            r.map(|c| crate::inspect::cell_centroid_in(mesh, c, &mut faces))
                .collect()
        });
        let mut scratch: Vec<u32> = Vec::new();
        let mut face_start = vec![0];
        let mut face_nodes = Vec::new();
        let mut face_centre = Vec::with_capacity(mesh.boundary.len());
        for f in mesh.boundary.iter() {
            scratch.clear();
            scratch.extend(f.iter().copied().filter(|&i| (i as usize) < np));
            scratch.sort_unstable();
            scratch.dedup();
            face_nodes.extend_from_slice(&scratch);
            face_start.push(face_nodes.len());
            let k = scratch.len().max(1) as f64;
            face_centre.push([0, 1, 2].map(|d| {
                scratch
                    .iter()
                    .map(|&i| mesh.points[i as usize][d])
                    .sum::<f64>()
                    / k
            }));
        }
        let (point_face_start, point_faces) = invert(np, &face_start, &face_nodes);
        Topology {
            cell_start,
            cell_nodes,
            point_start,
            point_cells,
            centre,
            point_face_start,
            point_faces,
            face_centre,
        }
    }

    fn nodes_of(&self, c: usize) -> &[u32] {
        match (self.cell_start.get(c), self.cell_start.get(c + 1)) {
            (Some(&a), Some(&b)) => &self.cell_nodes[a..b],
            _ => &[],
        }
    }

    /// The cells that share a node with cell `c`, each once (with `c`).
    fn near(&self, c: usize, out: &mut Vec<u32>) {
        for &n in self.nodes_of(c) {
            out.extend_from_slice(self.cells_of(n as usize));
        }
        out.sort_unstable();
        out.dedup();
    }

    fn cells_of(&self, p: usize) -> &[u32] {
        &self.point_cells[self.point_start[p]..self.point_start[p + 1]]
    }

    fn faces_of(&self, p: usize) -> &[u32] {
        &self.point_faces[self.point_face_start[p]..self.point_face_start[p + 1]]
    }
}

/// Shepard's inverse distance weighted mean at `at` of `values` sitting
/// at `centres` (of the items `ids`); a value at `at` itself wins. NaN
/// when no item has a finite value.
fn idw(at: [f64; 3], ids: &[u32], centres: &[[f64; 3]], values: &[f64]) -> f64 {
    let (mut sw, mut sv) = (0.0, 0.0);
    for &i in ids {
        let (Some(&v), Some(c)) = (values.get(i as usize), centres.get(i as usize)) else {
            continue;
        };
        if !v.is_finite() {
            continue;
        }
        let d = ((at[0] - c[0]).powi(2) + (at[1] - c[1]).powi(2) + (at[2] - c[2]).powi(2)).sqrt();
        if d <= f64::MIN_POSITIVE {
            return v;
        }
        sw += 1.0 / d;
        sv += v / d;
    }
    if sw > 0.0 { sv / sw } else { f64::NAN }
}

/// The mean at `at` of what the cells `ids` (centroids `centres`) predict
/// there from their values and gradients (`grad` of a cell), weighted as
/// Shepard's mean by the inverse distance, and kept within those cells'
/// values ([`crate::gradient`]); a cell whose centroid is `at` wins. NaN
/// when no cell has a finite value.
fn predicted(
    at: [f64; 3],
    ids: &[u32],
    centres: &[[f64; 3]],
    values: &[f64],
    mut grad: impl FnMut(usize) -> [f64; 3],
) -> f64 {
    let (mut sw, mut sv) = (0.0, 0.0);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &i in ids {
        let i = i as usize;
        let (Some(&v), Some(&c)) = (values.get(i), centres.get(i)) else {
            continue;
        };
        if !v.is_finite() {
            continue;
        }
        let d = ((at[0] - c[0]).powi(2) + (at[1] - c[1]).powi(2) + (at[2] - c[2]).powi(2)).sqrt();
        if d <= f64::MIN_POSITIVE {
            return v;
        }
        lo = lo.min(v);
        hi = hi.max(v);
        let x = crate::gradient::predict(v, c, grad(i), at);
        let x = if x.is_finite() { x } else { v };
        sw += 1.0 / d;
        sv += x / d;
    }
    if sw > 0.0 {
        (sv / sw).clamp(lo, hi)
    } else {
        f64::NAN
    }
}

/// Scalar `c` of a field whose entities have the values `comps` (one
/// list per component).
fn combine(comps: &[Vec<f64>], i: usize, c: Component) -> f64 {
    let at = |k: usize| {
        comps
            .get(k)
            .and_then(|v| v.get(i))
            .copied()
            .unwrap_or(f64::NAN)
    };
    match (comps.len(), c) {
        (1, _) => at(0),
        (_, Component::Magnitude) => (0..comps.len()).map(|k| at(k).powi(2)).sum::<f64>().sqrt(),
        (_, Component::X) => at(0),
        (_, Component::Y) => at(1),
        (_, Component::Z) => at(2),
    }
}

/// The components scalar `c` of a field of `k` components reads: all of
/// a scalar's or a vector's magnitude, else the one asked for.
fn needed(k: usize, c: Component) -> Vec<usize> {
    match (k, c) {
        (1, _) | (_, Component::Magnitude) => (0..k).collect(),
        (_, Component::X) => vec![0],
        (_, Component::Y) => vec![1],
        (_, Component::Z) => vec![2],
    }
}

/// Scalar `c` at every node of the node values `by_comp` of the
/// components [`needed`] gives: the one component as it is, else as
/// [`pick`] of all of them, on all threads.
fn picked(mut by_comp: Vec<Vec<f64>>, c: Component) -> Vec<f64> {
    if by_comp.len() == 1 {
        return by_comp.pop().unwrap_or_default();
    }
    let n = by_comp.iter().map(Vec::len).min().unwrap_or(0);
    murmel_mesh::par::collect(n, 1 << 14, |r| {
        let mut v = Vec::with_capacity(by_comp.len());
        r.map(|p| {
            v.clear();
            v.extend(by_comp.iter().map(|x| x[p]));
            pick(&v, c)
        })
        .collect()
    })
}

/// Scalar `c` of one entity's components (one value for a scalar).
fn pick(v: &[f64], c: Component) -> f64 {
    match (v, c) {
        ([x], _) => *x,
        (_, Component::Magnitude) => v.iter().map(|x| x * x).sum::<f64>().sqrt(),
        (_, Component::X) => v.first().copied().unwrap_or(f64::NAN),
        (_, Component::Y) => v.get(1).copied().unwrap_or(f64::NAN),
        (_, Component::Z) => v.get(2).copied().unwrap_or(f64::NAN),
    }
}

/// The values of each component of `f` (one list for a scalar).
fn components(f: &Field) -> Vec<Vec<f64>> {
    let k = f.components.max(1);
    (0..k)
        .map(|j| (0..f.len()).map(|i| f.values[i * k + j]).collect())
        .collect()
}

/// The values of the fields computed so far, by field and component.
type ValueCache = Arc<Mutex<HashMap<(String, Component), Arc<FieldValues>>>>;

/// The fields of the open mesh.
#[derive(Clone, Debug, Default)]
pub struct FieldSet {
    pub results: Results,
    topology: OnceLock<Arc<Topology>>,
    /// The cells' centroids on a bucket grid (streamline seeds).
    grid: OnceLock<Arc<crate::stream::CellGrid>>,
    /// The cells in bins, for slices and iso-surfaces ([`crate::bins`]).
    bins: OnceLock<Arc<crate::bins::CellBins>>,
    /// Values made once per field and component (colour mode field and
    /// every iso-surface ask for them again on each redraw).
    cache: ValueCache,
    /// How each derived or calculated field was made, each after the
    /// fields it reads ([`crate::derive`], [`crate::calc`]); their
    /// fields are in `results` with the file's.
    derived: Vec<Made>,
    /// The smallest and largest value of a field over the whole volume,
    /// as the file says, by field: the block ranges of a blocked VTKHDF
    /// file opened as an overview, whose fields hold only the boundary's
    /// values (a scalar's value, a vector's magnitude).
    volume_ranges: HashMap<String, (f64, f64)>,
    /// The preview grid of an overview's volume, once read: its cells and
    /// their fields, which slices, iso-surfaces and arrows draw on while
    /// the mesh shown has no cells ([`crate::preview`]).
    preview: OnceLock<Arc<crate::preview::Preview>>,
}

impl FieldSet {
    pub fn new(results: Results) -> Self {
        Self {
            results,
            topology: OnceLock::new(),
            grid: OnceLock::new(),
            bins: OnceLock::new(),
            cache: ValueCache::default(),
            derived: Vec::new(),
            volume_ranges: HashMap::new(),
            preview: OnceLock::new(),
        }
    }

    /// The preview grid of the volume, once read.
    pub fn preview(&self) -> Option<&Arc<crate::preview::Preview>> {
        self.preview.get()
    }

    /// Keep `preview` as these fields' preview grid; a set that has one
    /// keeps it.
    pub(crate) fn set_preview(&self, preview: Arc<crate::preview::Preview>) {
        let _ = self.preview.set(preview);
    }

    /// These fields with the ranges of some over the whole volume, which
    /// their colour range then spans ([`FieldValues::range`]).
    pub fn with_volume_ranges(
        mut self,
        ranges: impl IntoIterator<Item = (String, (f64, f64))>,
    ) -> Self {
        self.volume_ranges.extend(
            ranges
                .into_iter()
                .filter(|(_, (a, b))| a.is_finite() && b.is_finite() && a <= b),
        );
        self
    }

    /// The range of field `name` over the whole volume, when the file
    /// gave one.
    pub fn volume_range(&self, name: &str) -> Option<(f64, f64)> {
        self.volume_ranges.get(name).copied()
    }

    /// How derived field `name` was made; `None` for a field of the file
    /// or of a formula.
    pub fn recipe(&self, name: &str) -> Option<&Recipe> {
        match self.maker(name)? {
            Made::Quantity(r) => Some(r),
            Made::Formula(_) => None,
        }
    }

    /// The formula of calculated field `name`.
    pub fn formula(&self, name: &str) -> Option<&Formula> {
        match self.maker(name)? {
            Made::Formula(f) => Some(f),
            Made::Quantity(_) => None,
        }
    }

    /// How field `name` was made; `None` for a field of the file.
    pub fn maker(&self, name: &str) -> Option<&Made> {
        self.derived.iter().find(|m| m.field() == name)
    }

    /// How the derived and calculated fields were made, each after the
    /// fields it reads.
    pub fn made(&self) -> &[Made] {
        &self.derived
    }

    /// This set with the fields of `made` (made again when it made them
    /// before), and every field made from those made again after them;
    /// the names of the fields made, `made`'s first. Refused when `made`
    /// reads its own field or one made from it, or when a field made
    /// from it fails (the set stays as it was).
    pub(crate) fn remake(
        &self,
        mesh: &Mesh,
        made: Made,
    ) -> Result<(FieldSet, Vec<String>), Refusal> {
        let name = made.field().to_string();
        // The fields made from `name`, near or far, in order.
        let mut from: Vec<String> = vec![name.clone()];
        let mut after: Vec<Made> = Vec::new();
        for m in &self.derived {
            if m.field() == name {
                continue;
            }
            if m.reads(self).iter().any(|r| from.contains(r)) {
                from.push(m.field().to_string());
                after.push(m.clone());
            }
        }
        let reads = made.reads(self);
        if let Some(r) = reads.iter().find(|r| from.contains(r)) {
            return Err(Refusal::Cycle(if *r == name {
                format!("{name} reads itself")
            } else {
                format!("{name} reads {r}, which is made from {name}")
            }));
        }
        let fields = made.compute(mesh, self)?;
        let mut out = self.without(&name);
        out.derived
            .retain(|m| m.field() != name && !after.iter().any(|a| a.field() == m.field()));
        out.derived.push(made);
        out.derived.extend(after.iter().cloned());
        out.results.fields.extend(fields);
        let mut names = vec![name];
        for m in after {
            let n = m.field().to_string();
            let fields = m
                .compute(mesh, &out)
                .map_err(|e| Refusal::Breaks(n.clone(), Box::new(e)))?;
            out = out.without(&n);
            out.results.fields.extend(fields);
            names.push(n);
        }
        Ok((out, names))
    }

    /// This set without made field `name`. Refused when it has no such
    /// field, or when another made field reads it.
    pub(crate) fn unmake(&self, name: &str) -> Result<FieldSet, Refusal> {
        if self.maker(name).is_none() {
            return Err(Refusal::NotDerived(name.into()));
        }
        let readers: Vec<String> = self
            .derived
            .iter()
            .filter(|m| m.field() != name && m.reads(self).iter().any(|r| r == name))
            .map(|m| m.field().to_string())
            .collect();
        if !readers.is_empty() {
            return Err(Refusal::InUse(name.into(), readers));
        }
        let mut out = self.without(name);
        out.derived.retain(|m| m.field() != name);
        Ok(out)
    }

    /// A copy without the fields called `name` and their cached values.
    fn without(&self, name: &str) -> FieldSet {
        let mut out = self.clone();
        out.results.fields.retain(|f| f.name != name);
        let kept: HashMap<(String, Component), Arc<FieldValues>> = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|((n, _), _)| n != name)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        out.cache = Arc::new(Mutex::new(kept));
        out
    }

    /// Take `from`'s node and cell lists, built for the same mesh, so
    /// another step's fields need not build them again.
    pub(crate) fn share_topology(&self, from: &FieldSet) {
        if let Some(t) = from.topology.get() {
            let _ = self.topology.set(t.clone());
        }
        if let Some(b) = from.bins.get() {
            let _ = self.bins.set(b.clone());
        }
    }

    /// The velocity gradient Gᵢⱼ = ∂uᵢ/∂xⱼ in every cell of the vector
    /// field `f`: for a cell field the least-squares gradient of each
    /// component (with its values on the boundary faces when the file
    /// has them), for a node field each cell's own fit to its nodes
    /// about their mean, exact for a field linear in space. `None` for a
    /// field on the boundary faces alone or one that does not fit.
    pub(crate) fn velocity_gradient(&self, mesh: &Mesh, f: &Field) -> Option<Vec<[[f64; 3]; 3]>> {
        if !f.is_vector() || !Self::fits(f, mesh) {
            return None;
        }
        let comps = components(f);
        let g: Vec<Vec<[f64; 3]>> = match f.location {
            Location::Cell => {
                let twin = self.boundary_twin(mesh, f);
                comps
                    .iter()
                    .enumerate()
                    .map(|(j, v)| {
                        let b = twin.as_ref().and_then(|b| b.get(j));
                        self.cell_gradient(mesh, v, b.map(Vec::as_slice))
                    })
                    .collect()
            }
            Location::Point => comps
                .iter()
                .map(|v| self.node_fit_gradient(mesh, v))
                .collect(),
            Location::Boundary => return None,
        };
        Some(
            (0..mesh.cell_count())
                .map(|c| [0, 1, 2].map(|i| g[i].get(c).copied().unwrap_or([0.0; 3])))
                .collect(),
        )
    }

    /// The least-squares gradient in every cell of `values` (one per
    /// cell, NaN for none), fitted to the cells that share a node with
    /// each and to `boundary`, the values on the boundary faces, when
    /// given ([`crate::gradient`]).
    pub(crate) fn cell_gradient(
        &self,
        mesh: &Mesh,
        values: &[f64],
        boundary: Option<&[f64]>,
    ) -> Vec<[f64; 3]> {
        let t = self.topology(mesh);
        crate::gradient::cell_gradients(
            mesh,
            &t.centre,
            values,
            boundary
                .filter(|b| b.len() == t.face_centre.len())
                .map(|b| (b, t.face_centre.as_slice())),
            &|c, out| t.near(c, out),
        )
    }

    /// The gradient in every cell of `values` (one per node): each
    /// cell's own least-squares fit to its nodes about their mean, exact
    /// for a field linear in space.
    pub(crate) fn node_fit_gradient(&self, mesh: &Mesh, values: &[f64]) -> Vec<[f64; 3]> {
        let t = self.topology(mesh);
        murmel_mesh::par::map(mesh.cell_count(), 4096, |r| {
            r.map(|c| {
                let nodes = t.nodes_of(c);
                let k = nodes.len().max(1) as f64;
                let x = [0, 1, 2].map(|d| {
                    nodes
                        .iter()
                        .map(|&p| mesh.points[p as usize][d])
                        .sum::<f64>()
                        / k
                });
                let at = |p: u32| values.get(p as usize).copied().unwrap_or(f64::NAN);
                let mean = nodes.iter().map(|&p| at(p)).sum::<f64>() / k;
                crate::gradient::fit(
                    x,
                    mean,
                    nodes.iter().map(|&p| (mesh.points[p as usize], at(p))),
                )
            })
            .collect::<Vec<_>>()
        })
        .into_iter()
        .flatten()
        .collect()
    }

    /// The mean over each cell's nodes of `values` (`k` per node), `k`
    /// per cell, as [`FieldSet::cell_values`] gives a node field in a
    /// cell.
    pub(crate) fn cell_means(&self, mesh: &Mesh, values: &[f64], k: usize) -> Vec<f64> {
        let t = self.topology(mesh);
        let k = k.max(1);
        murmel_mesh::par::map(mesh.cell_count(), 4096, |r| {
            let mut out = Vec::with_capacity(r.len() * k);
            for c in r {
                let nodes = t.nodes_of(c);
                for j in 0..k {
                    if nodes.is_empty() {
                        out.push(f64::NAN);
                        continue;
                    }
                    let sum: f64 = nodes
                        .iter()
                        .map(|&p| values.get(p as usize * k + j).copied().unwrap_or(f64::NAN))
                        .sum();
                    out.push(sum / nodes.len() as f64);
                }
            }
            out
        })
        .concat()
    }

    /// Where the values at `location` sit: the cells' centroids, the
    /// nodes, or the boundary faces' centres.
    pub(crate) fn places<'a>(&'a self, mesh: &'a Mesh, location: Location) -> &'a [[f64; 3]] {
        match location {
            Location::Cell => &self.topology(mesh).centre,
            Location::Point => &mesh.points,
            Location::Boundary => &self.topology(mesh).face_centre,
        }
    }

    /// [`FieldSet::values`], made once per field and component.
    pub fn values_cached(&self, mesh: &Mesh, choice: &FieldChoice) -> Option<Arc<FieldValues>> {
        let f = self.get(&choice.name)?;
        let component = if f.is_vector() {
            choice.component
        } else {
            Component::Magnitude
        };
        let key = (choice.name.clone(), component);
        let cached = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        if cached.is_some() {
            return cached;
        }
        let v = Arc::new(self.values(mesh, choice)?);
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, v.clone());
        Some(v)
    }

    /// The colour range `choice`'s values would have
    /// ([`FieldValues::range`]), from its values when they are made, else
    /// from the field's own values (no node values made for it).
    pub(crate) fn range(&self, mesh: &Mesh, choice: &FieldChoice) -> Option<(f64, f64)> {
        let f = self.get(&choice.name)?;
        if !Self::fits(f, mesh) {
            return None;
        }
        let component = if f.is_vector() {
            choice.component
        } else {
            Component::Magnitude
        };
        let key = (choice.name.clone(), component);
        if let Some(v) = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
        {
            return v.range;
        }
        if f.location == Location::Point {
            // A point field's range is its node values'.
            return self.values_cached(mesh, choice)?.range;
        }
        let comps = components(f);
        let n = comps.first().map_or(0, Vec::len);
        let range = range_of((0..n).map(|i| combine(&comps, i, component)));
        match (component, self.volume_range(&choice.name)) {
            (Component::Magnitude, Some((a, b))) => {
                Some(range.map_or((a, b), |(lo, hi)| (lo.min(a), hi.max(b))))
            }
            _ => range,
        }
    }

    /// The cells' centroids on a bucket grid, made once.
    pub(crate) fn cell_grid(&self, mesh: &Mesh) -> &crate::stream::CellGrid {
        self.grid
            .get_or_init(|| Arc::new(crate::stream::CellGrid::build(mesh, self)))
    }

    /// The slope at mesh point `p` of a field whose cell gradients are
    /// `grad`: the inverse distance weighted mean of its cells' (the
    /// normals of iso-surfaces follow it).
    pub(crate) fn node_gradient(&self, mesh: &Mesh, grad: &[[f32; 3]], p: u32) -> [f64; 3] {
        let t = self.topology(mesh);
        let Some(&at) = mesh.points.get(p as usize) else {
            return [0.0; 3];
        };
        let (mut sw, mut sg) = (0.0, [0.0; 3]);
        for &c in t.cells_of(p as usize) {
            let (Some(g), Some(x)) = (grad.get(c as usize), t.centre.get(c as usize)) else {
                continue;
            };
            let d =
                ((at[0] - x[0]).powi(2) + (at[1] - x[1]).powi(2) + (at[2] - x[2]).powi(2)).sqrt();
            let w = 1.0 / d.max(f64::MIN_POSITIVE);
            sw += w;
            for k in 0..3 {
                sg[k] += w * f64::from(g[k]);
            }
        }
        if sw > 0.0 {
            sg.map(|x| x / sw)
        } else {
            [0.0; 3]
        }
    }

    /// The volume centroid of cell `c`.
    pub(crate) fn centroid(&self, mesh: &Mesh, c: usize) -> [f64; 3] {
        self.topology(mesh)
            .centre
            .get(c)
            .copied()
            .unwrap_or([f64::NAN; 3])
    }

    pub fn fields(&self) -> &[Field] {
        &self.results.fields
    }

    pub fn is_empty(&self) -> bool {
        self.results.fields.is_empty()
    }

    /// The field names, each once, in the order of the file.
    pub fn names(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for f in &self.results.fields {
            if !out.contains(&f.name.as_str()) {
                out.push(&f.name);
            }
        }
        out
    }

    /// The field called `name` at `location`.
    pub fn at(&self, name: &str, location: Location) -> Option<&Field> {
        self.results
            .fields
            .iter()
            .find(|f| f.name == name && f.location == location)
    }

    /// The field called `name`: its cell values, else its point values,
    /// else its boundary values.
    pub fn get(&self, name: &str) -> Option<&Field> {
        [Location::Cell, Location::Point, Location::Boundary]
            .into_iter()
            .find_map(|l| self.at(name, l))
    }

    /// Whether `f` has one value per entity of `mesh` it sits on.
    pub fn fits(f: &Field, mesh: &Mesh) -> bool {
        let n = match f.location {
            Location::Cell => mesh.cell_count(),
            Location::Point => mesh.points.len(),
            Location::Boundary => mesh.boundary.len(),
        };
        f.len() == n && f.values.len() == n * f.components.max(1)
    }

    /// The cells of `mesh` in bins ([`crate::bins`]), made once.
    pub(crate) fn bins(&self, mesh: &Mesh) -> &crate::bins::CellBins {
        self.bins.get_or_init(|| {
            let _phase = murmel_mesh::profile::span("field.bins");
            let t = self.topology(mesh);
            Arc::new(crate::bins::CellBins::build(mesh, |c| t.nodes_of(c)))
        })
    }

    /// The nodes of cell `c`, each once.
    pub(crate) fn nodes_of_cell(&self, mesh: &Mesh, c: usize) -> &[u32] {
        self.topology(mesh).nodes_of(c)
    }

    fn topology(&self, mesh: &Mesh) -> &Topology {
        self.topology
            .get_or_init(|| Arc::new(Topology::build(mesh)))
    }

    /// Every field's value in cell `c`, in file order: cell fields as
    /// written, point fields as the mean of the cell's nodes. Boundary
    /// fields have no value in a cell and are left out.
    pub fn cell_values(&self, mesh: &Mesh, c: usize) -> Vec<FieldValue> {
        if c >= mesh.cell_count() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for f in &self.results.fields {
            if !Self::fits(f, mesh) {
                continue;
            }
            let value = match f.location {
                Location::Cell => f.value(c).map(<[f64]>::to_vec),
                Location::Point => {
                    let nodes = self.topology(mesh).nodes_of(c);
                    let k = f.components.max(1);
                    (!nodes.is_empty()).then(|| {
                        (0..k)
                            .map(|j| {
                                nodes
                                    .iter()
                                    .map(|&n| f.values[n as usize * k + j])
                                    .sum::<f64>()
                                    / nodes.len() as f64
                            })
                            .collect()
                    })
                }
                Location::Boundary => None,
            };
            if let Some(value) = value {
                out.push(FieldValue {
                    name: f.name.clone(),
                    unit: f.unit.clone(),
                    location: f.location,
                    value,
                });
            }
        }
        out
    }

    /// Every field's value at `point` in cell `cell`, in file order:
    /// each component the inverse distance weighted mean of the cell's
    /// node values (point fields' own, cell fields' interpolated), as a
    /// probe reads them. Boundary fields are left out.
    pub fn point_values(&self, mesh: &Mesh, cell: usize, point: [f64; 3]) -> Vec<FieldValue> {
        if cell >= mesh.cell_count() {
            return Vec::new();
        }
        let t = self.topology(mesh);
        let nodes = t.nodes_of(cell);
        let centres: Vec<[f64; 3]> = nodes.iter().map(|&n| mesh.points[n as usize]).collect();
        let ids: Vec<u32> = (0..nodes.len() as u32).collect();
        let mut out = Vec::new();
        for f in &self.results.fields {
            if f.location == Location::Boundary || !Self::fits(f, mesh) {
                continue;
            }
            let comps = components(f);
            let twin = self.boundary_twin(mesh, f);
            let at: Vec<Vec<f64>> = nodes
                .iter()
                .map(|&n| self.node_components(mesh, f, &comps, twin.as_deref(), n as usize))
                .collect();
            let value: Vec<f64> = (0..comps.len())
                .map(|j| {
                    let vals: Vec<f64> = at.iter().map(|v| v[j]).collect();
                    idw(point, &ids, &centres, &vals)
                })
                .collect();
            if value.iter().all(|v| v.is_finite()) {
                out.push(FieldValue {
                    name: f.name.clone(),
                    unit: f.unit.clone(),
                    location: f.location,
                    value,
                });
            }
        }
        out
    }

    /// Every field's value at `point` in cell `cell`, read as a probe
    /// reads it (the maintainer's card of 2026-11-24, "cell value plus
    /// gradient"): a cell field's value is the cell's own plus its
    /// least-squares gradient ([`crate::gradient`], fitted to the cells
    /// sharing a node with it and to its boundary faces' values when the
    /// file has them) times the way from the centroid to the point, as a
    /// finite-volume solver reconstructs it; a node field's is the cell's
    /// own linear fit to its nodes. Both keep the cell's peak (inverse
    /// distance from the nodes flattened it) and are exact for a field
    /// linear in space. When `face` is a boundary face (a probe snapped
    /// to a wall), the boundary fields' values there follow. Derived and
    /// calculated fields are read like the file's.
    pub fn probe_values(
        &self,
        mesh: &Mesh,
        cell: usize,
        point: [f64; 3],
        face: Option<u32>,
    ) -> Vec<FieldValue> {
        if cell >= mesh.cell_count() {
            return Vec::new();
        }
        let t = self.topology(mesh);
        let mut around = Vec::new();
        // As the colouring's gradients take them: a cell sharing more
        // nodes counts more.
        t.near(cell, &mut around);
        let mut out: Vec<FieldValue> = self
            .results
            .fields
            .iter()
            .filter(|f| f.location != Location::Boundary && Self::fits(f, mesh))
            .filter_map(|f| {
                let value = self.probe_reading(mesh, f, cell, point, &around)?;
                Some(FieldValue {
                    name: f.name.clone(),
                    unit: f.unit.clone(),
                    location: f.location,
                    value,
                })
            })
            .collect();
        if let Some(u) = face
            && let Some(&b) = mesh.face_boundary.get(u as usize)
            && b != NO_CELL
        {
            out.extend(self.face_values(mesh, b as usize));
        }
        out
    }

    /// Scalar `component` of field `name` at `point` in cell `cell`, read
    /// as [`FieldSet::probe_values`] reads it; `None` for a boundary
    /// field, one that does not fit or no finite value.
    pub fn probe_value(
        &self,
        mesh: &Mesh,
        name: &str,
        component: Component,
        cell: usize,
        point: [f64; 3],
    ) -> Option<f64> {
        let f = self
            .results
            .fields
            .iter()
            .find(|f| f.name == name && f.location != Location::Boundary)?;
        if cell >= mesh.cell_count() || !Self::fits(f, mesh) {
            return None;
        }
        let mut around = Vec::new();
        self.topology(mesh).near(cell, &mut around);
        let v = pick(
            &self.probe_reading(mesh, f, cell, point, &around)?,
            component,
        );
        v.is_finite().then_some(v)
    }

    /// Each component of cell or node field `f` at `point` in `cell`
    /// (`around`: the cells sharing a node with it), read in place,
    /// without copying the field.
    fn probe_reading(
        &self,
        mesh: &Mesh,
        f: &Field,
        cell: usize,
        point: [f64; 3],
        around: &[u32],
    ) -> Option<Vec<f64>> {
        let k = f.components.max(1);
        let t = self.topology(mesh);
        let value: Vec<f64> = match f.location {
            Location::Cell => {
                let twin = self
                    .at(&f.name, Location::Boundary)
                    .filter(|b| b.components == f.components && Self::fits(b, mesh));
                let x = t.centre[cell];
                (0..k)
                    .map(|j| {
                        let v = f.values[cell * k + j];
                        let cells = around
                            .iter()
                            .map(|&o| o as usize)
                            .filter(|&o| o != cell)
                            .map(|o| (t.centre[o], f.values[o * k + j]));
                        let faces = twin.into_iter().flat_map(|b| {
                            mesh.cell(cell).filter_map(move |face| {
                                let i = *mesh.face_boundary.get(face.id as usize)?;
                                if i == NO_CELL {
                                    return None;
                                }
                                Some((
                                    *t.face_centre.get(i as usize)?,
                                    *b.values.get(i as usize * k + j)?,
                                ))
                            })
                        });
                        let g = crate::gradient::fit(x, v, cells.chain(faces));
                        crate::gradient::predict(v, x, g, point)
                    })
                    .collect()
            }
            Location::Point => {
                let nodes = t.nodes_of(cell);
                if nodes.is_empty() {
                    return None;
                }
                let n = nodes.len() as f64;
                let x = [0, 1, 2].map(|d| {
                    nodes
                        .iter()
                        .map(|&p| mesh.points[p as usize][d])
                        .sum::<f64>()
                        / n
                });
                (0..k)
                    .map(|j| {
                        let at = |p: u32| f.values[p as usize * k + j];
                        let mean = nodes.iter().map(|&p| at(p)).sum::<f64>() / n;
                        let g = crate::gradient::fit(
                            x,
                            mean,
                            nodes.iter().map(|&p| (mesh.points[p as usize], at(p))),
                        );
                        crate::gradient::predict(mean, x, g, point)
                    })
                    .collect()
            }
            Location::Boundary => return None,
        };
        value.iter().all(|v| v.is_finite()).then_some(value)
    }

    /// Every boundary field's value on boundary face `b`, in file order.
    pub fn face_values(&self, mesh: &Mesh, b: usize) -> Vec<FieldValue> {
        self.results
            .fields
            .iter()
            .filter(|f| f.location == Location::Boundary && Self::fits(f, mesh))
            .filter_map(|f| {
                Some(FieldValue {
                    name: f.name.clone(),
                    unit: f.unit.clone(),
                    location: f.location,
                    value: f.value(b)?.to_vec(),
                })
            })
            .collect()
    }

    /// The node value of each component of field `f` at point `p`.
    fn node_components(
        &self,
        mesh: &Mesh,
        f: &Field,
        comps: &[Vec<f64>],
        boundary: Option<&[Vec<f64>]>,
        p: usize,
    ) -> Vec<f64> {
        let t = self.topology(mesh);
        let at = mesh.points[p];
        match f.location {
            Location::Point => comps.iter().map(|v| v[p]).collect(),
            Location::Boundary => comps
                .iter()
                .map(|v| idw(at, t.faces_of(p), &t.face_centre, v))
                .collect(),
            Location::Cell => {
                if let Some(b) = boundary
                    && !t.faces_of(p).is_empty()
                {
                    let v: Vec<f64> = b
                        .iter()
                        .map(|v| idw(at, t.faces_of(p), &t.face_centre, v))
                        .collect();
                    if v.iter().all(|x| x.is_finite()) {
                        return v;
                    }
                }
                // What the cells around predict, each gradient made
                // for these few cells alone (a probe or a sample).
                comps
                    .iter()
                    .enumerate()
                    .map(|(j, v)| {
                        let b = boundary
                            .and_then(|b| b.get(j))
                            .map(|b| (b.as_slice(), t.face_centre.as_slice()));
                        let mut around = Vec::new();
                        predicted(at, t.cells_of(p), &t.centre, v, |c| {
                            around.clear();
                            t.near(c, &mut around);
                            crate::gradient::gradient_of(mesh, &t.centre, v, b, c, &around)
                        })
                    })
                    .collect()
            }
        }
    }

    /// The node values of components `need` of cell field `comps` (with
    /// the boundary values `twin` when the file has them), and the cell
    /// gradients of each, then of `extra` (cell values and their boundary
    /// values) when given: every gradient made in one pass over the cells
    /// around each cell.
    fn cell_nodes(
        &self,
        mesh: &Mesh,
        comps: &[Vec<f64>],
        twin: Option<&[Vec<f64>]>,
        need: &[usize],
        extra: Option<(&[f64], Option<&[f64]>)>,
    ) -> (Vec<Vec<f64>>, Vec<Vec<[f64; 3]>>) {
        let t = {
            let _phase = murmel_mesh::profile::span("field.topology");
            self.topology(mesh)
        };
        let np = mesh.points.len();
        let boundary = |j: usize| twin.and_then(|b| b.get(j)).map(Vec::as_slice);
        let faces = t.face_centre.as_slice();
        let mut inputs: Vec<crate::gradient::Input<'_>> = need
            .iter()
            .map(|&j| (comps[j].as_slice(), boundary(j).map(|b| (b, faces))))
            .collect();
        if let Some((v, b)) = extra {
            inputs.push((v, b.map(|b| (b, faces))));
        }
        let phase = murmel_mesh::profile::span("field.gradients");
        let grads = crate::gradient::cell_gradients_many(mesh, &t.centre, &inputs, &|c, out| {
            t.near(c, out)
        });
        drop(phase);
        let _phase = murmel_mesh::profile::span("field.nodes");
        let nodes = need
            .iter()
            .zip(&grads)
            .map(|(&j, g)| {
                let (v, b) = (&comps[j], boundary(j));
                murmel_mesh::par::map(np, 8192, |r| {
                    r.map(|p| {
                        let at = mesh.points[p];
                        if let Some(b) = b
                            && !t.faces_of(p).is_empty()
                        {
                            let x = idw(at, t.faces_of(p), &t.face_centre, b);
                            if x.is_finite() {
                                return x;
                            }
                        }
                        predicted(at, t.cells_of(p), &t.centre, v, |c| {
                            g.get(c).copied().unwrap_or([0.0; 3])
                        })
                    })
                    .collect::<Vec<_>>()
                })
                .into_iter()
                .flatten()
                .collect()
            })
            .collect();
        (nodes, grads)
    }

    /// The boundary values of the cell field `f` (a boundary field of the
    /// same name and shape), by component.
    fn boundary_twin(&self, mesh: &Mesh, f: &Field) -> Option<Vec<Vec<f64>>> {
        if f.location != Location::Cell {
            return None;
        }
        let b = self.at(&f.name, Location::Boundary)?;
        (b.components == f.components && Self::fits(b, mesh)).then(|| components(b))
    }

    /// Scalar `component` of field `name` at every mesh node: point
    /// values as written, cell and boundary values by inverse distance
    /// weighting (see the module notes). `None` for an unknown field or
    /// one that does not fit the mesh.
    pub fn node_values(&self, mesh: &Mesh, name: &str, component: Component) -> Option<Vec<f64>> {
        let f = self.get(name)?;
        if !Self::fits(f, mesh) {
            return None;
        }
        let comps = components(f);
        let twin = self.boundary_twin(mesh, f);
        if f.location == Location::Cell {
            let need = needed(comps.len(), component);
            let (by_comp, _) = self.cell_nodes(mesh, &comps, twin.as_deref(), &need, None);
            return Some(picked(by_comp, component));
        }
        Some(
            (0..mesh.points.len())
                .map(|p| {
                    pick(
                        &self.node_components(mesh, f, &comps, twin.as_deref(), p),
                        component,
                    )
                })
                .collect(),
        )
    }

    /// Scalar `component` of field `name` at `point` in cell `cell`:
    /// the cell's value, or the inverse distance weighted mean of its
    /// node values (`interp`). `None` without a value there.
    pub fn sample(
        &self,
        mesh: &Mesh,
        name: &str,
        component: Component,
        cell: usize,
        point: [f64; 3],
        interp: Interp,
    ) -> Option<f64> {
        let f = self.get(name)?;
        if !Self::fits(f, mesh) || cell >= mesh.cell_count() || f.location == Location::Boundary {
            return None;
        }
        let comps = components(f);
        let v = match (interp, f.location) {
            (Interp::Cell, Location::Cell) => combine(&comps, cell, component),
            _ => {
                let t = self.topology(mesh);
                let nodes = t.nodes_of(cell);
                let twin = self.boundary_twin(mesh, f);
                let node_comps: Vec<Vec<f64>> = nodes
                    .iter()
                    .map(|&n| self.node_components(mesh, f, &comps, twin.as_deref(), n as usize))
                    .collect();
                let centres: Vec<[f64; 3]> =
                    nodes.iter().map(|&n| mesh.points[n as usize]).collect();
                let ids: Vec<u32> = (0..nodes.len() as u32).collect();
                let k = comps.len();
                let at: Vec<f64> = (0..k)
                    .map(|j| {
                        let vals: Vec<f64> = node_comps.iter().map(|v| v[j]).collect();
                        match interp {
                            Interp::Nodes => idw(point, &ids, &centres, &vals),
                            // A point field's cell value: the mean of
                            // its nodes.
                            Interp::Cell => vals.iter().sum::<f64>() / vals.len().max(1) as f64,
                        }
                    })
                    .collect();
                pick(&at, component)
            }
        };
        v.is_finite().then_some(v)
    }

    /// The values colouring by `choice` draws: per cell, per node and per
    /// boundary face. `None` for an unknown field or one that does not
    /// fit the mesh.
    pub fn values(&self, mesh: &Mesh, choice: &FieldChoice) -> Option<FieldValues> {
        let f = self.get(&choice.name)?;
        if !Self::fits(f, mesh) {
            return None;
        }
        let component = if f.is_vector() {
            choice.component
        } else {
            Component::Magnitude
        };
        let comps = components(f);
        let twin = self.boundary_twin(mesh, f);
        let np = mesh.points.len();
        let nc = mesh.cell_count();
        // The cell values shown, and for a vector's magnitude its values
        // on the boundary (its gradient is its own, made with the
        // components').
        let cell_of = || -> Vec<f64> { (0..nc).map(|c| combine(&comps, c, component)).collect() };
        let magnitude = comps.len() > 1 && component == Component::Magnitude;
        let shown = (f.location == Location::Cell).then(cell_of);
        let shown_boundary: Option<Vec<f64>> = twin.as_deref().filter(|_| magnitude).map(|b| {
            (0..mesh.boundary.len())
                .map(|i| combine(b, i, component))
                .collect()
        });
        let (node, grad) = if f.location == Location::Cell {
            let need = needed(comps.len(), component);
            let extra = shown
                .as_deref()
                .filter(|_| magnitude)
                .map(|v| (v, shown_boundary.as_deref()));
            let (by_comp, mut grads) = self.cell_nodes(mesh, &comps, twin.as_deref(), &need, extra);
            let node = picked(by_comp, component);
            // a component's own gradient, or the magnitude's (last)
            (node, grads.pop().unwrap_or_default())
        } else {
            let node = (0..np)
                .map(|p| pick(&self.node_components(mesh, f, &comps, None, p), component))
                .collect();
            (node, Vec::new())
        };
        let (cell, boundary, range) = match f.location {
            Location::Cell => {
                let cell = shown.unwrap_or_else(cell_of);
                let r = range_of(cell.iter().copied());
                (cell, Vec::new(), r)
            }
            Location::Point => {
                let t = self.topology(mesh);
                let k = comps.len();
                let cell = (0..nc)
                    .map(|c| {
                        let nodes = t.nodes_of(c);
                        if nodes.is_empty() {
                            return f64::NAN;
                        }
                        let mean: Vec<f64> = (0..k)
                            .map(|j| {
                                nodes.iter().map(|&n| comps[j][n as usize]).sum::<f64>()
                                    / nodes.len() as f64
                            })
                            .collect();
                        pick(&mean, component)
                    })
                    .collect();
                let r = range_of(node.iter().copied());
                (cell, Vec::new(), r)
            }
            Location::Boundary => {
                let b: Vec<f64> = (0..mesh.boundary.len())
                    .map(|i| combine(&comps, i, component))
                    .collect();
                let r = range_of(b.iter().copied());
                (vec![f64::NAN; nc], b, r)
            }
        };
        // Over the whole volume when the file says (an overview's
        // fields hold the boundary's values): a scalar or a magnitude.
        let range = match (component, self.volume_range(&choice.name)) {
            (Component::Magnitude, Some((a, b))) => {
                Some(range.map_or((a, b), |(lo, hi)| (lo.min(a), hi.max(b))))
            }
            _ => range,
        };
        let origin = range.map_or(0.0, |(a, b)| 0.5 * (a + b));
        Some(FieldValues {
            choice: FieldChoice {
                name: choice.name.clone(),
                component,
            },
            vector: f.is_vector(),
            unit: f.unit.clone(),
            location: f.location,
            cell,
            node,
            boundary,
            grad: grad.into_iter().map(|g| g.map(|x| x as f32)).collect(),
            range,
            origin,
            spans: Default::default(),
        })
    }
}

/// The values of the field the view colours by.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldValues {
    pub choice: FieldChoice,
    pub vector: bool,
    pub unit: Option<String>,
    pub location: Location,
    /// Per cell; NaN where there is none (a boundary field).
    pub cell: Vec<f64>,
    /// Per mesh point; NaN where there is none.
    pub node: Vec<f64>,
    /// Per boundary face, for a field of the boundary; else empty.
    pub boundary: Vec<f64>,
    /// Per cell, the gradient of `cell` ([`crate::gradient`]), which
    /// the values between cells are rebuilt from; empty for a field not
    /// on the cells.
    pub grad: Vec<[f32; 3]>,
    /// The smallest and largest finite value the file has: over the
    /// cells, the points or the boundary faces, and over the whole
    /// volume when the file says ([`FieldSet::volume_range`]).
    pub range: Option<(f64, f64)>,
    /// Values reach the GPU relative to this.
    pub origin: f64,
    /// The smallest and largest value the iso-surface test meets in each
    /// bin of cells ([`crate::bins`]), made the first time an
    /// iso-surface of these values is drawn.
    pub(crate) spans: crate::bins::Kept<crate::bins::Spans>,
}

impl FieldValues {
    /// The value boundary face `b` (of a boundary field) or cell `c`
    /// shows in colour mode flat.
    pub fn flat(&self, cell: u32, boundary: u32) -> f64 {
        if self.location == Location::Boundary {
            return self
                .boundary
                .get(boundary as usize)
                .copied()
                .unwrap_or(f64::NAN);
        }
        if cell == NO_CELL {
            return f64::NAN;
        }
        self.cell.get(cell as usize).copied().unwrap_or(f64::NAN)
    }
}

/// A range with room: a field of one value spans a little around it, so
/// the colour sits mid-map and the legend has a scale.
pub fn widen((lo, hi): (f64, f64)) -> (f64, f64) {
    if hi > lo {
        return (lo, hi);
    }
    let d = if lo != 0.0 { lo.abs() * 1e-3 } else { 1.0 };
    (lo - d, lo + d)
}

/// Whether a range is centred on zero closely enough for the diverging
/// map: zero lies in its middle fifth.
pub fn symmetric((lo, hi): (f64, f64)) -> bool {
    lo < 0.0 && hi > 0.0 && (lo + hi).abs() <= 0.2 * (hi - lo)
}

/// The colour map `style` names, or the one it picks for `range`.
pub fn map_for(style: &FieldStyle, range: (f64, f64)) -> Option<&'static Colormap> {
    if let Some(m) = style.map.as_deref().and_then(colormap::get) {
        return Some(m);
    }
    colormap::get(if symmetric(range) {
        colormap::DIVERGING
    } else {
        colormap::DEFAULT
    })
}

/// What the renderer needs to colour the field. Read from the session
/// every frame; it changes without the surface being built again.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldPaint {
    pub mode: FieldMode,
    /// The range's ends relative to the surface's value origin.
    pub lo: f32,
    pub hi: f32,
    pub bands: u32,
    /// Contour lines; 0 for none.
    pub lines: u32,
    /// The colour map's id and direction.
    pub map: &'static str,
    pub reverse: bool,
    /// Linear colour of the contour lines and how much of it they mix in.
    pub line_color: [f32; 3],
    pub line_mix: f32,
    /// Linear selection colour and how much selected faces mix in.
    pub selection: [f32; 3],
}

impl Session {
    /// The fields of the open mesh.
    pub fn fields(&self) -> Option<&FieldSet> {
        self.mesh.as_ref().map(|m| m.fields.as_ref())
    }

    /// The values of the field shown now (colour mode field, values
    /// computed).
    pub fn field_values(&self) -> Option<&FieldValues> {
        self.display.field_values()
    }

    /// The style of the field chosen now (or the default style).
    pub fn field_style(&self) -> FieldStyle {
        self.display.field_style()
    }

    /// The colour range of the field shown now, as the legend and the
    /// faces use it.
    pub fn field_range(&self) -> Option<(f64, f64)> {
        let v = self.field_values()?;
        let style = self.field_style();
        let r = match style.range {
            RangeMode::Field => v.range?,
            RangeMode::Shown => self
                .mesh
                .as_ref()
                .and_then(|m| m.surface.field_range)
                .or(v.range)?,
            RangeMode::Symmetric => {
                let (a, b) = v.range?;
                let m = a.abs().max(b.abs());
                (-m, m)
            }
            RangeMode::Fixed { min, max } => (min, max),
        };
        Some(widen(r))
    }

    /// The colour map of the field shown now, and whether it is reversed.
    pub fn field_map(&self) -> Option<(&'static Colormap, bool)> {
        let style = self.field_style();
        Some((map_for(&style, self.field_range()?)?, style.reverse))
    }

    /// What the renderer draws the field with; `None` when the view is
    /// not coloured by a field.
    pub fn field_paint(&self) -> Option<FieldPaint> {
        let m = self.mesh.as_ref()?;
        self.field_values()?;
        let (lo, hi) = self.field_range()?;
        let (map, reverse) = self.field_map()?;
        let style = self.field_style();
        let o = m.surface.value_origin;
        let t = &self.tokens.mesh;
        Some(FieldPaint {
            mode: style.mode,
            lo: (lo - o) as f32,
            hi: (hi - o) as f32,
            bands: style.bands.clamp(MIN_BANDS, MAX_BANDS),
            lines: if style.lines {
                style.line_count.min(MAX_LINES)
            } else {
                0
            },
            map: map.id.as_str(),
            reverse,
            line_color: t.field.line.linear(),
            line_mix: t.field.line_mix.clamp(0.0, 1.0),
            selection: t.selection.linear(),
        })
    }
}

/// Compute the values of the chosen field the first time colour mode
/// field needs them, and again when the choice changes.
pub(crate) fn ensure_field(display: &mut crate::display::Display, mesh: &Mesh, fields: &FieldSet) {
    if display.color_by != ColorBy::Field {
        return;
    }
    let Some(choice) = display.field.clone() else {
        display.field_values = None;
        return;
    };
    let fresh = display.field_values.as_ref().is_some_and(|v| {
        v.choice.name == choice.name && (!v.vector || v.choice.component == choice.component)
    });
    if !fresh {
        let _phase = murmel_mesh::profile::span("field.values");
        display.field_values = fields.values_cached(mesh, &choice);
    }
}

/// The styles of the fields, by name.
pub type FieldStyles = BTreeMap<String, FieldStyle>;

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> (Mesh, FieldSet) {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
        let (m, r) = murmel_mesh::read_results(&p).unwrap();
        (m, FieldSet::new(r))
    }

    /// Invariant: each node takes what its cells predict there from their
    /// least-squares gradients, kept within those cells' values: so node
    /// values lie inside the cell range, a constant stays constant, a
    /// field linear in space is exact at every node off the boundary
    /// (inverse distance weighting was not, and its iso-surfaces waved),
    /// every node of a cell mesh gets a value, and a probe's few nodes
    /// (gradients made for their cells alone) read the same as the whole
    /// mesh's. Breaks when a gradient or a weight is wrong, the bound is
    /// dropped, or the two ways of making node values part.
    #[test]
    fn node_values_from_cell_gradients() {
        let (m, set) = channel();
        let choice = FieldChoice {
            name: "pressure".into(),
            component: Component::Magnitude,
        };
        let v = set.values(&m, &choice).unwrap();
        let (lo, hi) = v.range.unwrap();
        assert_eq!(v.cell.len(), m.cell_count());
        assert_eq!(v.node.len(), m.points.len());
        assert_eq!(v.grad.len(), m.cell_count());
        assert!(v.node.iter().all(|x| x.is_finite()));
        assert!(v.node.iter().all(|&x| x >= lo - 1e-9 && x <= hi + 1e-9));
        // A constant field stays constant at every node.
        let mut r = set.results.clone();
        for f in &mut r.fields {
            if f.name == "pressure" {
                f.values.iter_mut().for_each(|x| *x = 7.5);
            }
        }
        let flat = FieldSet::new(r);
        let n = flat
            .node_values(&m, "pressure", Component::Magnitude)
            .unwrap();
        assert!(n.iter().all(|&x| (x - 7.5).abs() < 1e-12));
        // A linear field (cell values only) is exact inside.
        let g0 = [0.7, -1.3, 2.1];
        let lin = |x: [f64; 3]| 3.0 + g0[0] * x[0] + g0[1] * x[1] + g0[2] * x[2];
        let mut r = set.results.clone();
        r.fields
            .retain(|f| f.name != "pressure" || f.location == Location::Cell);
        for f in &mut r.fields {
            if f.name == "pressure" {
                for (c, x) in f.values.iter_mut().enumerate() {
                    *x = lin(crate::inspect::cell_centroid(&m, c));
                }
            }
        }
        let linear = FieldSet::new(r);
        let n = linear
            .node_values(&m, "pressure", Component::Magnitude)
            .unwrap();
        let mut inner = 0;
        for (p, &x) in n.iter().enumerate() {
            let on_wall = m.boundary.iter().any(|f| f.contains(&(p as u32)));
            if on_wall {
                continue;
            }
            inner += 1;
            let want = lin(m.points[p]);
            assert!(
                (x - want).abs() < 1e-9 * want.abs().max(1.0),
                "node {p}: {x} vs {want}"
            );
        }
        assert!(inner > 0, "no inner node");
        // A probe's nodes read the same as the whole mesh's.
        let f = linear.get("pressure").unwrap();
        let comps = components(f);
        for p in (0..m.points.len()).step_by(37) {
            let local = linear.node_components(&m, f, &comps, None, p);
            assert!((local[0] - n[p]).abs() < 1e-9, "node {p}");
        }
    }

    /// Invariant (the maintainer's card of 2026-11-24, "cell value plus
    /// gradient"): a probe reads a cell field as the cell's value plus
    /// its least-squares gradient to the point, so it reads the cell's
    /// own value at the centroid (the peak is kept) and a field linear in
    /// space exactly anywhere in a cell, wall cells included (their
    /// boundary values join the fit); a node field reads exactly through
    /// the cell's fit to its nodes; probe_value picks components as the
    /// colouring does, and a wall face adds the boundary fields. Breaks
    /// if the probe went back to node averages (a coarse cell's peak
    /// flattens) or the stencil or the centroid were wrong.
    #[test]
    fn probes_read_cell_value_plus_gradient() {
        let (m, set) = channel();
        let g0 = [0.7, -1.3, 2.1];
        let lin = |x: [f64; 3]| 3.0 + g0[0] * x[0] + g0[1] * x[1] + g0[2] * x[2];
        let mut r = set.results.clone();
        // Linear pressure in the cells and on the boundary faces, linear
        // velocity (x and z components) at the nodes.
        let t = set.topology(&m);
        for f in &mut r.fields {
            match (f.name.as_str(), f.location) {
                ("pressure", Location::Cell) => {
                    for (c, x) in f.values.iter_mut().enumerate() {
                        *x = lin(t.centre[c]);
                    }
                }
                ("pressure", Location::Boundary) => {
                    for (b, x) in f.values.iter_mut().enumerate() {
                        *x = lin(t.face_centre[b]);
                    }
                }
                _ => {}
            }
        }
        r.fields.retain(|f| f.name != "velocity");
        r.fields.push(Field {
            name: "velocity".into(),
            unit: Some("m/s".into()),
            location: Location::Point,
            components: 3,
            values: m
                .points
                .iter()
                .flat_map(|&x| [lin(x), 0.0, -lin(x)])
                .collect(),
        });
        let linear = FieldSet::new(r);
        for c in (0..m.cell_count()).step_by(53) {
            let x = linear.topology(&m).centre[c];
            let at = linear
                .probe_value(&m, "pressure", Component::Magnitude, c, x)
                .unwrap();
            assert!((at - lin(x)).abs() < 1e-12, "centroid of cell {c}");
            // Halfway to the cell's first node.
            let n = m.points[linear.topology(&m).nodes_of(c)[0] as usize];
            let y = [0, 1, 2].map(|k| 0.5 * (x[k] + n[k]));
            let want = lin(y);
            let tol = 1e-9 * want.abs().max(1.0);
            let p = linear
                .probe_value(&m, "pressure", Component::Magnitude, c, y)
                .unwrap();
            assert!((p - want).abs() < tol, "cell {c}: {p} vs {want}");
            let vz = linear
                .probe_value(&m, "velocity", Component::Z, c, y)
                .unwrap();
            // The fit's tiny ridge (gradient::solve) leaves a few parts
            // in a billion on a cell's handful of nodes.
            let node_tol = 1e-7 * want.abs().max(1.0);
            assert!(
                (vz + want).abs() < node_tol,
                "velocity_z in cell {c}: {vz} vs {}",
                -want
            );
            let all = linear.probe_values(&m, c, y, None);
            let v = all.iter().find(|f| f.name == "velocity").unwrap();
            assert_eq!(v.value.len(), 3);
            assert!((v.value[0] - want).abs() < node_tol);
        }
        // The real pressure at a centroid is the cell's own value.
        let pc = set.at("pressure", Location::Cell).unwrap();
        let c = 11;
        let x = set.topology(&m).centre[c];
        let got = set
            .probe_value(&m, "pressure", Component::Magnitude, c, x)
            .unwrap();
        assert!((got - pc.values[c]).abs() < 1e-12 * pc.values[c].abs().max(1.0));
        // A wall face brings the boundary fields.
        let wall = m.boundary_face_id[0];
        let owner = m.face_owner[wall as usize] as usize;
        let vals = set.probe_values(&m, owner, set.topology(&m).centre[owner], Some(wall));
        assert!(
            vals.iter()
                .any(|f| f.name == "y_plus" && f.location == Location::Boundary)
        );
    }

    /// Invariant (fields on large meshes): a vector's component made
    /// alone (its gradient and node values only) is what it was made
    /// with all three; the magnitude's node values are the length of the
    /// components' and its gradient its own, made in the same pass as
    /// theirs; `range` without node values is the values' range. Breaks
    /// when the one pass over the cells mixes the inputs' gradients up,
    /// or when a component alone is fitted differently. The test below
    /// looks at the values, not at how they were made.
    #[test]
    fn a_component_alone_as_with_all_three() {
        let (m, set) = channel();
        let choice = |component| FieldChoice {
            name: "velocity".into(),
            component,
        };
        let f = set.get("velocity").unwrap();
        let comps = components(f);
        let twin = set.boundary_twin(&m, f);
        let (all, grads) = set.cell_nodes(&m, &comps, twin.as_deref(), &[0, 1, 2], None);
        let t = set.topology(&m);
        for (j, c) in [Component::X, Component::Y, Component::Z]
            .into_iter()
            .enumerate()
        {
            let v = set.values(&m, &choice(c)).unwrap();
            assert_eq!(v.node, all[j], "{c:?}");
            let g: Vec<[f32; 3]> = grads[j].iter().map(|g| g.map(|x| x as f32)).collect();
            assert_eq!(v.grad, g, "{c:?}");
            // a component's gradient made alone
            let alone = crate::gradient::cell_gradients(
                &m,
                &t.centre,
                &comps[j],
                twin.as_ref()
                    .map(|b| (b[j].as_slice(), t.face_centre.as_slice())),
                &|c, out| t.near(c, out),
            );
            assert_eq!(alone, grads[j]);
            assert_eq!(set.range(&m, &choice(c)), v.range);
        }
        let mag = set.values(&m, &choice(Component::Magnitude)).unwrap();
        for p in 0..m.points.len() {
            let l = (all[0][p].powi(2) + all[1][p].powi(2) + all[2][p].powi(2)).sqrt();
            assert_eq!(mag.node[p].to_bits(), l.to_bits());
        }
        let own = crate::gradient::cell_gradients(&m, &t.centre, &mag.cell, None, &|c, out| {
            t.near(c, out)
        });
        let own: Vec<[f32; 3]> = own.iter().map(|g| g.map(|x| x as f32)).collect();
        if twin.is_none() {
            assert_eq!(mag.grad, own);
        }
        let fresh = FieldSet::new(set.results.clone());
        assert_eq!(
            fresh.range(&m, &choice(Component::Magnitude)),
            mag.range,
            "without the values made"
        );
    }

    /// Invariant: a vector's magnitude at the nodes is the length of the
    /// interpolated vector, components are taken as asked, and a scalar
    /// ignores the component; the title names the scalar the legend
    /// shows without words to translate.
    #[test]
    fn vectors_and_components() {
        let (m, set) = channel();
        let mag = set
            .values(
                &m,
                &FieldChoice {
                    name: "velocity".into(),
                    component: Component::Magnitude,
                },
            )
            .unwrap();
        let x = set
            .values(
                &m,
                &FieldChoice {
                    name: "velocity".into(),
                    component: Component::X,
                },
            )
            .unwrap();
        let y = set.node_values(&m, "velocity", Component::Y).unwrap();
        let z = set.node_values(&m, "velocity", Component::Z).unwrap();
        for p in 0..m.points.len() {
            let want = (x.node[p].powi(2) + y[p].powi(2) + z[p].powi(2)).sqrt();
            assert!((mag.node[p] - want).abs() < 1e-9 * (1.0 + want));
        }
        assert!(mag.vector);
        assert_eq!(mag.choice.title(true), "|velocity|");
        assert_eq!(x.choice.title(true), "velocity_x");
        let p = set
            .values(
                &m,
                &FieldChoice {
                    name: "pressure".into(),
                    component: Component::Y,
                },
            )
            .unwrap();
        assert_eq!(p.choice.component, Component::Magnitude);
        assert_eq!(p.choice.title(false), "pressure");
    }

    /// Invariant: a boundary field (y+) has values on the boundary faces
    /// and the boundary nodes only, none in the cells; cell values of a
    /// cell field come back in file order with their units; sampling a
    /// cell's value returns the cell field's value, and sampling by nodes
    /// at a node returns that node's value.
    #[test]
    fn boundary_fields_cells_and_samples() {
        let (m, set) = channel();
        let y = set
            .values(
                &m,
                &FieldChoice {
                    name: "y_plus".into(),
                    component: Component::Magnitude,
                },
            )
            .unwrap();
        assert_eq!(y.location, Location::Boundary);
        assert_eq!(y.boundary.len(), m.boundary.len());
        assert!(y.cell.iter().all(|x| x.is_nan()));
        let on_boundary: std::collections::HashSet<u32> =
            m.boundary.iter().flatten().copied().collect();
        for (p, v) in y.node.iter().enumerate() {
            assert_eq!(v.is_finite(), on_boundary.contains(&(p as u32)), "node {p}");
        }
        let rows = set.cell_values(&m, 3);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["velocity", "speed", "pressure", "k", "omega", "nu_t"]
        );
        assert_eq!(rows[0].value.len(), 3);
        assert_eq!(rows[0].unit.as_deref(), Some("m/s"));
        let pressure = set.get("pressure").unwrap().scalar(3, Component::Magnitude);
        let c = crate::inspect::cell_centroid(&m, 3);
        assert_eq!(
            set.sample(&m, "pressure", Component::Magnitude, 3, c, Interp::Cell),
            Some(pressure)
        );
        let n = set
            .node_values(&m, "pressure", Component::Magnitude)
            .unwrap();
        let node = m.cell(3).next().unwrap().nodes[0] as usize;
        let at_node = set
            .sample(
                &m,
                "pressure",
                Component::Magnitude,
                3,
                m.points[node],
                Interp::Nodes,
            )
            .unwrap();
        assert!((at_node - n[node]).abs() < 1e-12);
    }

    /// Invariant: the diverging map is picked only for a range centred on
    /// zero, a named map always wins, a range of one value widens around
    /// it, and n contour lines split the range into n + 1 even steps
    /// (11 lines on the edges of 12 bands).
    #[test]
    fn maps_ranges_and_contours() {
        let st = FieldStyle::default();
        assert_eq!(map_for(&st, (0.0, 5.0)).unwrap().id, "rainbow_desaturated");
        assert_eq!(map_for(&st, (-2.0, 2.1)).unwrap().id, "rainbow_uniform");
        assert_eq!(map_for(&st, (-0.5, 2.0)).unwrap().id, "rainbow_desaturated");
        let turbo = FieldStyle {
            map: Some("turbo".into()),
            ..FieldStyle::default()
        };
        assert_eq!(map_for(&turbo, (-1.0, 1.0)).unwrap().id, "turbo");
        assert_eq!(widen((3.0, 3.0)), (2.997, 3.003));
        assert_eq!(widen((0.0, 0.0)), (-1.0, 1.0));
        let lines = FieldStyle {
            lines: true,
            line_count: 11,
            ..FieldStyle::default()
        };
        let c = lines.contours((0.0, 12.0));
        assert_eq!(c.len(), 11);
        assert!((c[0] - 1.0).abs() < 1e-12 && (c[10] - 11.0).abs() < 1e-12);
        assert!(FieldStyle::default().contours((0.0, 1.0)).is_empty());
    }
}
