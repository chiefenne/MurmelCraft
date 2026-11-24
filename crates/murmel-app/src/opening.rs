//! Opening a large mesh on a thread of its own: the window keeps drawing,
//! the status bar shows the phase and the time so far, and Cancel (or Esc)
//! stops the reader at its next check. Once read, the mesh opens through
//! `mesh.open` (or `session.load`) like any other, from
//! `Session::preloaded`, so the tools, the journal and the agents see the
//! same open as before.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use egui::{RichText, Role, Ui, WidgetInfo};
use murmel_engine::{Loaded, OpenMode};
use murmel_mesh::MeshError;
use murmel_mesh::profile::{self, Progress};
use murmel_tools::Diagnostic;

use crate::app::MurmelApp;

/// Meshes of at least this many bytes are read on a thread of their own;
/// smaller ones open at once (a few tenths of a second at most).
pub(crate) const BACKGROUND_BYTES: u64 = 32 << 20;

/// What opens once the mesh is read: the mesh, or the session file that
/// names it.
#[derive(Debug)]
pub(crate) enum Then {
    /// Open the mesh, as the mode asks (`mesh.open`).
    Mesh(PathBuf, OpenMode),
    Session(PathBuf),
}

/// A mesh being read on its own thread.
pub(crate) struct Opening {
    pub(crate) path: PathBuf,
    then: Then,
    pub(crate) progress: Arc<Progress>,
    handle: JoinHandle<Result<Loaded, MeshError>>,
    /// The frame time it started at.
    started: f64,
}

/// The bytes of a mesh file, or of the files in a polyMesh folder (the
/// case, `constant` or `polyMesh` folder given).
pub(crate) fn mesh_bytes(path: &Path) -> u64 {
    if path.is_file() {
        return std::fs::metadata(path).map_or(0, |m| m.len());
    }
    let poly = [
        path.to_path_buf(),
        path.join("polyMesh"),
        path.join("constant").join("polyMesh"),
    ]
    .into_iter()
    .find(|d| d.join("faces").is_file() || d.join("faces.gz").is_file());
    poly.and_then(|d| std::fs::read_dir(d).ok())
        .map_or(0, |entries| {
            entries
                .flatten()
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
}

impl MurmelApp {
    /// Read the mesh at `path` on a thread of its own when it is large,
    /// and open it (`then`) once read. False when it is small: the caller
    /// opens it at once.
    pub(crate) fn open_in_background(&mut self, path: PathBuf, then: Then) -> bool {
        if mesh_bytes(&path) < self.background_bytes {
            return false;
        }
        // The newer request wins.
        if let Some(o) = self.opening.take() {
            o.progress.cancel();
        }
        let progress = Arc::new(Progress::default());
        let watched = progress.clone();
        // The surface is built for the display and tokens of now; the
        // open builds it again if they change meanwhile.
        let display = self.session.display.clone();
        let tokens = self.session.tokens.clone();
        let from = path.clone();
        // A mesh opens before its files are hashed (the session hashes
        // them afterwards); a session file needs the digest at once, to
        // tell whether its mesh changed, so its mesh is hashed here.
        let hash = matches!(then, Then::Session(_)) || !self.session.hash_after_open;
        let checked = self.session.checked_dir().map(std::path::Path::to_path_buf);
        let fit = match &then {
            Then::Mesh(_, mode) => self.session.fit_for(*mode),
            Then::Session(_) => self.session.budget_fit(),
        };
        #[cfg(test)]
        let gate = self.opening_gate.take();
        let handle = std::thread::spawn(move || {
            #[cfg(test)]
            if let Some(g) = gate {
                let _ = g.recv();
            }
            profile::watch(watched, || {
                let mut l = murmel_engine::load_checked(&from, hash, checked.as_deref(), fit)?;
                profile::check()?;
                l.prepare(&display, &tokens);
                Ok(l)
            })
        });
        self.opening = Some(Opening {
            path,
            then,
            progress,
            handle,
            started: self.now,
        });
        true
    }

    /// Stop the mesh being read; the window stays as it is.
    pub(crate) fn cancel_opening(&mut self) {
        if let Some(o) = &self.opening {
            o.progress.cancel();
        }
    }

    /// Open the mesh read on its own thread once it is read; keep the
    /// window drawing while it is.
    pub(crate) fn poll_opening(&mut self, ctx: &egui::Context) {
        let Some(o) = &self.opening else { return };
        if !o.handle.is_finished() {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        let Some(o) = self.opening.take() else { return };
        let cancelled = o.progress.cancelled();
        match o.handle.join() {
            // Cancelled: whatever was read is dropped.
            _ if cancelled => {}
            Ok(Ok(l)) => {
                self.session.preloaded = Some(l);
                self.open_now(o.then);
                // A mesh the call did not take (a session file that names
                // another one now) is not kept.
                self.session.preloaded = None;
            }
            Ok(Err(e)) => self.set_error(Some(Diagnostic::new(e.code, e.message))),
            // The reader stopped unexpectedly: open the usual way, which
            // reports what went wrong.
            Err(_) => self.open_now(o.then),
        }
    }

    /// Show the badge the digests of the open mesh's files decide once
    /// they are taken; keep looking while they are.
    pub(crate) fn poll_digest(&mut self, ctx: &egui::Context) {
        if self.session.poll_digest() {
            ctx.request_repaint();
        } else if self.session.hashing.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// Follow the clip preview: start the builds it needs and draw the
    /// exact cut once it is in place.
    pub(crate) fn poll_surface(&mut self, ctx: &egui::Context) {
        let before = self.session.surface_revision;
        if self.session.poll_surface() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        if self.session.surface_revision != before {
            ctx.request_repaint();
        }
    }

    /// Open the mesh, or load the session file, on this thread.
    pub(crate) fn open_now(&mut self, then: Then) {
        match then {
            Then::Mesh(mesh, mode) => {
                if self
                    .call(
                        "mesh.open",
                        serde_json::json!({ "path": mesh, "mode": mode }),
                    )
                    .is_some()
                {
                    // Ctrl + S no longer writes the last session over it.
                    self.session_path = None;
                }
            }
            Then::Session(file) => {
                if self
                    .call("session.load", serde_json::json!({ "path": file }))
                    .is_some()
                {
                    self.session_path = Some(file);
                }
            }
        }
    }

    /// What the window says while a mesh is read: its name, the phase and
    /// the seconds so far.
    pub(crate) fn opening_text(&self) -> Option<String> {
        let o = self.opening.as_ref()?;
        let phase = self.i18n.tr(phase_message(o.progress.phase()));
        let file = o
            .path
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let seconds = format!("{:.1}", (self.now - o.started).max(0.0));
        Some(self.i18n.tr_args(
            "status-loading",
            &[
                ("file", file.into()),
                ("phase", phase.into()),
                ("seconds", seconds.into()),
            ],
        ))
    }

    /// The status bar while a mesh is read: what it says and Cancel.
    pub(crate) fn opening_status(&mut self, ui: &mut Ui, said: &mut Vec<String>) -> bool {
        let Some(text) = self.opening_text() else {
            return false;
        };
        let cancel = self.i18n.tr("status-loading-cancel");
        let tip = self.i18n.tr("status-loading-cancel-tip");
        let b = ui.small_button(&cancel).on_hover_text(&tip);
        let name = format!("{cancel}. {tip}");
        b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
        if b.clicked() {
            self.cancel_opening();
        }
        ui.label(RichText::new(&text).small());
        said.push(text);
        true
    }
}

/// The message the window shows for a phase of the reader (the name of
/// its current `profile::span`).
fn phase_message(phase: &str) -> &'static str {
    match phase {
        "read.xml"
        | "read.numbers"
        | "read.points"
        | "read.faces"
        | "read.owner_neighbour"
        | "read.vtkhdf" => "loading-phase-numbers",
        "read.cells" => "loading-phase-cells",
        "read.overview" => "loading-phase-overview",
        p if p == "read.topology" || p.starts_with("topology.") => "loading-phase-topology",
        p if p == "open.surface" || p.starts_with("surface.") => "loading-phase-surface",
        _ => "loading-phase-file",
    }
}

#[cfg(test)]
mod tests {
    use super::phase_message;

    /// Invariant: while the cell faces are linked, the window says
    /// "matching the faces" through every step of the linking
    /// (`topology.*`), not only at its start. Dropping the `topology.` arm
    /// would show "reading the file" for most of that time. The window test
    /// of a background open reads the text once, before the reader starts,
    /// so it would not notice.
    #[test]
    fn the_steps_of_linking_show_as_matching_the_faces() {
        for p in ["read.topology", "topology.buckets", "topology.rest"] {
            assert_eq!(phase_message(p), "loading-phase-topology", "{p}");
        }
        assert_eq!(phase_message("read.stream"), "loading-phase-file");
    }
}
