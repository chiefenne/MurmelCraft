//! A large mesh that is not blocked (the maintainer's card 4 of
//! 2026-11-24: offer to convert it, you confirm; a setting converts
//! without asking). A `.vtu`, an OpenFOAM `polyMesh` or an unblocked
//! VTKHDF file whose whole read needs more than the memory budget is
//! offered for conversion, once, to a blocked VTKHDF file beside it,
//! which then opens as an overview.
//!
//! - **The offer** ([`offer`]) comes from the head of the files alone
//!   ([`murmel_mesh::estimate`]), before anything is read.
//! - **The blocked copy** is `NAME.blocked.vtkhdf` beside the file (a
//!   case's: in the case folder, named after it). A copy newer than every
//!   file of the mesh is opened instead of converting again.
//! - **Converting** ([`ConvertJob`]) reads the mesh once whole and writes
//!   the copy as `murmel blocks make` does, to a `.part` file renamed when
//!   whole, so a stopped or failed conversion leaves no copy behind.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Instant, SystemTime};

use murmel_mesh::MeshError;
use murmel_mesh::blocks::{DEFAULT_BLOCK_CELLS, Written};
use murmel_mesh::profile::{self, Progress};

/// About the bytes a blocked file takes on disk per cell: 1,470 to 1,520
/// on the honeycombs of `murmel bench make` (polyhedra, without and with
/// three cell fields); cells with fewer faces take less.
pub const DISK_BYTES_PER_CELL: u64 = 1_500;

/// A mesh offered for conversion, and what converting would take.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    /// The mesh asked for.
    pub path: PathBuf,
    /// Its blocked copy.
    pub out: PathBuf,
    pub cells: u64,
    /// About the memory a whole read peaks at.
    pub needs: u64,
    /// The memory budget it is beyond.
    pub budget: u64,
    /// About the copy's size on disk.
    pub disk: u64,
    /// The copy is there and newer than every file of the mesh: it is
    /// opened instead.
    pub fresh: bool,
    /// Converting reads the mesh whole once, which needs more than the
    /// computer's memory.
    pub beyond_memory: bool,
}

/// The offer for the mesh at `path` when its whole read needs more than
/// `budget` bytes and it is not a blocked VTKHDF file; `None` otherwise
/// (or when its files do not say how many cells it has).
pub fn offer(path: &Path, budget: u64) -> Option<Offer> {
    let e = murmel_mesh::estimate::estimate(path)?;
    if e.blocked || e.open_bytes <= budget {
        return None;
    }
    let out = blocked_copy(path)?;
    let fresh = is_fresh(path, &out);
    Some(Offer {
        path: path.to_path_buf(),
        out,
        cells: e.cells,
        needs: e.open_bytes,
        budget,
        disk: e.cells.saturating_mul(DISK_BYTES_PER_CELL),
        fresh,
        beyond_memory: crate::memory::system_bytes().is_some_and(|m| e.open_bytes > m),
    })
}

/// Where the blocked copy of the mesh at `path` goes: `NAME.blocked.vtkhdf`
/// beside a file; in an OpenFOAM case's folder, named after the case.
pub fn blocked_copy(path: &Path) -> Option<PathBuf> {
    let vtu = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("vtu"));
    let vtkhdf = path.is_file() && murmel_mesh::vtkhdf::is_vtkhdf(path);
    if !vtu && !vtkhdf && murmel_mesh::polymesh::is_openfoam(path) {
        let poly = murmel_mesh::polymesh::polymesh_dir(path).ok()?;
        // polyMesh in constant in the case; else beside the polyMesh
        let case = match poly.parent() {
            Some(c) if c.file_name().is_some_and(|n| n == "constant") => c.parent()?,
            Some(p) => p,
            None => return None,
        };
        let name = case.file_name()?.to_string_lossy().into_owned();
        return Some(case.join(format!("{name}.blocked.vtkhdf")));
    }
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    Some(path.with_file_name(format!("{stem}.blocked.vtkhdf")))
}

/// When the mesh at `path` last changed: its file, or the newest file of
/// its `polyMesh` folder.
fn changed(path: &Path) -> Option<SystemTime> {
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    if path.is_file() && !murmel_mesh::polymesh::is_openfoam(path) {
        return modified(path);
    }
    let dir = murmel_mesh::polymesh::polymesh_dir(path).ok()?;
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| modified(&e.path()))
        .max()
}

/// Whether `out` is a blocked VTKHDF file newer than the mesh at `path`.
fn is_fresh(path: &Path, out: &Path) -> bool {
    let copied = std::fs::metadata(out).and_then(|m| m.modified()).ok();
    let blocked = out.is_file() && murmel_mesh::vtkhdf::size_of(out).is_ok_and(|s| s.blocked);
    match (copied, changed(path)) {
        (Some(c), Some(m)) => blocked && c >= m,
        _ => false,
    }
}

/// The `.part` file a copy is written to before it is whole.
fn part_of(out: &Path) -> PathBuf {
    let mut name = out.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    out.with_file_name(name)
}

/// Convert the mesh at `path` to the blocked VTKHDF file `out`: read it
/// whole and write the copy, renamed into place once whole. Stopped
/// through [`profile::check`] (MESH019) when watched.
pub fn convert(path: &Path, out: &Path) -> Result<Written, MeshError> {
    let (mesh, results) = murmel_mesh::read_results_with(path, false)?;
    profile::check()?;
    let part = part_of(out);
    let w = murmel_mesh::blocks::write_blocked(&part, &mesh, &results, DEFAULT_BLOCK_CELLS);
    let w = match w {
        Ok(w) => w,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    std::fs::rename(&part, out).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        MeshError::new(
            "BLOCK001",
            format!("the blocked file cannot be put at {}: {e}", out.display()),
        )
    })?;
    Ok(w)
}

type Slot = Arc<(Mutex<Option<Result<Written, MeshError>>>, Condvar)>;

/// A mesh converted on a thread of its own.
#[derive(Clone, Debug)]
pub struct ConvertJob {
    pub path: PathBuf,
    pub out: PathBuf,
    pub progress: Arc<Progress>,
    pub started: Instant,
    done: Slot,
}

impl ConvertJob {
    /// Start converting the mesh at `path` to `out`.
    pub fn start(path: PathBuf, out: PathBuf) -> ConvertJob {
        let done: Slot = Arc::new((Mutex::new(None), Condvar::new()));
        let progress = Arc::new(Progress::default());
        let (slot, watched, from, to) = (done.clone(), progress.clone(), path.clone(), out.clone());
        std::thread::spawn(move || {
            let r = profile::watch(watched, || convert(&from, &to));
            let (lock, ready) = &*slot;
            if let Ok(mut v) = lock.lock() {
                *v = Some(r);
                ready.notify_all();
            }
        });
        ConvertJob {
            path,
            out,
            progress,
            started: Instant::now(),
            done,
        }
    }

    pub fn is_done(&self) -> bool {
        self.done.0.lock().map_or(true, |v| v.is_some())
    }

    /// Ask it to stop at its next check; nothing is left behind.
    pub fn stop(&self) {
        self.progress.cancel();
    }

    /// What it gave once done (waiting for it with `wait`).
    pub fn take(&self, wait: bool) -> Option<Result<Written, MeshError>> {
        let (lock, ready) = &*self.done;
        let mut v = lock.lock().ok()?;
        while wait && v.is_none() {
            v = ready.wait(v).ok()?;
        }
        v.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
            .join(rel)
    }

    /// Invariant (the maintainer's card 4 of 2026-11-24): a mesh that is
    /// not blocked is offered for conversion only beyond the budget, with
    /// its cells, the memory it needs and its blocked copy beside it (a
    /// case's in its folder, named after it); a blocked file never is.
    /// Converting writes a blocked copy with the mesh's cells, which then
    /// is fresh (opened instead of converting again) until the mesh
    /// changes; a conversion stopped leaves no copy and no `.part`.
    /// Breaks when the estimate's budget test is turned around, when the
    /// copy lands where the next offer does not look, when a stale copy
    /// counts as fresh, or when a stopped write leaves a file. No other
    /// test converts on its own.
    #[test]
    fn large_meshes_are_offered_for_conversion() {
        let dir = std::env::temp_dir().join(format!("murmel-convert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let vtu = dir.join("channel.vtu");
        std::fs::copy(fixture("channel/foehncraft.vtu"), &vtu).unwrap();
        assert_eq!(offer(&vtu, u64::MAX), None, "within the budget");
        let o = offer(&vtu, 1).expect("beyond the budget");
        assert_eq!(o.out, dir.join("channel.blocked.vtkhdf"));
        assert!(o.cells >= 776 && o.needs > 1 && !o.fresh);
        assert_eq!(o.disk, o.cells * DISK_BYTES_PER_CELL);

        let w = convert(&vtu, &o.out).unwrap();
        assert_eq!(w.cells, 776);
        assert!(!part_of(&o.out).exists());
        assert_eq!(offer(&o.out, 1), None, "a blocked file is not offered");
        assert!(offer(&vtu, 1).unwrap().fresh);
        // The mesh changed after the copy: not fresh.
        let f = std::fs::OpenOptions::new().append(true).open(&vtu).unwrap();
        f.set_modified(SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        assert!(!offer(&vtu, 1).unwrap().fresh);

        // A case: the copy in its folder, named after it.
        let case = fixture("openfoam/channel_ascii");
        for p in [
            case.clone(),
            case.join("constant"),
            case.join("constant/polyMesh"),
        ] {
            assert_eq!(
                blocked_copy(&p).unwrap(),
                case.join("channel_ascii.blocked.vtkhdf")
            );
        }
        let h = fixture("vtkhdf/channel_fields.vtkhdf");
        assert_eq!(
            blocked_copy(&h).unwrap(),
            h.with_file_name("channel_fields.blocked.vtkhdf")
        );

        // Stopped: nothing left behind.
        let out = dir.join("stopped.vtkhdf");
        let job = ConvertJob::start(vtu.clone(), out.clone());
        job.stop();
        let r = job.take(true).unwrap();
        if let Err(e) = r {
            assert_eq!(e.code, "MESH019");
            assert!(!out.exists());
        }
        assert!(!part_of(&out).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
