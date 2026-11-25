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
//!
//! Each step runs on all threads: the blocks' points are read at once,
//! joined by their ids shard by shard ([`join_blocks`]), and each block's
//! cells are read on a thread of its own (its windows share the threads
//! left over); the blocks are then put together in order.
//!
//! A [`BlockFile`] reads block after block of one file: its structure is
//! read once, and the owner cell of every patch face once, so each read
//! takes from the patches only the rows of the faces its cells own. Read
//! whole, the patches of a 100 M cell file are about 550 MB, a block's
//! own rows about 20 MB (docs/performance.md, "Slices of the 100 M cell
//! file over the network").

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex, PoisonError};

use super::cells::{self, PointMap, Set};
use super::overview::{
    CELL_RESERVED, NEAR, POINT_RESERVED, field_sets, index_of, not_blocked, rows_at,
};
use super::{
    Builder, GridSets, Parts, as_fields, attr_text, bad, blocks_of, components, count, dataset_of,
    ints, names_of, read_patches, text_attrs,
};
use crate::fasthash::FastMap;
use crate::field::{Location, Results};
use crate::hdf5::{Dataset, Dtype, H5, Object, Values};
use crate::{Mesh, MeshError, par};

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
/// without a block index, BLOCK005 for a block it does not have. A
/// caller reading several times from one file keeps a [`BlockFile`].
pub fn read_blocks(
    path: &Path,
    blocks: &[usize],
    halo: bool,
) -> Result<(Mesh, Results, Subset), MeshError> {
    BlockFile::new(path).read(blocks, halo)
}

/// A blocked VTKHDF file read block by block (see the module notes): the
/// file opened once, and the owner of each patch face read the first
/// time a block is. Shared between threads; reads may run at once.
#[derive(Debug)]
pub struct BlockFile {
    path: PathBuf,
    opened: Mutex<Option<Arc<Opened>>>,
}

/// A [`BlockFile`] opened: the file with its structure read, and the
/// owners of the patch faces once read.
struct Opened {
    h: H5,
    owners: Mutex<Option<Arc<Owners>>>,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Opened")
    }
}

/// The file cell owning each face of each patch, in patch order; `None`
/// for a patch read whole instead (more than one partition, no
/// `GlobalPointIds` or no `OwnerCell`, so the read fails as it did).
type Owners = Vec<Option<Vec<i64>>>;

impl BlockFile {
    /// The file at `path`, opened on the first read.
    pub fn new(path: &Path) -> BlockFile {
        BlockFile {
            path: path.to_path_buf(),
            opened: Mutex::new(None),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// As [`read_blocks`].
    pub fn read(&self, blocks: &[usize], halo: bool) -> Result<(Mesh, Results, Subset), MeshError> {
        if !self.path.is_file() {
            return Err(MeshError::new(
                "MESH001",
                format!("{} is not a readable file", self.path.display()),
            ));
        }
        let o = self.opened()?;
        let t = crate::profile::span("read.blocks");
        let read = subset_parts(&o.h, blocks, halo, Some(&o));
        drop(t);
        let (parts, subset) = read?;
        let (mut mesh, results) = parts.finish()?;
        mesh.files = vec![self.path.clone()];
        Ok((mesh, results, subset))
    }

    /// The bytes read from the file so far (structure, owners and blocks).
    pub fn bytes_read(&self) -> u64 {
        let o = self.opened.lock().unwrap_or_else(PoisonError::into_inner);
        o.as_ref().map_or(0, |o| o.h.bytes_read())
    }

    /// The file opened, its structure read: the first time, else as it
    /// was (a failed open is tried again next time).
    fn opened(&self) -> Result<Arc<Opened>, MeshError> {
        let mut o = self.opened.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(o) = o.as_ref() {
            return Ok(o.clone());
        }
        let io = crate::profile::span("read.file");
        let h = H5::open(&self.path)?;
        drop(io);
        let opened = Arc::new(Opened {
            h,
            owners: Mutex::new(None),
        });
        *o = Some(opened.clone());
        Ok(opened)
    }
}

impl Opened {
    /// The owners of the faces of `patches`, read the first time.
    fn owners(&self, patches: &[(String, Object)]) -> Result<Arc<Owners>, MeshError> {
        let mut o = self.owners.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(o) = o.as_ref() {
            return Ok(o.clone());
        }
        let _t = crate::profile::span("read.blocks.owners");
        let h = &self.h;
        let mut all = Vec::with_capacity(patches.len());
        for (_, obj) in patches {
            let one = |name: &str| ints(h, obj, name).is_ok_and(|v| v.len() == 1);
            let by_rows = one("NumberOfPoints")
                && h.child(obj, "Polygons")?
                    .is_some_and(|p| ints(h, &p, "NumberOfCells").is_ok_and(|v| v.len() == 1))
                && optional_dataset(h, obj, "PointData", "GlobalPointIds")?.is_some();
            all.push(match by_rows {
                true => super::optional_ints(h, obj, "CellData", "OwnerCell")?,
                false => None,
            });
        }
        let all = Arc::new(all);
        *o = Some(all.clone());
        Ok(all)
    }
}

/// Patch `patch` (the next of `b`) with the faces at `rows` (ascending)
/// of its `faces`, and its boundary fields at those rows: what
/// [`read_patches`] keeps of it, read row by row. Its points are those of
/// the volume, found by their `GlobalPointIds` (a blocked file's patch
/// has one partition, § 7).
fn patch_rows(
    h: &H5,
    b: &mut Builder,
    (name, obj): &(String, Object),
    roles: &BTreeMap<usize, String>,
    faces: usize,
    rows: &[u32],
) -> Result<(usize, Vec<crate::Field>), MeshError> {
    let p = b.mesh.patches.len() as u32;
    let before = b.mesh.boundary.len();
    if b.mesh.boundary.start.is_empty() {
        b.mesh.boundary = crate::Polygons::new();
    }
    if let Some(polys) = h.child(obj, "Polygons")?
        && !rows.is_empty()
    {
        let npart = ints(h, obj, "NumberOfPoints")?
            .first()
            .map_or(Ok(0), |&n| count(n, "points"))?;
        // each face's offsets: its row and the next
        let mut at: Vec<usize> = rows
            .iter()
            .flat_map(|&r| [r as usize, r as usize + 1])
            .collect();
        at.dedup();
        let offs = ints_at(h, &dataset_of(h, &polys, "Offsets")?, &at)
            .map_err(|_| bad("Polygons/Offsets is shorter than its cells"))?;
        let off = |r: usize| at.binary_search(&r).ok().map(|i| offs[i]);
        let mut spans = Vec::with_capacity(rows.len());
        for &r in rows {
            let span = off(r as usize)
                .zip(off(r as usize + 1))
                .and_then(|(s, e)| usize::try_from(s).ok().zip(usize::try_from(e).ok()))
                .filter(|(s, e)| *e >= s + 3)
                .ok_or_else(|| MeshError::new("MESH006", format!("polygon {r}: offsets")))?;
            spans.push(span.0..span.1);
        }
        let mut ids: Vec<usize> = spans.iter().flat_map(Clone::clone).collect();
        ids.sort_unstable();
        ids.dedup();
        let conn = ints_at(h, &dataset_of(h, &polys, "Connectivity")?, &ids)
            .map_err(|_| bad("Polygons/Connectivity is shorter than its count"))?;
        let local = |k: usize| ids.binary_search(&k).ok().map(|i| conn[i]);
        // the patch's points the faces use, and their global ids
        let mut pts: Vec<usize> = ids
            .iter()
            .filter_map(|&k| local(k))
            .filter_map(|v| usize::try_from(v).ok().filter(|&v| v < npart))
            .collect();
        pts.sort_unstable();
        pts.dedup();
        let gp = optional_dataset(h, obj, "PointData", "GlobalPointIds")?
            .ok_or_else(|| bad(format!("patch {p} has no PointData/GlobalPointIds")))?;
        let gids = ints_at(h, &gp, &pts).map_err(|_| {
            bad(format!(
                "patch {p}: GlobalPointIds is shorter than its points"
            ))
        })?;
        let map = b
            .gids
            .as_ref()
            .ok_or_else(|| bad("the volume has no GlobalPointIds"))?;
        for (&r, span) in rows.iter().zip(&spans) {
            let face: Vec<u32> = span
                .clone()
                .map(|k| {
                    local(k)
                        .and_then(|v| usize::try_from(v).ok())
                        .filter(|&v| v < npart)
                        .and_then(|v| pts.binary_search(&v).ok())
                        .and_then(|i| map.get(gids[i]))
                        .ok_or_else(|| MeshError::new("MESH006", format!("polygon {r}: point")))
                })
                .collect::<Result<_, _>>()?;
            b.mesh.boundary.push(face);
            b.mesh.boundary_patch.push(p);
        }
    }
    let role = match attr_text(h, obj, "role").or_else(|| roles.get(&(p as usize)).cloned()) {
        Some(r) => crate::PatchRole::parse(&r),
        None => crate::polymesh::patch_role(name, ""),
    };
    b.mesh.patches.push(crate::Patch {
        name: name.clone(),
        role,
    });
    Ok((before, fields_at(h, obj, faces, rows)?))
}

/// The boundary fields of a patch of `faces` faces at `rows`, as
/// [`super::data_fields`] gives them with those rows kept.
fn fields_at(
    h: &H5,
    obj: &Object,
    faces: usize,
    rows: &[u32],
) -> Result<Vec<crate::Field>, MeshError> {
    let Some(g) = h.child(obj, "CellData")? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for link in h.links(&g)? {
        if ["boundary", "OwnerCell"].contains(&link.name.as_str()) {
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
        let Some(comps) = components(&ds, faces) else {
            return Err(MeshError::new(
                "MESH005",
                format!(
                    "CellData/{} has shape {:?} for {faces} {}s",
                    link.name,
                    ds.dims,
                    Location::Boundary.name()
                ),
            ));
        };
        let values = rows_at(h, &ds, comps, rows)?;
        out.extend(as_fields(&link.name, Location::Boundary, comps, values));
    }
    Ok(out)
}

/// The integers of the one-dimensional `ds` at `rows` (ascending, each
/// once), read in windows of rows near each other.
fn ints_at(h: &H5, ds: &Dataset, rows: &[usize]) -> Result<Vec<i64>, MeshError> {
    let short = || bad("a dataset is shorter than its rows");
    if !h.reads_rows(ds) {
        let v = h.int_values(ds)?;
        return rows
            .iter()
            .map(|&r| (r < v.len()).then(|| v.int(r)).ok_or_else(short))
            .collect();
    }
    let mut out = Vec::with_capacity(rows.len());
    let mut i = 0;
    while i < rows.len() {
        let mut j = i + 1;
        while j < rows.len() && rows[j] - rows[j - 1] <= NEAR {
            j += 1;
        }
        let (first, last) = (rows[i], rows[j - 1]);
        let v = h.int_values_in(ds, first..last + 1)?;
        if v.held().end <= last {
            return Err(short());
        }
        out.extend(rows[i..j].iter().map(|&r| v.int(r)));
        i = j;
    }
    Ok(out)
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

fn subset_parts(
    h: &H5,
    asked: &[usize],
    halo: bool,
    file: Option<&Opened>,
) -> Result<(Parts, Subset), MeshError> {
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

    // the faces of the patches whose owner cell was read: by their rows
    // when the owners are known, else from the patches read whole
    let read = |id: i64| usize::try_from(id).is_ok_and(|f| taken.has(f));
    let owners = match file {
        Some(o) => Some(o.owners(&patches)?),
        None => None,
    };
    let mut boundary_fields = Vec::with_capacity(patches.len());
    for (i, patch) in patches.iter().enumerate() {
        let by_rows = owners
            .as_deref()
            .and_then(|o| o.get(i))
            .and_then(Option::as_ref)
            .filter(|_| b.gids.is_some());
        match by_rows {
            Some(own) => {
                let rows: Vec<u32> = (0..own.len())
                    .filter(|&r| read(own[r]))
                    .map(|r| r as u32)
                    .collect();
                boundary_fields.push(patch_rows(h, &mut b, patch, &roles, own.len(), &rows)?);
            }
            None => boundary_fields.extend(read_patches(
                h,
                &mut b,
                std::slice::from_ref(patch),
                &roles,
                Some(&read),
            )?),
        }
    }
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

/// A block's (global id, position) pairs, by shard.
type ByShard = Vec<Vec<(u32, u32)>>;

/// The points of the blocks read, joined by their global ids.
struct Joined {
    /// By block, the point of each of its point rows.
    maps: Vec<Vec<u32>>,
    /// Each new point, in order: its block (among those read) and its
    /// row among the block's point rows.
    firsts: Vec<(u32, u32)>,
    /// The point of each id, by [`super::shard_of`] the id.
    shards: Vec<FastMap<u32, u32>>,
}

/// Join the point rows `rows` of the blocks read by their global `ids`
/// (one `Values` each), as a whole read joins the blocks one after the
/// other: the first row of each id, in block order and then row order, is
/// a new point, numbered from `base` in that order, and every row of the
/// id is that point. The ids are sharded by a hash, so each thread fills
/// the maps of its own shards over all blocks: one map of the millions of
/// points of a region takes most of a read to fill on one thread. The ids
/// are kept as 32-bit numbers, as mesh points are numbered; `None` when
/// one is not (negative, or beyond 2^32), for the reader joining one
/// block after the other.
fn join_blocks(ids: &[&Values], rows: &[Range<usize>], base: usize) -> Option<Joined> {
    let at: Vec<usize> = rows
        .iter()
        .scan(0, |n, r| {
            let a = *n;
            *n += r.len();
            Some(a)
        })
        .collect();
    let n: usize = rows.iter().map(|r| r.len()).sum();
    let shards = if par::threads() > 1 {
        4 * par::threads()
    } else {
        1
    };
    // Each block's (id, position) pairs by shard, in row order.
    let lists: Vec<Option<ByShard>> = cells::in_runs((0..rows.len()).collect(), |k| {
        let mut by: Vec<Vec<(u32, u32)>> = (0..shards)
            .map(|_| Vec::with_capacity(rows[k].len() / shards + 16))
            .collect();
        for (i, r) in rows[k].clone().enumerate() {
            let id = u32::try_from(ids[k].int(r)).ok()?;
            by[super::shard_of(i64::from(id), shards)].push((id, (at[k] + i) as u32));
        }
        Some(by)
    });
    let lists: Vec<ByShard> = lists.into_iter().collect::<Option<_>>()?;
    // The first position of each id, shard by shard.
    let mut first = vec![0u32; n];
    let mut maps = {
        let first = AtomicU32::from_mut_slice(&mut first);
        cells::in_runs((0..shards).collect(), |s| {
            let len = lists.iter().map(|b| b[s].len()).sum();
            let mut map: FastMap<u32, u32> = crate::fasthash::map_with_capacity(len);
            for b in &lists {
                for &(id, pos) in &b[s] {
                    let f = *map.entry(id).or_insert(pos);
                    first[pos as usize].store(f, Relaxed);
                }
            }
            map
        })
    };
    drop(lists);
    // The new points numbered in order of their first position.
    let is_first = |p: usize| first[p] as usize == p;
    let ranges = par::ranges(n, 1 << 15);
    let counts = par::each(ranges.clone(), |r| r.filter(|&p| is_first(p)).count());
    let mut point = vec![0u32; n];
    {
        let pieces = par::split_mut(
            &mut point,
            &std::iter::once(0)
                .chain(ranges.iter().map(|r| r.end))
                .collect::<Vec<_>>(),
        );
        let mut before = base;
        let jobs: Vec<_> = ranges
            .iter()
            .zip(pieces)
            .zip(&counts)
            .map(|((r, piece), &c)| {
                let start = before;
                before += c;
                (r.clone(), piece, start)
            })
            .collect();
        par::each(jobs, |(r, piece, start)| {
            let mut next = start as u32;
            for (p, q) in r.zip(piece.iter_mut()) {
                if is_first(p) {
                    *q = next;
                    next += 1;
                }
            }
        });
    }
    let firsts: Vec<(u32, u32)> = par::collect(n, 1 << 15, |r| {
        r.filter(|&p| is_first(p))
            .map(|p| {
                let k = at.partition_point(|&a| a <= p) - 1;
                (k as u32, (p - at[k]) as u32)
            })
            .collect()
    });
    // Every row is the point of its id's first row.
    let maps_by_block: Vec<Vec<u32>> = (0..rows.len())
        .map(|k| {
            let r = at[k]..at[k] + rows[k].len();
            par::collect(r.len(), 1 << 15, |q| {
                q.map(|i| point[first[r.start + i] as usize]).collect()
            })
        })
        .collect();
    let point = &point;
    let pieces: Vec<&mut FastMap<u32, u32>> = maps.iter_mut().collect();
    par::each(pieces, |m| {
        for v in m.values_mut() {
            *v = point[*v as usize];
        }
    });
    Some(Joined {
        maps: maps_by_block,
        firsts,
        shards: maps,
    })
}

impl Builder {
    /// Read the cells of partitions `blocks` of grid `g` as each block's
    /// plan says, on all threads, put together partition after
    /// partition; gives the rows of the cell datasets and of `Points` the
    /// mesh took.
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
        let parts: Vec<cells::Part> = blocks.iter().map(|&b| part_list[b]).collect();
        // Each block's points and their global ids, on all threads.
        let t = crate::profile::span("read.blocks.points");
        let read: Vec<(Values, Values)> = cells::in_runs(parts.clone(), |part| {
            let rows = part.point_at..part.point_at + part.points;
            let xyz = h.values_in(&points, 3 * rows.start..3 * rows.end)?;
            let ids = h.int_values_in(&gids, rows.clone())?;
            if xyz.held().end < 3 * rows.end || ids.held().end < rows.end {
                return Err(bad("Points or GlobalPointIds is shorter than the points"));
            }
            Ok((xyz, ids))
        })
        .into_iter()
        .collect::<Result<_, MeshError>>()?;
        drop(t);
        // a read in the background stops between the steps
        crate::profile::check()?;
        // Joined by their ids, as a whole read joins the blocks one after
        // the other.
        let t = crate::profile::span("read.blocks.join");
        let rows: Vec<Range<usize>> = parts
            .iter()
            .map(|p| p.point_at..p.point_at + p.points)
            .collect();
        let ids: Vec<&Values> = read.iter().map(|(_, ids)| ids).collect();
        let (maps, point_rows) = match join_blocks(&ids, &rows, self.mesh.points.len()) {
            Some(joined) => {
                let mut fresh = vec![[0.0; 3]; joined.firsts.len()];
                par::chunks_mut(&mut fresh, 1 << 15, |at, part| {
                    for (q, &(k, i)) in part.iter_mut().zip(&joined.firsts[at..]) {
                        let row = rows[k as usize].start + i as usize;
                        *q = super::xyz_at(&read[k as usize].0, row);
                    }
                });
                self.mesh.points.extend(fresh);
                let point_rows: Vec<u32> = joined
                    .firsts
                    .iter()
                    .map(|&(k, i)| (rows[k as usize].start + i as usize) as u32)
                    .collect();
                self.gids = Some(super::Gids::Shards(joined.shards));
                (joined.maps, point_rows)
            }
            // ids that are no point numbers: block after block
            None => {
                let (mut maps, mut point_rows) = (Vec::new(), Vec::new());
                for ((xyz, ids), r) in read.iter().zip(&rows) {
                    let (map, new) = self.join_points(xyz, ids, r.clone());
                    maps.push(map);
                    point_rows.extend(new);
                }
                (maps, point_rows)
            }
        };
        drop(read);
        drop(t);
        crate::profile::check()?;
        // Each block's cells on threads of their own, the threads left
        // over for its windows.
        let t = crate::profile::span("read.cells");
        let outer = par::threads().min(parts.len()).max(1);
        let inner = (par::threads() / outer).max(1);
        let xyz = &self.mesh.points;
        let jobs: Vec<_> = parts.iter().zip(maps).zip(plans).collect();
        let reads: Vec<cells::Cells> = cells::in_runs(jobs, |((part, map), plan)| {
            par::with_threads(inner, || {
                let skip = Values::of_bytes_in(plan.skip.clone(), part.cell_at, total_cells);
                let planned = Planned { sets: &sets, skip };
                let map = PointMap::From(part.point_at, map);
                let grid = cells::Grid {
                    parts: std::slice::from_ref(part),
                    poly,
                    zone: None,
                    points: &map,
                    xyz,
                };
                cells::read_rows(&grid, &planned, part.cell_at..part.cell_at + part.cells)
            })
        })
        .into_iter()
        .collect::<Result<_, MeshError>>()?;
        drop(t);
        crate::profile::check()?;
        // Put together in block order.
        let mut cell_rows = Vec::new();
        for read in reads {
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
    /// Keep rule: a block read of a [`BlockFile`] takes from the patches
    /// only the rows of the faces its cells own, and reads what a read
    /// of its own reads. Reading the patches whole for every block (as
    /// before 2026-11-25) read 550 MB a block of the 100 M cell file and
    /// made a slice over the network take 19 minutes; the parity tests
    /// above pass either way, so only this test measures the bytes.
    #[test]
    fn a_block_reads_only_its_own_patch_rows() {
        // a boundary far larger than a block, with a boundary field
        let m = crate::synth::honeycomb(20);
        let tau: Vec<f64> = (0..m.boundary.len()).map(|i| i as f64).collect();
        let r = Results {
            fields: vec![crate::Field {
                name: "tau".into(),
                unit: None,
                location: Location::Boundary,
                components: 1,
                values: tau,
            }],
            ..Results::default()
        };
        let path = scratch("own-rows.vtkhdf");
        write_blocked(&path, &m, &r, 64).unwrap();
        let nb = (0..)
            .find(|&k| read_blocks(&path, &[k], false).is_err())
            .unwrap();
        // the first block, at a corner of the curve, has faces on three
        // patches
        let k = 0;
        // the patches read whole, as a read without the owners does
        let whole = {
            let h = H5::open(&path).unwrap();
            let base = h.bytes_read();
            subset_parts(&h, &[k], true, None).unwrap();
            h.bytes_read() - base
        };
        let file = BlockFile::new(&path);
        file.read(&[nb / 2], true).unwrap();
        let base = file.bytes_read();
        let (bm, br, bs) = file.read(&[k], true).unwrap();
        let rows = file.bytes_read() - base;
        // The block's own cells take about a third of what the whole
        // read takes here; read whole, the patches take the rest.
        assert!(
            rows * 2 < whole,
            "{rows} bytes by rows, {whole} with the patches whole"
        );
        let (om, or, os) = read_blocks(&path, &[k], true).unwrap();
        assert!(!bm.boundary.is_empty());
        assert_eq!(bm.boundary, om.boundary);
        assert_eq!(bm.boundary_patch, om.boundary_patch);
        assert_eq!(bm.patches, om.patches);
        assert_eq!((bm.points, bm.cell_faces), (om.points, om.cell_faces));
        assert_eq!((br, bs), (or, os));
    }

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

    /// Invariant: the join numbers each global id's first row, in block
    /// order and then row order, from `base`, maps every row to its id's
    /// point and the shards to the same points, on one thread as on
    /// four; an id that is no 32-bit point number leaves the join to the
    /// reader that goes block after block. Breaks when the shards number
    /// the points in the order they meet them, or a negative id wraps to
    /// a point. The file tests have few repeated ids per shard.
    #[test]
    fn ids_join_in_block_and_row_order() {
        let all = [5, 7, 9, 9, 2, 5, 11, 2, 13];
        let rows = [0..3, 3..7, 7..9];
        let values: Vec<Values> = rows
            .iter()
            .map(|r| Values::of_ints_in(&all, r.clone()))
            .collect();
        let ids: Vec<&Values> = values.iter().collect();
        for n in [1, 4] {
            let j = crate::par::with_threads(n, || join_blocks(&ids, &rows, 10)).unwrap();
            assert_eq!(
                j.maps,
                vec![vec![10, 11, 12], vec![12, 13, 10, 14], vec![13, 15]]
            );
            assert_eq!(
                j.firsts,
                vec![(0, 0), (0, 1), (0, 2), (1, 1), (1, 3), (2, 1)]
            );
            for (id, point) in [(5, 10), (7, 11), (9, 12), (2, 13), (11, 14), (13, 15)] {
                let shard = &j.shards[super::super::shard_of(id, j.shards.len())];
                assert_eq!(shard.get(&(id as u32)), Some(&point), "id {id} on {n}");
            }
        }
        for bad in [-1, 1 << 33] {
            let v = Values::of_ints_in(&[1, bad, 2], 0..3);
            let rows = 0..3;
            assert!(
                join_blocks(&[&v], std::slice::from_ref(&rows), 0).is_none(),
                "{bad}"
            );
        }
    }

    /// Invariant: blocks read on one thread, on three or on four give the
    /// same mesh, fields and subset, bit for bit: the points joined in
    /// the same order however the ids are sharded, the blocks put
    /// together in order however they are read. Breaks when the join
    /// numbers a point by the shard that met it first, or the cells of
    /// the blocks are put together in the order they finished; the other
    /// tests run on the machine's threads only.
    #[test]
    fn blocks_read_alike_on_any_threads() {
        for (name, path) in files("threads") {
            let nb = (0..)
                .find(|&k| read_blocks(&path, &[k], false).is_err())
                .unwrap();
            let some: Vec<usize> = (0..nb).filter(|k| k % 3 != 1).collect();
            let read = |n: usize| crate::par::with_threads(n, || read_blocks(&path, &some, true));
            let (m1, r1, s1) = read(1).unwrap();
            for n in [3, 4] {
                let (m, r, s) = read(n).unwrap();
                assert_eq!(m.points, m1.points, "{name} on {n}");
                assert_eq!(m.cell_faces, m1.cell_faces, "{name} on {n}");
                for c in 0..m.cell_count() {
                    let a: Vec<&[u32]> = m.cell(c).map(|f| f.nodes).collect();
                    let b: Vec<&[u32]> = m1.cell(c).map(|f| f.nodes).collect();
                    assert_eq!(a, b, "{name} on {n}, cell {c}");
                }
                assert_eq!(
                    (&m.boundary.start, &m.boundary.nodes, &m.boundary_patch),
                    (&m1.boundary.start, &m1.boundary.nodes, &m1.boundary_patch),
                    "{name} on {n}"
                );
                assert_eq!(m.boundary_owner, m1.boundary_owner, "{name} on {n}");
                assert_eq!((&m.cell_zone, &m.cell_kind), (&m1.cell_zone, &m1.cell_kind));
                assert_eq!(r, r1, "{name} on {n}");
                assert_eq!(s, s1, "{name} on {n}");
            }
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
