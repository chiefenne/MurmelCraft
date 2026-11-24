//! Session files (UI spec, viewer tools: "`session.save`, `session.load`:
//! save or load the session (meshes, camera, display, clip) as JSON").
//!
//! A session file (`*.murmel`, format `murmelcraft.session`) names the
//! mesh by its path and digest (BLAKE3 since 1.15, SHA-256 before) and holds the camera, the display (colour
//! mode, render mode, quality metric and threshold, pick target, the
//! hidden, isolated and selected items and the colours set on items, all
//! by their tree ids), the clip planes (since 1.2), the saved views (named
//! cameras, since format 1.1), the measurements and probes (since 1.3),
//! the field shown and each field's style (since 1.4), the slices,
//! iso-surfaces and the mesh's look beside them (since 1.5), the
//! vector arrows (since 1.6), the regions of interest (since 1.7; their
//! flat cut since 1.8), the streamlines (since 1.9), the derived fields
//! as recipes, computed again on loading (since 1.11), the spray
//! parcels shown, their file read again on loading (since 1.12), the time
//! series walked and the step shown (since 1.13), the fields of your
//! formulas beside the derived ones (since 1.14), the mesh's BLAKE3
//! hash in place of its SHA-256 (since 1.15), whether an overview's
//! block boxes show (since 1.16), the user's mesh
//! colours and the view cube
//! placement. Loading puts all of it back; the colours and the
//! view cube apply to the session only and are not written to
//! `settings.toml` (decided 2026-11-24).
//!
//! The mesh path is kept twice: relative to the session file's folder, so
//! a case folder that moves as a whole still opens, and absolute, for a
//! session file moved on its own. Loading tries the relative one first.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::clip::Clip;
use crate::cube::CubeSettings;
use crate::derive::Made;
use crate::display::{ColorBy, Item, PickTarget, RenderMode, SavedView};
use crate::extract::{Iso, MeshLook, Slice};
use crate::field::{FieldChoice, FieldStyle, RangeMode};
use crate::glyph::Arrows;
use crate::measure::{Measurement, Probe};
use crate::parcel::ParcelView;
use crate::quality::Metric;
use crate::region::Region;
use crate::session::Session;
use crate::stream::Stream;
use crate::theme::{MeshColors, Rgb, Tokens};

pub const SESSION_FORMAT: &str = "murmelcraft.session";
pub const SESSION_VERSION: &str = "1.17";
/// The extension of session files (JSON inside).
pub const EXTENSION: &str = "murmel";

/// The time series a session walks (1.13): its manifest or folder, kept
/// twice as the mesh is, the step shown and the player's rate and loop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesRef {
    pub path: PathBuf,
    pub absolute: PathBuf,
    pub step: u64,
    pub rate: f64,
    #[serde(rename = "loop")]
    pub looping: bool,
}

/// The mesh a session shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshRef {
    /// Relative to the session file's folder when the mesh is on the same
    /// drive, else absolute.
    pub path: PathBuf,
    /// The absolute path when the session was saved.
    pub absolute: PathBuf,
    /// `OpenMesh::digest` when the session was saved, under the name of
    /// its hash: `blake3` since 1.15; `sha256` before, and since then
    /// for a mesh whose evidence recorded SHA-256.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blake3: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl MeshRef {
    /// The digest saved and its hash; BLAKE3 when both are there.
    pub fn digest(&self) -> Option<(murmel_mesh::FileHash, &str)> {
        use murmel_mesh::FileHash;
        match (&self.blake3, &self.sha256) {
            (Some(d), _) => Some((FileHash::Blake3, d)),
            (None, Some(d)) => Some((FileHash::Sha256, d)),
            (None, None) => None,
        }
    }
}

/// What the viewport shows, by item ids.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DisplayState {
    pub color_by: ColorBy,
    pub render_mode: RenderMode,
    pub metric: Metric,
    pub threshold: Option<f64>,
    pub pick: PickTarget,
    pub hidden: Vec<String>,
    pub isolated: Vec<String>,
    pub selection: Vec<String>,
    /// Colours set on items (`display.set_color`), by item id.
    pub colors: BTreeMap<String, Rgb>,
    /// The one clip plane of format 1.0 and 1.1; read, never written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip: Option<Clip>,
    /// The clip planes, in order (1.2).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<Clip>,
    /// Distances and angles, in order (1.3).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub measurements: Vec<Measurement>,
    /// Probes, in order (1.3).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub probes: Vec<Probe>,
    /// The field colour mode field shows (1.4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<FieldChoice>,
    /// How each field is drawn, by field name (1.4).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub field_styles: BTreeMap<String, FieldStyle>,
    /// Slices and series of slices, in order (1.5).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub slices: Vec<Slice>,
    /// Iso-surfaces, in order (1.5).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub isos: Vec<Iso>,
    /// What the mesh looks like while slices or iso-surfaces show (1.5).
    #[serde(skip_serializing_if = "is_outline")]
    pub mesh_look: MeshLook,
    /// The vector arrows (1.6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arrows: Option<Arrows>,
    /// Regions of interest, in order (1.7).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<Region>,
    /// Sets of streamlines, in order (1.9).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub streams: Vec<Stream>,
    /// The spray parcels shown (1.12): their path relative to the
    /// session file's folder (as the mesh's), the absolute one beside.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parcels: Option<ParcelView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parcels_absolute: Option<PathBuf>,
    /// Whether the boxes of an overview's blocks show (1.16).
    #[serde(skip_serializing_if = "is_false")]
    pub show_blocks: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_outline(l: &MeshLook) -> bool {
    *l == MeshLook::Outline
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionFile {
    pub format: String,
    pub format_version: String,
    #[serde(default)]
    pub mesh: Option<MeshRef>,
    pub camera: Camera,
    #[serde(default)]
    pub display: DisplayState,
    /// The user's mesh colours when the session was saved.
    #[serde(default)]
    pub colors: MeshColors,
    #[serde(default)]
    pub view_cube: CubeSettings,
    /// The saved views of the mesh, in the order they were saved (1.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<SavedView>,
    /// The derived fields, as recipes (1.11), and the fields of your
    /// formulas (1.14), each after the fields it reads; loading computes
    /// them again before the display returns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived: Vec<Made>,
    /// The time series walked (1.13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<SeriesRef>,
}

/// `path` made absolute from the working folder, without touching the
/// disk (`std::path::absolute`), and with `.` and `..` folded.
pub fn absolute(path: &Path) -> PathBuf {
    let p = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `to` relative to the folder `from` (both absolute and folded); `None`
/// when they share no root (another drive on Windows).
pub fn relative(from: &Path, to: &Path) -> Option<PathBuf> {
    let f: Vec<Component> = from.components().collect();
    let t: Vec<Component> = to.components().collect();
    if f.first() != t.first() {
        return None;
    }
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..f.len() {
        out.push("..");
    }
    for c in &t[common..] {
        out.push(c);
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    Some(out)
}

/// A session file that cannot be used, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Refused(pub String);

impl SessionFile {
    /// The state of `s`, to be written at `file`.
    pub fn of(s: &Session, file: &Path) -> SessionFile {
        let dir = absolute(file)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mesh = s.mesh.as_ref().map(|m| {
            let abs = absolute(&m.path);
            let (blake3, sha256) = match m.hash {
                murmel_mesh::FileHash::Blake3 => (Some(m.digest.clone()), None),
                murmel_mesh::FileHash::Sha256 => (None, Some(m.digest.clone())),
            };
            MeshRef {
                path: relative(&dir, &abs).unwrap_or_else(|| abs.clone()),
                absolute: abs,
                blake3,
                sha256,
            }
        });
        let d = &s.display;
        let ids = |items: &mut dyn Iterator<Item = &Item>| -> Vec<String> {
            match &s.mesh {
                Some(m) => items.map(|i| i.id(&m.mesh)).collect(),
                None => Vec::new(),
            }
        };
        let colors = match &s.mesh {
            Some(m) => d.colors.iter().map(|(i, c)| (i.id(&m.mesh), *c)).collect(),
            None => BTreeMap::new(),
        };
        SessionFile {
            format: SESSION_FORMAT.into(),
            format_version: SESSION_VERSION.into(),
            mesh,
            camera: s.camera.clone(),
            display: DisplayState {
                color_by: d.color_by,
                render_mode: d.render_mode,
                metric: d.metric,
                threshold: d.threshold,
                pick: d.pick,
                hidden: ids(&mut d.hidden.iter()),
                isolated: ids(&mut d.isolated.iter()),
                selection: ids(&mut d.selection.iter()),
                colors,
                clip: None,
                clips: if s.mesh.is_some() {
                    d.clips.clone()
                } else {
                    Vec::new()
                },
                measurements: if s.mesh.is_some() {
                    d.measurements.clone()
                } else {
                    Vec::new()
                },
                probes: if s.mesh.is_some() {
                    d.probes.clone()
                } else {
                    Vec::new()
                },
                field: d.field.clone(),
                field_styles: d.field_styles.clone(),
                slices: if s.mesh.is_some() {
                    d.slices.clone()
                } else {
                    Vec::new()
                },
                isos: if s.mesh.is_some() {
                    d.isos.clone()
                } else {
                    Vec::new()
                },
                mesh_look: d.mesh_look,
                show_blocks: d.show_blocks,
                arrows: if s.mesh.is_some() {
                    d.arrows.clone()
                } else {
                    None
                },
                regions: if s.mesh.is_some() {
                    d.regions.clone()
                } else {
                    Vec::new()
                },
                streams: if s.mesh.is_some() {
                    d.streams.clone()
                } else {
                    Vec::new()
                },
                parcels: d.parcels.clone().filter(|_| s.mesh.is_some()).map(|v| {
                    let abs = absolute(&v.path);
                    ParcelView {
                        path: relative(&dir, &abs).unwrap_or(abs),
                        ..v
                    }
                }),
                parcels_absolute: d
                    .parcels
                    .as_ref()
                    .filter(|_| s.mesh.is_some())
                    .map(|v| absolute(&v.path)),
            },
            colors: s.colors.clone(),
            view_cube: s.view_cube.clone(),
            views: if s.mesh.is_some() {
                d.views.clone()
            } else {
                Vec::new()
            },
            derived: s.fields().map(|f| f.made().to_vec()).unwrap_or_default(),
            series: s.series.as_ref().map(|r| {
                let abs = absolute(&r.path);
                SeriesRef {
                    path: relative(&dir, &abs).unwrap_or_else(|| abs.clone()),
                    absolute: abs,
                    step: r.entry().step,
                    rate: r.playback.rate,
                    looping: r.playback.looping,
                }
            }),
        }
    }

    /// Parse and check a session file's text: the format, finite numbers,
    /// a positive distance, a non-zero clip normal and colours the tokens
    /// take. Nothing of the session changes.
    pub fn parse(text: &str) -> Result<SessionFile, Refused> {
        let probe: serde_json::Value =
            serde_json::from_str(text).map_err(|e| Refused(format!("not JSON: {e}")))?;
        if probe["format"] != SESSION_FORMAT {
            return Err(Refused(format!(
                "not a session file (format is not {SESSION_FORMAT})"
            )));
        }
        let major = |v: &str| v.split('.').next().unwrap_or("").to_string();
        let version = probe["format_version"].as_str().unwrap_or("");
        if major(version) != major(SESSION_VERSION) {
            return Err(Refused(format!(
                "format version {version}; this MurmelCraft reads {SESSION_VERSION}"
            )));
        }
        let mut f: SessionFile =
            serde_json::from_value(probe).map_err(|e| Refused(e.to_string()))?;
        check_camera(&mut f.camera).map_err(|()| {
            Refused("the camera needs finite numbers and a positive distance".into())
        })?;
        for m in &f.derived {
            let checked = match m {
                Made::Quantity(r) => r.reference.as_ref().map_or(Ok(()), |rf| rf.check()),
                Made::Formula(x) => crate::calc::check(x).map_err(|e| e.message()),
            };
            checked.map_err(|e| Refused(format!("derived field {}: {e}", m.field())))?;
        }
        let mut names = Vec::new();
        for v in &mut f.views {
            let name = crate::display::view_name(&v.name)
                .filter(|n| !names.contains(n))
                .ok_or_else(|| {
                    Refused(format!(
                        "saved view name {:?} is empty, too long or given twice",
                        v.name
                    ))
                })?;
            check_camera(&mut v.camera).map_err(|()| {
                Refused(format!(
                    "saved view {name:?}: the camera needs finite numbers and a positive distance"
                ))
            })?;
            v.name = name.clone();
            names.push(name);
        }
        // A 1.0 or 1.1 file names one plane.
        if let Some(c) = f.display.clip.take() {
            f.display.clips.insert(0, c);
        }
        for clip in &mut f.display.clips {
            if !clip.origin.iter().all(|v| v.is_finite()) {
                return Err(Refused("the clip origin needs finite numbers".into()));
            }
            clip.normal = crate::clip::unit(clip.normal)
                .ok_or_else(|| Refused("the clip normal must not be zero".into()))?;
        }
        let mut numbers = Vec::new();
        for m in &f.display.measurements {
            if let Some(why) = m.problem() {
                return Err(Refused(format!("measure:{}: {why}", m.n)));
            }
            if m.n == 0 || numbers.contains(&m.n) {
                return Err(Refused(format!(
                    "measure:{} is numbered 0 or given twice",
                    m.n
                )));
            }
            numbers.push(m.n);
        }
        numbers.clear();
        for p in &f.display.probes {
            let name_ok = p
                .name
                .as_ref()
                .is_none_or(|n| crate::display::view_name(n).as_deref() == Some(n.as_str()));
            if !p.at.is_finite() || !name_ok || p.n == 0 || numbers.contains(&p.n) {
                return Err(Refused(format!(
                    "probe:{} needs finite coordinates, a number given once and a valid name",
                    p.n
                )));
            }
            numbers.push(p.n);
        }
        for (name, st) in &f.display.field_styles {
            let fixed_ok = match st.range {
                RangeMode::Fixed { min, max } => min.is_finite() && max.is_finite() && min < max,
                _ => true,
            };
            let ok = (crate::field::MIN_BANDS..=crate::field::MAX_BANDS).contains(&st.bands)
                && (1..=crate::field::MAX_LINES).contains(&st.line_count)
                && st
                    .map
                    .as_deref()
                    .is_none_or(|m| crate::colormap::get(m).is_some())
                && fixed_ok;
            if !ok {
                return Err(Refused(format!(
                    "the style of field {name}: bands 2 to 64, lines 1 to 64, a known colour map and a fixed range with finite min < max"
                )));
            }
        }
        if f.display.slices.len() > crate::extract::MAX_SLICES
            || f.display.isos.len() > crate::extract::MAX_ISOS
        {
            return Err(Refused(format!(
                "at most {} slices and {} iso-surfaces",
                crate::extract::MAX_SLICES,
                crate::extract::MAX_ISOS
            )));
        }
        for (k, sl) in f.display.slices.iter_mut().enumerate() {
            let series_ok = (1..=crate::extract::MAX_SERIES).contains(&sl.count)
                && sl.spacing.is_finite()
                && sl.spacing >= 0.0
                && (sl.count == 1 || sl.spacing > 0.0);
            let normal = crate::clip::unit(sl.normal);
            match normal {
                Some(n) if sl.origin.iter().all(|v| v.is_finite()) && series_ok => sl.normal = n,
                _ => {
                    return Err(Refused(format!(
                        "slice:{}: a finite origin, a non-zero normal, 1 to {} planes and a positive spacing between them",
                        k + 1,
                        crate::extract::MAX_SERIES
                    )));
                }
            }
        }
        for (k, iso) in f.display.isos.iter().enumerate() {
            if !iso.value.is_finite() || iso.field.name.is_empty() {
                return Err(Refused(format!(
                    "iso:{}: a field and a finite value",
                    k + 1
                )));
            }
        }
        if let Some(a) = &f.display.arrows
            && (a.field.is_empty()
                || !(crate::glyph::MIN_DENSITY..=crate::glyph::MAX_DENSITY).contains(&a.density)
                || !(a.scale.is_finite() && a.scale > 0.0 && a.scale <= crate::glyph::MAX_SCALE))
        {
            return Err(Refused(format!(
                "arrows: a field, a density of {} to {} and a scale above 0 up to {}",
                crate::glyph::MIN_DENSITY,
                crate::glyph::MAX_DENSITY,
                crate::glyph::MAX_SCALE
            )));
        }
        if let Some(v) = &f.display.parcels
            && !(v.scale.is_finite() && v.scale > 0.0 && v.scale <= crate::parcel::MAX_SCALE)
        {
            return Err(Refused(format!(
                "parcels: a scale above 0 up to {}",
                crate::parcel::MAX_SCALE
            )));
        }
        if let Some(r) = &f.series
            && !(r.rate.is_finite()
                && (crate::series::MIN_RATE..=crate::series::MAX_RATE).contains(&r.rate))
        {
            return Err(Refused(format!(
                "series: a rate of {} to {} steps a second",
                crate::series::MIN_RATE,
                crate::series::MAX_RATE
            )));
        }
        if f.display.streams.len() > crate::stream::MAX_STREAMS {
            return Err(Refused(format!(
                "at most {} streamline sets",
                crate::stream::MAX_STREAMS
            )));
        }
        for (k, st) in f.display.streams.iter().enumerate() {
            if let Some(why) = st.problem_alone() {
                return Err(Refused(format!("stream:{}: {why}", k + 1)));
            }
        }
        if f.display.regions.len() > crate::region::MAX_REGIONS {
            return Err(Refused(format!(
                "at most {} regions",
                crate::region::MAX_REGIONS
            )));
        }
        for (k, r) in f.display.regions.iter_mut().enumerate() {
            if let crate::region::Shape::Slab { normal, .. } = &mut r.shape
                && let Some(n) = crate::clip::unit(*normal)
            {
                *normal = n;
            }
            if let Some(why) = r.problem_alone() {
                return Err(Refused(format!("region:{}: {why}", k + 1)));
            }
        }
        if let Some(t) = f.display.threshold
            && !t.is_finite()
        {
            return Err(Refused("the quality threshold must be finite".into()));
        }
        let v = &f.view_cube;
        if !(0.0..=crate::cube::MAX_DISTANCE).contains(&v.distance_px)
            || v.position.is_some_and(|p| !p.iter().all(|x| x.is_finite()))
        {
            return Err(Refused("the view cube settings are out of range".into()));
        }
        Tokens::builtin()
            .and_then(|t| t.with_colors(&f.colors))
            .map_err(|e| Refused(format!("colours: {e}")))?;
        Ok(f)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// The parcel file to read: relative to `file`'s folder if that
    /// exists, else the absolute path saved.
    pub fn parcels_path(&self, file: &Path) -> Option<PathBuf> {
        let v = self.display.parcels.as_ref()?;
        let dir = absolute(file).parent().map(Path::to_path_buf)?;
        let rel = absolute(&dir.join(&v.path));
        let abs = self.display.parcels_absolute.clone();
        Some(match abs {
            Some(a) if !rel.exists() && a.exists() => a,
            _ => rel,
        })
    }

    /// The series manifest or folder to read: relative to `file`'s folder
    /// if that exists, else the absolute path saved.
    pub fn series_path(&self, file: &Path) -> Option<PathBuf> {
        let r = self.series.as_ref()?;
        let dir = absolute(file).parent().map(Path::to_path_buf)?;
        let rel = absolute(&dir.join(&r.path));
        Some(if rel.exists() || !r.absolute.exists() {
            rel
        } else {
            r.absolute.clone()
        })
    }

    /// The mesh file to open: relative to `file`'s folder if that exists,
    /// else the absolute path saved.
    pub fn mesh_path(&self, file: &Path) -> Option<PathBuf> {
        let m = self.mesh.as_ref()?;
        let dir = absolute(file).parent().map(Path::to_path_buf)?;
        let rel = absolute(&dir.join(&m.path));
        Some(if rel.exists() || !m.absolute.exists() {
            rel
        } else {
            m.absolute.clone()
        })
    }
}

/// Check a camera read from a file and clamp its angles into range.
fn check_camera(c: &mut Camera) -> Result<(), ()> {
    let finite = c.target.iter().all(|v| v.is_finite())
        && [c.distance, c.yaw_deg, c.pitch_deg, c.fov_deg, c.roll_deg]
            .iter()
            .all(|v| v.is_finite());
    if !finite || c.distance <= 0.0 {
        return Err(());
    }
    c.pitch_deg = c.pitch_deg.clamp(-90.0, 90.0);
    c.fov_deg = c.fov_deg.clamp(1.0, 120.0);
    Ok(())
}

impl Session {
    /// Put the camera, the display and the settings of `f` back on the
    /// open mesh (the caller opened the right one). Ids the mesh does not
    /// have are skipped and returned. The selection is restored too, and
    /// the journal starts again.
    pub fn restore(&mut self, f: SessionFile) -> Vec<String> {
        let mut missing = Vec::new();
        // Checked by `parse`; the surface is rebuilt once, below.
        if self.colors != f.colors
            && let Ok(t) = Tokens::builtin().and_then(|t| t.with_colors(&f.colors))
        {
            self.tokens = t;
            self.colors = f.colors;
        }
        self.view_cube = f.view_cube;
        self.camera = f.camera;
        // The derived fields first, so the display finds them.
        for m in f.derived {
            let name = m.field().to_string();
            if let Err(e) = self.make(m) {
                missing.push(format!("derived field {name}: {}", e.message()));
            }
        }
        let d = &mut self.display;
        let s = f.display;
        d.color_by = s.color_by;
        d.render_mode = s.render_mode;
        d.metric = s.metric;
        d.threshold = s.threshold;
        d.pick = s.pick;
        d.hidden.clear();
        d.isolated.clear();
        d.selection.clear();
        d.colors.clear();
        d.clips.clear();
        d.views.clear();
        d.measurements.clear();
        d.probes.clear();
        d.slices.clear();
        d.isos.clear();
        d.arrows = None;
        d.regions.clear();
        d.streams.clear();
        d.stream_traces = None;
        d.parcels = None;
        d.parcel_data = None;
        d.mesh_look = s.mesh_look;
        d.show_blocks = s.show_blocks;
        if let Some(m) = &self.mesh {
            let mut find = |id: &String| {
                let i = Item::parse(id, &m.mesh);
                if i.is_none() && !missing.contains(id) {
                    missing.push(id.clone());
                }
                i
            };
            d.hidden = s.hidden.iter().filter_map(&mut find).collect();
            d.isolated = s.isolated.iter().filter_map(&mut find).collect();
            for id in &s.selection {
                if let Some(i) = find(id)
                    && !d.selection.contains(&i)
                {
                    d.selection.push(i);
                }
            }
            d.colors = s
                .colors
                .iter()
                .filter_map(|(id, c)| find(id).map(|i| (i, *c)))
                .collect();
            d.clips = s.clips;
            // Clip planes the file selects but does not have.
            d.selection.retain(|&i| match i {
                Item::Clip(k) => (k as usize) < d.clips.len(),
                _ => true,
            });
            d.views = f.views;
            d.measurements = s.measurements;
            // A probe whose cell the mesh does not have (it changed since
            // the session was saved) is located again.
            d.probes = s.probes;
            for p in &mut d.probes {
                if p.at.cell.is_none_or(|c| c as usize >= m.mesh.cell_count()) {
                    p.at.cell = crate::measure::locate(&m.mesh, p.at.point);
                }
            }
            d.selection.retain(|&i| match i {
                Item::Measure(n) => d.measurements.iter().any(|x| x.n == n),
                Item::Probe(n) => d.probes.iter().any(|x| x.n == n),
                _ => true,
            });
            // The field shown, if this result has it; the styles of all.
            d.field_styles = s.field_styles;
            d.field_values = None;
            d.field = s.field.filter(|c| {
                let known = m.fields.get(&c.name).is_some();
                if !known {
                    missing.push(format!("field:{}", c.name));
                }
                known
            });
            d.slices = s.slices;
            d.isos = s.isos;
            // Iso-surfaces of fields this result does not have go, with
            // their ids (the later ones move up).
            for k in (0..d.isos.len()).rev() {
                let name = d.isos[k].field.name.clone();
                if m.fields.get(&name).is_none() {
                    d.remove_iso(k);
                    let id = format!("field:{name}");
                    if !missing.contains(&id) {
                        missing.push(id);
                    }
                }
            }
            // Arrows of a field this result does not have, or of one
            // without three components, are left out.
            d.arrows = s.arrows.filter(|a| {
                let vector = m.fields.get(&a.field).is_some_and(|f| f.is_vector());
                if !vector {
                    let id = format!("field:{}", a.field);
                    if !missing.contains(&id) {
                        missing.push(id);
                    }
                }
                vector
            });
            if d.arrows.is_none() {
                d.selection.retain(|&i| i != Item::Arrows);
                d.colors.remove(&Item::Arrows);
            }
            // Regions of a zone or patch this mesh does not have, or of
            // cells beyond its count, go, with their ids.
            d.regions = s.regions;
            for k in (0..d.regions.len()).rev() {
                if d.regions[k].shape.problem(&m.mesh).is_some() {
                    d.remove_region(k);
                    missing.push(format!("region:{}", k + 1));
                }
            }
            let nr = d.regions.len();
            d.selection.retain(|&i| match i {
                Item::Region(k) => (k as usize) < nr,
                _ => true,
            });
            d.colors.retain(|&i, _| !matches!(i, Item::Region(_)));
            // Streamlines of a field this result does not have (or not a
            // vector), or seeded on a patch this mesh does not have, go
            // with their ids.
            let nst = s.streams.len();
            d.streams = s.streams;
            d.selection.retain(|&i| match i {
                Item::Stream(k) => (k as usize) < nst,
                _ => true,
            });
            d.colors.retain(|&i, _| match i {
                Item::Stream(k) => (k as usize) < nst,
                _ => true,
            });
            for k in (0..d.streams.len()).rev() {
                let st = &d.streams[k];
                let vector = m.fields.get(&st.field).is_some_and(|f| f.is_vector());
                let colour = st
                    .color
                    .as_ref()
                    .is_none_or(|c| m.fields.get(&c.name).is_some());
                let id = if !vector {
                    Some(format!("field:{}", st.field))
                } else if !colour {
                    st.color.as_ref().map(|c| format!("field:{}", c.name))
                } else {
                    st.seed
                        .problem(&m.mesh)
                        .map(|_| format!("stream:{}", k + 1))
                };
                if let Some(id) = id {
                    d.remove_stream(k);
                    if !missing.contains(&id) {
                        missing.push(id);
                    }
                }
            }
            // The parcels, their file read again (the caller put the
            // path to read in place, `SessionFile::parcels_path`).
            if let Some(v) = s.parcels {
                match murmel_mesh::read_parcels(&v.path) {
                    Ok(p) => {
                        d.parcel_data = Some(std::sync::Arc::new(p));
                        d.parcels = Some(v);
                    }
                    Err(_) => missing.push(format!("parcels:{}", v.path.display())),
                }
            }
            if d.parcels.is_none() {
                d.selection.retain(|&i| i != Item::Parcels);
                d.colors.remove(&Item::Parcels);
            }
            let (ns, ni) = (d.slices.len(), d.isos.len());
            d.selection.retain(|&i| match i {
                Item::Slice(k) => (k as usize) < ns,
                Item::Iso(k) => (k as usize) < ni,
                _ => true,
            });
            d.colors.retain(|&i, _| match i {
                Item::Iso(k) => (k as usize) < ni,
                Item::Slice(_) => false,
                _ => true,
            });
        }
        if self.display.color_by == ColorBy::Field && self.display.field.is_none() {
            self.display.color_by = ColorBy::Solid;
        }
        self.journal.clear();
        self.redraw();
        missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: a relative path climbs out of the session's folder with
    /// `..` and back down to the mesh, so `dir.join(relative)` is the
    /// mesh again. A path joined without the climb would point inside
    /// the session's folder.
    #[test]
    fn relative_paths() {
        let r = |a: &str, b: &str| relative(Path::new(a), Path::new(b)).unwrap();
        assert_eq!(r("/a/b", "/a/b/c/m.vtu"), PathBuf::from("c/m.vtu"));
        assert_eq!(r("/a/b/s", "/a/c/m.vtu"), PathBuf::from("../../c/m.vtu"));
        assert_eq!(r("/a/b", "/a/b"), PathBuf::from("."));
        assert_eq!(absolute(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
    }

    /// Invariant: a file of another format, a newer major version, a zero
    /// camera distance, a zero clip normal or arrows out of range is
    /// refused before anything
    /// changes; a session from a different tool would otherwise reset
    /// the view to defaults.
    #[test]
    fn bad_files_are_refused() {
        let ok = SessionFile {
            format: SESSION_FORMAT.into(),
            format_version: SESSION_VERSION.into(),
            mesh: None,
            camera: Camera::default(),
            display: DisplayState::default(),
            colors: MeshColors::default(),
            view_cube: CubeSettings::default(),
            views: vec![SavedView {
                name: "Inlet".into(),
                camera: Camera::default(),
            }],
            derived: vec![
                Made::Quantity(crate::derive::Recipe::new(
                    crate::derive::Quantity::Vorticity,
                )),
                Made::Formula(crate::calc::Formula {
                    name: "speed".into(),
                    formula: "mag(U)".into(),
                    unit: Some("m/s".into()),
                }),
            ],
            series: None,
        };
        assert!(SessionFile::parse(&ok.to_json()).is_ok());
        let with = |f: &dyn Fn(&mut serde_json::Value)| {
            let mut v = serde_json::to_value(&ok).unwrap();
            f(&mut v);
            SessionFile::parse(&v.to_string())
        };
        assert!(with(&|v| v["format"] = "murmelcraft.settings".into()).is_err());
        assert!(with(&|v| v["format_version"] = "2.0".into()).is_err());
        assert!(with(&|v| v["format_version"] = "1.3".into()).is_ok());
        for bad in [
            serde_json::json!({"bands": 1}),
            serde_json::json!({"line_count": 99}),
            serde_json::json!({"map": "lava"}),
            serde_json::json!({"range": {"fixed": {"min": 1.0, "max": 1.0}}}),
        ] {
            assert!(
                with(&|v| v["display"]["field_styles"] =
                    serde_json::json!({ "pressure": bad.clone() }))
                .is_err(),
                "{bad}"
            );
        }
        assert!(
            with(&|v| v["display"]["field_styles"] =
                serde_json::json!({"pressure": {"mode": "banded", "range": {"fixed": {"min": 0.0, "max": 2.0}}}}))
            .is_ok()
        );
        // Arrows (1.6): a density of 4 to 200 and a scale in (0, 100].
        let arrows = |extra: serde_json::Value| {
            let mut a = serde_json::json!({"field": "velocity", "enabled": true});
            for (k, x) in extra.as_object().unwrap() {
                a[k] = x.clone();
            }
            with(&|v| v["display"]["arrows"] = a.clone())
        };
        assert!(arrows(serde_json::json!({"density": 40, "scale": 2.5, "in_plane": true})).is_ok());
        for bad in [
            serde_json::json!({"density": 3}),
            serde_json::json!({"density": 201}),
            serde_json::json!({"scale": 0.0}),
            serde_json::json!({"scale": 101.0}),
            serde_json::json!({"field": ""}),
            serde_json::json!({"shape": "cone"}),
        ] {
            assert!(arrows(bad.clone()).is_err(), "{bad}");
        }
        assert!(with(&|v| v["camera"]["distance"] = 0.0.into()).is_err());
        assert!(with(&|v| v["surprise"] = 1.into()).is_err());
        // A derived field with a reference density of 0, or unknown.
        assert!(
            with(&|v| v["derived"] = serde_json::json!([{
                "quantity": "pressure-coefficient",
                "reference": {"density": 0.0, "pressure": 0.0, "velocity": 1.0}
            }]))
            .is_err()
        );
        assert!(with(&|v| v["derived"] = serde_json::json!([{"quantity": "helicity"}])).is_err());
        // A formula that cannot be read, or names a quantity's field
        // (1.14); a formula of fields the mesh lacks loads (they are
        // reported missing then).
        for bad in [
            serde_json::json!({"name": "s", "formula": "mag(U"}),
            serde_json::json!({"name": "vorticity", "formula": "1"}),
            serde_json::json!({"name": "s", "formula": "1", "colour": "red"}),
        ] {
            assert!(
                with(&|v| v["derived"] = serde_json::json!([bad.clone()])).is_err(),
                "{bad}"
            );
        }
        assert!(
            with(&|v| v["derived"] =
                serde_json::json!([{"name": "s", "formula": "nothing_here * 2"}]))
            .is_ok()
        );
        assert!(
            with(&|v| v["display"]["clip"] = serde_json::json!({
                "origin": [0, 0, 0], "normal": [0, 0, 0], "mode": "flat", "enabled": true
            }))
            .is_err()
        );
        assert!(with(&|v| v["views"][0]["camera"]["distance"] = (-1.0).into()).is_err());
        assert!(with(&|v| v["views"][0]["name"] = " ".into()).is_err());
        assert!(
            with(&|v| v["views"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({"name": "Inlet", "camera": Camera::default()})))
            .is_err()
        );
        // A 1.0 file has no views and still loads.
        assert!(
            with(&|v| {
                v["format_version"] = "1.0".into();
                v.as_object_mut().unwrap().remove("views");
            })
            .is_ok()
        );
        assert!(with(&|v| v["colors"]["role"] = serde_json::json!({"lava": "#FF0000"})).is_err());
        assert!(SessionFile::parse("{").is_err());
    }
}
