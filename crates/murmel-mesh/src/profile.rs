//! Phases of reading and opening a mesh. Readers and the engine mark
//! their phases with [`span`]; `murmel bench` records how long each takes
//! ([`record`]; nothing is recorded otherwise, so a mark costs one atomic
//! load), and a window that opens a mesh on another thread follows the
//! phase and can cancel it through a [`Progress`] ([`watch`], [`check`]).

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::MeshError;

/// The phase an opening is in, and whether it should stop.
#[derive(Debug, Default)]
pub struct Progress {
    phase: Mutex<&'static str>,
    cancel: AtomicBool,
}

impl Progress {
    /// The phase last begun (a [`span`] name such as `read.numbers`), or
    /// "" before the first.
    pub fn phase(&self) -> &'static str {
        self.phase.lock().map_or("", |p| *p)
    }

    /// Ask the opening to stop at its next [`check`].
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

thread_local! {
    static WATCHED: RefCell<Option<Arc<Progress>>> = const { RefCell::new(None) };
}

/// Run `f` on this thread with `progress` following its phases.
pub fn watch<R>(progress: Arc<Progress>, f: impl FnOnce() -> R) -> R {
    let before = WATCHED.with(|w| w.replace(Some(progress)));
    let r = f();
    WATCHED.with(|w| *w.borrow_mut() = before);
    r
}

/// `MESH019` when the opening this thread does was cancelled.
pub fn check() -> Result<(), MeshError> {
    let stop = WATCHED.with(|w| w.borrow().as_ref().is_some_and(|p| p.cancelled()));
    if stop {
        Err(MeshError::new("MESH019", "opening the mesh was cancelled"))
    } else {
        Ok(())
    }
}

static ON: AtomicBool = AtomicBool::new(false);
static SPANS: Mutex<Vec<(&'static str, f64)>> = Mutex::new(Vec::new());

/// A running phase; its time is recorded when it is dropped.
pub struct Span {
    name: &'static str,
    start: Option<Instant>,
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some(t) = self.start {
            let ms = t.elapsed().as_secs_f64() * 1e3;
            if let Ok(mut v) = SPANS.lock() {
                v.push((self.name, ms));
            }
        }
    }
}

/// Mark a phase from now until the returned guard is dropped.
pub fn span(name: &'static str) -> Span {
    WATCHED.with(|w| {
        if let Some(p) = w.borrow().as_ref()
            && let Ok(mut phase) = p.phase.lock()
        {
            *phase = name;
        }
    });
    Span {
        name,
        start: ON.load(Ordering::Relaxed).then(Instant::now),
    }
}

/// Turn recording on or off; turning it on clears what was recorded.
pub fn record(on: bool) {
    if on && let Ok(mut v) = SPANS.lock() {
        v.clear();
    }
    ON.store(on, Ordering::Relaxed);
}

/// The phases recorded since [`record`] turned recording on, in the
/// order they ended, in milliseconds.
pub fn take() -> Vec<(&'static str, f64)> {
    SPANS
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: a watched thread's spans set the phase the window shows,
    /// `check` stops it once cancelled and never on another thread, and
    /// recording keeps the spans' names. A phase set on the wrong thread
    /// would show another opening's phase, a check that ignored the
    /// cancel would keep a cancelled opening running to its end.
    #[test]
    fn phases_and_cancel() {
        let p = Arc::new(Progress::default());
        let q = p.clone();
        watch(p.clone(), || {
            let _s = span("read.file");
            assert_eq!(q.phase(), "read.file");
            assert!(check().is_ok());
            q.cancel();
            assert_eq!(check().unwrap_err().code, "MESH019");
        });
        assert!(check().is_ok(), "not watched here");
        std::thread::spawn(|| assert!(check().is_ok()))
            .join()
            .unwrap();
    }
}
