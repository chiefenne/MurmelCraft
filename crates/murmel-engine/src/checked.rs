//! The digests of files already checked, remembered so that a file opened
//! again unchanged is not hashed again (the user's choice, 2026-11-24:
//! SHA-256 of a 5 GB mesh took about 17 s on the iMac). A file keeps its
//! BLAKE3 hash and, once one was needed (older evidence or sessions, see
//! [`murmel_mesh::hash`]), its SHA-256 beside it.
//!
//! A file counts as unchanged while its size and modification time, and
//! on Unix its change time and file id (device and inode), are the ones it
//! had when it was hashed. Writing to a file changes its times, and
//! replacing it changes its id; setting the modification time back by
//! hand changes the change time, which programs cannot set. The list is
//! `checked-files.json` in the settings folder: the [`KEEP`] files checked
//! last, the latest first. It is a cache: a list that cannot be read or
//! written means only that files are hashed again.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::UNIX_EPOCH;

use murmel_mesh::FileHash;
use serde::{Deserialize, Serialize};

/// The list's file name in the settings folder.
pub const FILE: &str = "checked-files.json";
pub const FORMAT: &str = "murmelcraft.checked-files";
/// 1.1 (2026-11-24): each file has `blake3`, `sha256` or both; 1.0 had
/// only `sha256`, and is read as it is.
pub const FORMAT_VERSION: &str = "1.1";
/// The files the list keeps.
pub const KEEP: usize = 500;
/// How long ago a file must have been modified to be remembered. A file
/// written again within the same tick of its file system's clock (2 s on
/// FAT) and at the same size would look unchanged; one modified earlier
/// than this gets a later time when it is written again.
pub const SETTLED: std::time::Duration = std::time::Duration::from_secs(3);

/// What tells that a file changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub size: u64,
    /// Modification time, nanoseconds since 1970.
    pub modified_ns: u64,
    /// Change time (seconds, nanoseconds), where the system has one.
    #[serde(default)]
    pub changed: Option<[i64; 2]>,
    /// Device and inode, where the system has them.
    #[serde(default)]
    pub id: Option<[u64; 2]>,
}

impl Stamp {
    /// The stamp of the file at `path` now; `None` for no file, or one
    /// without a modification time.
    pub fn of(path: &Path) -> Option<Stamp> {
        let m = std::fs::metadata(path).ok()?;
        if !m.is_file() {
            return None;
        }
        let modified = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        #[cfg(unix)]
        let (changed, id) = {
            use std::os::unix::fs::MetadataExt;
            (Some([m.ctime(), m.ctime_nsec()]), Some([m.dev(), m.ino()]))
        };
        #[cfg(not(unix))]
        let (changed, id) = (None, None);
        Some(Stamp {
            size: m.len(),
            modified_ns: u64::try_from(modified.as_nanos()).ok()?,
            changed,
            id,
        })
    }
}

/// A file checked: where it is, as it was, and its digests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    #[serde(flatten)]
    stamp: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blake3: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

impl Entry {
    fn digest(&self, hash: FileHash) -> Option<&String> {
        match hash {
            FileHash::Blake3 => self.blake3.as_ref(),
            FileHash::Sha256 => self.sha256.as_ref(),
        }
        .filter(|d| hash.is_digest(d))
    }

    fn set(&mut self, hash: FileHash, digest: String) {
        match hash {
            FileHash::Blake3 => self.blake3 = Some(digest),
            FileHash::Sha256 => self.sha256 = Some(digest),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct List {
    format: String,
    format_version: String,
    files: Vec<Entry>,
}

/// The files in `dir`'s list; none when it is missing or unreadable.
fn read(dir: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(dir.join(FILE)) else {
        return Vec::new();
    };
    match serde_json::from_str::<List>(&text) {
        Ok(l) if l.format == FORMAT => l.files,
        _ => Vec::new(),
    }
}

/// Write `files` as `dir`'s list, through a file of its own renamed into
/// place, so a reader never sees half a list.
fn write(dir: &Path, files: Vec<Entry>) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let list = List {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION.into(),
        files,
    };
    let text = serde_json::to_string_pretty(&list).map_err(std::io::Error::other)?;
    let tmp = dir.join(format!("{FILE}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, dir.join(FILE)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// The digest of `hash` remembered in `dir` for `file`, if the file is
/// unchanged since it was hashed.
pub fn lookup(dir: &Path, file: &Path, hash: FileHash) -> Option<String> {
    let path = std::fs::canonicalize(file).ok()?;
    let stamp = Stamp::of(&path)?;
    read(dir)
        .into_iter()
        .find(|e| e.path == path && e.stamp == stamp)
        .and_then(|e| e.digest(hash).cloned())
}

/// Remember in `dir` that `file`, as `before` (its stamp before it was
/// hashed), has the digest `digest` of `hash`: only if it is still as it
/// was, so a file written while it was hashed is not remembered, and was
/// modified at least [`SETTLED`] ago. A digest of the other hash
/// remembered for the file as it is now stays beside it.
pub fn remember(dir: &Path, file: &Path, before: &Stamp, hash: FileHash, digest: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let settled = u128::from(before.modified_ns) + SETTLED.as_nanos() <= now;
    if !hash.is_digest(digest) || !settled {
        return;
    }
    let Ok(path) = std::fs::canonicalize(file) else {
        return;
    };
    if Stamp::of(&path).as_ref() != Some(before) {
        return;
    }
    let mut files = read(dir);
    let mut entry = files
        .iter()
        .find(|e| e.path == path && &e.stamp == before)
        .cloned()
        .unwrap_or_else(|| Entry {
            path: path.clone(),
            stamp: before.clone(),
            blake3: None,
            sha256: None,
        });
    entry.set(hash, digest.to_ascii_lowercase());
    files.retain(|e| e.path != path);
    files.insert(0, entry);
    files.truncate(KEEP);
    let _ = write(dir, files);
}

/// The digest of `hash` of `file`: remembered in `dir`, or taken (and
/// then remembered); without `dir`, always taken. `Ok(None)` when `stop`
/// was set while hashing.
pub fn digest(
    dir: Option<&Path>,
    file: &Path,
    hash: FileHash,
    stop: &AtomicBool,
) -> std::io::Result<Option<String>> {
    let Some(dir) = dir else {
        return hash.of_file(file, stop);
    };
    if let Some(d) = lookup(dir, file, hash) {
        return Ok(Some(d));
    }
    let before = Stamp::of(file);
    let digest = hash.of_file(file, stop)?;
    if let (Some(b), Some(d)) = (&before, &digest) {
        remember(dir, file, b, hash, d);
    }
    Ok(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("murmel-checked-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Set the modification time of `path` back by `secs`, as a file
    /// written a while ago.
    fn age(path: &Path, secs: u64) {
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(secs);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    const B3: FileHash = FileHash::Blake3;
    const SHA: FileHash = FileHash::Sha256;
    const FAKE: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    /// Invariant: a file remembered is found again while unchanged, and
    /// is not once its content, its size or its modification time
    /// changed, or another file took its place; a file modified a moment
    /// ago, or written while it was hashed, is not remembered. Breaks
    /// when the list is looked up by path alone (a changed mesh would pass
    /// its check with the old digest), when a stamp leaves out the time or
    /// the file id, or when a file just written is remembered (written
    /// again within the same tick of the clock it would look unchanged).
    /// The session tests only see a digest taken or not.
    #[test]
    fn only_an_unchanged_file_is_found() {
        let dir = temp("found");
        let list = dir.join("settings");
        let f = dir.join("mesh.vtu");
        std::fs::write(&f, b"0123456789").unwrap();
        age(&f, 3600);
        remember(&list, &f, &Stamp::of(&f).unwrap(), B3, FAKE);
        assert_eq!(lookup(&list, &f, B3).as_deref(), Some(FAKE));
        // By another name of the same file.
        let again = dir.join(".").join("mesh.vtu");
        assert_eq!(lookup(&list, &again, B3).as_deref(), Some(FAKE));

        // The same size, another content and time; just written, so not
        // remembered either.
        std::fs::write(&f, b"9876543210").unwrap();
        assert_eq!(lookup(&list, &f, B3), None);
        remember(&list, &f, &Stamp::of(&f).unwrap(), B3, FAKE);
        assert_eq!(lookup(&list, &f, B3), None);

        // Replaced by another file of the same size and time.
        age(&f, 3600);
        remember(&list, &f, &Stamp::of(&f).unwrap(), B3, FAKE);
        assert!(lookup(&list, &f, B3).is_some());
        let other = dir.join("other.vtu");
        std::fs::write(&other, b"9876543210").unwrap();
        let t = std::fs::metadata(&f).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&other)
            .unwrap()
            .set_modified(t)
            .unwrap();
        std::fs::rename(&other, &f).unwrap();
        if cfg!(unix) {
            assert_eq!(lookup(&list, &f, B3), None);
        }

        // Written between the stamp and the digest.
        age(&f, 3600);
        let stale = Stamp::of(&f).unwrap();
        std::fs::write(&f, b"0000000000").unwrap();
        age(&f, 1800);
        remember(&list, &f, &stale, B3, FAKE);
        assert_eq!(lookup(&list, &f, B3), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Invariant: hashing through the list takes the digest of a file
    /// once and then finds it, each hash on its own and both side by
    /// side, and the list keeps the files checked last, each once; a
    /// list of format 1.0 (`sha256` only) still finds its files. Breaks
    /// when `digest` hashes on every call (the open would wait again),
    /// one hash's digest is returned for the other, taking one drops the
    /// other, or the list grows without end.
    #[test]
    fn the_list_remembers_the_files_checked_last() {
        let dir = temp("list");
        let list = dir.join("settings");
        let stop = AtomicBool::new(false);
        let f = dir.join("a.vtu");
        std::fs::write(&f, b"abc").unwrap();
        age(&f, 3600);
        let taken = digest(Some(&list), &f, SHA, &stop).unwrap().unwrap();
        assert_eq!(
            taken,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(lookup(&list, &f, SHA), Some(taken.clone()));
        assert_eq!(lookup(&list, &f, B3), None);
        // The BLAKE3 hash joins the SHA-256 of the same file.
        let b3 = digest(Some(&list), &f, B3, &stop).unwrap().unwrap();
        assert_eq!(b3, FileHash::Blake3.of_bytes(b"abc"));
        assert_eq!(lookup(&list, &f, SHA), Some(taken.clone()));
        assert_eq!(lookup(&list, &f, B3), Some(b3));
        // Found, not taken: the list says FAKE now.
        remember(&list, &f, &Stamp::of(&f).unwrap(), SHA, FAKE);
        assert_eq!(
            digest(Some(&list), &f, SHA, &stop).unwrap().as_deref(),
            Some(FAKE)
        );
        // Without a list, always taken.
        assert_eq!(digest(None, &f, SHA, &stop).unwrap(), Some(taken));

        for i in 0..KEEP + 5 {
            let g = dir.join(format!("f{i}"));
            std::fs::write(&g, b"x").unwrap();
            age(&g, 3600);
            remember(&list, &g, &Stamp::of(&g).unwrap(), B3, FAKE);
            remember(&list, &g, &Stamp::of(&g).unwrap(), B3, FAKE);
        }
        let files = read(&list);
        assert_eq!(files.len(), KEEP);
        assert!(files[0].path.ends_with(format!("f{}", KEEP + 4)));
        assert_eq!(lookup(&list, &f, B3), None);

        // A list written before BLAKE3 (format 1.0).
        let g = dir.join(format!("f{}", KEEP + 4));
        let s = Stamp::of(&g).unwrap();
        let old = serde_json::json!({
            "format": FORMAT, "format_version": "1.0",
            "files": [{"path": std::fs::canonicalize(&g).unwrap(), "size": s.size,
                "modified_ns": s.modified_ns, "changed": s.changed, "id": s.id,
                "sha256": FAKE}],
        });
        std::fs::write(list.join(FILE), old.to_string()).unwrap();
        assert_eq!(lookup(&list, &g, SHA).as_deref(), Some(FAKE));
        assert_eq!(lookup(&list, &g, B3), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
