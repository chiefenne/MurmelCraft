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
//! the strings of `/VTKHDF` are kept. [`read_overview`] reads the
//! boundary and the block index alone, [`read_blocks`] the cells of some
//! blocks, [`read_preview`] the cell fields on a coarse regular grid.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use crate::fasthash::FastMap;
use crate::field::{Field, Location, Results, group_vectors, parse_units};
use crate::hdf5::{Dataset, H5, Object, Target, Value, Values};
use crate::{Mesh, MeshError, Patch, Polygons, boundary, par};

mod cells;
mod overview;
mod preview;
mod subset;

pub use overview::{
    Block, OPEN_BYTES_PER_CELL, Overview, Size, VolumeField, read_overview, size_of,
};
pub use preview::{PREVIEW_CELLS, PREVIEW_VERSION, PreviewGrid, read_preview};
pub use subset::{Subset, read_blocks};

/// No mesh point yet for a global point id.
const NO_POINT: u32 = u32::MAX;

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

/// As [`read_vtkhdf`]; without `hash` no BLAKE3 hash is taken
/// (`Mesh::file_blake3` stays empty).
pub(crate) fn read_vtkhdf_with(path: &Path, hash: bool) -> Result<(Mesh, Results), MeshError> {
    if !path.is_file() {
        return Err(MeshError::new(
            "MESH001",
            format!("{} is not a readable file", path.display()),
        ));
    }
    // the file's structure; its datasets are read as they are needed
    let io = crate::profile::span("read.file");
    let h = H5::open(path)?;
    drop(io);
    crate::profile::check()?;
    let t = crate::profile::span("read.vtkhdf");
    // Hash the file on another thread while it is read.
    let (read, digest) = std::thread::scope(|s| {
        let digest = hash.then(|| {
            s.spawn(|| {
                let stop = std::sync::atomic::AtomicBool::new(false);
                crate::FileHash::Blake3.of_file(path, &stop)
            })
        });
        let read = read_parts(&h);
        let digest = digest.map(|d| {
            d.join()
                .ok()
                .and_then(Result::ok)
                .flatten()
                .unwrap_or_default()
        });
        (read, digest)
    });
    drop(t);
    // The faces are linked after the datasets are freed.
    drop(h);
    let (mut mesh, results) = read?.finish()?;
    mesh.files = vec![path.to_path_buf()];
    mesh.file_blake3 = digest.into_iter().collect();
    Ok((mesh, results))
}

/// Read the mesh and fields of an open HDF5 file in the VTKHDF layout.
pub fn read_h5(h: &H5) -> Result<(Mesh, Results), MeshError> {
    read_parts(h)?.finish()
}

/// What a VTKHDF file gives before its faces are linked: all that needs
/// the file's bytes.
struct Parts {
    b: Builder,
    cell_fields: Vec<Vec<Field>>,
    point_fields: Vec<Vec<Field>>,
    boundary_fields: Vec<(usize, Vec<Field>)>,
    /// The text attributes of the root group and of `/VTKHDF`.
    attrs: BTreeMap<String, String>,
    vtk_attrs: Vec<(String, String)>,
}

/// The volume blocks (UnstructuredGrid) and the patches (PolyData) of a
/// file, each with its name, in the order of their `Index`.
type Blocks = (Vec<(String, Object)>, Vec<(String, Object)>);

/// `/VTKHDF` and its blocks.
fn blocks_of(h: &H5) -> Result<(Object, Blocks), MeshError> {
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
    Ok((vtk, (volumes, patches)))
}

/// The zone names and patch roles a volume block keeps as field-data
/// strings (§ 8).
fn names_of(
    h: &H5,
    obj: &Object,
    b: &mut Builder,
    roles: &mut BTreeMap<usize, String>,
) -> Result<(), MeshError> {
    let Some(fd) = h.child(obj, "FieldData")? else {
        return Ok(());
    };
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
    Ok(())
}

/// Which faces of the patches to read: those whose owner cell (its file
/// cell id, `OwnerCell`) the predicate takes.
pub(super) type Owners<'a> = &'a (dyn Fn(i64) -> bool + Sync);

/// Read the patches into `b`, each with its cell data as boundary fields
/// (and its first face); with `keep`, only the faces whose owner cell it
/// takes.
fn read_patches(
    h: &H5,
    b: &mut Builder,
    patches: &[(String, Object)],
    roles: &BTreeMap<usize, String>,
    keep: Option<Owners>,
) -> Result<Vec<(usize, Vec<Field>)>, MeshError> {
    let mut boundary_fields: Vec<(usize, Vec<Field>)> = Vec::new();
    for (name, obj) in patches {
        let p = b.mesh.patches.len() as u32;
        let before = b.mesh.boundary.len();
        let (n, kept) = b.poly_data(h, obj, p, keep)?;
        let role = match attr_text(h, obj, "role").or_else(|| roles.get(&(p as usize)).cloned()) {
            Some(r) => crate::PatchRole::parse(&r),
            None => crate::polymesh::patch_role(name, ""),
        };
        b.mesh.patches.push(Patch {
            name: name.clone(),
            role,
        });
        boundary_fields.push((
            before,
            data_fields(
                h,
                obj,
                "CellData",
                Location::Boundary,
                (n, kept.as_deref()),
                &["boundary", "OwnerCell"],
            )?,
        ));
    }
    Ok(boundary_fields)
}

/// The text attributes of the root group and of `/VTKHDF` (but its type
/// and version).
type Attrs = (BTreeMap<String, String>, Vec<(String, String)>);

fn text_attrs(h: &H5, vtk: &Object) -> Result<Attrs, MeshError> {
    let root = h.root()?;
    let mut attrs = BTreeMap::new();
    for (k, v) in h.attrs(&root)? {
        if let Some(t) = v.text() {
            attrs.insert(k, t);
        }
    }
    let mut vtk_attrs = Vec::new();
    for (k, v) in h.attrs(vtk)? {
        if k == "Type" || k == "Version" {
            continue;
        }
        if let Some(t) = v.text() {
            vtk_attrs.push((k, t));
        }
    }
    Ok((attrs, vtk_attrs))
}

fn read_parts(h: &H5) -> Result<Parts, MeshError> {
    let (vtk, (volumes, patches)) = blocks_of(h)?;
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
            names_of(h, obj, &mut b, &mut roles)?;
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
    let boundary_fields = read_patches(h, &mut b, &patches, &roles, None)?;
    let (attrs, vtk_attrs) = text_attrs(h, &vtk)?;
    Ok(Parts {
        b,
        cell_fields,
        point_fields,
        boundary_fields,
        attrs,
        vtk_attrs,
    })
}

impl Parts {
    /// Link the faces and gather the fields.
    fn finish(self) -> Result<(Mesh, Results), MeshError> {
        let Parts {
            b,
            cell_fields,
            point_fields,
            boundary_fields,
            attrs,
            vtk_attrs,
        } = self;
        let mut mesh = b.finish()?;

        // Fields: the cell and point data of every volume block joined
        // block after block (a field missing in one block is NaN there),
        // the cell data of the patches as boundary fields.
        let mut fields = join(cell_fields, mesh.cell_count());
        fields.extend(join(point_fields, mesh.points.len()));
        fields.extend(join_boundary(boundary_fields, mesh.boundary.len()));
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
        for (k, t) in vtk_attrs {
            mesh.metadata.insert(k, t);
        }
        // Front ends name the format from this key.
        mesh.metadata.insert("vtkhdf".into(), "2".into());
        Ok((mesh, results))
    }
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
        let Some(comps) = components(&ds, n) else {
            return Err(MeshError::new(
                "MESH005",
                format!(
                    "{group}/{} has shape {:?} for {n} {}s",
                    link.name,
                    ds.dims,
                    location.name()
                ),
            ));
        };
        let values = gather(&h.values(&ds)?, comps, keep);
        out.extend(as_fields(&link.name, location, comps, values));
    }
    Ok(out)
}

/// The values per row of a dataset of `n` rows (1 to 9), if its shape
/// fits.
fn components(ds: &Dataset, n: usize) -> Option<usize> {
    match ds.dims.as_slice() {
        [rows] if *rows as usize == n => Some(1),
        [rows, k] if *rows as usize == n && (1..=9).contains(k) => Some(*k as usize),
        _ => None,
    }
}

/// The values of dataset `name` (`comps` per row) as fields: scalars and
/// vectors as they are, other tuples (tensors) as scalars `name_0`,
/// `name_1`, …
fn as_fields(name: &str, location: Location, comps: usize, values: Vec<f64>) -> Vec<Field> {
    if comps == 1 || comps == 3 {
        return vec![Field {
            name: name.to_string(),
            unit: None,
            location,
            components: comps,
            values,
        }];
    }
    (0..comps)
        .map(|k| Field {
            name: format!("{name}_{k}"),
            unit: None,
            location,
            components: 1,
            values: values.iter().skip(k).step_by(comps).copied().collect(),
        })
        .collect()
}

/// The values of a field of `comps` components, of the rows `keep`
/// names when it is given (else of all rows), on all threads. A dataset
/// whose values the reader does not convert gives none, as before.
fn gather(v: &Values, comps: usize, keep: Option<&[u32]>) -> Vec<f64> {
    let Some(keep) = keep else {
        let mut out = vec![0.0; v.len()];
        par::chunks_mut(&mut out, 1 << 15, |at, part| {
            for (k, o) in part.iter_mut().enumerate() {
                *o = v.float(at + k);
            }
        });
        return out;
    };
    if keep.iter().any(|&r| (r as usize + 1) * comps > v.len()) {
        return Vec::new();
    }
    let mut out = vec![0.0; keep.len() * comps];
    par::chunks_mut(&mut out, 1 << 15, |at, part| {
        for (k, o) in part.iter_mut().enumerate() {
            let i = at + k;
            *o = v.float(keep[i / comps] as usize * comps + i % comps);
        }
    });
    out
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

/// Volume points by their `GlobalPointIds`: in a list by id when the ids
/// are small numbers (as the blocked writers number points), else in a
/// map.
enum Gids {
    Dense(Vec<u32>),
    Hash(FastMap<i64, u32>),
    /// By [`shard_of`] the id: the blocks of a region, joined on all
    /// threads (`subset::join_blocks`).
    Shards(Vec<FastMap<u32, u32>>),
}

/// The shard of global id `id` among `n`: a multiplicative hash (the
/// golden ratio's), its high bits, so the shards' own maps, which hash by
/// the low bits, stay spread.
pub(super) fn shard_of(id: i64, n: usize) -> usize {
    ((id as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 32) as usize % n.max(1)
}

impl Gids {
    fn get(&self, id: i64) -> Option<u32> {
        match self {
            Gids::Dense(points) => usize::try_from(id)
                .ok()
                .and_then(|i| points.get(i).copied())
                .filter(|&p| p != NO_POINT),
            Gids::Hash(map) => map.get(&id).copied(),
            Gids::Shards(maps) => {
                let key = u32::try_from(id).ok()?;
                maps.get(shard_of(id, maps.len()))?.get(&key).copied()
            }
        }
    }

    fn into_map(self) -> FastMap<i64, u32> {
        match self {
            Gids::Hash(map) => map,
            Gids::Shards(maps) => maps
                .into_iter()
                .flatten()
                .map(|(id, p)| (i64::from(id), p))
                .collect(),
            Gids::Dense(points) => points
                .into_iter()
                .enumerate()
                .filter(|&(_, p)| p != NO_POINT)
                .map(|(id, p)| (id as i64, p))
                .collect(),
        }
    }
}

#[derive(Default)]
struct Builder {
    mesh: Mesh,
    /// The faces of the volume cells as read (`Polygons::new()` once the
    /// first grid starts), and each face's smallest point.
    polys: Polygons,
    least: Vec<u32>,
    zone_max: Option<u32>,
    /// The `zone` cell data of a single volume block.
    zone_values: Option<Vec<u32>>,
    zone_names: BTreeMap<u32, String>,
    /// Volume points by their exact coordinates, for the patches.
    lookup: Option<HashMap<[u64; 3], u32>>,
    /// Volume points by their `GlobalPointIds`, when the file has them.
    gids: Option<Gids>,
    /// Without volume cells: the patch points by their `GlobalPointIds`.
    surface_gids: Option<HashMap<i64, u32>>,
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

/// The integer dataset `name` of subgroup `group`, when there is one,
/// read where it lies.
fn optional_values<'h>(
    h: &'h H5,
    obj: &Object,
    group: &str,
    name: &str,
) -> Result<Option<Values<'h>>, MeshError> {
    let Some(g) = h.child(obj, group)? else {
        return Ok(None);
    };
    match h.child(&g, name)? {
        Some(o) => Ok(Some(h.int_values(&h.dataset(&o)?)?)),
        None => Ok(None),
    }
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

/// `more` added after the polygons of `to`.
fn append(to: &mut Polygons, more: Polygons) {
    if to.is_empty() && to.nodes.is_empty() {
        *to = more;
        return;
    }
    let base = to.nodes.len() as u32;
    to.start.extend(more.start[1..].iter().map(|&e| base + e));
    to.nodes.extend_from_slice(&more.nodes);
}

/// The names of the zones of cells whose highest zone is `zone_max`:
/// the names the file gives, others `zone_<i>` (a single zone without a
/// name is `cells`).
fn zone_list(zone_max: Option<u32>, names: &BTreeMap<u32, String>) -> Vec<String> {
    let zones = zone_max
        .map_or(0, |m| m as usize + 1)
        .max(names.keys().next_back().map_or(0, |&k| k as usize + 1));
    (0..zones as u32)
        .map(|i| {
            names.get(&i).cloned().unwrap_or_else(|| {
                if zones == 1 {
                    "cells".into()
                } else {
                    format!("zone_{i}")
                }
            })
        })
        .collect()
}

/// Point `r` of `xyz` (rows of three).
fn xyz_at(xyz: &Values, r: usize) -> [f64; 3] {
    [xyz.float(3 * r), xyz.float(3 * r + 1), xyz.float(3 * r + 2)]
}

/// Rows `0..n` of `xyz` as points, on all threads.
fn points_of(xyz: &Values, n: usize) -> Vec<[f64; 3]> {
    let mut out = vec![[0.0; 3]; n];
    par::chunks_mut(&mut out, 1 << 15, |at, part| {
        for (k, q) in part.iter_mut().enumerate() {
            *q = xyz_at(xyz, at + k);
        }
    });
    out
}

/// The datasets of a grid's cells: those that lie whole in the file are
/// read a window of rows at a time, the others (chunked) once.
struct GridSets<'h> {
    h: &'h H5,
    sets: Vec<(cells::Set, Source<'h>)>,
}

enum Source<'h> {
    /// Read by rows; `true` for numbers (else integers are expected).
    Rows(Dataset, bool),
    Whole(Values<'h>),
}

impl<'h> GridSets<'h> {
    /// Add dataset `ds` as `set`, checked as reading it whole would check
    /// it; gives how many values it has.
    fn add(&mut self, set: cells::Set, ds: &Dataset, numbers: bool) -> Result<usize, MeshError> {
        let h = self.h;
        let rows = h.reads_rows(ds);
        let read = |r: std::ops::Range<usize>| {
            if numbers {
                h.values_in(ds, r)
            } else {
                h.int_values_in(ds, r)
            }
        };
        let (source, len) = if rows {
            let len = read(0..0)?.len();
            (Source::Rows(ds.clone(), numbers), len)
        } else {
            let v = read(0..usize::MAX)?;
            let len = v.len();
            (Source::Whole(v), len)
        };
        self.sets.push((set, source));
        Ok(len)
    }
}

impl cells::Sets for GridSets<'_> {
    fn rows(
        &self,
        set: cells::Set,
        rows: std::ops::Range<usize>,
    ) -> Result<Option<Values<'_>>, MeshError> {
        let Some((_, source)) = self.sets.iter().find(|(s, _)| *s == set) else {
            return Ok(None);
        };
        Ok(Some(match source {
            Source::Rows(ds, true) => self.h.values_in(ds, rows)?,
            Source::Rows(ds, false) => self.h.int_values_in(ds, rows)?,
            Source::Whole(v) => v.view(),
        }))
    }
}

/// The dataset `name` of `obj`.
fn dataset_of(h: &H5, obj: &Object, name: &str) -> Result<Dataset, MeshError> {
    let o = h
        .child(obj, name)?
        .ok_or_else(|| bad(format!("no {name} dataset")))?;
    h.dataset(&o)
}

impl Builder {
    /// Read an UnstructuredGrid block (all its partitions). Ghost cells
    /// (`vtkGhostType` with the duplicate bit) are left out, and points
    /// with the same `GlobalPointIds` are one point, so the partitions of
    /// a blocked file join into one mesh. The datasets are read where they
    /// lie and the cells built on all threads (`cells.rs`).
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
        let xyz = {
            let o = h
                .child(g, "Points")?
                .ok_or_else(|| bad("no Points dataset"))?;
            let ds = h.dataset(&o)?;
            let dims = ds.dims.clone();
            (h.values(&ds)?, dims)
        };
        let (xyz, dims) = xyz;
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
        use cells::Set;
        let mut sets = GridSets {
            h,
            sets: Vec::new(),
        };
        for (set, name) in [
            (Set::Types, "Types"),
            (Set::Offsets, "Offsets"),
            (Set::Connectivity, "Connectivity"),
        ] {
            sets.add(set, &dataset_of(h, g, name)?, false)?;
        }
        let poly = h.child(g, "PolyhedronOffsets")?.is_some();
        let poly_counts = if poly {
            for (set, name) in [
                (Set::PolyhedronOffsets, "PolyhedronOffsets"),
                (Set::PolyhedronToFaces, "PolyhedronToFaces"),
                (Set::FaceOffsets, "FaceOffsets"),
                (Set::FaceConnectivity, "FaceConnectivity"),
            ] {
                sets.add(set, &dataset_of(h, g, name)?, false)?;
            }
            Some((
                ints(h, g, "NumberOfFaces")?,
                ints(h, g, "NumberOfPolyhedronToFaceIds")?,
                ints(h, g, "NumberOfFaceConnectivityIds")?,
            ))
        } else {
            None
        };
        let cell_data = h.child(g, "CellData")?;
        let zone_data = match &cell_data {
            Some(cd) => match h.child(cd, "zone")? {
                Some(z) => Some(sets.add(Set::Zone, &h.dataset(&z)?, true)?),
                None => None,
            },
            None => None,
        };
        let ghost = match &cell_data {
            Some(cd) => match h.child(cd, "vtkGhostType")? {
                Some(o) => Some(sets.add(Set::Ghost, &h.dataset(&o)?, false)?),
                None => None,
            },
            None => None,
        };
        let total_cells: usize = nc
            .iter()
            .map(|&n| count(n, "cells"))
            .sum::<Result<_, _>>()?;
        if ghost.is_some_and(|n| n != total_cells) {
            return Err(MeshError::new(
                "MESH005",
                "CellData/vtkGhostType does not have one value per cell",
            ));
        }
        let gids = optional_values(h, g, "PointData", "GlobalPointIds")?;
        // the mesh point of each row of Points
        let (points, point_rows) = match gids {
            Some(ids) if ids.len() == total_points => {
                let (map, rows) = self.join_points(&xyz, &ids, 0..total_points);
                (cells::PointMap::Rows(map), Some(rows))
            }
            Some(_) => {
                return Err(MeshError::new(
                    "MESH005",
                    "PointData/GlobalPointIds does not have one value per point",
                ));
            }
            None => {
                let base = self.mesh.points.len() as u32;
                let new = points_of(&xyz, total_points);
                if self.mesh.points.is_empty() {
                    self.mesh.points = new;
                } else {
                    self.mesh.points.extend_from_slice(&new);
                }
                (cells::PointMap::Offset(base), None)
            }
        };
        drop(xyz);
        if self.mesh.cell_faces.is_empty() {
            self.mesh.cell_faces.push(0);
            self.polys = Polygons::new();
            self.least = Vec::new();
            self.mesh.boundary = Polygons::new();
        }
        let counts = poly_counts
            .as_ref()
            .map(|(nf, np2f, nfc)| [&nf[..], &np2f[..], &nfc[..]]);
        let part_list = cells::parts(&np, &nc, &ncon, counts);
        let t = crate::profile::span("read.cells");
        let grid = cells::Grid {
            parts: &part_list,
            poly,
            zone,
            points: &points,
            xyz: &self.mesh.points,
        };
        let read = cells::read(&grid, &sets, total_cells)?;
        drop(t);
        // back to the phase of reading the datasets (the window's text)
        drop(crate::profile::span("read.vtkhdf"));
        let zones = zone_data.is_some().then(|| read.zone.clone());
        self.add(read.polys, read.least, read.cell_end);
        self.mesh.cell_zone.extend_from_slice(&read.zone);
        self.mesh.cell_kind.extend_from_slice(&read.kind);
        self.zone_max = self.zone_max.max(read.zone_max);
        let polygons = read.boundary.len();
        append(&mut self.mesh.boundary, read.boundary);
        self.mesh
            .boundary_patch
            .extend(std::iter::repeat_n(u32::MAX, polygons));
        let cell_rows = read.rows;
        let all_cells = cell_rows.len() == total_cells;
        let all_points = point_rows.as_ref().is_none_or(|r| r.len() == total_points);
        Ok(GridRead {
            zones,
            cell_rows_total: total_cells,
            cell_rows: (!all_cells).then_some(cell_rows),
            point_rows_total: total_points,
            point_rows: point_rows.filter(|_| !all_points),
        })
    }

    /// Add a grid's cell faces after those read before.
    fn add(&mut self, polys: Polygons, least: Vec<u32>, cell_end: Vec<u32>) {
        let faces = self.polys.len() as u32;
        if self.polys.is_empty() && self.polys.nodes.is_empty() {
            self.polys = polys;
            self.least = least;
        } else {
            append(&mut self.polys, polys);
            self.least.extend_from_slice(&least);
        }
        self.mesh
            .cell_faces
            .extend(cell_end.iter().map(|&e| faces + e));
    }

    /// Join the points of rows `rows` of `xyz` by their global `ids`: a
    /// row whose id came before is that point, any other a new point, in
    /// row order. Gives the point of each row (of `rows.start + i` at
    /// `i`) and the rows that became new points.
    fn join_points(
        &mut self,
        xyz: &Values,
        ids: &Values,
        rows: std::ops::Range<usize>,
    ) -> (Vec<u32>, Vec<u32>) {
        let (from, total) = (rows.start, rows.len());
        let before = self.mesh.points.len();
        let (lo, hi) = par::map(total, 1 << 15, |r| {
            r.map(|row| ids.int(from + row))
                .fold((i64::MAX, i64::MIN), |(lo, hi), id| {
                    (lo.min(id), hi.max(id))
                })
        })
        .into_iter()
        .fold((i64::MAX, i64::MIN), |(lo, hi), (a, b)| {
            (lo.min(a), hi.max(b))
        });
        // Ids up to twice the points (and a little) go in a list by id.
        let limit = 2 * (before + total) as u64 + (1 << 16);
        let small = total == 0 || (lo >= 0 && (hi as u64) < limit);
        let mut dense = match self.gids.take() {
            None if small => Vec::new(),
            Some(Gids::Dense(points)) if small => points,
            other => {
                let mut map = other.map(Gids::into_map).unwrap_or_default();
                let joined = self.join_mapped(xyz, ids, rows, &mut map);
                self.gids = Some(Gids::Hash(map));
                return joined;
            }
        };
        if total > 0 && dense.len() <= hi as usize {
            dense.resize(hi as usize + 1, NO_POINT);
        }
        // The first row of each id that is not a point yet.
        let mut first = vec![NO_POINT; dense.len()];
        {
            let firsts = AtomicU32::from_mut_slice(&mut first);
            let known = &dense;
            par::map(total, 1 << 15, |r| {
                for row in r {
                    let id = ids.int(from + row) as usize;
                    if known[id] == NO_POINT {
                        firsts[id].fetch_min(row as u32, Relaxed);
                    }
                }
            });
        }
        let is_new = |row: usize| first[ids.int(from + row) as usize] == row as u32;
        let ranges = par::ranges(total, 1 << 15);
        let counts = par::each(ranges.clone(), |r| r.filter(|&row| is_new(row)).count());
        let mut at = Vec::with_capacity(counts.len());
        let mut new = 0;
        for c in counts {
            at.push(new);
            new += c;
        }
        let mut points = vec![[0.0; 3]; new];
        let mut rows = vec![0u32; new];
        {
            let ds = AtomicU32::from_mut_slice(&mut dense);
            let cuts: Vec<usize> = at.iter().copied().chain([new]).collect();
            let jobs: Vec<_> = ranges
                .into_iter()
                .zip(par::split_mut(&mut points, &cuts))
                .zip(par::split_mut(&mut rows, &cuts))
                .zip(&at)
                .collect();
            par::each(jobs, |(((r, pts), rws), &base)| {
                let mut k = 0;
                for row in r {
                    if is_new(row) {
                        pts[k] = xyz_at(xyz, from + row);
                        rws[k] = (from + row) as u32;
                        ds[ids.int(from + row) as usize].store((before + base + k) as u32, Relaxed);
                        k += 1;
                    }
                }
            });
        }
        let mut map = vec![0u32; total];
        par::chunks_mut(&mut map, 1 << 15, |at, part| {
            for (k, m) in part.iter_mut().enumerate() {
                *m = dense[ids.int(from + at + k) as usize];
            }
        });
        if self.mesh.points.is_empty() {
            self.mesh.points = points;
        } else {
            self.mesh.points.extend_from_slice(&points);
        }
        self.gids = Some(Gids::Dense(dense));
        (map, rows)
    }

    /// [`Builder::join_points`] by a map, for ids that are not small
    /// numbers.
    fn join_mapped(
        &mut self,
        xyz: &Values,
        ids: &Values,
        rows: std::ops::Range<usize>,
        map: &mut FastMap<i64, u32>,
    ) -> (Vec<u32>, Vec<u32>) {
        map.reserve(rows.len());
        let points = &mut self.mesh.points;
        let mut new = Vec::new();
        let mut pmap = Vec::with_capacity(rows.len());
        for r in rows {
            let i = *map.entry(ids.int(r)).or_insert_with(|| {
                points.push(xyz_at(xyz, r));
                new.push(r as u32);
                (points.len() - 1) as u32
            });
            pmap.push(i);
        }
        (pmap, new)
    }

    /// Read a PolyData block's polygons as boundary faces of patch `p`;
    /// with `keep`, only those whose owner cell it takes. Gives the
    /// polygons the block has and, with `keep`, the rows of those read.
    fn poly_data(
        &mut self,
        h: &H5,
        g: &Object,
        p: u32,
        keep: Option<Owners>,
    ) -> Result<(usize, Option<Vec<u32>>), MeshError> {
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
            return Ok((0, keep.map(|_| Vec::new())));
        };
        // the owner of each polygon, when only some are read
        let owners = match keep {
            Some(_) => Some(
                optional_ints(h, g, "CellData", "OwnerCell")?.ok_or_else(|| {
                    bad(format!(
                        "patch {p} has no CellData/OwnerCell, so its faces cannot be read by block"
                    ))
                })?,
            ),
            None => None,
        };
        // a file without volume cells starts its boundary here
        if self.mesh.boundary.start.is_empty() {
            self.mesh.boundary = Polygons::new();
        }
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
                ids.push(match map.get(id) {
                    Some(i) => i,
                    // a point of a face that is not read
                    None if keep.is_some() => NO_POINT,
                    None => {
                        return Err(bad(format!(
                            "patch {p} has the global point {id}, which no volume cell uses"
                        )));
                    }
                });
            }
        }
        // without volume cells (an overview), the patches join among
        // themselves by GlobalPointIds when they have them
        if ids.is_empty()
            && self.gids.is_none()
            && self.mesh.cell_count() == 0
            && let Some(gp) = optional_ints(h, g, "PointData", "GlobalPointIds")?
            && gp.len() == total
        {
            let map = self.surface_gids.get_or_insert_with(HashMap::new);
            for (id, q) in gp.into_iter().zip(xyz.as_chunks::<3>().0) {
                let next = self.mesh.points.len() as u32;
                let i = *map.entry(id).or_insert(next);
                if i == next {
                    self.mesh.points.push(*q);
                }
                ids.push(i);
            }
        }
        if ids.len() < total && self.lookup.is_none() {
            let mut m = HashMap::with_capacity(self.mesh.points.len());
            for (i, q) in self.mesh.points.iter().enumerate() {
                m.entry(q.map(f64::to_bits)).or_insert(i as u32);
            }
            self.lookup = Some(m);
        }
        let looked_up = ids.len() < total;
        let mut lookup = self.lookup.take().unwrap_or_default();
        for q in xyz.as_chunks::<3>().0.iter().skip(ids.len()) {
            match lookup.get(&q.map(f64::to_bits)) {
                Some(&i) => ids.push(i),
                None if self.mesh.cell_count() == 0 => {
                    // a point of the patches before is found again
                    let i = self.mesh.points.len() as u32;
                    lookup.insert(q.map(f64::to_bits), i);
                    ids.push(i);
                    self.mesh.points.push(*q);
                }
                None if keep.is_some() => ids.push(NO_POINT),
                None => {
                    return Err(bad(format!(
                        "a point of patch {p} ({:?}) is not a point of the volume mesh",
                        q
                    )));
                }
            }
        }
        if looked_up {
            self.lookup = Some(lookup);
        }
        let (mut pb, mut ob, mut kb) = (0usize, 0usize, 0usize);
        let mut row = 0usize;
        let mut kept = keep.map(|_| Vec::new());
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
                let here = row;
                row += 1;
                if let (Some(keep), Some(owners), Some(kept)) = (keep, &owners, &mut kept) {
                    if !owners.get(here).is_some_and(|&o| keep(o)) {
                        continue;
                    }
                    kept.push(here as u32);
                }
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
                            .filter(|&i| i != NO_POINT)
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
        Ok((row, kept))
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
        mesh.zones = zone_list(self.zone_max, &self.zone_names);
        let _ = &self.zone_values;
        let polys = std::mem::take(&mut self.polys);
        let least = std::mem::take(&mut self.least);
        let least = (least.len() == polys.len()).then_some(least);
        let _t = crate::profile::span("read.topology");
        boundary::link(mesh, polys, least);
        if mesh.cell_count() == 0 && mesh.boundary.is_empty() {
            return Err(MeshError::new(
                "MESH010",
                "no volume cells and no boundary faces",
            ));
        }
        Ok(self.mesh)
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
    use std::collections::HashSet;

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

    /// Invariant: points join by their global ids alike whether the ids
    /// go in a list by id (small numbers, as the blocked writers number
    /// points) or in a map (any other ids), on one thread or several, and
    /// across grids, also when a grid's ids move a list into a map: a
    /// row whose id came before, in its grid or an earlier one, is that
    /// point, any other row a new point, in row order. Breaks when the
    /// threads number new points in the order they finish, a row takes
    /// the first row of its id within its own range only, or a list that
    /// becomes a map loses an id. The blocked files of the other tests
    /// number their points 0 … n − 1, so they join by list only.
    #[test]
    fn points_join_alike_by_list_or_map() {
        let mut seed = 5u64;
        let mut next = |m: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) % m) as i64
        };
        // three grids whose ids repeat within each and across them; the
        // last has one large id
        let mut grids: Vec<(Vec<i64>, Vec<f64>)> = [(300, 120), (200, 260), (50, 40)]
            .into_iter()
            .map(|(rows, ids)| {
                let ids = (0..rows).map(|_| next(ids)).collect();
                let xyz = (0..3 * rows).map(|_| next(1000) as f64).collect();
                (ids, xyz)
            })
            .collect();
        grids[2].0[17] = 1 << 40;
        let distinct: HashSet<i64> = grids.iter().flat_map(|(ids, _)| ids.clone()).collect();
        let join = |threads: usize, shift: i64| {
            par::with_threads(threads, || {
                let mut b = Builder::default();
                let mut out = Vec::new();
                for (ids, xyz) in &grids {
                    let ids: Vec<i64> = ids.iter().map(|&i| i + shift).collect();
                    let (map, rows) = b.join_points(
                        &Values::of_floats(xyz),
                        &Values::of_ints(&ids),
                        0..ids.len(),
                    );
                    let list = matches!(b.gids, Some(Gids::Dense(_)));
                    out.push((map, rows, list));
                }
                (b.mesh.points, out)
            })
        };
        let by_map = join(1, -(1 << 50));
        assert_eq!(by_map.0.len(), distinct.len());
        for threads in [1, 2, 3, 7] {
            let (points, grids) = join(threads, 0);
            let lists: Vec<bool> = grids.iter().map(|g| g.2).collect();
            assert_eq!(lists, [true, true, false], "{threads} threads");
            assert_eq!(points, by_map.0, "{threads} threads");
            for (g, (a, b)) in grids.iter().zip(&by_map.1).enumerate() {
                assert_eq!((&a.0, &a.1), (&b.0, &b.1), "grid {g}, {threads} threads");
            }
        }
    }

    /// Invariant: a field's values are gathered row by row, all its
    /// components, in the order of the rows kept (the owned cells of a
    /// blocked file); a dataset whose values the reader does not convert
    /// (half floats) gives no values, whether rows are kept or not. Breaks
    /// when the kept rows index the values unchecked: before this test a
    /// blocked file with ghosts and a half-float field panicked. The
    /// fixtures hold no such field.
    #[test]
    fn fields_gather_the_rows_kept() {
        let v = Values::of_floats(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(gather(&v, 2, Some(&[2, 0])), [5.0, 6.0, 1.0, 2.0]);
        assert_eq!(gather(&v, 3, None), [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let half = crate::hdf5::Dtype::Float { size: 2, be: false };
        let half = Values::new(std::borrow::Cow::Owned(vec![0; 12]), &half).unwrap();
        assert!(gather(&half, 1, Some(&[0, 3])).is_empty());
        assert!(gather(&half, 1, None).is_empty());
    }

    /// Invariant: a VTKHDF file of patches only (PolyData blocks, no
    /// volume) reads every polygon of every patch, the first one too, with
    /// one patch number per face and the points the patches share once.
    /// Breaks when the boundary starts without its first offset (the first
    /// polygon is lost and the patch numbers no longer match the faces),
    /// which only a file without volume cells meets: the volume reader
    /// starts the boundary. No fixture is a surface only.
    #[test]
    fn a_file_of_patches_only_reads_every_polygon() {
        use crate::h5write::{AttrValue, H5Writer, Link};
        let path =
            std::env::temp_dir().join(format!("murmel-surface-{}.vtkhdf", std::process::id()));
        let mut w = H5Writer::create(&path).unwrap();
        let patch = |w: &mut H5Writer, index: i64, x: f64| {
            let xyz = [0.0, 0.0, 0.0, x, 0.0, 0.0, x, 1.0, 0.0, 0.0, 1.0, 0.0];
            let cells = w.dataset(&[1], &[2i64]).unwrap();
            let ids = w.dataset(&[1], &[6i64]).unwrap();
            let offsets = w.dataset(&[3], &[0i64, 3, 6]).unwrap();
            let conn = w.dataset(&[6], &[0i64, 1, 2, 0, 2, 3]).unwrap();
            let polygons = w
                .group(
                    &[
                        Link::Hard("NumberOfCells".into(), cells),
                        Link::Hard("NumberOfConnectivityIds".into(), ids),
                        Link::Hard("Offsets".into(), offsets),
                        Link::Hard("Connectivity".into(), conn),
                    ],
                    &[],
                )
                .unwrap();
            let np = w.dataset(&[1], &[4i64]).unwrap();
            let points = w.dataset(&[4, 3], &xyz).unwrap();
            let links = [
                Link::Hard("NumberOfPoints".into(), np),
                Link::Hard("Points".into(), points),
                Link::Hard("Polygons".into(), polygons),
            ];
            w.group(
                &links,
                &[
                    ("Index", AttrValue::Int(index)),
                    ("Type", AttrValue::Text("PolyData".into())),
                    ("Version", AttrValue::Ints(vec![2, 4])),
                ],
            )
            .unwrap()
        };
        let (a, b) = (patch(&mut w, 0, 1.0), patch(&mut w, 1, -1.0));
        let vtk = w
            .group(
                &[Link::Hard("left".into(), a), Link::Hard("right".into(), b)],
                &[
                    (
                        "Type",
                        AttrValue::Text("PartitionedDataSetCollection".into()),
                    ),
                    ("Version", AttrValue::Ints(vec![2, 4])),
                ],
            )
            .unwrap();
        let root = w.group(&[Link::Hard("VTKHDF".into(), vtk)], &[]).unwrap();
        w.finish(root).unwrap();
        let (m, _) = read_vtkhdf(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(m.cell_count(), 0);
        assert_eq!(m.boundary.len(), 4);
        assert_eq!(m.boundary_patch, [0, 0, 1, 1]);
        assert_eq!(m.boundary.get(0), [0, 1, 2]);
        assert_eq!(m.patches.len(), 2);
        // (0, 0, 0) and (0, 1, 0) are in both patches
        assert_eq!(m.points.len(), 6);
        assert_eq!(m.boundary.get(2), [0, 4, 5]);
    }

    /// Invariant: a file that is not HDF5 is refused with MESH018, not
    /// read as some other format.
    #[test]
    fn refuses_non_vtkhdf() {
        let e = read_vtkhdf(&fixture("README.md")).err().map(|e| e.code);
        assert_eq!(e, Some("MESH018"));
    }
}
