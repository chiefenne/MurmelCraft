//! The tool journal (UI spec, Principles: "Undo is the tool journal").
//! Every call of a tool that changes the camera or the display records
//! the state before it; undo goes back to that state and redo forward
//! again. Calls inside one group (the window groups each mouse gesture
//! and each run of wheel steps) make one step, so undo steps are
//! gestures, not frames.
//!
//! The selection and the pick filter are not part of a step: undo and
//! redo leave them as they are, as CAD tools do. Opening a mesh starts a
//! new journal. Settings are files and are not undone.

use crate::camera::Camera;
use crate::display::Display;
use crate::session::Session;

/// Steps kept; the oldest go first.
pub const MAX_STEPS: usize = 200;

/// The undoable state of a session: camera and display, without the
/// selection, the pick filter and the quality and field value caches.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub camera: Camera,
    pub display: Display,
}

impl Snapshot {
    pub fn of(s: &Session) -> Self {
        let mut display = s.display.clone();
        display.selection.clear();
        display.pick = Default::default();
        display.quality = None;
        display.field_values = None;
        display.region_cells = None;
        Self {
            camera: s.camera.clone(),
            display,
        }
    }

    /// Put this state back into `s`, keeping its selection, pick filter
    /// and quality; the surface is rebuilt only when the display changed.
    fn restore(self, s: &mut Session) {
        // Saved views, measurements and probes are part of a step but not
        // of the picture.
        let mut now = Snapshot::of(s).display;
        now.views.clone_from(&self.display.views);
        now.measurements.clone_from(&self.display.measurements);
        now.probes.clone_from(&self.display.probes);
        let redraw = now != self.display;
        s.camera = self.camera;
        let mut d = self.display;
        d.selection = std::mem::take(&mut s.display.selection);
        d.pick = s.display.pick;
        d.quality = s.display.quality.take();
        d.field_values = s.display.field_values.take();
        // Found again by the redraw when the regions differ.
        d.region_cells = s.display.region_cells.take();
        // The values of the time step open now: streamlines traced on it,
        // and its parcel file under the restored parcel settings.
        d.stream_traces = if d.streams == s.display.streams {
            s.display.stream_traces.take()
        } else {
            None
        };
        if let (Some(v), Some(now)) = (d.parcels.as_mut(), s.display.parcels.as_ref()) {
            v.path.clone_from(&now.path);
            d.parcel_data = s.display.parcel_data.clone();
        }
        // Probes and measurements of a step name the cells of the mesh
        // shown then; an overview's regions may show other blocks now.
        if let Some(m) = s.mesh.as_ref().filter(|m| m.blocks.is_some()) {
            crate::session::anchor_to(&mut d, &m.mesh);
        }
        s.display = d;
        if redraw {
            s.redraw();
        }
    }
}

/// One step: the tool that made it and the state on the other side of it
/// (before it on the undo stack, after it on the redo stack).
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub tool: String,
    pub state: Snapshot,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Journal {
    pub undo: Vec<Step>,
    pub redo: Vec<Step>,
    /// A group is open; `true` once it has recorded its step.
    group: Option<bool>,
}

impl Journal {
    /// Open (`true`) or close a group. Opening an open group keeps it.
    pub fn group(&mut self, open: bool) {
        self.group = match (open, self.group) {
            (true, Some(recorded)) => Some(recorded),
            (true, None) => Some(false),
            (false, _) => None,
        };
    }

    pub fn group_open(&self) -> bool {
        self.group.is_some()
    }

    /// Record a call of `tool` that turned `before` into `after`. A call
    /// that changed nothing, or a later call of a group that already has
    /// its step, records nothing. A new step drops the redo stack.
    pub fn record(&mut self, tool: &str, before: Snapshot, after: &Snapshot) {
        if &before == after || self.group == Some(true) {
            return;
        }
        if let Some(recorded) = &mut self.group {
            *recorded = true;
        }
        self.undo.push(Step {
            tool: tool.into(),
            state: before,
        });
        if self.undo.len() > MAX_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Forget every step (a new mesh).
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

impl Session {
    /// Go back one step; the tool it undid, or `None` when there is none.
    pub fn undo(&mut self) -> Option<String> {
        self.journal.group(false);
        let step = self.journal.undo.pop()?;
        self.journal.redo.push(Step {
            tool: step.tool.clone(),
            state: Snapshot::of(self),
        });
        step.state.restore(self);
        Some(step.tool)
    }

    /// Go forward one undone step; the tool it redid, or `None`.
    pub fn redo(&mut self) -> Option<String> {
        self.journal.group(false);
        let step = self.journal.redo.pop()?;
        self.journal.undo.push(Step {
            tool: step.tool.clone(),
            state: Snapshot::of(self),
        });
        step.state.restore(self);
        Some(step.tool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::{ColorBy, Item};
    use crate::theme::Tokens;

    fn session() -> Session {
        Session::new(Tokens::builtin().unwrap())
    }

    /// Change the camera of `s` as a tool would, recording the step.
    fn orbit(s: &mut Session, deg: f64) {
        let before = Snapshot::of(s);
        s.camera.orbit(deg, 0.0);
        let after = Snapshot::of(s);
        s.journal.record("view.orbit", before, &after);
    }

    /// Invariant: undo returns to the state before a step and redo to the
    /// state after it; a new step drops the redo stack; a call that
    /// changes nothing records nothing. Breaks when undo restores the
    /// wrong side of a step or redo survives a new change; the tool tests
    /// only call through the registry.
    #[test]
    fn undo_and_redo_walk_the_steps() {
        let mut s = session();
        let c0 = s.camera.clone();
        orbit(&mut s, 10.0);
        let c1 = s.camera.clone();
        orbit(&mut s, 0.0);
        assert_eq!(s.journal.undo.len(), 1);
        assert_eq!(s.undo().as_deref(), Some("view.orbit"));
        assert_eq!(s.camera, c0);
        assert_eq!(s.undo(), None);
        assert_eq!(s.redo().as_deref(), Some("view.orbit"));
        assert_eq!(s.camera, c1);
        s.undo();
        orbit(&mut s, 5.0);
        assert!(s.journal.redo.is_empty());
        assert_eq!(s.redo(), None);
    }

    /// Invariant: all calls inside a group make one step, which undoes
    /// to the state before the group. Breaks when a drag of 60 frames
    /// needs 60 undos (the spec asks for gestures, not frames).
    #[test]
    fn a_group_is_one_step() {
        let mut s = session();
        let c0 = s.camera.clone();
        s.journal.group(true);
        for _ in 0..30 {
            orbit(&mut s, 1.0);
        }
        s.journal.group(true);
        orbit(&mut s, 1.0);
        s.journal.group(false);
        orbit(&mut s, 1.0);
        assert_eq!(s.journal.undo.len(), 2);
        s.undo();
        s.undo();
        assert_eq!(s.camera, c0);
    }

    /// Invariant: undo and redo leave the selection as it is and restore
    /// the display; the journal keeps at most MAX_STEPS steps. Breaks when
    /// undoing a colour change also drops the selection.
    #[test]
    fn selection_is_not_undone() {
        let mut s = session();
        s.display.selection = vec![Item::Mesh];
        let before = Snapshot::of(&s);
        s.display.color_by = ColorBy::Patch;
        let after = Snapshot::of(&s);
        s.journal.record("display.color_by", before, &after);
        s.undo();
        assert_eq!(s.display.color_by, ColorBy::default());
        assert_eq!(s.display.selection, vec![Item::Mesh]);
        for i in 0..MAX_STEPS + 10 {
            orbit(&mut s, 1.0 + i as f64);
        }
        assert_eq!(s.journal.undo.len(), MAX_STEPS);
    }
}
