//! The mesh edges while the view moves: hidden only when a frame drawn
//! with them is slow (the maintainer's card: longer than 33 ms; nothing
//! else is simplified), and back as soon as the view rests.
//!
//! The frames drawn with edges while the view moves are timed start to
//! start, with the next frame asked for at once, so the gap is the
//! frame's own cost: the GPU holds a frame back until it has drawn the
//! ones before, so a slow draw shows as a long gap after a frame or two.
//! Once the median of the last three gaps is over the budget, the edges
//! hide while the view moves, for as long as the surface and the size of
//! the viewport stay the same; a new surface or size is timed again.

use murmel_engine::Camera;

/// The longest a frame may take before the edges hide while moving: 30
/// frames a second. A frame that keeps 30 frames a second (two refreshes
/// of a 60 Hz screen) is within it, give or take the timer's jitter.
pub(crate) const FRAME_BUDGET: f64 = 1.0 / 30.0;

/// The jitter of a frame's gap that still counts as within the budget.
const JITTER: f64 = 0.002;

/// How long after the camera's last change the view still moves.
pub(crate) const REST: f64 = 0.15;

/// The gaps timed before deciding.
const SAMPLES: usize = 3;

/// What a frame does with the edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Frame {
    /// The edges are left out of this frame.
    pub hide: bool,
    /// Ask for the next frame after this many seconds: at once to time
    /// this one, or once the view rests to bring the edges back.
    pub repaint: Option<f64>,
}

/// The timing of the frames drawn with edges while the view moves.
#[derive(Default)]
pub(crate) struct MovingEdges {
    /// The surface (its revision) and the viewport's size in pixels the
    /// timing belongs to.
    key: Option<(u64, [u32; 2])>,
    /// The last gaps timed, in seconds.
    gaps: Vec<f64>,
    /// Frames with edges were timed over the budget.
    slow: bool,
    /// The last frame drew the edges while moving and asked for this
    /// one at once: this frame's gap times it.
    timing: bool,
    /// The camera of the last frame and the time it last changed.
    camera: Option<Camera>,
    moved: Option<f64>,
}

impl MovingEdges {
    /// What this frame does with the edges. `now` is the frame's time and
    /// `gap` the time since the last frame began, in seconds; `key` the
    /// surface and the size drawn; `hideable` whether the edges are drawn
    /// with the mesh's faces (alone they are all there is to see, so they
    /// stay).
    pub(crate) fn frame(
        &mut self,
        now: f64,
        gap: f64,
        camera: &Camera,
        key: (u64, [u32; 2]),
        hideable: bool,
    ) -> Frame {
        if self.key != Some(key) {
            self.key = Some(key);
            self.gaps.clear();
            self.slow = false;
            self.timing = false;
        }
        if std::mem::take(&mut self.timing) && hideable {
            self.gaps.push(gap);
            if self.gaps.len() > SAMPLES {
                self.gaps.remove(0);
            }
            if self.gaps.len() == SAMPLES && median(&self.gaps) > FRAME_BUDGET + JITTER {
                self.slow = true;
            }
        }
        let changed = self.camera.as_ref() != Some(camera);
        if changed {
            // The first frame has nothing to move from.
            if self.camera.is_some() {
                self.moved = Some(now);
            }
            self.camera = Some(camera.clone());
        }
        let moving = self.moved.is_some_and(|t| now - t < REST);
        let still = Frame {
            hide: false,
            repaint: None,
        };
        if !hideable || !moving {
            return still;
        }
        if self.slow {
            return Frame {
                hide: true,
                repaint: Some(REST),
            };
        }
        if changed {
            // A new picture with edges: time it by the next frame.
            self.timing = true;
            return Frame {
                hide: false,
                repaint: Some(0.0),
            };
        }
        still
    }

    /// Whether frames with edges were timed over the budget.
    #[cfg(test)]
    pub(crate) fn slow(&self) -> bool {
        self.slow
    }
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[s.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: (u64, [u32; 2]) = (1, [800, 600]);

    fn turned(deg: f64) -> Camera {
        let mut c = Camera::default();
        c.orbit(deg, 0.0);
        c
    }

    /// Run frames `gap` seconds apart, turning the camera a degree each,
    /// and give what the last one did.
    fn orbit(
        e: &mut MovingEdges,
        t: &mut f64,
        gap: f64,
        frames: usize,
        key: (u64, [u32; 2]),
    ) -> Frame {
        let mut f = e.frame(*t, gap, &turned(0.0), key, true);
        for k in 1..=frames {
            *t += gap;
            f = e.frame(*t, gap, &turned(k as f64), key, true);
        }
        f
    }

    /// Invariant: frames that keep 30 a second (here two refreshes of a
    /// 60 Hz screen) leave the edges on while the view moves; frames over
    /// 33 ms hide them while it moves, and they come back once it rests.
    /// Breaks when the budget, the jitter or the rest change, or the
    /// edges are hidden from a single slow frame.
    #[test]
    fn edges_hide_only_while_slow_frames_move() {
        let (mut e, mut t) = (MovingEdges::default(), 0.0);
        let f = orbit(&mut e, &mut t, 2.0 / 60.0, 10, KEY);
        assert!(!f.hide && !e.slow());
        assert_eq!(f.repaint, Some(0.0), "a moving frame with edges is timed");

        let (mut e, mut t) = (MovingEdges::default(), 0.0);
        let f = orbit(&mut e, &mut t, 2.0 / 60.0, 5, KEY);
        assert!(!f.hide);
        // One hitch is not enough.
        t += 0.2;
        let f = e.frame(t, 0.2, &turned(3.0), KEY, true);
        assert!(!f.hide && !e.slow());
        let f = orbit(&mut e, &mut t, 3.0 / 60.0, 4, KEY);
        assert!(f.hide && e.slow());
        assert_eq!(f.repaint, Some(REST), "the edges come back once it rests");

        // At rest the edges are drawn; the next move hides them at once.
        t += REST + 0.01;
        let rest = e.frame(t, REST, &turned(4.0), KEY, true);
        assert!(!rest.hide);
        assert_eq!(rest.repaint, None);
        t += 5.0;
        assert!(e.frame(t, 5.0, &turned(5.0), KEY, true).hide);
    }

    /// Invariant: a new surface or viewport size is timed again, gaps
    /// after an idle pause or a frame drawn without edges are not timed,
    /// and edges drawn without the faces never hide. Breaks when the
    /// timing outlives its surface or counts idle time as drawing.
    #[test]
    fn only_frames_drawn_with_edges_are_timed() {
        let (mut e, mut t) = (MovingEdges::default(), 0.0);
        orbit(&mut e, &mut t, 0.1, 5, KEY);
        assert!(e.slow());
        let f = e.frame(t + 0.01, 0.01, &turned(6.0), (2, [800, 600]), true);
        assert!(!f.hide && !e.slow(), "a new surface is timed again");
        let f = e.frame(t + 0.02, 0.01, &turned(7.0), (2, [900, 600]), true);
        assert!(!f.hide && !e.slow(), "a new size is timed again");

        // Single moves seconds apart: the gaps are idle time.
        let (mut e, mut t) = (MovingEdges::default(), 0.0);
        e.frame(t, 0.0, &turned(0.0), KEY, true);
        for k in 1..10 {
            t += 2.0;
            let f = e.frame(t, 2.0, &turned(k as f64), KEY, true);
            assert!(!f.hide);
            // The frame asked for at once comes, without a move.
            t += 1.0 / 60.0;
            e.frame(t, 1.0 / 60.0, &turned(k as f64), KEY, true);
        }
        assert!(!e.slow());

        // Wireframe: the edges are all there is.
        let (mut e, mut t) = (MovingEdges::default(), 0.0);
        for k in 0..10 {
            t += 0.1;
            let f = e.frame(t, 0.1, &turned(k as f64), KEY, false);
            assert_eq!(
                f,
                Frame {
                    hide: false,
                    repaint: None
                }
            );
        }
        assert!(!e.slow());
    }
}
