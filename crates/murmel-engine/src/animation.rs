//! The motions of exported movies (`export.movie`): the camera turning
//! about the target, a clip plane sweeping through the mesh, and a flight
//! through saved views. Each gives one [`Frame`] (camera and clip offset)
//! per picture; the tools render them. No GPU, no files.
//!
//! The flight eases into every saved view and can hold there (the
//! maintainer's card of 2026-11-24, "Ease at each view, optional hold"),
//! with the window's view flight between two cameras ([`Camera::lerp`]:
//! shortest-arc rotation, smoothstep in time).

use crate::camera::Camera;

/// Most frames one movie may have (20 minutes at 30 frames a second).
pub const MAX_FRAMES: usize = 36_000;

/// The view of one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub camera: Camera,
    /// The clip plane (index into `Display::clips`) and its offset along
    /// its normal from the centre of the mesh's bounds, when it moves.
    pub clip: Option<(usize, f64)>,
}

/// Number of frames of `seconds` at `fps` (at least one).
pub fn frame_count(seconds: f64, fps: f64) -> usize {
    ((seconds * fps).round().max(1.0)) as usize
}

/// The camera turning `degrees` about the vertical axis through its
/// target (a turntable, as the mouse orbits), in `n` frames. A whole turn
/// leaves out the last frame, which equals the first, so the movie loops
/// without a stutter; a part turn ends exactly at `degrees`.
pub fn orbit(base: &Camera, degrees: f64, n: usize) -> Vec<Frame> {
    let n = n.max(1);
    let whole = (degrees.abs() - 360.0).abs() < 1e-9;
    let steps = if whole || n == 1 { n } else { n - 1 } as f64;
    (0..n)
        .map(|k| {
            let mut c = base.clone();
            c.orbit(degrees * k as f64 / steps, 0.0);
            Frame {
                camera: c,
                clip: None,
            }
        })
        .collect()
}

/// Clip plane `plane` moving from offset `from` to `to` (along its
/// normal, from the centre of the bounds) in `n` frames, both ends
/// included, the camera still.
pub fn clip_sweep(base: &Camera, plane: usize, from: f64, to: f64, n: usize) -> Vec<Frame> {
    let n = n.max(1);
    (0..n)
        .map(|k| {
            let t = if n == 1 {
                0.0
            } else {
                k as f64 / (n - 1) as f64
            };
            Frame {
                camera: base.clone(),
                clip: Some((plane, from + (to - from) * t)),
            }
        })
        .collect()
}

/// A flight through `views`: each view shows for `hold` seconds (at
/// least one frame), and the camera flies `travel` seconds to the next,
/// easing out of one view and into the next.
pub fn flight(views: &[Camera], hold: f64, travel: f64, fps: f64) -> Vec<Frame> {
    let shown = frame_count(hold, fps);
    let between = frame_count(travel, fps);
    let mut out = Vec::new();
    let still = |c: &Camera| Frame {
        camera: c.clone(),
        clip: None,
    };
    for (i, v) in views.iter().enumerate() {
        out.extend(std::iter::repeat_n(still(v), shown));
        if let Some(next) = views.get(i + 1) {
            for j in 1..between {
                out.push(still(&Camera::lerp(v, next, j as f64 / between as f64)));
            }
        }
    }
    out
}

/// Frames of a flight, before building them (to refuse a long one).
pub fn flight_count(views: usize, hold: f64, travel: f64, fps: f64) -> usize {
    let shown = frame_count(hold, fps);
    let between = frame_count(travel, fps);
    views * shown + views.saturating_sub(1) * (between - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::StandardView;

    /// Invariant: a whole turn has `n` distinct frames that step evenly
    /// and leave the last one (= the first) out, so a looping movie does
    /// not stutter; a part turn ends exactly at its angle; pitch, target
    /// and distance stay. Breaks with an off-by-one in the step.
    #[test]
    fn orbit_loops_and_ends() {
        let base = Camera::default();
        let f = orbit(&base, 360.0, 36);
        assert_eq!(f.len(), 36);
        let dyaw = |a: &Camera, b: &Camera| (b.yaw_deg - a.yaw_deg).rem_euclid(360.0);
        assert!((dyaw(&f[0].camera, &f[1].camera) - 10.0).abs() < 1e-9);
        assert!((dyaw(&f[35].camera, &f[0].camera) - 10.0).abs() < 1e-9);
        assert!(f.iter().all(|x| x.camera.pitch_deg == base.pitch_deg
            && x.camera.distance == base.distance
            && x.clip.is_none()));
        let part = orbit(&base, 90.0, 10);
        assert!((dyaw(&base, &part[9].camera) - 90.0).abs() < 1e-9);
        assert_eq!(part[0].camera, base);
    }

    /// Invariant: a sweep starts and ends at its offsets, both included,
    /// steps evenly, and moves only the plane it names.
    #[test]
    fn clip_sweep_covers_both_ends() {
        let f = clip_sweep(&Camera::default(), 1, -2.0, 2.0, 5);
        let offs: Vec<f64> = f.iter().map(|x| x.clip.unwrap().1).collect();
        assert_eq!(offs, vec![-2.0, -1.0, 0.0, 1.0, 2.0]);
        assert!(f.iter().all(|x| x.clip.unwrap().0 == 1));
    }

    /// Invariant (the maintainer's card "Ease at each view, optional
    /// hold"): the flight shows each view exactly for its hold, passes
    /// through every view exactly, eases (the first step out of a view is
    /// smaller than a middle step), and has as many frames as
    /// `flight_count` says. Breaks if the ease goes or a view is missed.
    #[test]
    fn flight_holds_and_eases() {
        let mut a = Camera::default();
        a.set_standard(StandardView::FRONT);
        let mut b = Camera::default();
        b.set_standard(StandardView::RIGHT);
        let mut c = Camera::default();
        c.set_standard(StandardView::TOP);
        let f = flight(&[a.clone(), b.clone(), c.clone()], 0.5, 1.0, 10.0);
        assert_eq!(f.len(), flight_count(3, 0.5, 1.0, 10.0));
        assert_eq!(f.len(), 3 * 5 + 2 * 9);
        assert!(f[..5].iter().all(|x| x.camera == a));
        assert!(f[14..19].iter().all(|x| x.camera == b));
        assert!(f[28..].iter().all(|x| x.camera == c));
        let angle = |x: &Camera, y: &Camera| x.to_eye().angle_between(y.to_eye());
        let first = angle(&f[4].camera, &f[5].camera);
        let middle = angle(&f[8].camera, &f[9].camera);
        assert!(first < middle / 2.0, "no ease: {first} vs {middle}");
        assert_eq!(flight(&[a.clone()], 0.0, 1.0, 30.0).len(), 1);
    }
}
