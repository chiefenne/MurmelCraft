//! The viewer session: the open mesh, its surface, the camera and the
//! settings. Front ends read it and change it only through tools.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use murmel_mesh::{Evidence, Mesh, MeshError};

use crate::camera::Camera;
use crate::clip::Clip;
use crate::cube::CubeSettings;
use crate::display::{ColorBy, Display, Item};
use crate::field::{Component, FieldSet};
use crate::journal::Journal;
use crate::navigation::Navigation;
use crate::quality::Quality;
use crate::surface::Surface;
use crate::theme::{MeshColors, Theme, Tokens};

/// A mesh loaded into the session.
#[derive(Clone, Debug)]
pub struct OpenMesh {
    pub path: PathBuf,
    /// Shared, so a surface can be built on a thread of its own (the
    /// window's clip drag: [`Session::defer_clips`]).
    pub mesh: Arc<Mesh>,
    pub evidence: Option<Evidence>,
    /// Why the evidence beside the mesh could not be read (MESH011).
    pub evidence_problem: Option<MeshError>,
    pub surface: Surface,
    /// SHA-256 of the mesh file, hex. A mesh of several files (an
    /// OpenFOAM polyMesh) gets the SHA-256 of its `files` listed as
    /// `sha256sum` prints them (`<hex>  <name>` lines).
    pub digest: String,
    /// The name and SHA-256 (hex) of each file the mesh was read from.
    pub files: Vec<(String, String)>,
    /// The validity badge of the tree root.
    pub validity: Validity,
    /// The fields read with the mesh (a `.vtkhdf` result, the data
    /// arrays of a `.vtu`); empty for a bare mesh.
    pub fields: Arc<FieldSet>,
    /// The display `surface` was built for.
    pub built_for: Display,
}

/// The validity badge (UI spec, Model tree): green when the evidence
/// beside the mesh matches it, amber when there is none, red when a check
/// fails; grey while the SHA-256 of the files is still being taken after
/// the mesh opened.
#[derive(Clone, Debug, PartialEq)]
pub enum Validity {
    Valid,
    NoEvidence,
    Failed(Vec<String>),
    Checking,
}

impl Validity {
    pub fn key(&self) -> &'static str {
        match self {
            Validity::Valid => "valid",
            Validity::NoEvidence => "no_evidence",
            Validity::Failed(_) => "failed",
            Validity::Checking => "checking",
        }
    }

    /// The badge of a mesh whose files are not hashed yet: the checks
    /// that need no digest (the evidence's problem, its counts) can fail
    /// already; otherwise it is checking, or has no evidence.
    pub fn before_hash(
        mesh: &Mesh,
        evidence: Option<&Evidence>,
        problem: Option<&MeshError>,
    ) -> Validity {
        if let Some(p) = problem {
            return Validity::Failed(vec![format!("{}: {}", p.code, p.message)]);
        }
        let Some(e) = evidence else {
            return Validity::NoEvidence;
        };
        let why = counts(mesh, e);
        if why.is_empty() {
            Validity::Checking
        } else {
            Validity::Failed(why)
        }
    }

    /// Check `mesh`, read from `files` (name and SHA-256 of each), against
    /// its evidence: every file the evidence lists must match, and the
    /// mesh file (the `.vtu`, or the polyMesh's `faces`) must be listed.
    pub fn check(
        files: &[(String, String)],
        mesh: &Mesh,
        evidence: Option<&Evidence>,
        problem: Option<&MeshError>,
    ) -> Validity {
        if let Some(p) = problem {
            return Validity::Failed(vec![format!("{}: {}", p.code, p.message)]);
        }
        let Some(e) = evidence else {
            return Validity::NoEvidence;
        };
        let mut why = Vec::new();
        let mut listed = 0;
        for (name, digest) in files {
            // OpenFOAM's compressed files are checked by their plain name.
            let plain = name.strip_suffix(".gz").unwrap_or(name);
            match e.digest(name).or_else(|| e.digest(plain)) {
                Some(d) if d.eq_ignore_ascii_case(digest) => listed += 1,
                Some(_) => {
                    listed += 1;
                    why.push(format!(
                        "{name} differs from the file the evidence describes"
                    ));
                }
                None => {}
            }
        }
        if listed == 0 {
            let name = files.first().map_or("the mesh file", |f| f.0.as_str());
            why.push(format!("the evidence does not list {name}"));
        }
        why.extend(counts(mesh, e));
        if why.is_empty() {
            Validity::Valid
        } else {
            Validity::Failed(why)
        }
    }
}

/// What the evidence's counts say against `mesh`.
fn counts(mesh: &Mesh, e: &Evidence) -> Vec<String> {
    let mut why = Vec::new();
    if let Some(c) = e.cells
        && c as usize != mesh.cell_count()
    {
        why.push(format!("{} cells, evidence says {c}", mesh.cell_count()));
    }
    if let Some(b) = e.boundary_faces
        && b as usize != mesh.boundary.len()
    {
        why.push(format!(
            "{} boundary faces, evidence says {b}",
            mesh.boundary.len()
        ));
    }
    if let Some(n) = e.inverted_faces
        && n > 0
    {
        why.push(format!("{n} inverted faces"));
    }
    why
}

/// SHA-256 of a file, hex, read in pieces; empty when it cannot be read.
pub fn sha256(path: &Path) -> String {
    murmel_mesh::sha256_file(path, &AtomicBool::new(false))
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// The digest of a mesh read from `files` (name and SHA-256 of each): the
/// file's own, or for several files their listing as `sha256sum` prints
/// it.
fn combined(files: &[(String, String)]) -> String {
    match files {
        [(_, d)] => d.clone(),
        _ => sha256_bytes(
            files
                .iter()
                .map(|(n, d)| format!("{d}  {n}\n"))
                .collect::<String>()
                .as_bytes(),
        ),
    }
}

/// The name of a file, as `files` lists it.
fn file_name(f: &Path) -> String {
    f.file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned())
}

/// The SHA-256 of an open mesh's files, taken on a thread of its own
/// after the mesh opened (`Session::hash_after_open`). The open mesh
/// shows meanwhile, its badge [`Validity::Checking`];
/// [`Session::poll_digest`] or [`Session::wait_digest`] puts the
/// digests in place.
#[derive(Clone, Debug)]
pub struct Hashing {
    stop: Arc<AtomicBool>,
    /// The digest of each file once all are taken (empty for a file that
    /// could not be read).
    done: Arc<(Mutex<Option<Vec<String>>>, Condvar)>,
}

impl Hashing {
    /// Hash `files` on a thread of its own, remembering them in
    /// `checked` (the settings folder; see [`crate::checked`]).
    fn start(files: Vec<PathBuf>, checked: Option<PathBuf>) -> Hashing {
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new((Mutex::new(None), Condvar::new()));
        let (s, d) = (stop.clone(), done.clone());
        std::thread::spawn(move || {
            let mut digests = Vec::with_capacity(files.len());
            for f in &files {
                match crate::checked::sha256(checked.as_deref(), f, &s) {
                    Ok(Some(h)) => digests.push(h),
                    Ok(None) => return,
                    Err(_) => digests.push(String::new()),
                }
            }
            let (slot, ready) = &*d;
            if let Ok(mut v) = slot.lock() {
                *v = Some(digests);
                ready.notify_all();
            }
        });
        Hashing { stop, done }
    }

    /// The digests, if taken; `wait` blocks until they are.
    fn take(&self, wait: bool) -> Option<Vec<String>> {
        let (slot, ready) = &*self.done;
        let mut v = slot.lock().ok()?;
        while wait && v.is_none() {
            v = ready.wait(v).ok()?;
        }
        v.take()
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// SHA-256 of bytes, hex.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    murmel_mesh::sha256_hex(bytes)
}

/// Whether two displays are equal, with the values per cell they carry
/// (the quality, the field's values, the cells of the regions) compared
/// by allocation, not value by value: they are made once and shared by
/// every copy of the display, and the preview compares displays on every
/// frame, where comparing millions of values would cost more than the
/// frame.
fn same_display(a: &Display, b: &Display) -> bool {
    fn same<T>(x: &Option<Arc<T>>, y: &Option<Arc<T>>) -> bool {
        match (x, y) {
            (None, None) => true,
            (Some(p), Some(q)) => Arc::ptr_eq(p, q),
            _ => false,
        }
    }
    let bare = |d: &Display| Display {
        quality: None,
        field_values: None,
        region_cells: None,
        stream_traces: None,
        parcel_data: None,
        ..d.clone()
    };
    same(&a.quality, &b.quality)
        && same(&a.field_values, &b.field_values)
        && same(&a.region_cells, &b.region_cells)
        && same(&a.stream_traces, &b.stream_traces)
        && same(&a.parcel_data, &b.parcel_data)
        && bare(a) == bare(b)
}

/// The clip planes that move in `new` against `old` (their origin or
/// normal changed, nothing else in the display), or `None` when the
/// change is not only such a move.
fn clip_moves(old: &Display, new: &Display) -> Option<Vec<usize>> {
    if old.clips.len() != new.clips.len() {
        return None;
    }
    let mut moving = Vec::new();
    for (k, (a, b)) in old.clips.iter().zip(&new.clips).enumerate() {
        if a.mode != b.mode || a.enabled != b.enabled {
            return None;
        }
        if a != b {
            moving.push(k);
        }
    }
    let same_otherwise = same_display(
        &Display {
            clips: new.clips.clone(),
            ..old.clone()
        },
        new,
    );
    (same_otherwise && !moving.is_empty()).then_some(moving)
}

/// A clip drag in the window: the planes `moving` have left the place
/// the surface was cut at. Until the exact cut is built (on a thread of
/// its own), the renderer draws the surface built without the moving
/// planes (`base`) and cuts it on the GPU, which shows the cut faces
/// hollow but follows the pointer at once.
#[derive(Clone, Debug, Default)]
pub struct ClipPreview {
    /// The planes (indices into `Display::clips`) that moved.
    pub moving: Vec<usize>,
    /// When a plane last moved.
    changed: Option<Instant>,
    /// The surface without the moving planes, the display it was built
    /// for and its revision.
    base: Option<(Display, Arc<Surface>, u64)>,
    /// The builds running: of the surface without the moving planes, and
    /// of the exact cut (started once the planes rest).
    base_job: Option<SurfaceJob>,
    exact_job: Option<SurfaceJob>,
}

impl ClipPreview {
    /// How long the planes rest before the exact cut is built, so a
    /// drag does not start a build on every step.
    pub const REST: Duration = Duration::from_millis(120);

    /// The display of the surface without the moving planes: `display`
    /// with them switched off where the surface had them, so moving them
    /// further keeps it.
    fn base_display(&self, built_for: &Display, display: &Display) -> Display {
        let mut d = display.clone();
        for &k in &self.moving {
            if let (Some(c), Some(old)) = (d.clips.get_mut(k), built_for.clips.get(k)) {
                *c = Clip {
                    enabled: false,
                    ..*old
                };
            }
        }
        d
    }
}

/// A surface built on a thread of its own for `display`.
#[derive(Clone, Debug)]
struct SurfaceJob {
    display: Display,
    done: Arc<(Mutex<Option<Surface>>, Condvar)>,
}

impl SurfaceJob {
    fn start(m: &OpenMesh, tokens: &Tokens, display: Display) -> SurfaceJob {
        let done = Arc::new((Mutex::new(None), Condvar::new()));
        let (mesh, fields, tokens, d, out) = (
            m.mesh.clone(),
            m.fields.clone(),
            tokens.clone(),
            display.clone(),
            done.clone(),
        );
        std::thread::spawn(move || {
            let s = Surface::build_with(&mesh, Some(&fields), &tokens, &d);
            let (lock, ready) = &*out;
            if let Ok(mut v) = lock.lock() {
                *v = Some(s);
                ready.notify_all();
            }
        });
        SurfaceJob { display, done }
    }

    /// The surface once built (waiting for it with `wait`).
    fn take(&self, wait: bool) -> Option<Surface> {
        let (lock, ready) = &*self.done;
        let mut v = lock.lock().ok()?;
        while wait && v.is_none() {
            v = ready.wait(v).ok()?;
        }
        v.take()
    }
}

/// Compute the cell quality of `mesh` the first time colour mode quality
/// needs it; it is kept until another mesh opens.
fn ensure_quality(display: &mut Display, mesh: &Mesh) {
    if display.color_by == ColorBy::Quality && display.quality.is_none() {
        display.quality = Some(std::sync::Arc::new(Quality::compute(mesh)));
    }
}

/// The display a new mesh opens with: everything shown, nothing selected
/// and no saved views, measurements, probes, slices or iso-surfaces (they
/// are in the old mesh's coordinates); the colouring stays. The field
/// colouring stays when the new mesh has the field (the next time step of
/// the same case), else it falls back to solid.
fn display_for_new_mesh(old: &Display, fields: &FieldSet) -> Display {
    let mut d = old.clone();
    d.hidden.clear();
    d.selection.clear();
    d.isolated.clear();
    d.colors.clear();
    d.clips.clear();
    d.threshold = None;
    d.quality = None;
    d.views.clear();
    d.measurements.clear();
    d.probes.clear();
    d.slices.clear();
    d.isos.clear();
    d.arrows = None;
    d.streams.clear();
    d.stream_traces = None;
    d.parcels = None;
    d.parcel_data = None;
    d.field_values = None;
    d.regions.clear();
    d.region_cells = None;
    keep_field(&mut d, fields);
    d
}

/// The field colouring stays when `fields` has the field, else it falls
/// back to solid.
fn keep_field(d: &mut Display, fields: &FieldSet) {
    if d.field
        .as_ref()
        .is_some_and(|c| fields.get(&c.name).is_none())
    {
        d.field = None;
        if d.color_by == ColorBy::Field {
            d.color_by = ColorBy::Solid;
        }
    }
    if d.color_by == ColorBy::Field && d.field.is_none() {
        d.color_by = ColorBy::Solid;
    }
}

/// The display (with the quality or field values it colours by) and the
/// surface of a new mesh, from its display `base`.
fn prepare(
    mut base: Display,
    mesh: &Mesh,
    fields: &FieldSet,
    tokens: &Tokens,
) -> (Display, Surface) {
    crate::region::ensure(&mut base, mesh);
    ensure_quality(&mut base, mesh);
    crate::field::ensure_field(&mut base, mesh, fields);
    let _t = murmel_mesh::profile::span("open.surface");
    let surface = Surface::build_with(mesh, Some(fields), tokens, &base);
    (base, surface)
}

/// The size and modification time of a file, `None` when it is missing.
type Stamp = Option<(u64, Option<SystemTime>)>;

fn stamp(p: &Path) -> Stamp {
    std::fs::metadata(p)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

/// A mesh read with its fields, its evidence and the digests of its
/// files, not yet open: [`load`] makes it on any thread, and
/// [`Session::open`] takes it from `Session::preloaded`.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub path: PathBuf,
    pub mesh: Mesh,
    pub fields: FieldSet,
    pub evidence: Option<Evidence>,
    pub evidence_problem: Option<MeshError>,
    pub files: Vec<(String, String)>,
    pub digest: String,
    pub validity: Validity,
    /// The files are not hashed yet (`load_with` without `hash`).
    pub pending: bool,
    /// Each file read and the evidence beside them, as they were then.
    stamps: Vec<(PathBuf, Stamp)>,
    prepared: Option<Prepared>,
}

/// The surface of a loaded mesh, built for a display and tokens.
#[derive(Clone, Debug)]
struct Prepared {
    /// The display the mesh opens with, before quality or field values.
    base: Display,
    tokens: Tokens,
    display: Display,
    surface: Surface,
}

/// Read a mesh, its fields and the evidence beside it, and hash its files.
pub fn load(path: &Path) -> Result<Loaded, MeshError> {
    load_with(path, true)
}

/// As [`load`]; without `hash` the files are not hashed: the digests stay
/// empty and the badge checking until the session that opens the mesh has
/// taken them ([`Hashing`]).
pub fn load_with(path: &Path, hash: bool) -> Result<Loaded, MeshError> {
    load_checked(path, hash, None)
}

/// As [`load_with`], with the files checked before remembered in
/// `checked` (the settings folder; see [`crate::checked`]): a mesh file
/// unchanged since it was hashed is read without hashing it again, and
/// the files hashed now are remembered.
pub fn load_checked(path: &Path, hash: bool, checked: Option<&Path>) -> Result<Loaded, MeshError> {
    // A mesh of one file, checked before and unchanged since, is read
    // without hashing; the stamp tells later whether the digest the
    // reader takes may be remembered.
    let (known, before) = match checked {
        Some(dir) if hash && path.is_file() => (
            crate::checked::lookup(dir, path),
            crate::checked::Stamp::of(path),
        ),
        _ => (None, None),
    };
    let t = murmel_mesh::profile::span("open.read");
    let (mut mesh, results) = murmel_mesh::read_results_with(path, hash && known.is_none())?;
    drop(t);
    let one_file = |mesh: &Mesh| match mesh.files.as_slice() {
        [f] => std::fs::canonicalize(f).ok() == std::fs::canonicalize(path).ok(),
        _ => false,
    };
    if let Some(d) = &known
        && one_file(&mesh)
    {
        mesh.file_sha256 = vec![d.clone()];
    }
    let fields = FieldSet::new(results);
    // The evidence sits beside the mesh file: beside the .vtu, or in the
    // polyMesh folder among its files.
    let first = mesh
        .files
        .first()
        .cloned()
        .unwrap_or_else(|| path.to_path_buf());
    let (evidence, evidence_problem) = match Evidence::beside(&first) {
        Ok(e) => (e, None),
        Err(e) => (None, Some(e)),
    };
    if let Some(e) = &evidence {
        mesh.apply_evidence_roles(e);
    }
    // The readers hash the bytes they read; a file they did not hash is
    // read again here. Without `hash`, the session hashes them later.
    let files: Vec<(String, String)> = mesh
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let digest = match mesh.file_sha256.get(i).filter(|d| d.len() == 64) {
                Some(d) => d.clone(),
                None if hash => crate::checked::sha256(checked, f, &AtomicBool::new(false))
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                None => String::new(),
            };
            (file_name(f), digest)
        })
        .collect();
    // The digest the reader took of a mesh of one file.
    if let (Some(dir), Some(b), None, [(_, d)]) = (checked, &before, &known, files.as_slice())
        && one_file(&mesh)
    {
        crate::checked::remember(dir, path, b, d);
    }
    let (digest, validity) = if hash {
        let v = Validity::check(&files, &mesh, evidence.as_ref(), evidence_problem.as_ref());
        (combined(&files), v)
    } else {
        let v = Validity::before_hash(&mesh, evidence.as_ref(), evidence_problem.as_ref());
        (String::new(), v)
    };
    let evidence_path = first.parent().map(|d| d.join("mesh-evidence.json"));
    let stamps = mesh
        .files
        .iter()
        .chain(evidence_path.as_ref())
        .map(|f| (f.clone(), stamp(f)))
        .collect();
    Ok(Loaded {
        path: path.to_path_buf(),
        mesh,
        fields,
        evidence,
        evidence_problem,
        files,
        digest,
        validity,
        pending: !hash,
        stamps,
        prepared: None,
    })
}

impl Loaded {
    /// Build in advance the surface the mesh opens with in a session of
    /// `display` and `tokens` (used if they are unchanged when it opens).
    pub fn prepare(&mut self, display: &Display, tokens: &Tokens) {
        let base = display_for_new_mesh(display, &self.fields);
        let (display, surface) = prepare(base.clone(), &self.mesh, &self.fields, tokens);
        self.prepared = Some(Prepared {
            base,
            tokens: tokens.clone(),
            display,
            surface,
        });
    }

    /// Whether its files and evidence are as they were when it was read.
    pub fn is_fresh(&self) -> bool {
        self.stamps.iter().all(|(f, s)| stamp(f) == *s)
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub tokens: Tokens,
    pub mesh: Option<OpenMesh>,
    pub camera: Camera,
    /// Colouring, hidden items and selection of the open mesh.
    pub display: Display,
    pub navigation: Navigation,
    pub theme: Theme,
    /// The user's mesh colours, already laid over `tokens`.
    pub colors: MeshColors,
    /// Where the window shows the view cube.
    pub view_cube: CubeSettings,
    /// Viewport size in pixels, for aspect, pan and zoom.
    pub viewport: [f64; 2],
    /// Bumped whenever the surface changes, so renderers re-upload.
    pub surface_revision: u64,
    /// Whether the window lets agents drive it (`settings.set_agents`).
    pub agents: bool,
    /// Where settings tools write; `None` disables them.
    pub config_dir: Option<PathBuf>,
    /// Where new items start (`settings.set_default`, settings.toml).
    pub defaults: crate::defaults::Defaults,
    /// Your formulas (`settings.keep_formula`, settings.toml).
    pub formulas: Vec<crate::calc::Formula>,
    /// The undo and redo steps of the open mesh.
    pub journal: Journal,
    /// A mesh read in advance (the window reads a large one on a thread
    /// of its own), which the next `mesh.open` of its path takes.
    pub preloaded: Option<Loaded>,
    /// Open a mesh before the SHA-256 of its files is taken, and take it
    /// on a thread of its own afterwards (the window: hashing a large
    /// file can take longer than reading it). Off, `mesh.open` returns
    /// with the digest, as the command line and agents expect.
    pub hash_after_open: bool,
    /// The SHA-256 of the open mesh's files being taken.
    pub hashing: Option<Hashing>,
    /// Remember the SHA-256 of the files hashed in `config_dir`, and take
    /// it from there for a file unchanged since ([`crate::checked`]; the
    /// window and the command line).
    pub remember_checked: bool,
    /// Build the cut of a moving clip plane on a thread of its own (the
    /// window, the user's choice for the clip drag): a change of `display`
    /// that only moves or turns clip planes leaves the surface as it is
    /// and starts a [`ClipPreview`]; the renderer clips on the GPU until
    /// the exact cut is in place. Off, every change builds the surface
    /// before it returns, as the command line and agents expect.
    pub defer_clips: bool,
    /// The clip planes moving while their exact cut is built.
    pub preview: Option<ClipPreview>,
    /// The time series being walked (`series.open`), whose current entry
    /// is the open mesh and parcels; ended by opening another mesh.
    pub series: Option<crate::series::Series>,
    /// The tables of a run charted (`charts.open`) and the chart the
    /// window's panel shows; kept when another mesh opens.
    pub charts: Option<crate::chart::Charts>,
}

impl Session {
    pub fn new(tokens: Tokens) -> Self {
        Self {
            tokens,
            mesh: None,
            camera: Camera::default(),
            display: Display::default(),
            navigation: Navigation::default(),
            theme: Theme::default(),
            colors: MeshColors::default(),
            view_cube: CubeSettings::default(),
            viewport: [1280.0, 720.0],
            surface_revision: 0,
            agents: true,
            config_dir: crate::settings::config_dir(),
            defaults: crate::defaults::Defaults::default(),
            formulas: Vec::new(),
            journal: Journal::default(),
            preloaded: None,
            hash_after_open: false,
            hashing: None,
            remember_checked: false,
            defer_clips: false,
            preview: None,
            series: None,
            charts: None,
        }
    }

    /// Put the digests of the open mesh's files in place once they are
    /// taken (`hash_after_open`), with the badge they decide. True when
    /// that happened now (the window draws the badge again).
    pub fn poll_digest(&mut self) -> bool {
        self.finish_digest(false)
    }

    /// Wait for the digests of the open mesh's files and put them in
    /// place: for tools that need them (`session.save`, `session.load`).
    pub fn wait_digest(&mut self) {
        self.finish_digest(true);
    }

    fn finish_digest(&mut self, wait: bool) -> bool {
        let Some(h) = &self.hashing else {
            return false;
        };
        let Some(digests) = h.take(wait) else {
            return false;
        };
        self.hashing = None;
        let Some(m) = &mut self.mesh else {
            return false;
        };
        for ((_, d), h) in m.files.iter_mut().zip(digests) {
            *d = h;
        }
        m.digest = combined(&m.files);
        m.validity = Validity::check(
            &m.files,
            &m.mesh,
            m.evidence.as_ref(),
            m.evidence_problem.as_ref(),
        );
        true
    }

    /// Where the files checked are remembered, if anywhere.
    pub fn checked_dir(&self) -> Option<&Path> {
        self.config_dir.as_deref().filter(|_| self.remember_checked)
    }

    pub fn aspect(&self) -> f64 {
        self.viewport[0].max(1.0) / self.viewport[1].max(1.0)
    }

    /// Read a mesh and its evidence, replace the open one and fit the view.
    /// The mesh [`load`]ed in advance into `preloaded` is used when it is
    /// of `path` and its files have not changed since.
    pub fn open(&mut self, path: &Path) -> Result<&OpenMesh, MeshError> {
        let loaded = match self.preloaded.take() {
            Some(l) if l.path == path && l.is_fresh() => l,
            _ => load_checked(path, !self.hash_after_open, self.checked_dir())?,
        };
        self.series = None;
        Ok(self.install(loaded))
    }

    /// Open the fields file of another time step of the same run: when
    /// its mesh has the open one's cells and points, the display (slices,
    /// iso-surfaces, streamlines, clip planes, …), the derived fields,
    /// the camera and the journal stay and only the values change;
    /// otherwise it opens as a new mesh does, but the camera stays. The
    /// series stays open either way.
    pub fn open_step(&mut self, path: &Path) -> Result<(), MeshError> {
        let mut l = load_with(path, !self.hash_after_open)?;
        let same = self.mesh.as_ref().is_some_and(|m| {
            m.mesh.cell_count() == l.mesh.cell_count() && m.mesh.points.len() == l.mesh.points.len()
        });
        if !same {
            let camera = self.camera.clone();
            self.install(l);
            self.camera = camera;
            return Ok(());
        }
        let Some(old) = self.mesh.take() else {
            return Ok(());
        };
        // The fields derived and calculated on the step before, made
        // again of this one's, in order.
        for m in old.fields.made() {
            if let Ok((set, _)) = l.fields.remake(&l.mesh, m.clone()) {
                l.fields = set;
            }
        }
        if let Some(h) = self.hashing.take() {
            h.stop();
        }
        if l.pending {
            let dir = self.checked_dir().map(Path::to_path_buf);
            self.hashing = Some(Hashing::start(l.mesh.files.clone(), dir));
        }
        let mut base = self.display.clone();
        base.field_values = None;
        base.stream_traces = None;
        base.quality = None;
        base.region_cells = None;
        keep_field(&mut base, &l.fields);
        let (display, surface) = prepare(base, &l.mesh, &l.fields, &self.tokens);
        self.display = display;
        self.surface_revision += 1;
        self.preview = None;
        self.mesh = Some(OpenMesh {
            path: l.path,
            built_for: self.display.clone(),
            mesh: Arc::new(l.mesh),
            evidence: l.evidence,
            evidence_problem: l.evidence_problem,
            surface,
            digest: l.digest,
            files: l.files,
            validity: l.validity,
            fields: Arc::new(l.fields),
        });
        Ok(())
    }

    /// Put a loaded mesh in place of the open one and fit the view. The
    /// surface built in advance (`Loaded::prepare`) is used when the
    /// display and the tokens are still the ones it was built for.
    fn install(&mut self, mut l: Loaded) -> &OpenMesh {
        self.journal.clear();
        if let Some(h) = self.hashing.take() {
            h.stop();
        }
        if l.pending {
            let dir = self.checked_dir().map(Path::to_path_buf);
            self.hashing = Some(Hashing::start(l.mesh.files.clone(), dir));
        }
        let base = display_for_new_mesh(&self.display, &l.fields);
        let (display, surface) = match l.prepared.take() {
            Some(p) if p.base == base && p.tokens == self.tokens => (p.display, p.surface),
            _ => prepare(base, &l.mesh, &l.fields, &self.tokens),
        };
        self.display = display;
        if let Some(b) = &surface.bounds {
            self.camera.fit(b, self.aspect());
        }
        self.surface_revision += 1;
        self.preview = None;
        self.mesh.insert(OpenMesh {
            path: l.path,
            built_for: self.display.clone(),
            mesh: Arc::new(l.mesh),
            evidence: l.evidence,
            evidence_problem: l.evidence_problem,
            surface,
            digest: l.digest,
            files: l.files,
            validity: l.validity,
            fields: Arc::new(l.fields),
        })
    }

    pub fn close(&mut self) {
        self.mesh = None;
        self.preview = None;
        if let Some(h) = self.hashing.take() {
            h.stop();
        }
        self.journal.clear();
        self.display.hidden.clear();
        self.display.selection.clear();
        self.display.isolated.clear();
        self.display.colors.clear();
        self.display.clips.clear();
        self.display.threshold = None;
        self.display.quality = None;
        self.display.views.clear();
        self.display.measurements.clear();
        self.display.probes.clear();
        self.display.slices.clear();
        self.display.isos.clear();
        self.display.arrows = None;
        self.display.streams.clear();
        self.display.stream_traces = None;
        self.display.field_values = None;
        self.display.regions.clear();
        self.display.region_cells = None;
        self.display.parcels = None;
        self.display.parcel_data = None;
        self.series = None;
        self.surface_revision += 1;
    }

    /// Replace the user's mesh colours: the built-in tokens with `colors`
    /// laid over them, and the surface rebuilt. Refused colours change
    /// nothing.
    pub fn set_colors(&mut self, colors: MeshColors) -> Result<(), String> {
        self.tokens = Tokens::builtin()?.with_colors(&colors)?;
        self.colors = colors;
        self.redraw();
        Ok(())
    }

    /// Build the surface again after a change of `display`. With
    /// [`Session::defer_clips`], a change that only moves or turns clip
    /// planes starts a [`ClipPreview`] instead and returns at once.
    pub fn redraw(&mut self) {
        let Some(m) = &mut self.mesh else {
            self.preview = None;
            self.surface_revision += 1;
            return;
        };
        crate::region::ensure(&mut self.display, &m.mesh);
        ensure_quality(&mut self.display, &m.mesh);
        crate::field::ensure_field(&mut self.display, &m.mesh, &m.fields);
        crate::stream::ensure(&mut self.display, &m.mesh, &m.fields);
        if self.defer_clips
            && let Some(moving) = clip_moves(&m.built_for, &self.display)
        {
            let p = self.preview.get_or_insert_with(ClipPreview::default);
            p.moving = moving;
            p.changed = Some(Instant::now());
            return;
        }
        self.preview = None;
        m.surface = Surface::build_with(&m.mesh, Some(&m.fields), &self.tokens, &self.display);
        m.built_for = self.display.clone();
        self.surface_revision += 1;
    }

    /// Compute the derived field of `recipe` ([`crate::derive`]) on the
    /// open mesh and add it to the mesh's fields, in place of one the
    /// recipe made before, then draw again; the name of the field made.
    pub fn derive(
        &mut self,
        recipe: crate::derive::Recipe,
    ) -> Result<&'static str, crate::derive::Refusal> {
        let name = recipe.field();
        self.make(crate::derive::Made::Quantity(recipe))?;
        Ok(name)
    }

    /// Compute the field of `formula` ([`crate::calc`]) on the open mesh
    /// and add it to the mesh's fields, as [`Session::derive`]; the names
    /// of the fields made, the formula's first, then those made from it
    /// again.
    pub fn calculate(
        &mut self,
        formula: crate::calc::Formula,
    ) -> Result<Vec<String>, crate::derive::Refusal> {
        self.make(crate::derive::Made::Formula(formula))
    }

    /// Make the field of `made` (again), and every field made from it,
    /// then draw again; the names of the fields made. Refused, with
    /// nothing changed, when a field made again changes between scalar
    /// and vector while items are made from it.
    pub fn make(
        &mut self,
        made: crate::derive::Made,
    ) -> Result<Vec<String>, crate::derive::Refusal> {
        use crate::derive::Refusal;
        let Some(m) = &self.mesh else {
            return Err(Refusal::NoFields);
        };
        let (set, names) = m.fields.remake(&m.mesh, made)?;
        for n in &names {
            let before = m.fields.get(n).map(|f| f.components);
            let after = set.get(n).map(|f| f.components);
            if before.is_some() && before != after {
                let users = self.users_of(n);
                if !users.is_empty() {
                    return Err(Refusal::InUse(n.clone(), users));
                }
            }
        }
        if let Some(m) = &mut self.mesh {
            m.fields = Arc::new(set);
        }
        for n in &names {
            self.forget_values(n);
        }
        self.redraw();
        Ok(names)
    }

    /// The ids of the items made from field `name`: iso-surfaces, the
    /// arrows, streamline sets.
    fn users_of(&self, name: &str) -> Vec<String> {
        let Some(m) = &self.mesh else {
            return Vec::new();
        };
        let d = &self.display;
        let mut users: Vec<Item> = Vec::new();
        for (k, iso) in d.isos.iter().enumerate() {
            if iso.field.name == name {
                users.push(Item::Iso(k as u32));
            }
        }
        if d.arrows.as_ref().is_some_and(|a| a.field == name) {
            users.push(Item::Arrows);
        }
        for (k, st) in d.streams.iter().enumerate() {
            if st.field == name || st.color.as_ref().is_some_and(|c| c.name == name) {
                users.push(Item::Stream(k as u32));
            }
        }
        users.iter().map(|i| i.id(&m.mesh)).collect()
    }

    /// Remove derived or calculated field `name` from the open mesh's
    /// fields. Refused while an iso-surface, the arrows, a streamline
    /// set or another made field is made from it (their ids and names
    /// are given); colouring by it moves on to the result's first field.
    pub fn underive(&mut self, name: &str) -> Result<(), crate::derive::Refusal> {
        use crate::derive::Refusal;
        let Some(m) = &self.mesh else {
            return Err(Refusal::NotDerived(name.into()));
        };
        let set = m.fields.unmake(name)?;
        let users = self.users_of(name);
        if !users.is_empty() {
            return Err(Refusal::InUse(name.into(), users));
        }
        let first = set.names().first().map(|n| n.to_string());
        if let Some(m) = &mut self.mesh {
            m.fields = Arc::new(set);
        }
        let d = &mut self.display;
        if d.field.as_ref().is_some_and(|c| c.name == name) {
            d.field = first.map(|name| crate::field::FieldChoice {
                name,
                component: Component::Magnitude,
            });
            if d.field.is_none() && d.color_by == ColorBy::Field {
                d.color_by = ColorBy::Solid;
            }
        }
        self.forget_values(name);
        self.redraw();
        Ok(())
    }

    /// Drop what the display computed from field `name` (it changed).
    fn forget_values(&mut self, name: &str) {
        let d = &mut self.display;
        if d.field_values
            .as_ref()
            .is_some_and(|v| v.choice.name == name)
        {
            d.field_values = None;
        }
        d.stream_traces = None;
    }

    /// Follow the clip preview: put a finished surface in place and start
    /// the builds it needs (the surface without the moving planes, and,
    /// once the planes have rested for [`ClipPreview::REST`], the exact
    /// cut; one of each at a time, the latest display winning). True
    /// while the preview goes on; the window asks every frame.
    pub fn poll_surface(&mut self) -> bool {
        let (Some(p), Some(m)) = (&mut self.preview, &mut self.mesh) else {
            self.preview = None;
            return false;
        };
        if let Some(surface) = p.exact_job.as_ref().and_then(|j| j.take(false)) {
            let job = p.exact_job.take();
            if let Some(job) = job.filter(|j| same_display(&j.display, &self.display)) {
                m.surface = surface;
                m.built_for = job.display;
                self.surface_revision += 1;
                self.preview = None;
                return false;
            }
        }
        let base = p.base_display(&m.built_for, &self.display);
        if let Some(surface) = p.base_job.as_ref().and_then(|j| j.take(false))
            && let Some(job) = p
                .base_job
                .take()
                .filter(|j| same_display(&j.display, &base))
        {
            self.surface_revision += 1;
            p.base = Some((job.display, Arc::new(surface), self.surface_revision));
        }
        if p.base_job.is_none() && p.base.as_ref().is_none_or(|b| !same_display(&b.0, &base)) {
            p.base_job = Some(SurfaceJob::start(m, &self.tokens, base));
        }
        let rested = p.changed.is_none_or(|t| t.elapsed() >= ClipPreview::REST);
        if rested && p.exact_job.is_none() {
            p.exact_job = Some(SurfaceJob::start(m, &self.tokens, self.display.clone()));
        }
        true
    }

    /// Put the exact surface of `display` in place now, waiting for the
    /// build of it already running or building it here; nothing to do
    /// without a preview. Tools call this first, so what they read or
    /// draw is the exact cut.
    pub fn settle(&mut self) {
        let Some(p) = self.preview.take() else {
            return;
        };
        let Some(m) = &mut self.mesh else {
            return;
        };
        let done = p
            .exact_job
            .filter(|j| same_display(&j.display, &self.display))
            .and_then(|j| j.take(true));
        m.surface = done.unwrap_or_else(|| {
            Surface::build_with(&m.mesh, Some(&m.fields), &self.tokens, &self.display)
        });
        m.built_for = self.display.clone();
        self.surface_revision += 1;
    }

    /// The surface to draw, its revision and the clip planes the renderer
    /// cuts it with (`[n, d]`: a point `p` relative to the surface's
    /// origin is cut away where `n · p + d > 0`; empty without a preview).
    pub fn drawn(&self) -> Option<(&Surface, u64, Vec<[f32; 4]>)> {
        let m = self.mesh.as_ref()?;
        let Some(p) = &self.preview else {
            return Some((&m.surface, self.surface_revision, Vec::new()));
        };
        let base = p.base_display(&m.built_for, &self.display);
        let (surface, revision) = match &p.base {
            Some((d, s, r)) if same_display(d, &base) => (s.as_ref(), *r),
            _ => (&m.surface, self.surface_revision),
        };
        let o = surface.origin;
        let planes = p
            .moving
            .iter()
            .filter_map(|&k| self.display.clips.get(k))
            .filter(|c| c.enabled)
            .map(|c| {
                let n = c.normal;
                let d: f64 = (0..3).map(|k| n[k] * (o[k] - c.origin[k])).sum();
                [n[0] as f32, n[1] as f32, n[2] as f32, d as f32]
            })
            .collect();
        Some((surface, revision, planes))
    }

    /// Frame the faces shown (or the whole mesh when everything is
    /// hidden).
    pub fn fit(&mut self) -> bool {
        let aspect = self.aspect();
        match self
            .mesh
            .as_ref()
            .and_then(|m| m.surface.visible_bounds.or(m.surface.bounds))
        {
            Some(b) => {
                self.camera.fit(&b, aspect);
                true
            }
            None => false,
        }
    }

    /// Frame the shown faces of `item`; false when it has none.
    pub fn fit_item(&mut self, item: Item) -> bool {
        let aspect = self.aspect();
        match self
            .mesh
            .as_ref()
            .and_then(|m| self.display.bounds(&m.mesh, Some(item)))
        {
            Some(b) => {
                self.camera.fit(&b, aspect);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(p: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
            .join(p)
    }

    /// Invariant: the channel matches the evidence KoflCraft wrote beside
    /// it (green badge), the box has none (amber), and a file whose bytes
    /// differ from the recorded SHA-256 fails (red), so a stale evidence
    /// file never shows green.
    #[test]
    fn validity_badge() {
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir = None;
        let m = s.open(&fixture("channel/foehncraft.vtu")).unwrap();
        assert_eq!(m.validity, Validity::Valid);
        let (mesh, ev) = (m.mesh.clone(), m.evidence.clone());
        let stale = Validity::check(
            &[("foehncraft.vtu".into(), "00".into())],
            &mesh,
            ev.as_ref(),
            None,
        );
        assert!(matches!(stale, Validity::Failed(w) if w.len() == 1));
        let m = s.open(&fixture("box/foehncraft.vtu")).unwrap();
        assert_eq!(m.validity, Validity::NoEvidence);
    }

    /// Invariant: with `defer_clips`, moving a clip plane leaves the
    /// surface as it was and starts a preview that the renderer cuts on
    /// the GPU; polling ends it with exactly the surface a direct build
    /// gives, and so does `settle`; any change other than a plane move
    /// builds at once and ends the preview. A preview that never ends, or
    /// ends with a surface built for an older position, would leave the
    /// window showing a cut the plane has left.
    #[test]
    fn clip_moves_are_previewed_then_cut_exactly() {
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir = None;
        s.open(&fixture("channel/foehncraft.vtu")).unwrap();
        let b = s.mesh.as_ref().unwrap().mesh.bounds();
        let at = |f: f64| Clip {
            origin: [b.min[0] + f * (b.max[0] - b.min[0]), 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            mode: crate::ClipMode::Crinkle,
            enabled: true,
        };
        let exact = |s: &Session| {
            let m = s.mesh.as_ref().unwrap();
            Surface::build_with(&m.mesh, Some(&m.fields), &s.tokens, &s.display)
        };
        s.defer_clips = true;
        // A new plane is not a move: it is cut at once.
        s.display.clips = vec![at(0.8)];
        s.redraw();
        assert!(s.preview.is_none());
        let before = s.mesh.as_ref().unwrap().surface.clone();
        assert_eq!(before, exact(&s));
        // A move previews: the surface stays, the renderer gets the plane.
        s.display.clips = vec![at(0.4)];
        s.redraw();
        assert!(s.preview.is_some());
        assert_eq!(s.mesh.as_ref().unwrap().surface, before);
        let (_, _, planes) = s.drawn().unwrap();
        assert_eq!(planes.len(), 1);
        assert_eq!(planes[0][..3], [1.0, 0.0, 0.0]);
        // Polling ends it with the exact cut.
        let start = Instant::now();
        while s.poll_surface() {
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "the preview never ended"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(s.mesh.as_ref().unwrap().surface, exact(&s));
        assert_eq!(s.mesh.as_ref().unwrap().built_for, s.display);
        assert!(s.drawn().unwrap().2.is_empty());
        // settle ends it at once, with the same cut.
        s.display.clips = vec![at(0.6)];
        s.redraw();
        s.poll_surface();
        s.display.clips = vec![at(0.3)];
        s.redraw();
        s.settle();
        assert!(s.preview.is_none());
        assert_eq!(s.mesh.as_ref().unwrap().surface, exact(&s));
        // Anything but a move builds at once and ends the preview.
        s.display.clips = vec![at(0.5)];
        s.redraw();
        assert!(s.preview.is_some());
        s.display.hidden.insert(Item::Zone(0));
        s.redraw();
        assert!(s.preview.is_none());
        assert_eq!(s.mesh.as_ref().unwrap().surface, exact(&s));
        // Colouring by quality keeps its values across moves (shared, not
        // copied), so a move still previews.
        s.display.color_by = ColorBy::Quality;
        s.redraw();
        assert!(s.preview.is_none());
        s.display.clips = vec![at(0.45)];
        s.redraw();
        assert!(s.preview.is_some());
        s.settle();
        assert_eq!(s.mesh.as_ref().unwrap().surface, exact(&s));
        // Without defer_clips (the command line, agents) a move cuts at once.
        s.defer_clips = false;
        s.display.clips = vec![at(0.2)];
        s.redraw();
        assert!(s.preview.is_none());
        assert_eq!(s.mesh.as_ref().unwrap().surface, exact(&s));
    }

    /// Invariant: with `hash_after_open` a mesh opens before its files are
    /// hashed: the badge is checking and the digest empty, and once the
    /// digest is taken it is the one a direct open gives, with the same
    /// badge. Counts the evidence contradicts fail at once, without the
    /// hash; another open or a close drops the old hashing, so its digest
    /// never lands on the next mesh. A digest put on the wrong mesh would
    /// show green for a file never checked; the window test only checks
    /// that the badge ends up set.
    /// Invariant: with `remember_checked`, a mesh file hashed once is
    /// not hashed again while it is unchanged, whether the open hashes it
    /// at once (the command line) or afterwards (the window), and the
    /// digest it remembered is still checked against the evidence; a
    /// changed file, or a session that does not remember, hashes again;
    /// the files of a polyMesh folder are remembered one by one. Breaks
    /// when an open skips the list (the iMac waits 17 s again for a 4 M
    /// cell mesh) or trusts it for a changed file. `checked`'s own tests
    /// see the list without a session.
    #[test]
    fn remembers_checked_files() {
        use crate::checked::{Stamp, lookup, remember};
        const FAKE: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        let dir = std::env::temp_dir().join(format!("murmel-remember-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Files written a while ago (`checked::SETTLED`).
        let age = |f: &Path| {
            let t = SystemTime::now() - Duration::from_secs(3600);
            let file = std::fs::File::options().write(true).open(f).unwrap();
            file.set_modified(t).unwrap();
        };
        for f in ["foehncraft.vtu", "mesh-evidence.json"] {
            std::fs::copy(fixture("channel").join(f), dir.join(f)).unwrap();
            age(&dir.join(f));
        }
        let mesh = dir.join("foehncraft.vtu");
        let list = dir.join("settings");
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir = Some(list.clone());
        s.remember_checked = true;

        let real = s.open(&mesh).unwrap().digest.clone();
        assert_eq!(real, sha256(&mesh));
        assert_eq!(lookup(&list, &mesh), Some(real.clone()));
        // A remembered digest is taken as it is, and checked: one the
        // evidence does not list fails.
        remember(&list, &mesh, &Stamp::of(&mesh).unwrap(), FAKE);
        let m = s.open(&mesh).unwrap();
        assert_eq!(m.digest, FAKE);
        assert!(matches!(m.validity, Validity::Failed(_)));
        // The same after the open, as the window hashes.
        s.hash_after_open = true;
        s.open(&mesh).unwrap();
        s.wait_digest();
        assert_eq!(s.mesh.as_ref().unwrap().digest, FAKE);

        // Changed: hashed again, and remembered anew.
        let mut text = std::fs::read(&mesh).unwrap();
        text.extend_from_slice(b"\n");
        std::fs::write(&mesh, &text).unwrap();
        age(&mesh);
        s.open(&mesh).unwrap();
        s.wait_digest();
        let changed = sha256(&mesh);
        assert_ne!(changed, real);
        assert_eq!(s.mesh.as_ref().unwrap().digest, changed);
        assert_eq!(lookup(&list, &mesh), Some(changed.clone()));

        // A session that does not remember hashes every time.
        remember(&list, &mesh, &Stamp::of(&mesh).unwrap(), FAKE);
        s.remember_checked = false;
        s.hash_after_open = false;
        assert_eq!(s.open(&mesh).unwrap().digest, changed);

        // The files of a polyMesh folder, hashed after the open.
        s.remember_checked = true;
        s.hash_after_open = true;
        let case = fixture("openfoam/box");
        s.open(&case).unwrap();
        s.wait_digest();
        let m = s.mesh.as_ref().unwrap();
        assert!(m.mesh.files.len() > 1);
        for f in &m.mesh.files {
            assert_eq!(lookup(&list, f), Some(sha256(f)), "{}", f.display());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hash_after_open() {
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir = None;
        let channel = fixture("channel/foehncraft.vtu");
        let direct = s.open(&channel).unwrap().clone();
        assert!(s.hashing.is_none());
        s.hash_after_open = true;
        let m = s.open(&channel).unwrap();
        assert_eq!(m.validity, Validity::Checking);
        assert!(m.digest.is_empty());
        assert!(m.files.iter().all(|(_, d)| d.is_empty()));
        assert!(s.hashing.is_some());
        s.wait_digest();
        let m = s.mesh.as_ref().unwrap();
        assert_eq!(m.digest, direct.digest);
        assert_eq!(m.files, direct.files);
        assert_eq!(m.validity, Validity::Valid);
        assert!(s.hashing.is_none());
        assert!(!s.poll_digest(), "nothing left to put in place");

        // Polled until done, as the window does.
        s.open(&channel).unwrap();
        while !s.poll_digest() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(s.mesh.as_ref().unwrap().validity, Validity::Valid);

        // A count the evidence contradicts fails before the hash.
        let mut wrong = direct.evidence.clone().unwrap();
        wrong.cells = Some(1);
        let early = Validity::before_hash(&direct.mesh, Some(&wrong), None);
        assert!(matches!(early, Validity::Failed(w) if w.len() == 1));
        assert_eq!(
            Validity::before_hash(&direct.mesh, None, None),
            Validity::NoEvidence
        );

        // The hashing of the channel ends with the next open.
        s.open(&channel).unwrap();
        let first = s.hashing.clone().unwrap();
        let m = s.open(&fixture("box/foehncraft.vtu")).unwrap();
        assert_eq!(m.validity, Validity::NoEvidence);
        assert!(first.stop.load(Ordering::Relaxed));
        s.wait_digest();
        let m = s.mesh.as_ref().unwrap();
        assert_eq!(m.digest, sha256(&fixture("box/foehncraft.vtu")));
        s.open(&channel).unwrap();
        let h = s.hashing.clone().unwrap();
        s.close();
        assert!(h.stop.load(Ordering::Relaxed) && s.hashing.is_none());
    }

    /// Invariant: KoflCraft's OpenFOAM export is checked file by file
    /// against the evidence in its polyMesh folder (green), one changed
    /// file turns it red, and a polyMesh without evidence is amber. A
    /// check of only one file, or of the folder path, would show green on
    /// an edited `owner`.
    #[test]
    fn validity_of_a_polymesh() {
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir = None;
        let m = s.open(&fixture("openfoam/box")).unwrap();
        assert_eq!(m.validity, Validity::Valid, "{:?}", m.validity);
        assert_eq!(m.files.len(), 6);
        assert_eq!(m.digest.len(), 64);
        let (mesh, ev) = (m.mesh.clone(), m.evidence.clone());
        let mut files = m.files.clone();
        files[2].1 = "00".into();
        let stale = Validity::check(&files, &mesh, ev.as_ref(), None);
        assert!(
            matches!(stale, Validity::Failed(w) if w == ["owner differs from the file the evidence describes"])
        );
        let m = s.open(&fixture("openfoam/channel_ascii")).unwrap();
        assert_eq!(m.validity, Validity::NoEvidence);
    }

    /// Invariant: a mesh loaded in advance opens with the surface built
    /// for it while the display and tokens are unchanged; a changed display
    /// builds the surface again, and a file changed since it was read, a
    /// removed evidence file or another path reads the mesh again. A
    /// reused stale surface would show the old colouring, or a mesh no
    /// longer on disk under a green badge; the window test only checks
    /// that a large mesh opens at all.
    #[test]
    fn preloaded_mesh() {
        let dir = std::env::temp_dir().join(format!("murmel-preload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["foehncraft.vtu", "mesh-evidence.json"] {
            std::fs::copy(fixture("channel").join(f), dir.join(f)).unwrap();
        }
        let path = dir.join("foehncraft.vtu");
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir = None;
        let direct = s.open(&path).unwrap().clone();
        // Marked, to tell the loaded mesh and its surface from a new read.
        let marked = |s: &Session| {
            let mut l = load(&path).unwrap();
            l.prepare(&s.display, &s.tokens);
            if let Some(p) = &mut l.prepared {
                p.surface.cells = 7;
            }
            l.mesh.metadata.insert("preloaded".into(), "yes".into());
            l
        };
        let preloaded = |m: &OpenMesh| m.mesh.metadata.contains_key("preloaded");

        s.preloaded = Some(marked(&s));
        let m = s.open(&path).unwrap();
        assert!(preloaded(m));
        assert_eq!(m.surface.cells, 7);
        assert_eq!(m.digest, direct.digest);
        assert_eq!(m.validity, Validity::Valid);
        assert!(s.preloaded.is_none());

        let l = marked(&s);
        s.display.color_by = ColorBy::Quality;
        s.preloaded = Some(l);
        let m = s.open(&path).unwrap();
        assert!(preloaded(m));
        assert_eq!(m.surface.cells, direct.surface.cells);
        assert!(s.display.quality.is_some());
        s.display.color_by = ColorBy::Solid;

        let l = marked(&s);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000))
            .unwrap();
        assert!(!l.is_fresh());
        s.preloaded = Some(l);
        let m = s.open(&path).unwrap();
        assert!(!preloaded(m));
        assert_eq!(m.surface.cells, direct.surface.cells);

        let l = marked(&s);
        assert!(l.is_fresh());
        std::fs::remove_file(dir.join("mesh-evidence.json")).unwrap();
        assert!(!l.is_fresh());

        s.preloaded = Some(marked(&s));
        let m = s.open(&fixture("box/foehncraft.vtu")).unwrap();
        assert!(!preloaded(m));
        assert!(s.preloaded.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
