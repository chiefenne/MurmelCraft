//! The cells of a VTKHDF `UnstructuredGrid`, built on all threads. The
//! cells are walked in units of `UNIT` cells in two passes, as the `.vtu`
//! reader walks its own (`vtu/cells.rs`): each unit first checks its
//! cells and counts what they give, then writes that at the place the
//! counts give. A unit reads only the rows of the datasets its cells use
//! (a window, [`Sets`]), and each thread walks a run of units one after
//! the other, so a grid of any size is read a few units at a time. The
//! mesh is the one a single walk in file order builds, and the first bad
//! cell in file order gives the error.

use std::cmp::max;
use std::ops::Range;

use super::{bad, count};
use crate::hdf5::Values;
use crate::vtu::{
    HEX_FACES, HEXAHEDRON, POLYGON, POLYHEDRON, PYRAMID, PYRAMID_FACES, QUAD, TETRA, TETRA_FACES,
    TRIANGLE, WEDGE, WEDGE_FACES,
};
use crate::{CellKind, MeshError, Polygons, par};

/// The cells of a unit (the last has the rest).
const UNIT: usize = 1 << 15;

/// A partition as the file counts it, and where its rows start in each
/// dataset (after the partitions before it).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Part {
    /// Its points and cells (checked before the walk).
    pub points: usize,
    pub cells: usize,
    /// `NumberOfConnectivityIds`, and the polyhedra's `NumberOfFaces`,
    /// `NumberOfPolyhedronToFaceIds` and `NumberOfFaceConnectivityIds`
    /// (0 without polyhedra), checked when the walk comes to it.
    pub conn: i64,
    pub faces: i64,
    pub to_faces: i64,
    pub face_conn: i64,
    pub point_at: usize,
    pub cell_at: usize,
    pub offset_at: usize,
    pub conn_at: usize,
    pub poly_offset_at: usize,
    pub to_faces_at: usize,
    pub face_offset_at: usize,
    pub face_conn_at: usize,
}

/// The partitions of a grid from their counts, each with where its rows
/// start. A negative count adds nothing here and fails when the walk
/// comes to its partition, as a walk in file order would fail there.
pub(super) fn parts(
    points: &[i64],
    cells: &[i64],
    conn: &[i64],
    poly: Option<[&[i64]; 3]>,
) -> Vec<Part> {
    let size = |v: i64| usize::try_from(v).unwrap_or(0);
    let mut at = Part::default();
    let mut out = Vec::with_capacity(points.len());
    for p in 0..points.len() {
        let mut part = at;
        part.points = size(points[p]);
        part.cells = size(cells[p]);
        part.conn = conn[p];
        if let Some([faces, to_faces, face_conn]) = poly {
            part.faces = faces.get(p).copied().unwrap_or(0);
            part.to_faces = to_faces.get(p).copied().unwrap_or(0);
            part.face_conn = face_conn.get(p).copied().unwrap_or(0);
        }
        out.push(part);
        at.point_at += part.points;
        at.cell_at += part.cells;
        at.offset_at += part.cells + 1;
        at.conn_at += size(part.conn);
        if poly.is_some() {
            at.poly_offset_at += part.cells + 1;
            at.to_faces_at += size(part.to_faces);
            at.face_offset_at += size(part.faces) + 1;
            at.face_conn_at += size(part.face_conn);
        }
    }
    out
}

/// The polyhedra's datasets (VTKHDF 2.4).
#[derive(Clone, Copy)]
pub(super) struct Poly<'a> {
    pub offsets: &'a Values<'a>,
    pub to_faces: &'a Values<'a>,
    pub face_offsets: &'a Values<'a>,
    pub face_conn: &'a Values<'a>,
}

/// The mesh point of each row of `Points`.
pub(super) enum PointMap {
    /// Row `r` is point `base + r`.
    Offset(u32),
    /// Row `r` is point `rows[r]` (points joined by their global ids).
    Rows(Vec<u32>),
}

impl PointMap {
    fn get(&self, row: usize) -> u32 {
        match self {
            PointMap::Offset(base) => base + row as u32,
            PointMap::Rows(rows) => rows[row],
        }
    }
}

/// A grid's datasets and what its cells need besides.
#[derive(Clone, Copy)]
pub(super) struct Input<'a> {
    pub parts: &'a [Part],
    pub types: &'a Values<'a>,
    pub offsets: &'a Values<'a>,
    pub conn: &'a Values<'a>,
    pub poly: Option<Poly<'a>>,
    pub ghost: Option<&'a Values<'a>>,
    /// The `zone` cell data, when there is no zone for the whole grid.
    pub zone_data: Option<&'a Values<'a>>,
    pub zone: Option<u32>,
    pub points: &'a PointMap,
}

/// The datasets of the cells, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Set {
    Types,
    Offsets,
    Connectivity,
    PolyhedronOffsets,
    PolyhedronToFaces,
    FaceOffsets,
    FaceConnectivity,
    /// `CellData/vtkGhostType`
    Ghost,
    /// `CellData/zone`
    Zone,
}

/// Where the cells' datasets come from.
pub(super) trait Sets: Sync {
    /// The values of `set` with (at least) the values `rows` (element
    /// numbers; those past the end are left out), numbered as in the
    /// whole dataset; `None` when the grid has no such dataset.
    fn rows(&self, set: Set, rows: Range<usize>) -> Result<Option<Values<'_>>, MeshError>;
}

/// What the cells of a grid need besides the datasets.
#[derive(Clone, Copy)]
pub(super) struct Grid<'a> {
    pub parts: &'a [Part],
    /// Whether the grid has the polyhedron datasets.
    pub poly: bool,
    /// The zone of every cell, when the grid has one.
    pub zone: Option<u32>,
    pub points: &'a PointMap,
    /// The mesh points, for turning polyhedra outward.
    pub xyz: &'a [[f64; 3]],
}

/// The partitions a walk of rows `cells` enters, in order, each with the
/// rows of its own that the walk reads (local to the partition): the
/// partitions without cells at its start, and with `last` those after its
/// end, so that the walks of consecutive ranges enter every partition
/// once.
fn spans(
    parts: &[Part],
    cells: Range<usize>,
    last: bool,
) -> impl Iterator<Item = (usize, Range<usize>)> + '_ {
    // the first partition that ends after the first cell, or starts at it
    // without cells
    let mut p = parts.partition_point(|q| {
        let end = q.cell_at + q.cells;
        end < cells.start || (end == cells.start && q.cells > 0)
    });
    let mut row = cells.start;
    std::iter::from_fn(move || {
        if p >= parts.len() || !(row < cells.end || last) {
            return None;
        }
        let part = parts[p];
        let until = cells.end.min(part.cell_at + part.cells).max(row);
        let span = (p, row - part.cell_at..until - part.cell_at);
        row = until;
        p += 1;
        Some(span)
    })
}

/// The rows of the datasets a walk of rows `cells` reads, and the
/// values in them. A row range is widened to all the rows the walk may
/// read after its checks: the connectivity between the offsets of the
/// cells (clamped to their partition's count), and so on, so that the
/// walk never reads a value outside its window.
struct Window<'s> {
    types: Values<'s>,
    offsets: Values<'s>,
    conn: Values<'s>,
    poly: Option<[Values<'s>; 4]>,
    ghost: Option<Values<'s>>,
    zone: Option<Values<'s>>,
}

/// The smallest range holding `a` and `b` (an empty one holds nothing).
fn join(a: Range<usize>, b: Range<usize>) -> Range<usize> {
    if a.is_empty() {
        b
    } else if b.is_empty() {
        a
    } else {
        a.start.min(b.start)..a.end.max(b.end)
    }
}

/// The values `at + lo .. at + hi` of `v` read as positions into
/// `0..=count` (clamped), as the range of `[min, max)` they span, shifted
/// by `base`; nothing when `count` is negative.
fn reach(v: &Values, at: usize, rows: Range<usize>, count: i64, base: usize) -> Range<usize> {
    let Ok(count) = usize::try_from(count) else {
        return 0..0;
    };
    let held = v.held();
    let (mut lo, mut hi) = (usize::MAX, 0);
    for i in at + rows.start..at + rows.end {
        if held.contains(&i) {
            let x = v.int(i).clamp(0, count as i64) as usize;
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    if lo > hi { 0..0 } else { base + lo..base + hi }
}

impl<'s> Window<'s> {
    /// The window of the walk of rows `cells`.
    fn of(
        grid: &Grid,
        sets: &'s dyn Sets,
        cells: Range<usize>,
        last: bool,
    ) -> Result<Self, MeshError> {
        let spans: Vec<(usize, Range<usize>)> = spans(grid.parts, cells.clone(), last)
            .filter(|(_, r)| !r.is_empty())
            .collect();
        let need = |set: Set, rows: Range<usize>| -> Result<Values<'s>, MeshError> {
            Ok(sets.rows(set, rows)?.unwrap_or(Values::none()))
        };
        let parts = grid.parts;
        // a cell's offsets and the one after its last
        let offset_rows = |at: fn(&Part) -> usize| {
            spans.iter().fold(0..0, |all, (p, r)| {
                let a = at(&parts[*p]);
                join(all, a + r.start..a + r.end + 1)
            })
        };
        let types = need(Set::Types, cells.clone())?;
        let ghost = sets.rows(Set::Ghost, cells.clone())?;
        let zone = sets.rows(Set::Zone, cells.clone())?;
        let offsets = need(Set::Offsets, offset_rows(|p| p.offset_at))?;
        let conn_rows = spans.iter().fold(0..0, |all, (p, r)| {
            let q = &parts[*p];
            join(
                all,
                reach(&offsets, q.offset_at, r.start..r.end + 1, q.conn, q.conn_at),
            )
        });
        let conn = need(Set::Connectivity, conn_rows)?;
        let poly = if grid.poly {
            let po = need(Set::PolyhedronOffsets, offset_rows(|p| p.poly_offset_at))?;
            let to_rows = spans.iter().fold(0..0, |all, (p, r)| {
                let q = &parts[*p];
                join(
                    all,
                    reach(
                        &po,
                        q.poly_offset_at,
                        r.start..r.end + 1,
                        q.to_faces,
                        q.to_faces_at,
                    ),
                )
            });
            let to_faces = need(Set::PolyhedronToFaces, to_rows)?;
            // each face its offset and the one after, partition by
            // partition (local rows of FaceOffsets)
            let mut face_rows = 0..0;
            let mut faces = Vec::with_capacity(spans.len());
            for (p, r) in &spans {
                let q = &parts[*p];
                let k = reach(&po, q.poly_offset_at, r.start..r.end + 1, q.to_faces, 0);
                let f = reach(&to_faces, q.to_faces_at, k, q.faces.saturating_sub(1), 0);
                let rows_of = usize::try_from(q.faces).map_or(0, |n| n + 1);
                let f = f.start.min(rows_of)..(f.end + 2).min(rows_of);
                face_rows = join(
                    face_rows,
                    q.face_offset_at + f.start..q.face_offset_at + f.end,
                );
                faces.push((q, f));
            }
            let fo = need(Set::FaceOffsets, face_rows)?;
            let fc_rows = faces.iter().fold(0..0, |all, (q, f)| {
                join(
                    all,
                    reach(
                        &fo,
                        q.face_offset_at,
                        f.clone(),
                        q.face_conn,
                        q.face_conn_at,
                    ),
                )
            });
            let fc = need(Set::FaceConnectivity, fc_rows)?;
            Some([po, to_faces, fo, fc])
        } else {
            None
        };
        Ok(Window {
            types,
            offsets,
            conn,
            poly,
            ghost,
            zone,
        })
    }

    fn input<'a>(&'a self, grid: &Grid<'a>) -> Input<'a> {
        Input {
            parts: grid.parts,
            types: &self.types,
            offsets: &self.offsets,
            conn: &self.conn,
            poly: self
                .poly
                .as_ref()
                .map(|[offsets, to_faces, face_offsets, face_conn]| Poly {
                    offsets,
                    to_faces,
                    face_offsets,
                    face_conn,
                }),
            ghost: self.ghost.as_ref(),
            zone_data: self.zone.as_ref(),
            zone: grid.zone,
            points: grid.points,
        }
    }
}

/// What the cells of a grid became. Face and node numbers start at 0
/// for the grid.
pub(super) struct Cells {
    pub polys: Polygons,
    /// Each face's smallest point, for linking.
    pub least: Vec<u32>,
    /// Where each cell's faces end.
    pub cell_end: Vec<u32>,
    pub zone: Vec<u32>,
    pub kind: Vec<CellKind>,
    /// The row of each cell in the cell datasets.
    pub rows: Vec<u32>,
    /// The polygons among the cells (boundary faces).
    pub boundary: Polygons,
    pub zone_max: Option<u32>,
}

/// A partition the walk is in: its rows, checked against its datasets.
struct Here {
    part: Part,
    conn: usize,
    to_faces: usize,
    faces: usize,
    face_conn: usize,
}

/// Check partition `part` the way a walk in file order meets it.
fn enter(input: &Input, part: Part) -> Result<Here, MeshError> {
    let conn = count(part.conn, "connectivity ids")?;
    if part.offset_at + part.cells + 1 > input.offsets.len() {
        return Err(bad("Offsets is shorter than the cells"));
    }
    if part.conn_at + conn > input.conn.len() {
        return Err(bad("Connectivity is shorter than NumberOfConnectivityIds"));
    }
    if part.cell_at + part.cells > input.types.len() {
        return Err(bad("Types is shorter than the cells"));
    }
    let mut here = Here {
        part,
        conn,
        to_faces: 0,
        faces: 0,
        face_conn: 0,
    };
    if let Some(p) = input.poly {
        here.faces = count(part.faces, "faces")?;
        here.to_faces = count(part.to_faces, "polyhedron faces")?;
        here.face_conn = count(part.face_conn, "face ids")?;
        if part.poly_offset_at + part.cells + 1 > p.offsets.len()
            || part.to_faces_at + here.to_faces > p.to_faces.len()
            || part.face_offset_at + here.faces + 1 > p.face_offsets.len()
            || part.face_conn_at + here.face_conn > p.face_conn.len()
        {
            return Err(bad("the polyhedron arrays are shorter than their counts"));
        }
    }
    Ok(here)
}

/// What a walk hands on, cell by cell.
trait Sink {
    /// Whether it needs the mesh points of the faces (a count needs only
    /// their number; the points are checked either way).
    const POINTS: bool;
    /// A face of the volume cell being read.
    fn face(&mut self, nodes: &[u32]);
    /// The volume cell whose faces came before ends; a polyhedron's faces
    /// are turned outward.
    fn cell(&mut self, zone: u32, kind: CellKind, row: usize, polyhedron: bool);
    fn polygon(&mut self, nodes: &[u32]);
}

/// The point of local id `v` in partition `here` (0 when `map` is
/// false: the id is only checked).
fn point(input: &Input, here: &Here, v: i64, map: bool) -> Result<u32, MeshError> {
    let n = here.part.points;
    match usize::try_from(v).ok().filter(|&v| v < n) {
        Some(v) if map => Ok(input.points.get(here.part.point_at + v)),
        Some(_) => Ok(0),
        None => Err(MeshError::new(
            "MESH006",
            format!("point {v} is out of range 0..{n}"),
        )),
    }
}

/// The local range `a..b` of a dataset row pair, when it is one.
fn span(a: i64, b: i64) -> Option<Range<usize>> {
    Some(usize::try_from(a).ok()?..usize::try_from(b).ok()?)
}

/// Walk the cells of rows `cells`, checking each partition as it comes
/// to it and each cell as it goes; the error is that of the first bad
/// partition or cell. A range enters the partitions without cells at its
/// start, and the `last` range those after its end, so every partition
/// is checked as a walk over all of them would check it.
fn walk<S: Sink>(
    input: &Input,
    cells: Range<usize>,
    last: bool,
    sink: &mut S,
) -> Result<(), MeshError> {
    let mut ids: Vec<u32> = Vec::with_capacity(64);
    let mut face: Vec<u32> = Vec::with_capacity(16);
    for (p, rows) in spans(input.parts, cells, last) {
        let here = enter(input, input.parts[p])?;
        let part = here.part;
        for c in rows {
            let r = part.cell_at + c;
            if input.ghost.is_some_and(|g| g.int(r) & 1 != 0) {
                continue;
            }
            let (s, e) = (
                input.offsets.int(part.offset_at + c),
                input.offsets.int(part.offset_at + c + 1),
            );
            let nodes = span(s, e)
                .filter(|n| n.start <= n.end && n.end <= here.conn)
                .ok_or_else(|| MeshError::new("MESH006", format!("cell {c}: offsets {s}..{e}")))?;
            let nodes = part.conn_at + nodes.start..part.conn_at + nodes.end;
            let zone = match (input.zone, input.zone_data) {
                (Some(z), _) => z,
                (None, Some(zd)) => {
                    let v = if r < zd.len() { zd.float(r) } else { 0.0 };
                    if !(0.0..1e6).contains(&v) {
                        return Err(MeshError::new("MESH006", format!("cell {c}: zone {v}")));
                    }
                    v as u32
                }
                (None, None) => 0,
            };
            let t = input.types.int(r);
            let table = match t {
                TETRA => Some(TETRA_FACES),
                HEXAHEDRON => Some(HEX_FACES),
                WEDGE => Some(WEDGE_FACES),
                PYRAMID => Some(PYRAMID_FACES),
                _ => None,
            };
            if let Some(table) = table {
                let need = table
                    .iter()
                    .flat_map(|f| f.iter())
                    .max()
                    .map_or(0, |m| m + 1);
                if nodes.len() != need {
                    return Err(MeshError::new(
                        "MESH006",
                        format!("cell {c}: type {t} with {} points", nodes.len()),
                    ));
                }
                ids.clear();
                for i in nodes {
                    ids.push(point(input, &here, input.conn.int(i), S::POINTS)?);
                }
                for f in table {
                    face.clear();
                    face.extend(f.iter().map(|&i| ids[i]));
                    sink.face(&face);
                }
                let kind = CellKind::classify(table.iter().map(|f| f.len()));
                sink.cell(zone, kind, r, false);
            } else if t == POLYHEDRON {
                let Some(poly) = input.poly else {
                    return Err(MeshError::new(
                        "MESH009",
                        format!("cell {c}: PolyhedronOffsets"),
                    ));
                };
                let range = span(
                    poly.offsets.int(part.poly_offset_at + c),
                    poly.offsets.int(part.poly_offset_at + c + 1),
                )
                .filter(|r| r.start < r.end && r.end <= here.to_faces)
                .ok_or_else(|| MeshError::new("MESH009", format!("cell {c}: PolyhedronOffsets")))?;
                let (mut faces, mut tris, mut quads) = (0, 0, 0);
                for k in range {
                    let f = poly.to_faces.int(part.to_faces_at + k);
                    let fr = usize::try_from(f)
                        .ok()
                        .filter(|&f| f < here.faces)
                        .and_then(|f| {
                            span(
                                poly.face_offsets.int(part.face_offset_at + f),
                                poly.face_offsets.int(part.face_offset_at + f + 1),
                            )
                        })
                        .filter(|r| r.end >= r.start + 3 && r.end <= here.face_conn)
                        .ok_or_else(|| MeshError::new("MESH009", format!("cell {c}: face {f}")))?;
                    face.clear();
                    for i in fr {
                        let v = poly.face_conn.int(part.face_conn_at + i);
                        face.push(point(input, &here, v, S::POINTS)?);
                    }
                    faces += 1;
                    match face.len() {
                        3 => tris += 1,
                        4 => quads += 1,
                        _ => {}
                    }
                    sink.face(&face);
                }
                sink.cell(zone, CellKind::of_counts(faces, tris, quads), r, true);
            } else if matches!(t, TRIANGLE | POLYGON | QUAD) {
                if nodes.len() < 3 {
                    return Err(MeshError::new(
                        "MESH006",
                        format!("cell {c}: polygon with {} points", nodes.len()),
                    ));
                }
                face.clear();
                for i in nodes {
                    face.push(point(input, &here, input.conn.int(i), S::POINTS)?);
                }
                sink.polygon(&face);
            } else {
                return Err(MeshError::new(
                    "MESH008",
                    format!("cell {c} has VTK type {t}"),
                ));
            }
        }
    }
    Ok(())
}

/// What a range of cells gives.
#[derive(Clone, Copy, Default)]
struct Count {
    cells: usize,
    faces: usize,
    nodes: usize,
    polygons: usize,
    polygon_nodes: usize,
    zone_max: Option<u32>,
}

impl Count {
    fn plus(self, o: Count) -> Count {
        Count {
            cells: self.cells + o.cells,
            faces: self.faces + o.faces,
            nodes: self.nodes + o.nodes,
            polygons: self.polygons + o.polygons,
            polygon_nodes: self.polygon_nodes + o.polygon_nodes,
            zone_max: max(self.zone_max, o.zone_max),
        }
    }
}

impl Sink for Count {
    const POINTS: bool = false;

    fn face(&mut self, nodes: &[u32]) {
        self.faces += 1;
        self.nodes += nodes.len();
    }

    fn cell(&mut self, zone: u32, _: CellKind, _: usize, _: bool) {
        self.cells += 1;
        self.zone_max = max(self.zone_max, Some(zone));
    }

    fn polygon(&mut self, nodes: &[u32]) {
        self.polygons += 1;
        self.polygon_nodes += nodes.len();
    }
}

/// A range's pieces of the arrays, written as its walk goes.
struct Write<'a> {
    base: Count,
    done: Count,
    xyz: &'a [[f64; 3]],
    face_end: &'a mut [u32],
    least: &'a mut [u32],
    nodes: &'a mut [u32],
    cell_end: &'a mut [u32],
    zone: &'a mut [u32],
    kind: &'a mut [CellKind],
    rows: &'a mut [u32],
    polygon_end: &'a mut [u32],
    polygon_nodes: &'a mut [u32],
    /// Where the faces of this cell lie in `nodes`.
    bounds: Vec<Range<usize>>,
}

impl Write<'_> {
    /// Turn the faces of the cell just written outward when they point
    /// in: its signed volume, by the divergence theorem over fan
    /// triangles, is negative. VTK asks for outward polyhedron faces but
    /// does not check them.
    fn orient(&mut self) {
        let p = self.xyz;
        let mut v = 0.0;
        for b in &self.bounds {
            let face = &self.nodes[b.clone()];
            let p0 = p[face[0] as usize];
            for k in 1..face.len().saturating_sub(1) {
                let (a, b) = (p[face[k] as usize], p[face[k + 1] as usize]);
                v += p0[0] * (a[1] * b[2] - a[2] * b[1]) - p0[1] * (a[0] * b[2] - a[2] * b[0])
                    + p0[2] * (a[0] * b[1] - a[1] * b[0]);
            }
        }
        if v < 0.0 {
            for b in &self.bounds {
                self.nodes[b.clone()].reverse();
            }
        }
    }
}

impl Sink for Write<'_> {
    const POINTS: bool = true;

    fn face(&mut self, nodes: &[u32]) {
        let (f, at) = (self.done.faces, self.done.nodes);
        self.nodes[at..at + nodes.len()].copy_from_slice(nodes);
        self.bounds.push(at..at + nodes.len());
        self.done.nodes += nodes.len();
        self.face_end[f] = (self.base.nodes + self.done.nodes) as u32;
        self.least[f] = nodes.iter().copied().min().unwrap_or(u32::MAX);
        self.done.faces += 1;
    }

    fn cell(&mut self, zone: u32, kind: CellKind, row: usize, polyhedron: bool) {
        if polyhedron {
            self.orient();
        }
        let c = self.done.cells;
        self.cell_end[c] = (self.base.faces + self.done.faces) as u32;
        self.zone[c] = zone;
        self.kind[c] = kind;
        self.rows[c] = row as u32;
        self.done.cells += 1;
        self.bounds.clear();
    }

    fn polygon(&mut self, nodes: &[u32]) {
        let (p, at) = (self.done.polygons, self.done.polygon_nodes);
        self.polygon_nodes[at..at + nodes.len()].copy_from_slice(nodes);
        self.done.polygon_nodes += nodes.len();
        self.polygon_end[p] = (self.base.polygon_nodes + self.done.polygon_nodes) as u32;
        self.done.polygons += 1;
    }
}

/// `data` cut where each range's piece starts (`at`, increasing from 0).
fn pieces<T>(data: &mut [T], at: impl Iterator<Item = usize>) -> Vec<&mut [T]> {
    let len = data.len();
    let cuts: Vec<usize> = at.chain([len]).collect();
    par::split_mut(data, &cuts)
}

/// `f` for each of `items` on all threads, each thread taking a run of
/// consecutive items one after the other; results in order.
fn in_runs<T: Send, R: Send>(items: Vec<T>, f: impl Fn(T) -> R + Sync) -> Vec<R> {
    let mut items = items.into_iter();
    let runs: Vec<Vec<T>> = par::ranges(items.len(), 1)
        .into_iter()
        .map(|r| items.by_ref().take(r.len()).collect())
        .collect();
    par::each(runs, |run| run.into_iter().map(&f).collect::<Vec<R>>())
        .into_iter()
        .flatten()
        .collect()
}

/// Build the cells of the `total` rows of `grid`, reading the datasets
/// from `sets` a unit at a time.
pub(super) fn read(grid: &Grid, sets: &dyn Sets, total: usize) -> Result<Cells, MeshError> {
    read_in(grid, sets, total, UNIT)
}

/// [`read`] in units of `unit` cells.
fn read_in(grid: &Grid, sets: &dyn Sets, total: usize, unit: usize) -> Result<Cells, MeshError> {
    let unit = unit.max(1);
    let numbered: Vec<(usize, Range<usize>)> = (0..total.div_ceil(unit).max(1))
        .map(|i| (i, (i * unit).min(total)..((i + 1) * unit).min(total)))
        .collect();
    let n = numbered.len();
    let counts = in_runs(numbered.clone(), |(i, r)| {
        let last = i + 1 == n;
        let window = Window::of(grid, sets, r.clone(), last)?;
        let mut count = Count::default();
        walk(&window.input(grid), r, last, &mut count).map(|()| count)
    });
    let mut base = Vec::with_capacity(counts.len());
    let mut all = Count::default();
    for count in counts {
        base.push(all);
        all = all.plus(count?);
    }

    let mut polys = Polygons {
        start: vec![0; all.faces + 1],
        nodes: vec![0; all.nodes],
    };
    let mut least = vec![0; all.faces];
    let mut cell_end = vec![0; all.cells];
    let mut zone = vec![0; all.cells];
    let mut kind = vec![CellKind::Polyhedron; all.cells];
    let mut rows = vec![0; all.cells];
    let mut boundary = Polygons {
        start: vec![0; all.polygons + 1],
        nodes: vec![0; all.polygon_nodes],
    };
    let at = |f: fn(&Count) -> usize| base.iter().map(f).collect::<Vec<_>>();
    let (faces, nodes, cells) = (at(|b| b.faces), at(|b| b.nodes), at(|b| b.cells));
    let (polygons, polygon_nodes) = (at(|b| b.polygons), at(|b| b.polygon_nodes));
    let mut jobs = Vec::with_capacity(n);
    let mut face_end = pieces(&mut polys.start[1..], faces.iter().copied()).into_iter();
    let mut face_least = pieces(&mut least, faces.iter().copied()).into_iter();
    let mut node = pieces(&mut polys.nodes, nodes.iter().copied()).into_iter();
    let mut ends = pieces(&mut cell_end, cells.iter().copied()).into_iter();
    let mut zones = pieces(&mut zone, cells.iter().copied()).into_iter();
    let mut kinds = pieces(&mut kind, cells.iter().copied()).into_iter();
    let mut cell_rows = pieces(&mut rows, cells.iter().copied()).into_iter();
    let mut polygon_end = pieces(&mut boundary.start[1..], polygons.iter().copied()).into_iter();
    let mut polygon_node = pieces(&mut boundary.nodes, polygon_nodes.iter().copied()).into_iter();
    for ((i, r), &base) in numbered.into_iter().zip(&base) {
        let (
            Some(face_end),
            Some(least),
            Some(nodes),
            Some(cell_end),
            Some(zone),
            Some(kind),
            Some(rows),
            Some(polygon_end),
            Some(polygon_nodes),
        ) = (
            face_end.next(),
            face_least.next(),
            node.next(),
            ends.next(),
            zones.next(),
            kinds.next(),
            cell_rows.next(),
            polygon_end.next(),
            polygon_node.next(),
        )
        else {
            break;
        };
        let write = Write {
            base,
            done: Count::default(),
            xyz: grid.xyz,
            face_end,
            least,
            nodes,
            cell_end,
            zone,
            kind,
            rows,
            polygon_end,
            polygon_nodes,
            bounds: Vec::new(),
        };
        jobs.push((i, r, write));
    }
    for written in in_runs(jobs, |(i, r, mut write)| {
        let last = i + 1 == n;
        let window = Window::of(grid, sets, r.clone(), last)?;
        walk(&window.input(grid), r, last, &mut write)
    }) {
        written?;
    }
    Ok(Cells {
        polys,
        least,
        cell_end,
        zone,
        kind,
        rows,
        boundary,
        zone_max: all.zone_max,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The points of each partition.
    const POINTS: usize = 12;

    /// Every cell type the walk knows, mixed.
    const MIXED: [i64; 9] = [
        POLYHEDRON, TETRA, HEXAHEDRON, QUAD, WEDGE, POLYHEDRON, PYRAMID, TRIANGLE, POLYGON,
    ];
    /// Standard cells and polygons only (no polyhedron datasets).
    const STANDARD: [i64; 6] = [TETRA, HEXAHEDRON, QUAD, WEDGE, PYRAMID, TRIANGLE];

    /// The datasets of an `UnstructuredGrid` and the mesh points.
    #[derive(Clone, Default)]
    struct Data {
        poly: bool,
        points: Vec<i64>,
        cells: Vec<i64>,
        conn_ids: Vec<i64>,
        faces: Vec<i64>,
        to_face_ids: Vec<i64>,
        face_ids: Vec<i64>,
        types: Vec<i64>,
        offsets: Vec<i64>,
        conn: Vec<i64>,
        poly_offsets: Vec<i64>,
        to_faces: Vec<i64>,
        face_offsets: Vec<i64>,
        face_conn: Vec<i64>,
        ghost: Vec<i64>,
        zone: Vec<f64>,
        xyz: Vec<[f64; 3]>,
    }

    impl Data {
        fn parts(&self) -> Vec<Part> {
            let counts = [&self.faces[..], &self.to_face_ids[..], &self.face_ids[..]];
            parts(
                &self.points,
                &self.cells,
                &self.conn_ids,
                self.poly.then_some(counts),
            )
        }

        fn total(&self) -> usize {
            self.cells.iter().map(|&n| n.max(0) as usize).sum()
        }
    }

    /// Partitions of `sizes` cells (and `POINTS` points each) of the types
    /// `kinds` in a mixed order: polyhedra of 4 to 8 faces of 3 to 6
    /// points, standard cells, polygons of 3 to 6 points; every 9th cell
    /// from the 5th a ghost; zones 0 to 2.
    fn grid(sizes: &[usize], kinds: &[i64]) -> Data {
        let mut g = Data {
            poly: kinds.contains(&POLYHEDRON),
            ..Data::default()
        };
        let mut seed = 11u64;
        let mut next = |m: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) % m) as i64
        };
        let mut row = 0;
        for &n in sizes {
            let (mut conn, mut faces, mut to_faces, mut face_conn) = (0, 0, 0, 0);
            g.offsets.push(0);
            if g.poly {
                g.poly_offsets.push(0);
                g.face_offsets.push(0);
            }
            for _ in 0..n {
                let t = kinds[(row * 5 + row / 7) % kinds.len()];
                let corners = match t {
                    TETRA => 4,
                    HEXAHEDRON => 8,
                    WEDGE => 6,
                    PYRAMID => 5,
                    TRIANGLE => 3,
                    QUAD => 4,
                    _ => 3 + next(4),
                };
                for _ in 0..corners {
                    g.conn.push(next(POINTS as u64));
                }
                conn += corners;
                g.offsets.push(conn);
                if t == POLYHEDRON {
                    for _ in 0..4 + next(5) {
                        g.to_faces.push(faces);
                        to_faces += 1;
                        let k = 3 + next(4);
                        for _ in 0..k {
                            g.face_conn.push(next(POINTS as u64));
                        }
                        face_conn += k;
                        faces += 1;
                        g.face_offsets.push(face_conn);
                    }
                }
                if g.poly {
                    g.poly_offsets.push(to_faces);
                }
                g.types.push(t);
                g.ghost.push(i64::from(row % 9 == 4));
                g.zone.push(next(3) as f64);
                row += 1;
            }
            g.points.push(POINTS as i64);
            g.cells.push(n as i64);
            g.conn_ids.push(conn);
            g.faces.push(faces);
            g.to_face_ids.push(to_faces);
            g.face_ids.push(face_conn);
        }
        let rows = sizes.len() * POINTS;
        g.xyz = (0..rows + 3)
            .map(|_| {
                let mut c = || next(1000) as f64 / 7.0;
                [c(), c(), c()]
            })
            .collect();
        g
    }

    type Got = (
        Polygons,
        Vec<u32>,
        Vec<u32>,
        Vec<u32>,
        Vec<CellKind>,
        Vec<u32>,
        Polygons,
        Option<u32>,
    );

    /// The datasets of [`Data`] as a file hands them out: the rows asked
    /// for (a window), or each dataset whole.
    struct Windows<'a> {
        d: &'a Data,
        whole: bool,
    }

    impl Sets for Windows<'_> {
        fn rows(&self, set: Set, rows: Range<usize>) -> Result<Option<Values<'_>>, MeshError> {
            let d = self.d;
            let poly = matches!(
                set,
                Set::PolyhedronOffsets
                    | Set::PolyhedronToFaces
                    | Set::FaceOffsets
                    | Set::FaceConnectivity
            );
            if poly && !d.poly {
                return Ok(None);
            }
            let window = |len: usize| {
                if self.whole {
                    0..len
                } else {
                    let start = rows.start.min(len);
                    start..rows.end.clamp(start, len)
                }
            };
            let ints: &[i64] = match set {
                Set::Types => &d.types,
                Set::Offsets => &d.offsets,
                Set::Connectivity => &d.conn,
                Set::PolyhedronOffsets => &d.poly_offsets,
                Set::PolyhedronToFaces => &d.to_faces,
                Set::FaceOffsets => &d.face_offsets,
                Set::FaceConnectivity => &d.face_conn,
                Set::Ghost => &d.ghost,
                Set::Zone => {
                    let r = window(d.zone.len());
                    return Ok(Some(Values::of_floats_in(&d.zone, r)));
                }
            };
            Ok(Some(Values::of_ints_in(ints, window(ints.len()))))
        }
    }

    /// The units the tests read in: one cell, a few, a size that ends
    /// inside partitions, and more than any test grid holds.
    const UNITS: [usize; 6] = [1, 2, 3, 7, 16, 1000];

    /// The cells of `g` read on `threads` threads in units of `unit`
    /// cells, each unit given only its window of the datasets; a `unit`
    /// of 0 reads one unit from the whole datasets (one walk).
    fn read_on(
        threads: usize,
        unit: usize,
        g: &Data,
        map: &PointMap,
    ) -> Result<Got, (&'static str, String)> {
        let parts = g.parts();
        let grid = Grid {
            parts: &parts,
            poly: g.poly,
            zone: None,
            points: map,
            xyz: &g.xyz,
        };
        let sets = Windows {
            d: g,
            whole: unit == 0,
        };
        let unit = if unit == 0 { usize::MAX } else { unit };
        par::with_threads(threads, || read_in(&grid, &sets, g.total(), unit))
            .map(|c| {
                (
                    c.polys, c.least, c.cell_end, c.zone, c.kind, c.rows, c.boundary, c.zone_max,
                )
            })
            .map_err(|e| (e.code, e.message))
    }

    /// The point maps the tests use: an offset, and rows joined out of
    /// order.
    fn maps(g: &Data) -> [PointMap; 2] {
        let rows = g.points.len() * POINTS;
        [
            PointMap::Offset(3),
            PointMap::Rows((0..rows).map(|r| ((r * 7) % (rows + 3)) as u32).collect()),
        ]
    }

    /// What one walk must give, checked against the datasets.
    fn check(g: &Data, map: &PointMap, got: &Got) {
        let (polys, least, cell_end, zone, kind, rows, boundary, zone_max) = got;
        let parts = g.parts();
        let keep = |want: &dyn Fn(i64) -> bool| -> Vec<u32> {
            (0..g.types.len())
                .filter(|&r| g.ghost[r] == 0 && want(g.types[r]))
                .map(|r| r as u32)
                .collect()
        };
        let polygon = |t: i64| matches!(t, TRIANGLE | QUAD | POLYGON);
        assert_eq!(*rows, keep(&|t| !polygon(t)));
        assert_eq!(boundary.len(), keep(&polygon).len());
        assert_eq!(least.len(), polys.len());
        for (f, &l) in least.iter().enumerate() {
            assert_eq!(Some(l), polys.get(f).iter().copied().min());
        }
        assert_eq!(cell_end.len(), rows.len());
        assert_eq!(cell_end.last().map_or(0, |&e| e as usize), polys.len());
        for (c, &r) in rows.iter().enumerate() {
            let r = r as usize;
            assert_eq!(zone[c], g.zone[r] as u32);
            let faces = c.checked_sub(1).map_or(0, |b| cell_end[b] as usize)..cell_end[c] as usize;
            let p = parts.iter().rfind(|p| p.cell_at <= r).unwrap();
            let k = r - p.cell_at;
            let used: BTreeSet<u32> = faces.clone().flat_map(|f| polys.get(f).to_vec()).collect();
            if g.types[r] == POLYHEDRON {
                // the file's faces, all turned when their volume is
                // negative
                let point = |v: i64| map.get(p.point_at + v as usize);
                let mut want: Vec<Vec<u32>> = (g.poly_offsets[p.poly_offset_at + k]
                    ..g.poly_offsets[p.poly_offset_at + k + 1])
                    .map(|i| {
                        let f = g.to_faces[p.to_faces_at + i as usize] as usize;
                        (g.face_offsets[p.face_offset_at + f]
                            ..g.face_offsets[p.face_offset_at + f + 1])
                            .map(|j| point(g.face_conn[p.face_conn_at + j as usize]))
                            .collect()
                    })
                    .collect();
                let mut v = 0.0;
                for face in &want {
                    let p0 = g.xyz[face[0] as usize];
                    for i in 1..face.len() - 1 {
                        let (a, b) = (g.xyz[face[i] as usize], g.xyz[face[i + 1] as usize]);
                        v += p0[0] * (a[1] * b[2] - a[2] * b[1])
                            - p0[1] * (a[0] * b[2] - a[2] * b[0])
                            + p0[2] * (a[0] * b[1] - a[1] * b[0]);
                    }
                }
                if v < 0.0 {
                    want.iter_mut().for_each(|f| f.reverse());
                }
                let got: Vec<Vec<u32>> = faces.map(|f| polys.get(f).to_vec()).collect();
                assert_eq!(got, want, "cell {c}");
                let sizes = |n: usize| want.iter().filter(|f| f.len() == n).count();
                assert_eq!(kind[c], CellKind::of_counts(want.len(), sizes(3), sizes(4)));
            } else {
                // the faces use exactly the cell's points
                let corners =
                    g.offsets[p.offset_at + k] as usize..g.offsets[p.offset_at + k + 1] as usize;
                let want: BTreeSet<u32> = corners
                    .map(|i| map.get(p.point_at + g.conn[p.conn_at + i] as usize))
                    .collect();
                assert_eq!(used, want, "cell {c}");
                let named = match g.types[r] {
                    TETRA => CellKind::Tetrahedron,
                    HEXAHEDRON => CellKind::Hexahedron,
                    WEDGE => CellKind::Prism,
                    _ => CellKind::Pyramid,
                };
                assert_eq!(kind[c], named);
            }
        }
        assert_eq!(*zone_max, zone.iter().copied().max());
    }

    /// Invariant: the cells built in units of any size on any number of
    /// threads, each unit given only its window of the datasets, are
    /// exactly those of one walk over the whole datasets in file order,
    /// for partitions of any size (those without cells first, between
    /// others, at a unit's edge and last), with ghosts, every cell type,
    /// with and without polyhedron datasets, and points by an offset or
    /// joined out of order; and the one walk gives what the reader
    /// promises: no ghost, the row of each cell kept, the polygons as
    /// boundary faces, each face's smallest point, standard cells with
    /// their own points, polyhedra turned outward. Breaks when a unit
    /// starts reading at its partition's first row instead of its own,
    /// takes another partition's bases, misses a row of its window (the
    /// offset after its last cell, a face of its last polyhedron), orients
    /// a polyhedron with faces of the cell before, or writes its pieces
    /// where the counts of the units before do not put them. The reader
    /// tests read small files, which fit in one unit.
    #[test]
    fn threads_give_the_cells_of_one_walk() {
        let layouts: [&[usize]; 7] = [
            &[],
            &[0],
            &[1],
            &[0, 7, 0, 0, 13, 1, 0],
            &[40, 0, 3, 55, 0],
            &[0, 30, 0, 30, 0, 0, 60, 0],
            &[120],
        ];
        for sizes in layouts {
            for kinds in [&MIXED[..], &STANDARD[..]] {
                let g = grid(sizes, kinds);
                for map in &maps(&g) {
                    let one = read_on(1, 0, &g, map).unwrap();
                    check(&g, map, &one);
                    for threads in 1..=5 {
                        for unit in UNITS {
                            assert_eq!(
                                read_on(threads, unit, &g, map).unwrap(),
                                one,
                                "{sizes:?}, {threads} threads, units of {unit}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A change that makes row `r` of partition `p` bad.
    type Spoil = dyn Fn(&mut Data, &Part, usize);
    /// A change that makes partition `p` bad.
    type SpoilPart = dyn Fn(&mut Data, usize);

    /// Invariant: a bad cell or partition gives the error of one walk
    /// over the whole datasets in file order, in units of any size on any
    /// number of threads: the first bad one's, even when a later unit
    /// holds another; and every partition is checked, those without cells
    /// too, at a unit's edge or after the last cell; an offset that points
    /// outside its dataset is reported, not cut off by a unit's window.
    /// Breaks when the units' errors are taken in the order the threads
    /// finish, a unit reports a partition that a unit before it checks
    /// first, a partition without cells at a unit's edge or the end is
    /// checked by no unit, or a window clamps a bad offset into range. The
    /// reader's error tests read small files, which fit in one unit.
    #[test]
    fn threads_report_the_first_bad_cell_or_partition() {
        let sizes = [0, 30, 0, 30, 0, 0, 60, 0];
        let base = grid(&sizes, &MIXED);
        let parts = base.parts();
        let map = PointMap::Offset(0);
        // a later bad cell
        let later = 119;
        let same = |g: &Data| {
            let one = read_on(1, 0, g, &map).unwrap_err();
            for threads in 1..=5 {
                for unit in UNITS {
                    assert_eq!(
                        read_on(threads, unit, g, &map).unwrap_err(),
                        one,
                        "{threads} threads, units of {unit}"
                    );
                }
            }
            one
        };
        let at = |r: usize| {
            *parts
                .iter()
                .rfind(|p| p.cell_at <= r && r < p.cell_at + p.cells)
                .unwrap()
        };
        let cells: [(&Spoil, &dyn Fn(i64) -> bool); 7] = [
            (&|g, _, r| g.types[r] = 99, &|_| true),
            (&|g, _, r| g.zone[r] = -1.0, &|_| true),
            (
                &|g, p, r| g.offsets[p.offset_at + r - p.cell_at + 1] += 1_000_000,
                &|_| true,
            ),
            (
                &|g, p, r| {
                    let i = g.offsets[p.offset_at + r - p.cell_at] as usize;
                    g.conn[p.conn_at + i] = 1_000;
                },
                &|t| t != POLYHEDRON,
            ),
            (
                &|g, p, r| g.poly_offsets[p.poly_offset_at + r - p.cell_at + 1] = -5,
                &|t| t == POLYHEDRON,
            ),
            (
                &|g, p, r| {
                    let i = g.poly_offsets[p.poly_offset_at + r - p.cell_at] as usize;
                    g.to_faces[p.to_faces_at + i] = 1_000_000;
                },
                &|t| t == POLYHEDRON,
            ),
            (
                &|g, p, r| {
                    let i = g.poly_offsets[p.poly_offset_at + r - p.cell_at] as usize;
                    let f = g.to_faces[p.to_faces_at + i] as usize;
                    g.face_offsets[p.face_offset_at + f + 1] = 1_000_000;
                },
                &|t| t == POLYHEDRON,
            ),
        ];
        let mut spoiled = 0;
        for (spoil, fits) in cells {
            for r in [0, 1, 29, 30, 59, 60, 61, 100, 110, 119] {
                if base.ghost[r] != 0 || !fits(base.types[r]) {
                    continue;
                }
                let mut g = base.clone();
                spoil(&mut g, &at(r), r);
                if r < later {
                    g.types[later] = 98;
                }
                let one = same(&g);
                assert!(!one.1.contains("type 98"), "{one:?}");
                spoiled += 1;
            }
        }
        assert!(spoiled >= 30);
        let partitions: [&SpoilPart; 4] = [
            &|g, p| g.conn_ids[p] = -1,
            &|g, p| g.faces[p] = -1,
            &|g, p| g.to_face_ids[p] = -1,
            &|g, p| g.face_ids[p] = i64::MIN,
        ];
        for spoil in partitions {
            for (p, part) in parts.iter().enumerate() {
                let mut g = base.clone();
                spoil(&mut g, p);
                if part.cell_at + part.cells <= later {
                    g.types[later] = 98;
                }
                let one = same(&g);
                assert!(!one.1.contains("type 98"), "partition {p}: {one:?}");
            }
        }
        // datasets shorter than the counts: the last partition (no cells),
        // the last with cells, and an early bad cell before either
        let short: [&dyn Fn(&mut Data); 3] = [
            &|g| {
                g.offsets.pop();
            },
            &|g| g.types.truncate(100),
            &|g| {
                g.face_conn.pop();
            },
        ];
        for spoil in short {
            let mut g = base.clone();
            spoil(&mut g);
            let one = same(&g);
            assert!(one.1.contains("shorter"), "{one:?}");
            g.types[10] = 97;
            assert!(same(&g).1.contains("type 97"));
        }
    }
}
