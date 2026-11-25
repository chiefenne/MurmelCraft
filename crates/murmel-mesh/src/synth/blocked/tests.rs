use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use super::*;
use crate::field::{Field, Location, Results};
use crate::hdf5::H5;
use crate::{Mesh, NO_CELL};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("murmel-synth-blocked-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn quiet(_: &str, _: usize, _: usize) {}

/// The cells of `honeycomb(n)` in its order, by centre.
fn centres(l: &Lattice) -> Vec<[i32; 3]> {
    (0..l.slots() as u32)
        .map(|c| l.centre(c))
        .filter(|&c| l.exists(c))
        .collect()
}

/// `honeycomb(n)` with the fields the streamed file carries.
fn reference(n: usize) -> (Mesh, Results) {
    let m = super::super::honeycomb(n);
    let l = Lattice::new(n);
    let (mut p, mut u) = (Vec::new(), Vec::new());
    for c in centres(&l) {
        let (v, q) = flow(l.xyz(c));
        p.push(q);
        u.extend(v);
    }
    let r = Results {
        fields: vec![
            Field {
                name: "pressure".into(),
                unit: Some("Pa".into()),
                location: Location::Cell,
                components: 1,
                values: p,
            },
            Field {
                name: "velocity".into(),
                unit: Some("m/s".into()),
                location: Location::Cell,
                components: 3,
                values: u,
            },
        ],
        ..Results::default()
    };
    (m, r)
}

fn bits(p: [f64; 3]) -> [u64; 3] {
    p.map(f64::to_bits)
}

/// The points of cell `c`, as exact bits.
fn corners(m: &Mesh, c: usize) -> BTreeSet<[u64; 3]> {
    m.cell_points(c)
        .map(|p| bits(m.points[p as usize]))
        .collect()
}

/// A dataset of `group` (a path under `/VTKHDF/internal`) as numbers.
fn ints(h: &H5, group: &str, name: &str) -> Vec<i64> {
    let g = h.path(&format!("/VTKHDF/internal{group}")).unwrap();
    h.read_i64(&h.dataset(&h.child(&g, name).unwrap().unwrap()).unwrap())
        .unwrap()
}

fn floats(h: &H5, group: &str, name: &str) -> Vec<f64> {
    let g = h.path(&format!("/VTKHDF/internal{group}")).unwrap();
    h.read_f64(&h.dataset(&h.child(&g, name).unwrap().unwrap()).unwrap())
        .unwrap()
}

/// Each partition cell of the file as its corners and its faces (each a
/// cycle of corners, turned to start at its least), in coordinates; and
/// each block's points with their ghost types.
type Cells = Vec<(BTreeSet<[u64; 3]>, BTreeSet<Vec<[u64; 3]>>)>;
type Points = Vec<BTreeMap<[u64; 3], i64>>;

fn partitions(path: &Path) -> (Cells, Points) {
    let h = H5::open(path).unwrap();
    let (np, nc) = (
        ints(&h, "", "NumberOfPoints"),
        ints(&h, "", "NumberOfCells"),
    );
    let xyz = floats(&h, "", "Points");
    let (offs, conn) = (ints(&h, "", "Offsets"), ints(&h, "", "Connectivity"));
    let (po, p2f) = (
        ints(&h, "", "PolyhedronOffsets"),
        ints(&h, "", "PolyhedronToFaces"),
    );
    let (fo, fc) = (
        ints(&h, "", "FaceOffsets"),
        ints(&h, "", "FaceConnectivity"),
    );
    let (nf, nfc) = (
        ints(&h, "", "NumberOfFaces"),
        ints(&h, "", "NumberOfFaceConnectivityIds"),
    );
    let ghosts = ints(&h, "/PointData", "vtkGhostType");
    let (mut cells, mut points) = (Vec::new(), Vec::new());
    let (mut pb, mut ob, mut kb, mut fb, mut cb, mut p2b) = (0, 0, 0, 0, 0, 0);
    for b in 0..np.len() {
        let at = |local: i64| bits(std::array::from_fn(|k| xyz[3 * (pb + local as usize) + k]));
        points.push(
            (0..np[b] as usize)
                .map(|i| (at(i as i64), ghosts[pb + i]))
                .collect(),
        );
        for c in 0..nc[b] as usize {
            let pts = (offs[ob + c]..offs[ob + c + 1])
                .map(|i| at(conn[kb + i as usize]))
                .collect();
            let faces = (po[ob + c]..po[ob + c + 1])
                .map(|i| {
                    let f = p2f[p2b + i as usize] as usize;
                    let mut cyc: Vec<[u64; 3]> = (fo[fb + f]..fo[fb + f + 1])
                        .map(|j| at(fc[cb + j as usize]))
                        .collect();
                    let least = (0..cyc.len()).min_by_key(|&i| cyc[i]).unwrap();
                    cyc.rotate_left(least);
                    cyc
                })
                .collect();
            cells.push((pts, faces));
        }
        kb += offs[ob + nc[b] as usize] as usize;
        pb += np[b] as usize;
        ob += nc[b] as usize + 1;
        fb += nf[b] as usize + 1;
        cb += nfc[b] as usize;
        p2b += nf[b] as usize;
    }
    (cells, points)
}

/// Invariant: the lattice numbers each cell of `honeycomb(n)` by its
/// place in the honeycomb's own order, and each point once, densely
/// enough (below 12 n² (n + 1)) and reversibly. Breaks when a point gets
/// two numbers (the blocks then would not join) or two points one, or a
/// cell's number is not the honeycomb's. The file test below sees the
/// numbering only through what it reads back; this one names the cause.
#[test]
fn the_lattice_numbers_every_cell_and_point_once() {
    for n in [2, 5, 6] {
        let m = super::super::honeycomb(n);
        let l = Lattice::new(n);
        let shape = Shape::new();
        let cs = centres(&l);
        assert_eq!(cs.len(), m.cell_count());
        let mut ids = BTreeSet::new();
        for (i, &c) in cs.iter().enumerate() {
            assert_eq!(l.number(c), i as u64, "n {n}, cell {c:?}");
            // the honeycomb's cell i is centred there
            let pts: Vec<[f64; 3]> = m.cell_points(i).map(|p| m.points[p as usize]).collect();
            let mean: [f64; 3] =
                std::array::from_fn(|k| pts.iter().map(|p| p[k]).sum::<f64>() / pts.len() as f64);
            let want = l.xyz(c);
            assert!(
                (0..3).all(|k| (mean[k] - want[k]).abs() < 1e-12),
                "{mean:?} {want:?}"
            );
            assert_eq!(l.code(c).map(|k| l.centre(k)), Some(c));
            for p in &shape.corners {
                let q = add(c, *p);
                let id = l.point_id(q);
                assert_eq!(l.point_of(id), q);
                ids.insert(id);
            }
        }
        assert_eq!(ids.len(), m.points.len(), "n {n}");
        let nn = n as u32;
        assert!(ids.iter().all(|&id| id < 12 * nn * nn * (nn + 1)));
        // the first-use order is a permutation of the corners
        let mut used = shape.first_use.clone();
        used.sort_unstable();
        assert_eq!(used, (0..24).collect::<Vec<_>>());
    }
}

/// Invariant: the streamed file of `honeycomb(6)` in blocks of 40 cells
/// holds what `blocks::write_blocked` writes for the honeycomb in memory
/// with the same fields: the same blocks (owned cells in the same file
/// order, the same ghosts, the same points with the same ghost types),
/// each cell with the same corners and faces (turning the same way),
/// the same zones, cell ids, field rows, block index and size. It reads
/// back whole as the same mesh with the same patches, and blocks 1 and 4
/// with their halo read alike. Only the numbering of the points
/// differs. Breaks when a block misses a ghost or a point, a ghost type
/// or a range is wrong, a row lands in another block's place, or the
/// point numbers do not join the blocks. Nothing else compares the
/// streamed writer with the one the converter uses.
#[test]
fn a_streamed_file_holds_what_the_mesh_writer_writes() {
    let (m, r) = reference(6);
    let mine = scratch("streamed.vtkhdf");
    let theirs = scratch("in-memory.vtkhdf");
    let ws = write_blocked(&mine, 6, 40, LocalIds::I64, &quiet).unwrap();
    let wr = blocks::write_blocked(&theirs, &m, &r, 40).unwrap();
    assert!(!PathBuf::from(format!("{}.part", mine.display())).exists());
    assert_eq!(ws, wr);
    assert_eq!((ws.blocks, ws.cells, ws.polyhedra), (9, 341, 341));
    assert!(ws.ghost_cells > 0 && ws.block_points > ws.points);

    let (a, b) = (H5::open(&mine).unwrap(), H5::open(&theirs).unwrap());
    for name in [
        "NumberOfPoints",
        "NumberOfCells",
        "NumberOfConnectivityIds",
        "NumberOfFaces",
        "NumberOfPolyhedronToFaceIds",
        "NumberOfFaceConnectivityIds",
    ] {
        assert_eq!(ints(&a, "", name), ints(&b, "", name), "{name}");
    }
    for name in ["zone", "GlobalCellIds", "OriginalCellIds", "vtkGhostType"] {
        assert_eq!(
            ints(&a, "/CellData", name),
            ints(&b, "/CellData", name),
            "{name}"
        );
    }
    for name in ["pressure", "velocity"] {
        assert_eq!(
            floats(&a, "/CellData", name),
            floats(&b, "/CellData", name),
            "{name}"
        );
    }
    assert_eq!(
        ints(&a, "/FieldData", "BlockOwnedCells"),
        ints(&b, "/FieldData", "BlockOwnedCells")
    );
    assert_eq!(
        floats(&a, "/FieldData", "BlockBounds"),
        floats(&b, "/FieldData", "BlockBounds")
    );
    for name in ["BlockRange_pressure", "BlockRange_velocity"] {
        let (x, y) = (
            floats(&a, "/FieldData", name),
            floats(&b, "/FieldData", name),
        );
        assert_eq!(x.len(), y.len());
        for (p, q) in x.iter().zip(&y) {
            assert!((p - q).abs() <= 1e-12 * q.abs().max(1.0), "{name}: {p} {q}");
        }
    }
    let (cells_a, points_a) = partitions(&mine);
    let (cells_b, points_b) = partitions(&theirs);
    assert_eq!(points_a, points_b);
    assert_eq!(cells_a.len(), cells_b.len());
    for (i, (x, y)) in cells_a.iter().zip(&cells_b).enumerate() {
        assert_eq!(x, y, "partition cell {i}");
    }

    // whole
    let (sm, sr) = crate::read_results(&mine).unwrap();
    let (rm, rr) = crate::read_results(&theirs).unwrap();
    assert_eq!(sm.cell_count(), rm.cell_count());
    assert_eq!(sm.points.len(), rm.points.len());
    assert_eq!(sm.zones, rm.zones);
    assert_eq!(sm.cell_zone, rm.cell_zone);
    for c in 0..sm.cell_count() {
        assert_eq!(corners(&sm, c), corners(&rm, c), "cell {c}");
    }
    assert_eq!(sm.patches, rm.patches);
    for p in 0..rm.patches.len() as u32 {
        let faces = |mesh: &Mesh| -> Vec<(BTreeSet<[u64; 3]>, u32)> {
            (0..mesh.boundary.len())
                .filter(|&i| mesh.boundary_patch[i] == p)
                .map(|i| {
                    let pts = mesh
                        .boundary
                        .get(i)
                        .iter()
                        .map(|&q| bits(mesh.points[q as usize]))
                        .collect();
                    (pts, mesh.boundary_owner[i])
                })
                .collect()
        };
        assert_eq!(faces(&sm), faces(&rm), "patch {p}");
        assert!(!faces(&sm).is_empty());
    }
    assert!(sm.boundary_owner.iter().all(|&c| c != NO_CELL));
    for f in &rr.fields {
        let g = sr.get(&f.name).unwrap();
        assert_eq!(
            (&g.unit, g.components, &g.values),
            (&f.unit, f.components, &f.values)
        );
    }
    assert_eq!(
        sm.metadata.get("generator").map(String::as_str),
        Some("murmel synth honeycomb n=6")
    );

    // two blocks and their halo
    let (sb, _, ss) = crate::vtkhdf::read_blocks(&mine, &[1, 4], true).unwrap();
    let (rb, _, rs) = crate::vtkhdf::read_blocks(&theirs, &[1, 4], true).unwrap();
    assert_eq!((&ss.file_cells, &ss.halo), (&rs.file_cells, &rs.halo));
    assert_eq!(sb.points.len(), rb.points.len());
    for c in 0..sb.cell_count() {
        assert_eq!(corners(&sb, c), corners(&rb, c), "cell {c} of blocks 1, 4");
    }
    std::fs::remove_file(&mine).ok();
    std::fs::remove_file(&theirs).ok();
}

/// Invariant: with 32-bit block ids the file reads back as the same
/// mesh as with 64-bit ones, the six datasets of ids within a block are
/// 32-bit, the cell and point ids of the whole mesh stay 64-bit, and the
/// file is 30 % smaller or more. Breaks when a narrow id is written
/// at a wide one's address or read with the wrong width.
#[test]
fn narrow_block_ids_read_as_wide_ones() {
    let wide = scratch("wide.vtkhdf");
    let narrow = scratch("narrow.vtkhdf");
    let ww = write_blocked(&wide, 6, 40, LocalIds::I64, &quiet).unwrap();
    let wn = write_blocked(&narrow, 6, 40, LocalIds::I32, &quiet).unwrap();
    assert_eq!(
        Written {
            bytes: 0,
            ..ww.clone()
        },
        Written {
            bytes: 0,
            ..wn.clone()
        }
    );
    assert!(wn.bytes * 10 < ww.bytes * 7, "{} of {}", wn.bytes, ww.bytes);
    assert_eq!(std::fs::metadata(&narrow).unwrap().len(), wn.bytes);
    let h = H5::open(&narrow).unwrap();
    let g = h.path("/VTKHDF/internal").unwrap();
    let size = |g: &crate::hdf5::Object, name: &str| {
        h.dataset(&h.child(g, name).unwrap().unwrap())
            .unwrap()
            .dtype
            .size()
    };
    for name in [
        "Offsets",
        "Connectivity",
        "FaceOffsets",
        "FaceConnectivity",
        "PolyhedronOffsets",
        "PolyhedronToFaces",
    ] {
        assert_eq!(size(&g, name), 4, "{name}");
    }
    let cd = h.path("/VTKHDF/internal/CellData").unwrap();
    let pd = h.path("/VTKHDF/internal/PointData").unwrap();
    assert_eq!(size(&cd, "GlobalCellIds"), 8);
    assert_eq!(size(&pd, "GlobalPointIds"), 8);
    let (a, ra) = crate::read_results(&wide).unwrap();
    let (b, rb) = crate::read_results(&narrow).unwrap();
    assert_eq!(a.points, b.points);
    assert_eq!(a.cell_faces, b.cell_faces);
    assert_eq!(a.boundary_owner, b.boundary_owner);
    for c in 0..a.cell_count() {
        assert_eq!(corners(&a, c), corners(&b, c));
    }
    assert_eq!(ra.fields, rb.fields);
    std::fs::remove_file(&wide).ok();
    std::fs::remove_file(&narrow).ok();
}

/// Invariant: the writer reports each step up to its total (the command
/// line shows it on a run of many minutes); a honeycomb whose point
/// numbers would not fit 32 bits is BLOCK001 and a block of no cells
/// BLOCK002, and neither leaves a file or a `.part` behind. Breaks when
/// a refusal writes first or the report stops short.
#[test]
fn the_writer_reports_its_steps_and_refuses_what_it_cannot_write() {
    let seen: Mutex<BTreeMap<String, (usize, usize)>> = Mutex::new(BTreeMap::new());
    let report = |step: &str, done: usize, total: usize| {
        let mut s = seen.lock().unwrap();
        let e = s.entry(step.to_string()).or_insert((0, total));
        e.0 = e.0.max(done);
    };
    let path = scratch("steps.vtkhdf");
    let w = write_blocked(&path, 3, 10, LocalIds::I32, &report).unwrap();
    let s = seen.into_inner().unwrap();
    assert_eq!(s.get("order"), Some(&(1, 1)));
    assert_eq!(s.get("count"), Some(&(w.blocks, w.blocks)));
    assert_eq!(s.get("write"), Some(&(w.blocks, w.blocks)));
    assert_eq!(s.get("patches"), Some(&(6, 6)));
    std::fs::remove_file(&path).ok();

    let big = scratch("big.vtkhdf");
    let e = write_blocked(&big, MAX_N + 1, 1 << 16, LocalIds::I32, &quiet).unwrap_err();
    assert_eq!(e.code, "BLOCK001");
    let e = write_blocked(&big, 3, 0, LocalIds::I32, &quiet).unwrap_err();
    assert_eq!(e.code, "BLOCK002");
    assert!(!big.exists());
    assert!(!PathBuf::from(format!("{}.part", big.display())).exists());
}
