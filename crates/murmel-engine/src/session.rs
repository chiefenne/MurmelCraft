//! The viewer session: the open mesh, its surface, the camera and the
//! settings. Front ends read it and change it only through tools.

use std::path::{Path, PathBuf};

use murmel_mesh::{Evidence, Mesh, MeshError};

use crate::camera::Camera;
use crate::navigation::Navigation;
use crate::surface::Surface;
use crate::theme::{Theme, Tokens};

/// A mesh loaded into the session.
#[derive(Clone, Debug)]
pub struct OpenMesh {
    pub path: PathBuf,
    pub mesh: Mesh,
    pub evidence: Option<Evidence>,
    /// Why the evidence beside the mesh could not be read (MESH011).
    pub evidence_problem: Option<MeshError>,
    pub surface: Surface,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub tokens: Tokens,
    pub mesh: Option<OpenMesh>,
    pub camera: Camera,
    pub navigation: Navigation,
    pub theme: Theme,
    /// Viewport size in pixels, for aspect, pan and zoom.
    pub viewport: [f64; 2],
    /// Bumped whenever the surface changes, so renderers re-upload.
    pub surface_revision: u64,
    /// Where settings tools write; `None` disables them.
    pub config_dir: Option<PathBuf>,
}

impl Session {
    pub fn new(tokens: Tokens) -> Self {
        Self {
            tokens,
            mesh: None,
            camera: Camera::default(),
            navigation: Navigation::default(),
            theme: Theme::default(),
            viewport: [1280.0, 720.0],
            surface_revision: 0,
            config_dir: crate::settings::config_dir(),
        }
    }

    pub fn aspect(&self) -> f64 {
        self.viewport[0].max(1.0) / self.viewport[1].max(1.0)
    }

    /// Read a mesh and its evidence, replace the open one and fit the view.
    pub fn open(&mut self, path: &Path) -> Result<&OpenMesh, MeshError> {
        let mesh = murmel_mesh::read(path)?;
        let (evidence, evidence_problem) = match Evidence::beside(path) {
            Ok(e) => (e, None),
            Err(e) => (None, Some(e)),
        };
        let surface = Surface::build(&mesh, &self.tokens);
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
        }))
    }

    pub fn close(&mut self) {
        self.mesh = None;
        self.surface_revision += 1;
    }

    /// Frame the open mesh.
    pub fn fit(&mut self) -> bool {
        let aspect = self.aspect();
        match self.mesh.as_ref().and_then(|m| m.surface.bounds) {
            Some(b) => {
                self.camera.fit(&b, aspect);
                true
            }
            None => false,
        }
    }
}
