//! Blocked VTKHDF files (`docs/blocked-vtkhdf.md`, layout 1): the mesh
//! cut into blocks of cells along the 3-D Hilbert curve, each block a
//! partition of the `internal` UnstructuredGrid with its own points and
//! one layer of ghost cells, the boundary patches as PolyData blocks, and
//! an index of the blocks' bounds and field ranges. MurmelCraft reads a
//! region of such a file by its blocks; any VTKHDF reader (ParaView 5.13
//! and later) opens it whole.
//!
//! [`write_blocked`] writes one from a mesh in memory and its fields
//! (`murmel blocks make` converts a `.vtu` or a `polyMesh` once). The
//! file is written front to back in one pass ([`crate::h5write`]): each
//! dataset streams block after block, so beyond the mesh only the block
//! plan (the ghost cells and points of each block) is held.

use std::io;
use std::path::Path;

use serde::Serialize;

use crate::field::{Field, Location, Results, range_of};
use crate::h5write::{AttrValue, Element, H5Writer, Link};
use crate::hdf5::H5;
use crate::vtu::{
    HEX_FACES, HEXAHEDRON, POLYHEDRON, PYRAMID, PYRAMID_FACES, TETRA, TETRA_FACES, WEDGE,
    WEDGE_FACES,
};
use crate::{Bounds, CellKind, Mesh, MeshError, NO_CELL, hilbert, par};

/// The layout version written as `murmelcraft_blocks`.
pub const LAYOUT: i64 = 1;
/// Owned cells per block unless the writer picks another size.
pub const DEFAULT_BLOCK_CELLS: usize = 1 << 16;
/// The block sizes a writer may pick (§ 2).
pub const MIN_BLOCK_CELLS: usize = 1 << 14;
pub const MAX_BLOCK_CELLS: usize = 1 << 20;

/// Cell and point arrays the layout uses itself; a field of one of these
/// names is not written.
const RESERVED: &[&str] = &[
    "zone",
    "GlobalCellIds",
    "GlobalPointIds",
    "vtkGhostType",
    "OriginalCellIds",
    "OwnerCell",
];

/// What [`write_blocked`] wrote.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Written {
    pub blocks: usize,
    pub block_cells: usize,
    pub cells: usize,
    /// Ghost cells over all blocks (each a copy of a cell of another block).
    pub ghost_cells: usize,
    pub points: usize,
    /// Points over all blocks: a point shared by blocks is in each.
    pub block_points: usize,
    /// Cells written as VTK's linear cells (tetra, hexahedron, wedge,
    /// pyramid); the others are polyhedra.
    pub linear_cells: usize,
    pub polyhedra: usize,
    pub patches: usize,
    pub fields: usize,
    /// Fields not written: their name is one the layout uses itself.
    pub skipped_fields: Vec<String>,
    pub bytes: u64,
}

fn failed(e: io::Error) -> MeshError {
    if e.kind() == io::ErrorKind::Interrupted {
        return MeshError::new("MESH019", "writing the blocked file was cancelled");
    }
    MeshError::new(
        "BLOCK001",
        format!("the blocked file cannot be written: {e}"),
    )
}

/// Whether an open HDF5 file is a blocked VTKHDF file (it carries
/// `murmelcraft_blocks` on `/VTKHDF`); the layout version when it is.
pub fn layout_of(h: &H5) -> Option<i64> {
    let vtk = h.path("/VTKHDF").ok()?;
    let v = h.attr(&vtk, "murmelcraft_blocks").ok()??.number()?;
    Some(v as i64)
}

/// The cells as VTK stores them: a type per cell and its points (VTK's
/// node order for a linear cell, the distinct points in face order for a
/// polyhedron).
struct VtkCells {
    ty: Vec<u8>,
    start: Vec<usize>,
    conn: Vec<u32>,
}

impl VtkCells {
    fn points(&self, c: usize) -> &[u32] {
        &self.conn[self.start[c]..self.start[c + 1]]
    }
}

/// The faces of one cell, outward as the cell lists them.
#[derive(Default)]
struct Faces {
    start: Vec<usize>,
    nodes: Vec<u32>,
}

impl Faces {
    fn of(&mut self, mesh: &Mesh, c: usize) {
        self.start.clear();
        self.nodes.clear();
        self.start.push(0);
        for f in mesh.cell(c) {
            self.nodes.extend(f.oriented());
            self.start.push(self.nodes.len());
        }
    }

    fn len(&self) -> usize {
        self.start.len() - 1
    }

    fn get(&self, i: usize) -> &[u32] {
        &self.nodes[self.start[i]..self.start[i + 1]]
    }

    /// The node joined to `n` by an edge of a face and not in `not`, when
    /// there is exactly one.
    fn across(&self, n: u32, not: &[u32]) -> Option<u32> {
        let mut found = None;
        for i in 0..self.len() {
            let f = self.get(i);
            for k in 0..f.len() {
                let (a, b) = (f[k], f[(k + 1) % f.len()]);
                let other = if a == n {
                    b
                } else if b == n {
                    a
                } else {
                    continue;
                };
                if not.contains(&other) {
                    continue;
                }
                match found {
                    None => found = Some(other),
                    Some(o) if o == other => {}
                    Some(_) => return None,
                }
            }
        }
        found
    }

    /// The first face with `n` points.
    fn first_of(&self, n: usize) -> Option<&[u32]> {
        (0..self.len()).map(|i| self.get(i)).find(|f| f.len() == n)
    }
}

/// Cell `c` as one of VTK's linear cells, when its faces are exactly the
/// faces of that cell (same points, same turn): the type and the nodes in
/// VTK's order. A hexahedron whose quads are warped into another shape,
/// a collapsed node or an inconsistent turn give `None`, and the cell is
/// written as a polyhedron with its faces as they are.
///
/// VTK's orders (`vtkTetra`, `vtkPyramid`, `vtkWedge`, `vtkHexahedron`
/// of VTK 9, whose volumes are then positive): the base of each cell
/// turns towards the rest of the cell, so an outward base face (a, b, c)
/// or (a, b, c, d) gives the base (a, c, b) or (a, d, c, b). A top node
/// is the one joined to its base node by an edge.
fn linear(kind: CellKind, f: &Faces) -> Option<(i64, Vec<u32>)> {
    let (ty, nodes, table): (i64, Vec<u32>, &[&[usize]]) = match kind {
        CellKind::Tetrahedron => {
            let t = f.first_of(3)?;
            let base = [t[0], t[2], t[1]];
            let apex = *f.nodes.iter().find(|n| !base.contains(n))?;
            (TETRA, vec![t[0], t[2], t[1], apex], TETRA_FACES)
        }
        CellKind::Pyramid => {
            let q = f.first_of(4)?;
            let base = [q[0], q[3], q[2], q[1]];
            let apex = *f.nodes.iter().find(|n| !base.contains(n))?;
            (PYRAMID, vec![q[0], q[3], q[2], q[1], apex], PYRAMID_FACES)
        }
        CellKind::Prism => {
            let t = f.first_of(3)?;
            let base = [t[0], t[2], t[1]];
            let mut nodes = base.to_vec();
            for &b in &base {
                nodes.push(f.across(b, &base)?);
            }
            (WEDGE, nodes, WEDGE_FACES)
        }
        CellKind::Hexahedron => {
            let q = f.first_of(4)?;
            let base = [q[0], q[3], q[2], q[1]];
            let mut nodes = base.to_vec();
            for &b in &base {
                nodes.push(f.across(b, &base)?);
            }
            (HEXAHEDRON, nodes, HEX_FACES)
        }
        CellKind::Polyhedron => return None,
    };
    // the nodes are distinct and are all the cell's points
    let mut sorted = nodes.clone();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.len() != nodes.len() || f.nodes.iter().any(|n| sorted.binary_search(n).is_err()) {
        return None;
    }
    // each face of the table is one face of the cell, same turn
    if table.len() != f.len() {
        return None;
    }
    let mut used = vec![false; f.len()];
    for t in table {
        let hit = (0..f.len()).find(|&i| {
            let g = f.get(i);
            !used[i]
                && g.len() == t.len()
                && (0..g.len()).any(|r| (0..g.len()).all(|k| g[(r + k) % g.len()] == nodes[t[k]]))
        })?;
        used[hit] = true;
    }
    Some((ty, nodes))
}

fn vtk_cells(mesh: &Mesh) -> VtkCells {
    let n = mesh.cell_count();
    let parts = par::map(n, 1 << 14, |r| {
        let mut ty = Vec::with_capacity(r.len());
        let mut len = Vec::with_capacity(r.len());
        let mut conn = Vec::new();
        let mut f = Faces::default();
        for c in r {
            f.of(mesh, c);
            let at = conn.len();
            match linear(mesh.cell_kind[c], &f) {
                Some((t, nodes)) => {
                    ty.push(t as u8);
                    conn.extend(nodes);
                }
                None => {
                    ty.push(POLYHEDRON as u8);
                    for &p in &f.nodes {
                        if !conn[at..].contains(&p) {
                            conn.push(p);
                        }
                    }
                }
            }
            len.push(conn.len() - at);
        }
        (ty, len, conn)
    });
    let mut out = VtkCells {
        ty: Vec::with_capacity(n),
        start: Vec::with_capacity(n + 1),
        conn: Vec::new(),
    };
    out.start.push(0);
    for (ty, len, conn) in parts {
        out.ty.extend(ty);
        for l in len {
            out.start.push(out.start.last().copied().unwrap_or(0) + l);
        }
        out.conn.extend(conn);
    }
    out
}

/// The blocks: which cells and points each holds.
struct Plan {
    bs: usize,
    /// The mesh cell of each file cell id, and the file cell id of each
    /// mesh cell.
    order: Vec<u32>,
    file_id: Vec<u32>,
    /// Per block: its ghost cells (mesh cells, by file cell id), its
    /// points (mesh points, ascending: the local id is the position) and
    /// the bounds of its owned cells' points.
    ghosts: Vec<Vec<u32>>,
    points: Vec<Vec<u32>>,
    bounds: Vec<Bounds>,
    /// The lowest block whose owned cells use each point.
    point_owner: Vec<u32>,
}

impl Plan {
    fn blocks(&self) -> usize {
        self.ghosts.len()
    }

    fn owned(&self, b: usize) -> &[u32] {
        &self.order[b * self.bs..((b + 1) * self.bs).min(self.order.len())]
    }

    /// The cells of block `b`: the owned cells, then the ghosts.
    fn cells(&self, b: usize) -> impl Iterator<Item = u32> + '_ {
        self.owned(b).iter().chain(self.ghosts[b].iter()).copied()
    }

    fn local(&self, b: usize, p: u32) -> i64 {
        self.points[b].binary_search(&p).map_or(-1, |i| i as i64)
    }
}

fn plan(mesh: &Mesh, cells: &VtkCells, bs: usize) -> Plan {
    let n = mesh.cell_count();
    // Hilbert order of the cell centroids (the mean of the cell's points)
    // in the bounding box of all points
    let mut all = Bounds::empty();
    for &p in &mesh.points {
        all.add(p);
    }
    let grid = hilbert::Grid::new(all.min, all.max);
    let mut keyed: Vec<(u64, u32)> = par::collect(n, 1 << 14, |r| {
        r.map(|c| {
            let pts = cells.points(c);
            let mut m = [0.0; 3];
            for &p in pts {
                let q = mesh.points[p as usize];
                for k in 0..3 {
                    m[k] += q[k];
                }
            }
            let k = pts.len().max(1) as f64;
            (grid.index(m.map(|v| v / k)), c as u32)
        })
        .collect()
    });
    keyed.sort_unstable();
    let order: Vec<u32> = keyed.into_iter().map(|(_, c)| c).collect();
    let mut file_id = vec![0u32; n];
    for (f, &c) in order.iter().enumerate() {
        file_id[c as usize] = f as u32;
    }
    let blocks = n.div_ceil(bs);
    // the cells of each point
    let np = mesh.points.len();
    let mut head = vec![0usize; np + 1];
    for &p in &cells.conn {
        head[p as usize + 1] += 1;
    }
    for p in 0..np {
        head[p + 1] += head[p];
    }
    let mut next = head.clone();
    let mut point_cells = vec![0u32; cells.conn.len()];
    for c in 0..n {
        for &p in cells.points(c) {
            point_cells[next[p as usize]] = c as u32;
            next[p as usize] += 1;
        }
    }
    drop(next);
    let mut point_owner = vec![u32::MAX; np];
    for (f, &c) in order.iter().enumerate() {
        let b = (f / bs) as u32;
        for &p in cells.points(c as usize) {
            let o = &mut point_owner[p as usize];
            if *o == u32::MAX {
                *o = b;
            }
        }
    }
    let per: Vec<(Vec<u32>, Vec<u32>, Bounds)> = par::collect(blocks, 1, |r| {
        r.map(|b| {
            let owned = &order[b * bs..((b + 1) * bs).min(n)];
            let mut ghosts = Vec::new();
            let mut bounds = Bounds::empty();
            for &c in owned {
                for &p in cells.points(c as usize) {
                    bounds.add(mesh.points[p as usize]);
                    for &d in &point_cells[head[p as usize]..head[p as usize + 1]] {
                        let f = file_id[d as usize] as usize;
                        if f / bs != b {
                            ghosts.push(f as u32);
                        }
                    }
                }
            }
            ghosts.sort_unstable();
            ghosts.dedup();
            let ghosts: Vec<u32> = ghosts.into_iter().map(|f| order[f as usize]).collect();
            let mut points: Vec<u32> = owned
                .iter()
                .chain(&ghosts)
                .flat_map(|&c| cells.points(c as usize).iter().copied())
                .collect();
            points.sort_unstable();
            points.dedup();
            (ghosts, points, bounds)
        })
        .collect()
    });
    let mut plan = Plan {
        bs,
        order,
        file_id,
        ghosts: Vec::with_capacity(blocks),
        points: Vec::with_capacity(blocks),
        bounds: Vec::with_capacity(blocks),
        point_owner,
    };
    for (g, p, b) in per {
        plan.ghosts.push(g);
        plan.points.push(p);
        plan.bounds.push(b);
    }
    plan
}

/// A dataset streamed block after block: `rows(b, out)` appends block
/// `b`'s values.
fn per_block<T: Element>(
    w: &mut H5Writer,
    blocks: usize,
    width: u64,
    mut rows: impl FnMut(usize, &mut Vec<T>),
) -> io::Result<u64> {
    let start = w.pos();
    let mut buf = Vec::new();
    let mut count = 0u64;
    for b in 0..blocks {
        // A conversion in the window can be stopped between blocks.
        if crate::profile::check().is_err() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        buf.clear();
        rows(b, &mut buf);
        count += buf.len() as u64;
        w.put(&buf)?;
    }
    let dims: Vec<u64> = if width > 1 {
        vec![count / width, width]
    } else {
        vec![count]
    };
    w.dataset_header(T::TY, &dims, start)
}

/// The values of field `f` for the rows `ids`.
fn rows_of(f: &Field, ids: impl Iterator<Item = u32>, out: &mut Vec<f64>) {
    let k = f.components.max(1);
    for i in ids {
        match f.value(i as usize) {
            Some(v) => out.extend_from_slice(v),
            None => out.extend(std::iter::repeat_n(f64::NAN, k)),
        }
    }
}

/// A dataset of a group: its name and header address.
type Member = (String, u64);

fn links(members: Vec<Member>) -> Vec<Link> {
    members.into_iter().map(|(n, a)| Link::Hard(n, a)).collect()
}

/// A patch's group name: its own name unless HDF5 or the layout cannot
/// take it (a `/`, `.`, empty, or a name of the layout or of another
/// patch).
fn group_name(name: &str, taken: &[String]) -> String {
    let mut s: String = name.replace('/', "_");
    if s.is_empty() || s == "." || s == ".." {
        s = "patch".into();
    }
    let base = s.clone();
    let mut k = 1;
    while ["internal", "Assembly"].contains(&s.as_str()) || taken.contains(&s) {
        s = format!("{base}_{k}");
        k += 1;
    }
    s
}

/// Write `mesh` and `results` as a blocked VTKHDF file at `path` with
/// `block_cells` owned cells per block (the last block holds the rest).
/// Any size from 1 is written; files for others to read use
/// [`MIN_BLOCK_CELLS`] to [`MAX_BLOCK_CELLS`] (the `blocks.make` tool
/// checks that).
pub fn write_blocked(
    path: &Path,
    mesh: &Mesh,
    results: &Results,
    block_cells: usize,
) -> Result<Written, MeshError> {
    let n = mesh.cell_count();
    if n == 0 {
        return Err(MeshError::new(
            "BLOCK003",
            "the mesh has no volume cells to put in blocks",
        ));
    }
    if block_cells == 0 {
        return Err(MeshError::new(
            "BLOCK002",
            "a block needs at least one cell",
        ));
    }
    let cells = {
        let _t = crate::profile::span("blocks.cells");
        vtk_cells(mesh)
    };
    let plan = {
        let _t = crate::profile::span("blocks.plan");
        plan(mesh, &cells, block_cells)
    };
    let _t = crate::profile::span("blocks.write");
    write(path, mesh, results, &cells, &plan).map_err(failed)
}

fn write(
    path: &Path,
    mesh: &Mesh,
    results: &Results,
    cells: &VtkCells,
    plan: &Plan,
) -> io::Result<Written> {
    let nb = plan.blocks();
    let poly = POLYHEDRON as u8;
    let any_poly = cells.ty.contains(&poly);
    let mut w = H5Writer::create(path)?;
    let mut internal: Vec<Member> = Vec::new();

    // counts per block
    let n_points: Vec<i64> = plan.points.iter().map(|p| p.len() as i64).collect();
    let n_cells: Vec<i64> = (0..nb).map(|b| plan.cells(b).count() as i64).collect();
    let n_conn: Vec<i64> = (0..nb)
        .map(|b| {
            plan.cells(b)
                .map(|c| cells.points(c as usize).len() as i64)
                .sum()
        })
        .collect();
    internal.push(("NumberOfPoints".into(), w.dataset(&[nb as u64], &n_points)?));
    internal.push(("NumberOfCells".into(), w.dataset(&[nb as u64], &n_cells)?));
    internal.push((
        "NumberOfConnectivityIds".into(),
        w.dataset(&[nb as u64], &n_conn)?,
    ));
    let a = per_block::<f64>(&mut w, nb, 3, |b, out| {
        for &p in &plan.points[b] {
            out.extend(mesh.points[p as usize]);
        }
    })?;
    internal.push(("Points".into(), a));
    let a = per_block::<u8>(&mut w, nb, 1, |b, out| {
        out.extend(plan.cells(b).map(|c| cells.ty[c as usize]));
    })?;
    internal.push(("Types".into(), a));
    let a = per_block::<i64>(&mut w, nb, 1, |b, out| {
        let mut o = 0i64;
        out.push(0);
        for c in plan.cells(b) {
            o += cells.points(c as usize).len() as i64;
            out.push(o);
        }
    })?;
    internal.push(("Offsets".into(), a));
    let a = per_block::<i64>(&mut w, nb, 1, |b, out| {
        for c in plan.cells(b) {
            out.extend(cells.points(c as usize).iter().map(|&p| plan.local(b, p)));
        }
    })?;
    internal.push(("Connectivity".into(), a));

    if any_poly {
        let mut nf = Vec::with_capacity(nb);
        let mut nfc = Vec::with_capacity(nb);
        for b in 0..nb {
            let (mut f, mut k) = (0i64, 0i64);
            for c in plan.cells(b) {
                if cells.ty[c as usize] == poly {
                    for face in mesh.cell(c as usize) {
                        f += 1;
                        k += face.len() as i64;
                    }
                }
            }
            nf.push(f);
            nfc.push(k);
        }
        internal.push(("NumberOfFaces".into(), w.dataset(&[nb as u64], &nf)?));
        internal.push((
            "NumberOfPolyhedronToFaceIds".into(),
            w.dataset(&[nb as u64], &nf)?,
        ));
        internal.push((
            "NumberOfFaceConnectivityIds".into(),
            w.dataset(&[nb as u64], &nfc)?,
        ));
        let a = per_block::<i64>(&mut w, nb, 1, |b, out| {
            let mut k = 0i64;
            out.push(0);
            for c in plan.cells(b) {
                if cells.ty[c as usize] == poly {
                    for face in mesh.cell(c as usize) {
                        k += face.len() as i64;
                        out.push(k);
                    }
                }
            }
        })?;
        internal.push(("FaceOffsets".into(), a));
        let a = per_block::<i64>(&mut w, nb, 1, |b, out| {
            for c in plan.cells(b) {
                if cells.ty[c as usize] == poly {
                    for face in mesh.cell(c as usize) {
                        out.extend(face.oriented().map(|p| plan.local(b, p)));
                    }
                }
            }
        })?;
        internal.push(("FaceConnectivity".into(), a));
        let a = per_block::<i64>(&mut w, nb, 1, |b, out| {
            let mut f = 0i64;
            out.push(0);
            for c in plan.cells(b) {
                if cells.ty[c as usize] == poly {
                    f += mesh.cell_face_range(c as usize).len() as i64;
                }
                out.push(f);
            }
        })?;
        internal.push(("PolyhedronOffsets".into(), a));
        let a = per_block::<i64>(&mut w, nb, 1, |b, out| {
            let mut f = 0i64;
            for c in plan.cells(b) {
                if cells.ty[c as usize] == poly {
                    for _ in mesh.cell_face_range(c as usize) {
                        out.push(f);
                        f += 1;
                    }
                }
            }
        })?;
        internal.push(("PolyhedronToFaces".into(), a));
    }

    // cell data
    let mut cd: Vec<Member> = Vec::new();
    let a = per_block::<u32>(&mut w, nb, 1, |b, out| {
        out.extend(plan.cells(b).map(|c| mesh.cell_zone[c as usize]));
    })?;
    cd.push(("zone".into(), a));
    let a = per_block::<i64>(&mut w, nb, 1, |b, out| {
        out.extend(plan.cells(b).map(|c| i64::from(plan.file_id[c as usize])));
    })?;
    cd.push(("GlobalCellIds".into(), a));
    let a = per_block::<i64>(&mut w, nb, 1, |b, out| {
        out.extend(plan.cells(b).map(i64::from));
    })?;
    cd.push(("OriginalCellIds".into(), a));
    let a = per_block::<u8>(&mut w, nb, 1, |b, out| {
        out.extend(std::iter::repeat_n(0u8, plan.owned(b).len()));
        out.extend(std::iter::repeat_n(1u8, plan.ghosts[b].len()));
    })?;
    cd.push(("vtkGhostType".into(), a));
    // point data
    let mut pd: Vec<Member> = Vec::new();
    let a = per_block::<i64>(&mut w, nb, 1, |b, out| {
        out.extend(plan.points[b].iter().map(|&p| i64::from(p)));
    })?;
    pd.push(("GlobalPointIds".into(), a));
    let a = per_block::<u8>(&mut w, nb, 1, |b, out| {
        out.extend(
            plan.points[b]
                .iter()
                .map(|&p| u8::from(plan.point_owner[p as usize] as usize != b)),
        );
    })?;
    pd.push(("vtkGhostType".into(), a));

    // fields and their block ranges
    let mut written = Written {
        blocks: nb,
        block_cells: plan.bs,
        cells: mesh.cell_count(),
        ghost_cells: plan.ghosts.iter().map(Vec::len).sum(),
        points: mesh.points.len(),
        block_points: plan.points.iter().map(Vec::len).sum(),
        linear_cells: cells.ty.iter().filter(|&&t| t != poly).count(),
        polyhedra: cells.ty.iter().filter(|&&t| t == poly).count(),
        patches: mesh.patches.len(),
        ..Written::default()
    };
    let mut fd: Vec<Member> = Vec::new();
    let mut ranges: Vec<Member> = Vec::new();
    for f in &results.fields {
        if RESERVED.contains(&f.name.as_str()) {
            written.skipped_fields.push(f.name.clone());
            continue;
        }
        let k = f.components.max(1) as u64;
        let range = |ids: &mut dyn Iterator<Item = u32>| {
            let (lo, hi) = range_of(ids.map(|i| f.scalar(i as usize, crate::Component::Magnitude)))
                .unwrap_or((f64::NAN, f64::NAN));
            [lo, hi]
        };
        match f.location {
            Location::Cell => {
                let a = per_block::<f64>(&mut w, nb, k, |b, out| rows_of(f, plan.cells(b), out))?;
                cd.push((f.name.clone(), a));
                let r: Vec<f64> = (0..nb)
                    .flat_map(|b| range(&mut plan.owned(b).iter().copied()))
                    .collect();
                ranges.push((
                    format!("BlockRange_{}", f.name),
                    w.dataset(&[nb as u64, 2], &r)?,
                ));
            }
            Location::Point => {
                let a = per_block::<f64>(&mut w, nb, k, |b, out| {
                    rows_of(f, plan.points[b].iter().copied(), out)
                })?;
                pd.push((f.name.clone(), a));
                let r: Vec<f64> = (0..nb)
                    .flat_map(|b| {
                        range(
                            &mut plan.points[b]
                                .iter()
                                .copied()
                                .filter(|&p| plan.point_owner[p as usize] as usize == b),
                        )
                    })
                    .collect();
                ranges.push((
                    format!("BlockRange_{}", f.name),
                    w.dataset(&[nb as u64, 2], &r)?,
                ));
            }
            Location::Boundary => continue,
        }
        written.fields += 1;
    }
    let owned: Vec<i64> = (0..nb).map(|b| plan.owned(b).len() as i64).collect();
    fd.push(("BlockOwnedCells".into(), w.dataset(&[nb as u64], &owned)?));
    let bounds: Vec<f64> = plan
        .bounds
        .iter()
        .flat_map(|b| b.min.into_iter().chain(b.max))
        .collect();
    fd.push(("BlockBounds".into(), w.dataset(&[nb as u64, 6], &bounds)?));
    fd.extend(ranges);
    if !mesh.zones.is_empty() {
        let names: Vec<String> = mesh
            .zones
            .iter()
            .enumerate()
            .map(|(i, z)| format!("{i}={z}"))
            .collect();
        fd.push(("zone_names".into(), w.strings(&names)?));
    }
    let cd = w.group(&links(cd), &[])?;
    let pd = w.group(&links(pd), &[])?;
    let fd = w.group(&links(fd), &[])?;
    internal.push(("CellData".into(), cd));
    internal.push(("PointData".into(), pd));
    internal.push(("FieldData".into(), fd));
    let internal = w.group(
        &links(internal),
        &[
            ("Index", AttrValue::Int(0)),
            ("Type", AttrValue::Text("UnstructuredGrid".into())),
            ("Version", AttrValue::Ints(vec![2, 4])),
            ("block_cells", AttrValue::Int(plan.bs as i64)),
        ],
    )?;

    // boundary patches
    let mut by_patch: Vec<Vec<u32>> = vec![Vec::new(); mesh.patches.len()];
    for (i, &p) in mesh.boundary_patch.iter().enumerate() {
        if let Some(v) = by_patch.get_mut(p as usize) {
            v.push(i as u32);
        }
    }
    let mut patches: Vec<(String, u64)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for (pi, (patch, faces)) in mesh.patches.iter().zip(&by_patch).enumerate() {
        let polygon = |i: u32| -> &[u32] {
            match mesh.boundary_face_id.get(i as usize) {
                Some(&u) if u != NO_CELL && (u as usize) < mesh.distinct.len() => {
                    mesh.distinct.get(u as usize)
                }
                _ => mesh.boundary.get(i as usize),
            }
        };
        let mut start = vec![0usize];
        let mut nodes = Vec::new();
        for &i in faces {
            nodes.extend_from_slice(polygon(i));
            start.push(nodes.len());
        }
        // cell data: the owner of each face and the boundary fields
        let owner: Vec<i64> = faces
            .iter()
            .map(|&i| match mesh.boundary_owner.get(i as usize) {
                Some(&c) if c != NO_CELL => i64::from(plan.file_id[c as usize]),
                _ => -1,
            })
            .collect();
        let mut values: Vec<(&str, usize, Vec<f64>)> = Vec::new();
        for f in &results.fields {
            if f.location != Location::Boundary || RESERVED.contains(&f.name.as_str()) {
                continue;
            }
            let mut v = Vec::new();
            rows_of(f, faces.iter().copied(), &mut v);
            values.push((f.name.as_str(), f.components, v));
        }
        let xyz = |p: u32| mesh.points[p as usize];
        let addr = patch_block(
            &mut w,
            &start,
            &nodes,
            &xyz,
            &owner,
            &values,
            pi + 1,
            patch.role,
        )?;
        let name = group_name(&patch.name, &names);
        names.push(name.clone());
        patches.push((name, addr));
    }
    written.fields += results
        .fields
        .iter()
        .filter(|f| f.location == Location::Boundary && !RESERVED.contains(&f.name.as_str()))
        .count();
    for f in &results.fields {
        if f.location == Location::Boundary && RESERVED.contains(&f.name.as_str()) {
            written.skipped_fields.push(f.name.clone());
        }
    }
    // the file's root: the attributes of the run, the units of the fields
    let units: Vec<String> = results
        .fields
        .iter()
        .filter_map(|f| f.unit.as_ref().map(|u| format!("{} {u}", f.name)))
        .collect();
    let mut root_attrs: Vec<(&str, AttrValue)> = results
        .attrs
        .iter()
        .filter(|(k, _)| k.as_str() != "units")
        .map(|(k, v)| (k.as_str(), AttrValue::Text(v.clone())))
        .collect();
    let units = units.join("; ");
    if !units.is_empty() {
        root_attrs.push(("units", AttrValue::Text(units)));
    }
    written.bytes = close(w, internal, &patches, &mesh.metadata, &root_attrs)?;
    Ok(written)
}

/// A boundary patch as a PolyData block of one partition (§ 7): polygon
/// `i` is `nodes[start[i]..start[i + 1]]` in global point numbers (`xyz`
/// gives their coordinates), `owner` the file cell id behind each face
/// and `fields` the patch's own values (name, components, values); the
/// patch is block `index` of the collection. Returns the group's address.
#[allow(clippy::too_many_arguments)]
pub(crate) fn patch_block(
    w: &mut H5Writer,
    start: &[usize],
    nodes: &[u32],
    xyz: &dyn Fn(u32) -> [f64; 3],
    owner: &[i64],
    fields: &[(&str, usize, Vec<f64>)],
    index: usize,
    role: crate::PatchRole,
) -> io::Result<u64> {
    let faces = start.len().saturating_sub(1);
    let mut pts: Vec<u32> = nodes.to_vec();
    pts.sort_unstable();
    pts.dedup();
    let local = |p: u32| pts.binary_search(&p).map_or(-1, |i| i as i64);
    let mut g: Vec<Member> = Vec::new();
    g.push((
        "NumberOfPoints".into(),
        w.dataset(&[1], &[pts.len() as i64])?,
    ));
    let coords: Vec<f64> = pts.iter().flat_map(|&p| xyz(p)).collect();
    g.push(("Points".into(), w.dataset(&[pts.len() as u64, 3], &coords)?));
    drop(coords);
    for sub in ["Vertices", "Lines", "Strips"] {
        let m = vec![
            ("NumberOfCells".into(), w.dataset(&[1], &[0i64])?),
            ("NumberOfConnectivityIds".into(), w.dataset(&[1], &[0i64])?),
            ("Offsets".into(), w.dataset(&[1], &[0i64])?),
            ("Connectivity".into(), w.dataset::<i64>(&[0], &[])?),
        ];
        g.push((sub.into(), w.group(&links(m), &[])?));
    }
    let offs: Vec<i64> = start.iter().map(|&s| s as i64).collect();
    let conn: Vec<i64> = nodes.iter().map(|&p| local(p)).collect();
    let m = vec![
        ("NumberOfCells".into(), w.dataset(&[1], &[faces as i64])?),
        (
            "NumberOfConnectivityIds".into(),
            w.dataset(&[1], &[conn.len() as i64])?,
        ),
        ("Offsets".into(), w.dataset(&[offs.len() as u64], &offs)?),
        (
            "Connectivity".into(),
            w.dataset(&[conn.len() as u64], &conn)?,
        ),
    ];
    g.push(("Polygons".into(), w.group(&links(m), &[])?));
    let mut pcd: Vec<Member> = vec![("OwnerCell".into(), w.dataset(&[faces as u64], owner)?)];
    for (name, k, v) in fields {
        let dims: Vec<u64> = if *k > 1 {
            vec![faces as u64, *k as u64]
        } else {
            vec![faces as u64]
        };
        pcd.push((name.to_string(), w.dataset(&dims, v)?));
    }
    let ids: Vec<i64> = pts.iter().map(|&p| i64::from(p)).collect();
    let ppd = vec![(
        "GlobalPointIds".into(),
        w.dataset(&[pts.len() as u64], &ids)?,
    )];
    g.push(("CellData".into(), w.group(&links(pcd), &[])?));
    g.push(("PointData".into(), w.group(&links(ppd), &[])?));
    g.push(("FieldData".into(), w.group(&[], &[])?));
    let mut attrs = vec![
        ("Index", AttrValue::Int(index as i64)),
        ("Type", AttrValue::Text("PolyData".into())),
        ("Version", AttrValue::Ints(vec![2, 4])),
    ];
    if role != crate::PatchRole::Unspecified {
        attrs.push(("role", AttrValue::Text(role.name().into())));
    }
    w.group(&links(g), &attrs)
}

/// Close a blocked file: the assembly of `internal` and the `patches`
/// (group names and addresses), `/VTKHDF` with the layout's attributes
/// and the mesh's `metadata`, and the root group with `root_attrs`.
/// Returns the bytes written before the superblock.
pub(crate) fn close(
    mut w: H5Writer,
    internal: u64,
    patches: &[(String, u64)],
    metadata: &std::collections::BTreeMap<String, String>,
    root_attrs: &[(&str, AttrValue)],
) -> io::Result<u64> {
    let mut top: Vec<Link> = vec![Link::Hard("internal".into(), internal)];
    top.extend(patches.iter().map(|(n, a)| Link::Hard(n.clone(), *a)));
    // the assembly: the cells, and the patches under `boundary`
    let boundary: Vec<Link> = patches
        .iter()
        .map(|(n, _)| Link::Soft(n.clone(), format!("/VTKHDF/{n}")))
        .collect();
    let boundary = w.group(&boundary, &[])?;
    let assembly = w.group(
        &[
            Link::Soft("internal".into(), "/VTKHDF/internal".into()),
            Link::Hard("boundary".into(), boundary),
        ],
        &[],
    )?;
    top.push(Link::Hard("Assembly".into(), assembly));
    let mut attrs: Vec<(&str, AttrValue)> = vec![
        ("Version", AttrValue::Ints(vec![2, 4])),
        (
            "Type",
            AttrValue::Text("PartitionedDataSetCollection".into()),
        ),
        ("murmelcraft_blocks", AttrValue::Int(LAYOUT)),
        ("block_order", AttrValue::Text("hilbert".into())),
    ];
    for (k, v) in metadata {
        if matches!(
            k.as_str(),
            "vtkhdf" | "Version" | "Type" | "murmelcraft_blocks" | "block_order"
        ) {
            continue;
        }
        attrs.push((k.as_str(), AttrValue::Text(v.clone())));
    }
    let vtk = w.group(&top, &attrs)?;
    let root = w.group(&[Link::Hard("VTKHDF".into(), vtk)], root_attrs)?;
    let bytes = w.pos();
    w.finish(root)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::Field;
    use crate::hdf5::H5;
    use std::collections::{BTreeSet, HashMap};

    fn fixture(rel: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
            .join(rel)
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("murmel-blocks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// Signed volume of cell `c` by the divergence theorem over its faces
    /// as the cell lists them (positive when they point out).
    fn volume(m: &Mesh, c: usize) -> f64 {
        let mut v = 0.0;
        for f in m.cell(c) {
            let p: Vec<[f64; 3]> = f.oriented().map(|i| m.points[i as usize]).collect();
            for k in 1..p.len() - 1 {
                let (a, b, d) = (p[0], p[k], p[k + 1]);
                v += a[0] * (b[1] * d[2] - b[2] * d[1]) - a[1] * (b[0] * d[2] - b[2] * d[0])
                    + a[2] * (b[0] * d[1] - b[1] * d[0]);
            }
        }
        v / 6.0
    }

    /// The coordinates of cell `c`'s points, as exact bits.
    fn corners(m: &Mesh, c: usize) -> BTreeSet<[u64; 3]> {
        m.cell_points(c)
            .map(|p| m.points[p as usize].map(f64::to_bits))
            .collect()
    }

    /// The linear cells of a file, and how many of them have their base
    /// (0, 1, 2) turning away from the rest of the cell: those have a
    /// negative volume in VTK (`vtkCellSizeFilter`, `vtkMeshQuality`).
    /// The reader turns any cell out, so a round trip alone would not see
    /// them.
    fn linear_cells_turned_in(path: &Path) -> (usize, usize) {
        let h = H5::open(path).unwrap();
        let g = h.path("/VTKHDF/internal").unwrap();
        let read = |n: &str| {
            h.read_i64(&h.dataset(&h.child(&g, n).unwrap().unwrap()).unwrap())
                .unwrap()
        };
        let (np, nc) = (read("NumberOfPoints"), read("NumberOfCells"));
        let (types, offs, conn) = (read("Types"), read("Offsets"), read("Connectivity"));
        let xyz = h
            .read_f64(&h.dataset(&h.child(&g, "Points").unwrap().unwrap()).unwrap())
            .unwrap();
        let (mut pb, mut cb, mut ob, mut kb) = (0usize, 0usize, 0usize, 0usize);
        let (mut linear, mut turned) = (0, 0);
        for part in 0..np.len() {
            for c in 0..nc[part] as usize {
                let (s, e) = (offs[ob + c] as usize, offs[ob + c + 1] as usize);
                if types[cb + c] == POLYHEDRON {
                    continue;
                }
                let p: Vec<[f64; 3]> = conn[kb + s..kb + e]
                    .iter()
                    .map(|&i| {
                        let r = 3 * (pb + i as usize);
                        [xyz[r], xyz[r + 1], xyz[r + 2]]
                    })
                    .collect();
                let base = if types[cb + c] == HEXAHEDRON || types[cb + c] == PYRAMID {
                    4
                } else {
                    3
                };
                let d = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                let (u, v) = (d(p[1], p[0]), d(p[2], p[0]));
                let n = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                let k = (p.len() - base) as f64;
                let rest = p[base..].iter().fold([0.0; 3], |a, q| {
                    [a[0] + q[0] / k, a[1] + q[1] / k, a[2] + q[2] / k]
                });
                let t = d(rest, p[0]);
                linear += 1;
                if n[0] * t[0] + n[1] * t[1] + n[2] * t[2] <= 0.0 {
                    turned += 1;
                }
            }
            pb += np[part] as usize;
            cb += nc[part] as usize;
            ob += nc[part] as usize + 1;
            kb += offs[ob - 1] as usize;
        }
        (linear, turned)
    }

    /// The writer's cell number of each file cell id, from the file.
    fn original_ids(path: &Path) -> Vec<usize> {
        let h = H5::open(path).unwrap();
        let cd = h.path("/VTKHDF/internal/CellData").unwrap();
        let read = |n: &str| {
            h.read_i64(&h.dataset(&h.child(&cd, n).unwrap().unwrap()).unwrap())
                .unwrap()
        };
        let (orig, ghost) = (read("OriginalCellIds"), read("vtkGhostType"));
        orig.into_iter()
            .zip(ghost)
            .filter(|&(_, g)| g == 0)
            .map(|(o, _)| o as usize)
            .collect()
    }

    /// Invariant: a blocked file of FoehnCraft's channel (wedges and
    /// polyhedra, three patches, cell and boundary fields), cut into
    /// blocks far smaller than the mesh so that every block has ghosts,
    /// reads back as the same mesh: every cell with the same points, zone,
    /// kind, volume (so its faces still point out) and field values, every
    /// patch with its name, role, faces and boundary fields, and the run's
    /// attributes. The ghost cells and repeated points of the partitions
    /// are gone. Breaks if the reader kept a ghost, split a point between
    /// blocks, or the writer turned a cell or mixed up the rows of a field.
    #[test]
    fn a_blocked_file_reads_back_as_the_same_mesh() {
        let (m, r) = crate::read_results(&fixture("vtkhdf/channel_fields.vtkhdf")).unwrap();
        let path = scratch("channel.vtkhdf");
        let w = write_blocked(&path, &m, &r, 100).unwrap();
        assert_eq!(w.blocks, 8);
        assert_eq!(w.cells, 776);
        assert_eq!((w.linear_cells, w.polyhedra), (768, 8));
        assert!(w.ghost_cells > 0 && w.block_points > m.points.len());
        // every linear cell, owned or ghost, has its base turned in
        let (linear, turned) = linear_cells_turned_in(&path);
        assert!(linear > 768);
        assert_eq!(turned, 0);
        let (b, rb) = crate::read_results(&path).unwrap();
        assert_eq!(b.cell_count(), m.cell_count());
        assert_eq!(b.points.len(), m.points.len());
        assert_eq!(
            b.metadata.get("murmelcraft_blocks").map(String::as_str),
            Some("1")
        );
        assert_eq!(b.zones, m.zones);
        let orig = original_ids(&path);
        assert_eq!(orig.len(), m.cell_count());
        for (f, &c) in orig.iter().enumerate() {
            assert_eq!(corners(&b, f), corners(&m, c), "cell {f} (was {c})");
            assert_eq!(b.cell_zone[f], m.cell_zone[c]);
            assert_eq!(b.cell_kind[f], m.cell_kind[c]);
            let (vb, vm) = (volume(&b, f), volume(&m, c));
            assert!(
                vm > 0.0 && (vb - vm).abs() <= 1e-12 * vm.abs().max(1.0),
                "{vb} vs {vm}"
            );
        }
        for f in &r.fields {
            let g = rb
                .get(&f.name)
                .unwrap_or_else(|| panic!("no field {}", f.name));
            assert_eq!(
                (g.location, g.components, g.unit.clone()),
                (f.location, f.components, f.unit.clone())
            );
            match f.location {
                Location::Cell => {
                    for (k, &c) in orig.iter().enumerate() {
                        assert_eq!(g.value(k), f.value(c), "{} of cell {k}", f.name);
                    }
                }
                Location::Boundary => {
                    // the faces keep their order within each patch
                    for p in 0..m.patches.len() as u32 {
                        let of = |mesh: &Mesh, fld: &Field| -> Vec<Vec<f64>> {
                            (0..mesh.boundary.len())
                                .filter(|&i| mesh.boundary_patch[i] == p)
                                .map(|i| fld.value(i).unwrap().to_vec())
                                .collect()
                        };
                        assert_eq!(of(&b, g), of(&m, f), "{} on patch {p}", f.name);
                    }
                }
                Location::Point => unreachable!("the channel has no point fields"),
            }
        }
        assert_eq!(b.patches, m.patches);
        for p in 0..m.patches.len() as u32 {
            let faces = |mesh: &Mesh| -> Vec<BTreeSet<[u64; 3]>> {
                (0..mesh.boundary.len())
                    .filter(|&i| mesh.boundary_patch[i] == p)
                    .map(|i| {
                        mesh.boundary
                            .get(i)
                            .iter()
                            .map(|&q| mesh.points[q as usize].map(f64::to_bits))
                            .collect()
                    })
                    .collect()
            };
            assert_eq!(faces(&b), faces(&m), "patch {p}");
        }
        assert!(b.boundary_owner.iter().all(|&c| c != NO_CELL));
        assert_eq!(rb.iteration, r.iteration);
        assert_eq!(rb.attrs.get("case"), r.attrs.get("case"));
        std::fs::remove_file(&path).ok();
    }

    /// Invariant: the blocks are what § 2 to § 6 of the layout say. File
    /// cell ids run 0 … n − 1 over the owned cells block by block; each
    /// block has `block_cells` owned cells but the last; its ghosts are
    /// exactly the cells of other blocks that share a point with one of
    /// its owned cells (found here by brute force), each with the file
    /// cell id of the cell it copies; a point is a duplicate (its ghost
    /// type is 1) unless this block is the lowest one whose owned cells
    /// use it;
    /// the block bounds hold the points of the owned cells; and the block
    /// ranges of a cell and a point field are the minimum and maximum
    /// over the owned cells and owned points. Uses the synthetic
    /// honeycomb (polyhedra only) with a cell and a point field.
    #[test]
    fn the_blocks_follow_the_layout() {
        let m = crate::synth::honeycomb(6);
        let n = m.cell_count();
        let cell_field: Vec<f64> = (0..n).map(|c| c as f64 * 0.5 - 7.0).collect();
        let point_field: Vec<f64> = m.points.iter().map(|p| p[0] + 2.0 * p[1] - p[2]).collect();
        let r = Results {
            fields: vec![
                Field {
                    name: "c".into(),
                    unit: Some("Pa".into()),
                    location: Location::Cell,
                    components: 1,
                    values: cell_field.clone(),
                },
                Field {
                    name: "p".into(),
                    unit: None,
                    location: Location::Point,
                    components: 1,
                    values: point_field.clone(),
                },
            ],
            ..Results::default()
        };
        let path = scratch("honeycomb.vtkhdf");
        let bs = 64;
        let w = write_blocked(&path, &m, &r, bs).unwrap();
        assert_eq!(w.blocks, n.div_ceil(bs));
        let h = H5::open(&path).unwrap();
        let g = h.path("/VTKHDF/internal").unwrap();
        let ds = |path: &str| -> Vec<i64> {
            let o = h.path(&format!("/VTKHDF/internal/{path}")).unwrap();
            h.read_i64(&h.dataset(&o).unwrap()).unwrap()
        };
        let dsf = |path: &str| -> Vec<f64> {
            let o = h.path(&format!("/VTKHDF/internal/{path}")).unwrap();
            h.read_f64(&h.dataset(&o).unwrap()).unwrap()
        };
        assert_eq!(
            h.attr(&g, "block_cells").unwrap().unwrap().number(),
            Some(bs as f64)
        );
        let (nc, np) = (ds("NumberOfCells"), ds("NumberOfPoints"));
        let owned = ds("FieldData/BlockOwnedCells");
        let (gc, orig, cg) = (
            ds("CellData/GlobalCellIds"),
            ds("CellData/OriginalCellIds"),
            ds("CellData/vtkGhostType"),
        );
        let (gp, pg) = (ds("PointData/GlobalPointIds"), ds("PointData/vtkGhostType"));
        let bounds = dsf("FieldData/BlockBounds");
        let (rc, rp) = (dsf("FieldData/BlockRange_c"), dsf("FieldData/BlockRange_p"));
        let cvals = dsf("CellData/c");
        let pvals = dsf("PointData/p");
        assert_eq!(owned.iter().sum::<i64>() as usize, n);
        // the file cell id and the block of each writer's cell
        let mut file_of = vec![usize::MAX; n];
        let (mut cb, mut pb, mut next_id) = (0usize, 0usize, 0i64);
        let mut lowest = vec![usize::MAX; m.points.len()];
        let mut blocks: Vec<(Vec<usize>, Vec<usize>, Vec<usize>)> = Vec::new();
        for b in 0..w.blocks {
            let (c, p, o) = (nc[b] as usize, np[b] as usize, owned[b] as usize);
            assert_eq!(o, if b + 1 < w.blocks { bs } else { n - bs * b });
            let mut own = Vec::new();
            let mut ghosts = Vec::new();
            for k in cb..cb + c {
                if k < cb + o {
                    assert_eq!(cg[k], 0);
                    assert_eq!(gc[k], next_id, "file cell ids run in order");
                    next_id += 1;
                    file_of[orig[k] as usize] = gc[k] as usize;
                    own.push(orig[k] as usize);
                } else {
                    assert_eq!(cg[k], 1, "ghosts follow the owned cells");
                    ghosts.push(orig[k] as usize);
                }
            }
            let pts: Vec<usize> = gp[pb..pb + p].iter().map(|&q| q as usize).collect();
            blocks.push((own, ghosts, pts));
            cb += c;
            pb += p;
        }
        for (b, (own, _, _)) in blocks.iter().enumerate() {
            for &c in own {
                for q in m.cell_points(c) {
                    lowest[q as usize] = lowest[q as usize].min(b);
                }
            }
        }
        let block_of = |c: usize| file_of[c] / bs;
        let mut pb = 0usize;
        let mut cb = 0usize;
        for (b, (own, ghosts, pts)) in blocks.iter().enumerate() {
            // ghosts: the point neighbours in other blocks, each once
            let used: BTreeSet<u32> = own.iter().flat_map(|&c| m.cell_points(c)).collect();
            let want: BTreeSet<usize> = (0..n)
                .filter(|&c| block_of(c) != b && m.cell_points(c).any(|q| used.contains(&q)))
                .collect();
            let have: BTreeSet<usize> = ghosts.iter().copied().collect();
            assert_eq!(have.len(), ghosts.len());
            assert_eq!(have, want, "ghosts of block {b}");
            // ghost rows carry their original's id and value
            for (k, &c) in own.iter().chain(ghosts).enumerate() {
                assert_eq!(gc[cb + k] as usize, file_of[c]);
                assert_eq!(cvals[cb + k], cell_field[c]);
            }
            // points: each once, duplicates marked, values copied
            let set: BTreeSet<usize> = pts.iter().copied().collect();
            assert_eq!(set.len(), pts.len());
            for (k, &q) in pts.iter().enumerate() {
                assert_eq!(pg[pb + k] == 0, lowest[q] == b, "point {q} in block {b}");
                assert_eq!(pvals[pb + k], point_field[q]);
            }
            // bounds and ranges over what the block owns
            let mut bx = Bounds::empty();
            for &q in &used {
                bx.add(m.points[q as usize]);
            }
            assert_eq!(
                &bounds[6 * b..6 * b + 6],
                [bx.min, bx.max].concat().as_slice()
            );
            let cr = range_of(own.iter().map(|&c| cell_field[c])).unwrap();
            assert_eq!(&rc[2 * b..2 * b + 2], [cr.0, cr.1].as_slice());
            let pr = range_of(
                pts.iter()
                    .filter(|&&q| lowest[q] == b)
                    .map(|&q| point_field[q]),
            )
            .unwrap();
            assert_eq!(&rp[2 * b..2 * b + 2], [pr.0, pr.1].as_slice());
            pb += pts.len();
            cb += own.len() + ghosts.len();
        }
        // and the whole mesh reads back with its fields
        let (back, rb) = crate::read_results(&path).unwrap();
        assert_eq!((back.cell_count(), back.points.len()), (n, m.points.len()));
        let p = rb.get("p").unwrap();
        let by_coord: HashMap<[u64; 3], f64> = m
            .points
            .iter()
            .zip(&point_field)
            .map(|(q, &v)| (q.map(f64::to_bits), v))
            .collect();
        for (i, q) in back.points.iter().enumerate() {
            assert_eq!(p.values[i], by_coord[&q.map(f64::to_bits)]);
        }
        assert_eq!(rb.get("c").unwrap().unit.as_deref(), Some("Pa"));
        std::fs::remove_file(&path).ok();
    }

    /// Invariant: a tetrahedron, pyramid, wedge and hexahedron given as
    /// faces in any order, each face starting at any of its points, are
    /// found as VTK's linear cell with nodes whose face table gives back
    /// the same faces; turning one face round (an inconsistent cell) or
    /// collapsing a node makes the cell a polyhedron instead.
    #[test]
    fn linear_cells_are_found_from_their_faces() {
        let cases: [(CellKind, i64, &[&[usize]], usize); 4] = [
            (CellKind::Tetrahedron, TETRA, TETRA_FACES, 4),
            (CellKind::Pyramid, PYRAMID, PYRAMID_FACES, 5),
            (CellKind::Prism, WEDGE, WEDGE_FACES, 6),
            (CellKind::Hexahedron, HEXAHEDRON, HEX_FACES, 8),
        ];
        for (kind, ty, table, nodes) in cases {
            // point ids that are not 0 … n − 1, so a mix-up shows
            let ids: Vec<u32> = (0..nodes as u32).map(|i| 100 + 7 * i).collect();
            for shift in 0..table.len() {
                let mut f = Faces::default();
                f.start.push(0);
                for k in 0..table.len() {
                    let t = table[(k + shift) % table.len()];
                    let r = (k + shift) % t.len();
                    f.nodes
                        .extend((0..t.len()).map(|i| ids[t[(i + r) % t.len()]]));
                    f.start.push(f.nodes.len());
                }
                let (got, n) = linear(kind, &f).unwrap_or_else(|| panic!("{kind:?} shift {shift}"));
                assert_eq!(got, ty);
                // the table on the found nodes gives the same faces
                let canon = |v: Vec<u32>| {
                    let m = (0..v.len()).min_by_key(|&i| v[i]).unwrap();
                    (0..v.len())
                        .map(|i| v[(i + m) % v.len()])
                        .collect::<Vec<u32>>()
                };
                let mut a: Vec<Vec<u32>> = table
                    .iter()
                    .map(|t| canon(t.iter().map(|&i| n[i]).collect()))
                    .collect();
                let mut b: Vec<Vec<u32>> = (0..f.len()).map(|i| canon(f.get(i).to_vec())).collect();
                a.sort();
                b.sort();
                assert_eq!(a, b, "{kind:?}");
                // one face turned round: not a consistent cell
                let mut bad = Faces {
                    start: f.start.clone(),
                    nodes: f.nodes.clone(),
                };
                bad.nodes[f.start[1]..f.start[2]].reverse();
                assert!(linear(kind, &bad).is_none(), "{kind:?} with a turned face");
                // a collapsed node
                let mut col = Faces {
                    start: f.start.clone(),
                    nodes: f.nodes.clone(),
                };
                for v in &mut col.nodes {
                    if *v == ids[1] {
                        *v = ids[0];
                    }
                }
                assert!(linear(kind, &col).is_none(), "{kind:?} collapsed");
            }
        }
    }

    /// Invariant: KoflCraft's own blocked files (another writer of the
    /// layout) read as the same mesh as their `.vtu` twins: the same
    /// counts, zones, patches with roles, metadata digests and cells (by
    /// the coordinates of their points). Breaks if the reader depends on a
    /// choice the layout leaves to the writer, such as the order of a
    /// block's points.
    #[test]
    fn koflcraft_blocked_files_read_as_their_vtu_twins() {
        for (blocked, vtu) in [
            ("vtkhdf/koflcraft/channel.vtkhdf", "channel/foehncraft.vtu"),
            ("vtkhdf/koflcraft/box4.vtkhdf", "box/foehncraft.vtu"),
        ] {
            let b = crate::read(&fixture(blocked)).unwrap();
            let v = crate::read(&fixture(vtu)).unwrap();
            assert_eq!(
                b.metadata.get("murmelcraft_blocks").map(String::as_str),
                Some("1")
            );
            let (sb, sv) = (b.summary(), v.summary());
            assert_eq!(
                (
                    sb.cells,
                    sb.points,
                    sb.boundary_faces,
                    &sb.cell_kinds,
                    &sb.zones,
                    &sb.patches
                ),
                (
                    sv.cells,
                    sv.points,
                    sv.boundary_faces,
                    &sv.cell_kinds,
                    &sv.zones,
                    &sv.patches
                ),
                "{blocked}"
            );
            for (k, val) in &v.metadata {
                assert_eq!(b.metadata.get(k), Some(val), "{blocked}: {k}");
            }
            let cells = |m: &Mesh| {
                let mut c: Vec<BTreeSet<[u64; 3]>> =
                    (0..m.cell_count()).map(|i| corners(m, i)).collect();
                c.sort();
                c
            };
            assert_eq!(cells(&b), cells(&v), "{blocked}");
            assert!((0..b.cell_count()).all(|c| volume(&b, c) > 0.0));
        }
    }

    /// Invariant: a mesh with no cells is refused with BLOCK003, a block
    /// of no cells with BLOCK002, and an unwritable path with BLOCK001.
    #[test]
    fn refusals_name_their_code() {
        let r = Results::default();
        let empty = Mesh::default();
        assert_eq!(
            write_blocked(&scratch("e.vtkhdf"), &empty, &r, 10)
                .unwrap_err()
                .code,
            "BLOCK003"
        );
        let m = crate::synth::honeycomb(2);
        assert_eq!(
            write_blocked(&scratch("z.vtkhdf"), &m, &r, 0)
                .unwrap_err()
                .code,
            "BLOCK002"
        );
        let nowhere = scratch("no/such/folder/x.vtkhdf");
        assert_eq!(
            write_blocked(&nowhere, &m, &r, 10).unwrap_err().code,
            "BLOCK001"
        );
    }
}
