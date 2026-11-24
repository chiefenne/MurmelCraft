//! The mesh model of MurmelCraft and its readers.
//!
//! A [`Mesh`] holds what the viewer needs from a KoflCraft hand-off
//! (KoflCraft `docs/foehncraft.md`): the points, the volume cells as
//! polyhedra (every cell a list of faces, every face a list of point ids),
//! the boundary faces with their patch, and the names and roles of zones
//! and patches. It is read-only: the viewer never edits a mesh.
//!
//! Readers never panic on a broken file. Every failure is a [`MeshError`]
//! with a registered diagnostic code (`MESH001` …), and counts a file
//! declares are checked against the file size before anything is
//! allocated from them (KoflCraft SW-17).

mod boundary;
pub mod evidence;
pub mod field;
pub mod hdf5;
mod numbers;
pub mod par;
pub mod polymesh;
pub mod profile;
pub mod synth;
pub mod vtkhdf;
mod vtu;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

pub use boundary::NO_CELL;
pub use evidence::{Evidence, EvidencePatch};
pub use field::{Component, Field, Location, Results};
pub use polymesh::{PolyMeshFiles, read_polymesh, read_polymesh_bytes};
pub use vtkhdf::read_vtkhdf;
pub use vtu::{read_vtu, read_vtu_str};

/// A failure to read a mesh, with its diagnostic code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshError {
    /// Registered code, such as `MESH004`.
    pub code: &'static str,
    pub message: String,
}

impl MeshError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for MeshError {}

/// The diagnostic codes this crate reports, with their meaning and fix.
/// The tool registry lists them in `diag.codes`.
pub const CODES: &[(&str, &str, &str)] = &[
    (
        "MESH001",
        "The mesh file cannot be read (missing, unreadable or not a file).",
        "Check the path and the file permissions.",
    ),
    (
        "MESH002",
        "The file is not a VTK XML UnstructuredGrid.",
        "Open a .vtu file such as KoflCraft's foehncraft.vtu.",
    ),
    (
        "MESH003",
        "The XML of the file is malformed or an expected element is missing.",
        "Write the file again; it may be truncated.",
    ),
    (
        "MESH004",
        "A count the file declares cannot fit in the file size.",
        "The file is truncated or its header is wrong; write it again.",
    ),
    (
        "MESH005",
        "A data array has the wrong number of values or a value that is not a number.",
        "Write the file again; it may be truncated or edited by hand.",
    ),
    (
        "MESH006",
        "A point, face or cell index is out of range.",
        "The connectivity is broken; write the file again.",
    ),
    (
        "MESH007",
        "The data encoding is not supported (appended or compressed data).",
        "Write the file with ascii or uncompressed binary (base64) arrays.",
    ),
    (
        "MESH008",
        "The file has a cell type the viewer does not read.",
        "Supported: polyhedron (42), tetra (10), hexahedron (12), wedge (13), pyramid (14), triangle (5), polygon (7), quad (9).",
    ),
    (
        "MESH009",
        "The polyhedron face stream (faces, faceoffsets) is broken.",
        "Write the file again; KoflCraft writes the legacy VTK polyhedron layout.",
    ),
    (
        "MESH010",
        "The mesh has no volume cells and no boundary faces.",
        "Open a volume mesh.",
    ),
    (
        "MESH012",
        "The file has more than one Piece.",
        "Write the mesh as one Piece.",
    ),
    (
        "MESH011",
        "mesh-evidence.json beside the mesh cannot be read; the mesh opens without it.",
        "Write the evidence again with KoflCraft or remove it.",
    ),
    (
        "MESH013",
        "The folder holds no OpenFOAM mesh, or the polyMesh lacks one of points, faces, owner, neighbour and boundary.",
        "Open a case folder, its constant folder or the polyMesh folder itself.",
    ),
    (
        "MESH014",
        "An OpenFOAM file is malformed: no FoamFile header, or a list or dictionary is broken.",
        "Write the mesh again; the file may be truncated or edited by hand.",
    ),
    (
        "MESH016",
        "The HDF5 structure of the file is broken: a signature, address or size does not fit the file.",
        "Write the file again; it may be truncated.",
    ),
    (
        "MESH017",
        "The file uses an HDF5 feature the reader does not support (a chunk index, filter or storage kind).",
        "Write the file with contiguous or chunked datasets and the deflate or shuffle filters, as FoehnCraft and VTK do.",
    ),
    (
        "MESH018",
        "The file is not a VTKHDF file, or its VTKHDF layout is broken (a block, dataset or count is missing or wrong).",
        "Open a .vtkhdf file written by FoehnCraft or VTK (VTKHDF 2.x, UnstructuredGrid or PartitionedDataSetCollection).",
    ),
    (
        "MESH019",
        "Opening the mesh was cancelled.",
        "Open it again when you want it.",
    ),
    (
        "MESH015",
        "The OpenFOAM boundary patches do not cover the boundary faces once each (startFace, nFaces).",
        "Check constant/polyMesh/boundary; renumbering or checkMesh shows the faces of each patch.",
    ),
];

/// The type of a volume cell, classed by its faces (UI spec, Cell type).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CellKind {
    Polyhedron,
    Prism,
    Hexahedron,
    Tetrahedron,
    Pyramid,
}

impl CellKind {
    pub const ALL: [CellKind; 5] = [
        CellKind::Polyhedron,
        CellKind::Prism,
        CellKind::Hexahedron,
        CellKind::Tetrahedron,
        CellKind::Pyramid,
    ];

    pub fn name(self) -> &'static str {
        match self {
            CellKind::Polyhedron => "polyhedron",
            CellKind::Prism => "prism",
            CellKind::Hexahedron => "hexahedron",
            CellKind::Tetrahedron => "tetrahedron",
            CellKind::Pyramid => "pyramid",
        }
    }

    /// 4 triangles: tet; 4 triangles and 1 quad: pyramid; 2 triangles and
    /// 3 quads: prism; 6 quads: hex; anything else: polyhedron.
    pub fn classify(face_sizes: impl IntoIterator<Item = usize>) -> CellKind {
        let (mut faces, mut tris, mut quads) = (0, 0, 0);
        for n in face_sizes {
            faces += 1;
            match n {
                3 => tris += 1,
                4 => quads += 1,
                _ => {}
            }
        }
        match (faces, tris, quads) {
            (4, 4, 0) => CellKind::Tetrahedron,
            (5, 4, 1) => CellKind::Pyramid,
            (5, 2, 3) => CellKind::Prism,
            (6, 0, 6) => CellKind::Hexahedron,
            _ => CellKind::Polyhedron,
        }
    }
}

/// The role of a boundary patch as KoflCraft writes it (`boundary_roles`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PatchRole {
    Wall,
    Inlet,
    Outlet,
    Symmetry,
    Interface,
    Unspecified,
}

impl PatchRole {
    pub const ALL: [PatchRole; 6] = [
        PatchRole::Wall,
        PatchRole::Inlet,
        PatchRole::Outlet,
        PatchRole::Symmetry,
        PatchRole::Interface,
        PatchRole::Unspecified,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PatchRole::Wall => "wall",
            PatchRole::Inlet => "inlet",
            PatchRole::Outlet => "outlet",
            PatchRole::Symmetry => "symmetry",
            PatchRole::Interface => "interface",
            PatchRole::Unspecified => "unspecified",
        }
    }

    pub fn parse(s: &str) -> PatchRole {
        match s.trim() {
            "wall" => PatchRole::Wall,
            "inlet" => PatchRole::Inlet,
            "outlet" => PatchRole::Outlet,
            "symmetry" => PatchRole::Symmetry,
            "interface" => PatchRole::Interface,
            _ => PatchRole::Unspecified,
        }
    }
}

/// A boundary patch.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Patch {
    pub name: String,
    pub role: PatchRole,
}

/// Polygons stored flat: `nodes[start[i]..start[i + 1]]` is polygon `i`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polygons {
    pub start: Vec<u32>,
    pub nodes: Vec<u32>,
}

impl Polygons {
    pub fn new() -> Self {
        Self {
            start: vec![0],
            nodes: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.start.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, i: usize) -> &[u32] {
        &self.nodes[self.start[i] as usize..self.start[i + 1] as usize]
    }

    pub fn push(&mut self, nodes: impl IntoIterator<Item = u32>) {
        self.nodes.extend(nodes);
        self.start.push(self.nodes.len() as u32);
    }

    pub fn iter(&self) -> impl Iterator<Item = &[u32]> {
        (0..self.len()).map(|i| self.get(i))
    }
}

/// A face of a cell: its distinct face (`face:<id>`) with the points as
/// stored, and whether the cell lists them the other way round. Code that
/// needs the cell's own turn (volumes, winding numbers, sections) reads
/// [`CellFace::oriented`] or [`CellFace::at`]; code that needs only the
/// points reads `nodes`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellFace<'a> {
    pub id: u32,
    pub nodes: &'a [u32],
    pub flip: bool,
}

impl<'a> CellFace<'a> {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Point `i` in the cell's own order: the stored order, or for a
    /// flipped face its first point and then the others backwards.
    pub fn at(&self, i: usize) -> u32 {
        if self.flip && i > 0 {
            self.nodes[self.nodes.len() - i]
        } else {
            self.nodes[i]
        }
    }

    /// The points in the cell's own order (outward from the cell in a
    /// well-formed mesh).
    pub fn oriented(self) -> impl ExactSizeIterator<Item = u32> + Clone + 'a {
        (0..self.nodes.len()).map(move |i| self.at(i))
    }
}

/// A read-only volume mesh.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub points: Vec<[f64; 3]>,
    /// The distinct faces of the volume cells (`face:<u>`), each stored
    /// once, as its owner lists it (outward from the owner in a
    /// well-formed mesh); empty when there are no cells. A cell reads its
    /// faces through [`Mesh::cell`].
    pub distinct: Polygons,
    /// Cell `c` has the cell faces `cell_faces[c]..cell_faces[c + 1]`
    /// (indices into `cell_face_id` and `cell_face_flip`).
    pub cell_faces: Vec<u32>,
    pub cell_zone: Vec<u32>,
    pub cell_kind: Vec<CellKind>,
    /// Boundary faces and the patch of each.
    pub boundary: Polygons,
    pub boundary_patch: Vec<u32>,
    /// The volume cell each boundary face belongs to ([`NO_CELL`] when
    /// none matches).
    pub boundary_owner: Vec<u32>,
    /// Distinct faces of the volume cells (interior plus boundary); the
    /// length of `face_owner`.
    pub unique_faces: usize,
    /// The distinct face (`face:<n>`) each cell face is, by cell-face index.
    pub cell_face_id: Vec<u32>,
    /// Whether a cell face lists the points of its distinct face the
    /// other way round (the neighbour's side of the face), by cell-face
    /// index.
    pub cell_face_flip: Vec<bool>,
    /// Owner and neighbour cell of each distinct face ([`NO_CELL`] for no
    /// neighbour: a boundary face). The owner is the first cell that uses it.
    pub face_owner: Vec<u32>,
    pub face_neighbour: Vec<u32>,
    /// The distinct face of each boundary polygon, and the boundary polygon
    /// of each distinct face ([`NO_CELL`] when there is none).
    pub boundary_face_id: Vec<u32>,
    pub face_boundary: Vec<u32>,
    pub zones: Vec<String>,
    pub patches: Vec<Patch>,
    /// Further field-data strings (`koflcraft_version`, digests).
    pub metadata: BTreeMap<String, String>,
    /// True when the boundary was found from the cells (the file had no
    /// boundary polygons).
    pub boundary_extracted: bool,
    /// The files the mesh was read from: the `.vtu`, or the files of an
    /// OpenFOAM `polyMesh` (empty when read from memory).
    pub files: Vec<PathBuf>,
    /// The SHA-256 (hex) of each of `files`, taken by the reader from the
    /// bytes it read (on another thread while it parses), so the file is
    /// not read twice; empty when the reader did not take them.
    pub file_sha256: Vec<String>,
}

/// SHA-256 of `bytes`, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Counts and extent of a mesh, as `mesh.open` reports them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Summary {
    pub points: usize,
    pub cells: usize,
    pub boundary_faces: usize,
    pub cell_kinds: BTreeMap<&'static str, usize>,
    pub zones: Vec<NamedCount>,
    pub patches: Vec<PatchCount>,
    pub bounds: Bounds,
    pub boundary_extracted: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NamedCount {
    pub name: String,
    pub cells: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PatchCount {
    pub name: String,
    pub role: PatchRole,
    pub faces: usize,
}

/// An axis-aligned box.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Bounds {
    pub fn empty() -> Self {
        Self {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        }
    }

    pub fn is_empty(&self) -> bool {
        (0..3).any(|k| self.min[k] > self.max[k])
    }

    pub fn add(&mut self, p: [f64; 3]) {
        for (k, v) in p.into_iter().enumerate() {
            self.min[k] = self.min[k].min(v);
            self.max[k] = self.max[k].max(v);
        }
    }

    pub fn center(&self) -> [f64; 3] {
        if self.is_empty() {
            return [0.0; 3];
        }
        [0, 1, 2].map(|k| 0.5 * (self.min[k] + self.max[k]))
    }

    /// Half the diagonal: the radius of the bounding sphere.
    pub fn radius(&self) -> f64 {
        if self.is_empty() {
            return 0.0;
        }
        let d: f64 = (0..3).map(|k| (self.max[k] - self.min[k]).powi(2)).sum();
        0.5 * d.sqrt()
    }
}

impl Mesh {
    pub fn cell_count(&self) -> usize {
        self.cell_faces.len().saturating_sub(1)
    }

    /// The faces of cell `c`, in the order the cell lists them.
    pub fn cell(&self, c: usize) -> impl Iterator<Item = CellFace<'_>> {
        self.cell_face_range(c).map(|f| {
            let id = self.cell_face_id[f];
            CellFace {
                id,
                nodes: self.distinct.get(id as usize),
                flip: self.cell_face_flip[f],
            }
        })
    }

    /// The faces of every cell as the cell lists them, cell by cell (each
    /// interior face twice): the layout of a VTK polyhedron's `faces`.
    pub fn cell_polygons(&self) -> Polygons {
        let mut p = Polygons::new();
        for c in 0..self.cell_count() {
            for f in self.cell(c) {
                p.push(f.oriented());
            }
        }
        p
    }

    /// The point ids of all faces of cell `c`, each as often as its faces
    /// use it.
    pub fn cell_points(&self, c: usize) -> impl Iterator<Item = u32> + '_ {
        self.cell(c).flat_map(|f| f.nodes.iter().copied())
    }

    /// The cell-face indices of cell `c`.
    pub fn cell_face_range(&self, c: usize) -> std::ops::Range<usize> {
        self.cell_faces[c] as usize..self.cell_faces[c + 1] as usize
    }

    /// The point ids of distinct face `u` (`face:<u>`), as its owner sees
    /// it (outward from the owner), or the boundary polygon when the mesh
    /// has no cells.
    pub fn face_nodes(&self, u: usize) -> &[u32] {
        if u < self.distinct.len() {
            return self.distinct.get(u);
        }
        match self.face_boundary.get(u) {
            Some(&b) if b != NO_CELL => self.boundary.get(b as usize),
            _ => &[],
        }
    }

    /// The cell across distinct face `u` from cell `c` ([`NO_CELL`] on
    /// the boundary).
    pub fn across(&self, u: usize, c: u32) -> u32 {
        let (o, n) = (self.face_owner[u], self.face_neighbour[u]);
        if o == c { n } else { o }
    }

    /// Take the patch roles KoflCraft recorded in its evidence (a patch map
    /// may have set them, BC-04); patches the evidence does not name keep
    /// the role the file gave.
    pub fn apply_evidence_roles(&mut self, evidence: &Evidence) {
        for p in &mut self.patches {
            if let Some(role) = evidence.patch(&p.name).and_then(|e| e.role.as_deref()) {
                p.role = PatchRole::parse(role);
            }
        }
    }

    /// The bytes of the model's arrays (points, faces, cells, topology),
    /// as `murmel bench` reports the mesh model and region estimates
    /// scale it.
    pub fn model_bytes(&self) -> usize {
        self.points.len() * 24
            + (self.distinct.start.len() + self.distinct.nodes.len()) * 4
            + (self.boundary.start.len() + self.boundary.nodes.len()) * 4
            + (self.cell_faces.len() + self.cell_zone.len()) * 4
            + self.cell_kind.len() * std::mem::size_of::<CellKind>()
            + (self.boundary_patch.len() + self.boundary_owner.len()) * 4
            + (self.cell_face_id.len() + self.face_owner.len() + self.face_neighbour.len()) * 4
            + self.cell_face_flip.len()
            + (self.boundary_face_id.len() + self.face_boundary.len()) * 4
    }

    /// The bounds of the points the boundary uses (all points when there
    /// is no boundary).
    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::empty();
        if self.boundary.is_empty() {
            for &p in &self.points {
                b.add(p);
            }
        } else {
            for &n in &self.boundary.nodes {
                b.add(self.points[n as usize]);
            }
        }
        b
    }

    pub fn summary(&self) -> Summary {
        let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
        for k in &self.cell_kind {
            *kinds.entry(k.name()).or_default() += 1;
        }
        let mut zone_cells = vec![0usize; self.zones.len()];
        for &z in &self.cell_zone {
            if let Some(n) = zone_cells.get_mut(z as usize) {
                *n += 1;
            }
        }
        let mut patch_faces = vec![0usize; self.patches.len()];
        for &p in &self.boundary_patch {
            if let Some(n) = patch_faces.get_mut(p as usize) {
                *n += 1;
            }
        }
        Summary {
            points: self.points.len(),
            cells: self.cell_count(),
            boundary_faces: self.boundary.len(),
            cell_kinds: kinds,
            zones: self
                .zones
                .iter()
                .zip(zone_cells)
                .map(|(name, cells)| NamedCount {
                    name: name.clone(),
                    cells,
                })
                .collect(),
            patches: self
                .patches
                .iter()
                .zip(patch_faces)
                .map(|(p, faces)| PatchCount {
                    name: p.name.clone(),
                    role: p.role,
                    faces,
                })
                .collect(),
            bounds: self.bounds(),
            boundary_extracted: self.boundary_extracted,
        }
    }
}

/// Read a mesh: a `.vtu` file, or an OpenFOAM mesh given by its case,
/// `constant` or `polyMesh` folder, a `.foam` file in the case or one of
/// the polyMesh files ([`polymesh::is_openfoam`]).
pub fn read(path: &Path) -> Result<Mesh, MeshError> {
    read_results(path).map(|(m, _)| m)
}

/// Read a mesh and the fields that come with it: a `.vtkhdf` result
/// (FoehnCraft's `fields.vtkhdf`), the data arrays of a `.vtu`, or an
/// OpenFOAM mesh (no fields yet).
pub fn read_results(path: &Path) -> Result<(Mesh, field::Results), MeshError> {
    if path.is_file() && vtkhdf::is_vtkhdf(path) {
        return vtkhdf::read_vtkhdf(path);
    }
    let vtu = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("vtu"));
    if !vtu && polymesh::is_openfoam(path) {
        return read_polymesh(path).map(|m| (m, field::Results::default()));
    }
    let (mut mesh, results) = vtu::read_vtu_results(path)?;
    mesh.files = vec![path.to_path_buf()];
    Ok((mesh, results))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the spec's face-count rules class KoflCraft's prism
    /// layers (polyhedra with prism topology) as prisms. A change that
    /// classed by VTK type instead would show every layer as a polyhedron.
    #[test]
    fn classify_by_faces() {
        assert_eq!(CellKind::classify([3, 3, 3, 3]), CellKind::Tetrahedron);
        assert_eq!(CellKind::classify([4, 3, 3, 3, 3]), CellKind::Pyramid);
        assert_eq!(CellKind::classify([3, 4, 4, 4, 3]), CellKind::Prism);
        assert_eq!(CellKind::classify([4; 6]), CellKind::Hexahedron);
        assert_eq!(
            CellKind::classify([5, 5, 4, 4, 4, 4, 4]),
            CellKind::Polyhedron
        );
        assert_eq!(CellKind::classify([4, 4, 4, 4, 4]), CellKind::Polyhedron);
    }
}
