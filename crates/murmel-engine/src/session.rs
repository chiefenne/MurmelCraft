//! The viewer session: the open mesh, its surface, the camera and the
//! settings. Front ends read it and change it only through tools.

use std::path::{Path, PathBuf};

use murmel_mesh::{Evidence, Mesh, MeshError};

use crate::camera::Camera;
use crate::cube::CubeSettings;
use crate::display::{ColorBy, Display, Item};
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
    pub fields: crate::field::FieldSet,
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
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Compute the cell quality of `mesh` the first time colour mode quality
/// needs it; it is kept until another mesh opens.
fn ensure_quality(display: &mut Display, mesh: &Mesh) {
    if display.color_by == ColorBy::Quality && display.quality.is_none() {
        display.quality = Some(std::sync::Arc::new(Quality::compute(mesh)));
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
        }
    }

    pub fn aspect(&self) -> f64 {
        self.viewport[0].max(1.0) / self.viewport[1].max(1.0)
    }

    /// Read a mesh and its evidence, replace the open one and fit the view.
    pub fn open(&mut self, path: &Path) -> Result<&OpenMesh, MeshError> {
        let (mut mesh, results) = murmel_mesh::read_results(path)?;
        let fields = crate::field::FieldSet::new(results);
        // The evidence sits beside the mesh file: beside the .vtu, or in
        // the polyMesh folder among its files.
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
        // A new mesh starts with everything shown, nothing selected and
        // no saved views, measurements or probes (they are in its
        // coordinates); the colouring stays. Its journal starts empty.
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
        // The field colouring stays when the new mesh has the field (the
        // next time step of the same case); else it falls back to solid.
        self.display.field_values = None;
        if self.display.field.as_ref().is_some_and(|c| fields.get(&c.name).is_none()) {
            self.display.field = None;
            if self.display.color_by == ColorBy::Field {
                self.display.color_by = ColorBy::Solid;
            }
        }
        if self.display.color_by == ColorBy::Field && self.display.field.is_none() {
            self.display.color_by = ColorBy::Solid;
        }
        ensure_quality(&mut self.display, &mesh);
        crate::field::ensure_field(&mut self.display, &mesh, &fields);
        let surface = Surface::build(&mesh, &self.tokens, &self.display);
        let files: Vec<(String, String)> = mesh
            .files
            .iter()
            .map(|f| {
                let name = f
                    .file_name()
                    .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                (name, sha256(f))
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
        if let Some(b) = &surface.bounds {
            self.camera.fit(b, self.aspect());
        }
        self.surface_revision += 1;
        Ok(self.mesh.insert(OpenMesh {
            path: path.to_path_buf(),
            mesh,
            evidence,
            evidence_problem,
            surface,
            digest,
            files,
            validity,
            fields,
        }))
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
            m.surface = Surface::build(&m.mesh, &self.tokens, &self.display);
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
}
