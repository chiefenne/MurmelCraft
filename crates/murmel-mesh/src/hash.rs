//! The hashes that check mesh files (the user's choice, 2026-11-24,
//! "BLAKE3 only, still check old"): BLAKE3 for every file MurmelCraft
//! hashes and every digest it writes, SHA-256 only to check what was
//! recorded with it before, KoflCraft's `mesh-evidence.json` 1.0 and
//! sessions saved before format 1.15.
//!
//! SHA-256 of a 5 GB mesh took 17 s on the iMac (its Core i9 has no SHA
//! instructions and SHA-256 runs on one thread); BLAKE3 uses the vector
//! units and every core, so the disk sets the pace.

use std::io::Read as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;

use serde::{Deserialize, Serialize};

/// A hash of file digests, named as the files that record it name it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileHash {
    /// What MurmelCraft takes and writes.
    #[default]
    Blake3,
    /// Read from older evidence and sessions, and checked as such.
    Sha256,
}

/// The bytes BLAKE3 hashes on many threads at once; below this one thread
/// is faster than handing the work out.
const PARALLEL: usize = 1 << 20;

/// The pieces a file is read in for hashing.
const PIECE: usize = 16 << 20;

impl FileHash {
    /// The name in files: `blake3`, `sha256`.
    pub fn key(self) -> &'static str {
        match self {
            FileHash::Blake3 => "blake3",
            FileHash::Sha256 => "sha256",
        }
    }

    /// The hash called `key`, as [`FileHash::key`] names it.
    pub fn from_key(key: &str) -> Option<FileHash> {
        match key {
            "blake3" => Some(FileHash::Blake3),
            "sha256" => Some(FileHash::Sha256),
            _ => None,
        }
    }

    /// Whether `d` looks like a digest of this hash: 64 hex digits (both
    /// hashes are 256 bits).
    pub fn is_digest(self, d: &str) -> bool {
        d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit())
    }

    /// A hasher to feed in pieces.
    pub fn hasher(self) -> Hasher {
        match self {
            FileHash::Blake3 => Hasher::Blake3(Box::new(blake3::Hasher::new())),
            FileHash::Sha256 => Hasher::Sha256(Box::default()),
        }
    }

    /// The digest of `bytes`, lowercase hex.
    pub fn of_bytes(self, bytes: &[u8]) -> String {
        let mut h = self.hasher();
        h.update(bytes);
        h.finish()
    }

    /// The digest (lowercase hex) of the file at `path`, read in pieces so
    /// that a file of many gigabytes is never held whole; one thread reads
    /// the next piece while the last is hashed. `None` when `stop` was set
    /// before the end (checked between pieces).
    pub fn of_file(self, path: &Path, stop: &AtomicBool) -> std::io::Result<Option<String>> {
        let mut f = std::fs::File::open(path)?;
        std::thread::scope(|s| {
            let (tx, rx) = sync_channel::<Vec<u8>>(2);
            let (back, spare) = sync_channel::<Vec<u8>>(3);
            let reader = s.spawn(move || -> std::io::Result<()> {
                loop {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let mut buf = spare
                        .try_recv()
                        .unwrap_or_else(|_| Vec::with_capacity(PIECE));
                    buf.clear();
                    match (&mut f).take(PIECE as u64).read_to_end(&mut buf) {
                        Ok(0) => return Ok(()),
                        Ok(_) => {
                            if tx.send(buf).is_err() {
                                return Ok(());
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(e) => return Err(e),
                    }
                }
            });
            let mut h = self.hasher();
            for piece in rx {
                h.update(&piece);
                let _ = back.try_send(piece);
            }
            reader
                .join()
                .unwrap_or_else(|_| Err(std::io::Error::other("the file reader stopped")))?;
            Ok((!stop.load(Ordering::Relaxed)).then(|| h.finish()))
        })
    }
}

/// A digest taken in pieces.
pub enum Hasher {
    Blake3(Box<blake3::Hasher>),
    Sha256(Box<sha2::Sha256>),
}

impl Hasher {
    pub fn update(&mut self, bytes: &[u8]) {
        match self {
            Hasher::Blake3(h) if bytes.len() >= PARALLEL => {
                h.update_rayon(bytes);
            }
            Hasher::Blake3(h) => {
                h.update(bytes);
            }
            Hasher::Sha256(h) => sha2::Digest::update(h.as_mut(), bytes),
        }
    }

    /// The digest, lowercase hex.
    pub fn finish(self) -> String {
        match self {
            Hasher::Blake3(h) => h.finalize().to_hex().to_string(),
            Hasher::Sha256(h) => crate::hex(&sha2::Digest::finalize(*h)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: both hashes give their published digests of "abc", in
    /// one piece, in many pieces and across the parallel threshold; a file
    /// hashed in pieces has the digest of its bytes hashed at once, and a
    /// set `stop` ends the hashing without a digest. Breaks when a piece
    /// is lost or hashed twice, or the parallel path (`update_rayon`)
    /// hashes differently from the plain one: the validity badge would
    /// call a good mesh changed. The session tests use only one digest.
    #[test]
    fn digests_of_bytes_and_files() {
        assert_eq!(
            FileHash::Sha256.of_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            FileHash::Blake3.of_bytes(b"abc"),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
        let bytes: Vec<u8> = (0..(2 * PIECE as u32) + 7)
            .map(|i| (i % 251) as u8)
            .collect();
        let path = std::env::temp_dir().join(format!("murmel-hash-{}", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        for hash in [FileHash::Blake3, FileHash::Sha256] {
            let whole = hash.of_bytes(&bytes);
            let mut h = hash.hasher();
            for piece in bytes.chunks(PARALLEL - 5) {
                h.update(piece);
            }
            assert_eq!(h.finish(), whole);
            let go = AtomicBool::new(false);
            assert_eq!(hash.of_file(&path, &go).unwrap(), Some(whole));
            let stop = AtomicBool::new(true);
            assert_eq!(hash.of_file(&path, &stop).unwrap(), None);
        }
        let _ = std::fs::remove_file(&path);
        let go = AtomicBool::new(false);
        assert!(FileHash::Blake3.of_file(&path, &go).is_err());
        assert_eq!(FileHash::from_key("blake3"), Some(FileHash::Blake3));
        assert_eq!(
            FileHash::from_key(FileHash::Sha256.key()),
            Some(FileHash::Sha256)
        );
        assert_eq!(FileHash::from_key("md5"), None);
    }
}
