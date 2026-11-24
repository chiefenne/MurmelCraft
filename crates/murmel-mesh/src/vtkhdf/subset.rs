//! Some blocks of a blocked VTKHDF file (`docs/blocked-vtkhdf.md`): each
//! block's partition read apart from the others, its owned cells and,
//! with the halo, the ghost cells it holds (one layer of point
//! neighbours, decided on the regions card 6), with their points joined
//! by `GlobalPointIds`, the cell and point fields at their rows, and the
//! patch faces their cells own. A region of a mesh beyond the memory
//! budget reads only the blocks it touches.
//!
//! The cells come block by block in the order asked (sorted), each
//! block's owned cells first, then the ghosts it holds that are neither
//! owned by a block read nor a ghost taken before. Read with every block
//! and no halo, the mesh is the whole read's, bit for bit.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;

use super::cells::{self, PointMap, Set};
use super::overview::{CELL_RESERVED, POINT_RESERVED, field_sets, index_of, not_blocked, rows_at};
use super::{
    Builder, GridSets, Parts, as_fields, bad, blocks_of, count, dataset_of, ints, names_of,
    read_patches, text_attrs,
};
use crate::field::{Location, Results};
use crate::hdf5::{H5, Object, Values};
use crate::{Mesh, MeshError};

/// What [`read_blocks`] read besides the mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Subset {
    /// The blocks read, sorted.
    pub blocks: Vec<usize>,
    /// The blocks of the file.
    pub of_blocks: usize,
    /// The volume cells of the file (owned cells).
    pub of_cells: u64,
    /// The file cell id of each cell of the mesh.
    pub file_cells: Vec<u64>,
    /// Whether each cell of the mesh is of the halo: a ghost of a block
    /// read, owned by a block not read.
    pub halo: Vec<bool>,
}

/// Read blocks `blocks` of the blocked VTKHDF file at `path`, with the
/// halo of their ghost cells when `halo` is set. BLOCK004 for a file
/// without a block index, BLOCK005 for a block it does not have.
pub fn read_blocks(
    path: &Path,
    blocks: &[usize],
    halo: bool,
) -> Result<(Mesh, Results, Subset), MeshError> {
    if !path.is_file() {
        return Err(MeshError::new(
            "MESH001",
            format!("{} is not a readable file", path.display()),
        ));
    }
    let io = crate::profile::span("read.file");
    let h = H5::open(path)?;
    drop(io);
    let t = crate::profile::span("read.blocks");
    let read = subset_parts(&h, blocks, halo);
    drop(t);
    drop(h);
    let (parts, subset) = read?;
    let (mut mesh, results) = parts.finish()?;
    mesh.files = vec![path.to_path_buf()];
    Ok((mesh, results, subset))
}

/// A block the file does not have, or none.
fn no_such_block(message: String) -> MeshError {
    MeshError::new("BLOCK005", message)
}

/// The file cell ids read so far, one bit each.
struct Taken(Vec<u64>);

impl Taken {
    fn new(cells: usize) -> Taken {
        Taken(vec![0; cells.div_ceil(64)])
    }

    fn has(&self, id: usize) -> bool {
        self.0
            .get(id / 64)
            .is_some_and(|w| w & (1 << (id % 64)) != 0)
    }

    fn take(&mut self, id: usize) {
        if let Some(w) = self.0.get_mut(id / 64) {
            *w |= 1 << (id % 64);
        }
    }
}

/// What a block gives: the rows of its partition to leave out (1) or
/// read (0), and the file cell id of each ghost row.
struct Plan {
    skip: Vec<u8>,
    ghost_ids: Vec<i64>,
}

fn subset_parts(h: &H5, asked: &[usize], halo: bool) -> Result<(Parts, Subset), MeshError> {
    let (vtk, (volumes, patches)) = blocks_of(h)?;
    let ix = index_of(h, &volumes)?.ok_or_else(not_blocked)?;
    let nb = ix.owned.len();
    let mut blocks = asked.to_vec();
    blocks.sort_unstable();
    blocks.dedup();
    if blocks.is_empty() {
        return Err(no_such_block("no block to read".into()));
    }
    if let Some(&b) = blocks.iter().find(|&&b| b >= nb) {
        return Err(no_such_block(format!(
            "block {b} is not one of the {nb} blocks of the file"
        )));
    }
    // where each block's owned cells start among the file's, and its rows
    let (mut owned_at, mut row_at) = (Vec::with_capacity(nb), Vec::with_capacity(nb));
    let (mut o, mut r) = (0usize, 0usize);
    for (&owned, &rows) in ix.owned.iter().zip(&ix.rows) {
        owned_at.push(o);
        row_at.push(r);
        o += owned;
        r += rows;
    }
    let (cells_total, rows_total) = (o, r);

    // The owned cells of every block read are taken before any ghost, so
    // a ghost of one block read that another owns is left out.
    let mut taken = Taken::new(cells_total);
    for &b in &blocks {
        for id in owned_at[b]..owned_at[b] + ix.owned[b] {
            taken.take(id);
        }
    }
    let gcid = optional_dataset(h, &ix.grid, "CellData", "GlobalCellIds")?;
    let mut plans = Vec::with_capacity(blocks.len());
    for &b in &blocks {
        let rows = row_at[b]..row_at[b] + ix.rows[b];
        let own = ix.owned[b];
        let mut skip = vec![0u8; rows.len()];
        let mut ghost_ids = Vec::new();
        if halo && rows.len() > own {
            let Some(ds) = &gcid else {
                return Err(bad(
                    "the cells have no CellData/GlobalCellIds, so the ghosts cannot be read as a halo",
                ));
            };
            let ghosts = rows.start + own..rows.end;
            let ids = h.int_values_in(ds, ghosts.clone())?;
            if ids.held().end < ghosts.end {
                return Err(bad("CellData/GlobalCellIds is shorter than the cells"));
            }
            for (k, r) in ghosts.enumerate() {
                let id = ids.int(r);
                let Some(f) = usize::try_from(id).ok().filter(|&f| f < cells_total) else {
                    return Err(MeshError::new(
                        "MESH006",
                        format!("ghost cell {r} copies cell {id}, not one of the {cells_total}"),
                    ));
                };
                if taken.has(f) {
                    skip[own + k] = 1;
                } else {
                    taken.take(f);
                }
                ghost_ids.push(id);
            }
        } else {
            skip[own..].fill(1);
        }
        plans.push(Plan { skip, ghost_ids });
    }

    let mut b = Builder::default();
    let mut roles = BTreeMap::new();
    names_of(h, &ix.grid, &mut b, &mut roles)?;
    let (cell_rows, point_rows) = b.grid_blocks(h, &ix.grid, &blocks, &plans)?;

    // the file cell id of each cell read, and whether it is of the halo
    let mut file_cells = Vec::with_capacity(cell_rows.len());
    let mut halo_of = Vec::with_capacity(cell_rows.len());
    for &row in &cell_rows {
        let row = row as usize;
        let k = blocks.partition_point(|&q| row_at[q] + ix.rows[q] <= row);
        let (blk, plan) = (blocks[k], &plans[k]);
        let i = row - row_at[blk];
        if i < ix.owned[blk] {
            file_cells.push((owned_at[blk] + i) as u64);
            halo_of.push(false);
        } else {
            file_cells.push(plan.ghost_ids[i - ix.owned[blk]] as u64);
            halo_of.push(true);
        }
    }

    // the fields at the rows read
    let points_total: usize = ints(h, &ix.grid, "NumberOfPoints")?
        .into_iter()
        .map(|n| count(n, "points"))
        .sum::<Result<_, _>>()?;
    let mut cell_fields = Vec::new();
    for (name, ds, comps) in field_sets(h, &ix.grid, "CellData", rows_total, &CELL_RESERVED)? {
        let values = rows_at(h, &ds, comps, &cell_rows)?;
        cell_fields.extend(as_fields(&name, Location::Cell, comps, values));
    }
    let mut point_fields = Vec::new();
    for (name, ds, comps) in field_sets(h, &ix.grid, "PointData", points_total, &POINT_RESERVED)? {
        let values = rows_at(h, &ds, comps, &point_rows)?;
        point_fields.extend(as_fields(&name, Location::Point, comps, values));
    }

    // the faces of the patches whose owner cell was read
    let read = |id: i64| usize::try_from(id).is_ok_and(|f| taken.has(f));
    let boundary_fields = read_patches(h, &mut b, &patches, &roles, Some(&read))?;
    let (attrs, vtk_attrs) = text_attrs(h, &vtk)?;
    Ok((
        Parts {
            b,
            cell_fields: vec![cell_fields],
            point_fields: vec![point_fields],
            boundary_fields,
            attrs,
            vtk_attrs,
        },
        Subset {
            blocks,
            of_blocks: nb,
            of_cells: cells_total as u64,
            file_cells,
            halo: halo_of,
        },
    ))
}

/// Dataset `name` of subgroup `group` of `obj`, when there is one.
fn optional_dataset(
    h: &H5,
    obj: &Object,
    group: &str,
    name: &str,
) -> Result<Option<crate::hdf5::Dataset>, MeshError> {
    let Some(g) = h.child(obj, group)? else {
        return Ok(None);
    };
    match h.child(&g, name)? {
        Some(o) => Ok(Some(h.dataset(&o)?)),
        None => Ok(None),
    }
}

/// A grid's datasets with the ghost rows replaced by a block's plan: the
/// rows it leaves out.
struct Planned<'a, 'h> {
    sets: &'a GridSets<'h>,
    skip: Values<'static>,
}

impl cells::Sets for Planned<'_, '_> {
    fn rows(&self, set: Set, rows: Range<usize>) -> Result<Option<Values<'_>>, MeshError> {
        if set == Set::Ghost {
            return Ok(Some(self.skip.view()));
        }
        self.sets.rows(set, rows)
    }
}

impl Builder {
    /// Read the cells of partitions `blocks` of grid `g` as each block's
    /// plan says, partition after partition; gives the rows of the cell
    /// datasets and of `Points` the mesh took.
    fn grid_blocks(
        &mut self,
        h: &H5,
        g: &Object,
        blocks: &[usize],
        plans: &[Plan],
    ) -> Result<(Vec<u32>, Vec<u32>), MeshError> {
        let np = ints(h, g, "NumberOfPoints")?;
        let nc = ints(h, g, "NumberOfCells")?;
        let ncon = ints(h, g, "NumberOfConnectivityIds")?;
        if nc.len() != np.len() || ncon.len() != np.len() {
            return Err(bad(
                "NumberOfPoints, NumberOfCells and NumberOfConnectivityIds differ in length",
            ));
        }
        let total_points: usize = np
            .iter()
            .map(|&n| count(n, "points"))
            .sum::<Result<_, _>>()?;
        let total_cells: usize = nc
            .iter()
            .map(|&n| count(n, "cells"))
            .sum::<Result<_, _>>()?;
        let points = dataset_of(h, g, "Points")?;
        if points.dims != [total_points as u64, 3] {
            return Err(MeshError::new(
                "MESH005",
                format!(
                    "Points has shape {:?} for {total_points} points",
                    points.dims
                ),
            ));
        }
        let gids = optional_dataset(h, g, "PointData", "GlobalPointIds")?.ok_or_else(|| {
            bad("a blocked file needs PointData/GlobalPointIds to join its blocks")
        })?;
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
        if let Some(z) = optional_dataset(h, g, "CellData", "zone")? {
            sets.add(Set::Zone, &z, true)?;
        }
        let counts = poly_counts
            .as_ref()
            .map(|(nf, np2f, nfc)| [&nf[..], &np2f[..], &nfc[..]]);
        let part_list = cells::parts(&np, &nc, &ncon, counts);
        if self.mesh.cell_faces.is_empty() {
            self.mesh.cell_faces.push(0);
            self.polys = crate::Polygons::new();
            self.least = Vec::new();
            self.mesh.boundary = crate::Polygons::new();
        }
        let (mut cell_rows, mut point_rows) = (Vec::new(), Vec::new());
        for (&blk, plan) in blocks.iter().zip(plans) {
            // a read in the background stops between blocks
            crate::profile::check()?;
            let part = part_list[blk];
            // its points, joined to those of the blocks before
            let rows = part.point_at..part.point_at + part.points;
            let xyz = h.values_in(&points, 3 * rows.start..3 * rows.end)?;
            let ids = h.int_values_in(&gids, rows.clone())?;
            if xyz.held().end < 3 * rows.end || ids.held().end < rows.end {
                return Err(bad("Points or GlobalPointIds is shorter than the points"));
            }
            let (map, new) = self.join_points(&xyz, &ids, rows.clone());
            drop(xyz);
            point_rows.extend(new);
            let map = PointMap::From(rows.start, map);
            let skip = Values::of_bytes_in(plan.skip.clone(), part.cell_at, total_cells);
            let planned = Planned { sets: &sets, skip };
            let t = crate::profile::span("read.cells");
            let grid = cells::Grid {
                parts: std::slice::from_ref(&part),
                poly,
                zone: None,
                points: &map,
                xyz: &self.mesh.points,
            };
            let read = cells::read_rows(&grid, &planned, part.cell_at..part.cell_at + part.cells)?;
            drop(t);
            drop(crate::profile::span("read.blocks"));
            self.add(read.polys, read.least, read.cell_end);
            self.mesh.cell_zone.extend_from_slice(&read.zone);
            self.mesh.cell_kind.extend_from_slice(&read.kind);
            self.zone_max = self.zone_max.max(read.zone_max);
            let polygons = read.boundary.len();
            super::append(&mut self.mesh.boundary, read.boundary);
            self.mesh
                .boundary_patch
                .extend(std::iter::repeat_n(u32::MAX, polygons));
            cell_rows.extend(read.rows);
        }
        Ok((cell_rows, point_rows))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use super::*;
    use crate::blocks::write_blocked;
    use crate::{Field, NO_CELL};

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("murmel-subset-{}", std::process::id()));
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
        let c: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin()).collect();
        let u: Vec<f64> = (0..3 * n).map(|i| (i as f64 * 0.11).cos()).collect();
        let p: Vec<f64> = (0..m.points.len()).map(|i| i as f64 * 0.5).collect();
        let tau: Vec<f64> = (0..m.boundary.len()).map(|i| i as f64).collect();
        let r = Results {
            fields: vec![
                field("c", Location::Cell, 1, c),
                field("u", Location::Cell, 3, u),
                field("p", Location::Point, 1, p),
                field("tau", Location::Boundary, 1, tau),
            ],
            ..Results::default()
        };
        (m, r)
    }

    /// The files of the tests: the channel in blocks of 100, the
    /// honeycomb in blocks of 40, each test's own (`test`).
    fn files(test: &str) -> Vec<(&'static str, std::path::PathBuf)> {
        let channel = crate::read_results(&fixture("vtkhdf/channel_fields.vtkhdf")).unwrap();
        let mut out = Vec::new();
        for (name, (m, r), block) in [("channel", channel, 100), ("honeycomb", honeycomb(), 40)] {
            let path = scratch(&format!("{test}-{name}.vtkhdf"));
            write_blocked(&path, &m, &r, block).unwrap();
            out.push((name, path));
        }
        out
    }

    /// A cell's points, as coordinates, sorted.
    fn corners(m: &Mesh, c: usize) -> Vec<[u64; 3]> {
        let mut v: Vec<[u64; 3]> = m
            .cell(c)
            .flat_map(|f| {
                f.nodes
                    .iter()
                    .map(|&p| m.points[p as usize].map(f64::to_bits))
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        v.sort_unstable();
        v
    }

    /// A boundary face's points, as coordinates, sorted.
    fn face_corners(m: &Mesh, b: usize) -> Vec<[u64; 3]> {
        let mut v: Vec<[u64; 3]> = m
            .boundary
            .get(b)
            .iter()
            .map(|&p| m.points[p as usize].map(f64::to_bits))
            .collect();
        v.sort_unstable();
        v
    }

    /// The cells sharing a point with one of `cells` and not among them.
    fn neighbours(m: &Mesh, cells: &BTreeSet<usize>) -> BTreeSet<usize> {
        let mut at: HashMap<u32, Vec<usize>> = HashMap::new();
        for c in 0..m.cell_count() {
            for p in m
                .cell(c)
                .flat_map(|f| f.nodes.iter().copied())
                .collect::<BTreeSet<_>>()
            {
                at.entry(p).or_default().push(c);
            }
        }
        let mut out = BTreeSet::new();
        for &c in cells {
            for p in m.cell(c).flat_map(|f| f.nodes.iter().copied()) {
                for &d in &at[&p] {
                    if !cells.contains(&d) {
                        out.insert(d);
                    }
                }
            }
        }
        out
    }

    /// Invariant: every block without the halo is the whole read, bit
    /// for bit (points, cells, faces, boundary and its owners, zones,
    /// fields), so a mesh read by blocks is the mesh. Breaks when the
    /// partitions read apart join their points in another order, a cell
    /// row or field row shifts, or a patch face is lost; the whole read's
    /// tests never read a partition alone.
    #[test]
    fn every_block_is_the_whole_read() {
        for (name, path) in files("all") {
            let (whole, rw) = crate::read_results(&path).unwrap();
            let nb = (0..)
                .find(|&k| read_blocks(&path, &[k], false).is_err())
                .unwrap();
            let all: Vec<usize> = (0..nb).rev().collect();
            let (m, r, sub) = read_blocks(&path, &all, false).unwrap();
            assert_eq!(sub.blocks, (0..nb).collect::<Vec<_>>(), "{name}");
            assert_eq!(sub.of_blocks, nb);
            assert_eq!(sub.of_cells, whole.cell_count() as u64);
            assert_eq!(
                sub.file_cells,
                (0..whole.cell_count() as u64).collect::<Vec<_>>()
            );
            assert!(sub.halo.iter().all(|&h| !h));
            assert_eq!(m.points, whole.points, "{name}");
            assert_eq!(m.cell_faces, whole.cell_faces, "{name}");
            assert_eq!(m.cell_count(), whole.cell_count());
            for c in 0..m.cell_count() {
                let a: Vec<&[u32]> = m.cell(c).map(|f| f.nodes).collect();
                let b: Vec<&[u32]> = whole.cell(c).map(|f| f.nodes).collect();
                assert_eq!(a, b, "{name} cell {c}");
            }
            assert_eq!(
                (&m.boundary.start, &m.boundary.nodes),
                (&whole.boundary.start, &whole.boundary.nodes)
            );
            assert_eq!(m.boundary_patch, whole.boundary_patch);
            assert_eq!(m.boundary_owner, whole.boundary_owner);
            assert_eq!(
                (&m.cell_zone, &m.cell_kind),
                (&whole.cell_zone, &whole.cell_kind)
            );
            assert_eq!((&m.zones, &m.patches), (&whole.zones, &whole.patches));
            assert_eq!(r.fields, rw.fields, "{name}");
        }
    }

    /// Invariant: one block, or two, read with the halo give the cells
    /// the block owns and every cell of other blocks sharing a point with
    /// them, each once, flagged as the halo, with their own coordinates
    /// and field values and the patch faces they own (each with its
    /// owner); without the halo, the owned cells alone. Breaks when a
    /// ghost owned by another block read comes twice, the halo misses a
    /// point neighbour, a field row of a ghost is taken from the wrong
    /// row, or a patch face of a cell not read appears; the whole read
    /// drops every ghost.
    #[test]
    fn blocks_read_with_their_halo() {
        for (name, path) in files("halo") {
            let (whole, rw) = crate::read_results(&path).unwrap();
            let fields: Vec<&Field> = rw
                .fields
                .iter()
                .filter(|f| f.location == Location::Cell)
                .collect();
            assert!(!fields.is_empty());
            for asked in [vec![1], vec![2, 1], vec![0]] {
                let (_, _, plain) = read_blocks(&path, &asked, false).unwrap();
                let (m, r, sub) = read_blocks(&path, &asked, true).unwrap();
                let own: BTreeSet<usize> = plain.file_cells.iter().map(|&f| f as usize).collect();
                assert!(plain.halo.iter().all(|&h| !h));
                let halo = neighbours(&whole, &own);
                assert!(!halo.is_empty(), "{name} {asked:?}");
                let got: Vec<usize> = sub.file_cells.iter().map(|&f| f as usize).collect();
                let unique: BTreeSet<usize> = got.iter().copied().collect();
                assert_eq!(unique.len(), got.len(), "{name} {asked:?}: each cell once");
                let want: BTreeSet<usize> = own.union(&halo).copied().collect();
                assert_eq!(unique, want, "{name} {asked:?}");
                for (k, &f) in got.iter().enumerate() {
                    assert_eq!(sub.halo[k], halo.contains(&f));
                    assert_eq!(corners(&m, k), corners(&whole, f), "{name} cell {f}");
                    for w in &fields {
                        let g = r.get(&w.name).unwrap();
                        assert_eq!(g.location, Location::Cell);
                        for j in 0..w.components {
                            assert_eq!(
                                g.values[k * w.components + j],
                                w.values[f * w.components + j],
                                "{name} {} of cell {f}",
                                w.name
                            );
                        }
                    }
                }
                // the patch faces of the cells read, each with its owner
                let mut want_faces: Vec<Vec<[u64; 3]>> = (0..whole.boundary.len())
                    .filter(|&b| unique.contains(&(whole.boundary_owner[b] as usize)))
                    .map(|b| face_corners(&whole, b))
                    .collect();
                let mut faces: Vec<Vec<[u64; 3]>> =
                    (0..m.boundary.len()).map(|b| face_corners(&m, b)).collect();
                want_faces.sort();
                faces.sort();
                assert_eq!(faces, want_faces, "{name} {asked:?}");
                assert!(m.boundary_owner.iter().all(|&o| o != NO_CELL));
                for (b, &o) in m.boundary_owner.iter().enumerate() {
                    let f = got[o as usize];
                    let wb = (0..whole.boundary.len())
                        .find(|&w| face_corners(&whole, w) == face_corners(&m, b))
                        .unwrap();
                    assert_eq!(whole.boundary_owner[wb] as usize, f);
                }
                if let Some(tau) = r.get("tau") {
                    assert_eq!(tau.values.len(), m.boundary.len());
                }
            }
        }
    }

    /// Invariant: a block the file does not have, or none, is BLOCK005;
    /// a file without a block index is BLOCK004. Breaks when a bad block
    /// number reads nothing quietly, or a plain file reads as blocks.
    #[test]
    fn blocks_beyond_the_file_are_refused() {
        let (_, path) = files("refused").remove(0);
        let e = read_blocks(&path, &[999], false).unwrap_err();
        assert_eq!(e.code, "BLOCK005");
        assert_eq!(read_blocks(&path, &[], true).unwrap_err().code, "BLOCK005");
        let plain = fixture("vtkhdf/box_latest.vtkhdf");
        assert_eq!(
            read_blocks(&plain, &[0], false).unwrap_err().code,
            "BLOCK004"
        );
    }
}
