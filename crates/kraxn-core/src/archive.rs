//! Unpacking release archives (`.tar.gz` on macOS and Linux, `.zip` on
//! Windows). No entry may leave the target folder, and links in archives
//! are refused: the release archives hold only plain files.

use std::path::{Component, Path, PathBuf};

use crate::diag::{Res, diag};

fn bad(archive: &Path, why: impl std::fmt::Display) -> crate::diag::Diagnostic {
    diag("KRX006", format!("{}: {why}", archive.display()))
}

/// A relative path made only of normal parts.
fn safe(p: &Path) -> bool {
    !p.as_os_str().is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Unpacks `archive` into the empty folder `dest` and returns the one
/// folder at its top (e.g. `foehncraft-1.3.0-linux-x86_64`).
pub fn unpack(archive: &Path, dest: &Path) -> Res<PathBuf> {
    std::fs::create_dir_all(dest).map_err(|e| bad(dest, e))?;
    let name = archive.to_string_lossy();
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let f = std::fs::File::open(archive).map_err(|e| bad(archive, e))?;
        let mut t = tar::Archive::new(flate2::read::GzDecoder::new(f));
        t.set_preserve_permissions(true);
        t.set_overwrite(false);
        for entry in t.entries().map_err(|e| bad(archive, e))? {
            let mut entry = entry.map_err(|e| bad(archive, e))?;
            let path = entry.path().map_err(|e| bad(archive, e))?.into_owned();
            let kind = entry.header().entry_type();
            if !safe(&path) || !(kind.is_file() || kind.is_dir()) {
                return Err(bad(
                    archive,
                    format!(
                        "refuses the entry {} (a link or a path outside the folder)",
                        path.display()
                    ),
                ));
            }
            let out = dest.join(&path);
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| bad(archive, e))?;
            }
            entry.unpack(&out).map_err(|e| bad(archive, e))?;
        }
    } else if name.ends_with(".zip") {
        let f = std::fs::File::open(archive).map_err(|e| bad(archive, e))?;
        let mut z = zip::ZipArchive::new(f).map_err(|e| bad(archive, e))?;
        for i in 0..z.len() {
            let mut entry = z.by_index(i).map_err(|e| bad(archive, e))?;
            let Some(path) = entry.enclosed_name().filter(|p| safe(p)) else {
                return Err(bad(
                    archive,
                    format!("refuses the entry {:?}", entry.name_raw()),
                ));
            };
            if entry.is_symlink() {
                return Err(bad(archive, format!("refuses the link {}", path.display())));
            }
            let out = dest.join(&path);
            if entry.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| bad(archive, e))?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| bad(archive, e))?;
            }
            let mut w = std::fs::File::create(&out).map_err(|e| bad(archive, e))?;
            std::io::copy(&mut entry, &mut w).map_err(|e| bad(archive, e))?;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o777))
                    .map_err(|e| bad(archive, e))?;
            }
        }
    } else {
        return Err(bad(archive, "is neither a .tar.gz nor a .zip"));
    }
    let tops: Vec<PathBuf> = std::fs::read_dir(dest)
        .map_err(|e| bad(dest, e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    match tops.as_slice() {
        [one] if one.is_dir() => Ok(one.clone()),
        _ => Err(bad(archive, "does not hold exactly one folder at its top")),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A `.tar.gz` holding `top/` with the given files (name, contents,
    /// mode), for tests.
    pub(crate) fn make_tar(path: &Path, top: &str, files: &[(&str, &[u8], u32)]) {
        let f = std::fs::File::create(path).unwrap();
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        let mut b = tar::Builder::new(gz);
        for (name, data, mode) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(*mode);
            h.set_cksum();
            b.append_data(&mut h, format!("{top}/{name}"), *data)
                .unwrap();
        }
        b.into_inner().unwrap().finish().unwrap();
    }

    /// Invariant: an archive unpacks into its one top folder with the
    /// programs' modes kept, and a `..` entry is refused with KRX006 before
    /// anything lands outside. Breaks if the check is dropped (a crafted
    /// archive could write over files elsewhere) or modes are lost (the
    /// programs would not start).
    #[test]
    fn unpacks_safely() {
        let d = std::env::temp_dir().join(format!("kraxn-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let a = d.join("a.tar.gz");
        make_tar(
            &a,
            "p-1.0.0-linux-x86_64",
            &[("prog", b"#!/bin/sh\n", 0o755), ("README.md", b"r", 0o644)],
        );
        let top = unpack(&a, &d.join("out")).unwrap();
        assert!(top.ends_with("p-1.0.0-linux-x86_64"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(top.join("prog"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(m & 0o111, 0o111);
        }
        // a hand-made header with `..` (tar::Builder refuses to write one)
        let evil = d.join("evil.tar.gz");
        {
            let f = std::fs::File::create(&evil).unwrap();
            let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
            let mut b = tar::Builder::new(gz);
            let mut h = tar::Header::new_old();
            let name = b"../escape";
            h.as_old_mut().name[..name.len()].copy_from_slice(name);
            h.set_size(1);
            h.set_mode(0o644);
            h.set_entry_type(tar::EntryType::Regular);
            h.set_cksum();
            b.append(&h, &b"x"[..]).unwrap();
            b.into_inner().unwrap().finish().unwrap();
        }
        let e = unpack(&evil, &d.join("out2")).unwrap_err();
        assert_eq!(e.code, "KRX006");
        assert!(!d.join("escape").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
