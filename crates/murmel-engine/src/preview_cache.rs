//! Preview grids kept in the cache folder ([`crate::settings::cache_dir`]),
//! so an overview opened again shows its slices, iso-surfaces and arrows
//! without sampling the volume again: on 100 M cells the sampling reads
//! tens of GB, the kept grid about 50 bytes a grid cell (at most some
//! 120 MB).
//!
//! - **Where.** `preview/` in the cache folder, one file a mesh file and
//!   grid size (`<name>-<key>-<cells>.mpv`). The [`KEEP`] files used last
//!   stay, and no more than [`MOST_BYTES`] in all.
//! - **When it holds.** While the mesh file is as it was when it was
//!   sampled (its size, times and file id: [`crate::checked::Stamp`]),
//!   the grid asked for has as many cells along its longest side, and the
//!   grid is sampled as [`PREVIEW_VERSION`] samples it. A file modified
//!   less than [`crate::checked::SETTLED`] before it was sampled is not
//!   kept, as one written again at once could look unchanged.
//! - **The file.** `MURMELPV`, the format version and the length of a
//!   JSON header (little-endian `u32`s), the header (the mesh file and its
//!   stamp, the grid, the zones and the fields), then the grid's voxels
//!   (`u32`), file cells (`u64`) and zones (`u32`), and each field's
//!   values (`f64`), all little-endian. A file whose length is not what
//!   its header declares, or whose grid is not one, is not used.
//! - It is a cache: one that cannot be read or written means only that
//!   the grid is sampled again.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use murmel_mesh::vtkhdf::{PREVIEW_VERSION, PreviewGrid};
use murmel_mesh::{Field, Location, Mesh, Results};
use serde::{Deserialize, Serialize};

use crate::checked::{SETTLED, Stamp};

/// The cache's folder in the cache folder.
pub const DIR: &str = "preview";
/// The file name extension of a kept grid.
pub const EXTENSION: &str = "mpv";
const MAGIC: &[u8; 8] = b"MURMELPV";
/// The layout of a kept grid.
pub const FORMAT_VERSION: u32 = 1;
/// The grids kept.
pub const KEEP: usize = 16;
/// The most the kept grids take in all.
pub const MOST_BYTES: u64 = 2 << 30;

/// What a kept grid is of, and what follows its header.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    /// The mesh file (canonical) and its stamp when it was sampled.
    path: PathBuf,
    stamp: Stamp,
    /// The grid cells along the longest side asked for, and how the grid
    /// was sampled.
    cells: usize,
    sampler: u32,
    shape: [usize; 3],
    origin: [f64; 3],
    step: [f64; 3],
    /// The grid cells inside the volume.
    count: usize,
    zones: Vec<String>,
    fields: Vec<FieldHead>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldHead {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unit: Option<String>,
    components: usize,
}

/// A preview read from the cache or kept there: its mesh, its fields and
/// its grid, as [`murmel_mesh::vtkhdf::read_preview`] gives them.
pub type Kept = (Mesh, Results, PreviewGrid);

/// The file of `file`'s grid of `cells` in `dir`: its name, then a
/// 64-bit FNV-1a of its full path (the header names it whole).
fn path_in(dir: &Path, file: &Path, cells: usize) -> PathBuf {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in file.as_os_str().as_encoded_bytes() {
        h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
    }
    let name: String = file
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(40)
        .collect();
    dir.join(DIR)
        .join(format!("{name}-{h:016x}-{cells}.{EXTENSION}"))
}

/// The grid of `cells` along its longest side kept in `dir` for `file`,
/// if `file` is as it was when it was sampled.
pub fn load(dir: &Path, file: &Path, cells: usize) -> Option<Kept> {
    let file = std::fs::canonicalize(file).ok()?;
    let stamp = Stamp::of(&file)?;
    let at = path_in(dir, &file, cells);
    let bytes = std::fs::read(&at).ok()?;
    let kept = parse(&bytes, &file, &stamp, cells)?;
    // used now: kept the longest
    let _ = std::fs::File::options()
        .append(true)
        .open(&at)
        .and_then(|f| f.set_modified(std::time::SystemTime::now()));
    Some(kept)
}

/// A little-endian reader of the arrays after the header.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let b = self.bytes.get(self.at..self.at + N)?;
        self.at += N;
        b.try_into().ok()
    }

    fn u32s(&mut self, n: usize) -> Option<Vec<u32>> {
        (0..n)
            .map(|_| self.take().map(u32::from_le_bytes))
            .collect()
    }

    fn u64s(&mut self, n: usize) -> Option<Vec<u64>> {
        (0..n)
            .map(|_| self.take().map(u64::from_le_bytes))
            .collect()
    }

    fn f64s(&mut self, n: usize) -> Option<Vec<f64>> {
        (0..n)
            .map(|_| self.take().map(f64::from_le_bytes))
            .collect()
    }
}

/// The grid in `bytes`, if it is a whole kept grid of `file` as `stamp`
/// says it is now, of `cells` along its longest side.
fn parse(bytes: &[u8], file: &Path, stamp: &Stamp, cells: usize) -> Option<Kept> {
    let mut c = Cursor { bytes, at: 0 };
    if &c.take::<8>()? != MAGIC || u32::from_le_bytes(c.take()?) != FORMAT_VERSION {
        return None;
    }
    let len = u32::from_le_bytes(c.take()?) as usize;
    let head = bytes.get(c.at..c.at.checked_add(len)?)?;
    c.at += len;
    let h: Header = serde_json::from_slice(head).ok()?;
    if h.path != file || &h.stamp != stamp || h.cells != cells || h.sampler != PREVIEW_VERSION {
        return None;
    }
    // the length the header declares, before reading any array
    let per_cell = h.fields.iter().try_fold(4 + 8 + 4usize, |a, f| {
        (1..=9)
            .contains(&f.components)
            .then(|| a + 8 * f.components)
    })?;
    if bytes.len().checked_sub(c.at)? != h.count.checked_mul(per_cell)? {
        return None;
    }
    let voxels = c.u32s(h.count)?;
    let file_cells = c.u64s(h.count)?;
    let zones = c.u32s(h.count)?;
    if zones.iter().any(|&z| z as usize >= h.zones.len().max(1)) {
        return None;
    }
    let mut fields = Vec::with_capacity(h.fields.len());
    for f in &h.fields {
        fields.push(Field {
            name: f.name.clone(),
            unit: f.unit.clone(),
            location: Location::Cell,
            components: f.components,
            values: c.f64s(h.count * f.components)?,
        });
    }
    let grid = PreviewGrid {
        shape: h.shape,
        origin: h.origin,
        step: h.step,
        voxels,
        file_cells,
    };
    let mut mesh = grid.mesh().ok()?;
    mesh.cell_zone = zones;
    mesh.zones = h.zones;
    mesh.files = vec![file.to_path_buf()];
    let results = Results {
        fields,
        ..Results::default()
    };
    Some((mesh, results, grid))
}

/// Keep in `dir` the grid `kept` of `cells` along its longest side,
/// sampled from `file` as `before` (its stamp before it was read): only
/// if it is still as it was, and was modified at least [`SETTLED`]
/// before. The grids used longest ago go beyond [`KEEP`] grids or
/// [`MOST_BYTES`].
pub fn store(dir: &Path, file: &Path, before: &Stamp, cells: usize, kept: &Kept) {
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    if u128::from(before.modified_ns) + SETTLED.as_nanos() > now {
        return;
    }
    let Ok(file) = std::fs::canonicalize(file) else {
        return;
    };
    if Stamp::of(&file).as_ref() != Some(before) {
        return;
    }
    let Some(bytes) = encode(&file, before, cells, kept) else {
        return;
    };
    let at = path_in(dir, &file, cells);
    let Some(folder) = at.parent() else {
        return;
    };
    if std::fs::create_dir_all(folder).is_err() {
        return;
    }
    // written beside and renamed into place, so a reader never sees half
    let tmp = at.with_extension(format!("{EXTENSION}.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, bytes).is_err() || std::fs::rename(&tmp, &at).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    trim(folder);
}

/// The bytes of a kept grid; `None` for fields the file cannot hold.
fn encode(
    file: &Path,
    stamp: &Stamp,
    cells: usize,
    (mesh, results, grid): &Kept,
) -> Option<Vec<u8>> {
    let n = grid.voxels.len();
    let fields: Vec<&Field> = results
        .fields
        .iter()
        .filter(|f| f.location == Location::Cell)
        .collect();
    if grid.file_cells.len() != n
        || mesh.cell_zone.len() != n
        || fields
            .iter()
            .any(|f| !(1..=9).contains(&f.components) || f.values.len() != n * f.components)
    {
        return None;
    }
    let header = Header {
        path: file.to_path_buf(),
        stamp: stamp.clone(),
        cells,
        sampler: PREVIEW_VERSION,
        shape: grid.shape,
        origin: grid.origin,
        step: grid.step,
        count: n,
        zones: mesh.zones.clone(),
        fields: fields
            .iter()
            .map(|f| FieldHead {
                name: f.name.clone(),
                unit: f.unit.clone(),
                components: f.components,
            })
            .collect(),
    };
    let head = serde_json::to_vec(&header).ok()?;
    let per_cell: usize = 16 + fields.iter().map(|f| 8 * f.components).sum::<usize>();
    let mut out = Vec::with_capacity(16 + head.len() + n * per_cell);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&u32::try_from(head.len()).ok()?.to_le_bytes());
    out.extend_from_slice(&head);
    out.extend(grid.voxels.iter().flat_map(|v| v.to_le_bytes()));
    out.extend(grid.file_cells.iter().flat_map(|v| v.to_le_bytes()));
    out.extend(mesh.cell_zone.iter().flat_map(|v| v.to_le_bytes()));
    for f in fields {
        out.extend(f.values.iter().flat_map(|v| v.to_le_bytes()));
    }
    Some(out)
}

/// Remove the grids in `folder` used longest ago beyond [`KEEP`] or
/// [`MOST_BYTES`].
fn trim(folder: &Path) {
    let Ok(read) = std::fs::read_dir(folder) else {
        return;
    };
    let mut kept: Vec<(std::time::SystemTime, u64, PathBuf)> = read
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == EXTENSION))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    kept.sort_by_key(|k| std::cmp::Reverse(k.0));
    let mut total = 0u64;
    for (k, (_, len, path)) in kept.into_iter().enumerate() {
        total = total.saturating_add(len);
        if k >= KEEP || total > MOST_BYTES {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blocked honeycomb with a cell field and zones, written a while
    /// ago ([`SETTLED`]), in a folder of its own.
    fn sampled(name: &str) -> (PathBuf, PathBuf, Kept) {
        let dir = std::env::temp_dir().join(format!("murmel-pcache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures");
        let (mesh, results) =
            murmel_mesh::read_results(&fixtures.join("vtkhdf/channel_fields.vtkhdf")).unwrap();
        let path = dir.join("blocked.vtkhdf");
        murmel_mesh::blocks::write_blocked(&path, &mesh, &results, 100).unwrap();
        settle(&path);
        let kept = murmel_mesh::vtkhdf::read_preview(&path, 16).unwrap();
        (dir, path, kept)
    }

    fn settle(path: &Path) {
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        std::fs::File::options()
            .append(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    fn same(a: &Kept, b: &Kept) {
        assert_eq!(a.2, b.2, "the grid");
        assert_eq!(a.1.fields, b.1.fields, "the fields");
        assert_eq!(a.0.points, b.0.points);
        assert_eq!(a.0.face_owner, b.0.face_owner);
        assert_eq!(a.0.face_neighbour, b.0.face_neighbour);
        assert_eq!(a.0.cell_zone, b.0.cell_zone);
        assert_eq!(a.0.zones, b.0.zones);
        assert_eq!(a.0.files, b.0.files);
    }

    /// Invariant: a grid kept for a mesh file comes back exactly as it
    /// was sampled (mesh, zones, fields to the bit, grid) while the file
    /// is unchanged and the same grid size is asked for; a file written
    /// again, another grid size, a file modified just before it was
    /// sampled, a cut or altered cache file, and a header that declares
    /// more than the file holds all give nothing (sampled again), never a
    /// wrong grid or a panic. Breaks when the cache trusts a changed
    /// mesh file or its own declared counts. No other test reads the
    /// cache.
    #[test]
    fn a_kept_grid_comes_back_while_its_file_is_unchanged() {
        let (dir, path, kept) = sampled("back");
        let cache = dir.join("cache");
        let before = Stamp::of(&path).unwrap();
        assert!(load(&cache, &path, 16).is_none(), "nothing kept yet");
        store(&cache, &path, &before, 16, &kept);
        let back = load(&cache, &path, 16).expect("kept");
        same(&back, &kept);
        assert!(load(&cache, &path, 32).is_none(), "another grid size");

        // the cache file cut, altered or lying about its counts
        let at = path_in(&cache, &std::fs::canonicalize(&path).unwrap(), 16);
        let bytes = std::fs::read(&at).unwrap();
        let canonical = std::fs::canonicalize(&path).unwrap();
        for cut in [0, 7, 15, bytes.len() / 2, bytes.len() - 1] {
            assert!(parse(&bytes[..cut], &canonical, &before, 16).is_none());
        }
        let mut longer = bytes.clone();
        longer.extend_from_slice(&[0; 8]);
        assert!(parse(&longer, &canonical, &before, 16).is_none());
        let head_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let head = std::str::from_utf8(&bytes[16..16 + head_len]).unwrap();
        let n = kept.2.voxels.len();
        let lie = head.replace(&format!("\"count\":{n}"), &format!("\"count\":{}", n + 1));
        assert_ne!(lie, head);
        let mut lying = bytes[..12].to_vec();
        lying.extend_from_slice(&(lie.len() as u32).to_le_bytes());
        lying.extend_from_slice(lie.as_bytes());
        lying.extend_from_slice(&bytes[16 + head_len..]);
        assert!(parse(&lying, &canonical, &before, 16).is_none());
        let mut bad_voxel = bytes.clone();
        let first = 16 + head_len;
        bad_voxel[first..first + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&bad_voxel, &canonical, &before, 16).is_none());

        // the mesh file written again: not used, and kept again
        let (mesh, results) = murmel_mesh::read_results(&path).unwrap();
        let again = dir.join("again.vtkhdf");
        murmel_mesh::blocks::write_blocked(&again, &mesh, &results, 100).unwrap();
        std::fs::rename(&again, &path).unwrap();
        settle(&path);
        assert!(load(&cache, &path, 16).is_none(), "the file changed");
        // a file modified just now is not kept
        let fresh = dir.join("fresh.vtkhdf");
        murmel_mesh::blocks::write_blocked(&fresh, &mesh, &results, 100).unwrap();
        let k = murmel_mesh::vtkhdf::read_preview(&fresh, 16).unwrap();
        store(&cache, &fresh, &Stamp::of(&fresh).unwrap(), 16, &k);
        assert!(load(&cache, &fresh, 16).is_none(), "not settled");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Invariant: no more than [`KEEP`] grids stay, those used longest ago
    /// going first, and a grid read is used. Breaks when the cache grows
    /// without end or drops the grid just read. The test above keeps one.
    #[test]
    fn the_cache_keeps_the_grids_used_last() {
        let (dir, path, kept) = sampled("trim");
        let cache = dir.join("cache");
        let folder = cache.join(DIR);
        std::fs::create_dir_all(&folder).unwrap();
        // older grids of other files
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for k in 0..KEEP + 3 {
            let f = folder.join(format!("other-{k}.{EXTENSION}"));
            std::fs::write(&f, b"x").unwrap();
            std::fs::File::options()
                .append(true)
                .open(&f)
                .unwrap()
                .set_modified(old + std::time::Duration::from_secs(k as u64))
                .unwrap();
        }
        store(&cache, &path, &Stamp::of(&path).unwrap(), 16, &kept);
        let names: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), KEEP, "{names:?}");
        assert!(load(&cache, &path, 16).is_some(), "the new one stays");
        for k in 0..4 {
            assert!(!names.contains(&format!("other-{k}.{EXTENSION}")));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
