//! The files opened lately, for File ▸ Open recent: meshes, sessions,
//! time series, spray parcels and chart folders, the latest first. The
//! list is `recent.toml` in the settings folder, kept apart from
//! settings.toml because it is state the window keeps, not a choice of
//! yours. Only a session with [`crate::Session::remember_recent`] (the
//! window) adds to it, so scripts and agents leave it as it is. A list
//! that cannot be read is an empty one.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The list's file name in the settings folder.
pub const FILE: &str = "recent.toml";
/// The files the list keeps.
pub const KEEP: usize = 10;

/// What was opened, which says how to open it again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// `mesh.open`: a .vtu, .vtkhdf or an OpenFOAM case.
    Mesh,
    /// `session.load`: a .murmel file.
    Session,
    /// `series.open`: a folder or a file of a time series.
    Series,
    /// `parcels.open`: spray parcels.
    Parcels,
    /// `charts.open`: the tables of a run.
    Charts,
}

impl Kind {
    /// The kind a tool's `path` is, for the tools that open something.
    pub fn of_tool(tool: &str) -> Option<Kind> {
        Some(match tool {
            "mesh.open" => Kind::Mesh,
            "session.load" => Kind::Session,
            "series.open" => Kind::Series,
            "parcels.open" => Kind::Parcels,
            "charts.open" => Kind::Charts,
            _ => return None,
        })
    }

    /// The tool that opens this kind again.
    pub fn tool(self) -> &'static str {
        match self {
            Kind::Mesh => "mesh.open",
            Kind::Session => "session.load",
            Kind::Series => "series.open",
            Kind::Parcels => "parcels.open",
            Kind::Charts => "charts.open",
        }
    }
}

/// One file of the list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub kind: Kind,
}

#[derive(Default, Serialize, Deserialize)]
struct List {
    #[serde(default)]
    files: Vec<Entry>,
}

/// The list in `dir`, the latest first; empty when there is none or it
/// cannot be read.
pub fn load(dir: &Path) -> Vec<Entry> {
    std::fs::read_to_string(dir.join(FILE))
        .ok()
        .and_then(|t| toml::from_str::<List>(&t).ok())
        .map(|l| l.files)
        .unwrap_or_default()
}

/// Write the list to `dir`.
pub fn save(dir: &Path, files: &[Entry]) -> Result<PathBuf, String> {
    let text = toml::to_string_pretty(&List {
        files: files.to_vec(),
    })
    .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let p = dir.join(FILE);
    std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(p)
}

/// Put `path` first (as an absolute path), dropping an older entry of
/// the same path and the ones beyond [`KEEP`].
pub fn add(files: &mut Vec<Entry>, path: &Path, kind: Kind) {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    files.retain(|e| e.path != path);
    files.insert(0, Entry { path, kind });
    files.truncate(KEEP);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: a file opened again moves to the front instead of
    /// standing twice, the list keeps the latest KEEP files, and it
    /// survives a write and a read; a missing or broken file is an empty
    /// list. Breaks if the menu could show a file twice, grow without
    /// end, or a damaged recent.toml stopped the window. The tools test
    /// checks which calls add to it.
    #[test]
    fn latest_first_once_and_kept() {
        let mut l = Vec::new();
        for k in 0..KEEP + 3 {
            add(&mut l, Path::new(&format!("/m/{k}.vtu")), Kind::Mesh);
        }
        assert_eq!(l.len(), KEEP);
        assert_eq!(l[0].path, Path::new(&format!("/m/{}.vtu", KEEP + 2)));
        add(&mut l, Path::new("/m/5.vtu"), Kind::Mesh);
        assert_eq!(l[0].path, Path::new("/m/5.vtu"));
        assert_eq!(
            l.iter().filter(|e| e.path == Path::new("/m/5.vtu")).count(),
            1
        );
        let dir = std::env::temp_dir().join(format!("murmel-recent-{}", std::process::id()));
        assert!(load(&dir).is_empty(), "no file: empty");
        save(&dir, &l).unwrap();
        assert_eq!(load(&dir), l);
        std::fs::write(dir.join(FILE), "files = 3").unwrap();
        assert!(load(&dir).is_empty(), "broken file: empty");
        let _ = std::fs::remove_dir_all(dir);
    }
}
