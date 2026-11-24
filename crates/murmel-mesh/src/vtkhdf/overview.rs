//! The overview of a blocked VTKHDF file (`docs/blocked-vtkhdf.md`): its
//! boundary, the fields on it and its block index, without reading the
//! volume cells. A mesh beyond the memory budget opens this way (the
//! maintainer's cards of 2026-11-24: an overview plus regions beyond the
//! budget; the overview is the surface, the block ranges and a preview
//! grid).
//!
//! The surface is a mesh of boundary faces only, as a file without volume
//! cells gives. Each cell field of the volume is shown on it as the value
//! of each face's owner cell (`OwnerCell`), read a run of nearby rows at a
//! time, so a field of 100 M cells is not read whole.

use std::collections::BTreeMap;
use std::path::Path;

use super::{
    Builder, Parts, as_fields, bad, blocks_of, components, count, floats, ints, names_of,
    optional_ints, read_patches, text_attrs,
};
use crate::field::{Field, Location, Results};
use crate::hdf5::{Dataset, Dtype, H5, Object};
use crate::{Bounds, Mesh, MeshError};

/// The cell datasets of the volume that are not fields.
pub(super) const CELL_RESERVED: [&str; 4] =
    ["zone", "vtkGhostType", "GlobalCellIds", "OriginalCellIds"];
/// The point datasets of the volume that are not fields.
pub(super) const POINT_RESERVED: [&str; 2] = ["vtkGhostType", "GlobalPointIds"];

/// Rows of a dataset this close to the row before are read with it.
const NEAR: usize = 4096;

/// What an overview knows of the volume without reading its cells.
#[derive(Clone, Debug, PartialEq)]
pub struct Overview {
    /// The volume cells of the file (owned cells; ghosts are copies).
    pub cells: u64,
    /// What reading the file whole would build.
    pub size: Size,
    /// The blocks, in file order.
    pub blocks: Vec<Block>,
    /// The cell and point fields of the volume.
    pub fields: Vec<VolumeField>,
}

/// A block of the volume, from the block index (§ 6).
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Its owned cells.
    pub cells: u64,
    /// The box of the points of its owned cells.
    pub bounds: Bounds,
    /// The range of each of [`Overview::fields`] over its owned cells or
    /// points (of the magnitude for a vector); `None` when the file has
    /// no range for that field.
    pub ranges: Vec<Option<[f64; 2]>>,
}

/// A field of the volume.
#[derive(Clone, Debug, PartialEq)]
pub struct VolumeField {
    pub name: String,
    /// [`Location::Cell`] or [`Location::Point`].
    pub location: Location,
    pub components: usize,
    /// Whether the overview's surface shows it (a cell field, as the
    /// value of each face's owner cell); point fields wait for a region.
    pub on_surface: bool,
}

impl Overview {
    /// The range of field `name` over all blocks, from the block index.
    pub fn range(&self, name: &str) -> Option<[f64; 2]> {
        let k = self.fields.iter().position(|f| f.name == name)?;
        self.blocks
            .iter()
            .filter_map(|b| b.ranges.get(k).copied().flatten())
            .filter(|r| r[0] <= r[1])
            .reduce(|a, b| [a[0].min(b[0]), a[1].max(b[1])])
    }
}

/// The memory a whole read of a VTKHDF file peaks at, per volume cell:
/// measured on blocked files of 1 M and 4 M polyhedral cells (the
/// honeycomb of `murmel bench make`), which peaked at 1,500 to 1,540
/// bytes per cell while opening (the read, the face linking and the
/// surface). Cells with fewer faces (hexahedra, wedges) take less.
pub const OPEN_BYTES_PER_CELL: u64 = 1_500;

/// What reading a VTKHDF file whole would build, from its structure
/// alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    /// The volume cells (the owned cells of a blocked file; all cell rows
    /// otherwise).
    pub cells: u64,
    /// The values of the volume's cell and point fields (rows times
    /// components).
    pub field_values: u64,
    /// Whether the file can open as an overview: one volume block with a
    /// block index (§ 6).
    pub blocked: bool,
}

impl Size {
    /// About the memory a whole read peaks at: [`OPEN_BYTES_PER_CELL`]
    /// per cell and 8 bytes per field value.
    pub fn open_bytes(&self) -> u64 {
        self.cells
            .saturating_mul(OPEN_BYTES_PER_CELL)
            .saturating_add(self.field_values.saturating_mul(8))
    }
}

/// The volume block of a blocked file and its block index.
pub(super) struct Index {
    pub grid: Object,
    /// Each partition's cell rows (owned cells, then ghosts).
    pub rows: Vec<usize>,
    /// Each block's owned cells.
    pub owned: Vec<usize>,
    pub bounds: Vec<f64>,
    pub fd: Object,
}

/// The block index of `volumes`, if the file is blocked.
pub(super) fn index_of(h: &H5, volumes: &[(String, Object)]) -> Result<Option<Index>, MeshError> {
    let [(_, grid)] = volumes else {
        return Ok(None);
    };
    let Some(fd) = h.child(grid, "FieldData")? else {
        return Ok(None);
    };
    if h.child(&fd, "BlockOwnedCells")?.is_none() || h.child(&fd, "BlockBounds")?.is_none() {
        return Ok(None);
    }
    let rows = ints(h, grid, "NumberOfCells")?
        .into_iter()
        .map(|n| count(n, "cells"))
        .collect::<Result<Vec<_>, _>>()?;
    let owned = ints(h, &fd, "BlockOwnedCells")?
        .into_iter()
        .map(|n| count(n, "owned cells"))
        .collect::<Result<Vec<_>, _>>()?;
    if owned.len() != rows.len() || owned.iter().zip(&rows).any(|(o, r)| o > r) {
        return Err(bad(
            "BlockOwnedCells does not fit the partitions' NumberOfCells",
        ));
    }
    let (bounds, dims) = floats(h, &fd, "BlockBounds")?;
    if dims != [owned.len() as u64, 6] {
        return Err(bad(format!(
            "BlockBounds has shape {dims:?} for {} blocks",
            owned.len()
        )));
    }
    Ok(Some(Index {
        grid: grid.clone(),
        rows,
        owned,
        bounds,
        fd,
    }))
}

/// The numeric datasets of `group` of `obj` that are fields, with the
/// values per row of each (`n` rows).
pub(super) fn field_sets(
    h: &H5,
    obj: &Object,
    group: &str,
    n: usize,
    reserved: &[&str],
) -> Result<Vec<(String, Dataset, usize)>, MeshError> {
    let Some(g) = h.child(obj, group)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for link in h.links(&g)? {
        if reserved.contains(&link.name.as_str()) {
            continue;
        }
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
        let Some(comps) = components(&ds, n) else {
            return Err(MeshError::new(
                "MESH005",
                format!("{group}/{} has shape {:?} for {n} rows", link.name, ds.dims),
            ));
        };
        out.push((link.name, ds, comps));
    }
    Ok(out)
}

/// What reading the VTKHDF file at `path` whole would build, from its
/// structure alone (a few pages of the file).
pub fn size_of(path: &Path) -> Result<Size, MeshError> {
    let h = H5::open(path)?;
    let (_, (volumes, _)) = blocks_of(&h)?;
    let index = index_of(&h, &volumes)?;
    let mut cells = 0u64;
    let mut field_values = 0u64;
    for (_, g) in &volumes {
        let rows: usize = ints(&h, g, "NumberOfCells")?
            .into_iter()
            .map(|n| count(n, "cells"))
            .sum::<Result<_, _>>()?;
        let points: usize = ints(&h, g, "NumberOfPoints")?
            .into_iter()
            .map(|n| count(n, "points"))
            .sum::<Result<_, _>>()?;
        cells += rows as u64;
        for (group, n, reserved) in [
            ("CellData", rows, &CELL_RESERVED[..]),
            ("PointData", points, &POINT_RESERVED[..]),
        ] {
            for (_, _, comps) in field_sets(&h, g, group, n, reserved)? {
                field_values += (n * comps) as u64;
            }
        }
    }
    if let Some(ix) = &index {
        cells = ix.owned.iter().sum::<usize>() as u64;
    }
    Ok(Size {
        cells,
        field_values,
        blocked: index.is_some(),
    })
}

/// Read the overview of the blocked VTKHDF file at `path`: its boundary as
/// a mesh without volume cells, the patches' fields and the volume's cell
/// fields on it, and the block index. Without `hash` the file is not
/// hashed (`Mesh::file_blake3` stays empty).
pub fn read_overview(path: &Path, hash: bool) -> Result<(Mesh, Results, Overview), MeshError> {
    if !path.is_file() {
        return Err(MeshError::new(
            "MESH001",
            format!("{} is not a readable file", path.display()),
        ));
    }
    let io = crate::profile::span("read.file");
    let h = H5::open(path)?;
    drop(io);
    let t = crate::profile::span("read.overview");
    let (read, digest) = std::thread::scope(|s| {
        let digest = hash.then(|| {
            s.spawn(|| {
                let stop = std::sync::atomic::AtomicBool::new(false);
                crate::FileHash::Blake3.of_file(path, &stop)
            })
        });
        let read = overview_parts(&h);
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
    drop(h);
    let (parts, overview) = read?;
    let (mut mesh, results) = parts.finish()?;
    mesh.files = vec![path.to_path_buf()];
    mesh.file_blake3 = digest.into_iter().collect();
    Ok((mesh, results, overview))
}

pub(super) fn not_blocked() -> MeshError {
    MeshError::new(
        "BLOCK004",
        "the file has no block index (one volume block with FieldData/BlockOwnedCells and BlockBounds), so it cannot open as an overview",
    )
}

fn overview_parts(h: &H5) -> Result<(Parts, Overview), MeshError> {
    let (vtk, (volumes, patches)) = blocks_of(h)?;
    let ix = index_of(h, &volumes)?.ok_or_else(not_blocked)?;
    let mut b = Builder::default();
    let mut roles = BTreeMap::new();
    names_of(h, &ix.grid, &mut b, &mut roles)?;
    let mut boundary_fields = read_patches(h, &mut b, &patches, &roles, None)?;

    // the row of each face's owner cell in the volume's cell datasets
    let total_rows: usize = ix.rows.iter().sum();
    let mut owned_at = Vec::with_capacity(ix.owned.len());
    let mut row_at = Vec::with_capacity(ix.rows.len());
    let (mut o, mut r) = (0usize, 0usize);
    for (&owned, &rows) in ix.owned.iter().zip(&ix.rows) {
        owned_at.push(o);
        row_at.push(r);
        o += owned;
        r += rows;
    }
    let cells = o;
    let mut owners: Vec<u32> = Vec::with_capacity(b.mesh.boundary.len());
    let starts: Vec<usize> = boundary_fields
        .iter()
        .map(|(s, _)| *s)
        .chain([b.mesh.boundary.len()])
        .collect();
    for ((_, obj), w) in patches.iter().zip(starts.windows(2)) {
        let n = w[1] - w[0];
        let ids = optional_ints(h, obj, "CellData", "OwnerCell")?.unwrap_or_default();
        for k in 0..n {
            let row = match ids.get(k) {
                Some(&f) if f >= 0 && (f as u64) < cells as u64 => {
                    let f = f as usize;
                    let blk = owned_at.partition_point(|&a| a <= f) - 1;
                    (row_at[blk] + f - owned_at[blk]) as u32
                }
                Some(&f) if f >= 0 => {
                    return Err(MeshError::new(
                        "MESH006",
                        format!("OwnerCell {f} is not one of the {cells} cells"),
                    ));
                }
                _ => NO_ROW,
            };
            owners.push(row);
        }
    }

    // the volume's fields: cell fields on the surface, both in the index
    let points: usize = ints(h, &ix.grid, "NumberOfPoints")?
        .into_iter()
        .map(|n| count(n, "points"))
        .sum::<Result<_, _>>()?;
    let cell_sets = field_sets(h, &ix.grid, "CellData", total_rows, &CELL_RESERVED)?;
    let point_sets = field_sets(h, &ix.grid, "PointData", points, &POINT_RESERVED)?;
    let field_values = cell_sets
        .iter()
        .map(|(_, _, k)| total_rows * k)
        .sum::<usize>()
        + point_sets.iter().map(|(_, _, k)| points * k).sum::<usize>();
    let own: std::collections::HashSet<String> = boundary_fields
        .iter()
        .flat_map(|(_, fs)| fs.iter().map(|f| f.name.clone()))
        .collect();
    let mut on_surface: Vec<Field> = Vec::new();
    let mut fields = Vec::new();
    for (name, ds, comps) in &cell_sets {
        let shown = !own.contains(name);
        if shown {
            let values = rows_at(h, ds, *comps, &owners)?;
            on_surface.extend(as_fields(name, Location::Boundary, *comps, values));
        }
        fields.push(VolumeField {
            name: name.clone(),
            location: Location::Cell,
            components: *comps,
            on_surface: shown,
        });
    }
    for (name, _, comps) in &point_sets {
        fields.push(VolumeField {
            name: name.clone(),
            location: Location::Point,
            components: *comps,
            on_surface: false,
        });
    }
    // all the boundary as one more "patch" from the first face
    boundary_fields.push((0, on_surface));

    let nb = ix.owned.len();
    let mut ranges: Vec<Vec<Option<[f64; 2]>>> = vec![vec![None; fields.len()]; nb];
    for (k, f) in fields.iter().enumerate() {
        let key = format!("BlockRange_{}", f.name);
        if h.child(&ix.fd, &key)?.is_none() {
            continue;
        }
        let (r, dims) = floats(h, &ix.fd, &key)?;
        if dims != [nb as u64, 2] {
            continue;
        }
        for (blk, &pair) in r.as_chunks::<2>().0.iter().enumerate() {
            ranges[blk][k] = Some(pair);
        }
    }
    let blocks = ix
        .owned
        .iter()
        .zip(ix.bounds.as_chunks::<6>().0)
        .zip(ranges)
        .map(|((&cells, bb), ranges)| Block {
            cells: cells as u64,
            bounds: Bounds {
                min: [bb[0], bb[1], bb[2]],
                max: [bb[3], bb[4], bb[5]],
            },
            ranges,
        })
        .collect();
    let (attrs, vtk_attrs) = text_attrs(h, &vtk)?;
    Ok((
        Parts {
            b,
            cell_fields: Vec::new(),
            point_fields: Vec::new(),
            boundary_fields,
            attrs,
            vtk_attrs,
        },
        Overview {
            cells: cells as u64,
            size: Size {
                cells: cells as u64,
                field_values: field_values as u64,
                blocked: true,
            },
            blocks,
            fields,
        },
    ))
}

/// A face without an owner cell in the file.
pub(super) const NO_ROW: u32 = u32::MAX;

/// The values of `ds` (`comps` per row) at `rows`, NaN at [`NO_ROW`]: a
/// dataset that lies whole in the file is read a run of nearby rows at a
/// time, any other once.
pub(super) fn rows_at(
    h: &H5,
    ds: &Dataset,
    comps: usize,
    rows: &[u32],
) -> Result<Vec<f64>, MeshError> {
    let mut out = vec![f64::NAN; rows.len() * comps];
    if !h.reads_rows(ds) {
        let v = h.values(ds)?;
        for (i, &r) in rows.iter().enumerate() {
            if r != NO_ROW {
                for k in 0..comps {
                    out[i * comps + k] = v.float(r as usize * comps + k);
                }
            }
        }
        return Ok(out);
    }
    let mut order: Vec<usize> = (0..rows.len()).filter(|&i| rows[i] != NO_ROW).collect();
    order.sort_unstable_by_key(|&i| rows[i]);
    let mut i = 0;
    while i < order.len() {
        let mut j = i + 1;
        while j < order.len() && (rows[order[j]] - rows[order[j - 1]]) as usize <= NEAR {
            j += 1;
        }
        let (first, last) = (rows[order[i]] as usize, rows[order[j - 1]] as usize);
        let v = h.values_in(ds, first * comps..(last + 1) * comps)?;
        if v.held().end < (last + 1) * comps {
            return Err(bad("a cell field is shorter than its cells"));
        }
        for &f in &order[i..j] {
            let r = rows[f] as usize;
            for k in 0..comps {
                out[f * comps + k] = v.float(r * comps + k);
            }
        }
        i = j;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::write_blocked;
    use crate::{Component, NO_CELL};

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("murmel-overview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn fixture(rel: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
            .join(rel)
    }

    /// The honeycomb with a scalar and a vector cell field, a point field
    /// and a boundary field.
    fn honeycomb() -> (Mesh, Results) {
        let m = crate::synth::honeycomb(6);
        let n = m.cell_count();
        let field = |name: &str, location, components, values| Field {
            name: name.into(),
            unit: None,
            location,
            components,
            values,
        };
        let r = Results {
            fields: vec![
                field(
                    "c",
                    Location::Cell,
                    1,
                    (0..n).map(|c| c as f64 * 0.5 - 7.0).collect(),
                ),
                field(
                    "u",
                    Location::Cell,
                    3,
                    (0..3 * n).map(|i| (i % 7) as f64 - 2.5).collect(),
                ),
                field(
                    "p",
                    Location::Point,
                    1,
                    m.points.iter().map(|p| p[0] - p[2]).collect(),
                ),
                field(
                    "tau",
                    Location::Boundary,
                    1,
                    (0..m.boundary.len()).map(|f| f as f64).collect(),
                ),
            ],
            ..Results::default()
        };
        (m, r)
    }

    /// The corners of boundary face `i`, by their coordinates.
    fn face(m: &Mesh, i: usize) -> Vec<[u64; 3]> {
        m.boundary
            .get(i)
            .iter()
            .map(|&q| m.points[q as usize].map(f64::to_bits))
            .collect()
    }

    /// Invariant: the overview of a blocked file is the boundary of the
    /// whole read with no volume cell: the same patches, zones and faces
    /// in the same order with the same corners, each point once; the
    /// patches' own fields as read whole; each cell field of the volume
    /// (a scalar and a vector) on the faces as the value of each face's
    /// owner cell; the cells, blocks and field ranges of the block index;
    /// and what `size_of` says a whole read builds. For FoehnCraft's
    /// channel (wedges and polyhedra, three patches) and the honeycomb,
    /// cut into blocks far smaller than the mesh so that the owners lie in
    /// many blocks and runs. Breaks when an owner's file cell id is taken
    /// as its row (ghost rows come between blocks), a run of rows is read
    /// at the wrong place, the patches are not joined by their global
    /// point ids, or the cells are read. No other test reads an overview.
    #[test]
    fn an_overview_is_the_boundary_of_the_whole_read() {
        let channel = crate::read_results(&fixture("vtkhdf/channel_fields.vtkhdf")).unwrap();
        for (name, (m, r), block) in [("channel", channel, 100), ("honeycomb", honeycomb(), 40)] {
            let path = scratch(&format!("{name}.vtkhdf"));
            let w = write_blocked(&path, &m, &r, block).unwrap();
            let (whole, rw) = crate::read_results(&path).unwrap();
            let (o, ro, ov) = read_overview(&path, false).unwrap();
            assert_eq!(o.cell_count(), 0, "{name}");
            assert_eq!((&o.patches, &o.zones), (&whole.patches, &whole.zones));
            assert_eq!(o.boundary_patch, whole.boundary_patch);
            for i in 0..whole.boundary.len() {
                assert_eq!(face(&o, i), face(&whole, i), "{name} face {i}");
            }
            let used: std::collections::BTreeSet<u32> =
                whole.boundary.nodes.iter().copied().collect();
            assert_eq!(o.points.len(), used.len(), "{name}: each point once");
            assert_eq!(o.bounds(), whole.bounds());

            assert_eq!(ov.cells, whole.cell_count() as u64);
            assert_eq!(ov.blocks.len(), w.blocks);
            assert_eq!(ov.blocks.iter().map(|b| b.cells).sum::<u64>(), ov.cells);
            let mut all = Bounds::empty();
            for b in &ov.blocks {
                all.add(b.bounds.min);
                all.add(b.bounds.max);
            }
            let wb = whole.bounds();
            assert!((0..3).all(|k| all.min[k] <= wb.min[k] && all.max[k] >= wb.max[k]));

            let mut shown = 0;
            for f in &rw.fields {
                let g = ro.get(&f.name);
                match f.location {
                    Location::Boundary => {
                        let g = g.unwrap();
                        assert_eq!(g.values, f.values, "{name}: {}", f.name);
                    }
                    Location::Cell => {
                        let g = g.unwrap_or_else(|| panic!("{name}: no {}", f.name));
                        assert_eq!(
                            (g.location, g.components, &g.unit),
                            (Location::Boundary, f.components, &f.unit)
                        );
                        for (i, &c) in whole.boundary_owner.iter().enumerate() {
                            assert_ne!(c, NO_CELL);
                            assert_eq!(g.value(i), f.value(c as usize), "{name}: {} {i}", f.name);
                        }
                        let v = ov.fields.iter().find(|v| v.name == f.name).unwrap();
                        assert!(v.on_surface && v.location == Location::Cell);
                        let (lo, hi) = (0..whole.cell_count())
                            .map(|c| f.scalar(c, Component::Magnitude))
                            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
                                (a.min(x), b.max(x))
                            });
                        assert_eq!(ov.range(&f.name), Some([lo, hi]), "{name}: {}", f.name);
                        shown += 1;
                    }
                    Location::Point => {
                        assert!(g.is_none());
                        let v = ov.fields.iter().find(|v| v.name == f.name).unwrap();
                        assert!(!v.on_surface && v.location == Location::Point);
                    }
                }
            }
            assert!(shown >= 1, "{name}");
            assert_eq!(ro.iteration, rw.iteration);
            assert_eq!(o.metadata.get("vtkhdf").map(String::as_str), Some("2"));

            let size = size_of(&path).unwrap();
            assert_eq!(ov.size, size);
            assert!(size.blocked);
            assert_eq!(size.cells, whole.cell_count() as u64);
            let rows = w.cells + w.ghost_cells;
            let want: usize = rw
                .fields
                .iter()
                .map(|f| match f.location {
                    Location::Cell => rows * f.components,
                    Location::Point => w.block_points * f.components,
                    Location::Boundary => 0,
                })
                .sum();
            assert_eq!(size.field_values, want as u64, "{name}");
            std::fs::remove_file(&path).ok();
        }
    }

    /// Invariant: a VTKHDF file without a block index (one written by
    /// FoehnCraft or VTK) says so (`BLOCK004`) instead of opening as an
    /// overview, and `size_of` counts its cells and calls it not blocked.
    /// Breaks when a file without the index opens as an empty overview or
    /// is called blocked: the window would then show a mesh without cells
    /// in place of one it could read.
    #[test]
    fn files_without_a_block_index_are_not_overviews() {
        let path = fixture("vtkhdf/box_latest.vtkhdf");
        let (m, _) = crate::read_results(&path).unwrap();
        let size = size_of(&path).unwrap();
        assert!(!size.blocked);
        assert_eq!(size.cells, m.cell_count() as u64);
        assert_eq!(
            read_overview(&path, false).err().map(|e| e.code),
            Some("BLOCK004")
        );
    }

    /// Invariant: values gathered at rows read in runs are the rows' own,
    /// for rows near and far apart, repeated, out of order and missing
    /// ([`NO_ROW`] gives NaN), of scalars and of vectors. Breaks when a
    /// run is cut at the wrong row, a row is read as a position of values
    /// (not times the components), or the order of the faces is lost. The
    /// overview test's owners are few and close, so only this test makes
    /// runs that are far apart.
    #[test]
    fn rows_gathered_in_runs_are_the_rows() {
        use crate::h5write::{H5Writer, Link};
        let path = scratch("rows.h5");
        let n = 50_000usize;
        let one: Vec<f64> = (0..n).map(|i| i as f64 * 1.5).collect();
        let three: Vec<f64> = (0..3 * n).map(|i| -(i as f64)).collect();
        let mut w = H5Writer::create(&path).unwrap();
        let a = w.dataset(&[n as u64], &one).unwrap();
        let b = w.dataset(&[n as u64, 3], &three).unwrap();
        let root = w
            .group(
                &[Link::Hard("one".into(), a), Link::Hard("three".into(), b)],
                &[],
            )
            .unwrap();
        w.finish(root).unwrap();
        let h = H5::open(&path).unwrap();
        let rows: Vec<u32> = vec![7, 49_999, NO_ROW, 7, 8, 30_000, 0, 30_000 + NEAR as u32, 12];
        for (name, comps, all) in [("one", 1, &one), ("three", 3, &three)] {
            let ds = h.dataset(&h.path(&format!("/{name}")).unwrap()).unwrap();
            assert!(h.reads_rows(&ds));
            let got = rows_at(&h, &ds, comps, &rows).unwrap();
            for (i, &r) in rows.iter().enumerate() {
                for k in 0..comps {
                    let g = got[i * comps + k];
                    if r == NO_ROW {
                        assert!(g.is_nan());
                    } else {
                        assert_eq!(g, all[r as usize * comps + k], "{name} row {r}");
                    }
                }
            }
        }
        std::fs::remove_file(&path).ok();
    }
}
