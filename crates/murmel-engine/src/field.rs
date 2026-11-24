//! Field results in the session: the fields read with the mesh
//! ([`FieldSet`]), the field the view colours by ([`FieldChoice`]), how
//! it is drawn ([`FieldStyle`]: flat, smooth or banded, contour lines,
//! colour map, range) and the values the surface takes to the GPU
//! ([`FieldValues`], [`FieldPaint`]).
//!
//! Smooth colouring needs values at the mesh nodes, and a finite-volume
//! solver writes them per cell. Each node takes the inverse distance
//! weighted mean of the cells around it, weights 1 / |node − cell
//! centroid| (D. Shepard, "A two-dimensional interpolation function for
//! irregularly-spaced data", Proc. ACM National Conference 1968,
//! pp. 517–524; decided by the maintainer 2026-11-24). Where the file
//! also has the field on the boundary faces, boundary nodes take those
//! face values instead, weighted the same way by the face centres. A
//! vector is interpolated by components and its magnitude taken at the
//! nodes.
//!
//! The values go to the GPU relative to [`FieldValues::origin`] (the
//! middle of the field's range), so they keep their precision in `f32`;
//! the range, the colour map, the bands and the contour lines are
//! uniforms, so changing them draws again without building the surface.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

pub use murmel_mesh::field::{Component, Field, Location, Results, range_of};
use murmel_mesh::{Mesh, NO_CELL};
use serde::{Deserialize, Serialize};

use crate::colormap::{self, Colormap};
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
    /// A colour map id (`colormap::ids`); `None` takes viridis, or
    /// cool_warm when the range is symmetric about zero (decided
    /// 2026-11-24).
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
        let mut cell_start = vec![0];
        let mut cell_nodes = Vec::new();
        let mut scratch: Vec<u32> = Vec::new();
        for c in 0..mesh.cell_count() {
            scratch.clear();
            scratch.extend(
                mesh.cell(c)
                    .flatten()
                    .copied()
                    .filter(|&i| (i as usize) < np),
            );
            scratch.sort_unstable();
            scratch.dedup();
            cell_nodes.extend_from_slice(&scratch);
            cell_start.push(cell_nodes.len());
        }
        let (point_start, point_cells) = invert(np, &cell_start, &cell_nodes);
        let centre = (0..mesh.cell_count())
            .map(|c| crate::inspect::cell_centroid(mesh, c))
            .collect();
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

/// Scalar `c` of a field whose entities have the values `comps` (one
/// list per component).
fn combine(comps: &[Vec<f64>], i: usize, c: Component) -> f64 {
    let at = |k: usize| comps.get(k).and_then(|v| v.get(i)).copied().unwrap_or(f64::NAN);
    match (comps.len(), c) {
        (1, _) => at(0),
        (_, Component::Magnitude) => (0..comps.len()).map(|k| at(k).powi(2)).sum::<f64>().sqrt(),
        (_, Component::X) => at(0),
        (_, Component::Y) => at(1),
        (_, Component::Z) => at(2),
    }
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

/// The fields of the open mesh.
#[derive(Clone, Debug, Default)]
pub struct FieldSet {
    pub results: Results,
    topology: OnceLock<Arc<Topology>>,
}

impl FieldSet {
    pub fn new(results: Results) -> Self {
        Self {
            results,
            topology: OnceLock::new(),
        }
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
    fn fits(f: &Field, mesh: &Mesh) -> bool {
        let n = match f.location {
            Location::Cell => mesh.cell_count(),
            Location::Point => mesh.points.len(),
            Location::Boundary => mesh.boundary.len(),
        };
        f.len() == n && f.values.len() == n * f.components.max(1)
    }

    fn topology(&self, mesh: &Mesh) -> &Topology {
        self.topology.get_or_init(|| Arc::new(Topology::build(mesh)))
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
    fn node_components(&self, mesh: &Mesh, f: &Field, comps: &[Vec<f64>], boundary: Option<&[Vec<f64>]>, p: usize) -> Vec<f64> {
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
                comps
                    .iter()
                    .map(|v| idw(at, t.cells_of(p), &t.centre, v))
                    .collect()
            }
        }
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
        Some(
            (0..mesh.points.len())
                .map(|p| pick(&self.node_components(mesh, f, &comps, twin.as_deref(), p), component))
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
                let centres: Vec<[f64; 3]> = nodes.iter().map(|&n| mesh.points[n as usize]).collect();
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
        let node_comps: Vec<Vec<f64>> = (0..np)
            .map(|p| self.node_components(mesh, f, &comps, twin.as_deref(), p))
            .collect();
        let node: Vec<f64> = node_comps.iter().map(|v| pick(v, component)).collect();
        let nc = mesh.cell_count();
        let (cell, boundary, range) = match f.location {
            Location::Cell => {
                let cell: Vec<f64> = (0..nc).map(|c| combine(&comps, c, component)).collect();
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
            range,
            origin,
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
    /// The smallest and largest finite value the file has: over the
    /// cells, the points or the boundary faces.
    pub range: Option<(f64, f64)>,
    /// Values reach the GPU relative to this.
    pub origin: f64,
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
        self.mesh.as_ref().map(|m| &m.fields)
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
    let fresh = display
        .field_values
        .as_ref()
        .is_some_and(|v| v.choice.name == choice.name && (!v.vector || v.choice.component == choice.component));
    if !fresh {
        display.field_values = fields.values(mesh, &choice).map(Arc::new);
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

    /// Invariant: node values are convex means of the cells around each
    /// node, so they lie inside the cell range; a linear field is
    /// reproduced closely in the interior (IDW is exact for a constant
    /// and close for a linear field on a regular grid); every node of a
    /// cell mesh gets a value. A weight on the wrong side of the division
    /// or a node left out would break one of these.
    #[test]
    fn node_values_by_inverse_distance() {
        let (m, set) = channel();
        let choice = FieldChoice {
            name: "pressure".into(),
            component: Component::Magnitude,
        };
        let v = set.values(&m, &choice).unwrap();
        let (lo, hi) = v.range.unwrap();
        assert_eq!(v.cell.len(), m.cell_count());
        assert_eq!(v.node.len(), m.points.len());
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
        let n = flat.node_values(&m, "pressure", Component::Magnitude).unwrap();
        assert!(n.iter().all(|&x| (x - 7.5).abs() < 1e-12));
    }

    /// Invariant: a vector's magnitude at the nodes is the length of the
    /// interpolated vector, components are taken as asked, and a scalar
    /// ignores the component; the title names the scalar the legend
    /// shows without words to translate.
    #[test]
    fn vectors_and_components() {
        let (m, set) = channel();
        let mag = set
            .values(&m, &FieldChoice { name: "velocity".into(), component: Component::Magnitude })
            .unwrap();
        let x = set
            .values(&m, &FieldChoice { name: "velocity".into(), component: Component::X })
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
            .values(&m, &FieldChoice { name: "pressure".into(), component: Component::Y })
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
            .values(&m, &FieldChoice { name: "y_plus".into(), component: Component::Magnitude })
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
        assert_eq!(names, ["velocity", "speed", "pressure", "k", "omega", "nu_t"]);
        assert_eq!(rows[0].value.len(), 3);
        assert_eq!(rows[0].unit.as_deref(), Some("m/s"));
        let pressure = set.get("pressure").unwrap().scalar(3, Component::Magnitude);
        let c = crate::inspect::cell_centroid(&m, 3);
        assert_eq!(
            set.sample(&m, "pressure", Component::Magnitude, 3, c, Interp::Cell),
            Some(pressure)
        );
        let n = set.node_values(&m, "pressure", Component::Magnitude).unwrap();
        let node = m.cell(3).next().unwrap()[0] as usize;
        let at_node = set
            .sample(&m, "pressure", Component::Magnitude, 3, m.points[node], Interp::Nodes)
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
        assert_eq!(map_for(&st, (0.0, 5.0)).unwrap().id, "viridis");
        assert_eq!(map_for(&st, (-2.0, 2.1)).unwrap().id, "cool_warm");
        assert_eq!(map_for(&st, (-0.5, 2.0)).unwrap().id, "viridis");
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
