//! Synthetic polyhedral meshes of any size, for benchmarks and tests of
//! large meshes, and writers of them in the formats the viewer reads.
//!
//! The mesh is the bitruncated cubic honeycomb: the Voronoi diagram of
//! the body-centred cubic lattice, every cell a truncated octahedron with
//! 6 square and 8 hexagonal faces (Conway, Burgiel and Goodman-Strauss,
//! "The Symmetries of Things", 2008, ch. 21). That is what a Voronoi core
//! looks like to the viewer: 14 faces and 72 face corners per cell, each
//! point shared by four cells. `honeycomb(n)` holds `n³ + (n - 1)³` cells
//! in about the unit cube; its outside is left jagged (cells are not
//! clipped), cut into six named patches by the nearest side of the box,
//! and the cells into two zones by `x`.
//!
//! The writers produce KoflCraft's `foehncraft.vtu` layout (polyhedra 42
//! with the boundary as polygons 7 and the cell array `region`), in ascii
//! as KoflCraft writes it or in binary (base64), and an OpenFOAM
//! `polyMesh` in ascii or binary (`faceCompactList`). They write what
//! the readers of this crate read; a test reads every format back.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use base64::Engine as _;

use crate::{CellKind, Mesh, NO_CELL, Patch, PatchRole, Polygons, boundary};

pub mod blocked;

/// The corners of the truncated octahedron about the origin in quarter
/// lattice units: every permutation of `(0, ±1, ±2)`.
fn corners() -> Vec<[i32; 3]> {
    let mut v = Vec::with_capacity(24);
    for zero in 0..3 {
        let (a, b) = ((zero + 1) % 3, (zero + 2) % 3);
        for (one, two) in [(a, b), (b, a)] {
            for s1 in [-1, 1] {
                for s2 in [-1, 1] {
                    let mut p = [0; 3];
                    p[one] = s1;
                    p[two] = 2 * s2;
                    v.push(p);
                }
            }
        }
    }
    v
}

/// The 14 faces of the cell as corner indices, turning counter-clockwise
/// seen from outside, with the offset (quarter units) to the cell across
/// each: squares toward `±4 e_k`, hexagons toward `(±2, ±2, ±2)`.
fn cell_faces() -> Vec<(Vec<usize>, [i32; 3])> {
    let pts = corners();
    let mut normals: Vec<[i32; 3]> = Vec::new();
    for k in 0..3 {
        for s in [-1, 1] {
            let mut n = [0; 3];
            n[k] = s;
            normals.push(n);
        }
    }
    for sx in [-1, 1] {
        for sy in [-1, 1] {
            for sz in [-1, 1] {
                normals.push([sx, sy, sz]);
            }
        }
    }
    normals
        .into_iter()
        .map(|n| {
            let dot = |p: &[i32; 3]| n[0] * p[0] + n[1] * p[1] + n[2] * p[2];
            let square = n.iter().filter(|&&v| v != 0).count() == 1;
            let level = if square { 2 } else { 3 };
            let mut on: Vec<usize> = (0..pts.len()).filter(|&i| dot(&pts[i]) == level).collect();
            // Order by angle about the outward normal.
            let nf = n.map(f64::from);
            let c: [f64; 3] = std::array::from_fn(|k| {
                on.iter().map(|&i| f64::from(pts[i][k])).sum::<f64>() / on.len() as f64
            });
            let u0 = {
                let p = pts[on[0]];
                [0, 1, 2].map(|k| f64::from(p[k]) - c[k])
            };
            let v0 = cross(nf, u0);
            on.sort_by(|&a, &b| {
                let ang = |i: usize| {
                    let d = [0, 1, 2].map(|k| f64::from(pts[i][k]) - c[k]);
                    dot3(d, v0).atan2(dot3(d, u0))
                };
                ang(a).total_cmp(&ang(b))
            });
            let across = if square {
                n.map(|v| 4 * v)
            } else {
                n.map(|v| 2 * v)
            };
            (on, across)
        })
        .collect()
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The cells of `honeycomb(n)`.
pub fn cells_of(n: usize) -> usize {
    n.pow(3) + n.saturating_sub(1).pow(3)
}

/// The smallest `n` whose honeycomb has at least `cells` cells.
pub fn n_for_cells(cells: usize) -> usize {
    let mut n = 2;
    while cells_of(n) < cells {
        n += 1;
    }
    n
}

/// The six patches: the side of the box each boundary face is nearest.
const PATCHES: [(&str, PatchRole); 6] = [
    ("inlet", PatchRole::Inlet),
    ("outlet", PatchRole::Outlet),
    ("front", PatchRole::Symmetry),
    ("back", PatchRole::Symmetry),
    ("bottom", PatchRole::Wall),
    ("top", PatchRole::Wall),
];

/// The honeycomb with `n` corner cells along each axis: the corner cells
/// at `4 (i, j, k)` (quarter units) and the body cells at `4 (i, j, k) +
/// 2` between them, ordered by layer, row and column so neighbours are
/// near in memory as in a real mesh. One quarter unit is `1 / (4 (n -
/// 1))`, so the cell centres span the unit cube.
pub fn honeycomb(n: usize) -> Mesh {
    let n = n.max(2);
    let pts = corners();
    let faces = cell_faces();
    let cells = cells_of(n);
    let mut centres: Vec<[i32; 3]> = Vec::with_capacity(cells);
    for k in 0..n as i32 {
        for j in 0..n as i32 {
            for i in 0..n as i32 {
                centres.push([4 * i, 4 * j, 4 * k]);
                if i + 1 < n as i32 && j + 1 < n as i32 && k + 1 < n as i32 {
                    centres.push([4 * i + 2, 4 * j + 2, 4 * k + 2]);
                }
            }
        }
    }
    let top = 4 * (n as i32 - 1);
    let exists = |c: [i32; 3]| {
        let corner = c.iter().all(|v| v % 4 == 0);
        let body = c.iter().all(|v| v % 4 == 2);
        (corner || body) && c.iter().all(|&v| (0..=top).contains(&v))
    };
    let unit = 1.0 / f64::from(top.max(1));
    let mut ids: HashMap<[i32; 3], u32> = HashMap::with_capacity(cells * 7);
    let mut mesh = Mesh {
        boundary: Polygons::new(),
        cell_faces: vec![0],
        ..Mesh::default()
    };
    let mut polys = Polygons::new();
    polys.start.reserve(cells * 14);
    polys.nodes.reserve(cells * 72);
    let mut bfaces: Vec<(u8, Vec<u32>)> = Vec::new();
    for &c in &centres {
        for (corners, across) in &faces {
            let face: Vec<u32> = corners
                .iter()
                .map(|&v| {
                    let q = [0, 1, 2].map(|k| c[k] + pts[v][k]);
                    let next = mesh.points.len() as u32;
                    let id = *ids.entry(q).or_insert(next);
                    if id == next {
                        mesh.points.push(q.map(|v| f64::from(v) * unit));
                    }
                    id
                })
                .collect();
            let other = [0, 1, 2].map(|k| c[k] + across[k]);
            if !exists(other) {
                // The side of the box nearest the face's centre (in
                // quarter units, times the corner count).
                let m = corners.len() as i32;
                let fc: [i32; 3] =
                    std::array::from_fn(|k| corners.iter().map(|&v| c[k] + pts[v][k]).sum::<i32>());
                let side = (0..6)
                    .min_by_key(|&s| {
                        let k = s / 2;
                        if s % 2 == 0 { fc[k] } else { top * m - fc[k] }
                    })
                    .unwrap_or(0);
                // Axis order of the sides is x, y, z; PATCHES is inlet,
                // outlet (x), front, back (y), bottom, top (z).
                bfaces.push((side as u8, face.clone()));
            }
            polys.push(face);
        }
        mesh.cell_faces.push(polys.len() as u32);
        mesh.cell_zone.push(u32::from(2 * c[0] > top));
        mesh.cell_kind.push(CellKind::Polyhedron);
    }
    bfaces.sort_by_key(|(p, _)| *p);
    for (p, f) in bfaces {
        mesh.boundary.push(f);
        mesh.boundary_patch.push(u32::from(p));
    }
    mesh.zones = vec!["fluid".into(), "solid".into()];
    mesh.patches = PATCHES
        .iter()
        .map(|&(name, role)| Patch {
            name: name.into(),
            role,
        })
        .collect();
    mesh.metadata
        .insert("generator".into(), format!("murmel synth honeycomb n={n}"));
    boundary::link(&mut mesh, polys, None);
    mesh
}

/// How a writer encodes numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Ascii,
    Binary,
}

fn file(path: &Path) -> io::Result<BufWriter<std::fs::File>> {
    Ok(BufWriter::with_capacity(
        1 << 20,
        std::fs::File::create(path)?,
    ))
}

/// Write `values` as one VTU `DataArray` body: ascii, or base64 of a
/// UInt64 byte count and the little-endian values.
fn vtu_array<T: Copy>(
    w: &mut impl Write,
    enc: Encoding,
    values: &[T],
    ascii: impl Fn(&mut String, T),
    bytes: impl Fn(&mut Vec<u8>, T),
) -> io::Result<()> {
    match enc {
        Encoding::Ascii => {
            let mut line = String::with_capacity(4096);
            for (i, &v) in values.iter().enumerate() {
                ascii(&mut line, v);
                line.push(if i % 12 == 11 { '\n' } else { ' ' });
                if line.len() > 3900 {
                    w.write_all(line.as_bytes())?;
                    line.clear();
                }
            }
            line.push('\n');
            w.write_all(line.as_bytes())
        }
        Encoding::Binary => {
            let mut raw = Vec::with_capacity(8 + values.len() * 8);
            raw.extend_from_slice(&[0u8; 8]);
            for &v in values {
                bytes(&mut raw, v);
            }
            let len = (raw.len() - 8) as u64;
            raw[..8].copy_from_slice(&len.to_le_bytes());
            let text = base64::engine::general_purpose::STANDARD.encode(&raw);
            w.write_all(text.as_bytes())?;
            w.write_all(b"\n")
        }
    }
}

fn ints(w: &mut impl Write, enc: Encoding, values: &[i64]) -> io::Result<()> {
    vtu_array(
        w,
        enc,
        values,
        |s, v| {
            let _ = write!(s, "{v}");
        },
        |b, v| b.extend_from_slice(&v.to_le_bytes()),
    )
}

fn strings_array(w: &mut impl Write, name: &str, entries: &[String]) -> io::Result<()> {
    let mut bytes: Vec<u8> = Vec::new();
    for e in entries {
        bytes.extend_from_slice(e.as_bytes());
        bytes.push(0);
    }
    writeln!(
        w,
        "      <DataArray type=\"String\" Name=\"{name}\" NumberOfTuples=\"{}\" format=\"ascii\">",
        entries.len()
    )?;
    let text: Vec<String> = bytes.iter().map(|b| b.to_string()).collect();
    writeln!(w, "{}", text.join(" "))?;
    writeln!(w, "      </DataArray>")
}

/// Write `mesh` as a KoflCraft `foehncraft.vtu`: the volume cells as
/// polyhedra (42), then the boundary polygons (7), with `region` the
/// zone of a cell and the patch of a polygon, and the names and roles as
/// field data.
pub fn write_vtu(mesh: &Mesh, path: &Path, enc: Encoding) -> io::Result<()> {
    let mut w = file(path)?;
    let cells = mesh.cell_count();
    let total = cells + mesh.boundary.len();
    let format = match enc {
        Encoding::Ascii => "ascii",
        Encoding::Binary => "binary",
    };
    writeln!(w, "<?xml version=\"1.0\"?>")?;
    writeln!(
        w,
        "<VTKFile type=\"UnstructuredGrid\" version=\"1.0\" byte_order=\"LittleEndian\" header_type=\"UInt64\">"
    )?;
    writeln!(w, "  <UnstructuredGrid>\n    <FieldData>")?;
    let ids = |names: Vec<String>| -> Vec<String> {
        names
            .into_iter()
            .enumerate()
            .map(|(i, n)| format!("{i}={n}"))
            .collect()
    };
    strings_array(&mut w, "zone_names", &ids(mesh.zones.clone()))?;
    strings_array(
        &mut w,
        "boundary_names",
        &ids(mesh.patches.iter().map(|p| p.name.clone()).collect()),
    )?;
    strings_array(
        &mut w,
        "boundary_roles",
        &ids(mesh
            .patches
            .iter()
            .map(|p| p.role.name().to_string())
            .collect()),
    )?;
    writeln!(w, "    </FieldData>")?;
    writeln!(
        w,
        "    <Piece NumberOfPoints=\"{}\" NumberOfCells=\"{total}\">",
        mesh.points.len()
    )?;
    writeln!(
        w,
        "      <Points>\n        <DataArray type=\"Float64\" NumberOfComponents=\"3\" format=\"{format}\">"
    )?;
    let xyz: Vec<f64> = mesh.points.iter().flatten().copied().collect();
    vtu_array(
        &mut w,
        enc,
        &xyz,
        |s, v| {
            let _ = write!(s, "{v}");
        },
        |b, v| b.extend_from_slice(&v.to_le_bytes()),
    )?;
    writeln!(w, "        </DataArray>\n      </Points>\n      <Cells>")?;
    // Connectivity: the distinct points of each cell, then each polygon.
    let mut conn: Vec<i64> = Vec::new();
    let mut offsets: Vec<i64> = Vec::with_capacity(total);
    let mut stream: Vec<i64> = Vec::new();
    let mut face_offsets: Vec<i64> = Vec::with_capacity(total);
    let mut mark: Vec<u32> = vec![NO_CELL; mesh.points.len()];
    for c in 0..cells {
        stream.push((mesh.cell_faces[c + 1] - mesh.cell_faces[c]) as i64);
        for f in mesh.cell(c) {
            stream.push(f.len() as i64);
            for p in f.oriented() {
                stream.push(i64::from(p));
                if mark[p as usize] != c as u32 {
                    mark[p as usize] = c as u32;
                    conn.push(i64::from(p));
                }
            }
        }
        offsets.push(conn.len() as i64);
        face_offsets.push(stream.len() as i64);
    }
    for f in mesh.boundary.iter() {
        conn.extend(f.iter().map(|&p| i64::from(p)));
        offsets.push(conn.len() as i64);
        face_offsets.push(-1);
    }
    let types: Vec<i64> = (0..total).map(|c| if c < cells { 42 } else { 7 }).collect();
    let region: Vec<i64> = mesh
        .cell_zone
        .iter()
        .chain(&mesh.boundary_patch)
        .map(|&v| i64::from(v))
        .collect();
    for (name, ty, values) in [
        ("connectivity", "Int64", &conn),
        ("offsets", "Int64", &offsets),
        ("types", "UInt8", &types),
        ("faces", "Int64", &stream),
        ("faceoffsets", "Int64", &face_offsets),
    ] {
        writeln!(
            w,
            "        <DataArray type=\"{ty}\" Name=\"{name}\" format=\"{format}\">"
        )?;
        if ty == "UInt8" {
            vtu_array(
                &mut w,
                enc,
                values,
                |s, v| {
                    let _ = write!(s, "{v}");
                },
                |b, v| b.push(v as u8),
            )?;
        } else {
            ints(&mut w, enc, values)?;
        }
        writeln!(w, "        </DataArray>")?;
    }
    writeln!(
        w,
        "      </Cells>\n      <CellData Scalars=\"region\">\n        <DataArray type=\"Int32\" Name=\"region\" format=\"{format}\">"
    )?;
    vtu_array(
        &mut w,
        enc,
        &region,
        |s, v| {
            let _ = write!(s, "{v}");
        },
        |b, v| b.extend_from_slice(&(v as i32).to_le_bytes()),
    )?;
    writeln!(
        w,
        "        </DataArray>\n      </CellData>\n    </Piece>\n  </UnstructuredGrid>\n</VTKFile>"
    )?;
    w.flush()
}

fn foam_header(w: &mut impl Write, enc: Encoding, class: &str, object: &str) -> io::Result<()> {
    let format = match enc {
        Encoding::Ascii => "ascii",
        Encoding::Binary => "binary",
    };
    writeln!(
        w,
        "FoamFile\n{{\n    version     2.0;\n    format      {format};\n    class       {class};\n    arch        \"LSB;label=32;scalar=64\";\n    location    \"constant/polyMesh\";\n    object      {object};\n}}\n"
    )
}

fn foam_labels(w: &mut impl Write, enc: Encoding, values: &[u32]) -> io::Result<()> {
    writeln!(w, "{}", values.len())?;
    match enc {
        Encoding::Ascii => {
            writeln!(w, "(")?;
            let mut s = String::with_capacity(1 << 16);
            for &v in values {
                let _ = writeln!(s, "{v}");
                if s.len() > (1 << 16) - 16 {
                    w.write_all(s.as_bytes())?;
                    s.clear();
                }
            }
            w.write_all(s.as_bytes())?;
            writeln!(w, ")")
        }
        Encoding::Binary => {
            w.write_all(b"(")?;
            for &v in values {
                w.write_all(&v.to_le_bytes())?;
            }
            writeln!(w, ")")
        }
    }
}

/// Write `mesh` as an OpenFOAM `polyMesh` in `dir` (created): the
/// interior faces in upper-triangular order (by owner, then neighbour),
/// then the boundary faces patch by patch, every face turned outward
/// from its owner; the zones as `cellZones`.
pub fn write_polymesh(mesh: &Mesh, dir: &Path, enc: Encoding) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let n = mesh.unique_faces;
    let mut interior: Vec<u32> = (0..n as u32)
        .filter(|&u| mesh.face_neighbour[u as usize] != NO_CELL)
        .collect();
    interior.sort_by_key(|&u| (mesh.face_owner[u as usize], mesh.face_neighbour[u as usize]));
    let mut order = interior.clone();
    let mut patch_sizes = vec![0usize; mesh.patches.len()];
    let mut by_patch: Vec<(u32, u32)> = Vec::new();
    for (b, &u) in mesh.boundary_face_id.iter().enumerate() {
        if u != NO_CELL {
            let p = mesh.boundary_patch[b];
            patch_sizes[p as usize] += 1;
            by_patch.push((p, u));
        }
    }
    by_patch.sort_by_key(|&(p, _)| p);
    order.extend(by_patch.iter().map(|&(_, u)| u));
    if order.len() != n {
        return Err(io::Error::other(format!(
            "{} of {n} faces are interior or on a patch",
            order.len()
        )));
    }

    let mut w = file(&dir.join("points"))?;
    foam_header(&mut w, enc, "vectorField", "points")?;
    writeln!(w, "{}", mesh.points.len())?;
    match enc {
        Encoding::Ascii => {
            writeln!(w, "(")?;
            for p in &mesh.points {
                writeln!(w, "({} {} {})", p[0], p[1], p[2])?;
            }
        }
        Encoding::Binary => {
            w.write_all(b"(")?;
            for p in &mesh.points {
                for v in p {
                    w.write_all(&v.to_le_bytes())?;
                }
            }
        }
    }
    writeln!(w, ")")?;
    w.flush()?;

    let mut w = file(&dir.join("faces"))?;
    let class = match enc {
        Encoding::Ascii => "faceList",
        Encoding::Binary => "faceCompactList",
    };
    foam_header(&mut w, enc, class, "faces")?;
    match enc {
        Encoding::Ascii => {
            writeln!(w, "{n}\n(")?;
            let mut s = String::with_capacity(1 << 16);
            for &u in &order {
                let f = mesh.face_nodes(u as usize);
                let _ = write!(s, "{}(", f.len());
                for (i, p) in f.iter().enumerate() {
                    let _ = write!(s, "{}{p}", if i > 0 { " " } else { "" });
                }
                s.push_str(")\n");
                if s.len() > (1 << 16) - 256 {
                    w.write_all(s.as_bytes())?;
                    s.clear();
                }
            }
            w.write_all(s.as_bytes())?;
            writeln!(w, ")")?;
        }
        Encoding::Binary => {
            let mut offsets = Vec::with_capacity(n + 1);
            let mut nodes = Vec::new();
            offsets.push(0u32);
            for &u in &order {
                nodes.extend_from_slice(mesh.face_nodes(u as usize));
                offsets.push(nodes.len() as u32);
            }
            foam_labels(&mut w, enc, &offsets)?;
            foam_labels(&mut w, enc, &nodes)?;
        }
    }
    w.flush()?;

    let owner: Vec<u32> = order.iter().map(|&u| mesh.face_owner[u as usize]).collect();
    let neighbour: Vec<u32> = interior
        .iter()
        .map(|&u| mesh.face_neighbour[u as usize])
        .collect();
    for (name, values) in [("owner", &owner), ("neighbour", &neighbour)] {
        let mut w = file(&dir.join(name))?;
        foam_header(&mut w, enc, "labelList", name)?;
        foam_labels(&mut w, enc, values)?;
        w.flush()?;
    }

    let mut w = file(&dir.join("boundary"))?;
    foam_header(&mut w, Encoding::Ascii, "polyBoundaryMesh", "boundary")?;
    writeln!(w, "{}\n(", mesh.patches.len())?;
    let mut start = interior.len();
    for (p, patch) in mesh.patches.iter().enumerate() {
        let kind = match patch.role {
            PatchRole::Wall => "wall",
            PatchRole::Symmetry => "symmetry",
            _ => "patch",
        };
        writeln!(
            w,
            "    {}\n    {{\n        type            {kind};\n        nFaces          {};\n        startFace       {start};\n    }}",
            patch.name, patch_sizes[p]
        )?;
        start += patch_sizes[p];
    }
    writeln!(w, ")")?;
    w.flush()?;

    let mut w = file(&dir.join("cellZones"))?;
    foam_header(&mut w, enc, "regIOobject", "cellZones")?;
    writeln!(w, "{}\n(", mesh.zones.len())?;
    for (z, name) in mesh.zones.iter().enumerate() {
        let cells: Vec<u32> = (0..mesh.cell_count() as u32)
            .filter(|&c| mesh.cell_zone[c as usize] == z as u32)
            .collect();
        writeln!(w, "{name}\n{{\n    type cellZone;\ncellLabels List<label> ")?;
        foam_labels(&mut w, enc, &cells)?;
        writeln!(w, ";\n}}")?;
    }
    writeln!(w, ")")?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the reference cell is a closed truncated octahedron
    /// turned outward: 24 corners, 6 squares and 8 hexagons, every edge
    /// used once in each direction, and a positive volume of 32 quarter
    /// units cubed (half the BCC cube of 64). A wrong turn or a missing
    /// corner breaks closure or the sign, which every other test of the
    /// generator relies on.
    #[test]
    fn reference_cell_is_a_closed_truncated_octahedron() {
        let pts = corners();
        let faces = cell_faces();
        assert_eq!(pts.len(), 24);
        let sizes: Vec<usize> = faces.iter().map(|(f, _)| f.len()).collect();
        assert_eq!(sizes.iter().filter(|&&s| s == 4).count(), 6);
        assert_eq!(sizes.iter().filter(|&&s| s == 6).count(), 8);
        let mut edges: HashMap<(usize, usize), i32> = HashMap::new();
        let mut vol = 0.0;
        for (f, _) in &faces {
            for i in 0..f.len() {
                let (a, b) = (f[i], f[(i + 1) % f.len()]);
                *edges.entry((a.min(b), a.max(b))).or_default() += if a < b { 1 } else { -1 };
            }
            let p = |i: usize| pts[f[i]].map(f64::from);
            for i in 1..f.len() - 1 {
                vol += dot3(p(0), cross(p(i), p(i + 1))) / 6.0;
            }
        }
        assert_eq!(edges.len(), 36);
        assert!(edges.values().all(|&v| v == 0));
        assert!((vol - 32.0).abs() < 1e-9, "{vol}");
    }

    /// Invariant: `honeycomb(n)` has `n³ + (n-1)³` cells of 14 faces,
    /// every interior face shared by two cells, the outside closed and
    /// all of it on the six patches (no boundary polygon left without its
    /// cell face). A neighbour test that missed a cell would leave faces
    /// used once that are on no patch.
    #[test]
    fn honeycomb_topology() {
        let m = honeycomb(4);
        assert_eq!(m.cell_count(), 64 + 27);
        assert!(m.cell_face_id.len() == 14 * m.cell_count());
        let boundary = m.face_neighbour.iter().filter(|&&n| n == NO_CELL).count();
        assert_eq!(boundary, m.boundary.len());
        assert!(m.boundary_face_id.iter().all(|&u| u != NO_CELL));
        assert_eq!(
            2 * (m.unique_faces - boundary) + boundary,
            m.cell_face_id.len()
        );
        assert_eq!(m.distinct.len(), m.unique_faces);
        let flipped = m.cell_face_flip.iter().filter(|&&f| f).count();
        assert_eq!(
            flipped,
            m.unique_faces - boundary,
            "one side of each interior face"
        );
        assert_eq!(m.patches.len(), 6);
        let s = m.summary();
        assert!(s.patches.iter().all(|p| p.faces > 0), "{:?}", s.patches);
        assert!(s.zones.iter().all(|z| z.cells > 0));
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("murmel-synth-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn same(a: &Mesh, b: &Mesh) {
        assert_eq!(a.points.len(), b.points.len());
        assert_eq!(a.cell_count(), b.cell_count());
        assert_eq!(a.unique_faces, b.unique_faces);
        assert_eq!(a.boundary.len(), b.boundary.len());
        assert_eq!(a.zones, b.zones);
        assert_eq!(
            a.patches.iter().map(|p| &p.name).collect::<Vec<_>>(),
            b.patches.iter().map(|p| &p.name).collect::<Vec<_>>()
        );
        assert_eq!(a.summary().patches, b.summary().patches);
        assert_eq!(a.summary().zones, b.summary().zones);
    }

    /// Invariant: every writer produces a file the readers of this crate
    /// read back to the same counts, names, zones and patch sizes. A
    /// writer that drifts from the reader's format would make the
    /// benchmark time a failure instead of a read.
    #[test]
    fn written_meshes_read_back() {
        let m = honeycomb(3);
        let d = temp("formats");
        for enc in [Encoding::Ascii, Encoding::Binary] {
            let p = d.join(format!("{enc:?}.vtu"));
            write_vtu(&m, &p, enc).unwrap();
            same(&m, &crate::read(&p).unwrap());
            let dir = d.join(format!("{enc:?}/constant/polyMesh"));
            write_polymesh(&m, &dir, enc).unwrap();
            let r = crate::read(&dir).unwrap();
            same(&m, &r);
            assert_eq!(r.cell_zone, m.cell_zone);
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
