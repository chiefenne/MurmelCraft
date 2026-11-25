//! The honeycomb of [`super::honeycomb`] written straight into a blocked
//! VTKHDF file (`docs/blocked-vtkhdf.md`) block by block, without the
//! mesh in memory: test files beyond any memory budget (100 M cells take
//! about 47 GB as a mesh model). Every cell, point and face of a block
//! follows from the lattice, so only the Hilbert order of the cells
//! (8 bytes a cell) and one block per thread are held.
//!
//! The file is the one [`crate::blocks::write_blocked`] writes for
//! `honeycomb(n)` with two cell fields, except for numbering: a point's
//! `GlobalPointIds` is its place on the lattice (with gaps where the
//! jagged outside has no point), and the points of a block and of a
//! polyhedron come in that order. The fields are a Taylor–Green vortex
//! array in a uniform stream (`velocity`, m/s) and its pressure
//! (`pressure`, Pa), so slices, iso-surfaces and streamlines have
//! something to show.
//!
//! A first pass counts each block's ghost cells and points; the datasets
//! are then set aside at their sizes ([`H5Writer::reserve`]) and every
//! thread writes its blocks' rows where they go.

use std::f64::consts::PI;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::blocks::{self, Written};
use crate::h5write::{AttrValue, Element, H5Writer, Link, Reserved, Rows, Ty};
use crate::vtu::POLYHEDRON;
use crate::{Bounds, MeshError, hilbert, par};

/// The largest honeycomb written, `n` = 700 (684,532,099 cells): its
/// point numbers and cell codes stay below 2³².
pub const MAX_N: usize = 700;

/// How the ids that count within a block (`Offsets`, `Connectivity` and
/// the polyhedron arrays) are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalIds {
    /// 64-bit, as § 5 of the layout writes every integer.
    I64,
    /// 32-bit: a block's ids are far below 2³¹, and the file is a third
    /// smaller.
    I32,
}

impl LocalIds {
    fn ty(self) -> Ty {
        match self {
            LocalIds::I64 => Ty::I64,
            LocalIds::I32 => Ty::I32,
        }
    }
}

/// What the writer reports as it goes: the step (`order`, `count`,
/// `write`, `patches`) and how far it is (done of total).
pub type Progress<'a> = &'a (dyn Fn(&str, usize, usize) + Sync);

/// The cells and points of `honeycomb(n)` by their place on the lattice,
/// in quarter units: corner cells at `4 (i, j, k)`, body cells at
/// `4 (i, j, k) + 2`. A cell's *code* is `2 site + body` with `site =
/// i + n (j + n k)`.
struct Lattice {
    n: u32,
    top: i32,
    unit: f64,
}

impl Lattice {
    fn new(n: usize) -> Lattice {
        let n = n.clamp(2, MAX_N) as u32;
        let top = 4 * (n as i32 - 1);
        Lattice {
            n,
            top,
            unit: 1.0 / f64::from(top),
        }
    }

    fn slots(&self) -> usize {
        2 * (self.n as usize).pow(3)
    }

    fn exists(&self, c: [i32; 3]) -> bool {
        let corner = c.iter().all(|v| v % 4 == 0);
        let body = c.iter().all(|v| v % 4 == 2);
        (corner || body) && c.iter().all(|&v| (0..=self.top).contains(&v))
    }

    fn code(&self, c: [i32; 3]) -> Option<u32> {
        if !self.exists(c) {
            return None;
        }
        let n = self.n;
        let s = c.map(|v| (v / 4) as u32);
        Some(2 * (s[0] + n * (s[1] + n * s[2])) + u32::from(c[0] % 4 == 2))
    }

    fn centre(&self, code: u32) -> [i32; 3] {
        let n = self.n;
        let site = code / 2;
        let off = 2 * (code % 2) as i32;
        [site % n, site / n % n, site / (n * n)].map(|v| 4 * v as i32 + off)
    }

    /// The cell's number in `honeycomb(n)`, which lists the cells by
    /// layer, row and column, a corner cell before the body cell after
    /// it.
    fn number(&self, c: [i32; 3]) -> u64 {
        let n = u64::from(self.n);
        let [i, j, k] = c.map(|v| (v / 4) as u64);
        let body = u64::from(c[0] % 4 == 2);
        let inner = k + 1 < n;
        let row_bodies = inner && j + 1 < n;
        k * (n * n + (n - 1) * (n - 1))
            + j * (n + if inner { n - 1 } else { 0 })
            + i * if row_bodies { 2 } else { 1 }
            + body
    }

    /// A point's number: each point of the honeycomb has one coordinate
    /// a multiple of 4 (axis `a`), one 2 more than a multiple of 4 (axis
    /// `b`) and one odd; the six ways to pick `a` and `b`, then the three
    /// coordinates, number it.
    fn point_id(&self, q: [i32; 3]) -> u32 {
        let (mut a, mut b) = (0, 0);
        for (k, v) in q.iter().enumerate() {
            match v.rem_euclid(4) {
                0 => a = k,
                2 => b = k,
                _ => {}
            }
        }
        let o = 3 - a - b;
        let class = (2 * a + b - usize::from(b > a)) as u32;
        let n = self.n;
        let (ia, ib, io) = (
            (q[a] / 4) as u32,
            ((q[b] + 2) / 4) as u32,
            ((q[o] + 1) / 2) as u32,
        );
        ((class * n + ia) * (n + 1) + ib) * (2 * n) + io
    }

    fn point_of(&self, id: u32) -> [i32; 3] {
        let n = self.n;
        let io = id % (2 * n);
        let r = id / (2 * n);
        let ib = r % (n + 1);
        let r = r / (n + 1);
        let ia = r % n;
        let class = (r / n) as usize;
        let a = class / 2;
        let b = class % 2 + usize::from(class % 2 >= a);
        let mut q = [0; 3];
        q[a] = 4 * ia as i32;
        q[b] = 4 * ib as i32 - 2;
        q[3 - a - b] = 2 * io as i32 - 1;
        q
    }

    fn xyz(&self, q: [i32; 3]) -> [f64; 3] {
        q.map(|v| f64::from(v) * self.unit)
    }

    /// Whether a cell can have a face on the outside: the outer layer.
    fn on_shell(&self, c: [i32; 3]) -> bool {
        c.iter().any(|&v| v <= 2 || v >= self.top - 2)
    }
}

/// What every cell has: its 24 corners, its 14 faces (corner indices
/// turning counter-clockwise seen from outside, with the offset to the
/// cell across), and its corners in the order its faces first use them
/// (a polyhedron's `Connectivity`).
struct Shape {
    corners: Vec<[i32; 3]>,
    faces: Vec<(Vec<usize>, [i32; 3])>,
    first_use: Vec<usize>,
    /// Face corners per cell (72).
    corner_uses: usize,
}

impl Shape {
    fn new() -> Shape {
        let corners = super::corners();
        let faces = super::cell_faces();
        let mut first_use: Vec<usize> = Vec::with_capacity(corners.len());
        for (f, _) in &faces {
            for &k in f {
                if !first_use.contains(&k) {
                    first_use.push(k);
                }
            }
        }
        let corner_uses = faces.iter().map(|(f, _)| f.len()).sum();
        Shape {
            corners,
            faces,
            first_use,
            corner_uses,
        }
    }
}

fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The cells in file order (by the Hilbert index of their centres, as
/// § 2 asks, ties by their number in `honeycomb(n)`) and the file cell
/// id of each code.
fn hilbert_order(l: &Lattice) -> (Vec<u32>, Vec<u32>) {
    let lo = l.xyz([-2; 3]);
    let hi = l.xyz([l.top + 2; 3]);
    let grid = hilbert::Grid::new(lo, hi);
    // codes ascend as the honeycomb's numbers do
    let mut keyed: Vec<(u64, u32)> = par::collect(l.slots(), 1 << 16, |r| {
        r.filter_map(|code| {
            let c = l.centre(code as u32);
            l.exists(c).then(|| (grid.index(l.xyz(c)), code as u32))
        })
        .collect()
    });
    keyed.sort_unstable();
    let order: Vec<u32> = keyed.iter().map(|&(_, c)| c).collect();
    drop(keyed);
    let mut file_id = vec![u32::MAX; l.slots()];
    for (f, &c) in order.iter().enumerate() {
        file_id[c as usize] = f as u32;
    }
    (order, file_id)
}

/// One block: its cells (owned, then the ghosts in file order), the file
/// cell id of each, its points by number (the local id is the position)
/// with whether another block owns each, and the local id of each
/// cell's corners.
struct Block {
    cells: Vec<u32>,
    file: Vec<u32>,
    owned: usize,
    points: Vec<u32>,
    duplicate: Vec<u8>,
    local: Vec<u32>,
}

struct Plan<'a> {
    l: &'a Lattice,
    shape: &'a Shape,
    order: &'a [u32],
    file_id: &'a [u32],
    bs: usize,
}

impl Plan<'_> {
    fn blocks(&self) -> usize {
        self.order.len().div_ceil(self.bs)
    }

    fn block(&self, b: usize) -> Block {
        let (l, bs) = (self.l, self.bs);
        let owned = &self.order[b * bs..((b + 1) * bs).min(self.order.len())];
        // the point neighbours of the owned cells in other blocks: in the
        // honeycomb the cells at a point are the cell and the three across
        // the faces meeting there, so these are the face neighbours
        let mut ghosts: Vec<u32> = Vec::new();
        for &code in owned {
            let c = l.centre(code);
            for (_, across) in &self.shape.faces {
                if let Some(o) = l.code(add(c, *across)) {
                    let f = self.file_id[o as usize];
                    if f as usize / bs != b {
                        ghosts.push(f);
                    }
                }
            }
        }
        ghosts.sort_unstable();
        ghosts.dedup();
        let mut cells = owned.to_vec();
        cells.extend(ghosts.iter().map(|&f| self.order[f as usize]));
        let mut file: Vec<u32> = (b * bs..b * bs + owned.len()).map(|f| f as u32).collect();
        file.extend(ghosts);
        // the points: (number, corner slot), sorted by number
        let k = self.shape.corners.len();
        let mut keys: Vec<u64> = Vec::with_capacity(cells.len() * k);
        for (i, &code) in cells.iter().enumerate() {
            let c = l.centre(code);
            for (j, p) in self.shape.corners.iter().enumerate() {
                let id = l.point_id(add(c, *p));
                keys.push((u64::from(id) << 32) | (i * k + j) as u64);
            }
        }
        keys.sort_unstable();
        let mut points = Vec::with_capacity(keys.len() / 3);
        let mut duplicate = Vec::with_capacity(keys.len() / 3);
        let mut local = vec![0u32; keys.len()];
        let mut s = 0;
        while s < keys.len() {
            let id = keys[s] >> 32;
            // a point is owned by the lowest block whose owned cells use
            // it; all its cells are here when an owned cell uses it
            let mut least = usize::MAX;
            let mut e = s;
            while e < keys.len() && keys[e] >> 32 == id {
                let slot = (keys[e] & 0xffff_ffff) as usize;
                local[slot] = points.len() as u32;
                least = least.min(file[slot / k] as usize / bs);
                e += 1;
            }
            points.push(id as u32);
            duplicate.push(u8::from(least != b));
            s = e;
        }
        Block {
            cells,
            file,
            owned: owned.len(),
            points,
            duplicate,
            local,
        }
    }
}

/// The flow: a Taylor–Green vortex array (two periods across the box) in
/// a uniform stream along x, and its pressure.
fn flow(x: [f64; 3]) -> ([f64; 3], f64) {
    let k = 4.0 * PI;
    let (sx, cx) = (k * x[0]).sin_cos();
    let (sy, cy) = (k * x[1]).sin_cos();
    let cz = (k * x[2]).cos();
    let a = 0.5;
    let u = [1.0 + a * sx * cy * cz, -a * cx * sy * cz, 0.0];
    let p = 1.2 * a * a / 16.0
        * ((2.0 * k * x[0]).cos() + (2.0 * k * x[1]).cos())
        * ((2.0 * k * x[2]).cos() + 2.0);
    (u, p)
}

fn magnitude(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// The datasets of `internal` that hold rows of every block.
struct Sets {
    points: Reserved,
    types: Reserved,
    offsets: Reserved,
    connectivity: Reserved,
    face_offsets: Reserved,
    face_connectivity: Reserved,
    polyhedron_offsets: Reserved,
    polyhedron_to_faces: Reserved,
    zone: Reserved,
    global_cells: Reserved,
    original_cells: Reserved,
    cell_ghosts: Reserved,
    pressure: Reserved,
    velocity: Reserved,
    global_points: Reserved,
    point_ghosts: Reserved,
}

/// What a block adds to the block index.
#[derive(Clone, Copy)]
struct Summary {
    bounds: Bounds,
    pressure: [f64; 2],
    speed: [f64; 2],
    owned_points: usize,
}

/// Local ids as stored.
fn put_ids(rows: &Rows, ids: LocalIds, addr: u64, v: &[i64]) -> io::Result<()> {
    match ids {
        LocalIds::I64 => rows.put(addr, v),
        LocalIds::I32 => {
            let narrow: Vec<i32> = v.iter().map(|&x| x as i32).collect();
            rows.put(addr, &narrow)
        }
    }
}

fn put<T: Element>(rows: &Rows, addr: u64, v: &[T]) -> io::Result<()> {
    rows.put(addr, v)
}

impl Plan<'_> {
    /// Write block `b`'s rows: its cells start at row `cell_at` of the
    /// cell datasets and its points at `point_at`.
    fn write(
        &self,
        rows: &Rows,
        sets: &Sets,
        ids: LocalIds,
        b: usize,
        cell_at: u64,
        point_at: u64,
    ) -> io::Result<Summary> {
        let (l, shape) = (self.l, self.shape);
        let blk = self.block(b);
        let nc = blk.cells.len();
        let k = shape.corners.len();
        let nf = shape.faces.len();
        let b64 = b as u64;
        // points
        let xyz: Vec<f64> = blk
            .points
            .iter()
            .flat_map(|&p| l.xyz(l.point_of(p)))
            .collect();
        put(rows, sets.points.at(point_at), &xyz)?;
        drop(xyz);
        let gids: Vec<i64> = blk.points.iter().map(|&p| i64::from(p)).collect();
        put(rows, sets.global_points.at(point_at), &gids)?;
        drop(gids);
        put(rows, sets.point_ghosts.at(point_at), &blk.duplicate)?;
        // cells
        put(rows, sets.types.at(cell_at), &vec![POLYHEDRON as u8; nc])?;
        let offsets: Vec<i64> = (0..=nc).map(|i| (i * k) as i64).collect();
        put_ids(rows, ids, sets.offsets.at(cell_at + b64), &offsets)?;
        let polys: Vec<i64> = (0..=nc).map(|i| (i * nf) as i64).collect();
        put_ids(rows, ids, sets.polyhedron_offsets.at(cell_at + b64), &polys)?;
        drop((offsets, polys));
        let mut conn: Vec<i64> = Vec::with_capacity(nc * k);
        for i in 0..nc {
            let at = &blk.local[i * k..(i + 1) * k];
            conn.extend(shape.first_use.iter().map(|&j| i64::from(at[j])));
        }
        put_ids(rows, ids, sets.connectivity.at(cell_at * k as u64), &conn)?;
        drop(conn);
        let to_faces: Vec<i64> = (0..(nc * nf) as i64).collect();
        put_ids(
            rows,
            ids,
            sets.polyhedron_to_faces.at(cell_at * nf as u64),
            &to_faces,
        )?;
        drop(to_faces);
        let mut face_offsets: Vec<i64> = Vec::with_capacity(nc * nf + 1);
        face_offsets.push(0);
        let mut o = 0i64;
        let mut face_conn: Vec<i64> = Vec::with_capacity(nc * shape.corner_uses);
        for i in 0..nc {
            let at = &blk.local[i * k..(i + 1) * k];
            for (f, _) in &shape.faces {
                o += f.len() as i64;
                face_offsets.push(o);
                face_conn.extend(f.iter().map(|&j| i64::from(at[j])));
            }
        }
        put_ids(
            rows,
            ids,
            sets.face_offsets.at(cell_at * nf as u64 + b64),
            &face_offsets,
        )?;
        put_ids(
            rows,
            ids,
            sets.face_connectivity
                .at(cell_at * shape.corner_uses as u64),
            &face_conn,
        )?;
        drop((face_offsets, face_conn));
        // cell data
        let centres: Vec<[i32; 3]> = blk.cells.iter().map(|&c| l.centre(c)).collect();
        let zone: Vec<u32> = centres
            .iter()
            .map(|c| u32::from(2 * c[0] > l.top))
            .collect();
        put(rows, sets.zone.at(cell_at), &zone)?;
        let global: Vec<i64> = blk.file.iter().map(|&f| i64::from(f)).collect();
        put(rows, sets.global_cells.at(cell_at), &global)?;
        let original: Vec<i64> = centres.iter().map(|&c| l.number(c) as i64).collect();
        put(rows, sets.original_cells.at(cell_at), &original)?;
        let ghost: Vec<u8> = (0..nc).map(|i| u8::from(i >= blk.owned)).collect();
        put(rows, sets.cell_ghosts.at(cell_at), &ghost)?;
        let mut pressure = Vec::with_capacity(nc);
        let mut velocity = Vec::with_capacity(3 * nc);
        let mut summary = Summary {
            bounds: Bounds::empty(),
            pressure: [f64::INFINITY, f64::NEG_INFINITY],
            speed: [f64::INFINITY, f64::NEG_INFINITY],
            owned_points: blk.duplicate.iter().filter(|&&d| d == 0).count(),
        };
        for (i, &c) in centres.iter().enumerate() {
            let (u, p) = flow(l.xyz(c));
            pressure.push(p);
            velocity.extend(u);
            if i < blk.owned {
                summary.bounds.add(l.xyz(add(c, [-2; 3])));
                summary.bounds.add(l.xyz(add(c, [2; 3])));
                let s = magnitude(u);
                summary.pressure = [summary.pressure[0].min(p), summary.pressure[1].max(p)];
                summary.speed = [summary.speed[0].min(s), summary.speed[1].max(s)];
            }
        }
        put(rows, sets.pressure.at(cell_at), &pressure)?;
        put(rows, sets.velocity.at(cell_at), &velocity)?;
        Ok(summary)
    }
}

/// Write `honeycomb(n)` with its fields as a blocked VTKHDF file at
/// `path` (through `path.part`, renamed when whole; removed when the
/// writing fails), `block_cells` owned cells per block.
pub fn write_blocked(
    path: &Path,
    n: usize,
    block_cells: usize,
    ids: LocalIds,
    progress: Progress<'_>,
) -> Result<Written, MeshError> {
    if block_cells == 0 {
        return Err(MeshError::new(
            "BLOCK002",
            "a block needs at least one cell",
        ));
    }
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let r = write(&part, n, block_cells, ids, progress)
        .and_then(|w| std::fs::rename(&part, path).map(|_| w));
    if r.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    r.map_err(|e| {
        MeshError::new(
            "BLOCK001",
            format!("the blocked file cannot be written: {e}"),
        )
    })
}

fn write(
    path: &Path,
    n: usize,
    bs: usize,
    ids: LocalIds,
    progress: Progress<'_>,
) -> io::Result<Written> {
    if n > MAX_N {
        return Err(io::Error::other(format!(
            "a honeycomb of more than {} cells has point numbers beyond 32 bits",
            super::cells_of(MAX_N)
        )));
    }
    let l = Lattice::new(n);
    let shape = Shape::new();
    progress("order", 0, 1);
    let (order, file_id) = hilbert_order(&l);
    progress("order", 1, 1);
    let plan = Plan {
        l: &l,
        shape: &shape,
        order: &order,
        file_id: &file_id,
        bs,
    };
    let nb = plan.blocks();
    let t = par::threads().clamp(1, nb.max(1));
    // pass 1: the cells (owned and ghosts) and points of each block
    let done = AtomicUsize::new(0);
    let mut counts = vec![(0usize, 0usize); nb];
    for part in par::map(t, 1, |r| {
        let mut out = Vec::new();
        for first in r {
            for b in (first..nb).step_by(t) {
                let blk = plan.block(b);
                out.push((b, blk.cells.len(), blk.points.len()));
                progress("count", done.fetch_add(1, Ordering::Relaxed) + 1, nb);
            }
        }
        out
    }) {
        for (b, c, p) in part {
            counts[b] = (c, p);
        }
    }
    let cell_at: Vec<u64> = counts
        .iter()
        .scan(0u64, |s, &(c, _)| {
            let a = *s;
            *s += c as u64;
            Some(a)
        })
        .collect();
    let point_at: Vec<u64> = counts
        .iter()
        .scan(0u64, |s, &(_, p)| {
            let a = *s;
            *s += p as u64;
            Some(a)
        })
        .collect();
    let cells: u64 = counts.iter().map(|&(c, _)| c as u64).sum();
    let points: u64 = counts.iter().map(|&(_, p)| p as u64).sum();
    let (k, nf, uses) = (
        shape.corners.len() as u64,
        shape.faces.len() as u64,
        shape.corner_uses as u64,
    );
    let nbu = nb as u64;

    let mut w = H5Writer::create(path)?;
    let per_block = |f: &dyn Fn(usize, usize) -> u64| -> Vec<i64> {
        counts.iter().map(|&(c, p)| f(c, p) as i64).collect()
    };
    let mut internal: Vec<(String, u64)> = Vec::new();
    internal.push((
        "NumberOfPoints".into(),
        w.dataset(&[nbu], &per_block(&|_, p| p as u64))?,
    ));
    internal.push((
        "NumberOfCells".into(),
        w.dataset(&[nbu], &per_block(&|c, _| c as u64))?,
    ));
    internal.push((
        "NumberOfConnectivityIds".into(),
        w.dataset(&[nbu], &per_block(&|c, _| c as u64 * k))?,
    ));
    let lid = ids.ty();
    let points_set = w.reserve(Ty::F64, &[points, 3])?;
    let types = w.reserve(Ty::U8, &[cells])?;
    let offsets = w.reserve(lid, &[cells + nbu])?;
    let connectivity = w.reserve(lid, &[cells * k])?;
    let faces = per_block(&|c, _| c as u64 * nf);
    let face_ids = per_block(&|c, _| c as u64 * uses);
    let face_offsets = w.reserve(lid, &[cells * nf + nbu])?;
    let face_connectivity = w.reserve(lid, &[cells * uses])?;
    let polyhedron_offsets = w.reserve(lid, &[cells + nbu])?;
    let polyhedron_to_faces = w.reserve(lid, &[cells * nf])?;
    let sets = Sets {
        points: points_set,
        types,
        offsets,
        connectivity,
        face_offsets,
        face_connectivity,
        polyhedron_offsets,
        polyhedron_to_faces,
        zone: w.reserve(Ty::U32, &[cells])?,
        global_cells: w.reserve(Ty::I64, &[cells])?,
        original_cells: w.reserve(Ty::I64, &[cells])?,
        cell_ghosts: w.reserve(Ty::U8, &[cells])?,
        pressure: w.reserve(Ty::F64, &[cells])?,
        velocity: w.reserve(Ty::F64, &[cells, 3])?,
        global_points: w.reserve(Ty::I64, &[points])?,
        point_ghosts: w.reserve(Ty::U8, &[points])?,
    };
    // pass 2: every block's rows, where they go
    let done = AtomicUsize::new(0);
    let mut summaries: Vec<Option<Summary>> = vec![None; nb];
    {
        let rows = w.rows()?;
        let parts = par::map(t, 1, |r| -> io::Result<Vec<(usize, Summary)>> {
            let mut out = Vec::new();
            for first in r {
                for b in (first..nb).step_by(t) {
                    let s = plan.write(&rows, &sets, ids, b, cell_at[b], point_at[b])?;
                    out.push((b, s));
                    progress("write", done.fetch_add(1, Ordering::Relaxed) + 1, nb);
                }
            }
            Ok(out)
        });
        for part in parts {
            for (b, s) in part? {
                summaries[b] = Some(s);
            }
        }
    }
    let summaries: Vec<Summary> = summaries.into_iter().flatten().collect();
    if summaries.len() != nb {
        return Err(io::Error::other("a block was not written"));
    }

    internal.push(("Points".into(), w.reserved_header(&sets.points)?));
    internal.push(("Types".into(), w.reserved_header(&sets.types)?));
    internal.push(("Offsets".into(), w.reserved_header(&sets.offsets)?));
    internal.push((
        "Connectivity".into(),
        w.reserved_header(&sets.connectivity)?,
    ));
    internal.push(("NumberOfFaces".into(), w.dataset(&[nbu], &faces)?));
    internal.push((
        "NumberOfPolyhedronToFaceIds".into(),
        w.dataset(&[nbu], &faces)?,
    ));
    internal.push((
        "NumberOfFaceConnectivityIds".into(),
        w.dataset(&[nbu], &face_ids)?,
    ));
    internal.push(("FaceOffsets".into(), w.reserved_header(&sets.face_offsets)?));
    internal.push((
        "FaceConnectivity".into(),
        w.reserved_header(&sets.face_connectivity)?,
    ));
    internal.push((
        "PolyhedronOffsets".into(),
        w.reserved_header(&sets.polyhedron_offsets)?,
    ));
    internal.push((
        "PolyhedronToFaces".into(),
        w.reserved_header(&sets.polyhedron_to_faces)?,
    ));
    let cd = vec![
        ("zone".to_string(), w.reserved_header(&sets.zone)?),
        (
            "GlobalCellIds".into(),
            w.reserved_header(&sets.global_cells)?,
        ),
        (
            "OriginalCellIds".into(),
            w.reserved_header(&sets.original_cells)?,
        ),
        ("vtkGhostType".into(), w.reserved_header(&sets.cell_ghosts)?),
        ("pressure".into(), w.reserved_header(&sets.pressure)?),
        ("velocity".into(), w.reserved_header(&sets.velocity)?),
    ];
    let pd = vec![
        (
            "GlobalPointIds".to_string(),
            w.reserved_header(&sets.global_points)?,
        ),
        (
            "vtkGhostType".into(),
            w.reserved_header(&sets.point_ghosts)?,
        ),
    ];
    // the block index
    let owned: Vec<i64> = (0..nb)
        .map(|b| (((b + 1) * bs).min(plan.order.len()) - b * bs) as i64)
        .collect();
    let bounds: Vec<f64> = summaries
        .iter()
        .flat_map(|s| s.bounds.min.into_iter().chain(s.bounds.max))
        .collect();
    let pressure: Vec<f64> = summaries.iter().flat_map(|s| s.pressure).collect();
    let speed: Vec<f64> = summaries.iter().flat_map(|s| s.speed).collect();
    let fd = vec![
        ("BlockOwnedCells".to_string(), w.dataset(&[nbu], &owned)?),
        ("BlockBounds".into(), w.dataset(&[nbu, 6], &bounds)?),
        (
            "BlockRange_pressure".into(),
            w.dataset(&[nbu, 2], &pressure)?,
        ),
        ("BlockRange_velocity".into(), w.dataset(&[nbu, 2], &speed)?),
        (
            "zone_names".into(),
            w.strings(&["0=fluid".to_string(), "1=solid".to_string()])?,
        ),
    ];
    let group = |w: &mut H5Writer, m: Vec<(String, u64)>| {
        let links: Vec<Link> = m.into_iter().map(|(n, a)| Link::Hard(n, a)).collect();
        w.group(&links, &[])
    };
    internal.push(("CellData".into(), group(&mut w, cd)?));
    internal.push(("PointData".into(), group(&mut w, pd)?));
    internal.push(("FieldData".into(), group(&mut w, fd)?));
    let links: Vec<Link> = internal
        .into_iter()
        .map(|(n, a)| Link::Hard(n, a))
        .collect();
    let internal = w.group(
        &links,
        &[
            ("Index", AttrValue::Int(0)),
            ("Type", AttrValue::Text("UnstructuredGrid".into())),
            ("Version", AttrValue::Ints(vec![2, 4])),
            ("block_cells", AttrValue::Int(bs as i64)),
        ],
    )?;

    // the boundary patches, the faces in the honeycomb's order
    let patches = boundary(&l, &shape);
    let mut groups: Vec<(String, u64)> = Vec::new();
    for (s, faces) in patches.iter().enumerate() {
        let mut start = Vec::with_capacity(faces.len() + 1);
        start.push(0);
        let mut nodes = Vec::new();
        let mut owner = Vec::with_capacity(faces.len());
        for &(code, f) in faces {
            let c = l.centre(code);
            let (corners, _) = &shape.faces[f as usize];
            nodes.extend(
                corners
                    .iter()
                    .map(|&j| l.point_id(add(c, shape.corners[j]))),
            );
            start.push(nodes.len());
            owner.push(i64::from(file_id[code as usize]));
        }
        let xyz = |p: u32| l.xyz(l.point_of(p));
        let (name, role) = super::PATCHES[s];
        let addr = blocks::patch_block(&mut w, &start, &nodes, &xyz, &owner, &[], s + 1, role)?;
        groups.push((name.to_string(), addr));
        progress("patches", s + 1, patches.len());
    }
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert(
        "generator".to_string(),
        format!("murmel synth honeycomb n={}", l.n),
    );
    let root_attrs = [("units", AttrValue::Text("pressure Pa; velocity m/s".into()))];
    let bytes = blocks::close(w, internal, &groups, &metadata, &root_attrs)?;
    let block_points = points as usize;
    Ok(Written {
        blocks: nb,
        block_cells: bs,
        cells: plan.order.len(),
        ghost_cells: cells as usize - plan.order.len(),
        points: summaries.iter().map(|s| s.owned_points).sum(),
        block_points,
        linear_cells: 0,
        polyhedra: plan.order.len(),
        patches: patches.len(),
        fields: 2,
        skipped_fields: Vec::new(),
        bytes,
    })
}

/// The boundary faces of each patch (the side of the box nearest the
/// face, as [`super::honeycomb`] picks it): the cell's code and the
/// face's index, in the honeycomb's order.
fn boundary(l: &Lattice, shape: &Shape) -> Vec<Vec<(u32, u8)>> {
    let mut by_side: Vec<Vec<(u32, u8)>> = vec![Vec::new(); super::PATCHES.len()];
    let top = l.top;
    for code in 0..l.slots() as u32 {
        let c = l.centre(code);
        if !l.on_shell(c) || !l.exists(c) {
            continue;
        }
        for (f, (corners, across)) in shape.faces.iter().enumerate() {
            if l.exists(add(c, *across)) {
                continue;
            }
            let m = corners.len() as i32;
            let fc: [i32; 3] = std::array::from_fn(|k| {
                corners
                    .iter()
                    .map(|&v| c[k] + shape.corners[v][k])
                    .sum::<i32>()
            });
            let side = (0..6)
                .min_by_key(|&s| {
                    let k = s / 2;
                    if s % 2 == 0 { fc[k] } else { top * m - fc[k] }
                })
                .unwrap_or(0);
            by_side[side].push((code, f as u8));
        }
    }
    by_side
}

#[cfg(test)]
mod tests;
