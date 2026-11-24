//! The viewer session: the open mesh, its surface, the camera and the
//! settings. Front ends read it and change it only through tools.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use murmel_mesh::vtkhdf::Overview;
use murmel_mesh::{Evidence, FileHash, Mesh, MeshError};

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
    /// The hash of `digest` and `files`: the one the evidence beside the
    /// mesh recorded its files with (SHA-256 in KoflCraft's evidence 1.0),
    /// else BLAKE3.
    pub hash: FileHash,
    /// The digest of the mesh file, hex. A mesh of several files (an
    /// OpenFOAM polyMesh) gets the digest of its `files` listed as
    /// `b3sum` or `sha256sum` prints them (`<hex>  <name>` lines).
    pub digest: String,
    /// The name and digest (hex) of each file the mesh was read from.
    pub files: Vec<(String, String)>,
    /// The validity badge of the tree root.
    pub validity: Validity,
    /// The fields read with the mesh (a `.vtkhdf` result, the data
    /// arrays of a `.vtu`); empty for a bare mesh.
    pub fields: Arc<FieldSet>,
    /// The display `surface` was built for.
    pub built_for: Display,
    /// Opened as an overview (a blocked VTKHDF file beyond the memory
    /// budget): `mesh` is the boundary only, and this tells what the
    /// volume holds.
    pub overview: Option<Arc<Overview>>,
    /// The blocks of an overview read for its regions: `mesh` and
    /// `fields` are theirs while they show ([`crate::blocks`]).
    pub blocks: Option<crate::blocks::Blocks>,
}

impl OpenMesh {
    /// The volume cells of the mesh: those of the file when it opened as
    /// an overview.
    pub fn cells(&self) -> usize {
        cells_of(&self.mesh, self.overview.as_deref())
    }

    /// The mesh of the whole domain: the overview's boundary while blocks
    /// of it show for the regions, else `mesh`.
    pub fn domain(&self) -> &Mesh {
        self.blocks.as_ref().map_or(&self.mesh, |b| &b.overview.0)
    }
}

/// The volume cells of `mesh`, or of the file `overview` is of.
fn cells_of(mesh: &Mesh, overview: Option<&Overview>) -> usize {
    overview.map_or(mesh.cell_count(), |o| o.cells as usize)
}

/// How `mesh.open` is asked to open a mesh (its `mode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenMode {
    /// Whole within the memory budget, else as an overview.
    #[default]
    Fit,
    /// Whole, whatever it needs.
    Whole,
    /// As an overview (a blocked VTKHDF file).
    Overview,
}

/// How a mesh opens with respect to the memory budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Whole, whatever it needs.
    Whole,
    /// Whole when a whole read needs at most this many bytes; a blocked
    /// VTKHDF file that needs more opens as an overview.
    Within(u64),
    /// As an overview (a blocked VTKHDF file only).
    Overview,
}

/// The validity badge (UI spec, Model tree): green when the evidence
/// beside the mesh matches it, amber when there is none, red when a check
/// fails; grey while the digests of the files are still being taken after
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
        cells: usize,
        evidence: Option<&Evidence>,
        problem: Option<&MeshError>,
    ) -> Validity {
        if let Some(p) = problem {
            return Validity::Failed(vec![format!("{}: {}", p.code, p.message)]);
        }
        let Some(e) = evidence else {
            return Validity::NoEvidence;
        };
        let why = counts(mesh, cells, e);
        if why.is_empty() {
            Validity::Checking
        } else {
            Validity::Failed(why)
        }
    }

    /// Check `mesh`, read from `files` (name and digest of each, of the
    /// evidence's `file_hash`), against
    /// its evidence: every file the evidence lists must match, and the
    /// mesh file (the `.vtu`, or the polyMesh's `faces`) must be listed.
    /// `cells` is the volume cells of the file ([`OpenMesh::cells`]).
    pub fn check(
        files: &[(String, String)],
        mesh: &Mesh,
        cells: usize,
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
        why.extend(counts(mesh, cells, e));
        if why.is_empty() {
            Validity::Valid
        } else {
            Validity::Failed(why)
        }
    }
}

/// What the evidence's counts say against `mesh` of `cells` volume cells.
fn counts(mesh: &Mesh, cells: usize, e: &Evidence) -> Vec<String> {
    let mut why = Vec::new();
    if let Some(c) = e.cells
        && c as usize != cells
    {
        why.push(format!("{cells} cells, evidence says {c}"));
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

/// The digest of `hash` of a file, hex, read in pieces; empty when it
/// cannot be read.
pub fn file_digest(hash: FileHash, path: &Path) -> String {
    hash.of_file(path, &AtomicBool::new(false))
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// The digest of a mesh read from `files` (name and digest of `hash` of
/// each): the file's own, or for several files their listing as `b3sum`
/// or `sha256sum` prints it, hashed.
fn combined(files: &[(String, String)], hash: FileHash) -> String {
    match files {
        [(_, d)] => d.clone(),
        _ => hash.of_bytes(
            files
                .iter()
                .map(|(n, d)| format!("{d}  {n}\n"))
                .collect::<String>()
                .as_bytes(),
        ),
    }
}

/// The digests of `hash` of `files`, remembered in `checked` or taken;
/// empty for a file that cannot be read.
fn digests(files: &[PathBuf], hash: FileHash, checked: Option<&Path>) -> Vec<String> {
    files
        .iter()
        .map(|f| {
            crate::checked::digest(checked, f, hash, &AtomicBool::new(false))
                .ok()
                .flatten()
                .unwrap_or_default()
        })
        .collect()
}

/// The name of a file, as `files` lists it.
fn file_name(f: &Path) -> String {
    f.file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned())
}

/// The digests of an open mesh's files, taken on a thread of its own
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
    /// Hash `files` with `hash` on a thread of its own, remembering them
    /// in `checked` (the settings folder; see [`crate::checked`]).
    fn start(files: Vec<PathBuf>, checked: Option<PathBuf>, hash: FileHash) -> Hashing {
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new((Mutex::new(None), Condvar::new()));
        let (s, d) = (stop.clone(), done.clone());
        std::thread::spawn(move || {
            let mut digests = Vec::with_capacity(files.len());
            for f in &files {
                match crate::checked::digest(checked.as_deref(), f, hash, &s) {
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

/// Find the regions' cells on the open mesh when they changed: on the
/// blocks of an overview read for them, without their halo; an overview
/// whose blocks are not read yet has no cells for them, and shows whole
/// meanwhile.
fn find_regions(display: &mut Display, m: &OpenMesh) {
    match (&m.overview, &m.blocks) {
        (Some(_), None) => display.region_cells = None,
        (_, blocks) => crate::region::ensure_on(
            display,
            &m.mesh,
            blocks.as_ref().map(|b| &b.subset.halo[..]),
        ),
    }
}

/// Forget what names the cells of the mesh shown before `mesh` (another
/// set of an overview's blocks, or the overview): picked, hidden,
/// isolated and coloured cells and faces, the quality, the values and
/// streamlines found on them, the regions' cells; probes and measurements
/// are pointed at `mesh` ([`anchor_to`]).
fn forget_cells(d: &mut Display, mesh: &Mesh) {
    let names_cells = |i: &Item| matches!(i, Item::Cell(_) | Item::Face(_));
    d.selection.retain(|i| !names_cells(i));
    d.isolated.retain(|i| !names_cells(i));
    d.hidden.retain(|i| !names_cells(i));
    d.colors.retain(|i, _| !names_cells(i));
    d.quality = None;
    d.field_values = None;
    d.stream_traces = None;
    d.region_cells = None;
    anchor_to(d, mesh);
}

/// Point the probes and measurements of `display` at the cells of `mesh`
/// (another set of an overview's blocks): each probe's cell is found
/// again from its point, and the node, edge and face numbers, which were
/// the other cells', are dropped.
pub(crate) fn anchor_to(display: &mut Display, mesh: &Mesh) {
    let clear = |a: &mut crate::measure::Anchor| {
        a.node = None;
        a.edge = None;
        a.face = None;
    };
    for m in &mut display.measurements {
        for a in &mut m.points {
            clear(a);
            a.cell = None;
        }
    }
    for p in &mut display.probes {
        clear(&mut p.at);
        p.at.cell = crate::measure::locate(mesh, p.at.point);
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
    overview: Option<&Overview>,
    tokens: &Tokens,
) -> (Display, Surface) {
    crate::block_boxes::ensure(&mut base, overview);
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
    /// The hash of `files` and `digest` (see [`OpenMesh::hash`]).
    pub hash: FileHash,
    pub files: Vec<(String, String)>,
    pub digest: String,
    pub validity: Validity,
    /// The files are not hashed yet (`load_with` without `hash`).
    pub pending: bool,
    /// Read as an overview ([`OpenMesh::overview`]).
    pub overview: Option<Overview>,
    /// How it was asked to fit the memory budget.
    pub fit: Fit,
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
    load_checked(path, hash, None, Fit::Whole)
}

/// Read the mesh at `path` as `fit` asks: whole, or as an overview of a
/// blocked VTKHDF file whose whole read needs more than the budget (from
/// its structure: [`murmel_mesh::vtkhdf::Size::open_bytes`]).
fn read_fitting(
    path: &Path,
    hash: bool,
    fit: Fit,
) -> Result<(Mesh, murmel_mesh::Results, Option<Overview>), MeshError> {
    use murmel_mesh::vtkhdf;
    let over = path.is_file()
        && vtkhdf::is_vtkhdf(path)
        && match fit {
            Fit::Whole => false,
            Fit::Overview => true,
            Fit::Within(budget) => {
                let size = vtkhdf::size_of(path)?;
                size.blocked && size.open_bytes() > budget
            }
        };
    if over {
        let (mesh, results, overview) = vtkhdf::read_overview(path, hash)?;
        return Ok((mesh, results, Some(overview)));
    }
    if fit == Fit::Overview {
        return Err(MeshError::new(
            "BLOCK004",
            format!(
                "{} is not a blocked VTKHDF file, so it cannot open as an overview",
                path.display()
            ),
        ));
    }
    let (mesh, results) = murmel_mesh::read_results_with(path, hash)?;
    Ok((mesh, results, None))
}

/// As [`load_with`], with the files checked before remembered in
/// `checked` (the settings folder; see [`crate::checked`]): a mesh file
/// unchanged since it was hashed is read without hashing it again, and
/// the files hashed now are remembered. `fit` says how a mesh beyond the
/// memory budget opens.
pub fn load_checked(
    path: &Path,
    hash: bool,
    checked: Option<&Path>,
    fit: Fit,
) -> Result<Loaded, MeshError> {
    // The evidence beside a mesh of one file says before the read which
    // hash its files need: older evidence (1.0) recorded SHA-256, which
    // the readers do not take beside the parsing.
    let early = path
        .is_file()
        .then(|| Evidence::beside(path).ok().flatten())
        .flatten();
    let read_hash = hash
        && early
            .as_ref()
            .is_none_or(|e| e.file_hash == FileHash::Blake3);
    // A mesh of one file, checked before and unchanged since, is read
    // without hashing; the stamp tells later whether the digest the
    // reader takes may be remembered.
    let (known, before) = match checked {
        Some(dir) if read_hash && path.is_file() => (
            crate::checked::lookup(dir, path, FileHash::Blake3),
            crate::checked::Stamp::of(path),
        ),
        _ => (None, None),
    };
    let t = murmel_mesh::profile::span("open.read");
    let (mut mesh, results, overview) = read_fitting(path, read_hash && known.is_none(), fit)?;
    drop(t);
    let cells = cells_of(&mesh, overview.as_ref());
    let one_file = |mesh: &Mesh| match mesh.files.as_slice() {
        [f] => std::fs::canonicalize(f).ok() == std::fs::canonicalize(path).ok(),
        _ => false,
    };
    if let Some(d) = &known
        && one_file(&mesh)
    {
        mesh.file_blake3 = vec![d.clone()];
    }
    // The digest the reader took of a mesh of one file.
    if let (Some(dir), Some(b), None, [d]) = (checked, &before, &known, mesh.file_blake3.as_slice())
        && one_file(&mesh)
    {
        crate::checked::remember(dir, path, b, FileHash::Blake3, d);
    }
    // An overview's fields span the volume's range, from the block
    // ranges.
    let ranges = overview.iter().flat_map(crate::blocks::volume_ranges);
    let fields = FieldSet::new(results).with_volume_ranges(ranges);
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
    let file_hash = evidence.as_ref().map_or(FileHash::Blake3, |e| e.file_hash);
    // The readers take the BLAKE3 hash of the bytes they read; a file
    // they did not hash, or one the evidence needs another hash of, is
    // read again here. Without `hash`, the session hashes them later.
    let taken: Vec<String> = match (hash, file_hash) {
        (false, _) => vec![String::new(); mesh.files.len()],
        (true, FileHash::Blake3) => mesh
            .files
            .iter()
            .enumerate()
            .map(
                |(i, f)| match mesh.file_blake3.get(i).filter(|d| d.len() == 64) {
                    Some(d) => d.clone(),
                    None => digests(std::slice::from_ref(f), file_hash, checked).remove(0),
                },
            )
            .collect(),
        (true, h) => digests(&mesh.files, h, checked),
    };
    let files: Vec<(String, String)> = mesh.files.iter().map(|f| file_name(f)).zip(taken).collect();
    let (digest, validity) = if hash {
        let v = Validity::check(
            &files,
            &mesh,
            cells,
            evidence.as_ref(),
            evidence_problem.as_ref(),
        );
        (combined(&files, file_hash), v)
    } else {
        let v = Validity::before_hash(&mesh, cells, evidence.as_ref(), evidence_problem.as_ref());
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
        hash: file_hash,
        files,
        digest,
        validity,
        pending: !hash,
        overview,
        fit,
        stamps,
        prepared: None,
    })
}

impl Loaded {
    /// Build in advance the surface the mesh opens with in a session of
    /// `display` and `tokens` (used if they are unchanged when it opens).
    pub fn prepare(&mut self, display: &Display, tokens: &Tokens) {
        let base = display_for_new_mesh(display, &self.fields);
        let (display, surface) = prepare(
            base.clone(),
            &self.mesh,
            &self.fields,
            self.overview.as_ref(),
            tokens,
        );
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
    /// The probe history read last (`probe.history`), shared by copies
    /// of the session so the window's reading on a copy serves the
    /// export after it.
    pub probe_history: crate::history::Cache,
    /// The tables of a run charted (`charts.open`) and the chart the
    /// window's panel shows; kept when another mesh opens.
    pub charts: Option<crate::chart::Charts>,
    /// The memory budget set, in GiB (`settings.set_memory`); `None`:
    /// half the computer's memory ([`crate::memory`]).
    pub memory_budget: Option<f64>,
    /// Read the blocks an overview's regions need on a thread of their
    /// own (the window): the overview shows until they are read, and
    /// [`Session::poll_blocks`] puts them in place. Off, a change of the
    /// regions reads them before it returns, as the command line and
    /// agents expect.
    pub defer_blocks: bool,
    /// The blocks being read for the regions (`defer_blocks`).
    pub block_job: Option<crate::blocks::BlockJob>,
    /// The blocks the regions need that are not read, and why: the
    /// memory budget, the read failed, or (`None`) the person stopped it.
    /// Blocks that failed or were stopped are not read again until the
    /// regions need others.
    pub blocks_problem: Option<(Vec<usize>, Option<MeshError>)>,
    /// The preview grid of the open overview being read (`defer_blocks`),
    /// for its slices, iso-surfaces and arrows ([`crate::preview`]).
    pub preview_job: Option<crate::preview::PreviewJob>,
    /// Why the open overview has no preview grid: the read failed, or
    /// (`None`) the person stopped it while that many slices, iso-surfaces
    /// and arrows showed. It is not read again for this mesh, unless more
    /// of them show after a stop.
    pub preview_problem: Option<(usize, Option<MeshError>)>,
    /// The cells of the preview grid along its longest side
    /// ([`murmel_mesh::vtkhdf::PREVIEW_CELLS`]; tests take fewer).
    pub preview_cells: usize,
    /// Where what can be made again is kept (the preview grids,
    /// [`crate::preview_cache`]); `None` keeps nothing. The window, the
    /// command line and the MCP server set it to
    /// [`crate::settings::cache_dir`].
    pub cache_dir: Option<PathBuf>,
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
            probe_history: crate::history::Cache::default(),
            charts: None,
            memory_budget: None,
            defer_blocks: false,
            block_job: None,
            blocks_problem: None,
            preview_job: None,
            preview_problem: None,
            preview_cells: murmel_mesh::vtkhdf::PREVIEW_CELLS,
            cache_dir: None,
        }
    }

    /// The memory budget in bytes.
    pub fn budget_bytes(&self) -> u64 {
        crate::memory::budget_bytes(self.memory_budget)
    }

    /// How a mesh opens: whole within the memory budget.
    pub fn budget_fit(&self) -> Fit {
        Fit::Within(self.budget_bytes())
    }

    /// How a mesh opens when `mode` is asked for.
    pub fn fit_for(&self, mode: OpenMode) -> Fit {
        match mode {
            OpenMode::Fit => self.budget_fit(),
            OpenMode::Whole => Fit::Whole,
            OpenMode::Overview => Fit::Overview,
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
        m.digest = combined(&m.files, m.hash);
        m.validity = Validity::check(
            &m.files,
            &m.mesh,
            m.cells(),
            m.evidence.as_ref(),
            m.evidence_problem.as_ref(),
        );
        true
    }

    /// The digest of `hash` of the open mesh (see [`OpenMesh::digest`]):
    /// its own when it was taken with `hash`, else taken now, through the
    /// files checked before (a session saved with SHA-256, before 1.15).
    /// Waits for the digests being taken. Empty without a mesh.
    pub fn digest_as(&mut self, hash: FileHash) -> String {
        self.wait_digest();
        let checked = self.checked_dir().map(Path::to_path_buf);
        let Some(m) = &self.mesh else {
            return String::new();
        };
        if m.hash == hash {
            return m.digest.clone();
        }
        let files: Vec<(String, String)> = m
            .mesh
            .files
            .iter()
            .map(|f| file_name(f))
            .zip(digests(&m.mesh.files, hash, checked.as_deref()))
            .collect();
        combined(&files, hash)
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
        self.open_as(path, self.budget_fit())
    }

    /// [`Session::open`] as `fit` asks.
    pub fn open_as(&mut self, path: &Path, fit: Fit) -> Result<&OpenMesh, MeshError> {
        let loaded = match self.preloaded.take() {
            Some(l) if l.path == path && l.fit == fit && l.is_fresh() => l,
            _ => load_checked(path, !self.hash_after_open, self.checked_dir(), fit)?,
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
        let mut l = load_checked(path, !self.hash_after_open, None, self.budget_fit())?;
        // An overview's step is like its overview, not the blocks shown.
        let same = self.mesh.as_ref().is_some_and(|m| {
            let shown = m.blocks.as_ref().map_or(&m.mesh, |b| &b.overview.0);
            shown.cell_count() == l.mesh.cell_count()
                && shown.points.len() == l.mesh.points.len()
                && m.overview.is_some() == l.overview.is_some()
                && m.cells() == cells_of(&l.mesh, l.overview.as_ref())
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
        self.stop_blocks();
        if l.pending {
            let dir = self.checked_dir().map(Path::to_path_buf);
            self.hashing = Some(Hashing::start(l.mesh.files.clone(), dir, l.hash));
        }
        let mut base = self.display.clone();
        base.field_values = None;
        base.stream_traces = None;
        base.quality = None;
        base.region_cells = None;
        if old.blocks.is_some() {
            forget_cells(&mut base, &l.mesh);
        }
        keep_field(&mut base, &l.fields);
        let (display, surface) =
            prepare(base, &l.mesh, &l.fields, l.overview.as_ref(), &self.tokens);
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
            hash: l.hash,
            digest: l.digest,
            files: l.files,
            validity: l.validity,
            fields: Arc::new(l.fields),
            overview: l.overview.map(Arc::new),
            blocks: None,
        });
        // An overview's regions read the blocks of this step's file.
        if self.mesh.as_ref().is_some_and(|m| m.overview.is_some()) {
            self.redraw();
        }
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
        self.stop_blocks();
        if l.pending {
            let dir = self.checked_dir().map(Path::to_path_buf);
            self.hashing = Some(Hashing::start(l.mesh.files.clone(), dir, l.hash));
        }
        let base = display_for_new_mesh(&self.display, &l.fields);
        let (display, surface) = match l.prepared.take() {
            Some(p) if p.base == base && p.tokens == self.tokens => (p.display, p.surface),
            _ => prepare(base, &l.mesh, &l.fields, l.overview.as_ref(), &self.tokens),
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
            hash: l.hash,
            digest: l.digest,
            files: l.files,
            validity: l.validity,
            fields: Arc::new(l.fields),
            overview: l.overview.map(Arc::new),
            blocks: None,
        })
    }

    pub fn close(&mut self) {
        self.mesh = None;
        self.preview = None;
        self.stop_blocks();
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
        self.display.block_boxes = None;
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
        let swapped = self.sync_blocks() | self.sync_preview();
        let Some(m) = &mut self.mesh else {
            self.preview = None;
            self.surface_revision += 1;
            return;
        };
        crate::block_boxes::ensure(&mut self.display, m.overview.as_deref());
        find_regions(&mut self.display, m);
        ensure_quality(&mut self.display, &m.mesh);
        crate::field::ensure_field(&mut self.display, &m.mesh, &m.fields);
        crate::stream::ensure(&mut self.display, &m.mesh, &m.fields);
        if self.defer_clips
            && !swapped
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

    /// The blocks the enabled box, sphere and slab regions of the open
    /// overview touch; `None` when the open mesh is not an overview.
    pub fn blocks_wanted(&self) -> Option<Vec<usize>> {
        let o = self.mesh.as_ref()?.overview.as_deref()?;
        Some(crate::blocks::wanted(o, &self.display.regions))
    }

    /// Show the cells the regions of the open overview need: read the
    /// blocks they touch that are not read (with `defer_blocks`, on a
    /// thread of their own, put in place by a later call once read), or
    /// show the overview again once no region needs a block. True when
    /// the mesh shown changed.
    fn sync_blocks(&mut self) -> bool {
        let Some(m) = &self.mesh else {
            return false;
        };
        let Some(o) = m.overview.clone() else {
            return false;
        };
        let wanted = crate::blocks::wanted(&o, &self.display.regions);
        let loaded: &[usize] = m.blocks.as_ref().map_or(&[], |b| &b.subset.blocks);
        let running = self.block_job.take();
        // The blocks read hold every cell the regions take: a region made
        // smaller keeps them, none at all shows the overview again.
        if crate::blocks::within(&wanted, loaded) {
            if let Some(j) = running {
                j.stop();
            }
            self.blocks_problem = None;
            return wanted.is_empty() && m.blocks.is_some() && self.show_blocks(None);
        }
        let job = match running {
            // A read of blocks that hold these goes on.
            Some(j) if crate::blocks::within(&wanted, &j.blocks) => j,
            other => {
                if let Some(j) = other {
                    j.stop();
                }
                if let Err(e) = crate::blocks::fits(&o, &wanted, self.budget_bytes()) {
                    self.blocks_problem = Some((wanted, Some(e)));
                    return false;
                }
                // Not again the read that failed or was stopped (beyond
                // the budget, it was not tried).
                if self.blocks_problem.as_ref().is_some_and(|(b, e)| {
                    *b == wanted && e.as_ref().is_none_or(|e| e.code != "BLOCK006")
                }) {
                    return false;
                }
                self.blocks_problem = None;
                let cells = crate::blocks::cells_of(&o, &wanted);
                crate::blocks::BlockJob::start(
                    m.path.clone(),
                    wanted,
                    cells,
                    m.evidence.clone(),
                    crate::blocks::volume_ranges(&o),
                )
            }
        };
        match job.take(!self.defer_blocks) {
            None => {
                self.block_job = Some(job);
                false
            }
            Some(Ok(read)) => self.show_blocks(Some(read)),
            Some(Err(e)) => {
                let stopped = e.code == "MESH019";
                self.blocks_problem = Some((job.blocks, (!stopped).then_some(e)));
                false
            }
        }
    }

    /// Show the blocks `read`, or the overview again (`None`): the fields
    /// derived and calculated are made again on the cells shown, and what
    /// names cells of the mesh shown before (picked cells and faces, the
    /// cells of probes, the quality, the values found) is dropped or found
    /// again. True when the mesh shown changed.
    fn show_blocks(&mut self, read: Option<crate::blocks::Read>) -> bool {
        let Some(m) = &mut self.mesh else {
            return false;
        };
        let (mesh, mut fields, blocks) = match read {
            Some(r) => {
                let overview = match &m.blocks {
                    Some(b) => b.overview.clone(),
                    None => (m.mesh.clone(), m.fields.clone()),
                };
                let blocks = crate::blocks::Blocks {
                    subset: Arc::new(r.subset),
                    overview,
                };
                (Arc::new(r.mesh), r.fields, Some(blocks))
            }
            None => match m.blocks.take() {
                Some(b) => (b.overview.0, (*b.overview.1).clone(), None),
                None => return false,
            },
        };
        for made in m.fields.made() {
            if let Ok((set, _)) = fields.remake(&mesh, made.clone()) {
                fields = set;
            }
        }
        let d = &mut self.display;
        forget_cells(d, &mesh);
        keep_field(d, &fields);
        m.mesh = mesh;
        m.fields = Arc::new(fields);
        m.blocks = blocks;
        self.preview = None;
        true
    }

    /// Put the blocks read for the regions in place once they are read
    /// (`defer_blocks`) and draw again. True while they are read, or when
    /// they were put in place now (the window draws again).
    pub fn poll_blocks(&mut self) -> bool {
        match &self.block_job {
            Some(j) if j.is_done() => {
                self.redraw();
                true
            }
            Some(_) => true,
            None => false,
        }
    }

    /// Stop reading the blocks for the regions; the mesh shown stays, and
    /// those blocks are not read again until the regions need others.
    pub fn cancel_blocks(&mut self) {
        if let Some(j) = self.block_job.take() {
            j.stop();
            self.blocks_problem = Some((j.blocks, None));
        }
    }

    fn stop_blocks(&mut self) {
        if let Some(j) = self.block_job.take() {
            j.stop();
        }
        self.blocks_problem = None;
        if let Some(j) = self.preview_job.take() {
            j.stop();
        }
        self.preview_problem = None;
    }

    /// How many slices, iso-surfaces and arrows show.
    fn extracts_showing(&self) -> usize {
        let d = &self.display;
        d.slices.iter().filter(|s| s.enabled).count()
            + d.isos.iter().filter(|i| i.enabled).count()
            + usize::from(d.arrows_shown())
    }

    /// Read the preview grid of the open overview the first time a
    /// slice, an iso-surface or the arrows show on it (with
    /// `defer_blocks`, on a thread of its own, put in place by a later
    /// call once read). True when it was put in place now.
    fn sync_preview(&mut self) -> bool {
        let Some(m) = &self.mesh else {
            return false;
        };
        let Some(o) = m.overview.as_deref() else {
            return false;
        };
        // the overview's fields, shown or kept while blocks show
        let fields = match &m.blocks {
            Some(b) => b.overview.1.clone(),
            None => m.fields.clone(),
        };
        let showing = self.extracts_showing();
        if fields.preview().is_some() || (m.blocks.is_some() && self.preview_job.is_none()) {
            return false;
        }
        let job = match self.preview_job.take() {
            Some(j) => j,
            None => {
                if showing == 0 {
                    return false;
                }
                match &self.preview_problem {
                    Some((_, Some(_))) => return false,
                    Some((at, None)) if showing <= *at => return false,
                    _ => {}
                }
                self.preview_problem = None;
                crate::preview::PreviewJob::start(
                    m.path.clone(),
                    self.preview_cells,
                    crate::blocks::volume_ranges(o),
                    self.cache_dir.clone(),
                )
            }
        };
        match job.take(!self.defer_blocks) {
            None => {
                self.preview_job = Some(job);
                false
            }
            Some(Ok(p)) => {
                fields.set_preview(Arc::new(p));
                m.blocks.is_none()
            }
            Some(Err(e)) => {
                let stopped = e.code == "MESH019";
                self.preview_problem = Some((showing, (!stopped).then_some(e)));
                false
            }
        }
    }

    /// Put the preview grid of the open overview in place once it is read
    /// (`defer_blocks`) and draw again. True while it is read, or when it
    /// was put in place now (the window draws again).
    pub fn poll_preview(&mut self) -> bool {
        match &self.preview_job {
            Some(j) if j.is_done() => {
                self.redraw();
                true
            }
            Some(_) => true,
            None => false,
        }
    }

    /// Stop reading the preview grid; it is read again once more slices,
    /// iso-surfaces or arrows show.
    pub fn cancel_preview(&mut self) {
        if let Some(j) = self.preview_job.take() {
            j.stop();
            self.preview_problem = Some((self.extracts_showing(), None));
        }
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
            mesh.cell_count(),
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
        // The channel's evidence is format 1.0: its files are checked
        // with SHA-256.
        const SHA: FileHash = FileHash::Sha256;
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
        assert_eq!(real, file_digest(SHA, &mesh));
        assert_eq!(lookup(&list, &mesh, SHA), Some(real.clone()));
        // A remembered digest is taken as it is, and checked: one the
        // evidence does not list fails.
        remember(&list, &mesh, &Stamp::of(&mesh).unwrap(), SHA, FAKE);
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
        let changed = file_digest(SHA, &mesh);
        assert_ne!(changed, real);
        assert_eq!(s.mesh.as_ref().unwrap().digest, changed);
        assert_eq!(lookup(&list, &mesh, SHA), Some(changed.clone()));

        // A session that does not remember hashes every time.
        remember(&list, &mesh, &Stamp::of(&mesh).unwrap(), SHA, FAKE);
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
            assert_eq!(
                lookup(&list, f, SHA),
                Some(file_digest(SHA, f)),
                "{}",
                f.display()
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Invariant: a mesh with KoflCraft's evidence 1.1 is checked with
    /// BLAKE3 (green, on open and after it), its reader's digest is
    /// remembered as BLAKE3, and the same digests under evidence 1.0
    /// (SHA-256) fail. Breaks when the evidence's `file_hash` is ignored,
    /// in either direction: every new bundle red, or old evidence
    /// compared with BLAKE3. The fixtures' evidence is 1.0 only.
    #[test]
    fn blake3_evidence() {
        let dir = std::env::temp_dir().join(format!("murmel-b3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mesh = dir.join("foehncraft.vtu");
        std::fs::copy(fixture("channel/foehncraft.vtu"), &mesh).unwrap();
        let t = SystemTime::now() - Duration::from_secs(3600);
        let f = std::fs::File::options().write(true).open(&mesh).unwrap();
        f.set_modified(t).unwrap();
        let b3 = file_digest(FileHash::Blake3, &mesh);
        let text = std::fs::read_to_string(fixture("channel/mesh-evidence.json")).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        v["format_version"] = "1.1".into();
        v["file_hash"] = "blake3".into();
        for f in v["files"].as_array_mut().unwrap() {
            if f[0] == "foehncraft.vtu" {
                f[1] = b3.clone().into();
            }
        }
        let evidence = dir.join("mesh-evidence.json");
        std::fs::write(&evidence, v.to_string()).unwrap();

        let list = dir.join("settings");
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir = Some(list.clone());
        s.remember_checked = true;
        let m = s.open(&mesh).unwrap();
        assert_eq!(m.hash, FileHash::Blake3);
        assert_eq!(m.digest, b3);
        assert_eq!(m.validity, Validity::Valid, "{:?}", m.validity);
        assert_eq!(
            crate::checked::lookup(&list, &mesh, FileHash::Blake3),
            Some(b3.clone())
        );
        s.hash_after_open = true;
        s.remember_checked = false;
        s.open(&mesh).unwrap();
        s.wait_digest();
        let m = s.mesh.as_ref().unwrap();
        assert_eq!(
            (m.digest.as_str(), &m.validity),
            (b3.as_str(), &Validity::Valid)
        );
        assert_eq!(
            s.digest_as(FileHash::Sha256),
            file_digest(FileHash::Sha256, &mesh)
        );

        // The same digests named SHA-256 (evidence 1.0) do not match.
        v.as_object_mut().unwrap().remove("file_hash");
        std::fs::write(&evidence, v.to_string()).unwrap();
        s.hash_after_open = false;
        let m = s.open(&mesh).unwrap();
        assert_eq!(m.hash, FileHash::Sha256);
        assert!(matches!(m.validity, Validity::Failed(_)));
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
        let cells = direct.mesh.cell_count();
        let early = Validity::before_hash(&direct.mesh, cells, Some(&wrong), None);
        assert!(matches!(early, Validity::Failed(w) if w.len() == 1));
        assert_eq!(
            Validity::before_hash(&direct.mesh, cells, None, None),
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
        assert_eq!(m.hash, FileHash::Blake3);
        assert_eq!(
            m.digest,
            file_digest(FileHash::Blake3, &fixture("box/foehncraft.vtu"))
        );
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
        let stale = Validity::check(&files, &mesh, mesh.cell_count(), ev.as_ref(), None);
        assert!(
            matches!(stale, Validity::Failed(w) if w == ["owner differs from the file the evidence describes"])
        );
        let m = s.open(&fixture("openfoam/channel_ascii")).unwrap();
        assert_eq!(m.validity, Validity::NoEvidence);
    }

    /// Invariant: a mesh loaded in advance opens with the surface built
    /// for it while the display and tokens are unchanged; a changed display
    /// builds the surface again, and a file changed since it was read, a
    /// removed evidence file, another path or another fit (whole, or
    /// within the memory budget) reads the mesh again. A
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
            let mut l = load_checked(&path, true, None, s.budget_fit()).unwrap();
            l.prepare(&s.display, &s.tokens);
            if let Some(p) = &mut l.prepared {
                p.surface.cells = 7;
            }
            l.mesh.metadata.insert("preloaded".into(), "yes".into());
            l
        };
        let preloaded = |m: &OpenMesh| m.mesh.metadata.contains_key("preloaded");

        // Read for another fit (whole whatever it needs), it is read again.
        s.preloaded = Some(marked(&s));
        let m = s.open_as(&path, Fit::Whole).unwrap();
        assert!(!preloaded(m));

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

    /// Invariant: a blocked VTKHDF file whose whole read needs more than
    /// the memory budget opens as an overview (the boundary with the
    /// cell fields on it, the cells counted from the file, and the badge
    /// checking those against the evidence, each cell field coloured
    /// over the volume's range, the Properties counting the file's cells
    /// and blocks); within the budget, or asked
    /// whole, it opens whole; a file without a block index opens whole
    /// whatever the budget and fails as an overview (BLOCK004). Breaks
    /// when the budget is ignored, the badge counts the overview's own
    /// none of the cells (a red badge on a good mesh), the colour range
    /// is the boundary's alone, or a plain file
    /// is taken for blocked. The mesh tests read overviews but never
    /// decide when one is read.
    #[test]
    fn a_mesh_beyond_the_budget_opens_as_an_overview() {
        let dir = std::env::temp_dir().join(format!("murmel-overview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain = fixture("vtkhdf/channel_fields.vtkhdf");
        let (whole, results) = murmel_mesh::read_results(&plain).unwrap();
        let path = dir.join("blocked.vtkhdf");
        murmel_mesh::blocks::write_blocked(&path, &whole, &results, 100).unwrap();
        std::fs::copy(
            fixture("channel/mesh-evidence.json"),
            dir.join("mesh-evidence.json"),
        )
        .unwrap();
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir = None;
        s.hash_after_open = true;

        // A budget of a byte: the blocked file opens as an overview.
        s.memory_budget = Some(1e-9);
        assert_eq!(s.budget_fit(), Fit::Within(1));
        let m = s.open(&path).unwrap();
        let ov = m.overview.clone().unwrap();
        assert_eq!(m.mesh.cell_count(), 0);
        assert_eq!(m.cells(), whole.cell_count());
        assert_eq!(ov.cells, whole.cell_count() as u64);
        assert_eq!(m.mesh.boundary.len(), whole.boundary.len());
        assert_eq!(m.validity, Validity::Checking);
        for f in results
            .fields
            .iter()
            .filter(|f| f.location == murmel_mesh::Location::Cell)
        {
            let g = m.fields.results.get(&f.name).unwrap();
            assert_eq!(g.location, murmel_mesh::Location::Boundary, "{}", f.name);
            // Its colour range is the volume's, from the block ranges.
            let choice = crate::field::FieldChoice {
                name: f.name.clone(),
                component: Component::Magnitude,
            };
            let (lo, hi) = m.fields.values(&m.mesh, &choice).unwrap().range.unwrap();
            let (wlo, whi) = FieldSet::new(results.clone())
                .values(&whole, &choice)
                .unwrap()
                .range
                .unwrap();
            let near = |a: f64, b: f64| (a - b).abs() <= 1e-12 * (1.0 + b.abs());
            assert!(
                near(lo, wlo) && near(hi, whi),
                "{}: {lo} {hi} {wlo} {whi}",
                f.name
            );
            assert!(m.fields.volume_range(&f.name).is_some());
        }
        assert!(m.surface.bounds.is_some());
        // Properties count the file's cells, in its blocks.
        let m = s.mesh.as_ref().unwrap();
        let rows = crate::inspect::rows(m, &s.display, crate::display::Item::Mesh);
        let value = |k: &str| rows.iter().find(|r| r.key == k).map(|r| r.value.clone());
        assert_eq!(value("cells"), Some(serde_json::json!(whole.cell_count())));
        assert_eq!(value("blocks"), Some(serde_json::json!(ov.blocks.len())));

        // Asked whole, or within the budget, it reads the cells.
        let m = s.open_as(&path, s.fit_for(OpenMode::Whole)).unwrap();
        assert!(m.overview.is_none());
        assert_eq!(m.mesh.cell_count(), whole.cell_count());
        s.memory_budget = None;
        let m = s.open(&path).unwrap();
        assert!(m.overview.is_none());

        // A file without a block index opens whole, and never as an
        // overview.
        s.memory_budget = Some(1e-9);
        let m = s.open(&plain).unwrap();
        assert!(m.overview.is_none());
        assert_eq!(m.mesh.cell_count(), whole.cell_count());
        let e = s
            .open_as(&plain, s.fit_for(OpenMode::Overview))
            .unwrap_err();
        assert_eq!(e.code, "BLOCK004");
        let m = s.open_as(&path, s.fit_for(OpenMode::Overview)).unwrap();
        assert!(m.overview.is_some());
        s.close();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A blocked copy of the channel (776 cells in blocks of 100) in a
    /// folder of its own, with its evidence, the whole read of it, and a
    /// session open on it as an overview.
    fn overview_session(name: &str) -> (PathBuf, PathBuf, Mesh, Session) {
        let dir = std::env::temp_dir().join(format!("murmel-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (mesh, results) =
            murmel_mesh::read_results(&fixture("vtkhdf/channel_fields.vtkhdf")).unwrap();
        let path = dir.join("blocked.vtkhdf");
        murmel_mesh::blocks::write_blocked(&path, &mesh, &results, 100).unwrap();
        std::fs::copy(
            fixture("channel/mesh-evidence.json"),
            dir.join("mesh-evidence.json"),
        )
        .unwrap();
        let (whole, _) = murmel_mesh::read_results(&path).unwrap();
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir = None;
        s.open_as(&path, Fit::Overview).unwrap();
        (dir, path, whole, s)
    }

    /// A small box near a corner of the channel (x 0.2 to 0.6, y 0.05 to
    /// 0.3, z 0.05 to 0.15 of 4 by 2 by 1), which touches two of its
    /// eight blocks.
    fn corner_box(m: &Mesh) -> crate::Region {
        let b = m.bounds();
        let e: [f64; 3] = std::array::from_fn(|k| b.max[k] - b.min[k]);
        let at = [0.1, 0.0875, 0.1];
        let size = [0.1, 0.125, 0.1];
        crate::Region::new(crate::Shape::Box {
            center: std::array::from_fn(|k| b.min[k] + at[k] * e[k]),
            size: std::array::from_fn(|k| size[k] * e[k]),
        })
    }

    /// Invariant (the maintainer's card 1 of 2026-11-24, overview plus
    /// regions): a box drawn on an overview reads the blocks it touches,
    /// with their halo, and shows the cells, surface and probe values the
    /// whole mesh shows for it: the same cells (by their number in the
    /// file), as many faces drawn, no halo cell among them, the volume's
    /// cell count and colour range kept. A probe in the box finds its
    /// cell. Turned off, the overview shows again; beyond the memory
    /// budget the blocks are not read (BLOCK006), and within it they are.
    /// Breaks when a block the box touches is not read (cells or faces
    /// missing), when the box takes cells of another numbering, when the
    /// probe keeps the overview's no cell, when the overview is not given
    /// back, or when the budget is ignored or a refusal for it sticks. The
    /// mesh tests read blocks but never choose them; the region test of
    /// the halo covers what a halo cell would do.
    #[test]
    fn a_region_on_an_overview_reads_the_blocks_it_touches() {
        let (dir, _path, whole, mut s) = overview_session("region-blocks");
        let overview_faces = s.mesh.as_ref().unwrap().surface.faces;
        let region = corner_box(&whole);
        // A probe in the middle of the box, before its blocks are read.
        let middle = match &region.shape {
            crate::Shape::Box { center, .. } => *center,
            _ => unreachable!(),
        };
        s.display.probes.push(crate::measure::Probe {
            n: 1,
            name: None,
            at: crate::measure::Anchor::typed(middle),
            visible: true,
        });
        s.display.regions.push(region.clone());
        s.redraw();
        let m = s.mesh.as_ref().unwrap();
        let o = m.overview.clone().unwrap();
        let b = m.blocks.as_ref().expect("the box's blocks are read");
        let wanted = crate::blocks::wanted(&o, &s.display.regions);
        assert!(
            !wanted.is_empty() && wanted.len() < o.blocks.len(),
            "{wanted:?}"
        );
        assert_eq!(b.subset.blocks, wanted);
        assert!(b.halo_cells() > 0);
        assert_eq!(m.cells(), whole.cell_count());

        // The same cells as on the whole mesh, by their number in the file.
        let rc = s.display.region_cells.clone().unwrap();
        let shown: std::collections::BTreeSet<u64> = (0..m.mesh.cell_count())
            .filter(|&c| rc.contains(c))
            .map(|c| b.subset.file_cells[c])
            .collect();
        let on_whole = crate::RegionCells::of(&whole, std::slice::from_ref(&region)).unwrap();
        let expected: std::collections::BTreeSet<u64> = (0..whole.cell_count())
            .filter(|&c| on_whole.contains(c))
            .map(|c| c as u64)
            .collect();
        assert!(!expected.is_empty() && expected.len() < whole.cell_count());
        assert_eq!(shown, expected);
        assert!(
            (0..m.mesh.cell_count()).all(|c| !(b.subset.halo[c] && rc.contains(c))),
            "a halo cell shows"
        );
        // As many cells and faces drawn as the whole mesh draws for it.
        let mut d = s.display.clone();
        d.region_cells = Some(Arc::new(on_whole));
        let full = FieldSet::new(murmel_mesh::read_results(&_path).unwrap().1);
        let w = Surface::build_with(&whole, Some(&full), &s.tokens, &d);
        assert_eq!((m.surface.cells, m.surface.faces), (w.cells, w.faces));
        // The probe found its cell, the one the whole mesh has there.
        let at = s.display.probes[0].at.cell.expect("the probe's cell");
        assert_eq!(
            Some(b.subset.file_cells[at as usize] as u32),
            crate::measure::locate(&whole, middle)
        );
        // The colour range stays the volume's.
        for f in &o.fields {
            assert_eq!(
                m.fields.volume_range(&f.name),
                o.range(&f.name).map(|[a, b]| (a, b)),
                "{}",
                f.name
            );
        }

        // Turned off: the overview again.
        s.display.regions[0].enabled = false;
        s.redraw();
        let m = s.mesh.as_ref().unwrap();
        assert!(m.blocks.is_none());
        assert_eq!(m.mesh.cell_count(), 0);
        assert_eq!(m.surface.faces, overview_faces);
        assert_eq!(s.display.probes[0].at.cell, None);

        // Beyond the budget: not read, and why.
        s.memory_budget = Some(1e-9);
        s.display.regions[0].enabled = true;
        s.redraw();
        let m = s.mesh.as_ref().unwrap();
        assert!(m.blocks.is_none());
        let (blocks, why) = s.blocks_problem.clone().unwrap();
        assert_eq!(blocks, wanted);
        assert_eq!(why.unwrap().code, "BLOCK006");
        // A budget large enough reads them.
        s.memory_budget = None;
        s.redraw();
        assert!(s.mesh.as_ref().unwrap().blocks.is_some());
        assert!(s.blocks_problem.is_none());
        s.close();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Invariant: with `defer_blocks` (the window), a region on an
    /// overview reads its blocks on a thread of its own: the overview
    /// shows whole meanwhile (its regions have no cells yet), and
    /// `poll_blocks` shows the blocks once read; a read stopped is not
    /// started again until the regions need other blocks. Breaks when
    /// the read blocks the call, when the overview goes blank while it
    /// runs, or when a stopped read starts again on the next redraw. The
    /// test above reads at once.
    #[test]
    fn the_window_reads_the_blocks_in_the_background() {
        let (dir, _path, whole, mut s) = overview_session("region-blocks-bg");
        s.defer_blocks = true;
        let overview_faces = s.mesh.as_ref().unwrap().surface.faces;
        s.display.regions.push(corner_box(&whole));
        s.redraw();
        let m = s.mesh.as_ref().unwrap();
        assert!(m.blocks.is_none());
        assert!(s.display.region_cells.is_none());
        assert_eq!(m.surface.faces, overview_faces);
        let job = s.block_job.clone().expect("a read in the background");
        let until = Instant::now() + Duration::from_secs(30);
        while !job.is_done() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(s.poll_blocks());
        assert!(s.block_job.is_none());
        let m = s.mesh.as_ref().unwrap();
        assert_eq!(m.blocks.as_ref().unwrap().subset.blocks, job.blocks);
        assert!(s.display.region_cells.as_ref().unwrap().filtering());
        assert!(!s.poll_blocks());

        // Another box elsewhere: a new read, which is stopped.
        let b = whole.bounds();
        s.display.regions[0] = crate::Region::new(crate::Shape::Box {
            center: b.min,
            size: [1e-3; 3],
        });
        s.redraw();
        let wanted = s.blocks_wanted().unwrap();
        if crate::blocks::within(&wanted, &job.blocks) {
            // The corner is in the blocks read: nothing to read.
            assert!(s.block_job.is_none());
        } else {
            assert!(s.block_job.is_some());
            s.cancel_blocks();
            assert_eq!(s.blocks_problem, Some((wanted, None)));
            s.redraw();
            assert!(s.block_job.is_none(), "a stopped read starts again");
        }
        s.close();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The area of the triangles of `s`.
    fn area_of(s: &Surface) -> f64 {
        s.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] =
                    t.map(|i| glam::DVec3::from(s.positions[i as usize].map(f64::from)));
                0.5 * (b - a).cross(c - a).length()
            })
            .sum()
    }

    fn slice_by_pressure(s: &mut Session, at: [f64; 3]) {
        s.display.color_by = ColorBy::Field;
        s.display.field = Some(crate::field::FieldChoice {
            name: "pressure".into(),
            component: Component::Magnitude,
        });
        s.display
            .slices
            .push(crate::Slice::new(at, [1.0, 0.0, 0.0]));
        s.redraw();
    }

    /// Invariant (the maintainer's card 7 of 2026-11-24: the overview is
    /// the surface, the block ranges and a preview grid): a slice on an
    /// overview draws on its preview grid, read the first time a slice
    /// shows (not for colouring alone), and covers the section the whole
    /// mesh's slice covers, in the field's colours; its triangles name no
    /// cell. Once a region's blocks show, the slice draws on their cells,
    /// and the preview stays with the overview's fields, given back as it
    /// was when the region goes. Breaks when the slice draws on the
    /// boundary (nothing), when its triangles name cells of the grid,
    /// when the grid is read before anything cuts the volume or again
    /// after the blocks, or when it is dropped with them. The mesh test
    /// reads the grid but nothing draws on it there.
    #[test]
    fn slices_on_an_overview_draw_on_its_preview_grid() {
        let (dir, path, whole, mut s) = overview_session("preview-slices");
        s.preview_cells = 16;
        let mid = whole.bounds().center();
        s.display.color_by = ColorBy::Field;
        s.redraw();
        assert!(s.mesh.as_ref().unwrap().fields.preview().is_none());
        slice_by_pressure(&mut s, mid);
        let m = s.mesh.as_ref().unwrap();
        let p = m.fields.preview().expect("read for the slice").clone();
        assert_eq!(p.grid.shape, [16, 8, 4]);
        assert!(m.surface.triangles() > 0);
        assert!(
            m.surface
                .tri_cell
                .iter()
                .all(|&c| c == murmel_mesh::NO_CELL)
        );
        assert!(
            m.surface.values.iter().all(|v| v[3] == 1.0),
            "coloured by the field"
        );
        // the section the whole mesh's slice covers
        let mut w = Session::new(Tokens::builtin().unwrap());
        w.config_dir = None;
        w.open_as(&path, Fit::Whole).unwrap();
        slice_by_pressure(&mut w, mid);
        let (a, b) = (
            area_of(&m.surface),
            area_of(&w.mesh.as_ref().unwrap().surface),
        );
        assert!((a - b).abs() < 0.02 * b, "preview {a}, whole {b}");

        // A region's blocks: the slice draws on their cells.
        s.display.regions.push(corner_box(&whole));
        s.redraw();
        let m = s.mesh.as_ref().unwrap();
        let blocks = m.blocks.as_ref().expect("the blocks read");
        assert!(Arc::ptr_eq(blocks.overview.1.preview().unwrap(), &p));
        assert!(m.fields.preview().is_none());
        assert!(
            m.surface
                .tri_cell
                .iter()
                .all(|&c| (c as usize) < m.mesh.cell_count())
        );
        // and the overview comes back with its grid
        s.display.regions.clear();
        s.redraw();
        let m = s.mesh.as_ref().unwrap();
        assert!(m.blocks.is_none());
        assert!(Arc::ptr_eq(m.fields.preview().unwrap(), &p));
        assert!((area_of(&m.surface) - a).abs() < 1e-9 * a);
        s.close();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Invariant: the eye of `blocks` on an overview draws the twelve
    /// edges of each block's box in the lines that hide behind faces (the
    /// mesh then draws as its faint outline), the blocks a region reads
    /// in their own colour and last, the others in the item's colour; the
    /// boxes stay while the region's blocks show, nothing is drawn on a
    /// mesh read whole (the setting kept for the next overview), and they
    /// go with the mesh. Breaks when the boxes draw while off, take the
    /// mesh's faces from it in no way, miss a corner, colour the wrong
    /// blocks as read or draw them under the others, or outlive the
    /// overview. No other test draws the boxes.
    #[test]
    fn an_overview_draws_the_boxes_of_its_blocks() {
        let (dir, path, whole, mut s) = overview_session("block-boxes");
        let t = Tokens::builtin().unwrap();
        let o = s.mesh.as_ref().unwrap().overview.clone().unwrap();
        let n = o.blocks.iter().filter(|b| !b.bounds.is_empty()).count();
        assert!(n > 2);
        // off: listed, not drawn
        s.redraw();
        let m = s.mesh.as_ref().unwrap();
        assert_eq!(s.display.block_boxes.as_ref().unwrap().count(), n);
        assert!(s.display.has(Item::Blocks));
        assert_eq!(m.surface.block_boxes, 0);
        assert!(m.surface.stream_positions.is_empty());
        assert!(m.surface.triangles() > 0);
        // on: every box, the mesh a faint outline
        let blocks = [Item::Blocks];
        s.display
            .set_visibility(crate::display::Visibility::Show, &blocks, &whole);
        s.redraw();
        let m = s.mesh.as_ref().unwrap();
        let sf = &m.surface;
        assert_eq!(sf.block_boxes, n);
        assert_eq!(sf.stream_positions.len(), 24 * n);
        assert_eq!(sf.stream_colors.len(), 24 * n);
        assert_eq!(sf.triangles(), 0, "the mesh draws as its outline");
        let plain = t.mesh.field.block.linear();
        assert!(sf.stream_colors.iter().all(|&c| c == plain));
        // each box's edges run along one axis between its corners
        let corners = |k: usize| -> Vec<[f64; 3]> {
            let b = o.blocks[k].bounds;
            (0..8)
                .map(|i| {
                    std::array::from_fn(|a| {
                        if i & (1 << a) == 0 {
                            b.min[a]
                        } else {
                            b.max[a]
                        }
                    })
                })
                .collect()
        };
        let back =
            |p: [f32; 3]| -> [f64; 3] { std::array::from_fn(|a| p[a] as f64 + sf.origin[a]) };
        let drawn: Vec<usize> = (0..o.blocks.len())
            .filter(|&k| !o.blocks[k].bounds.is_empty())
            .collect();
        for (j, &k) in drawn.iter().enumerate() {
            let c = corners(k);
            let mut seen = [false; 8];
            for e in sf.stream_positions[24 * j..24 * (j + 1)].chunks(2) {
                let (a, b) = (back(e[0]), back(e[1]));
                let differ = (0..3).filter(|&x| (a[x] - b[x]).abs() > 1e-6).count();
                assert_eq!(differ, 1, "an edge of box {k}");
                for p in [a, b] {
                    let i = c
                        .iter()
                        .position(|q| (0..3).all(|x| (p[x] - q[x]).abs() < 1e-5))
                        .expect("a corner of the box");
                    seen[i] = true;
                }
            }
            assert!(seen.iter().all(|&x| x), "all corners of box {k}");
        }
        // a region: its blocks take their own colour, drawn last
        s.display.regions.push(corner_box(&whole));
        s.redraw();
        let read = crate::blocks::wanted(&o, &s.display.regions);
        assert!(!read.is_empty() && read.len() < n);
        assert_eq!(s.display.block_boxes.as_ref().unwrap().read, read);
        let m = s.mesh.as_ref().unwrap();
        assert!(m.blocks.is_some(), "the region's blocks show");
        let sf = &m.surface;
        assert_eq!(sf.block_boxes, n);
        let split = 24 * (n - read.len());
        let hi = t.mesh.field.block_read.linear();
        assert!(sf.stream_colors[..split].iter().all(|&c| c == plain));
        assert!(sf.stream_colors[split..].iter().all(|&c| c == hi));
        // the item's colour: the others take it
        let violet = crate::theme::Rgb([0x70, 0x40, 0xC0]);
        s.display.colors.insert(Item::Blocks, violet);
        s.redraw();
        let sf = &s.mesh.as_ref().unwrap().surface;
        assert!(
            sf.stream_colors[..split]
                .iter()
                .all(|&c| c == violet.linear())
        );
        assert!(sf.stream_colors[split..].iter().all(|&c| c == hi));
        // a mesh read whole has none; the setting stays
        s.display.regions.clear();
        s.open_as(&path, Fit::Whole).unwrap();
        s.redraw();
        assert!(s.display.show_blocks);
        assert!(s.display.block_boxes.is_none());
        assert!(!s.display.has(Item::Blocks));
        assert_eq!(s.mesh.as_ref().unwrap().surface.block_boxes, 0);
        // the next overview shows them at once
        s.open_as(&path, Fit::Overview).unwrap();
        assert_eq!(s.mesh.as_ref().unwrap().surface.block_boxes, n);
        s.close();
        assert!(s.display.block_boxes.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Invariant: with a cache folder, the preview grid sampled for an
    /// overview is kept, and the same file opened again unchanged takes
    /// it from there (here a value altered in the kept file shows, so it
    /// was not sampled again); without one nothing is kept. Breaks when
    /// the session does not hand its cache folder to the read, or the
    /// read samples again despite a kept grid. The cache's own tests do
    /// not go through a session.
    #[test]
    fn an_overview_opened_again_takes_its_kept_preview() {
        let (dir, path, whole, mut s) = overview_session("preview-keep");
        let old = SystemTime::now() - std::time::Duration::from_secs(60);
        std::fs::File::options()
            .append(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let cache = dir.join("cache");
        s.cache_dir = Some(cache.clone());
        s.preview_cells = 16;
        let mid = whole.bounds().center();
        slice_by_pressure(&mut s, mid);
        let first = s.mesh.as_ref().unwrap().fields.preview().unwrap().clone();
        let kept: Vec<PathBuf> = std::fs::read_dir(cache.join(crate::preview_cache::DIR))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        assert_eq!(kept.len(), 1, "{kept:?}");
        // the last value of the last field, altered in the kept file
        let mut bytes = std::fs::read(&kept[0]).unwrap();
        let n = bytes.len();
        bytes[n - 8..].copy_from_slice(&12345.0f64.to_le_bytes());
        std::fs::write(&kept[0], bytes).unwrap();

        let mut t = Session::new(Tokens::builtin().unwrap());
        t.config_dir = None;
        t.cache_dir = Some(cache.clone());
        t.preview_cells = 16;
        t.open_as(&path, Fit::Overview).unwrap();
        slice_by_pressure(&mut t, mid);
        let again = t.mesh.as_ref().unwrap().fields.preview().unwrap().clone();
        assert_eq!(again.grid, first.grid);
        let last = |p: &crate::preview::Preview| {
            p.fields
                .fields()
                .last()
                .and_then(|f| f.values.last().copied())
        };
        assert_eq!(last(&again), Some(12345.0), "taken from the cache");
        assert_ne!(last(&first), Some(12345.0));
        assert!(t.mesh.as_ref().unwrap().surface.triangles() > 0);
        // no cache folder: sampled, nothing kept
        let mut u = Session::new(Tokens::builtin().unwrap());
        u.config_dir = None;
        u.preview_cells = 16;
        u.open_as(&path, Fit::Overview).unwrap();
        slice_by_pressure(&mut u, mid);
        let p = u.mesh.as_ref().unwrap().fields.preview().unwrap().clone();
        assert_eq!(last(&p), last(&first));
        s.close();
        t.close();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Invariant: with `defer_blocks` (the window), the preview grid is
    /// read on a thread of its own while the overview shows, and put in
    /// place by `poll_preview`; Cancel stops it until one more slice,
    /// iso-surface or the arrows show, and opening the mesh again
    /// forgets the stop. Breaks when the window waits for the grid, when
    /// a stopped read starts again at the next redraw, or when it never
    /// does. The test above reads the grid at once.
    #[test]
    fn the_window_reads_the_preview_in_the_background() {
        let (dir, path, whole, mut s) = overview_session("preview-bg");
        s.preview_cells = 16;
        s.defer_blocks = true;
        let mid = whole.bounds().center();
        slice_by_pressure(&mut s, mid);
        assert!(s.mesh.as_ref().unwrap().fields.preview().is_none());
        assert_eq!(s.mesh.as_ref().unwrap().surface.triangles(), 0);
        let job = s.preview_job.clone().expect("a read in the background");
        let until = Instant::now() + Duration::from_secs(30);
        while !job.is_done() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(s.poll_preview());
        assert!(s.preview_job.is_none());
        assert!(s.mesh.as_ref().unwrap().fields.preview().is_some());
        assert!(s.mesh.as_ref().unwrap().surface.triangles() > 0);
        assert!(!s.poll_preview());

        // Opened again (which drops the slices), sliced and stopped: not
        // read again until another slice.
        s.open_as(&path, Fit::Overview).unwrap();
        assert!(s.display.slices.is_empty());
        assert!(s.preview_job.is_none());
        slice_by_pressure(&mut s, mid);
        assert!(s.preview_job.is_some());
        s.cancel_preview();
        assert_eq!(s.preview_problem.as_ref().map(|p| p.0), Some(1));
        s.redraw();
        assert!(s.preview_job.is_none(), "a stopped read stays stopped");
        s.display
            .slices
            .push(crate::Slice::new(mid, [0.0, 1.0, 0.0]));
        s.redraw();
        assert!(s.preview_job.is_some(), "one more slice reads it");
        s.cancel_preview();
        s.open_as(&path, Fit::Overview).unwrap();
        assert!(s.preview_problem.is_none(), "a new open forgets the stop");
        slice_by_pressure(&mut s, mid);
        assert!(s.preview_job.is_some());
        s.close();
        assert!(s.preview_job.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
