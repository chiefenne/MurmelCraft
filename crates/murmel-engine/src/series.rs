//! Time series (D268, first milestone): the results of one run at many
//! time steps, each step a fields file, a parcel file or both. A series
//! comes from FoehnCraft's manifest `series.json` or from a folder of
//! `fields_<step>.vtkhdf` and `spray/parcels_<step>.vtkhdf`; the tools
//! `series.open`, `series.go` and `series.play` walk it and the window's
//! player bar plays it.
//!
//! The manifest (agreed with FoehnCraft's spray statistics work,
//! 2026-11-24):
//!
//! ```json
//! {"format": "foehncraft.series", "format_version": 1, "case": "spray_duct",
//!  "entries": [{"step": 50, "time": 0.25,
//!               "files": {"fields": "fields_000050.vtkhdf",
//!                         "parcels": "spray/parcels_000050.vtkhdf"}}]}
//! ```
//!
//! Paths are relative to the manifest's folder; entries are sorted by
//! step, one per step; keys MurmelCraft does not know are ignored, so the
//! manifest can grow.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The manifest's format and the version read.
pub const FORMAT: &str = "foehncraft.series";
pub const FORMAT_VERSION: u64 = 1;
/// The manifest's name in an output folder.
pub const MANIFEST: &str = "series.json";
/// The most entries a series may have (a manifest is read whole).
pub const MAX_ENTRIES: usize = 100_000;
/// The largest manifest read, in bytes.
const MAX_MANIFEST: u64 = 64 << 20;
/// Steps a second the player shows, its default and limits.
pub const RATE: f64 = 4.0;
pub const MIN_RATE: f64 = 0.1;
pub const MAX_RATE: f64 = 60.0;

/// One time step: its files (absolute) and its time when known.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub step: u64,
    pub time: Option<f64>,
    pub fields: Option<PathBuf>,
    pub parcels: Option<PathBuf>,
}

/// How the player plays (the window follows it; the command line and
/// agents step with `series.go`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playback {
    pub playing: bool,
    /// Steps a second.
    pub rate: f64,
    /// Start again at the first step after the last.
    pub looping: bool,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            playing: false,
            rate: RATE,
            looping: true,
        }
    }
}

/// Where the series came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Manifest,
    Folder,
}

impl Source {
    pub fn key(self) -> &'static str {
        match self {
            Source::Manifest => "manifest",
            Source::Folder => "folder",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    /// The manifest or the folder given.
    pub path: PathBuf,
    pub source: Source,
    pub case: Option<String>,
    pub entries: Vec<Entry>,
    /// The entry shown.
    pub current: usize,
    pub playback: Playback,
}

/// Why a series cannot be read (SERIES001).
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesError(pub String);

impl std::fmt::Display for SeriesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Deserialize)]
struct Manifest {
    format: String,
    format_version: u64,
    #[serde(default)]
    case: Option<String>,
    entries: Vec<ManifestEntry>,
}

#[derive(Deserialize)]
struct ManifestEntry {
    step: u64,
    #[serde(default)]
    time: Option<f64>,
    #[serde(default)]
    files: ManifestFiles,
}

#[derive(Deserialize, Default)]
struct ManifestFiles {
    #[serde(default)]
    fields: Option<String>,
    #[serde(default)]
    parcels: Option<String>,
}

impl Series {
    /// Read a series: `path` is a manifest, or a folder (its
    /// `series.json` when it has one, else its numbered files).
    pub fn read(path: &Path) -> Result<Series, SeriesError> {
        if path.is_dir() {
            let m = path.join(MANIFEST);
            if m.is_file() {
                Self::manifest(&m)
            } else {
                Self::folder(path)
            }
        } else if path.is_file() {
            Self::manifest(path)
        } else {
            Err(SeriesError(format!("{} does not exist", path.display())))
        }
    }

    fn manifest(path: &Path) -> Result<Series, SeriesError> {
        let len = std::fs::metadata(path)
            .map_err(|e| SeriesError(format!("cannot read {}: {e}", path.display())))?
            .len();
        if len > MAX_MANIFEST {
            return Err(SeriesError(format!(
                "{} has {len} bytes, more than the {MAX_MANIFEST} a manifest may have",
                path.display()
            )));
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| SeriesError(format!("cannot read {}: {e}", path.display())))?;
        let m: Manifest = serde_json::from_str(&text).map_err(|e| {
            SeriesError(format!(
                "{} is not a series manifest ({FORMAT}): {e}",
                path.display()
            ))
        })?;
        if m.format != FORMAT {
            return Err(SeriesError(format!(
                "{} has format {:?}, not {FORMAT}",
                path.display(),
                m.format
            )));
        }
        if m.format_version != FORMAT_VERSION {
            return Err(SeriesError(format!(
                "{} has format_version {}; MurmelCraft reads {FORMAT_VERSION}",
                path.display(),
                m.format_version
            )));
        }
        if m.entries.len() > MAX_ENTRIES {
            return Err(SeriesError(format!(
                "{} has {} entries, more than the {MAX_ENTRIES} a series may have",
                path.display(),
                m.entries.len()
            )));
        }
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut entries: Vec<Entry> = Vec::with_capacity(m.entries.len());
        for e in m.entries {
            if e.time.is_some_and(|t| !t.is_finite()) {
                return Err(SeriesError(format!(
                    "step {} of {} has a time that is not finite",
                    e.step,
                    path.display()
                )));
            }
            if e.files.fields.is_none() && e.files.parcels.is_none() {
                return Err(SeriesError(format!(
                    "step {} of {} names no file (files.fields or files.parcels)",
                    e.step,
                    path.display()
                )));
            }
            entries.push(Entry {
                step: e.step,
                time: e.time,
                fields: e.files.fields.map(|f| dir.join(f)),
                parcels: e.files.parcels.map(|f| dir.join(f)),
            });
        }
        entries.sort_by_key(|e| e.step);
        if let Some(w) = entries.windows(2).find(|w| w[0].step == w[1].step) {
            return Err(SeriesError(format!(
                "{} has step {} twice",
                path.display(),
                w[0].step
            )));
        }
        Self::made(path, Source::Manifest, m.case, entries)
    }

    /// The numbered files of an output folder: `fields_<step>.vtkhdf`
    /// in it and `spray/parcels_<step>.vtkhdf` (or `parcels_<step>.vtkhdf`
    /// in it), matched by step.
    fn folder(dir: &Path) -> Result<Series, SeriesError> {
        let mut steps: BTreeMap<u64, Entry> = BTreeMap::new();
        let mut add = |d: &Path, prefix: &str, parcels: bool| -> Result<(), SeriesError> {
            let Ok(list) = std::fs::read_dir(d) else {
                return Ok(());
            };
            for f in list.flatten() {
                let name = f.file_name();
                let Some(step) = name.to_str().and_then(|n| numbered(n, prefix)) else {
                    continue;
                };
                if steps.len() >= MAX_ENTRIES && !steps.contains_key(&step) {
                    return Err(SeriesError(format!(
                        "{} has more than the {MAX_ENTRIES} steps a series may have",
                        dir.display()
                    )));
                }
                let e = steps.entry(step).or_insert(Entry {
                    step,
                    time: None,
                    fields: None,
                    parcels: None,
                });
                if parcels {
                    e.parcels = Some(f.path());
                } else {
                    e.fields = Some(f.path());
                }
            }
            Ok(())
        };
        add(dir, "fields_", false)?;
        add(dir, "parcels_", true)?;
        add(&dir.join("spray"), "parcels_", true)?;
        let entries: Vec<Entry> = steps.into_values().collect();
        Self::made(dir, Source::Folder, None, entries)
    }

    fn made(
        path: &Path,
        source: Source,
        case: Option<String>,
        entries: Vec<Entry>,
    ) -> Result<Series, SeriesError> {
        if entries.is_empty() {
            return Err(SeriesError(format!(
                "{} holds no time steps (a series.json, or files fields_<step>.vtkhdf and spray/parcels_<step>.vtkhdf)",
                path.display()
            )));
        }
        Ok(Series {
            path: path.to_path_buf(),
            source,
            case,
            entries,
            current: 0,
            playback: Playback::default(),
        })
    }

    pub fn entry(&self) -> &Entry {
        &self.entries[self.current.min(self.entries.len() - 1)]
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry of `step`.
    pub fn index_of_step(&self, step: u64) -> Option<usize> {
        self.entries.binary_search_by_key(&step, |e| e.step).ok()
    }

    /// The entry nearest to `time` among those with a time.
    pub fn index_near_time(&self, time: f64) -> Option<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(k, e)| e.time.map(|t| (k, (t - time).abs())))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(k, _)| k)
    }

    /// The entry `by` steps on from the current one: wrapped round when
    /// the player loops, else held at the ends.
    pub fn index_by(&self, by: i64) -> usize {
        let n = self.entries.len() as i64;
        let k = self.current as i64 + by;
        if self.playback.looping {
            k.rem_euclid(n) as usize
        } else {
            k.clamp(0, n - 1) as usize
        }
    }

    /// The entry the player shows next, `None` at the end of a series
    /// that does not loop.
    pub fn next_played(&self) -> Option<usize> {
        if self.current + 1 < self.entries.len() {
            Some(self.current + 1)
        } else if self.playback.looping && self.entries.len() > 1 {
            Some(0)
        } else {
            None
        }
    }

    /// The range of the times known, first and last.
    pub fn times(&self) -> Option<(f64, f64)> {
        let mut t = self.entries.iter().filter_map(|e| e.time);
        let first = t.next()?;
        Some(t.fold((first, first), |(a, b), x| (a.min(x), b.max(x))))
    }
}

/// The step of `name` when it is `<prefix><digits>.vtkhdf`.
fn numbered(name: &str, prefix: &str) -> Option<u64> {
    let digits = name.strip_prefix(prefix)?.strip_suffix(".vtkhdf")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("murmel-series-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("spray")).unwrap();
        d
    }

    /// Invariant: a manifest's entries come sorted by step with paths
    /// joined to its folder and unknown keys ignored; a folder without
    /// one pairs fields_ and spray/parcels_ files by step. Breaks if a
    /// newer manifest key (FoehnCraft may add some) made it unreadable,
    /// or a folder scan paired the wrong files.
    #[test]
    fn manifest_and_folder() {
        let d = dir("read");
        std::fs::write(
            d.join(MANIFEST),
            r#"{"format":"foehncraft.series","format_version":1,"case":"duct","written_by":"x",
               "entries":[{"step":20,"time":0.2,"files":{"fields":"fields_000020.vtkhdf"}},
                          {"step":10,"time":0.1,"extra":1,"files":{"fields":"fields_000010.vtkhdf","parcels":"spray/parcels_000010.vtkhdf"}}]}"#,
        )
        .unwrap();
        let s = Series::read(&d).unwrap();
        assert_eq!(s.source, Source::Manifest);
        assert_eq!(s.case.as_deref(), Some("duct"));
        assert_eq!(
            s.entries.iter().map(|e| e.step).collect::<Vec<_>>(),
            [10, 20]
        );
        assert_eq!(
            s.entries[0].parcels,
            Some(d.join("spray/parcels_000010.vtkhdf"))
        );
        assert_eq!(s.times(), Some((0.1, 0.2)));
        assert_eq!(s.index_near_time(0.19), Some(1));
        assert_eq!(s.index_of_step(20), Some(1));
        std::fs::remove_file(d.join(MANIFEST)).unwrap();
        for f in [
            "fields_000010.vtkhdf",
            "fields_000020.vtkhdf",
            "fields.vtkhdf",
            "spray/parcels_000005.vtkhdf",
            "spray/parcels_000010.vtkhdf",
            "spray/parcels_x.vtkhdf",
        ] {
            std::fs::write(d.join(f), b"").unwrap();
        }
        let s = Series::read(&d).unwrap();
        assert_eq!(s.source, Source::Folder);
        assert_eq!(
            s.entries.iter().map(|e| e.step).collect::<Vec<_>>(),
            [5, 10, 20]
        );
        assert!(s.entries[0].fields.is_none());
        assert!(s.entries[1].fields.is_some() && s.entries[1].parcels.is_some());
        assert!(s.entries[2].parcels.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Invariant: another format, a newer version, a repeated step, an
    /// entry without files and an empty folder are refused with a reason.
    /// Breaks if a wrong file were read as an empty series.
    #[test]
    fn refusals() {
        let d = dir("refuse");
        let bad = |body: &str| {
            std::fs::write(d.join(MANIFEST), body).unwrap();
            Series::read(&d.join(MANIFEST)).unwrap_err().0
        };
        assert!(
            bad(r#"{"format":"other","format_version":1,"entries":[]}"#)
                .contains("not foehncraft.series")
        );
        assert!(
            bad(r#"{"format":"foehncraft.series","format_version":2,"entries":[]}"#)
                .contains("format_version 2")
        );
        assert!(bad(r#"{"format":"foehncraft.series","format_version":1,"entries":[{"step":1,"files":{"fields":"a"}},{"step":1,"files":{"fields":"b"}}]}"#).contains("step 1 twice"));
        assert!(
            bad(r#"{"format":"foehncraft.series","format_version":1,"entries":[{"step":1}]}"#)
                .contains("names no file")
        );
        assert!(
            bad(r#"{"format":"foehncraft.series","format_version":1,"entries":[]}"#)
                .contains("no time steps")
        );
        std::fs::remove_file(d.join(MANIFEST)).unwrap();
        assert!(Series::read(&d).unwrap_err().0.contains("no time steps"));
        assert!(
            Series::read(&d.join("nope"))
                .unwrap_err()
                .0
                .contains("does not exist")
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Invariant: stepping by wraps while the player loops and holds at
    /// the ends while it does not; the player stops after the last step
    /// of a series that does not loop. Breaks if Next on the last step
    /// fell off the series.
    #[test]
    fn stepping() {
        let e = |step| Entry {
            step,
            time: None,
            fields: Some(PathBuf::from("f")),
            parcels: None,
        };
        let mut s =
            Series::made(Path::new("x"), Source::Folder, None, vec![e(1), e(2), e(3)]).unwrap();
        assert_eq!(s.index_by(-1), 2);
        assert_eq!(s.index_by(4), 1);
        s.playback.looping = false;
        assert_eq!(s.index_by(-1), 0);
        s.current = 2;
        assert_eq!(s.index_by(5), 2);
        assert_eq!(s.next_played(), None);
        s.playback.looping = true;
        assert_eq!(s.next_played(), Some(0));
        assert_eq!(s.times(), None);
    }
}
