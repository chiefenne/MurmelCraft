//! Reader for device files: the injector and nozzle devices of a
//! FoehnCraft spray (`spray/devices.vtu`, FoehnCraft D343 to D345, plan
//! stage ID2m), drawn under the parcels.
//!
//! Written from the VTK documentation ("VTK File Formats", section "XML
//! File Formats", `UnstructuredGrid`) and FoehnCraft's writer
//! (`foehncraft-flow` `spray::write_devices`, FoehnCraft pull request
//! #343): one `Piece` of ASCII arrays, the points in metres in the case's
//! coordinates, the cells
//!
//! - triangles (VTK type 5) of the placed surface of each device,
//! - one line (type 3) per hole from its exit along its axis,
//! - one closed polyline (type 4) per injector that names a hole: the
//!   outline of the cone of its `half_angle` at the end of that line,
//!
//! and the cell fields `device` (index in the case's order), `part`
//! (0 surface, 1 hole axis, 2 cone), `hole` (index in the device) and
//! `injector` (index in the case's expanded injector list), `-1` where
//! they do not apply. The file is written once at the start of a run (the
//! devices do not move) and listed in `series.json` under
//! `"static": {"devices": "spray/devices.vtu"}`.
//!
//! Without a `part` field, faces (triangle, polygon, quad) are the
//! surface and lines and polylines are hole axes, so any small ASCII
//! `.vtu` of faces and lines draws as a device.

use std::path::{Path, PathBuf};

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

use crate::MeshError;
use crate::hash::FileHash;

fn bad(message: impl Into<String>) -> MeshError {
    MeshError::new("MESH020", message)
}

/// The largest device file read, in bytes (the file is read whole; a
/// device of a few hundred thousand triangles stays far below).
pub const MAX_BYTES: u64 = 256 << 20;

/// What a cell of a device file draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// A face of the placed surface.
    Surface,
    /// A hole's axis, from its exit into the chamber.
    Axis,
    /// The outline of an injector's cone.
    Cone,
}

impl Part {
    pub fn key(self) -> &'static str {
        match self {
            Part::Surface => "surface",
            Part::Axis => "axis",
            Part::Cone => "cone",
        }
    }
}

/// One cell: its points (indices into [`Devices::points`]) and what it
/// is. A face is a triangle fan of its points.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceCell {
    pub part: Part,
    /// Index of the device, of the hole in it and of the injector; `-1`
    /// where the file gives none.
    pub device: i64,
    pub hole: i64,
    pub injector: i64,
    pub points: std::ops::Range<usize>,
}

/// The devices of one file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Devices {
    pub path: PathBuf,
    /// BLAKE3 hash of the file, hex.
    pub blake3: String,
    /// The points of the cells, metres.
    pub points: Vec<[f64; 3]>,
    /// The cells in file order; the points of cell `k` are
    /// `points[cells[k].points]`.
    pub cells: Vec<DeviceCell>,
}

impl Devices {
    /// The number of devices (the largest `device` index plus one, one
    /// when the file names none).
    pub fn count(&self) -> usize {
        if self.cells.is_empty() {
            return 0;
        }
        self.cells
            .iter()
            .map(|c| c.device)
            .max()
            .map_or(1, |m| usize::try_from(m + 1).unwrap_or(0).max(1))
    }

    /// How many cells of `part` the file has.
    pub fn cells_of(&self, part: Part) -> usize {
        self.cells.iter().filter(|c| c.part == part).count()
    }

    /// The triangles of the surface (fans of its faces).
    pub fn triangles(&self) -> usize {
        self.cells
            .iter()
            .filter(|c| c.part == Part::Surface)
            .map(|c| c.points.len().saturating_sub(2))
            .sum()
    }

    /// The holes drawn (distinct device and hole pairs of the axes).
    pub fn holes(&self) -> usize {
        let mut h: Vec<(i64, i64)> = self
            .cells
            .iter()
            .filter(|c| c.part == Part::Axis)
            .map(|c| (c.device, c.hole))
            .collect();
        h.sort_unstable();
        h.dedup();
        h.len()
    }

    /// The box of every point, `None` without points.
    pub fn bounds(&self) -> Option<crate::Bounds> {
        let mut b = crate::Bounds::empty();
        for p in &self.points {
            b.add(*p);
        }
        (!b.is_empty()).then_some(b)
    }
}

/// Read a device file (`spray/devices.vtu`).
pub fn read_devices(path: &Path) -> Result<Devices, MeshError> {
    let len = std::fs::metadata(path)
        .map_err(|e| MeshError::new("MESH001", format!("cannot read {}: {e}", path.display())))?
        .len();
    if len > MAX_BYTES {
        return Err(bad(format!(
            "{} has {len} bytes, more than the {MAX_BYTES} a device file may have",
            path.display()
        )));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| MeshError::new("MESH001", format!("cannot read {}: {e}", path.display())))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| bad(format!("{} is not text (an ASCII .vtu)", path.display())))?;
    let mut d = read_devices_str(text)?;
    d.path = path.to_path_buf();
    d.blake3 = FileHash::Blake3.of_bytes(&bytes);
    Ok(d)
}

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Points,
    Cells,
    CellData,
    Other,
}

fn attr(e: &BytesStart<'_>, key: &str) -> Result<Option<String>, MeshError> {
    match e.try_get_attribute(key) {
        Ok(Some(a)) => a
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map(|v| Some(v.into_owned()))
            .map_err(|x| bad(format!("bad attribute {key}: {x}"))),
        Ok(None) => Ok(None),
        Err(x) => Err(bad(format!("bad attribute {key}: {x}"))),
    }
}

fn count(e: &BytesStart<'_>, key: &str) -> Result<usize, MeshError> {
    let v = attr(e, key)?.ok_or_else(|| bad(format!("Piece has no {key}")))?;
    v.trim()
        .parse()
        .map_err(|_| bad(format!("{key}=\"{v}\" is not a count")))
}

fn numbers(text: &str, name: &str, at_most: usize) -> Result<Vec<f64>, MeshError> {
    let mut out = Vec::new();
    for w in text.split_ascii_whitespace() {
        if out.len() >= at_most {
            return Err(bad(format!(
                "{name} has more than the {at_most} values the Piece declares"
            )));
        }
        let x: f64 = w
            .parse()
            .map_err(|_| bad(format!("{name} holds {w:?}, not a number")))?;
        out.push(x);
    }
    Ok(out)
}

fn index(x: f64, name: &str) -> Result<i64, MeshError> {
    if x.fract() != 0.0 || !x.is_finite() || x.abs() > 9.0e15 {
        return Err(bad(format!("{name} holds {x}, not a whole number")));
    }
    Ok(x as i64)
}

/// Read a device file from its text.
pub fn read_devices_str(text: &str) -> Result<Devices, MeshError> {
    let len = text.len();
    let mut reader = Reader::from_str(text);
    let mut section = Section::Other;
    let mut piece: Option<(usize, usize)> = None;
    let mut saw_file = false;
    let mut points: Option<Vec<f64>> = None;
    let mut arrays: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    let mut fields: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    loop {
        let ev = reader
            .read_event()
            .map_err(|e| bad(format!("the XML is broken: {e}")))?;
        match ev {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == "VTKFile" => {
                saw_file = true;
                let ty = attr(&e, "type")?.unwrap_or_default();
                if ty != "UnstructuredGrid" {
                    return Err(bad(format!(
                        "VTKFile type is \"{ty}\", not UnstructuredGrid"
                    )));
                }
            }
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == "Piece" => {
                if piece.is_some() {
                    return Err(bad("the file has more than one Piece"));
                }
                let (n, c) = (count(&e, "NumberOfPoints")?, count(&e, "NumberOfCells")?);
                // Each ASCII value takes two bytes at least (a digit and
                // a space): three per point, three per cell (offset,
                // type and one point).
                if n.saturating_mul(6) > len || c.saturating_mul(6) > len {
                    return Err(bad(format!(
                        "the Piece declares {n} points and {c} cells, more than {len} bytes can hold"
                    )));
                }
                piece = Some((n, c));
            }
            Event::Start(e) => match e.local_name().as_ref() {
                "Points" => section = Section::Points,
                "Cells" => section = Section::Cells,
                "CellData" => section = Section::CellData,
                "PointData" | "FieldData" => section = Section::Other,
                "AppendedData" => {
                    return Err(bad(
                        "appended data is not read; FoehnCraft writes devices.vtu as ASCII",
                    ));
                }
                "DataArray" => {
                    let Some((n, c)) = piece else {
                        return Err(bad("a DataArray before the Piece"));
                    };
                    let format = attr(&e, "format")?.unwrap_or_else(|| "ascii".into());
                    let name = attr(&e, "Name")?.unwrap_or_default();
                    let end = e.to_end().into_owned();
                    let body = reader
                        .read_text(end.name())
                        .map_err(|x| bad(format!("the XML is broken in {name}: {x}")))?;
                    if format != "ascii" {
                        return Err(bad(format!(
                            "{name} is {format}; device files are read as ASCII, as FoehnCraft writes them"
                        )));
                    }
                    let body: &str = body.as_ref();
                    match section {
                        Section::Points if points.is_none() => {
                            points = Some(numbers(body, "Points", 3 * n)?);
                        }
                        Section::Cells => {
                            let most = if name == "connectivity" { len } else { c };
                            arrays.insert(name.clone(), numbers(body, &name, most)?);
                        }
                        Section::CellData => {
                            fields.insert(name.clone(), numbers(body, &name, c)?);
                        }
                        _ => {}
                    }
                }
                _ => {}
            },
            Event::End(e) => {
                if matches!(
                    e.local_name().as_ref(),
                    "Points" | "Cells" | "CellData" | "PointData" | "FieldData"
                ) {
                    section = Section::Other;
                }
            }
            _ => {}
        }
    }
    if !saw_file {
        return Err(bad("no VTKFile element"));
    }
    let (n, c) = piece.ok_or_else(|| bad("no Piece element"))?;
    let flat = points.unwrap_or_default();
    if flat.len() != 3 * n {
        return Err(bad(format!(
            "Points holds {} values, not the 3 × {n} the Piece declares",
            flat.len()
        )));
    }
    if flat.iter().any(|x| !x.is_finite()) {
        return Err(bad("a point is not finite"));
    }
    let pts: Vec<[f64; 3]> = flat.as_chunks::<3>().0.to_vec();
    let get = |k: &str| -> Result<&Vec<f64>, MeshError> {
        arrays
            .get(k)
            .ok_or_else(|| bad(format!("the Cells have no {k} array")))
    };
    let (conn, offs, types) = (get("connectivity")?, get("offsets")?, get("types")?);
    if offs.len() != c || types.len() != c {
        return Err(bad(format!(
            "offsets has {} and types {} values, not the {c} cells the Piece declares",
            offs.len(),
            types.len()
        )));
    }
    let field = |k: &str| -> Result<Option<&Vec<f64>>, MeshError> {
        match fields.get(k) {
            Some(v) if v.len() != c => Err(bad(format!(
                "the cell field {k} has {} values, not one per cell ({c})",
                v.len()
            ))),
            v => Ok(v),
        }
    };
    let (fdev, fpart, fhole, finj) = (
        field("device")?,
        field("part")?,
        field("hole")?,
        field("injector")?,
    );
    let at = |f: Option<&Vec<f64>>, k: usize, name: &str| -> Result<i64, MeshError> {
        f.map_or(Ok(-1), |v| index(v[k], name))
    };
    let ids: Vec<usize> = conn
        .iter()
        .map(|&x| {
            let i = index(x, "connectivity")?;
            usize::try_from(i)
                .ok()
                .filter(|&i| i < n)
                .ok_or_else(|| bad(format!("connectivity names point {i}, outside 0 to {n}")))
        })
        .collect::<Result<_, _>>()?;
    let mut cells = Vec::with_capacity(c);
    let mut start = 0usize;
    for k in 0..c {
        let end = usize::try_from(index(offs[k], "offsets")?)
            .ok()
            .filter(|&e| e >= start && e <= ids.len())
            .ok_or_else(|| bad(format!("offset {} of cell {k} is out of order", offs[k])))?;
        let ty = index(types[k], "types")?;
        let pts_n = end - start;
        let shape = match ty {
            5 | 7 | 9 if pts_n >= 3 => Part::Surface,
            3 | 4 if pts_n >= 2 => Part::Axis,
            5 | 7 | 9 | 3 | 4 => {
                return Err(bad(format!("cell {k} of type {ty} has {pts_n} points")));
            }
            _ => {
                return Err(bad(format!(
                    "cell {k} has VTK type {ty}; device files hold triangles (5), polygons (7), quads (9), lines (3) and polylines (4)"
                )));
            }
        };
        let part = match (at(fpart, k, "part")?, shape) {
            (-1, s) => s,
            (0, Part::Surface) => Part::Surface,
            (1, Part::Axis) => Part::Axis,
            (2, Part::Axis) => Part::Cone,
            (p, _) => {
                return Err(bad(format!(
                    "cell {k} of type {ty} has part {p}; part is 0 for faces, 1 or 2 for lines"
                )));
            }
        };
        // The points of a cell are copied in order: `points` of a cell
        // is the run of its connectivity.
        cells.push(DeviceCell {
            part,
            device: at(fdev, k, "device")?,
            hole: at(fhole, k, "hole")?,
            injector: at(finj, k, "injector")?,
            points: start..end,
        });
        start = end;
    }
    let points: Vec<[f64; 3]> = ids.iter().map(|&i| pts[i]).collect();
    Ok(Devices {
        path: PathBuf::new(),
        blake3: String::new(),
        points,
        cells,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// A device as FoehnCraft's `write_devices` writes it: two triangles
    /// of a surface, one hole axis and the cone of its injector (a
    /// closed polyline of 5 points).
    pub(crate) const FILE: &str = r#"<?xml version="1.0"?>
<VTKFile type="UnstructuredGrid" version="1.0" byte_order="LittleEndian">
<UnstructuredGrid>
<Piece NumberOfPoints="13" NumberOfCells="4">
<Points>
<DataArray type="Float64" NumberOfComponents="3" format="ascii">
0e0 0e0 0e0
1e-3 0e0 0e0
0e0 1e-3 0e0
1e-3 0e0 0e0
1e-3 1e-3 0e0
0e0 1e-3 0e0
0e0 0e0 0e0
0e0 0e0 1e-3
1e-4 0e0 1e-3
0e0 1e-4 1e-3
-1e-4 0e0 1e-3
0e0 -1e-4 1e-3
1e-4 0e0 1e-3
</DataArray>
</Points>
<Cells>
<DataArray type="Int64" Name="connectivity" format="ascii">
0 1 2 3 4 5 6 7 8 9 10 11 12
</DataArray>
<DataArray type="Int64" Name="offsets" format="ascii">
3 6 8 13
</DataArray>
<DataArray type="UInt8" Name="types" format="ascii">
5 5 3 4
</DataArray>
</Cells>
<CellData>
<DataArray type="Int32" Name="device" format="ascii">
0 0 0 0
</DataArray>
<DataArray type="Int32" Name="part" format="ascii">
0 0 1 2
</DataArray>
<DataArray type="Int32" Name="hole" format="ascii">
-1 -1 0 0
</DataArray>
<DataArray type="Int32" Name="injector" format="ascii">
-1 -1 -1 0
</DataArray>
</CellData>
</Piece>
</UnstructuredGrid>
</VTKFile>
"#;

    /// Invariant: the cells of FoehnCraft's devices.vtu read with their
    /// part (surface, hole axis, cone), device, hole and injector and
    /// their points in order. Breaks if the cone read as a hole axis (it
    /// would draw as a second hole) or a face lost a point.
    #[test]
    fn reads_foehncraft_devices() {
        let d = read_devices_str(FILE).unwrap();
        assert_eq!(d.cells.len(), 4);
        assert_eq!(d.cells_of(Part::Surface), 2);
        assert_eq!(d.cells_of(Part::Axis), 1);
        assert_eq!(d.cells_of(Part::Cone), 1);
        assert_eq!(d.triangles(), 2);
        assert_eq!(d.holes(), 1);
        assert_eq!(d.count(), 1);
        assert_eq!(d.cells[3].injector, 0);
        assert_eq!(d.cells[2].hole, 0);
        assert_eq!(d.cells[0].injector, -1);
        assert_eq!(d.points[d.cells[2].points.end - 1], [0.0, 0.0, 1e-3]);
        assert_eq!(d.cells[3].points.len(), 5);
        let b = d.bounds().unwrap();
        assert_eq!(b.max, [1e-3, 1e-3, 1e-3]);
    }

    /// Invariant: the file FoehnCraft's `spray.place` wrote for the
    /// five-hole nozzle of its gallery 341 (test-fixtures/spray-device)
    /// reads as 5 hole axes and 5 cones of one device, each cone a ring
    /// of 37 points about the end of its axis. Breaks if FoehnCraft's
    /// real output stopped reading (the synthetic file above is ours).
    #[test]
    fn reads_the_foehncraft_fixture() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/spray-device/devices.vtu");
        let d = read_devices(&path).unwrap();
        assert_eq!(d.count(), 1);
        assert_eq!(d.holes(), 5);
        assert_eq!(d.cells_of(Part::Cone), 5);
        assert_eq!(d.triangles(), 0);
        assert!(
            d.cells
                .iter()
                .filter(|c| c.part == Part::Cone)
                .all(|c| c.points.len() == 37)
        );
        assert_eq!(d.blake3.len(), 64);
        // The nozzle is mounted at (0.06, 0.1, 0.05) and points down.
        let b = d.bounds().unwrap();
        assert!(b.max[1] < 0.1 && b.min[1] > 0.09, "{b:?}");
    }

    /// Invariant: without a `part` field faces are the surface and lines
    /// hole axes; a part that does not fit the cell's shape is refused.
    /// Breaks if any small .vtu of faces and lines stopped drawing.
    #[test]
    fn part_from_shape_and_refused_mismatch() {
        let plain = FILE.replace(
            "<DataArray type=\"Int32\" Name=\"part\" format=\"ascii\">\n0 0 1 2\n</DataArray>\n",
            "",
        );
        let d = read_devices_str(&plain).unwrap();
        assert_eq!(d.cells_of(Part::Axis), 2);
        assert_eq!(d.cells_of(Part::Cone), 0);
        let wrong = FILE.replace("0 0 1 2", "1 0 1 2");
        assert!(
            read_devices_str(&wrong)
                .unwrap_err()
                .message
                .contains("part 1")
        );
    }

    /// Invariant: broken files are refused with MESH020 and a reason,
    /// never read as fewer cells: a point index out of range, offsets out
    /// of order, a field of the wrong length, another cell type, binary
    /// arrays, counts the file cannot hold, a cut-off file. Breaks if a
    /// corrupt file allocated by its declared counts or drew garbage.
    #[test]
    fn refusals() {
        let e = |t: &str| read_devices_str(t).unwrap_err();
        let cases = [
            FILE.replace(
                "0 1 2 3 4 5 6 7 8 9 10 11 12",
                "0 1 2 3 4 5 6 7 8 9 10 11 99",
            ),
            FILE.replace("3 6 8 13", "3 2 8 13"),
            FILE.replace("-1 -1 0 0\n", "-1 -1 0\n"),
            FILE.replace("5 5 3 4", "5 5 3 10"),
            FILE.replace(
                "Name=\"offsets\" format=\"ascii\"",
                "Name=\"offsets\" format=\"binary\"",
            ),
            FILE.replace("NumberOfPoints=\"13\"", "NumberOfPoints=\"99999999\""),
            FILE[..FILE.len() / 2].to_string(),
            FILE.replace("UnstructuredGrid\" version", "PolyData\" version"),
        ];
        for (k, t) in cases.iter().enumerate() {
            let x = e(t);
            assert_eq!(x.code, "MESH020", "case {k}: {}", x.message);
        }
        assert!(e(&cases[0]).message.contains("point 99"));
        assert!(e(&cases[3]).message.contains("type 10"));
        assert!(e(&cases[4]).message.contains("binary"));
    }
}
