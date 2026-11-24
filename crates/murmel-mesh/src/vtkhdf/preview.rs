//! The preview grid of a blocked VTKHDF file (`docs/blocked-vtkhdf.md`):
//! the volume's cell fields on a regular grid of about
//! [`PREVIEW_CELLS`] cells along the longest side of the blocks' box, so
//! slices, iso-surfaces and arrows across the whole volume of a mesh
//! beyond the memory budget show at once (the maintainer's card 7 of
//! 2026-11-24: the overview is the surface, the block ranges and a
//! preview grid).
//!
//! Each grid cell takes the values of the file cell its centre lies in,
//! found as the cell whose centroid (the mean of its points) is nearest
//! the centre among the cells whose box holds it. In a Voronoi mesh that
//! is the cell holding the centre; elsewhere it is that cell or one next
//! to it. A centre in no cell's box is outside the volume, and the grid
//! has no cell there.
//!
//! The blocks are read a block to a thread at a time, only their points
//! and the point lists of their owned cells (`Points`, `Offsets` and
//! `Connectivity`), and each grid cell keeps the nearest centroid so far
//! in one atomic word: the distance's bits above the cell's row. Each cell
//! field, and the zones, are then read at the rows found, a run of nearby
//! rows at a time. The grid is a mesh of hexahedra, so what draws on a
//! mesh read whole draws on it unchanged.

use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use super::overview::{CELL_RESERVED, field_sets, index_of, not_blocked, rows_at};
use super::{Builder, as_fields, bad, blocks_of, cells, count, dataset_of, ints, names_of};
use crate::field::{Location, Results};
use crate::hdf5::H5;
use crate::{Bounds, CellKind, Mesh, MeshError, Polygons, boundary};

/// The grid cells along the longest side of the blocks' box (the
/// proposal's "about 128³"; 8 MB a scalar at most).
pub const PREVIEW_CELLS: usize = 128;

/// The most grid cells along a side: 256³ grid cells take 2.7 GB as
/// hexahedra.
const MOST_CELLS: usize = 256;

/// Blocks read between two checks for Cancel: a few for each thread.
const BATCH: usize = 64;

/// Where the cells of the preview grid lie, and what each shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewGrid {
    /// Grid cells along x, y and z.
    pub shape: [usize; 3],
    /// The grid's lowest corner and the size of its cells: it fills the
    /// box of the blocks, and is one cell thick about it along an axis
    /// the box has no extent in.
    pub origin: [f64; 3],
    pub step: [f64; 3],
    /// The grid position of each cell of the preview mesh, x fastest.
    pub voxels: Vec<u32>,
    /// The file cell id whose values each cell of the preview mesh shows.
    pub file_cells: Vec<u64>,
}

impl PreviewGrid {
    /// The box the grid fills.
    pub fn bounds(&self) -> Bounds {
        Bounds {
            min: self.origin,
            max: [0, 1, 2].map(|k| self.axis(k).end()),
        }
    }

    /// The centre of grid position `v` (x fastest).
    pub fn centre(&self, v: usize) -> [f64; 3] {
        let [nx, ny, _] = self.shape;
        let at = [
            v % nx.max(1),
            (v / nx.max(1)) % ny.max(1),
            v / (nx * ny).max(1),
        ];
        [0, 1, 2].map(|k| self.axis(k).centre(at[k]))
    }

    fn axis(&self, k: usize) -> Axis {
        Axis {
            start: self.origin[k],
            step: self.step[k],
            cells: self.shape[k],
        }
    }
}

/// One axis of the grid: where it starts, the size of a cell, the cells.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Axis {
    start: f64,
    step: f64,
    cells: usize,
}

impl Axis {
    fn centre(&self, i: usize) -> f64 {
        self.start + (i as f64 + 0.5) * self.step
    }

    fn end(&self) -> f64 {
        self.start + self.cells as f64 * self.step
    }

    /// The cells whose centres lie in `lo..=hi`.
    fn span(&self, lo: f64, hi: f64) -> Range<usize> {
        let a = ((lo - self.start) / self.step - 0.5).ceil().max(0.0);
        let b = ((hi - self.start) / self.step - 0.5).floor();
        if b.is_nan() || a > b || b < 0.0 {
            return 0..0;
        }
        let (a, b) = (a as usize, (b as usize).min(self.cells - 1));
        if a > b { 0..0 } else { a..b + 1 }
    }
}

/// The grid over `bounds` with `longest` cells along its longest side
/// (1 to [`MOST_CELLS`]), the cells as near cubes as whole numbers allow;
/// `None` for a box without extent.
fn axes_of(bounds: &Bounds, longest: usize) -> Option<[Axis; 3]> {
    if bounds.is_empty() {
        return None;
    }
    let ext = [0, 1, 2].map(|k| bounds.max[k] - bounds.min[k]);
    let top = ext.iter().copied().fold(0.0, f64::max);
    if !(top > 0.0 && top.is_finite()) {
        return None;
    }
    let longest = longest.clamp(1, MOST_CELLS);
    let h = top / longest as f64;
    Some([0, 1, 2].map(|k| {
        if ext[k] > 0.0 {
            let cells = ((ext[k] / h).round() as usize).clamp(1, longest);
            Axis {
                start: bounds.min[k],
                step: ext[k] / cells as f64,
                cells,
            }
        } else {
            Axis {
                start: bounds.min[k] - 0.5 * h,
                step: h,
                cells: 1,
            }
        }
    }))
}

/// No cell found for a grid cell yet.
const NONE: u64 = u64::MAX;

/// Read the preview grid of the blocked VTKHDF file at `path`, with
/// `longest` cells along the longest side of its blocks' box: a mesh of
/// one hexahedron for each grid cell inside the volume, with the cell
/// fields of the file there, and where those cells lie. BLOCK004 for a
/// file without a block index.
pub fn read_preview(
    path: &Path,
    longest: usize,
) -> Result<(Mesh, Results, PreviewGrid), MeshError> {
    if !path.is_file() {
        return Err(MeshError::new(
            "MESH001",
            format!("{} is not a readable file", path.display()),
        ));
    }
    let io = crate::profile::span("read.file");
    let h = H5::open(path)?;
    drop(io);
    let t = crate::profile::span("read.preview");
    let read = preview_parts(&h, longest);
    drop(t);
    drop(h);
    let (mut mesh, results, grid) = read?;
    mesh.files = vec![path.to_path_buf()];
    Ok((mesh, results, grid))
}

fn preview_parts(h: &H5, longest: usize) -> Result<(Mesh, Results, PreviewGrid), MeshError> {
    let (_, (volumes, _)) = blocks_of(h)?;
    let ix = index_of(h, &volumes)?.ok_or_else(not_blocked)?;
    let g = &ix.grid;
    let nb = ix.owned.len();
    let mut bounds = Bounds::empty();
    for bb in ix.bounds.as_chunks::<6>().0 {
        let b = Bounds {
            min: [bb[0], bb[1], bb[2]],
            max: [bb[3], bb[4], bb[5]],
        };
        if !b.is_empty() && bb.iter().all(|v| v.is_finite()) {
            bounds.add(b.min);
            bounds.add(b.max);
        }
    }
    let Some(axes) = axes_of(&bounds, longest) else {
        return Err(bad(
            "the blocks' bounds have no extent, so there is no preview grid",
        ));
    };
    let shape = axes.map(|a| a.cells);
    let grid_cells = shape[0] * shape[1] * shape[2];

    let np = ints(h, g, "NumberOfPoints")?;
    let nc = ints(h, g, "NumberOfCells")?;
    let ncon = ints(h, g, "NumberOfConnectivityIds")?;
    if np.len() != nb || nc.len() != nb || ncon.len() != nb {
        return Err(bad(
            "NumberOfPoints, NumberOfCells and NumberOfConnectivityIds do not have one value per block",
        ));
    }
    let parts = cells::parts(&np, &nc, &ncon, None);
    let total_points: usize = np
        .iter()
        .map(|&n| count(n, "points"))
        .sum::<Result<_, _>>()?;
    let total_rows: usize = ix.rows.iter().sum();
    if total_rows >= u32::MAX as usize {
        return Err(bad(format!(
            "{total_rows} cell rows are more than the preview grid numbers"
        )));
    }
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
    let offsets = dataset_of(h, g, "Offsets")?;
    let conn = dataset_of(h, g, "Connectivity")?;

    // The nearest centroid of each grid cell: its squared distance's bits
    // above the row of its cell.
    let best: Vec<AtomicU64> = (0..grid_cells).map(|_| AtomicU64::new(NONE)).collect();
    let t = crate::profile::span("read.preview.cells");
    let jobs: Vec<usize> = (0..nb).filter(|&b| ix.owned[b] > 0).collect();
    for batch in jobs.chunks(BATCH) {
        cells::in_runs(batch.to_vec(), |b| {
            let (part, own) = (parts[b], ix.owned[b]);
            let xyz = h.values_in(
                &points,
                3 * part.point_at..3 * (part.point_at + part.points),
            )?;
            let offs = h.int_values_in(&offsets, part.offset_at..part.offset_at + own + 1)?;
            if xyz.held().end < 3 * (part.point_at + part.points)
                || offs.held().end < part.offset_at + own + 1
            {
                return Err(bad("Points or Offsets is shorter than the blocks say"));
            }
            let at = |i: usize| offs.int(part.offset_at + i);
            let (first, last) = (at(0), at(own));
            if first < 0 || last < first || last > part.conn {
                return Err(MeshError::new(
                    "MESH006",
                    format!("block {b}: Offsets run outside its connectivity"),
                ));
            }
            let span = part.conn_at + first as usize..part.conn_at + last as usize;
            let ids = h.int_values_in(&conn, span.clone())?;
            if ids.held().end < span.end {
                return Err(bad("Connectivity is shorter than the blocks say"));
            }
            for i in 0..own {
                let (a, z) = (at(i), at(i + 1));
                if a < first || z < a || z > last {
                    return Err(MeshError::new(
                        "MESH006",
                        format!("block {b}: the Offsets of cell {i} do not ascend"),
                    ));
                }
                let mut lo = [f64::INFINITY; 3];
                let mut hi = [f64::NEG_INFINITY; 3];
                let mut sum = [0.0; 3];
                for q in a..z {
                    let p = ids.int(part.conn_at + q as usize);
                    let Some(p) = usize::try_from(p).ok().filter(|&p| p < part.points) else {
                        return Err(MeshError::new(
                            "MESH006",
                            format!("block {b}: cell {i} names point {p} of {}", part.points),
                        ));
                    };
                    let x = super::xyz_at(&xyz, part.point_at + p);
                    for k in 0..3 {
                        lo[k] = lo[k].min(x[k]);
                        hi[k] = hi[k].max(x[k]);
                        sum[k] += x[k];
                    }
                }
                if z == a {
                    continue;
                }
                let c = sum.map(|s| s / (z - a) as f64);
                if !c.iter().all(|v| v.is_finite()) {
                    continue;
                }
                let row = (part.cell_at + i) as u64;
                let [sx, sy, sz] = [0, 1, 2].map(|k| axes[k].span(lo[k], hi[k]));
                for kz in sz {
                    let dz = axes[2].centre(kz) - c[2];
                    for ky in sy.clone() {
                        let dy = axes[1].centre(ky) - c[1];
                        let base = (kz * shape[1] + ky) * shape[0];
                        for kx in sx.clone() {
                            let dx = axes[0].centre(kx) - c[0];
                            let d2 = (dx * dx + dy * dy + dz * dz) as f32;
                            let key = (u64::from(d2.to_bits()) << 32) | row;
                            best[base + kx].fetch_min(key, Relaxed);
                        }
                    }
                }
            }
            Ok(())
        })
        .into_iter()
        .collect::<Result<(), MeshError>>()?;
        // a read in the background stops between the batches
        crate::profile::check()?;
    }
    drop(t);

    // The grid cells inside, in grid order, and the rows they show.
    let mut voxels = Vec::new();
    let mut rows = Vec::new();
    for (v, b) in best.iter().enumerate() {
        let key = b.load(Relaxed);
        if key != NONE {
            voxels.push(v as u32);
            rows.push(key as u32);
        }
    }
    drop(best);
    let (mut owned_at, mut row_at) = (Vec::with_capacity(nb), Vec::with_capacity(nb));
    let (mut o, mut r) = (0usize, 0usize);
    for (&owned, &n) in ix.owned.iter().zip(&ix.rows) {
        owned_at.push(o);
        row_at.push(r);
        o += owned;
        r += n;
    }
    let file_cells: Vec<u64> = rows
        .iter()
        .map(|&row| {
            let row = row as usize;
            let blk = row_at.partition_point(|&a| a <= row) - 1;
            (owned_at[blk] + row - row_at[blk]) as u64
        })
        .collect();

    // The fields and the zones at those rows.
    let t = crate::profile::span("read.preview.fields");
    let mut fields = Vec::new();
    for (name, ds, comps) in field_sets(h, g, "CellData", total_rows, &CELL_RESERVED)? {
        let values = rows_at(h, &ds, comps, &rows)?;
        fields.extend(as_fields(&name, Location::Cell, comps, values));
        crate::profile::check()?;
    }
    let mut b = Builder::default();
    let mut roles = std::collections::BTreeMap::new();
    names_of(h, g, &mut b, &mut roles)?;
    let zones: Vec<u32> = match h.child(g, "CellData")? {
        Some(cd) => match h.child(&cd, "zone")? {
            Some(o) => {
                let ds = h.dataset(&o)?;
                rows_at(h, &ds, 1, &rows)?
                    .into_iter()
                    .map(|z| {
                        if z >= 0.0 && z < u32::MAX as f64 {
                            z as u32
                        } else {
                            0
                        }
                    })
                    .collect()
            }
            None => vec![0; rows.len()],
        },
        None => vec![0; rows.len()],
    };
    drop(t);

    let t = crate::profile::span("read.preview.mesh");
    let mut mesh = hexahedra(&axes, &voxels);
    mesh.cell_zone = zones;
    let zone_max = mesh.cell_zone.iter().copied().max();
    mesh.zones = super::zone_list(zone_max, &b.zone_names);
    drop(t);
    let n = mesh.cell_count();
    Ok((
        mesh,
        Results {
            fields,
            ..Results::default()
        },
        PreviewGrid {
            shape,
            origin: axes.map(|a| a.start),
            step: axes.map(|a| a.step),
            voxels,
            file_cells: {
                debug_assert_eq!(file_cells.len(), n);
                file_cells
            },
        },
    ))
}

/// A mesh of one hexahedron for each of grid cells `voxels` (x fastest)
/// of the grid `axes`, its faces turned out of it, linked where they
/// meet; the faces of the grid's outside are left open.
fn hexahedra(axes: &[Axis; 3], voxels: &[u32]) -> Mesh {
    let [nx, ny, _] = axes.map(|a| a.cells);
    let (px, py) = (nx + 1, ny + 1);
    let corner = |i: usize, j: usize, k: usize| (k * py + j) * px + i;
    let corners = px * py * (axes[2].cells + 1);
    let mut id = vec![u32::MAX; corners];
    let mut mesh = Mesh {
        boundary: Polygons::new(),
        cell_faces: Vec::with_capacity(voxels.len() + 1),
        ..Mesh::default()
    };
    mesh.cell_faces.push(0);
    let mut polys = Polygons::new();
    polys.start.reserve(6 * voxels.len());
    polys.nodes.reserve(24 * voxels.len());
    // The corners of each face, turned out of the cell (unit offsets).
    const FACES: [[[usize; 3]; 4]; 6] = [
        [[0, 0, 0], [0, 0, 1], [0, 1, 1], [0, 1, 0]],
        [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]],
        [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]],
        [[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]],
        [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]],
        [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]],
    ];
    for &v in voxels {
        let v = v as usize;
        let (i, j, k) = (v % nx, (v / nx) % ny, v / (nx * ny));
        for face in &FACES {
            polys.push(face.iter().map(|o| {
                let (a, b, c) = (i + o[0], j + o[1], k + o[2]);
                let q = corner(a, b, c);
                if id[q] == u32::MAX {
                    id[q] = mesh.points.len() as u32;
                    mesh.points.push([
                        axes[0].start + a as f64 * axes[0].step,
                        axes[1].start + b as f64 * axes[1].step,
                        axes[2].start + c as f64 * axes[2].step,
                    ]);
                }
                id[q]
            }));
        }
        mesh.cell_faces.push(polys.len() as u32);
        mesh.cell_kind.push(CellKind::Hexahedron);
    }
    mesh.metadata
        .insert("generator".into(), "murmel preview grid".into());
    boundary::link(&mut mesh, polys, None);
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Field;
    use crate::blocks::write_blocked;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("murmel-preview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// Invariant: each axis of the grid spans the box with near-cube
    /// cells, `longest` (at most [`MOST_CELLS`]) along the longest side,
    /// and a cell takes the grid cells whose centres its box holds,
    /// borders included. Breaks when the cells are counted by flooring (a
    /// side of 2.6 cells would get 2), when a box ending on a centre
    /// leaves that centre out, or when the cap goes; the reader test does
    /// not look at single cells or large grids.
    #[test]
    fn the_grid_spans_the_box_in_near_cubes() {
        let b = Bounds {
            min: [0.0, 0.0, 5.0],
            max: [4.0, 2.6, 5.0],
        };
        let [x, y, z] = axes_of(&b, 4).unwrap();
        assert_eq!([x.cells, y.cells, z.cells], [4, 3, 1]);
        assert!((y.step - 2.6 / 3.0).abs() < 1e-12);
        // a flat side is one cell thick about the box
        assert_eq!((z.start, z.step, z.centre(0)), (4.5, 1.0, 5.0));
        assert_eq!(x.span(0.5, 1.5), 0..2);
        assert!(x.span(0.6, 1.4).is_empty());
        assert!(x.span(-3.0, 0.4).is_empty());
        assert_eq!(x.span(2.5, 99.0), 2..4);
        assert_eq!(z.span(5.0, 5.0), 0..1);
        assert!(axes_of(&Bounds::empty(), 4).is_none());
        assert_eq!(axes_of(&b, 100_000).unwrap()[0].cells, MOST_CELLS);
    }

    /// The honeycomb of `n` with cell field `x` (each cell's centroid x)
    /// and `zone` by x, written blocked in blocks of `per` cells.
    fn blocked(name: &str, n: usize, per: usize) -> (std::path::PathBuf, Mesh) {
        let m = crate::synth::honeycomb(n);
        let xs: Vec<f64> = (0..m.cell_count())
            .map(|c| {
                let pts: Vec<u32> = m.cell_points(c).collect();
                pts.iter().map(|&p| m.points[p as usize][0]).sum::<f64>() / pts.len() as f64
            })
            .collect();
        let r = Results {
            fields: vec![Field {
                name: "x".into(),
                unit: None,
                location: Location::Cell,
                components: 1,
                values: xs,
            }],
            ..Results::default()
        };
        let path = scratch(name);
        write_blocked(&path, &m, &r, per).unwrap();
        (path, m)
    }

    /// Invariant: every grid cell whose centre lies in a cell's box shows
    /// the cell whose centroid is nearest among those (the cell holding
    /// it, in this Voronoi mesh), with that cell's values and zone; a
    /// centre in no cell's box has no grid cell; the result is the same
    /// on one thread and on four. Breaks when the grid takes the first or
    /// the last cell to reach it rather than the nearest (blocks run in
    /// any order on threads), when it reads ghost cells, or when the
    /// rows found are mapped to the wrong file cells. No other test reads
    /// a preview grid.
    #[test]
    fn grid_cells_show_the_cell_nearest_their_centre() {
        let (path, m) = blocked("honeycomb.vtkhdf", 5, 37);
        let (mesh, results, grid) = read_preview(&path, 12).unwrap();
        let again = crate::par::with_threads(1, || read_preview(&path, 12).unwrap());
        assert_eq!(again.2, grid);
        assert_eq!(again.1.fields[0].values, results.fields[0].values);
        // The file's cells in the writer's Hilbert order: map them back
        // through the file read whole.
        let (whole, wr) = crate::vtkhdf::read_vtkhdf(&path).unwrap();
        let x = &wr.fields.iter().find(|f| f.name == "x").unwrap().values;
        let cells: Vec<(Bounds, [f64; 3])> = (0..whole.cell_count())
            .map(|c| {
                let mut b = Bounds::empty();
                let mut s = [0.0; 3];
                let pts: Vec<u32> = whole.cell_points(c).collect();
                for &p in &pts {
                    b.add(whole.points[p as usize]);
                    for k in 0..3 {
                        s[k] += whole.points[p as usize][k];
                    }
                }
                (b, s.map(|v| v / pts.len() as f64))
            })
            .collect();
        let [nx, ny, nz] = grid.shape;
        assert_eq!(nx, 12);
        let holds = |b: &Bounds, p: [f64; 3]| (0..3).all(|q| b.min[q] <= p[q] && p[q] <= b.max[q]);
        let dist =
            |c: usize, p: [f64; 3]| -> f64 { (0..3).map(|q| (p[q] - cells[c].1[q]).powi(2)).sum() };
        let mut inside = 0;
        for v in 0..nx * ny * nz {
            let p = grid.centre(v);
            let nearest = (0..cells.len())
                .filter(|&c| holds(&cells[c].0, p))
                .map(|c| dist(c, p))
                .min_by(|a, b| a.total_cmp(b));
            let at = grid.voxels.binary_search(&(v as u32));
            match nearest {
                None => assert!(at.is_err(), "grid cell {v} is outside"),
                Some(d) => {
                    inside += 1;
                    let at = at.unwrap();
                    // the cell shown, or one as near (ties fall either way)
                    let c = grid.file_cells[at] as usize;
                    assert!(holds(&cells[c].0, p), "grid cell {v}");
                    assert!(dist(c, p) <= d * (1.0 + 1e-6) + 1e-12, "grid cell {v}");
                    assert_eq!(results.fields[0].values[at], x[c]);
                    assert_eq!(mesh.cell_zone[at], whole.cell_zone[c]);
                }
            }
        }
        assert_eq!(mesh.cell_count(), inside);
        assert!(inside > nx * ny * nz / 2, "{inside}");
        assert_eq!(mesh.zones, whole.zones);
        // The hexahedra fill their grid cells and meet their neighbours.
        let hexes = mesh.cell_kind.iter().all(|&k| k == CellKind::Hexahedron);
        assert!(hexes);
        let shared = mesh
            .face_neighbour
            .iter()
            .filter(|&&n| n != crate::NO_CELL)
            .count();
        assert!(shared > 2 * inside, "{shared} faces between cells");
        assert_eq!(m.cell_count(), whole.cell_count());
    }

    /// Invariant: a file without a block index has no preview grid
    /// (BLOCK004), as it has no overview. Breaks when the reader takes the
    /// first volume block of any file as blocked.
    #[test]
    fn a_file_without_blocks_has_no_preview() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
        let e = read_preview(&p, 16).unwrap_err();
        assert_eq!(e.code, "BLOCK004");
    }
}
