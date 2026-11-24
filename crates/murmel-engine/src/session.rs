//! The viewer session: the open mesh, its surface, the camera and the
//! settings. Front ends read it and change it only through tools.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use murmel_mesh::{Evidence, Mesh, MeshError};

use crate::camera::Camera;
use crate::cube::CubeSettings;
use crate::display::{ColorBy, Display, Item};
use crate::field::FieldSet;
use crate::journal::Journal;
use crate::navigation::Navigation;
use crate::quality::Quality;
use crate::surface::Surface;
use crate::theme::{MeshColors, Theme, Tokens};

/// A mesh loaded into the session.
#[derive(Clone, Debug)]
pub struct OpenMesh {
    pub path: PathBuf,
    pub mesh: Mesh,
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
    pub fields: FieldSet,
}

/// The validity badge (UI spec, Model tree): green when the evidence
/// beside the mesh matches it, amber when there is none, red when a check
/// fails.
#[derive(Clone, Debug, PartialEq)]
pub enum Validity {
    Valid,
    NoEvidence,
    Failed(Vec<String>),
}

impl Validity {
    pub fn key(&self) -> &'static str {
        match self {
            Validity::Valid => "valid",
            Validity::NoEvidence => "no_evidence",
            Validity::Failed(_) => "failed",
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
        if why.is_empty() {
            Validity::Valid
        } else {
            Validity::Failed(why)
        }
    }
}

/// SHA-256 of a file, hex; empty when it cannot be read.
pub fn sha256(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => sha256_bytes(&bytes),
        Err(_) => String::new(),
    }
}

/// SHA-256 of bytes, hex.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    murmel_mesh::sha256_hex(bytes)
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
    d.field_values = None;
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
    d
}

/// The display (with the quality or field values it colours by) and the
/// surface of a new mesh, from its display `base`.
fn prepare(
    mut base: Display,
    mesh: &Mesh,
    fields: &FieldSet,
    tokens: &Tokens,
) -> (Display, Surface) {
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
    let t = murmel_mesh::profile::span("open.read");
    let (mut mesh, results) = murmel_mesh::read_results(path)?;
    drop(t);
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
    // read again here.
    let files: Vec<(String, String)> = mesh
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let name = f
                .file_name()
                .map_or(String::new(), |n| n.to_string_lossy().into_owned());
            let digest = mesh
                .file_sha256
                .get(i)
                .filter(|d| d.len() == 64)
                .cloned()
                .unwrap_or_else(|| sha256(f));
            (name, digest)
        })
        .collect();
    let digest = match files.as_slice() {
        [(_, d)] => d.clone(),
        _ => sha256_bytes(
            files
                .iter()
                .map(|(n, d)| format!("{d}  {n}\n"))
                .collect::<String>()
                .as_bytes(),
        ),
    };
    let validity = Validity::check(&files, &mesh, evidence.as_ref(), evidence_problem.as_ref());
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
    /// The undo and redo steps of the open mesh.
    pub journal: Journal,
    /// A mesh read in advance (the window reads a large one on a thread
    /// of its own), which the next `mesh.open` of its path takes.
    pub preloaded: Option<Loaded>,
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
            journal: Journal::default(),
            preloaded: None,
        }
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
            _ => load(path)?,
        };
        Ok(self.install(loaded))
    }

    /// Put a loaded mesh in place of the open one and fit the view. The
    /// surface built in advance (`Loaded::prepare`) is used when the
    /// display and the tokens are still the ones it was built for.
    fn install(&mut self, mut l: Loaded) -> &OpenMesh {
        self.journal.clear();
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
        self.mesh.insert(OpenMesh {
            path: l.path,
            mesh: l.mesh,
            evidence: l.evidence,
            evidence_problem: l.evidence_problem,
            surface,
            digest: l.digest,
            files: l.files,
            validity: l.validity,
            fields: l.fields,
        })
    }

    pub fn close(&mut self) {
        self.mesh = None;
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
        self.display.field_values = None;
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

    /// Build the surface again after a change of `display`.
    pub fn redraw(&mut self) {
        if let Some(m) = &mut self.mesh {
            ensure_quality(&mut self.display, &m.mesh);
            crate::field::ensure_field(&mut self.display, &m.mesh, &m.fields);
            m.surface = Surface::build_with(&m.mesh, Some(&m.fields), &self.tokens, &self.display);
        }
        self.surface_revision += 1;
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
