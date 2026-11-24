//! The cells of a `.vtu` file turned into faces on all threads.
//!
//! Each VTK cell becomes a volume cell with its faces (polyhedra from the
//! `faces` stream, standard cells from their face tables) or a boundary
//! polygon. The cells are cut into one range per thread. A first pass
//! checks each range and counts what it adds; the counts give every range
//! its place in the arrays, and a second pass writes the ranges there.
//! Both passes walk the cells with the same [`walk`], so the arrays and
//! the first error are those of one walk over all cells in order.

use std::ops::Range;

use super::{
    HEX_FACES, HEXAHEDRON, POLYGON, POLYHEDRON, PYRAMID, PYRAMID_FACES, QUAD, TETRA, TETRA_FACES,
    TRIANGLE, WEDGE, WEDGE_FACES, err, points_in_range,
};
use crate::{CellKind, FaceList, MeshError, Polygons, par};

/// Cells per range at the least; fewer cells stay on one thread.
const MIN_CELLS: usize = 1 << 14;

/// The cell arrays of a file.
#[derive(Clone, Copy)]
pub(super) struct Input<'a> {
    pub connectivity: &'a [u32],
    pub offsets: &'a [i64],
    pub types: &'a [i64],
    /// The polyhedra's `faces` stream: per cell its face count, then each
    /// face as its point count and points.
    pub stream: &'a [u32],
    pub face_offsets: &'a [i64],
    pub region: Option<&'a [i64]>,
    pub points: usize,
}

/// The faces of the volume cells, cell after cell.
pub(super) enum Faces {
    /// Their points copied out, for files with standard cells.
    Packed(Polygons),
    /// Left in the `faces` stream (files of polyhedra and polygons only,
    /// as KoflCraft writes): where each face's point count is.
    InStream(Vec<u32>),
}

/// Cell faces left in the `faces` stream they were read from: face `f`
/// is the `stream[at[f]]` points after `at[f]`. Nothing is copied, and
/// the stream is freed with them after the faces are linked.
pub(crate) struct InStream {
    pub stream: Vec<u32>,
    pub at: Vec<u32>,
}

impl FaceList for InStream {
    fn count(&self) -> usize {
        self.at.len()
    }

    fn face(&self, i: usize) -> &[u32] {
        let a = self.at[i] as usize;
        let n = self.stream[a] as usize;
        &self.stream[a + 1..a + 1 + n]
    }
}

/// What the cells became.
pub(super) struct Cells {
    pub faces: Faces,
    /// Each face's smallest point (`u32::MAX` for a face of none), found
    /// while its points are at hand, for the linking that sorts by it.
    pub least: Vec<u32>,
    /// 0, then where each volume cell's faces end among the faces.
    pub cell_faces: Vec<u32>,
    pub cell_zone: Vec<u32>,
    pub cell_kind: Vec<CellKind>,
    pub boundary: Polygons,
    pub boundary_patch: Vec<u32>,
    pub zone_max: Option<u32>,
    pub patch_max: Option<u32>,
}

/// Where a range of cells starts reading: its first point in
/// `connectivity` and its first value in the `faces` stream.
#[derive(Clone, Copy, Default)]
struct At {
    connectivity: usize,
    stream: usize,
}

/// What a walk hands on, cell by cell.
trait Sink {
    /// A face of the volume cell being read: its points, whose count is
    /// at `at` in the stream.
    fn face(&mut self, at: usize, nodes: &[u32]);
    /// A face of a standard cell: its corners in the cell's `nodes`.
    fn table_face(&mut self, nodes: &[u32], corners: &[usize]);
    /// The volume cell whose faces came before ends.
    fn cell(&mut self, zone: u32, kind: CellKind);
    fn polygon(&mut self, nodes: &[u32], patch: u32);
}

/// Walk `cells`, starting to read at `at`, checking each cell as it
/// goes; the error is that of the first bad cell.
fn walk(input: Input, cells: Range<usize>, at: At, sink: &mut impl Sink) -> Result<(), MeshError> {
    let Input {
        connectivity: conn,
        offsets,
        types,
        stream,
        face_offsets,
        region,
        points,
    } = input;
    let mut start = at.connectivity;
    let mut face_pos = at.stream;
    for c in cells {
        let end = usize::try_from(offsets[c])
            .ok()
            .filter(|&e| e >= start && e <= conn.len())
            .ok_or_else(|| err("MESH006", format!("cell {c}: offset {}", offsets[c])))?;
        let nodes = &conn[start..end];
        start = end;
        let reg = match region {
            Some(r) => {
                if r[c] < 0 {
                    return Err(err("MESH006", format!("cell {c}: region {}", r[c])));
                }
                u32::try_from(r[c]).map_err(|_| err("MESH006", format!("cell {c}: region")))?
            }
            None => 0,
        };
        let t = types[c];
        let table = match t {
            TETRA => Some(TETRA_FACES),
            HEXAHEDRON => Some(HEX_FACES),
            WEDGE => Some(WEDGE_FACES),
            PYRAMID => Some(PYRAMID_FACES),
            _ => None,
        };
        if t == POLYHEDRON {
            let fo = face_offsets
                .get(c)
                .copied()
                .ok_or_else(|| err("MESH009", format!("cell {c}: no faceoffsets entry")))?;
            let fend = usize::try_from(fo)
                .ok()
                .filter(|&e| e > face_pos && e <= stream.len())
                .ok_or_else(|| err("MESH009", format!("cell {c}: faceoffset {fo}")))?;
            // The cell's part of the stream: its face count, then each
            // face as its point count and points.
            let nf = stream[face_pos] as usize;
            let mut k = face_pos + 1;
            let (mut tris, mut quads) = (0, 0);
            for _ in 0..nf {
                let n = (k < fend)
                    .then(|| stream[k] as usize)
                    .filter(|&n| n >= 3)
                    .ok_or_else(|| err("MESH009", format!("cell {c}: bad face size")))?;
                let face = k + 1..k + 1 + n;
                if face.end > fend {
                    return Err(err("MESH009", format!("cell {c}: face runs past its cell")));
                }
                points_in_range(&stream[face.clone()], points)?;
                sink.face(k, &stream[face]);
                tris += usize::from(n == 3);
                quads += usize::from(n == 4);
                k += 1 + n;
            }
            if k != fend {
                return Err(err(
                    "MESH009",
                    format!("cell {c}: {} values left after its faces", fend - k),
                ));
            }
            face_pos = fend;
            sink.cell(reg, CellKind::of_counts(nf, tris, quads));
        } else if let Some(table) = table {
            let need = table
                .iter()
                .flat_map(|f| f.iter())
                .max()
                .map_or(0, |m| m + 1);
            if nodes.len() != need {
                return Err(err(
                    "MESH006",
                    format!("cell {c}: type {t} with {} points", nodes.len()),
                ));
            }
            points_in_range(nodes, points)?;
            for f in table {
                sink.table_face(nodes, f);
            }
            sink.cell(reg, CellKind::classify(table.iter().map(|f| f.len())));
        } else if matches!(t, TRIANGLE | POLYGON | QUAD) {
            if nodes.len() < 3 {
                return Err(err(
                    "MESH006",
                    format!("cell {c}: polygon with {} points", nodes.len()),
                ));
            }
            points_in_range(nodes, points)?;
            sink.polygon(nodes, reg);
        } else {
            return Err(err("MESH008", format!("cell {c} has VTK type {t}")));
        }
    }
    Ok(())
}

/// What a range of cells adds.
#[derive(Clone, Copy, Default)]
struct Count {
    cells: usize,
    faces: usize,
    nodes: usize,
    polygons: usize,
    polygon_nodes: usize,
    zone_max: Option<u32>,
    patch_max: Option<u32>,
}

fn max(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        _ => a.or(b),
    }
}

impl Count {
    /// `self` followed by `next`.
    fn plus(self, next: Count) -> Count {
        Count {
            cells: self.cells + next.cells,
            faces: self.faces + next.faces,
            nodes: self.nodes + next.nodes,
            polygons: self.polygons + next.polygons,
            polygon_nodes: self.polygon_nodes + next.polygon_nodes,
            zone_max: max(self.zone_max, next.zone_max),
            patch_max: max(self.patch_max, next.patch_max),
        }
    }
}

impl Sink for Count {
    fn face(&mut self, _: usize, nodes: &[u32]) {
        self.faces += 1;
        self.nodes += nodes.len();
    }

    fn table_face(&mut self, _: &[u32], corners: &[usize]) {
        self.faces += 1;
        self.nodes += corners.len();
    }

    fn cell(&mut self, zone: u32, _: CellKind) {
        self.cells += 1;
        self.zone_max = max(self.zone_max, Some(zone));
    }

    fn polygon(&mut self, nodes: &[u32], patch: u32) {
        self.polygons += 1;
        self.polygon_nodes += nodes.len();
        self.patch_max = max(self.patch_max, Some(patch));
    }
}

/// A range's pieces of the arrays, written as its walk goes.
struct Write<'a> {
    /// What the ranges before this one wrote.
    base: Count,
    /// What this one has written so far.
    done: Count,
    /// Each face's place in the stream (`Faces::InStream`), or its end
    /// among the points copied out.
    in_stream: bool,
    face_end: &'a mut [u32],
    least: &'a mut [u32],
    nodes: &'a mut [u32],
    cell_end: &'a mut [u32],
    zone: &'a mut [u32],
    kind: &'a mut [CellKind],
    polygon_end: &'a mut [u32],
    polygon_nodes: &'a mut [u32],
    patch: &'a mut [u32],
}

impl Write<'_> {
    fn end_face(&mut self) {
        self.face_end[self.done.faces] = (self.base.nodes + self.done.nodes) as u32;
        self.done.faces += 1;
    }
}

impl Sink for Write<'_> {
    fn face(&mut self, at: usize, nodes: &[u32]) {
        self.least[self.done.faces] = nodes.iter().copied().min().unwrap_or(u32::MAX);
        if self.in_stream {
            self.face_end[self.done.faces] = at as u32;
            self.done.faces += 1;
            return;
        }
        let at = self.done.nodes;
        self.nodes[at..at + nodes.len()].copy_from_slice(nodes);
        self.done.nodes += nodes.len();
        self.end_face();
    }

    fn table_face(&mut self, nodes: &[u32], corners: &[usize]) {
        let least = corners.iter().map(|&i| nodes[i]).min();
        self.least[self.done.faces] = least.unwrap_or(u32::MAX);
        let at = self.done.nodes;
        for (out, &i) in self.nodes[at..at + corners.len()].iter_mut().zip(corners) {
            *out = nodes[i];
        }
        self.done.nodes += corners.len();
        self.end_face();
    }

    fn cell(&mut self, zone: u32, kind: CellKind) {
        let c = self.done.cells;
        self.cell_end[c] = (self.base.faces + self.done.faces) as u32;
        self.zone[c] = zone;
        self.kind[c] = kind;
        self.done.cells += 1;
    }

    fn polygon(&mut self, nodes: &[u32], patch: u32) {
        let (p, at) = (self.done.polygons, self.done.polygon_nodes);
        self.polygon_nodes[at..at + nodes.len()].copy_from_slice(nodes);
        self.done.polygon_nodes += nodes.len();
        self.polygon_end[p] = (self.base.polygon_nodes + self.done.polygon_nodes) as u32;
        self.patch[p] = patch;
        self.done.polygons += 1;
    }
}

/// `data` cut where each range's piece starts (`at`, increasing from 0).
fn pieces<T>(data: &mut [T], at: impl Iterator<Item = usize>) -> Vec<&mut [T]> {
    let len = data.len();
    let cuts: Vec<usize> = at.chain([len]).collect();
    par::split_mut(data, &cuts)
}

/// Turn every cell of `input` into faces or a boundary polygon.
pub(super) fn read(input: Input) -> Result<Cells, MeshError> {
    let n = input.types.len();
    let ranges = par::ranges(n, MIN_CELLS);
    // Where each range starts reading. The connectivity follows from the
    // offset of the cell before; the stream from the face offset of the
    // last polyhedron before, found range by range. A bad offset there
    // fails that earlier range first.
    let last_face_end: Vec<Option<usize>> = par::each(ranges.clone(), |r| {
        r.rev()
            .find(|&c| input.types[c] == POLYHEDRON)
            .and_then(|c| input.face_offsets.get(c))
            .map(|&fo| usize::try_from(fo).unwrap_or(0))
    });
    let mut at = Vec::with_capacity(ranges.len());
    let mut stream = 0;
    for (r, last) in ranges.iter().zip(&last_face_end) {
        let connectivity = match r.start {
            0 => 0,
            s => usize::try_from(input.offsets[s - 1]).unwrap_or(0),
        };
        at.push(At {
            connectivity,
            stream,
        });
        stream = last.unwrap_or(stream);
    }
    let jobs: Vec<(Range<usize>, At)> = ranges.into_iter().zip(at).collect();

    let counts = par::each(jobs.clone(), |(r, at)| {
        let mut count = Count::default();
        walk(input, r, at, &mut count).map(|()| count)
    });
    let mut base = Vec::with_capacity(counts.len());
    let mut total = Count::default();
    for count in counts {
        base.push(total);
        total = total.plus(count?);
    }

    // Faces stay in the stream unless standard cells need theirs copied
    // out.
    let in_stream = input
        .types
        .iter()
        .all(|&t| matches!(t, POLYHEDRON | TRIANGLE | POLYGON | QUAD));
    let (mut face_list, mut face_nodes) = if in_stream {
        (vec![0; total.faces], Vec::new())
    } else {
        (vec![0; total.faces + 1], vec![0; total.nodes])
    };
    let mut least = vec![0; total.faces];
    let mut cell_faces = vec![0; total.cells + 1];
    let mut cell_zone = vec![0; total.cells];
    let mut cell_kind = vec![CellKind::Polyhedron; total.cells];
    let mut boundary = Polygons {
        start: vec![0; total.polygons + 1],
        nodes: vec![0; total.polygon_nodes],
    };
    let mut boundary_patch = vec![0; total.polygons];

    let skip = usize::from(!in_stream);
    let face_end = pieces(&mut face_list[skip..], base.iter().map(|b| b.faces));
    let face_least = pieces(&mut least, base.iter().map(|b| b.faces));
    let node_at = base.iter().map(|b| if in_stream { 0 } else { b.nodes });
    let nodes = pieces(&mut face_nodes, node_at);
    let cell_end = pieces(&mut cell_faces[1..], base.iter().map(|b| b.cells));
    let zone = pieces(&mut cell_zone, base.iter().map(|b| b.cells));
    let kind = pieces(&mut cell_kind, base.iter().map(|b| b.cells));
    let polygon_end = pieces(&mut boundary.start[1..], base.iter().map(|b| b.polygons));
    let polygon_nodes = pieces(&mut boundary.nodes, base.iter().map(|b| b.polygon_nodes));
    let patch = pieces(&mut boundary_patch, base.iter().map(|b| b.polygons));
    let mut writes = Vec::with_capacity(jobs.len());
    let parts = face_end
        .into_iter()
        .zip(face_least)
        .zip(nodes)
        .zip(cell_end)
        .zip(zone)
        .zip(kind)
        .zip(polygon_end)
        .zip(polygon_nodes)
        .zip(patch);
    for (
        (
            (((((((face_end, least), nodes), cell_end), zone), kind), polygon_end), polygon_nodes),
            patch,
        ),
        (job, &base),
    ) in parts.zip(jobs.into_iter().zip(&base))
    {
        let write = Write {
            base,
            done: Count::default(),
            in_stream,
            face_end,
            least,
            nodes,
            cell_end,
            zone,
            kind,
            polygon_end,
            polygon_nodes,
            patch,
        };
        writes.push((job, write));
    }
    for written in par::each(writes, |((r, at), mut write)| {
        walk(input, r, at, &mut write)
    }) {
        written?;
    }
    let faces = if in_stream {
        Faces::InStream(face_list)
    } else {
        Faces::Packed(Polygons {
            start: face_list,
            nodes: face_nodes,
        })
    };
    Ok(Cells {
        faces,
        least,
        cell_faces,
        cell_zone,
        cell_kind,
        boundary,
        boundary_patch,
        zone_max: total.zone_max,
        patch_max: total.patch_max,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const POINTS: usize = 50;

    /// The cell arrays of a file.
    #[derive(Clone, Default)]
    struct Arrays {
        conn: Vec<u32>,
        offsets: Vec<i64>,
        types: Vec<i64>,
        stream: Vec<u32>,
        face_offsets: Vec<i64>,
        region: Vec<i64>,
    }

    impl Arrays {
        fn input(&self) -> Input<'_> {
            Input {
                connectivity: &self.conn,
                offsets: &self.offsets,
                types: &self.types,
                stream: &self.stream,
                face_offsets: &self.face_offsets,
                region: Some(&self.region),
                points: POINTS,
            }
        }
    }

    /// The cell types [`arrays`] mixes.
    const MIXED: [i64; 10] = [
        POLYHEDRON, TETRA, POLYHEDRON, HEXAHEDRON, QUAD, WEDGE, POLYHEDRON, PYRAMID, TRIANGLE,
        POLYGON,
    ];
    /// Only polyhedra and polygons, as KoflCraft writes.
    const KOFLCRAFT: [i64; 6] = [POLYHEDRON, POLYHEDRON, QUAD, POLYHEDRON, TRIANGLE, POLYGON];

    /// `cells` cells of every type the reader knows, in a mixed order with
    /// mixed sizes: polyhedra of 4 to 8 faces of 3 to 6 points, standard
    /// cells, triangles, quads and polygons, in zones and patches 0 to 2.
    fn arrays(cells: usize) -> Arrays {
        arrays_of(cells, &MIXED)
    }

    /// As [`arrays`], of the types `kinds`.
    fn arrays_of(cells: usize, kinds: &[i64]) -> Arrays {
        let mut a = Arrays::default();
        let mut seed = 7u64;
        let mut next = |m: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) % m) as u32
        };
        for c in 0..cells {
            let t = kinds[(c * 7 + c / 10) % kinds.len()];
            let points = match t {
                TETRA => 4,
                HEXAHEDRON => 8,
                WEDGE => 6,
                PYRAMID => 5,
                TRIANGLE => 3,
                QUAD => 4,
                POLYGON => 3 + next(4) as usize,
                _ => 4 + next(6) as usize,
            };
            for _ in 0..points {
                a.conn.push(next(POINTS as u64));
            }
            a.offsets.push(a.conn.len() as i64);
            a.types.push(t);
            a.region.push(i64::from(next(3)));
            if t == POLYHEDRON {
                let nf = 4 + next(5);
                a.stream.push(nf);
                for _ in 0..nf {
                    let n = 3 + next(4);
                    a.stream.push(n);
                    for _ in 0..n {
                        a.stream.push(next(POINTS as u64));
                    }
                }
                a.face_offsets.push(a.stream.len() as i64);
            } else {
                a.face_offsets.push(-1);
            }
        }
        a
    }

    /// The faces as polygons, wherever the reader left them, and whether
    /// they were left in the stream.
    fn faces(faces: Faces, stream: &[u32]) -> (bool, Polygons) {
        match faces {
            Faces::Packed(p) => (false, p),
            Faces::InStream(at) => {
                let s = InStream {
                    stream: stream.to_vec(),
                    at,
                };
                let mut p = Polygons::new();
                for i in 0..s.count() {
                    p.push(s.face(i).iter().copied());
                }
                (true, p)
            }
        }
    }

    type Read = (
        (bool, Polygons),
        Vec<u32>,
        Vec<u32>,
        Vec<CellKind>,
        Polygons,
        Vec<u32>,
        Option<u32>,
        Option<u32>,
        Vec<u32>,
    );

    fn read_on(threads: usize, a: &Arrays) -> Result<Read, (&'static str, String)> {
        par::with_threads(threads, || read(a.input()))
            .map(|c| {
                (
                    faces(c.faces, &a.stream),
                    c.cell_faces,
                    c.cell_zone,
                    c.cell_kind,
                    c.boundary,
                    c.boundary_patch,
                    c.zone_max,
                    c.patch_max,
                    c.least,
                )
            })
            .map_err(|e| (e.code, e.message))
    }

    /// Invariant: the cells read on several threads give exactly the
    /// arrays of one walk over all of them in order, for every cell type
    /// mixed, with each range starting at the right place of the
    /// connectivity and of the polyhedra's face stream, and the faces,
    /// cells and polygons of each range written after those of the ranges
    /// before; and each face's smallest point is the smallest of the
    /// points it reads back with. Breaks when a range starts the stream at
    /// its own first cell instead of after the last polyhedron before it,
    /// writes its face ends without the points before it, or a standard
    /// cell's face takes its smallest point from the cell's other corners
    /// (linking would sort it into the wrong group). The reader tests on files
    /// read with the machine's threads only, and their fixtures hold only
    /// polyhedra and polygons.
    #[test]
    fn threads_give_the_cells_of_one_walk() {
        for (cells, kinds) in [0, 1, 9, 10, 37, 400]
            .into_iter()
            .flat_map(|n| [(n, &MIXED[..]), (n, &KOFLCRAFT[..])])
        {
            let a = arrays_of(cells, kinds);
            let one = read_on(1, &a).unwrap();
            assert_eq!(one.1.len(), one.2.len() + 1);
            assert_eq!(*one.1.last().unwrap() as usize, one.0.1.len());
            let least: Vec<u32> = (0..one.0.1.len())
                .map(|f| one.0.1.get(f).iter().copied().min().unwrap_or(u32::MAX))
                .collect();
            assert_eq!(one.8, least);
            for threads in 2..=7 {
                assert_eq!(
                    read_on(threads, &a).unwrap(),
                    one,
                    "{cells} cells, {threads} threads"
                );
            }
        }
        // The kinds come from the faces: a tetrahedron, a hexahedron and a
        // polyhedron of the stream.
        let a = arrays(10);
        let kinds = read_on(1, &a).unwrap().3;
        assert!(kinds.contains(&CellKind::Tetrahedron));
        assert!(kinds.contains(&CellKind::Hexahedron));
        assert!(kinds.contains(&CellKind::Polyhedron));
    }

    /// Invariant: a file of polyhedra and polygons only leaves its faces
    /// in the stream, each face the points the file lists for it, cell
    /// after cell; one with standard cells copies them out. Breaks when a
    /// face's place points at its first point instead of its count (every
    /// face would lose a point and gain the next count), or when standard
    /// cells are left to a stream that does not hold their faces. The
    /// topology tests link meshes of either kind but see only the result.
    #[test]
    fn polyhedra_leave_their_faces_in_the_stream() {
        let a = arrays_of(200, &KOFLCRAFT);
        let ((in_stream, got), ..) = read_on(3, &a).unwrap();
        assert!(in_stream);
        let mut want = Polygons::new();
        let mut k = 0;
        for &t in &a.types {
            if t == POLYHEDRON {
                let nf = a.stream[k];
                k += 1;
                for _ in 0..nf {
                    let n = a.stream[k] as usize;
                    want.push(a.stream[k + 1..k + 1 + n].iter().copied());
                    k += 1 + n;
                }
            }
        }
        assert_eq!(k, a.stream.len());
        assert_eq!(got, want);
        assert!(!read_on(3, &arrays(200)).unwrap().0.0);
    }

    /// A change that makes cell `c` of the arrays bad.
    type Spoil = dyn Fn(&mut Arrays, usize);

    /// Invariant: a bad cell gives the error of one walk in order on any
    /// number of threads: the first bad cell's, even when a later range
    /// holds another bad cell. Breaks when the ranges' errors are taken
    /// in the order the threads finish, or a range after a bad one
    /// reports its own error first. The reader's error tests read small
    /// files, which stay on one thread.
    #[test]
    fn threads_report_the_first_bad_cell() {
        let base = arrays(200);
        let poly = base.types.iter().position(|&t| t == POLYHEDRON).unwrap();
        let spoil: [&Spoil; 4] = [
            &|a, c| a.types[c] = 99,
            &|a, c| a.region[c] = -1,
            &|a, c| a.offsets[c] += 1_000_000,
            &|a, c| a.face_offsets[c] = -5,
        ];
        for (k, spoil) in spoil.iter().enumerate() {
            for first in [0, 1, poly, 63, 150, 199] {
                if k == 3 && base.types[first] != POLYHEDRON {
                    continue;
                }
                let mut a = base.clone();
                spoil(&mut a, first);
                if first < 190 {
                    a.types[195] = 99;
                }
                let one = read_on(1, &a).unwrap_err();
                assert!(one.1.contains(&format!("cell {first}")), "{one:?}");
                for threads in 2..=7 {
                    assert_eq!(read_on(threads, &a).unwrap_err(), one, "{threads} threads");
                }
            }
        }
    }
}
