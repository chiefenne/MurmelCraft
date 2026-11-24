//! Export of movies (`export.movie`): the camera turning about its target,
//! a clip plane sweeping through the mesh, or a flight through saved views
//! (the motions of `murmel_engine::animation`). Every frame is rendered as
//! `export.image` renders a picture (size, scale, legend, axes, lines) and
//! written as PNG frames, a GIF or an MP4.
//!
//! Decided by the maintainer on a card (2026-11-24, "GIF built in, MP4 via
//! ffmpeg"): PNG frames and GIF are written in Rust (the `gif` crate, MIT
//! OR Apache-2.0, with `color_quant`, MIT). MP4 (H.264) pipes the frames
//! to the ffmpeg installed on the machine, which MurmelCraft neither links
//! nor ships; without one the call says how to install it (EXPORT002).
//!
//! A front end that wants a progress bar and a Cancel button runs the call
//! inside [`with_progress`] on a thread of its own.

use std::cell::RefCell;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;

use murmel_engine::animation::{self, Frame, MAX_FRAMES};
use murmel_engine::{Session, Theme};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::export::{self, Look, draw_overlays, picture_size};
use crate::tools::{camera_json, finite, need_mesh, object, parse, plane_index};
use crate::{Diagnostic, Run};

/// Length of a turn or a sweep when none is given, in seconds.
pub const SECONDS: f64 = 6.0;
/// Seconds at each saved view, and of each flight between two.
pub const HOLD: f64 = 0.5;
pub const TRAVEL: f64 = 2.0;
/// Longest movie and flight, in seconds.
const MAX_SECONDS: f64 = 1200.0;
const MAX_STEP: f64 = 60.0;
/// NeuQuant's speed for GIF palettes (1 best, 30 fastest); 10 is the
/// crate's advice.
const GIF_SPEED: i32 = 10;

// ---- progress and cancel -------------------------------------------------

/// How far a movie is, and a request to stop it: shared between the
/// thread that makes the movie and the window that shows its progress.
#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicUsize,
    total: AtomicUsize,
    cancel: AtomicBool,
}

impl Progress {
    /// Frames rendered and written so far.
    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }

    /// Frames of the movie (0 until the call has worked them out).
    pub fn total(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }

    /// Ask the movie to stop before its next frame (EXPORT006).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

thread_local! {
    static PROGRESS: RefCell<Option<Arc<Progress>>> = const { RefCell::new(None) };
}

/// Clears the progress of this thread when the call ends, even by a panic.
struct Reset;

impl Drop for Reset {
    fn drop(&mut self) {
        PROGRESS.with(|c| *c.borrow_mut() = None);
    }
}

/// Run `f` (a call of `export.movie` on this thread) with `p` counting its
/// frames and taking its Cancel.
pub fn with_progress<R>(p: Arc<Progress>, f: impl FnOnce() -> R) -> R {
    PROGRESS.with(|c| *c.borrow_mut() = Some(p));
    let _reset = Reset;
    f()
}

fn progress() -> Arc<Progress> {
    PROGRESS.with(|c| c.borrow().clone()).unwrap_or_default()
}

// ---- input ---------------------------------------------------------------

/// What moves in the movie.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    /// The camera turns about the vertical axis through its target.
    Orbit,
    /// A clip plane sweeps through the mesh.
    Clip,
    /// The camera flies through saved views.
    Views,
}

impl Motion {
    pub const ALL: [Motion; 3] = [Motion::Orbit, Motion::Clip, Motion::Views];

    pub fn name(self) -> &'static str {
        match self {
            Motion::Orbit => "orbit",
            Motion::Clip => "clip",
            Motion::Views => "views",
        }
    }
}

/// The file a movie is written as.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Mp4,
    Gif,
    /// A folder of PNG frames (`frame-00001.png`, …).
    Png,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Mp4, Format::Gif, Format::Png];

    /// The format a path asks for: `.mp4` or `.gif`, else a folder of PNG
    /// frames.
    pub fn of(path: &Path) -> Format {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some("mp4") => Format::Mp4,
            Some("gif") => Format::Gif,
            _ => Format::Png,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Mp4 => "mp4",
            Format::Gif => "gif",
            Format::Png => "png",
        }
    }
}

fn yes() -> bool {
    true
}

fn thirty() -> f64 {
    30.0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportMovie {
    path: PathBuf,
    format: Option<Format>,
    motion: Motion,
    seconds: Option<f64>,
    #[serde(default = "thirty")]
    fps: f64,
    degrees: Option<f64>,
    plane: Option<u32>,
    from: Option<f64>,
    to: Option<f64>,
    views: Option<Vec<String>>,
    hold: Option<f64>,
    travel: Option<f64>,
    width: Option<u32>,
    height: Option<u32>,
    scale: Option<f64>,
    #[serde(default)]
    transparent: bool,
    #[serde(default = "yes")]
    legend: bool,
    #[serde(default = "yes")]
    axes: bool,
    line_width: Option<f64>,
    theme: Option<Theme>,
}

/// `x` when it lies in `lo..=hi` (above `lo` when `open`), else EXPORT001.
fn within(x: f64, lo: f64, hi: f64, open: bool, what: &str) -> Result<f64, Diagnostic> {
    let above = if open { x > lo } else { x >= lo };
    if x.is_finite() && above && x <= hi {
        Ok(x)
    } else {
        let from = if open { "above" } else { "from" };
        Err(Diagnostic::new(
            "EXPORT001",
            format!("{what} {x} is outside {from} {lo} to {hi}"),
        ))
    }
}

/// Refuse the fields of another motion, so a mistyped call is not
/// silently a different movie.
fn only_for(i: &ExportMovie) -> Result<(), Diagnostic> {
    use Motion::{Clip, Orbit, Views};
    let given: [(&str, bool, &[Motion]); 8] = [
        ("seconds", i.seconds.is_some(), &[Orbit, Clip]),
        ("degrees", i.degrees.is_some(), &[Orbit]),
        ("plane", i.plane.is_some(), &[Clip]),
        ("from", i.from.is_some(), &[Clip]),
        ("to", i.to.is_some(), &[Clip]),
        ("views", i.views.is_some(), &[Views]),
        ("hold", i.hold.is_some(), &[Views]),
        ("travel", i.travel.is_some(), &[Views]),
    ];
    for (name, set, motions) in given {
        if set && !motions.contains(&i.motion) {
            let names: Vec<&str> = motions.iter().map(|m| m.name()).collect();
            return Err(Diagnostic::new(
                "TOOL002",
                format!("{name} is for motion {}", names.join(" or ")),
            ));
        }
    }
    Ok(())
}

fn too_long(n: usize) -> Result<(), Diagnostic> {
    if n > MAX_FRAMES {
        Err(Diagnostic::new(
            "EXPORT001",
            format!("{n} frames are more than the {MAX_FRAMES} a movie may have"),
        ))
    } else {
        Ok(())
    }
}

/// The frames of the movie `i` asks for, and the motion as the result
/// reports it.
fn frames(i: &ExportMovie, s: &Session, fps: f64) -> Result<(Vec<Frame>, Value), Diagnostic> {
    let seconds = || {
        within(
            i.seconds.unwrap_or(SECONDS),
            0.0,
            MAX_SECONDS,
            true,
            "seconds",
        )
    };
    match i.motion {
        Motion::Orbit => {
            let degrees = within(
                i.degrees.unwrap_or(360.0),
                -3600.0,
                3600.0,
                false,
                "degrees",
            )?;
            let n = animation::frame_count(seconds()?, fps);
            too_long(n)?;
            Ok((
                animation::orbit(&s.camera, degrees, n),
                json!({"kind": "orbit", "degrees": degrees}),
            ))
        }
        Motion::Clip => {
            let k = plane_index(s, i.plane)?.ok_or_else(|| {
                Diagnostic::new(
                    "EXPORT005",
                    "there is no clip plane to sweep; add one with clip.set",
                )
            })?;
            let bounds = s
                .mesh
                .as_ref()
                .map(|m| m.mesh.bounds())
                .unwrap_or_else(murmel_mesh::Bounds::empty);
            let (_, [lo, hi]) = s.display.clips[k].offset(&bounds);
            // From the end where the plane cuts nothing to the end where
            // it cuts everything.
            let from = finite(i.from.unwrap_or(hi), "from")?;
            let to = finite(i.to.unwrap_or(lo), "to")?;
            let n = animation::frame_count(seconds()?, fps);
            too_long(n)?;
            Ok((
                animation::clip_sweep(&s.camera, k, from, to, n),
                json!({"kind": "clip", "plane": k + 1, "from": from, "to": to, "range": [lo, hi]}),
            ))
        }
        Motion::Views => {
            let names: Vec<String> = match &i.views {
                Some(v) => v.clone(),
                None => s.display.views.iter().map(|v| v.name.clone()).collect(),
            };
            let cameras = names
                .iter()
                .map(|n| {
                    s.display.view(n).map(|v| v.camera.clone()).ok_or_else(|| {
                        Diagnostic::new(
                            "EXPORT005",
                            format!("there is no saved view {n:?}; views.list names them"),
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if cameras.len() < 2 {
                return Err(Diagnostic::new(
                    "EXPORT005",
                    format!(
                        "a flight needs two saved views or more, and has {}; save views with views.save",
                        cameras.len()
                    ),
                ));
            }
            let hold = within(i.hold.unwrap_or(HOLD), 0.0, MAX_STEP, false, "hold")?;
            let travel = within(i.travel.unwrap_or(TRAVEL), 0.0, MAX_STEP, true, "travel")?;
            too_long(animation::flight_count(cameras.len(), hold, travel, fps))?;
            Ok((
                animation::flight(&cameras, hold, travel, fps),
                json!({"kind": "views", "views": names, "hold": hold, "travel": travel}),
            ))
        }
    }
}

// ---- writers -------------------------------------------------------------

/// Where the frames go.
trait Sink {
    fn frame(&mut self, rgba: &[u8]) -> Result<(), Diagnostic>;
    /// Close the movie; the encoder that wrote it.
    fn finish(self: Box<Self>) -> Result<String, Diagnostic>;
    /// Stop after a failure or a Cancel, and remove a half-written file.
    fn abandon(self: Box<Self>);
}

fn write_err(path: &Path, e: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::new("EXPORT004", format!("{}: {e}", path.display()))
}

/// A folder of PNG frames, `frame-00001.png` and on (five digits hold
/// [`MAX_FRAMES`]).
struct PngFrames {
    dir: PathBuf,
    width: u32,
    height: u32,
    written: usize,
}

impl PngFrames {
    fn new(dir: &Path, width: u32, height: u32) -> Result<Self, Diagnostic> {
        std::fs::create_dir_all(dir).map_err(|e| write_err(dir, e))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            width,
            height,
            written: 0,
        })
    }

    fn name(k: usize) -> String {
        format!("frame-{:05}.png", k + 1)
    }
}

impl Sink for PngFrames {
    fn frame(&mut self, rgba: &[u8]) -> Result<(), Diagnostic> {
        let path = self.dir.join(Self::name(self.written));
        murmel_render::write_png_with(&path, self.width, self.height, rgba, None)
            .map_err(|e| Diagnostic::new("EXPORT004", e.message))?;
        self.written += 1;
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<String, Diagnostic> {
        Ok("png".into())
    }

    fn abandon(self: Box<Self>) {
        // The frames written stay: each is a whole picture, and the
        // diagnostic says how far the movie got.
    }
}

/// An animated GIF that loops, a palette of up to 256 colours per frame
/// (NeuQuant when a frame has more).
struct Gif {
    path: PathBuf,
    enc: gif::Encoder<BufWriter<File>>,
    width: u16,
    height: u16,
    /// Hundredths of a second per frame.
    delay: u16,
    transparent: bool,
}

/// GIF times frames in hundredths of a second, and players show delays
/// under two hundredths slower, so a GIF plays at most 50 frames a second.
pub fn gif_delay(fps: f64) -> u16 {
    (100.0 / fps).round().clamp(2.0, 100.0) as u16
}

impl Gif {
    fn new(
        path: &Path,
        width: u32,
        height: u32,
        fps: f64,
        transparent: bool,
    ) -> Result<Self, Diagnostic> {
        let (w, h) = (
            u16::try_from(width).map_err(|e| write_err(path, e))?,
            u16::try_from(height).map_err(|e| write_err(path, e))?,
        );
        let file = File::create(path).map_err(|e| write_err(path, e))?;
        let mut enc =
            gif::Encoder::new(BufWriter::new(file), w, h, &[]).map_err(|e| write_err(path, e))?;
        enc.set_repeat(gif::Repeat::Infinite)
            .map_err(|e| write_err(path, e))?;
        Ok(Self {
            path: path.to_path_buf(),
            enc,
            width: w,
            height: h,
            delay: gif_delay(fps),
            transparent,
        })
    }
}

impl Sink for Gif {
    fn frame(&mut self, rgba: &[u8]) -> Result<(), Diagnostic> {
        let mut px = rgba.to_vec();
        let mut f = gif::Frame::from_rgba_speed(self.width, self.height, &mut px, GIF_SPEED);
        f.delay = self.delay;
        if self.transparent {
            // Clear each frame before the next, or the earlier frames
            // show through its transparent pixels.
            f.dispose = gif::DisposalMethod::Background;
        }
        self.enc
            .write_frame(&f)
            .map_err(|e| write_err(&self.path, e))
    }

    fn finish(self: Box<Self>) -> Result<String, Diagnostic> {
        let path = self.path.clone();
        let done = self
            .enc
            .into_inner()
            .map_err(|e| write_err(&path, e))
            .and_then(|mut w| w.flush().map_err(|e| write_err(&path, e)));
        if let Err(d) = done {
            let _ = std::fs::remove_file(&path);
            return Err(d);
        }
        Ok("gif".into())
    }

    fn abandon(self: Box<Self>) {
        let path = self.path.clone();
        drop(self);
        let _ = std::fs::remove_file(path);
    }
}

/// The ffmpeg to call for MP4: the file `MURMEL_FFMPEG` names when set
/// (`var`), else the first `ffmpeg` on `PATH` (`path`) or in Homebrew's
/// folders, since a Mac app started from the Finder does not see the
/// shell's PATH.
pub fn find_ffmpeg(var: Option<OsString>, path: Option<OsString>) -> Result<PathBuf, Diagnostic> {
    if let Some(v) = var.filter(|v| !v.is_empty()) {
        let p = PathBuf::from(v);
        return if p.is_file() {
            Ok(p)
        } else {
            Err(Diagnostic::new(
                "EXPORT002",
                format!("MURMEL_FFMPEG names {}, which is not a file", p.display()),
            ))
        };
    }
    let exe = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let mut dirs: Vec<PathBuf> = path
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    dirs.into_iter()
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            Diagnostic::new(
                "EXPORT002",
                "MP4 needs ffmpeg, which was not found on PATH, in /opt/homebrew/bin or in /usr/local/bin",
            )
        })
}

/// The video encoder to ask `ffmpeg` for, best first: x264, Apple's
/// VideoToolbox H.264, else ffmpeg's own MPEG-4 part 2.
fn pick_encoder(ffmpeg: &Path) -> Result<&'static str, Diagnostic> {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| Diagnostic::new("EXPORT003", format!("{}: {e}", ffmpeg.display())))?;
    let list = String::from_utf8_lossy(&out.stdout);
    let has = |name: &str| {
        list.lines()
            .any(|l| l.split_whitespace().nth(1) == Some(name))
    };
    Ok(["libx264", "h264_videotoolbox", "mpeg4"]
        .into_iter()
        .find(|e| has(e))
        .unwrap_or("mpeg4"))
}

/// The arguments of ffmpeg: raw RGBA frames on stdin, an MP4 that plays
/// everywhere (yuv420p, BT.709, the index at the front) at `path`.
pub fn ffmpeg_args(path: &Path, width: u32, height: u32, fps: f64, encoder: &str) -> Vec<OsString> {
    let mut a: Vec<OsString> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        &format!("{width}x{height}"),
        "-framerate",
        &format!("{fps}"),
        "-i",
        "pipe:0",
        "-c:v",
        encoder,
    ]
    .map(OsString::from)
    .into();
    let quality: Vec<String> = match encoder {
        "libx264" => ["-preset", "medium", "-crf", "18"].map(String::from).into(),
        // About 0.1 bit a pixel, as a high quality stream needs.
        "h264_videotoolbox" => vec![
            "-b:v".into(),
            format!(
                "{}",
                (width as f64 * height as f64 * fps * 0.1).round() as u64
            ),
        ],
        _ => ["-q:v", "2"].map(String::from).into(),
    };
    a.extend(quality.into_iter().map(OsString::from));
    a.extend(
        [
            "-vf",
            "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p",
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ]
        .map(OsString::from),
    );
    // "file:" so a name like "x:y.mp4" is never read as a protocol.
    let mut out = OsString::from("file:");
    out.push(path.as_os_str());
    a.push(out);
    a
}

/// An MP4 written by ffmpeg, the frames piped to it as raw RGBA.
pub(crate) struct Ffmpeg {
    path: PathBuf,
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: Option<JoinHandle<String>>,
    encoder: String,
}

impl Ffmpeg {
    pub(crate) fn start(
        ffmpeg: &Path,
        path: &Path,
        width: u32,
        height: u32,
        fps: f64,
    ) -> Result<Self, Diagnostic> {
        let encoder = pick_encoder(ffmpeg)?;
        let mut child = Command::new(ffmpeg)
            .args(ffmpeg_args(path, width, height, fps, encoder))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Diagnostic::new("EXPORT003", format!("{}: {e}", ffmpeg.display())))?;
        let stdin = child.stdin.take();
        // Read what ffmpeg says on a thread of its own, so a full pipe
        // never stops it.
        let stderr = child.stderr.take().map(|mut e| {
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = e.read_to_string(&mut s);
                s
            })
        });
        Ok(Self {
            path: path.to_path_buf(),
            child,
            stdin,
            stderr,
            encoder: encoder.into(),
        })
    }

    /// Wait for ffmpeg to end; what it said when it failed.
    fn end(&mut self) -> Result<(), String> {
        self.stdin = None;
        let status = self.child.wait();
        let said = self
            .stderr
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("{s}: {}", last_lines(&said))),
            Err(e) => Err(e.to_string()),
        }
    }

    fn failed(&mut self, why: String) -> Diagnostic {
        let _ = std::fs::remove_file(&self.path);
        Diagnostic::new(
            "EXPORT003",
            format!("ffmpeg ({}) failed: {why}", self.encoder),
        )
    }
}

/// The last few lines of what a program said, enough to tell why.
fn last_lines(s: &str) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(4)..].join(" / ")
}

impl Sink for Ffmpeg {
    fn frame(&mut self, rgba: &[u8]) -> Result<(), Diagnostic> {
        let wrote = match self.stdin.as_mut() {
            Some(w) => w.write_all(rgba),
            None => Ok(()),
        };
        if let Err(e) = wrote {
            // ffmpeg stopped reading: it ended, and says why.
            let why = self.end().err().unwrap_or_else(|| e.to_string());
            return Err(self.failed(why));
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<String, Diagnostic> {
        match self.end() {
            Ok(()) => Ok(self.encoder.clone()),
            Err(why) => Err(self.failed(why)),
        }
    }

    fn abandon(mut self: Box<Self>) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.end();
        let _ = std::fs::remove_file(&self.path);
    }
}

// ---- the tool ------------------------------------------------------------

/// Render `frames` of `s` as `look` says into `sink`, counting them in
/// `p`; the GPU adapter used. The caller puts the camera and the clip
/// planes back.
fn render(
    s: &mut Session,
    frames: &[Frame],
    look: &Look,
    sink: &mut dyn Sink,
    p: &Progress,
) -> Result<String, Diagnostic> {
    let i18n = export::i18n()?;
    let bounds = s
        .mesh
        .as_ref()
        .map(|m| m.mesh.bounds())
        .unwrap_or_else(murmel_mesh::Bounds::empty);
    let shoot = look.shoot();
    murmel_render::with_offscreen(|o| {
        let mut adapter = String::new();
        for (k, f) in frames.iter().enumerate() {
            if p.cancelled() {
                return Err(Diagnostic::new(
                    "EXPORT006",
                    format!("cancelled after {k} of {} frames", frames.len()),
                ));
            }
            s.camera = f.camera.clone();
            if let Some((plane, offset)) = f.clip
                && let Some(c) = s.display.clips.get_mut(plane)
            {
                c.set_offset(&bounds, offset);
                c.enabled = true;
                s.redraw();
            }
            let mut shot = o
                .shoot(s, &shoot)
                .map_err(|e| Diagnostic::new(e.code, e.message))?;
            draw_overlays(s, look, &i18n, &mut shot.rgba)?;
            sink.frame(&shot.rgba)?;
            adapter = shot.adapter;
            p.done.store(k + 1, Ordering::Relaxed);
        }
        Ok(adapter)
    })
    .map_err(|e| Diagnostic::new(e.code, e.message))?
}

pub fn export_movie(input: Value, s: &mut Session) -> Run {
    let i: ExportMovie = parse(input)?;
    need_mesh(s)?;
    only_for(&i)?;
    let format = i.format.unwrap_or_else(|| Format::of(&i.path));
    let fps = within(i.fps, 1.0, 60.0, false, "fps")?;
    let (frames, motion) = frames(&i, s, fps)?;
    let (mut width, mut height, scale) = picture_size(s.viewport, i.width, i.height, i.scale)?;
    if format == Format::Mp4 {
        if i.transparent {
            return Err(Diagnostic::new(
                "EXPORT001",
                "an MP4 has no transparent background; write a GIF or PNG frames",
            ));
        }
        // H.264 in yuv420p needs even sides: an odd side loses a pixel.
        width -= width % 2;
        height -= height % 2;
        if width == 0 || height == 0 {
            return Err(Diagnostic::new(
                "EXPORT001",
                "an MP4 needs at least 2 x 2 pixels",
            ));
        }
    }
    let look = Look {
        width,
        height,
        scale,
        transparent: i.transparent,
        legend: i.legend,
        axes: i.axes,
        line_width: export::line_width(i.line_width, scale)?,
    };
    let p = progress();
    p.done.store(0, Ordering::Relaxed);
    p.total.store(frames.len(), Ordering::Relaxed);
    let mut sink: Box<dyn Sink> = match format {
        Format::Png => Box::new(PngFrames::new(&i.path, width, height)?),
        Format::Gif => Box::new(Gif::new(&i.path, width, height, fps, i.transparent)?),
        Format::Mp4 => {
            let ffmpeg = find_ffmpeg(std::env::var_os("MURMEL_FFMPEG"), std::env::var_os("PATH"))?;
            Box::new(Ffmpeg::start(&ffmpeg, &i.path, width, height, fps)?)
        }
    };
    let keep = (s.camera.clone(), s.display.clips.clone(), s.theme);
    if let Some(t) = i.theme {
        s.theme = t;
    }
    let made = render(s, &frames, &look, sink.as_mut(), &p);
    // The movie changes nothing: the camera, the planes and the theme go
    // back as they were.
    s.camera = keep.0;
    s.theme = keep.2;
    if s.display.clips != keep.1 {
        s.display.clips = keep.1;
        s.redraw();
    }
    let adapter = match made {
        Ok(a) => a,
        Err(mut d) => {
            if format == Format::Png {
                d.message = format!(
                    "{}; {} frames were written to {}",
                    d.message,
                    p.done(),
                    i.path.display()
                );
            }
            sink.abandon();
            return Err(d);
        }
    };
    let encoder = sink.finish()?;
    // A GIF plays at whole hundredths of a second a frame.
    let played = match format {
        Format::Gif => 100.0 / gif_delay(fps) as f64,
        _ => fps,
    };
    Ok(json!({
        "files": [i.path],
        "format": format.name(),
        "encoder": encoder,
        "frames": frames.len(),
        "fps": played,
        "seconds": frames.len() as f64 / played,
        "motion": motion,
        "width": width,
        "height": height,
        "scale": scale,
        "line_width": look.line_width,
        "transparent": i.transparent,
        "legend": i.legend,
        "axes": i.axes,
        "adapter": adapter,
        "camera": camera_json(s),
    }))
}

pub fn export_movie_schema() -> Value {
    let max = murmel_render::MAX_SIDE;
    object(
        json!({
            "path": {"type": "string", "description": "The movie to write: a .mp4 or .gif file, or a folder for PNG frames (frame-00001.png, …)"},
            "format": {"enum": ["mp4", "gif", "png"], "description": "Default: from the path (.mp4, .gif), else PNG frames. MP4 needs ffmpeg installed (brew install ffmpeg)"},
            "motion": {"enum": ["orbit", "clip", "views"], "description": "orbit: the camera turns about the vertical axis through its target; clip: a clip plane sweeps through the mesh; views: the camera flies through saved views, easing into each"},
            "seconds": {"type": "number", "exclusiveMinimum": 0, "maximum": MAX_SECONDS, "default": SECONDS, "description": "orbit and clip: the length of the movie"},
            "fps": {"type": "number", "minimum": 1, "maximum": 60, "default": 30, "description": "Frames a second (a GIF plays at most 50)"},
            "degrees": {"type": "number", "minimum": -3600, "maximum": 3600, "default": 360, "description": "orbit: the angle to turn; a whole turn loops without a stutter"},
            "plane": {"type": "integer", "minimum": 1, "description": "clip: the plane to sweep, counted from 1 (default: the current one)"},
            "from": {"type": "number", "description": "clip: the first offset along the plane's normal from the centre of the mesh's bounds (default: the end where the plane cuts nothing)"},
            "to": {"type": "number", "description": "clip: the last offset (default: the end where the plane cuts everything)"},
            "views": {"type": "array", "items": {"type": "string"}, "minItems": 2, "description": "views: the saved views to fly through, in order (default: all, in the order they were saved)"},
            "hold": {"type": "number", "minimum": 0, "maximum": MAX_STEP, "default": HOLD, "description": "views: seconds the camera stays at each view"},
            "travel": {"type": "number", "exclusiveMinimum": 0, "maximum": MAX_STEP, "default": TRAVEL, "description": "views: seconds of each flight from one view to the next"},
            "width": {"type": "integer", "minimum": 1, "maximum": max, "description": "Pixels, as in export.image (an MP4 makes odd sides even)"},
            "height": {"type": "integer", "minimum": 1, "maximum": max, "description": "Pixels, as in export.image"},
            "scale": {"type": "number", "minimum": export::SCALE.0, "maximum": export::SCALE.1, "description": "Pixels per point of the window, as in export.image"},
            "transparent": {"type": "boolean", "default": false, "description": "Leave the background out (GIF and PNG frames only)"},
            "legend": {"type": "boolean", "default": true, "description": "Draw the colour legend (top left)"},
            "axes": {"type": "boolean", "default": true, "description": "Draw the axis triad (bottom left)"},
            "theme": {"enum": ["dark", "light", "system"]},
            "line_width": {"type": "number", "minimum": 1, "maximum": 32, "description": "Edge width in pixels (default: the scale)"}
        }),
        &["path", "motion"],
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Invariant: the format follows the path's extension (any case), a
    /// path without one is a folder of frames; GIF delays are whole
    /// hundredths, at least two. Breaks if `.MP4` became a folder or a
    /// 60 fps GIF a delay of 1 (which players slow down to 10).
    #[test]
    fn formats_and_gif_delays() {
        assert_eq!(Format::of(Path::new("a/b.mp4")), Format::Mp4);
        assert_eq!(Format::of(Path::new("B.GIF")), Format::Gif);
        assert_eq!(Format::of(Path::new("frames")), Format::Png);
        assert_eq!(gif_delay(30.0), 3);
        assert_eq!(gif_delay(25.0), 4);
        assert_eq!(gif_delay(60.0), 2);
        assert_eq!(gif_delay(1.0), 100);
    }

    /// Invariant: MURMEL_FFMPEG wins and must name a file (EXPORT002
    /// otherwise, never a search elsewhere); without it the PATH is
    /// searched. Breaks if a wrong MURMEL_FFMPEG fell back silently.
    #[test]
    fn finds_ffmpeg() {
        let dir = std::env::temp_dir().join(format!("murmel-ffmpeg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        });
        std::fs::write(&exe, b"").unwrap();
        let missing = dir.join("nothing-here");
        assert_eq!(
            find_ffmpeg(Some(missing.into()), Some(dir.clone().into()))
                .unwrap_err()
                .code,
            "EXPORT002"
        );
        assert_eq!(find_ffmpeg(None, Some(dir.clone().into())).unwrap(), exe);
        assert_eq!(find_ffmpeg(Some(exe.clone().into()), None).unwrap(), exe);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stand-in for ffmpeg: it lists one encoder, then runs `body`
    /// with `$out` the file it was asked to write.
    #[cfg(unix)]
    fn fake_ffmpeg(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let exe = dir.join(name);
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in *-encoders*) echo ' V..... mpeg4  MPEG-4 part 2'; exit 0;; esac\nfor a; do out=$a; done\nout=${{out#file:}}\n{body}\n"
        );
        std::fs::write(&exe, script).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        exe
    }

    /// Invariant: the frames reach ffmpeg whole and in order, its encoder
    /// is reported, and when it fails the call says what it printed and
    /// leaves no half-written file. Runs everywhere with a stand-in for
    /// ffmpeg; breaks when frames are lost in the pipe or a failure of
    /// ffmpeg passes as success.
    #[cfg(unix)]
    #[test]
    fn frames_reach_ffmpeg() {
        let dir = std::env::temp_dir().join(format!("murmel-fake-ffmpeg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("m.mp4");
        let good = fake_ffmpeg(&dir, "good", "cat > \"$out\"");
        let mut f = Box::new(Ffmpeg::start(&good, &out, 4, 2, 30.0).unwrap());
        let frame = |k: u8| vec![k; 4 * 2 * 4];
        f.frame(&frame(1)).unwrap();
        f.frame(&frame(2)).unwrap();
        assert_eq!(f.finish().unwrap(), "mpeg4");
        assert_eq!(std::fs::read(&out).unwrap(), [frame(1), frame(2)].concat());

        let bad = fake_ffmpeg(
            &dir,
            "bad",
            "cat > \"$out\"; echo 'Unknown encoder' >&2; exit 1",
        );
        let mut f = Box::new(Ffmpeg::start(&bad, &out, 4, 2, 30.0).unwrap());
        f.frame(&frame(1)).unwrap();
        let d = f.finish().unwrap_err();
        assert_eq!(d.code, "EXPORT003");
        assert!(d.message.contains("Unknown encoder"), "{}", d.message);
        assert!(!out.exists(), "a failed MP4 stays");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Invariant: the MP4 is H.264 in yuv420p tagged BT.709 with the index
    /// at the front, the frames read as raw RGBA of the picture's size,
    /// and the output named with "file:". Breaks when an argument goes
    /// that QuickTime or browsers need.
    #[test]
    fn ffmpeg_arguments() {
        let a: Vec<String> = ffmpeg_args(Path::new("/tmp/m.mp4"), 640, 360, 30.0, "libx264")
            .into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        let after = |k: &str| a[a.iter().position(|x| x == k).unwrap() + 1].clone();
        assert_eq!(after("-pix_fmt"), "rgba");
        assert_eq!(after("-s"), "640x360");
        assert_eq!(after("-framerate"), "30");
        assert_eq!(after("-c:v"), "libx264");
        assert!(after("-vf").ends_with("format=yuv420p"));
        assert_eq!(after("-movflags"), "+faststart");
        assert_eq!(a.last().unwrap(), "file:/tmp/m.mp4");
    }
}
