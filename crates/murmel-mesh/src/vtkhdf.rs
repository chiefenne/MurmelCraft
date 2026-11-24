//! Reader for VTKHDF files (`.vtkhdf`): FoehnCraft's `fields.vtkhdf`
//! and VTK's own, written from the VTK documentation ("VTK File Formats",
//! section "VTKHDF File Format", versions 2.x) and FoehnCraft's
//! `docs/interfaces.md` ("Files of a run").
//!
//! Read: an `UnstructuredGrid` (the whole file, or the blocks of a
//! `PartitionedDataSetCollection` or `MultiBlockDataSet`), with any number
//! of partitions, the linear cells tetra (10), hexahedron (12), wedge (13)
//! and pyramid (14), polyhedra (42, the 2.4 layout of `FaceConnectivity`,
//! `FaceOffsets`, `PolyhedronToFaces` and `PolyhedronOffsets`) and the
//! polygons 5, 7 and 9 as boundary faces; `PolyData` blocks as boundary
//! patches named after their block (their points are matched to the
//! volume's points by their exact coordinates); `CellData` and `PointData`
//! as fields, and the cell data of the patches as boundary fields. The
//! root attributes of a FoehnCraft run give the units, the time, the step
//! or iteration. Time-dependent files (`Steps`) are not read yet.
//!
//! A blocked file (`docs/blocked-vtkhdf.md`, written by `murmel blocks
//! make`, KoflCraft and FoehnCraft) reads whole here: ghost cells
//! (`vtkGhostType` with the duplicate bit) are left out with their field
//! rows, points with the same `GlobalPointIds` are one point, the patches
//! join the cells by those ids, and `zone_names`, the patches' `role` and
//! the strings of `/VTKHDF` are kept. Reading only some of the blocks
//! comes with the large-mesh overview.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::field::{Field, Location, Results, group_vectors, parse_units};
use crate::hdf5::{H5, Object, Target, Value};
use crate::vtu::{
    HEX_FACES, HEXAHEDRON, POLYGON, POLYHEDRON, PYRAMID, PYRAMID_FACES, QUAD, TETRA, TETRA_FACES,
    TRIANGLE, WEDGE, WEDGE_FACES, push_cell,
};
use crate::{CellKind, Mesh, MeshError, Patch, Polygons, boundary};

fn bad(message: impl Into<String>) -> MeshError {
    MeshError::new("MESH018", message)
}

/// Whether `path` names an HDF5 file: by its extension (`.vtkhdf`,
/// `.hdf`, `.hdf5`, `.h5`) or by the signature at its start.
pub fn is_vtkhdf(path: &Path) -> bool {
    if path.extension().is_some_and(|e| {
        ["vtkhdf", "hdf", "hdf5", "h5"]
            .iter()
            .any(|x| e.eq_ignore_ascii_case(x))
    }) {
        return true;
    }
    use std::io::Read as _;
    let mut head = [0u8; 8];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok_and(|_| &head == b"\x89HDF\r\n\x1a\n")
}

/// Read the mesh and the fields of a VTKHDF file.
pub fn read_vtkhdf(path: &Path) -> Result<(Mesh, Results), MeshError> {
    read_vtkhdf_with(path, true)
}

/// As [`read_vtkhdf`]; without `hash` no SHA-256 is taken
/// (`Mesh::file_sha256` stays empty).
pub(crate) fn read_vtkhdf_with(path: &Path, hash: bool) -> Result<(Mesh, Results), MeshError> {
    if !path.is_file() {
        return Err(MeshError::new(
            "MESH001",
            format!("{} is not a readable file", path.display()),
        ));
    }
    let io = crate::profile::span("read.file");
    let h = H5::open(path)?;
    drop(io);
    crate::profile::check()?;
    let _t = crate::profile::span("read.vtkhdf");
    // Hash the file on another thread while it is read.
    let (read, digest) = std::thread::scope(|s| {
        let digest = hash.then(|| s.spawn(|| crate::sha256_hex(h.bytes())));
        let read = read_h5(&h);
        (read, digest.map(|d| d.join().unwrap_or_default()))
    });
    let (mut mesh, results) = read?;
    mesh.files = vec![path.to_path_buf()];
    mesh.file_sha256 = digest.into_iter().collect();
    Ok((mesh, results))
}

/// Read the mesh and fields of an open HDF5 file in the VTKHDF layout.
pub fn read_h5(h: &H5) -> Result<(Mesh, Results), MeshError> {
    let vtk = h
        .path("/VTKHDF")
        .map_err(|_| bad("the file has no /VTKHDF group: it is HDF5 but not VTKHDF"))?;
    let ty = type_of(h, &vtk)?;
    let mut volumes: Vec<(String, Object)> = Vec::new();
    let mut patches: Vec<(String, Object)> = Vec::new();
    match ty.as_str() {
        "UnstructuredGrid" => volumes.push(("cells".into(), vtk.clone())),
        "PartitionedDataSetCollection" | "MultiBlockDataSet" => {
            let mut blocks: Vec<(i64, usize, String, Object)> = Vec::new();
            for (k, link) in h.links(&vtk)?.into_iter().enumerate() {
                if link.name == "Assembly" {
                    continue;
                }
                let Target::Hard(addr) = link.target else {
                    continue;
                };
                let obj = h.object(addr)?;
                if !h.is_group(&obj) {
                    continue;
                }
                let index = h
                    .attr(&obj, "Index")?
                    .and_then(|v| v.number())
                    .map_or(i64::MAX, |v| v as i64);
                blocks.push((index, k, link.name, obj));
            }
            blocks.sort_by_key(|b| (b.0, b.1));
            for (_, _, name, obj) in blocks {
                match type_of(h, &obj)?.as_str() {
                    "UnstructuredGrid" => volumes.push((name, obj)),
                    "PolyData" => patches.push((name, obj)),
                    _ => {}
                }
            }
        }
        t => {
            return Err(MeshError::new(
                "MESH017",
                format!(
                    "VTKHDF type {t} (supported: UnstructuredGrid, PartitionedDataSetCollection)"
                ),
            ));
        }
    }
    if volumes.is_empty() && patches.is_empty() {
        return Err(bad("the file has no UnstructuredGrid or PolyData block"));
    }

    let mut b = Builder::default();
    let mut cell_fields: Vec<Vec<Field>> = Vec::new();
    let mut point_fields: Vec<Vec<Field>> = Vec::new();
    let multi = volumes.len() > 1;
    let mut roles: BTreeMap<usize, String> = BTreeMap::new();
    for (zone, (name, obj)) in volumes.iter().enumerate() {
        let read = b.grid(h, obj, multi.then_some(zone as u32))?;
        if multi {
            b.zone_names.insert(zone as u32, name.clone());
        } else {
            if let Some(z) = read.zones {
                b.zone_values = Some(z);
            }
            // zone names and patch roles as field-data strings (§ 8)
            if let Some(fd) = h.child(obj, "FieldData")? {
                for (key, table) in [("zone_names", 0), ("boundary_roles", 1)] {
                    let Some(o) = h.child(&fd, key)? else {
                        continue;
                    };
                    let ds = h.dataset(&o)?;
                    let Ok(entries) = h.read_strings(&ds) else {
                        continue;
                    };
                    for (id, text) in crate::vtu::id_table(&entries) {
                        if table == 0 {
                            b.zone_names.insert(id as u32, text);
                        } else {
                            roles.insert(id, text);
                        }
                    }
                }
            }
        }
        cell_fields.push(data_fields(
            h,
            obj,
            "CellData",
            Location::Cell,
            (read.cell_rows_total, read.cell_rows.as_deref()),
            &["zone", "vtkGhostType", "GlobalCellIds", "OriginalCellIds"],
        )?);
        point_fields.push(data_fields(
            h,
            obj,
            "PointData",
            Location::Point,
            (read.point_rows_total, read.point_rows.as_deref()),
            &["vtkGhostType", "GlobalPointIds"],
        )?);
    }
    let mut boundary_fields: Vec<(usize, Vec<Field>)> = Vec::new();
    for (name, obj) in &patches {
        let p = b.mesh.patches.len() as u32;
        let before = b.mesh.boundary.len();
        b.poly_data(h, obj, p)?;
        let role = match attr_text(h, obj, "role").or_else(|| roles.get(&(p as usize)).cloned()) {
            Some(r) => crate::PatchRole::parse(&r),
            None => crate::polymesh::patch_role(name, ""),
        };
        b.mesh.patches.push(Patch {
            name: name.clone(),
            role,
        });
        let n = b.mesh.boundary.len() - before;
        boundary_fields.push((
            before,
            data_fields(
                h,
                obj,
                "CellData",
                Location::Boundary,
                (n, None),
                &["boundary", "OwnerCell"],
            )?,
        ));
    }
    let mut mesh = b.finish()?;

    // Fields: the cell and point data of every volume block joined block
    // after block (a field missing in one block is NaN there), the cell
    // data of the patches as boundary fields.
    let mut fields = join(cell_fields, mesh.cell_count());
    fields.extend(join(point_fields, mesh.points.len()));
    fields.extend(join_boundary(boundary_fields, mesh.boundary.len()));
    let root = h.root()?;
    let mut attrs = BTreeMap::new();
    for (k, v) in h.attrs(&root)? {
        if let Some(t) = v.text() {
            attrs.insert(k, t);
        }
    }
    let units = attrs
        .get("units")
        .map(|u| parse_units(u))
        .unwrap_or_default();
    for f in &mut fields {
        f.unit = units.get(&f.name).cloned();
    }
    let num = |k: &str| attrs.get(k).and_then(|v| v.parse::<f64>().ok());
    let results = Results {
        fields: group_vectors(fields),
        time: num("time"),
        step: num("step").map(|v| v as u64),
        iteration: num("iteration").map(|v| v as u64),
        attrs: attrs.clone(),
    };
    for (k, v) in &attrs {
        if k == "case" || k.starts_with("foehncraft_") || k.starts_with("koflcraft_") {
            mesh.metadata.insert(k.clone(), v.clone());
        }
    }
    // the strings of /VTKHDF: the block layout, digests and versions
    for (k, v) in h.attrs(&vtk)? {
        if k == "Type" || k == "Version" {
            continue;
        }
        if let Some(t) = v.text() {
            mesh.metadata.insert(k, t);
        }
    }
    // Front ends name the format from this key.
    mesh.metadata.insert("vtkhdf".into(), "2".into());
    Ok((mesh, results))
}

fn type_of(h: &H5, obj: &Object) -> Result<String, MeshError> {
    Ok(h.attr(obj, "Type")?
        .and_then(|v| v.text())
        .unwrap_or_default())
}

/// A dataset of group `obj`, read as integers or floats.
fn ints(h: &H5, obj: &Object, name: &str) -> Result<Vec<i64>, MeshError> {
    let o = h
        .child(obj, name)?
        .ok_or_else(|| bad(format!("no {name} dataset")))?;
    h.read_i64(&h.dataset(&o)?)
}

fn floats(h: &H5, obj: &Object, name: &str) -> Result<(Vec<f64>, Vec<u64>), MeshError> {
    let o = h
        .child(obj, name)?
        .ok_or_else(|| bad(format!("no {name} dataset")))?;
    let ds = h.dataset(&o)?;
    Ok((h.read_f64(&ds)?, ds.dims))
}

fn count(v: i64, what: &str) -> Result<usize, MeshError> {
    usize::try_from(v).map_err(|_| bad(format!("{what} {v} is negative")))
}

/// The numeric datasets of `obj/<group>` with `rows.0` rows, as fields,
/// keeping only the rows `rows.1` names when it is given (the owned cells
/// and the distinct points of a file with ghosts); `skip` names datasets
/// the mesh itself uses.
fn data_fields(
    h: &H5,
    obj: &Object,
    group: &str,
    location: Location,
    rows: (usize, Option<&[u32]>),
    skip: &[&str],
) -> Result<Vec<Field>, MeshError> {
    let (n, keep) = rows;
    let Some(g) = h.child(obj, group)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for link in h.links(&g)? {
        if skip.contains(&link.name.as_str()) {
            continue;
        }
        let Some(o) = h.child(&g, &link.name)? else {
            continue;
        };
        if h.is_group(&o) {
            continue;
        }
        let ds = h.dataset(&o)?;
        if !matches!(
            ds.dtype,
            crate::hdf5::Dtype::Float { .. } | crate::hdf5::Dtype::Int { .. }
        ) {
            continue;
        }
        let comps = match ds.dims.as_slice() {
            [rows] if *rows as usize == n => 1,
            [rows, k] if *rows as usize == n && (1..=9).contains(k) => *k as usize,
            _ => {
                return Err(MeshError::new(
                    "MESH005",
                    format!(
                        "{group}/{} has shape {:?} for {n} {}s",
                        link.name,
                        ds.dims,
                        location.name()
                    ),
                ));
            }
        };
        let mut values = h.read_f64(&ds)?;
        if let Some(keep) = keep {
            values = keep
                .iter()
                .flat_map(|&r| {
                    let at = r as usize * comps;
                    values[at..at + comps].iter().copied()
                })
                .collect();
        }
        if comps == 1 || comps == 3 {
            out.push(Field {
                name: link.name.clone(),
                unit: None,
                location,
                components: comps,
                values,
            });
        } else {
            // other tuples (tensors) as scalars name_0, name_1, …
            for k in 0..comps {
                out.push(Field {
                    name: format!("{}_{k}", link.name),
                    unit: None,
                    location,
                    components: 1,
                    values: values.iter().skip(k).step_by(comps).copied().collect(),
                });
            }
        }
    }
    Ok(out)
}

/// Fields of several blocks joined block after block; `blocks[i]` has
/// the fields of block i. A field missing in a block is NaN there.
fn join(blocks: Vec<Vec<Field>>, total: usize) -> Vec<Field> {
    if blocks.len() == 1 {
        return blocks.into_iter().next().unwrap_or_default();
    }
    let mut order: Vec<(String, usize)> = Vec::new();
    for f in blocks.iter().flatten() {
        if !order.iter().any(|(n, _)| *n == f.name) {
            order.push((f.name.clone(), f.components));
        }
    }
    let sizes: Vec<usize> = blocks
        .iter()
        .map(|b| b.first().map_or(0, Field::len))
        .collect();
    let _ = total;
    order
        .into_iter()
        .map(|(name, comps)| {
            let mut values = Vec::new();
            let mut location = Location::Cell;
            for (b, size) in blocks.iter().zip(&sizes) {
                match b.iter().find(|f| f.name == name && f.components == comps) {
                    Some(f) => {
                        location = f.location;
                        values.extend_from_slice(&f.values);
                    }
                    None => values.extend(std::iter::repeat_n(f64::NAN, size * comps)),
                }
            }
            Field {
                name,
                unit: None,
                location,
                components: comps,
                values,
            }
        })
        .collect()
}

/// Boundary fields of the patches (each with its first face) as fields
/// over all boundary faces; NaN on patches that lack one.
fn join_boundary(patches: Vec<(usize, Vec<Field>)>, total: usize) -> Vec<Field> {
    let mut names: Vec<(String, usize)> = Vec::new();
    for (_, fs) in &patches {
        for f in fs {
            if !names.iter().any(|(n, _)| *n == f.name) {
                names.push((f.name.clone(), f.components));
            }
        }
    }
    names
        .into_iter()
        .map(|(name, comps)| {
            let mut values = vec![f64::NAN; total * comps];
            for (start, fs) in &patches {
                if let Some(f) = fs.iter().find(|f| f.name == name && f.components == comps) {
                    let at = start * comps;
                    let end = (at + f.values.len()).min(values.len());
                    values[at..end].copy_from_slice(&f.values[..end - at]);
                }
            }
            Field {
                name,
                unit: None,
                location: Location::Boundary,
                components: comps,
                values,
            }
        })
        .collect()
}

#[derive(Default)]
struct Builder {
    mesh: Mesh,
    /// The faces of the volume cells as read (`Polygons::new()` once the
    /// first grid starts).
    polys: Polygons,
    zone_max: Option<u32>,
    /// The `zone` cell data of a single volume block.
    zone_values: Option<Vec<u32>>,
    zone_names: BTreeMap<u32, String>,
    /// Volume points by their exact coordinates, for the patches.
    lookup: Option<HashMap<[u64; 3], u32>>,
    /// Volume points by their `GlobalPointIds`, when the file has them.
    gids: Option<HashMap<i64, u32>>,
}

/// What [`Builder::grid`] read besides the mesh: the `zone` cell data, and
/// which rows of the cell and point datasets the mesh took (`None`: all,
/// in order).
struct GridRead {
    zones: Option<Vec<u32>>,
    cell_rows_total: usize,
    cell_rows: Option<Vec<u32>>,
    point_rows_total: usize,
    point_rows: Option<Vec<u32>>,
}

/// The integer dataset `name` of subgroup `group`, when there is one.
fn optional_ints(
    h: &H5,
    obj: &Object,
    group: &str,
    name: &str,
) -> Result<Option<Vec<i64>>, MeshError> {
    let Some(g) = h.child(obj, group)? else {
        return Ok(None);
    };
    match h.child(&g, name)? {
        Some(o) => Ok(Some(h.read_i64(&h.dataset(&o)?)?)),
        None => Ok(None),
    }
}

impl Builder {
    /// Read an UnstructuredGrid block (all its partitions). Ghost cells
    /// (`vtkGhostType` with the duplicate bit) are left out, and points
    /// with the same `GlobalPointIds` are one point, so the partitions of
    /// a blocked file join into one mesh.
    fn grid(&mut self, h: &H5, g: &Object, zone: Option<u32>) -> Result<GridRead, MeshError> {
        let np = ints(h, g, "NumberOfPoints")?;
        let nc = ints(h, g, "NumberOfCells")?;
        let ncon = ints(h, g, "NumberOfConnectivityIds")?;
        let parts = np.len();
        if nc.len() != parts || ncon.len() != parts {
            return Err(bad(
                "NumberOfPoints, NumberOfCells and NumberOfConnectivityIds differ in length",
            ));
        }
        let (xyz, dims) = floats(h, g, "Points")?;
        let total_points: usize = np
            .iter()
            .map(|&n| count(n, "points"))
            .sum::<Result<_, _>>()?;
        if dims.last() != Some(&3) || xyz.len() != 3 * total_points {
            return Err(MeshError::new(
                "MESH005",
                format!("Points has {} values for {total_points} points", xyz.len()),
            ));
        }
        let types = ints(h, g, "Types")?;
        let offsets = ints(h, g, "Offsets")?;
        let conn = ints(h, g, "Connectivity")?;
        let poly = h.child(g, "PolyhedronOffsets")?.is_some();
        let (p_off, p_faces, f_off, f_conn, n_faces, n_p2f, n_fconn) = if poly {
            (
                ints(h, g, "PolyhedronOffsets")?,
                ints(h, g, "PolyhedronToFaces")?,
                ints(h, g, "FaceOffsets")?,
                ints(h, g, "FaceConnectivity")?,
                ints(h, g, "NumberOfFaces")?,
                ints(h, g, "NumberOfPolyhedronToFaceIds")?,
                ints(h, g, "NumberOfFaceConnectivityIds")?,
            )
        } else {
            Default::default()
        };
        let zone_data = match h.child(g, "CellData")? {
            Some(cd) => match h.child(&cd, "zone")? {
                Some(z) => Some(h.read_f64(&h.dataset(&z)?)?),
                None => None,
            },
            None => None,
        };
        let ghost = optional_ints(h, g, "CellData", "vtkGhostType")?;
        let total_cells: usize = nc
            .iter()
            .map(|&n| count(n, "cells"))
            .sum::<Result<_, _>>()?;
        if ghost.as_ref().is_some_and(|v| v.len() != total_cells) {
            return Err(MeshError::new(
                "MESH005",
                "CellData/vtkGhostType does not have one value per cell",
            ));
        }
        let gids = optional_ints(h, g, "PointData", "GlobalPointIds")?;
        // the mesh point of each row of Points
        let mut pmap: Vec<u32> = Vec::with_capacity(total_points);
        let mut point_rows: Option<Vec<u32>> = None;
        match gids {
            Some(ids) if ids.len() == total_points => {
                let map = self.gids.get_or_insert_with(HashMap::new);
                let points = &mut self.mesh.points;
                let mut rows = Vec::new();
                for (r, (q, id)) in xyz.as_chunks::<3>().0.iter().zip(ids).enumerate() {
                    let i = *map.entry(id).or_insert_with(|| {
                        points.push(*q);
                        rows.push(r as u32);
                        (points.len() - 1) as u32
                    });
                    pmap.push(i);
                }
                point_rows = Some(rows);
            }
            Some(_) => {
                return Err(MeshError::new(
                    "MESH005",
                    "PointData/GlobalPointIds does not have one value per point",
                ));
            }
            None => {
                let base = self.mesh.points.len() as u32;
                pmap.extend((0..total_points as u32).map(|r| base + r));
                self.mesh
                    .points
                    .extend(xyz.as_chunks::<3>().0.iter().copied());
            }
        }
        let mut cell_rows: Vec<u32> = Vec::with_capacity(total_cells);
        if self.mesh.cell_faces.is_empty() {
            self.mesh.cell_faces.push(0);
            self.polys = Polygons::new();
            self.mesh.boundary = Polygons::new();
        }
        // running starts of each partition in the concatenated arrays
        let (mut pb, mut cb, mut ob, mut kb) = (0usize, 0usize, 0usize, 0usize);
        let (mut pob, mut p2fb, mut fob, mut fcb) = (0usize, 0usize, 0usize, 0usize);
        let mut zones_out = Vec::new();
        for part in 0..parts {
            let npart = count(np[part], "points")?;
            let ncell = count(nc[part], "cells")?;
            let ncn = count(ncon[part], "connectivity ids")?;
            let offs = offsets
                .get(ob..ob + ncell + 1)
                .ok_or_else(|| bad("Offsets is shorter than the cells"))?;
            let cn = conn
                .get(kb..kb + ncn)
                .ok_or_else(|| bad("Connectivity is shorter than NumberOfConnectivityIds"))?;
            let tys = types
                .get(cb..cb + ncell)
                .ok_or_else(|| bad("Types is shorter than the cells"))?;
            let (po, pf, fo, fc) = if poly {
                let nf = count(*n_faces.get(part).unwrap_or(&0), "faces")?;
                let np2f = count(*n_p2f.get(part).unwrap_or(&0), "polyhedron faces")?;
                let nfc = count(*n_fconn.get(part).unwrap_or(&0), "face ids")?;
                let r = (
                    p_off.get(pob..pob + ncell + 1),
                    p_faces.get(p2fb..p2fb + np2f),
                    f_off.get(fob..fob + nf + 1),
                    f_conn.get(fcb..fcb + nfc),
                );
                pob += ncell + 1;
                p2fb += np2f;
                fob += nf + 1;
                fcb += nfc;
                match r {
                    (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
                    _ => return Err(bad("the polyhedron arrays are shorter than their counts")),
                }
            } else {
                (&[][..], &[][..], &[][..], &[][..])
            };
            let point = |v: i64| -> Result<u32, MeshError> {
                usize::try_from(v)
                    .ok()
                    .filter(|&v| v < npart)
                    .and_then(|v| pmap.get(v + pb).copied())
                    .ok_or_else(|| {
                        MeshError::new("MESH006", format!("point {v} is out of range 0..{npart}"))
                    })
            };
            for c in 0..ncell {
                if ghost.as_ref().is_some_and(|g| g[cb + c] & 1 != 0) {
                    continue;
                }
                let (s, e) = (offs[c], offs[c + 1]);
                let nodes = usize::try_from(s)
                    .ok()
                    .zip(usize::try_from(e).ok())
                    .filter(|(s, e)| s <= e && *e <= cn.len())
                    .map(|(s, e)| &cn[s..e])
                    .ok_or_else(|| {
                        MeshError::new("MESH006", format!("cell {c}: offsets {s}..{e}"))
                    })?;
                let z = match (zone, &zone_data) {
                    (Some(z), _) => z,
                    (None, Some(zd)) => {
                        let v = zd.get(cb + c).copied().unwrap_or(0.0);
                        if !(0.0..1e6).contains(&v) {
                            return Err(MeshError::new("MESH006", format!("cell {c}: zone {v}")));
                        }
                        v as u32
                    }
                    (None, None) => 0,
                };
                let t = tys[c];
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
                    let ids: Vec<u32> =
                        nodes.iter().map(|&v| point(v)).collect::<Result<_, _>>()?;
                    let mut sizes = Vec::with_capacity(table.len());
                    for f in table {
                        self.polys.push(f.iter().map(|&i| ids[i]));
                        sizes.push(f.len());
                    }
                    push_cell(
                        &mut self.mesh,
                        self.polys.len(),
                        z,
                        CellKind::classify(sizes),
                        &mut self.zone_max,
                    );
                    zones_out.push(z);
                    cell_rows.push((cb + c) as u32);
                } else if t == POLYHEDRON {
                    let (a, b) = (po.get(c), po.get(c + 1));
                    let range = a
                        .zip(b)
                        .and_then(|(&a, &b)| {
                            Some(usize::try_from(a).ok()?..usize::try_from(b).ok()?)
                        })
                        .filter(|r| r.start < r.end && r.end <= pf.len())
                        .ok_or_else(|| {
                            MeshError::new("MESH009", format!("cell {c}: PolyhedronOffsets"))
                        })?;
                    let mut sizes = Vec::with_capacity(range.len());
                    for &face in &pf[range] {
                        let fr = usize::try_from(face)
                            .ok()
                            .filter(|&f| f + 1 < fo.len())
                            .and_then(|f| {
                                Some(
                                    usize::try_from(fo[f]).ok()?
                                        ..usize::try_from(fo[f + 1]).ok()?,
                                )
                            })
                            .filter(|r| r.end >= r.start + 3 && r.end <= fc.len())
                            .ok_or_else(|| {
                                MeshError::new("MESH009", format!("cell {c}: face {face}"))
                            })?;
                        let ids: Vec<u32> =
                            fc[fr].iter().map(|&v| point(v)).collect::<Result<_, _>>()?;
                        sizes.push(ids.len());
                        self.polys.push(ids);
                    }
                    push_cell(
                        &mut self.mesh,
                        self.polys.len(),
                        z,
                        CellKind::classify(sizes),
                        &mut self.zone_max,
                    );
                    orient_last(&self.mesh, &mut self.polys);
                    zones_out.push(z);
                    cell_rows.push((cb + c) as u32);
                } else if matches!(t, TRIANGLE | POLYGON | QUAD) {
                    if nodes.len() < 3 {
                        return Err(MeshError::new(
                            "MESH006",
                            format!("cell {c}: polygon with {} points", nodes.len()),
                        ));
                    }
                    let ids: Vec<u32> =
                        nodes.iter().map(|&v| point(v)).collect::<Result<_, _>>()?;
                    self.mesh.boundary.push(ids);
                    self.mesh.boundary_patch.push(u32::MAX);
                } else {
                    return Err(MeshError::new(
                        "MESH008",
                        format!("cell {c} has VTK type {t}"),
                    ));
                }
            }
            pb += npart;
            cb += ncell;
            ob += ncell + 1;
            kb += ncn;
        }
        let all_cells = cell_rows.len() == total_cells;
        let all_points = point_rows.as_ref().is_none_or(|r| r.len() == total_points);
        Ok(GridRead {
            zones: zone_data.map(|_| zones_out),
            cell_rows_total: total_cells,
            cell_rows: (!all_cells).then_some(cell_rows),
            point_rows_total: total_points,
            point_rows: point_rows.filter(|_| !all_points),
        })
    }

    /// Read a PolyData block's polygons as boundary faces of patch `p`.
    fn poly_data(&mut self, h: &H5, g: &Object, p: u32) -> Result<(), MeshError> {
        let np = ints(h, g, "NumberOfPoints")?;
        let (xyz, _) = floats(h, g, "Points")?;
        let total: usize = np
            .iter()
            .map(|&n| count(n, "points"))
            .sum::<Result<_, _>>()?;
        if xyz.len() != 3 * total {
            return Err(MeshError::new(
                "MESH005",
                "PolyData Points do not match NumberOfPoints",
            ));
        }
        let Some(polys) = h.child(g, "Polygons")? else {
            return Ok(());
        };
        let nc = ints(h, &polys, "NumberOfCells")?;
        let ncon = ints(h, &polys, "NumberOfConnectivityIds")?;
        let offsets = ints(h, &polys, "Offsets")?;
        let conn = ints(h, &polys, "Connectivity")?;
        let mut ids: Vec<u32> = Vec::with_capacity(total);
        // a blocked file joins its patches by GlobalPointIds (§ 3)
        if let (Some(map), Some(gp)) = (
            self.gids.as_ref(),
            optional_ints(h, g, "PointData", "GlobalPointIds")?,
        ) && gp.len() == total
        {
            for id in gp {
                ids.push(*map.get(&id).ok_or_else(|| {
                    bad(format!(
                        "patch {p} has the global point {id}, which no volume cell uses"
                    ))
                })?);
            }
        }
        if ids.len() < total && self.lookup.is_none() {
            let mut m = HashMap::with_capacity(self.mesh.points.len());
            for (i, q) in self.mesh.points.iter().enumerate() {
                m.entry(q.map(f64::to_bits)).or_insert(i as u32);
            }
            self.lookup = Some(m);
        }
        let empty = HashMap::new();
        let lookup = self.lookup.as_ref().unwrap_or(&empty);
        for q in xyz.as_chunks::<3>().0.iter().skip(ids.len()) {
            match lookup.get(&q.map(f64::to_bits)) {
                Some(&i) => ids.push(i),
                None if self.mesh.cell_count() == 0 => {
                    ids.push(self.mesh.points.len() as u32);
                    self.mesh.points.push(*q);
                }
                None => {
                    return Err(bad(format!(
                        "a point of patch {p} ({:?}) is not a point of the volume mesh",
                        q
                    )));
                }
            }
        }
        let (mut pb, mut ob, mut kb) = (0usize, 0usize, 0usize);
        for part in 0..nc.len().min(np.len()).min(ncon.len()) {
            let n = count(nc[part], "polygons")?;
            let k = count(ncon[part], "connectivity ids")?;
            let npart = count(np[part], "points")?;
            let offs = offsets
                .get(ob..ob + n + 1)
                .ok_or_else(|| bad("Polygons/Offsets is shorter than its cells"))?;
            let cn = conn
                .get(kb..kb + k)
                .ok_or_else(|| bad("Polygons/Connectivity is shorter than its count"))?;
            for c in 0..n {
                let r = usize::try_from(offs[c])
                    .ok()
                    .zip(usize::try_from(offs[c + 1]).ok())
                    .filter(|(s, e)| e >= &(s + 3) && *e <= cn.len())
                    .ok_or_else(|| MeshError::new("MESH006", format!("polygon {c}: offsets")))?;
                let face: Vec<u32> = cn[r.0..r.1]
                    .iter()
                    .map(|&v| {
                        usize::try_from(v)
                            .ok()
                            .filter(|&v| v < npart)
                            .and_then(|v| ids.get(pb + v).copied())
                            .ok_or_else(|| {
                                MeshError::new("MESH006", format!("polygon {c}: point {v}"))
                            })
                    })
                    .collect::<Result<_, _>>()?;
                self.mesh.boundary.push(face);
                self.mesh.boundary_patch.push(p);
            }
            pb += npart;
            ob += n + 1;
            kb += k;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Mesh, MeshError> {
        let mesh = &mut self.mesh;
        if mesh.cell_faces.is_empty() {
            mesh.cell_faces.push(0);
        }
        // polygons that came with the volume cells form one patch of their own
        if mesh.boundary_patch.contains(&u32::MAX) {
            let p = mesh.patches.len() as u32;
            for b in &mut mesh.boundary_patch {
                if *b == u32::MAX {
                    *b = p;
                }
            }
            mesh.patches.push(Patch {
                name: "boundary".into(),
                role: crate::PatchRole::Unspecified,
            });
        }
        let zones = self.zone_max.map_or(0, |m| m as usize + 1).max(
            self.zone_names
                .keys()
                .next_back()
                .map_or(0, |&k| k as usize + 1),
        );
        mesh.zones = (0..zones as u32)
            .map(|i| {
                self.zone_names.get(&i).cloned().unwrap_or_else(|| {
                    if zones == 1 {
                        "cells".into()
                    } else {
                        format!("zone_{i}")
                    }
                })
            })
            .collect();
        let _ = &self.zone_values;
        boundary::link(mesh, std::mem::take(&mut self.polys), None);
        if mesh.cell_count() == 0 && mesh.boundary.is_empty() {
            return Err(MeshError::new(
                "MESH010",
                "no volume cells and no boundary faces",
            ));
        }
        Ok(self.mesh)
    }
}

/// Turn the faces of the last cell, a polyhedron, outward when they
/// point in (its signed volume, by the divergence theorem over fan
/// triangles, is negative): VTK asks for outward polyhedron faces but
/// does not check them. The linear cells take their faces from the
/// tables of VTK's node orders and are not turned, so a cell given
/// inside out stays visible to the quality check.
fn orient_last(mesh: &Mesh, polys: &mut Polygons) {
    let c = mesh.cell_count() - 1;
    let range = mesh.cell_face_range(c);
    let mut v = 0.0;
    for f in range.clone() {
        let face = polys.get(f);
        let p0 = mesh.points[face[0] as usize];
        for k in 1..face.len().saturating_sub(1) {
            let (a, b) = (
                mesh.points[face[k] as usize],
                mesh.points[face[k + 1] as usize],
            );
            v += p0[0] * (a[1] * b[2] - a[2] * b[1]) - p0[1] * (a[0] * b[2] - a[2] * b[0])
                + p0[2] * (a[0] * b[1] - a[1] * b[0]);
        }
    }
    if v < 0.0 {
        for f in range {
            let (s, e) = (polys.start[f] as usize, polys.start[f + 1] as usize);
            polys.nodes[s..e].reverse();
        }
    }
}

/// The text of attribute `name`, if it has one.
pub fn attr_text(h: &H5, obj: &Object, name: &str) -> Option<String> {
    h.attr(obj, name)
        .ok()
        .flatten()
        .and_then(|v: Value| v.text())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::Component;

    fn fixture(rel: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
            .join(rel)
    }

    /// The signed volume of cell `c` by the divergence theorem over its
    /// fan-triangulated faces; positive when the faces point outward.
    fn volume(m: &Mesh, c: usize) -> f64 {
        let mut v = 0.0;
        for f in m.cell(c) {
            let p0 = m.points[f.at(0) as usize];
            for k in 1..f.len() - 1 {
                let (a, b) = (m.points[f.at(k) as usize], m.points[f.at(k + 1) as usize]);
                v += p0[0] * (a[1] * b[2] - a[2] * b[1]) - p0[1] * (a[0] * b[2] - a[2] * b[0])
                    + p0[2] * (a[0] * b[1] - a[1] * b[0]);
            }
        }
        v / 6.0
    }

    /// Invariant: FoehnCraft's channel result opens as the same mesh as
    /// KoflCraft's channel hand-off (776 cells, the same prisms, 426
    /// boundary faces on inlet, outlet and walls, every boundary face
    /// owned by a cell, every cell closed with a positive volume: the
    /// VTK node orders of the linear cells are right), and its fields are
    /// the analytic ones of the fixture script: velocity joined from its
    /// components, pressure 50 − 100 x/L, the units from the file.
    #[test]
    fn reads_the_foehncraft_channel() {
        let (m, r) = read_vtkhdf(&fixture("vtkhdf/channel_fields.vtkhdf")).unwrap();
        let v = crate::read_vtu(&fixture("channel/foehncraft.vtu")).unwrap();
        assert_eq!(m.cell_count(), 776);
        assert_eq!(m.summary().cell_kinds, v.summary().cell_kinds);
        assert_eq!(m.boundary.len(), 426);
        let names: Vec<_> = m
            .patches
            .iter()
            .map(|p| (p.name.as_str(), p.role))
            .collect();
        assert_eq!(
            names,
            [
                ("inlet", crate::PatchRole::Inlet),
                ("outlet", crate::PatchRole::Outlet),
                ("walls", crate::PatchRole::Wall)
            ]
        );
        assert!(m.boundary_owner.iter().all(|&o| o != crate::NO_CELL));
        for c in 0..m.cell_count() {
            assert!(volume(&m, c) > 0.0, "cell {c}: volume {}", volume(&m, c));
        }
        let total: f64 = (0..m.cell_count()).map(|c| volume(&m, c)).sum();
        let vtotal: f64 = (0..v.cell_count()).map(|c| volume(&v, c)).sum();
        assert!((total - vtotal).abs() < 1e-9 * vtotal);
        let names: Vec<_> = r.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "velocity",
                "speed",
                "pressure",
                "k",
                "omega",
                "nu_t",
                "wall_shear_stress",
                "y_plus"
            ]
        );
        let u = r.get("velocity").unwrap();
        assert_eq!((u.components, u.unit.as_deref()), (3, Some("m/s")));
        let speed = r.get("speed").unwrap();
        assert_eq!(speed.values.len(), m.cell_count());
        for (c, s) in speed.values.iter().enumerate() {
            assert!((u.scalar(c, Component::Magnitude) - s).abs() < 1e-12);
        }
        let p = r.get("pressure").unwrap();
        assert_eq!(p.unit.as_deref(), Some("Pa"));
        let (lo, hi) = p.range(Component::Magnitude).unwrap();
        assert!(lo > -50.0 && hi < 50.0 && lo < -40.0 && hi > 40.0);
        let tau = r.get("wall_shear_stress").unwrap();
        assert_eq!((tau.location, tau.len()), (Location::Boundary, 426));
        let walls = m.patches.iter().position(|p| p.name == "walls").unwrap() as u32;
        for (f, &pa) in m.boundary_patch.iter().enumerate() {
            assert_eq!(tau.values[f] > 0.0, pa == walls);
        }
        assert_eq!(r.iteration, Some(250));
        assert_eq!(m.metadata.get("case").map(String::as_str), Some("channel"));
    }

    /// Invariant: the chunked and the newest-format box files give the
    /// same mesh and fields (their HDF5 storage differs, VTKHDF does not).
    #[test]
    fn storage_does_not_change_the_result() {
        let (a, ra) = read_vtkhdf(&fixture("vtkhdf/box_chunked.vtkhdf")).unwrap();
        let (b, rb) = read_vtkhdf(&fixture("vtkhdf/box_latest.vtkhdf")).unwrap();
        assert_eq!(a.points, b.points);
        assert_eq!(a.distinct, b.distinct);
        assert_eq!(a.cell_face_flip, b.cell_face_flip);
        assert_eq!(ra.fields, rb.fields);
        assert_eq!((ra.step, ra.time), (Some(12), Some(0.25)));
        assert_eq!(a.cell_count(), 4);
        assert!(ra.get("velocity").is_some_and(|v| v.components == 3));
    }

    /// Invariant: a file that is not HDF5 is refused with MESH018, not
    /// read as some other format.
    #[test]
    fn refuses_non_vtkhdf() {
        let e = read_vtkhdf(&fixture("README.md")).err().map(|e| e.code);
        assert_eq!(e, Some("MESH018"));
    }
}
