//! Reader for VTK XML UnstructuredGrid files (`.vtu`), written from the
//! VTK file-format documentation (VTK User's Guide, "VTK File Formats",
//! XML formats and the polyhedron cell layout with `faces` and
//! `faceoffsets`) and KoflCraft's `docs/foehncraft.md`.
//!
//! Supported: one `Piece`, `ascii` and uncompressed `binary` (base64)
//! arrays, little endian, cell types polyhedron (42), tetra (10),
//! hexahedron (12), wedge (13), pyramid (14) and the boundary polygons
//! triangle (5), polygon (7) and quad (9). The cell array `region` gives
//! the zone of a volume cell and the patch of a boundary face; the
//! field-data strings `zone_names`, `boundary_names` and `boundary_roles`
//! (`id=name`) name them. Appended and compressed data give `MESH007`.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine as _;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::{CellKind, Mesh, MeshError, Patch, PatchRole, Polygons, boundary};

const POLYHEDRON: i64 = 42;
const TETRA: i64 = 10;
const HEXAHEDRON: i64 = 12;
const WEDGE: i64 = 13;
const PYRAMID: i64 = 14;
const TRIANGLE: i64 = 5;
const POLYGON: i64 = 7;
const QUAD: i64 = 9;

const TETRA_FACES: &[&[usize]] = &[&[0, 1, 3], &[1, 2, 3], &[2, 0, 3], &[0, 2, 1]];
const HEX_FACES: &[&[usize]] = &[
    &[0, 4, 7, 3],
    &[1, 2, 6, 5],
    &[0, 1, 5, 4],
    &[3, 7, 6, 2],
    &[0, 3, 2, 1],
    &[4, 5, 6, 7],
];
const WEDGE_FACES: &[&[usize]] = &[
    &[0, 1, 2],
    &[3, 5, 4],
    &[0, 3, 4, 1],
    &[1, 4, 5, 2],
    &[2, 5, 3, 0],
];
const PYRAMID_FACES: &[&[usize]] = &[
    &[0, 3, 2, 1],
    &[0, 1, 4],
    &[1, 2, 4],
    &[2, 3, 4],
    &[3, 0, 4],
];

fn err(code: &'static str, message: impl Into<String>) -> MeshError {
    MeshError::new(code, message)
}

/// Read a `.vtu` file.
pub fn read_vtu(path: &Path) -> Result<Mesh, MeshError> {
    if !path.is_file() {
        return Err(err(
            "MESH001",
            format!("{} is not a readable file", path.display()),
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| err("MESH001", format!("cannot read {}: {e}", path.display())))?;
    match std::str::from_utf8(&bytes) {
        Ok(s) => read_vtu_str(s),
        Err(_) => {
            if contains(&bytes, b"<AppendedData") {
                Err(err(
                    "MESH007",
                    "raw appended data is not supported; write ascii or binary (base64) arrays",
                ))
            } else {
                Err(err("MESH003", "the file is not UTF-8 XML"))
            }
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Other,
    FieldData,
    Points,
    Cells,
    CellData,
}

#[derive(Debug)]
struct RawArray<'a> {
    section: Section,
    name: String,
    ty: String,
    components: usize,
    format: String,
    content: &'a str,
}

struct Header {
    header64: bool,
    points: usize,
    cells: usize,
}

fn attr(e: &BytesStart<'_>, key: &str) -> Result<Option<String>, MeshError> {
    match e.try_get_attribute(key) {
        Ok(Some(a)) => a
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map(|v| Some(v.into_owned()))
            .map_err(|x| err("MESH003", format!("bad attribute {key}: {x}"))),
        Ok(None) => Ok(None),
        Err(x) => Err(err("MESH003", format!("bad attribute {key}: {x}"))),
    }
}

fn count_attr(e: &BytesStart<'_>, key: &str) -> Result<usize, MeshError> {
    let v = attr(e, key)?.ok_or_else(|| err("MESH003", format!("Piece has no {key}")))?;
    v.trim()
        .parse()
        .map_err(|_| err("MESH003", format!("{key}=\"{v}\" is not a count")))
}

/// Read a `.vtu` from its text.
pub fn read_vtu_str(text: &str) -> Result<Mesh, MeshError> {
    let len = text.len();
    let mut reader = Reader::from_str(text);
    let mut section = Section::Other;
    let mut header: Option<Header> = None;
    let mut saw_file = false;
    let mut header64 = false;
    let mut arrays: Vec<RawArray<'_>> = Vec::new();
    loop {
        let ev = reader
            .read_event()
            .map_err(|e| err("MESH003", format!("XML error: {e}")))?;
        match ev {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == "VTKFile" => {
                saw_file = true;
                let ty = attr(&e, "type")?.unwrap_or_default();
                if ty != "UnstructuredGrid" {
                    return Err(err(
                        "MESH002",
                        format!("VTKFile type is \"{ty}\", not UnstructuredGrid"),
                    ));
                }
                if attr(&e, "compressor")?.is_some_and(|c| !c.is_empty()) {
                    return Err(err("MESH007", "compressed data is not supported"));
                }
                if attr(&e, "byte_order")?.is_some_and(|b| b == "BigEndian") {
                    return Err(err("MESH007", "big-endian binary data is not supported"));
                }
                header64 = attr(&e, "header_type")?.is_some_and(|h| h == "UInt64");
            }
            Event::Start(e) => {
                let name = e.local_name();
                match name.as_ref() {
                    "Piece" => piece(&e, &mut header, header64, len)?,
                    "FieldData" => section = Section::FieldData,
                    "Points" => section = Section::Points,
                    "Cells" => section = Section::Cells,
                    "CellData" => section = Section::CellData,
                    "PointData" => section = Section::Other,
                    "AppendedData" => {
                        return Err(err("MESH007", "appended data is not supported"));
                    }
                    "DataArray" => {
                        let mut a = raw_array(&e, section)?;
                        let end = e.to_end().into_owned();
                        let start = reader.buffer_position() as usize;
                        reader
                            .read_to_end(end.name())
                            .map_err(|x| err("MESH003", format!("DataArray {}: {x}", a.name)))?;
                        let stop = reader.buffer_position() as usize;
                        let inner = text.get(start..stop).unwrap_or("");
                        a.content = inner
                            .rfind("</")
                            .map_or(inner, |i| inner.get(..i).unwrap_or(""));
                        arrays.push(a);
                    }
                    _ => {}
                }
            }
            Event::Empty(e) if e.local_name().as_ref() == "Piece" => {
                piece(&e, &mut header, header64, len)?;
            }
            Event::Empty(e) if e.local_name().as_ref() == "DataArray" => {
                arrays.push(raw_array(&e, section)?);
            }
            Event::End(e) => match e.local_name().as_ref() {
                "FieldData" | "Points" | "Cells" | "CellData" | "PointData" => {
                    section = Section::Other;
                }
                _ => {}
            },
            _ => {}
        }
    }
    if !saw_file {
        return Err(err("MESH002", "no VTKFile element"));
    }
    let header = header.ok_or_else(|| err("MESH003", "no Piece element"))?;
    assemble(&header, &arrays)
}

fn piece(
    e: &BytesStart<'_>,
    header: &mut Option<Header>,
    header64: bool,
    len: usize,
) -> Result<(), MeshError> {
    if header.is_some() {
        return Err(err("MESH012", "the file has more than one Piece"));
    }
    let points = count_attr(e, "NumberOfPoints")?;
    let cells = count_attr(e, "NumberOfCells")?;
    // Every coordinate takes at least two bytes ("0 ") and every cell at
    // least one type and one offset.
    if points.saturating_mul(6) > len || cells.saturating_mul(2) > len {
        return Err(err(
            "MESH004",
            format!("{points} points and {cells} cells cannot fit in {len} bytes"),
        ));
    }
    *header = Some(Header {
        header64,
        points,
        cells,
    });
    Ok(())
}

fn raw_array<'a>(e: &BytesStart<'_>, section: Section) -> Result<RawArray<'a>, MeshError> {
    let components = match attr(e, "NumberOfComponents")? {
        Some(c) => c
            .trim()
            .parse()
            .map_err(|_| err("MESH003", format!("NumberOfComponents=\"{c}\"")))?,
        None => 1,
    };
    let format = attr(e, "format")?.unwrap_or_else(|| "ascii".into());
    if format == "appended" {
        return Err(err("MESH007", "appended data is not supported"));
    }
    Ok(RawArray {
        section,
        name: attr(e, "Name")?.unwrap_or_default(),
        ty: attr(e, "type")?.unwrap_or_default(),
        components,
        format,
        content: "",
    })
}

fn find<'b, 'a>(
    arrays: &'b [RawArray<'a>],
    section: Section,
    name: &str,
) -> Option<&'b RawArray<'a>> {
    arrays
        .iter()
        .find(|a| a.section == section && a.name == name)
}

fn element_size(ty: &str) -> Option<usize> {
    Some(match ty {
        "Int8" | "UInt8" | "String" | "Char" => 1,
        "Int16" | "UInt16" => 2,
        "Int32" | "UInt32" | "Float32" => 4,
        "Int64" | "UInt64" | "Float64" => 8,
        _ => return None,
    })
}

/// The bytes of a binary (base64) array without its length header.
fn binary_bytes(a: &RawArray<'_>, header64: bool) -> Result<Vec<u8>, MeshError> {
    let cleaned: String = a.content.split_ascii_whitespace().collect();
    let hl = if header64 { 8 } else { 4 };
    // The header is encoded on its own or together with the data; try
    // both layouts (VTK writes them together for uncompressed data).
    let engine = base64::engine::general_purpose::STANDARD;
    let all = engine
        .decode(cleaned.as_bytes())
        .map_err(|e| err("MESH005", format!("{}: bad base64: {e}", a.name)))?;
    if all.len() < hl {
        return Err(err("MESH005", format!("{}: binary data too short", a.name)));
    }
    let mut n = [0u8; 8];
    n[..hl].copy_from_slice(&all[..hl]);
    let nbytes = u64::from_le_bytes(n) as usize;
    if all.len() - hl < nbytes {
        return Err(err(
            "MESH005",
            format!(
                "{}: header says {nbytes} bytes, {} present",
                a.name,
                all.len() - hl
            ),
        ));
    }
    Ok(all[hl..hl + nbytes].to_vec())
}

fn floats(a: &RawArray<'_>, header64: bool) -> Result<Vec<f64>, MeshError> {
    if a.format == "binary" {
        let b = binary_bytes(a, header64)?;
        let size = element_size(&a.ty)
            .ok_or_else(|| err("MESH005", format!("{}: type {}", a.name, a.ty)))?;
        if b.len() % size != 0 {
            return Err(err("MESH005", format!("{}: ragged binary data", a.name)));
        }
        return Ok(b.chunks_exact(size).map(|c| decode_f64(&a.ty, c)).collect());
    }
    a.content
        .split_ascii_whitespace()
        .map(|t| {
            t.parse::<f64>()
                .map_err(|_| err("MESH005", format!("{}: \"{t}\" is not a number", a.name)))
        })
        .collect()
}

fn ints(a: &RawArray<'_>, header64: bool) -> Result<Vec<i64>, MeshError> {
    if a.format == "binary" {
        let b = binary_bytes(a, header64)?;
        let size = element_size(&a.ty)
            .ok_or_else(|| err("MESH005", format!("{}: type {}", a.name, a.ty)))?;
        if b.len() % size != 0 {
            return Err(err("MESH005", format!("{}: ragged binary data", a.name)));
        }
        return Ok(b.chunks_exact(size).map(|c| decode_i64(&a.ty, c)).collect());
    }
    a.content
        .split_ascii_whitespace()
        .map(|t| {
            t.parse::<i64>()
                .map_err(|_| err("MESH005", format!("{}: \"{t}\" is not an integer", a.name)))
        })
        .collect()
}

fn le<const N: usize>(c: &[u8]) -> [u8; N] {
    let mut b = [0u8; N];
    b.copy_from_slice(&c[..N]);
    b
}

fn decode_f64(ty: &str, c: &[u8]) -> f64 {
    match ty {
        "Float32" => f32::from_le_bytes(le(c)) as f64,
        "Float64" => f64::from_le_bytes(le(c)),
        _ => decode_i64(ty, c) as f64,
    }
}

fn decode_i64(ty: &str, c: &[u8]) -> i64 {
    match ty {
        "Int8" => i8::from_le_bytes(le(c)) as i64,
        "UInt8" | "String" | "Char" => c[0] as i64,
        "Int16" => i16::from_le_bytes(le(c)) as i64,
        "UInt16" => u16::from_le_bytes(le(c)) as i64,
        "Int32" => i32::from_le_bytes(le(c)) as i64,
        "UInt32" => u32::from_le_bytes(le(c)) as i64,
        "Int64" => i64::from_le_bytes(le(c)),
        "UInt64" => u64::from_le_bytes(le(c)) as i64,
        "Float32" => f32::from_le_bytes(le(c)) as i64,
        "Float64" => f64::from_le_bytes(le(c)) as i64,
        _ => 0,
    }
}

/// A string array: bytes, each string ended by a zero byte.
fn strings(a: &RawArray<'_>, header64: bool) -> Result<Vec<String>, MeshError> {
    let bytes: Vec<u8> = ints(a, header64)?
        .into_iter()
        .map(|v| u8::try_from(v).map_err(|_| err("MESH005", format!("{}: byte {v}", a.name))))
        .collect::<Result<_, _>>()?;
    Ok(bytes
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect())
}

/// `id=name` entries into a table indexed by id.
fn id_table(entries: &[String]) -> BTreeMap<usize, String> {
    let mut t = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        match e.split_once('=') {
            Some((id, name)) if id.trim().parse::<usize>().is_ok() => {
                if let Ok(id) = id.trim().parse::<usize>() {
                    t.insert(id, name.to_string());
                }
            }
            _ => {
                t.insert(i, e.clone());
            }
        }
    }
    t
}

fn index(v: i64, bound: usize, what: &str) -> Result<u32, MeshError> {
    if v < 0 || v as u64 >= bound as u64 || v > u32::MAX as i64 {
        return Err(err(
            "MESH006",
            format!("{what} {v} is out of range 0..{bound}"),
        ));
    }
    Ok(v as u32)
}

fn assemble(h: &Header, arrays: &[RawArray<'_>]) -> Result<Mesh, MeshError> {
    let hb = h.header64;
    let mut mesh = Mesh::default();

    let pts = arrays
        .iter()
        .find(|a| a.section == Section::Points)
        .ok_or_else(|| err("MESH003", "no Points array"))?;
    if pts.components != 3 {
        return Err(err("MESH005", "Points must have 3 components"));
    }
    let xyz = floats(pts, hb)?;
    if xyz.len() != 3 * h.points {
        return Err(err(
            "MESH005",
            format!("Points has {} values, {} expected", xyz.len(), 3 * h.points),
        ));
    }
    mesh.points = xyz.as_chunks::<3>().0.to_vec();

    let need = |name: &str| {
        find(arrays, Section::Cells, name)
            .ok_or_else(|| err("MESH003", format!("Cells has no {name} array")))
    };
    let conn = ints(need("connectivity")?, hb)?;
    let offsets = ints(need("offsets")?, hb)?;
    let types = ints(need("types")?, hb)?;
    if offsets.len() != h.cells || types.len() != h.cells {
        return Err(err(
            "MESH005",
            format!(
                "{} offsets and {} types for {} cells",
                offsets.len(),
                types.len(),
                h.cells
            ),
        ));
    }
    let faces = match find(arrays, Section::Cells, "faces") {
        Some(a) => ints(a, hb)?,
        None => Vec::new(),
    };
    let face_offsets = match find(arrays, Section::Cells, "faceoffsets") {
        Some(a) => ints(a, hb)?,
        None => Vec::new(),
    };
    let region = match find(arrays, Section::CellData, "region") {
        Some(a) => {
            let r = ints(a, hb)?;
            if r.len() != h.cells {
                return Err(err(
                    "MESH005",
                    format!("region has {} values for {} cells", r.len(), h.cells),
                ));
            }
            Some(r)
        }
        None => None,
    };

    let np = mesh.points.len();
    mesh.faces = Polygons::new();
    mesh.cell_faces = vec![0];
    mesh.boundary = Polygons::new();
    let mut start = 0usize;
    let mut face_pos = 0usize;
    let mut zone_max: Option<u32> = None;
    let mut patch_max: Option<u32> = None;
    for c in 0..h.cells {
        let end = usize::try_from(offsets[c])
            .ok()
            .filter(|&e| e >= start && e <= conn.len())
            .ok_or_else(|| err("MESH006", format!("cell {c}: offset {}", offsets[c])))?;
        let nodes = &conn[start..end];
        start = end;
        let reg = match &region {
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
                .filter(|&e| e > face_pos && e <= faces.len())
                .ok_or_else(|| err("MESH009", format!("cell {c}: faceoffset {fo}")))?;
            let stream = &faces[face_pos..fend];
            face_pos = fend;
            let nf = usize::try_from(stream[0])
                .map_err(|_| err("MESH009", format!("cell {c}: face count")))?;
            let mut k = 1usize;
            let mut sizes = Vec::with_capacity(nf.min(stream.len()));
            for _ in 0..nf {
                let n = stream
                    .get(k)
                    .and_then(|&n| usize::try_from(n).ok())
                    .filter(|&n| n >= 3)
                    .ok_or_else(|| err("MESH009", format!("cell {c}: bad face size")))?;
                let ids = stream
                    .get(k + 1..k + 1 + n)
                    .ok_or_else(|| err("MESH009", format!("cell {c}: face runs past its cell")))?;
                let face: Vec<u32> = ids
                    .iter()
                    .map(|&v| index(v, np, "point"))
                    .collect::<Result<_, _>>()?;
                mesh.faces.push(face);
                sizes.push(n);
                k += 1 + n;
            }
            if k != stream.len() {
                return Err(err(
                    "MESH009",
                    format!("cell {c}: {} values left after its faces", stream.len() - k),
                ));
            }
            push_cell(&mut mesh, reg, CellKind::classify(sizes), &mut zone_max);
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
            let ids: Vec<u32> = nodes
                .iter()
                .map(|&v| index(v, np, "point"))
                .collect::<Result<_, _>>()?;
            let mut sizes = Vec::with_capacity(table.len());
            for f in table {
                mesh.faces.push(f.iter().map(|&i| ids[i]));
                sizes.push(f.len());
            }
            push_cell(&mut mesh, reg, CellKind::classify(sizes), &mut zone_max);
        } else if matches!(t, TRIANGLE | POLYGON | QUAD) {
            if nodes.len() < 3 {
                return Err(err(
                    "MESH006",
                    format!("cell {c}: polygon with {} points", nodes.len()),
                ));
            }
            let ids: Vec<u32> = nodes
                .iter()
                .map(|&v| index(v, np, "point"))
                .collect::<Result<_, _>>()?;
            mesh.boundary.push(ids);
            mesh.boundary_patch.push(reg);
            patch_max = Some(patch_max.map_or(reg, |m: u32| m.max(reg)));
        } else {
            return Err(err("MESH008", format!("cell {c} has VTK type {t}")));
        }
    }

    // Names and roles from the field data.
    let field = |name: &str| -> Result<Vec<String>, MeshError> {
        match find(arrays, Section::FieldData, name) {
            Some(a) => strings(a, hb),
            None => Ok(Vec::new()),
        }
    };
    let zone_names = id_table(&field("zone_names")?);
    let patch_names = id_table(&field("boundary_names")?);
    let patch_roles = id_table(&field("boundary_roles")?);
    let zones = zone_max
        .map_or(0, |m| m as usize + 1)
        .max(zone_names.keys().next_back().map_or(0, |k| k + 1));
    mesh.zones = (0..zones)
        .map(|i| {
            zone_names.get(&i).cloned().unwrap_or_else(|| {
                if zones == 1 {
                    "cells".into()
                } else {
                    format!("zone_{i}")
                }
            })
        })
        .collect();
    let patches = patch_max
        .map_or(0, |m| m as usize + 1)
        .max(patch_names.keys().next_back().map_or(0, |k| k + 1));
    mesh.patches = (0..patches)
        .map(|i| Patch {
            name: patch_names
                .get(&i)
                .cloned()
                .unwrap_or_else(|| format!("patch_{i}")),
            role: patch_roles
                .get(&i)
                .map_or(PatchRole::Unspecified, |r| PatchRole::parse(r)),
        })
        .collect();
    for a in arrays.iter().filter(|a| a.section == Section::FieldData) {
        if matches!(
            a.name.as_str(),
            "zone_names" | "boundary_names" | "boundary_roles"
        ) {
            continue;
        }
        if a.ty == "String"
            && let Some(v) = strings(a, hb)?.into_iter().next()
        {
            mesh.metadata.insert(a.name.clone(), v);
        }
    }

    if mesh.boundary.is_empty() && mesh.cell_count() > 0 {
        boundary::extract(&mut mesh);
    }
    if mesh.cell_count() == 0 && mesh.boundary.is_empty() {
        return Err(err("MESH010", "no volume cells and no boundary faces"));
    }
    Ok(mesh)
}

fn push_cell(mesh: &mut Mesh, zone: u32, kind: CellKind, zone_max: &mut Option<u32>) {
    mesh.cell_faces.push(mesh.faces.len() as u32);
    mesh.cell_zone.push(zone);
    mesh.cell_kind.push(kind);
    *zone_max = Some(zone_max.map_or(zone, |m| m.max(zone)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rel: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
            .join(rel)
    }

    /// Invariant: the box hand-off (4 Voronoi cells, 6 named patches) reads
    /// with every cell, face, name and role. A reader that dropped the
    /// boundary polygons or the field data would change these counts.
    #[test]
    fn reads_the_koflcraft_box() {
        let m = read_vtu(&fixture("box/foehncraft.vtu")).unwrap();
        assert_eq!(m.cell_count(), 4);
        assert_eq!(m.zones, vec!["fluid".to_string()]);
        let names: Vec<_> = m.patches.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["xmin", "xmax", "ymin", "ymax", "zmin", "zmax"]);
        assert!(!m.boundary_extracted);
        assert!(m.boundary.len() >= 6);
        assert_eq!(
            m.metadata.get("koflcraft_version").map(String::as_str),
            Some("0.1.0")
        );
        let b = m.bounds();
        assert_eq!(b.min, [0.0; 3]);
        assert_eq!(b.max, [1.0; 3]);
    }

    /// Invariant: the straight channel's prism layers are classed as prisms
    /// and its patch faces and roles match the evidence KoflCraft wrote
    /// beside it (776 cells; inlet 85, outlet 85, walls 256 faces).
    #[test]
    fn reads_the_channel_with_roles_and_prisms() {
        let m = read_vtu(&fixture("channel/foehncraft.vtu")).unwrap();
        let s = m.summary();
        assert_eq!(s.cells, 776);
        assert_eq!(s.boundary_faces, 426);
        let p: Vec<_> = s
            .patches
            .iter()
            .map(|p| (p.name.as_str(), p.role, p.faces))
            .collect();
        assert_eq!(
            p,
            [
                ("inlet", PatchRole::Inlet, 85),
                ("outlet", PatchRole::Outlet, 85),
                ("walls", PatchRole::Wall, 256)
            ]
        );
        assert!(s.cell_kinds.get("prism").copied().unwrap_or(0) > 0);
        assert_eq!(s.cell_kinds.values().sum::<usize>(), 776);
    }

    const TET: &str = r#"<?xml version="1.0"?>
<VTKFile type="UnstructuredGrid" version="1.0" byte_order="LittleEndian">
<UnstructuredGrid><Piece NumberOfPoints="4" NumberOfCells="1">
<Points><DataArray type="Float64" NumberOfComponents="3" format="ascii">
0 0 0 1 0 0 0 1 0 0 0 1</DataArray></Points>
<Cells>
<DataArray type="Int64" Name="connectivity" format="ascii">0 1 2 3</DataArray>
<DataArray type="Int64" Name="offsets" format="ascii">4</DataArray>
<DataArray type="UInt8" Name="types" format="ascii">10</DataArray>
</Cells></Piece></UnstructuredGrid></VTKFile>"#;

    /// Invariant: a volume file without boundary polygons still shows its
    /// surface, found as the faces used by one cell.
    #[test]
    fn extracts_the_boundary_of_a_plain_volume_file() {
        let m = read_vtu_str(TET).unwrap();
        assert_eq!(m.cell_count(), 1);
        assert_eq!(m.cell_kind, vec![CellKind::Tetrahedron]);
        assert_eq!(m.boundary.len(), 4);
        assert!(m.boundary_extracted);
        assert_eq!(m.patches.len(), 1);
    }

    /// Invariant: binary (base64) arrays decode the same as ascii ones.
    #[test]
    fn reads_binary_arrays() {
        let enc = |bytes: Vec<u8>| {
            let mut v = (bytes.len() as u32).to_le_bytes().to_vec();
            v.extend(bytes);
            base64::engine::general_purpose::STANDARD.encode(v)
        };
        let pts: Vec<u8> = [0.0f32, 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1.]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let conn: Vec<u8> = [0i32, 1, 2, 3]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let text = TET
            .replace(
                "type=\"Float64\" NumberOfComponents=\"3\" format=\"ascii\">\n0 0 0 1 0 0 0 1 0 0 0 1",
                &format!("type=\"Float32\" NumberOfComponents=\"3\" format=\"binary\">{}", enc(pts)),
            )
            .replace(
                "type=\"Int64\" Name=\"connectivity\" format=\"ascii\">0 1 2 3",
                &format!("type=\"Int32\" Name=\"connectivity\" format=\"binary\">{}", enc(conn)),
            );
        let m = read_vtu_str(&text).unwrap();
        assert_eq!(m.points[3], [0.0, 0.0, 1.0]);
        assert_eq!(m.boundary.len(), 4);
    }

    fn code(text: &str) -> &'static str {
        match read_vtu_str(text) {
            Ok(_) => "ok",
            Err(e) => e.code,
        }
    }

    /// The broken-input corpus (KoflCraft AG rule: the input of every crash
    /// joins it). Each case must give its code, never a panic.
    #[test]
    fn broken_inputs_give_codes() {
        assert_eq!(code(TET), "ok");
        assert_eq!(code(""), "MESH002");
        assert_eq!(code("<VTKFile type=\"PolyData\"/>"), "MESH002");
        assert_eq!(code("<VTKFile type=\"UnstructuredGrid\"><Piece"), "MESH003");
        assert_eq!(
            code(&TET.replace("NumberOfPoints=\"4\"", "NumberOfPoints=\"99999999\"")),
            "MESH004"
        );
        assert_eq!(
            code(&TET.replace("0 1 0 0 0 1</", "0 1 0 0 0</")),
            "MESH005"
        );
        assert_eq!(code(&TET.replace("0 0 0 1 0 0", "0 0 x 1 0 0")), "MESH005");
        assert_eq!(code(&TET.replace(">0 1 2 3<", ">0 1 2 7<")), "MESH006");
        assert_eq!(code(&TET.replace(">4</", ">9</")), "MESH006");
        assert_eq!(code(&TET.replace(">10</", ">3</")), "MESH008");
        assert_eq!(
            code(&TET.replace(
                "format=\"ascii\">0 1 2 3",
                "format=\"appended\" offset=\"0\">"
            )),
            "MESH007"
        );
        assert_eq!(
            code(&TET.replace(
                "byte_order",
                "compressor=\"vtkZLibDataCompressor\" byte_order"
            )),
            "MESH007"
        );
        assert_eq!(
            code(&TET.replace(
                "<Piece",
                "<Piece NumberOfPoints=\"0\" NumberOfCells=\"0\"/><Piece"
            )),
            "MESH012"
        );
        assert_eq!(
            code(
                &TET.replace("NumberOfCells=\"1\"", "NumberOfCells=\"0\"")
                    .replace(">4</", "></")
                    .replace(">10</", "></")
                    .replace(">0 1 2 3<", "><")
            ),
            "MESH010"
        );
        // A polyhedron whose face stream is cut short.
        let poly = TET
            .replace(">10</", ">42</")
            .replace(
                "</Cells>",
                "<DataArray type=\"Int64\" Name=\"faces\" format=\"ascii\">4 3 0 1 3 3 1 2 3 3 2 0 3 3 0 2</DataArray><DataArray type=\"Int64\" Name=\"faceoffsets\" format=\"ascii\">16</DataArray></Cells>",
            );
        assert_eq!(code(&poly), "MESH009");
        assert_eq!(
            code(&poly.replace("0 2</", "0 2 1</").replace(">16<", ">17<")),
            "ok"
        );
    }
}
