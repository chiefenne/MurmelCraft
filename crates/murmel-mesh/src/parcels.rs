//! Reader for parcel files: the Lagrangian parcels of a FoehnCraft spray
//! (`spray/parcels_<step>.vtkhdf`, FoehnCraft SP1, D240 to D246, viewed
//! as decided in D268), and any VTKHDF `PolyData` file of points.
//!
//! Written from the VTK documentation ("VTK File Formats", section
//! "VTKHDF File Format", `PolyData`) and FoehnCraft's writer of the
//! format `foehncraft.parcels` version 1: a `PolyData` group `/VTKHDF`
//! with `NumberOfPoints`, `Points` (n × 3), one vertex cell per point and
//! the parcel fields as `PointData` (`velocity` n × 3, `diameter`,
//! `droplets`, `age`, `parcel_id`; later stages add `temperature` and
//! `injector`). The root attributes name the format and its version, the
//! case, the step, the simulated time, the liquid and the number of
//! parcels. A file of several partitions (`NumberOfPoints` with one
//! entry per partition) reads as one set of points.
//!
//! Every field of `PointData` is read, whatever its name, so a later
//! stage's fields show without a change here; vectors written as three
//! scalars (`velocity_x`, …) are joined as in the result files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::MeshError;
use crate::field::{Field, Location, group_vectors};
use crate::hdf5::{Dtype, H5, Object};

fn bad(message: impl Into<String>) -> MeshError {
    MeshError::new("MESH018", message)
}

/// The format FoehnCraft names in the root attribute `format`.
pub const FORMAT: &str = "foehncraft.parcels";

/// The parcels of one file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parcels {
    /// The file read.
    pub path: PathBuf,
    /// BLAKE3 hash of the file, hex.
    pub blake3: String,
    /// Each parcel's position.
    pub points: Vec<[f64; 3]>,
    /// The point data, one value or one 3-vector per parcel, with the
    /// unit where FoehnCraft's format names it.
    pub fields: Vec<Field>,
    /// The simulated time and the time step of the file.
    pub time: Option<f64>,
    pub step: Option<u64>,
    /// The root attributes as text (`format`, `format_version`, `case`,
    /// `liquid`, `units`, `parcels`, …).
    pub attrs: BTreeMap<String, String>,
}

impl Parcels {
    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// The names of the fields, for messages that list what exists.
    pub fn field_names(&self) -> Vec<String> {
        self.fields.iter().map(|f| f.name.clone()).collect()
    }

    /// Whether the file says it is FoehnCraft's parcel format.
    pub fn is_foehncraft(&self) -> bool {
        self.attrs.get("format").is_some_and(|f| f == FORMAT)
    }
}

/// The units of the fields of FoehnCraft's parcel format (its writer,
/// `foehncraft-particles` `Spray::write`, and the spray specification):
/// SI units, droplets counted.
fn foehncraft_unit(name: &str) -> Option<&'static str> {
    Some(match name {
        "velocity" => "m/s",
        "diameter" => "m",
        "age" => "s",
        "temperature" => "K",
        "mass" => "kg",
        "droplets" => "droplets",
        _ => return None,
    })
}

/// Read the parcels of a VTKHDF `PolyData` file.
pub fn read_parcels(path: &Path) -> Result<Parcels, MeshError> {
    if !path.is_file() {
        return Err(MeshError::new(
            "MESH001",
            format!("{} is not a readable file", path.display()),
        ));
    }
    let h = H5::open(path)?;
    let mut p = read_parcels_h5(&h)?;
    p.path = path.to_path_buf();
    let stop = std::sync::atomic::AtomicBool::new(false);
    p.blake3 = crate::FileHash::Blake3
        .of_file(path, &stop)
        .map_err(|e| MeshError::new("MESH001", format!("cannot read {}: {e}", path.display())))?
        .unwrap_or_default();
    Ok(p)
}

/// Read the parcels of an open HDF5 file.
pub fn read_parcels_h5(h: &H5) -> Result<Parcels, MeshError> {
    let vtk = h
        .path("/VTKHDF")
        .map_err(|_| bad("the file has no /VTKHDF group: it is HDF5 but not VTKHDF"))?;
    let ty = h
        .attr(&vtk, "Type")?
        .and_then(|v| v.text())
        .unwrap_or_default();
    if ty != "PolyData" {
        return Err(bad(format!(
            "VTKHDF type {ty:?}: a parcel file is PolyData (spray/parcels_<step>.vtkhdf); open a result with mesh.open"
        )));
    }
    let n_parts = ints(h, &vtk, "NumberOfPoints")?;
    let mut n = 0usize;
    for v in n_parts {
        n += usize::try_from(v).map_err(|_| bad(format!("NumberOfPoints {v} is negative")))?;
    }
    let pts = dataset(h, &vtk, "Points")?;
    // The count must fit the dataset (and with it the file) before
    // anything is allocated by it.
    if pts.1.as_slice() != [n as u64, 3] && !(n == 0 && pts.1.iter().product::<u64>() == 0) {
        return Err(MeshError::new(
            "MESH005",
            format!(
                "Points has shape {:?} for NumberOfPoints {n}: expected [{n}, 3]",
                pts.1
            ),
        ));
    }
    let flat = if n == 0 {
        Vec::new()
    } else {
        h.read_f64(&pts.0)?
    };
    if flat.len() != 3 * n {
        return Err(MeshError::new(
            "MESH005",
            format!("Points has {} values for {n} points", flat.len()),
        ));
    }
    let points: Vec<[f64; 3]> = flat.as_chunks::<3>().0.to_vec();
    let mut fields = point_data(h, &vtk, n)?;
    let mut attrs = BTreeMap::new();
    let root = h.root()?;
    let mut time = None;
    let mut step = None;
    for (k, v) in h.attrs(&root)? {
        match k.as_str() {
            "time" => time = v.number(),
            "step" => step = v.number().filter(|s| *s >= 0.0).map(|s| s as u64),
            _ => {}
        }
        if let Some(t) = v.text() {
            attrs.insert(k, t);
        }
    }
    if attrs.get("format").is_some_and(|f| f == FORMAT) {
        for f in &mut fields {
            f.unit = foehncraft_unit(&f.name).map(str::to_string);
        }
    }
    Ok(Parcels {
        path: PathBuf::new(),
        blake3: String::new(),
        points,
        fields,
        time,
        step,
        attrs,
    })
}

fn dataset(
    h: &H5,
    obj: &Object,
    name: &str,
) -> Result<(crate::hdf5::Dataset, Vec<u64>), MeshError> {
    let o = h
        .child(obj, name)?
        .ok_or_else(|| bad(format!("no {name} dataset")))?;
    let ds = h.dataset(&o)?;
    let dims = ds.dims.clone();
    Ok((ds, dims))
}

fn ints(h: &H5, obj: &Object, name: &str) -> Result<Vec<i64>, MeshError> {
    let (ds, _) = dataset(h, obj, name)?;
    h.read_i64(&ds)
}

/// The numeric datasets of `PointData` with one row per point: scalars,
/// 3-vectors, and other tuples as scalars `name_0`, `name_1`, ….
fn point_data(h: &H5, vtk: &Object, n: usize) -> Result<Vec<Field>, MeshError> {
    let Some(g) = h.child(vtk, "PointData")? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for link in h.links(&g)? {
        let Some(o) = h.child(&g, &link.name)? else {
            continue;
        };
        if h.is_group(&o) {
            continue;
        }
        let ds = h.dataset(&o)?;
        if !matches!(ds.dtype, Dtype::Float { .. } | Dtype::Int { .. }) {
            continue;
        }
        let comps = match ds.dims.as_slice() {
            [rows] if *rows as usize == n => 1,
            [rows, k] if *rows as usize == n && (1..=9).contains(k) => *k as usize,
            _ => {
                return Err(MeshError::new(
                    "MESH005",
                    format!(
                        "PointData/{} has shape {:?} for {n} parcels",
                        link.name, ds.dims
                    ),
                ));
            }
        };
        let values = if n == 0 { Vec::new() } else { h.read_f64(&ds)? };
        if values.len() != n * comps {
            return Err(MeshError::new(
                "MESH005",
                format!(
                    "PointData/{} has {} values for {n} parcels",
                    link.name,
                    values.len()
                ),
            ));
        }
        if comps == 1 || comps == 3 {
            out.push(Field {
                name: link.name.clone(),
                unit: None,
                location: Location::Point,
                components: comps,
                values,
            });
        } else {
            for k in 0..comps {
                out.push(Field {
                    name: format!("{}_{k}", link.name),
                    unit: None,
                    location: Location::Point,
                    components: 1,
                    values: values.iter().skip(k).step_by(comps).copied().collect(),
                });
            }
        }
    }
    Ok(group_vectors(out))
}

/// Writes a parcel file in FoehnCraft's layout (format
/// `foehncraft.parcels`, version 1) with this crate's HDF5 writer: the
/// test fixtures and `murmel parcels make` use it, so the viewer can be
/// tried without the solver.
pub fn write_parcels(
    path: &Path,
    points: &[[f64; 3]],
    fields: &[Field],
    time: f64,
    step: u64,
    case: &str,
) -> std::io::Result<()> {
    use crate::h5write::{AttrValue, H5Writer, Link};
    let n = points.len();
    let mut w = H5Writer::create(path)?;
    let flat: Vec<f64> = points.iter().flatten().copied().collect();
    let points_ds = w.dataset(&[n as u64, 3], &flat)?;
    let np = w.dataset(&[1], &[n as i64])?;
    let mut cells = Vec::new();
    for (name, count) in [
        ("Vertices", n),
        ("Lines", 0),
        ("Strips", 0),
        ("Polygons", 0),
    ] {
        let offsets: Vec<i64> = (0..=count as i64).collect();
        let conn: Vec<i64> = (0..count as i64).collect();
        let a = w.dataset(&[1], &[count as i64])?;
        let b = w.dataset(&[1], &[conn.len() as i64])?;
        let c = w.dataset(&[offsets.len() as u64], &offsets)?;
        let d = w.dataset(&[conn.len() as u64], &conn)?;
        let g = w.group(
            &[
                Link::Hard("NumberOfCells".into(), a),
                Link::Hard("NumberOfConnectivityIds".into(), b),
                Link::Hard("Offsets".into(), c),
                Link::Hard("Connectivity".into(), d),
            ],
            &[],
        )?;
        cells.push(Link::Hard(name.into(), g));
    }
    let mut pd = Vec::new();
    for f in fields {
        let dims: Vec<u64> = if f.components == 1 {
            vec![n as u64]
        } else {
            vec![n as u64, f.components as u64]
        };
        let at = w.dataset(&dims, &f.values)?;
        pd.push(Link::Hard(f.name.clone(), at));
    }
    let pd = w.group(&pd, &[])?;
    let cd = w.group(&[], &[])?;
    let fd = w.group(&[], &[])?;
    let mut links = vec![
        Link::Hard("NumberOfPoints".into(), np),
        Link::Hard("Points".into(), points_ds),
    ];
    links.extend(cells);
    links.extend([
        Link::Hard("PointData".into(), pd),
        Link::Hard("CellData".into(), cd),
        Link::Hard("FieldData".into(), fd),
    ]);
    let vtk = w.group(
        &links,
        &[
            ("Type", AttrValue::Text("PolyData".into())),
            ("Version", AttrValue::Ints(vec![2, 3])),
        ],
    )?;
    let root = w.group(
        &[Link::Hard("VTKHDF".into(), vtk)],
        &[
            ("format", AttrValue::Text(FORMAT.into())),
            ("format_version", AttrValue::Int(1)),
            ("case", AttrValue::Text(case.into())),
            ("step", AttrValue::Int(step as i64)),
            ("time", AttrValue::Float(time)),
            ("parcels", AttrValue::Int(n as i64)),
        ],
    )?;
    w.finish(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("murmel-parcels-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d.join(name)
    }

    fn scalar(name: &str, values: Vec<f64>) -> Field {
        Field {
            name: name.into(),
            unit: None,
            location: Location::Point,
            components: 1,
            values,
        }
    }

    /// Invariant: a parcel file in FoehnCraft's layout reads back with
    /// its points, every point field (the vector whole), the time, the
    /// step and FoehnCraft's units. A change in the PolyData layout or
    /// the root attributes breaks it; no other test reads parcels.
    #[test]
    fn reads_foehncraft_parcels() {
        let path = tmp("p.vtkhdf");
        let points = vec![[0.0, 1.0, 2.0], [3.0, 4.0, 5.0], [-1.0, 0.5, 0.25]];
        let fields = vec![
            Field {
                name: "velocity".into(),
                unit: None,
                location: Location::Point,
                components: 3,
                values: (0..9).map(f64::from).collect(),
            },
            scalar("diameter", vec![1e-4, 2e-4, 1.5e-4]),
            scalar("parcel_id", vec![7.0, 8.0, 9.0]),
        ];
        write_parcels(&path, &points, &fields, 0.25, 50, "spray_duct").unwrap();
        let p = read_parcels(&path).unwrap();
        assert_eq!(p.points, points);
        assert_eq!(p.time, Some(0.25));
        assert_eq!(p.step, Some(50));
        assert!(p.is_foehncraft());
        assert_eq!(p.attrs.get("case").map(String::as_str), Some("spray_duct"));
        let v = p.field("velocity").unwrap();
        assert_eq!((v.components, v.values[4]), (3, 4.0));
        assert_eq!(v.unit.as_deref(), Some("m/s"));
        assert_eq!(p.field("diameter").unwrap().unit.as_deref(), Some("m"));
        assert_eq!(p.field("parcel_id").unwrap().unit, None);
        assert_eq!(p.blake3.len(), 64);
    }

    /// Invariant: a file with no parcels (the first steps of a run, before
    /// the injector starts) reads as an empty set, not as an error. A
    /// reader that insists on a [n, 3] dataset of n > 0 breaks it.
    #[test]
    fn reads_an_empty_file() {
        let path = tmp("empty.vtkhdf");
        write_parcels(&path, &[], &[scalar("diameter", vec![])], 0.0, 0, "c").unwrap();
        let p = read_parcels(&path).unwrap();
        assert!(p.is_empty());
        assert_eq!(p.field("diameter").map(Field::len), Some(0));
    }

    /// Invariant: a result file (UnstructuredGrid) and a file that is not
    /// HDF5 are refused with a code, not read as parcels. Breaking the
    /// type check reads a mesh's points as parcels.
    #[test]
    fn refuses_other_files() {
        let path = tmp("not.vtkhdf");
        std::fs::write(&path, b"not hdf5 at all").unwrap();
        assert!(read_parcels(&path).is_err());
        let e = read_parcels(&tmp("missing.vtkhdf")).unwrap_err();
        assert_eq!(e.code, "MESH001");
        let grid = tmp("grid.vtkhdf");
        let mesh = crate::synth::honeycomb(2);
        crate::blocks::write_blocked(&grid, &mesh, &crate::Results::default(), 1 << 14).unwrap();
        assert_eq!(read_parcels(&grid).unwrap_err().code, "MESH018");
    }
}
