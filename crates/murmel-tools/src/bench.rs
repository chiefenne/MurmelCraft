//! `murmel bench`: timings of every stage a large mesh goes through, so
//! performance work is measured before and after (docs/performance.md).
//!
//! `make` writes the synthetic honeycomb of `murmel_mesh::synth` in the
//! formats the viewer reads; `run` opens a mesh and times reading,
//! hashing, opening as the window does (the files hashed afterwards), the
//! surface of several displays, cell quality, picking and the first and a
//! later offscreen frame, and reports the peak memory.
//! The report is JSON (`murmelcraft.bench`), so runs can be compared.

use std::path::{Path, PathBuf};
use std::time::Instant;

use murmel_engine::{
    Camera, Clip, ClipMode, ColorBy, Display, Item, Quality, RenderMode, Session, StandardView,
    Surface, Tokens,
};
use murmel_mesh::Mesh;
use murmel_mesh::synth::blocked::{self, LocalIds, Progress};
use murmel_mesh::synth::{self, Encoding};
use serde_json::{Value, json};

pub const BENCH_FORMAT: &str = "murmelcraft.bench";

/// The file formats `make` writes. `blocked` is a blocked VTKHDF file
/// with 32-bit block ids and two cell fields, written block by block
/// without the mesh in memory (`murmel_mesh::synth::blocked`), so it is
/// the one format for meshes beyond memory.
pub const FORMATS: [&str; 5] = [
    "vtu",
    "vtu-binary",
    "polymesh",
    "polymesh-binary",
    "blocked",
];

/// Write the honeycomb of at least `cells` cells into `out` in each of
/// `formats` (names from [`FORMATS`]); the blocked writer reports its
/// steps to `progress`.
pub fn make(
    cells: usize,
    out: &Path,
    formats: &[String],
    progress: Progress<'_>,
) -> Result<Value, String> {
    for f in formats {
        if !FORMATS.contains(&f.as_str()) {
            return Err(format!("unknown format {f}; one of {}", FORMATS.join(", ")));
        }
    }
    let n = synth::n_for_cells(cells);
    if n > blocked::MAX_N {
        return Err(format!(
            "at most {} cells (n = {})",
            synth::cells_of(blocked::MAX_N),
            blocked::MAX_N
        ));
    }
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    // the mesh in memory only for the formats written from it
    let t = Instant::now();
    let mesh = formats
        .iter()
        .any(|f| f != "blocked")
        .then(|| synth::honeycomb(n));
    let built = ms(t);
    let mut written = Vec::new();
    let mut points = mesh.as_ref().map(|m| m.points.len());
    for f in formats {
        let t = Instant::now();
        let mut blocks = Value::Null;
        let (path, result) = match (f.as_str(), &mesh) {
            ("blocked", _) => {
                let p = out.join("blocked/honeycomb.vtkhdf");
                let r = std::fs::create_dir_all(out.join(f))
                    .map_err(|e| e.to_string())
                    .and_then(|_| {
                        blocked::write_blocked(
                            &p,
                            n,
                            murmel_mesh::blocks::DEFAULT_BLOCK_CELLS,
                            LocalIds::I32,
                            progress,
                        )
                        .map_err(|e| e.message)
                    });
                if let Ok(w) = &r {
                    points = Some(w.points);
                    blocks = json!({
                        "blocks": w.blocks,
                        "block_cells": w.block_cells,
                        "ghost_cells": w.ghost_cells,
                        "block_points": w.block_points,
                    });
                }
                (p, r.map(|_| ()))
            }
            (_, None) => continue,
            ("vtu" | "vtu-binary", Some(mesh)) => {
                let enc = if f == "vtu" {
                    Encoding::Ascii
                } else {
                    Encoding::Binary
                };
                let p = out.join(format!("{f}/foehncraft.vtu"));
                let r = std::fs::create_dir_all(out.join(f))
                    .and_then(|_| synth::write_vtu(mesh, &p, enc))
                    .map_err(|e| e.to_string());
                (p, r)
            }
            (_, Some(mesh)) => {
                let enc = if f == "polymesh" {
                    Encoding::Ascii
                } else {
                    Encoding::Binary
                };
                let p = out.join(format!("{f}/constant/polyMesh"));
                let r = synth::write_polymesh(mesh, &p, enc).map_err(|e| e.to_string());
                (p, r)
            }
        };
        result.map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(json!({
            "format": f,
            "path": path.display().to_string(),
            "bytes": bytes_of(&path),
            "write_ms": ms(t),
            "blocks": blocks,
        }));
    }
    Ok(json!({
        "format": BENCH_FORMAT,
        "format_version": "1.0",
        "kind": "make",
        "generator": format!("honeycomb n={n}"),
        "cells": synth::cells_of(n),
        "points": points,
        "faces": mesh.as_ref().map(|m| m.unique_faces),
        "boundary_faces": mesh.as_ref().map(|m| m.boundary.len()),
        "build_ms": built,
        "files": written,
    }))
}

/// How `run` measures.
#[derive(Clone, Debug)]
pub struct Options {
    /// Runs of each stage; the report gives each time and the median.
    pub repeat: usize,
    /// Draw offscreen frames (needs a GPU or a software driver).
    pub render: bool,
    pub size: [u32; 2],
    /// Time the field stages too ([`fields`]).
    pub fields: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            repeat: 1,
            render: true,
            size: [1280, 720],
            fields: true,
        }
    }
}

fn ms(t: Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 1e5).round() / 100.0
}

fn bytes_of(path: &Path) -> u64 {
    if path.is_dir() {
        std::fs::read_dir(path)
            .map(|d| {
                d.flatten()
                    .map(|e| e.metadata().map_or(0, |m| m.len()))
                    .sum()
            })
            .unwrap_or(0)
    } else {
        std::fs::metadata(path).map_or(0, |m| m.len())
    }
}

/// Stage timings: each run and the median, in milliseconds.
#[derive(Default)]
struct Stages(Vec<(String, Vec<f64>, Value)>);

impl Stages {
    fn time<R>(&mut self, name: &str, repeat: usize, mut f: impl FnMut() -> R) -> R {
        let mut runs = Vec::with_capacity(repeat);
        let mut last = None;
        for _ in 0..repeat.max(1) {
            let t = Instant::now();
            let r = f();
            runs.push(ms(t));
            last = Some(r);
        }
        self.0.push((name.into(), runs, Value::Null));
        match last {
            Some(r) => r,
            None => f(),
        }
    }

    fn note(&mut self, extra: Value) {
        if let Some(s) = self.0.last_mut() {
            s.2 = extra;
        }
    }

    fn to_json(&self) -> Value {
        Value::Array(
            self.0
                .iter()
                .map(|(name, runs, extra)| {
                    let mut sorted = runs.clone();
                    sorted.sort_by(f64::total_cmp);
                    let mut v = json!({
                        "stage": name,
                        "median_ms": sorted[sorted.len() / 2],
                        "runs_ms": runs,
                    });
                    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
                        o.extend(e.clone());
                    }
                    v
                })
                .collect(),
        )
    }
}

/// The peak resident memory of this process in MiB, where the system
/// tells it (`getrusage` on Linux and macOS; none on Windows).
#[cfg(unix)]
pub fn peak_rss_mib() -> Option<f64> {
    use nix::sys::resource::{UsageWho, getrusage};
    // macOS counts the peak in bytes, Linux in KiB.
    let unit = if cfg!(target_vendor = "apple") {
        1.0
    } else {
        1024.0
    };
    let peak = getrusage(UsageWho::RUSAGE_SELF).ok()?.max_rss() as f64 * unit;
    Some(mib_of(peak))
}

/// The peak resident memory of this process in MiB, where the system
/// tells it (`getrusage` on Linux and macOS; none on Windows).
#[cfg(not(unix))]
pub fn peak_rss_mib() -> Option<f64> {
    None
}

fn mib(bytes: usize) -> f64 {
    mib_of(bytes as f64)
}

fn mib_of(bytes: f64) -> f64 {
    (bytes / (1024.0 * 1024.0) * 10.0).round() / 10.0
}

/// The phases recorded since the last `profile::record(true)`, in
/// milliseconds, and recording turned off.
fn phases() -> Value {
    murmel_mesh::profile::record(false);
    let mut m = serde_json::Map::new();
    for (name, ms) in murmel_mesh::profile::take() {
        let v = m.entry(name).or_insert(json!(0.0));
        *v = json!(v.as_f64().unwrap_or(0.0) + (ms * 100.0).round() / 100.0);
    }
    Value::Object(m)
}

/// The bytes of the mesh's arrays.
pub fn mesh_bytes(m: &Mesh) -> usize {
    m.model_bytes()
}

/// The bytes of a surface's arrays (CPU side).
pub fn surface_bytes(s: &Surface) -> usize {
    s.positions.len() * (12 + 12 + 12 + 16)
        + s.indices.len() * 4
        + (s.tri_cell.len() + s.tri_face.len()) * 4
        + s.edge_positions.len() * 24
        + s.tri_boxes.len() * 24
        + s.outline.len() * 12
}

fn surface_note(s: &Surface) -> Value {
    json!({
        "triangles": s.triangles(),
        "vertices": s.positions.len(),
        "edges": s.edges(),
        "surface_mib": mib(surface_bytes(s)),
    })
}

/// Open `path` and time every stage (see the module docs).
pub fn run(path: &Path, opts: &Options) -> Result<Value, String> {
    let tokens = Tokens::builtin()?;
    let r = opts.repeat.max(1);
    let mut st = Stages::default();

    let (mesh, _) = st
        .time("read", r, || {
            murmel_mesh::profile::record(true);
            murmel_mesh::read_results(path)
        })
        .map_err(|e| e.to_string())?;
    // Reading is the first stage: the peak so far is the reader's.
    st.note(json!({
        "file_bytes": mesh.files.iter().map(|f| bytes_of(f)).sum::<u64>(),
        "phases": phases(),
        "peak_rss_mib": peak_rss_mib(),
    }));
    let files: Vec<PathBuf> = mesh.files.clone();
    st.time("digest", r, || {
        files
            .iter()
            .map(|f| murmel_engine::session::file_digest(murmel_mesh::FileHash::Blake3, f).len())
            .sum::<usize>()
    });
    let mut session = Session::new(tokens.clone());
    session.config_dir = None;
    // As the window opens: the files are hashed after the mesh opens, and
    // the wait for the digest from then on is a stage of its own.
    session.hash_after_open = true;
    st.time("open", r, || {
        murmel_mesh::profile::record(true);
        session.open(path).map(|_| ())
    })
    .map_err(|e| e.to_string())?;
    st.note(json!({ "phases": phases() }));
    st.time("open.digest", 1, || session.wait_digest());

    let d0 = Display::default();
    let s = st.time("surface", r, || {
        murmel_mesh::profile::record(true);
        Surface::build(&mesh, &tokens, &d0)
    });
    let mut note = surface_note(&s);
    note["phases"] = phases();
    st.note(note);
    let b = mesh.bounds();
    let clip = |mode| Display {
        clips: vec![Clip {
            origin: b.center(),
            normal: [1.0, 0.0, 0.0],
            mode,
            enabled: true,
        }],
        ..Display::default()
    };
    for (name, d) in [
        ("surface.crinkle", clip(ClipMode::Crinkle)),
        ("surface.flat", clip(ClipMode::Flat)),
        (
            "surface.hidden_zone",
            Display {
                hidden: [Item::Zone(0)].into_iter().collect(),
                ..Display::default()
            },
        ),
        (
            "surface.cell_type",
            Display {
                color_by: ColorBy::CellType,
                ..Display::default()
            },
        ),
        (
            "surface.feature_edges",
            Display {
                render_mode: RenderMode::FeatureEdges,
                ..Display::default()
            },
        ),
    ] {
        let s = st.time(name, r, || {
            murmel_mesh::profile::record(true);
            Surface::build(&mesh, &tokens, &d)
        });
        let mut note = surface_note(&s);
        note["phases"] = phases();
        st.note(note);
    }
    st.time("quality", r, || Quality::compute(&mesh));
    // What the model tree and the legend ask of the mesh on every frame
    // (counts, the items to list), after the first frame.
    let tree_frame = || mesh.summary().cells + murmel_engine::display::all_items(&mesh).len();
    tree_frame();
    st.time("tree.frame", r.max(5), tree_frame);

    // The clip drag as the window does it: each step moves the plane and
    // returns at once (the renderer cuts on the GPU); the exact crinkle
    // cut is built on a thread of its own once the plane rests. As
    // before, a step that builds the cut before it returns.
    let along = |f: f64| Clip {
        origin: [
            b.min[0] + f * (b.max[0] - b.min[0]),
            b.center()[1],
            b.center()[2],
        ],
        normal: [1.0, 0.0, 0.0],
        mode: ClipMode::Crinkle,
        enabled: true,
    };
    session.display.clips = vec![along(0.5)];
    session.redraw();
    session.defer_clips = true;
    let mut f = 0.5;
    st.time("clip.step", r.max(5), || {
        f += 0.01;
        session.display.clips = vec![along(f)];
        session.redraw();
    });
    st.time("clip.exact", 1, || {
        while session.poll_surface() {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    });
    session.defer_clips = false;
    st.time("clip.step.built", r, || {
        f -= 0.01;
        session.display.clips = vec![along(f)];
        session.redraw();
    });

    let mut cam = Camera::default();
    cam.set_standard(StandardView::ISO);
    let aspect = f64::from(opts.size[0]) / f64::from(opts.size[1].max(1));
    if let Some(bb) = s.bounds {
        cam.fit(&bb, aspect);
    }
    let grid: Vec<[f64; 2]> = (0..9)
        .map(|i| [f64::from(i % 3) * 0.5 - 0.5, f64::from(i / 3) * 0.5 - 0.5])
        .collect();
    let hits = st.time("pick.9", r, || {
        grid.iter()
            .filter(|&&p| murmel_engine::pick::pick(&s, &cam, p, aspect).is_some())
            .count()
    });
    st.note(json!({ "hits": hits }));

    let mut adapter = Value::Null;
    if opts.render {
        session.viewport = [f64::from(opts.size[0]), f64::from(opts.size[1])];
        session.fit();
        let shoot = murmel_render::Shoot::new(opts.size[0], opts.size[1]);
        let out = murmel_render::with_offscreen(|o| {
            let first = st.time("render.first", 1, || o.shoot(&session, &shoot));
            let first = first.map(|s| s.adapter)?;
            st.time("render.next", r, || o.shoot(&session, &shoot).map(|_| ()))?;
            // The same frame without the edges, as the window draws it
            // while the view moves when frames with edges are slow.
            let mode = session.display.render_mode;
            session.display.render_mode = RenderMode::Shaded;
            let faces = st.time("render.next.faces", r, || {
                o.shoot(&session, &shoot).map(|_| ())
            });
            session.display.render_mode = mode;
            faces?;
            Ok::<String, murmel_render::RenderError>(first)
        })
        .and_then(|x| x);
        match out {
            Ok(a) => adapter = json!(a),
            Err(e) => adapter = json!(format!("not rendered: {e}")),
        }
    }

    if opts.fields {
        fields(&mut session, &mut st, r)?;
    }

    Ok(json!({
        "format": BENCH_FORMAT,
        "format_version": "1.0",
        "kind": "run",
        "mesh": {
            "path": path.display().to_string(),
            "cells": mesh.cell_count(),
            "points": mesh.points.len(),
            "faces": mesh.unique_faces,
            "cell_faces": mesh.cell_face_id.len(),
            "face_nodes": mesh.distinct.nodes.len(),
            "boundary_faces": mesh.boundary.len(),
            "mesh_mib": mib(mesh_bytes(&mesh)),
        },
        "stages": st.to_json(),
        "memory": { "peak_rss_mib": peak_rss_mib() },
        "machine": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "threads": std::thread::available_parallelism().map_or(1, |n| n.get()),
            "adapter": adapter,
            "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        },
    }))
}

/// The tool calls of one run of a stage.
type Steps = Vec<(&'static str, Value)>;

/// Run tool `id` on `session`, its failure as the error.
fn tool(session: &mut Session, id: &str, input: Value) -> Result<Value, String> {
    let e = crate::call(id, input, session);
    if e.exit_code == 0 {
        Ok(e.result)
    } else {
        Err(format!("{id}: {:?}", e.diagnostics))
    }
}

/// The field stages: two fields of the cells (a scalar `p` and a
/// swirling vector `U`, functions of each cell's centroid, so every mesh,
/// the honeycomb included, has results), then colouring by them, a slice and its steps, an
/// iso-surface and its steps, and arrows on the slice, each through its
/// tool as an agent or the window calls it.
fn fields(session: &mut Session, st: &mut Stages, r: usize) -> Result<(), String> {
    session.display.clips.clear();
    session.redraw();
    let Some(m) = &session.mesh else {
        return Ok(());
    };
    let b = m.mesh.bounds();
    let (c, h) = (
        b.center(),
        [0, 1, 2].map(|k| 0.5 * (b.max[k] - b.min[k]).max(1e-30)),
    );
    // Coordinates scaled to -1..1 over the mesh's box.
    let made = st.time("field.make", 1, || {
        let mesh = &m.mesh;
        let unit = |q: [f64; 3]| [0, 1, 2].map(|k| (q[k] - c[k]) / h[k]);
        let at: Vec<[f64; 3]> = murmel_mesh::par::collect(mesh.cell_count(), 1 << 14, |r| {
            r.map(|i| unit(murmel_engine::inspect::cell_centroid(mesh, i)))
                .collect()
        });
        let p = at
            .iter()
            .map(|q| (3.0 * q[0]).sin() * (2.0 * q[1]).cos() + 0.5 * q[2])
            .collect();
        let u = at
            .iter()
            .flat_map(|q| [-q[1], q[0], 0.2 + 0.1 * q[2]])
            .collect();
        let field = |name: &str, components, values| murmel_mesh::field::Field {
            name: name.into(),
            unit: None,
            location: murmel_mesh::field::Location::Cell,
            components,
            values,
        };
        murmel_mesh::field::Results {
            fields: vec![field("p", 1, p), field("U", 3, u)],
            ..Default::default()
        }
    });
    if let Some(m) = &mut session.mesh {
        m.fields = std::sync::Arc::new(murmel_engine::FieldSet::new(made));
    }
    let stage = |st: &mut Stages,
                 session: &mut Session,
                 name: &str,
                 repeat: usize,
                 calls: &mut dyn FnMut(usize) -> Steps|
     -> Result<(), String> {
        let mut k = 0;
        let out = st.time(name, repeat, || -> Result<(), String> {
            murmel_mesh::profile::record(true);
            for (id, input) in calls(k) {
                tool(session, id, input)?;
            }
            k += 1;
            Ok(())
        });
        out?;
        let mut note = session
            .mesh
            .as_ref()
            .map_or(Value::Null, |m| surface_note(&m.surface));
        note["phases"] = phases();
        st.note(note);
        Ok(())
    };
    let by = |name: &str| json!({"by": "field", "field": name});
    // The first colouring makes the field's node values; colouring by it
    // again builds the surface alone.
    stage(st, session, "field.color", 1, &mut |_| {
        vec![("display.color_by", by("p"))]
    })?;
    stage(st, session, "field.switch", 1, &mut |_| {
        vec![("display.color_by", by("U"))]
    })?;
    stage(st, session, "field.color.again", r, &mut |k| {
        vec![("display.color_by", by(if k % 2 == 0 { "p" } else { "U" }))]
    })?;
    stage(st, session, "field.style", r.max(3), &mut |k| {
        let range = if k % 2 == 0 { "symmetric" } else { "field" };
        vec![("field.style", json!({"range": range}))]
    })?;
    tool(session, "display.color_by", by("p"))?;
    let at = |f: f64| [b.min[0] + f * (b.max[0] - b.min[0]), c[1], c[2]];
    stage(st, session, "slice.set", 1, &mut |_| {
        vec![(
            "slice.set",
            json!({"new": true, "axis": "x", "origin": at(0.5)}),
        )]
    })?;
    // A step of a slice drag, as the window takes it.
    stage(st, session, "slice.step", r.max(5), &mut |k| {
        let f = 0.5 + 0.01 * (k + 1) as f64;
        vec![("slice.set", json!({"origin": at(f)}))]
    })?;
    // The same steps in the window: the slice drawn from the field's
    // grid at once (its grid made on the first), the exact faces later
    // (crate::call settles the next other tool).
    session.defer_clips = true;
    // Timed here, not on the thread the window builds them on.
    session.preview_grids_wait = true;
    stage(st, session, "slice.preview", 1, &mut |_| {
        vec![("slice.set", json!({"origin": at(0.4)}))]
    })?;
    stage(st, session, "slice.preview.step", r.max(5), &mut |k| {
        let f = 0.4 + 0.01 * (k + 1) as f64;
        vec![("slice.set", json!({"origin": at(f)}))]
    })?;
    session.defer_clips = false;
    session.settle();
    stage(st, session, "iso.set", 1, &mut |_| {
        vec![("iso.set", json!({"new": true, "field": "p", "value": 0.25}))]
    })?;
    stage(st, session, "iso.step", r.max(3), &mut |k| {
        vec![("iso.set", json!({"value": 0.25 + 0.05 * (k + 1) as f64}))]
    })?;
    // The same values in the window: ray cast from the field's grid at
    // once (its grid made on the first), the exact surface later.
    session.defer_clips = true;
    stage(st, session, "iso.preview", 1, &mut |_| {
        vec![("iso.set", json!({"value": 0.2}))]
    })?;
    stage(st, session, "iso.preview.step", r.max(3), &mut |k| {
        vec![("iso.set", json!({"value": 0.2 + 0.05 * (k + 1) as f64}))]
    })?;
    session.defer_clips = false;
    session.settle();
    tool(session, "iso.clear", json!({"all": true}))?;
    stage(st, session, "arrows.set", 1, &mut |_| {
        vec![("arrows.set", json!({"field": "U"}))]
    })?;
    stage(st, session, "slice.step.arrows", r.max(3), &mut |k| {
        let f = 0.45 - 0.01 * (k + 1) as f64;
        vec![("slice.set", json!({"origin": at(f)}))]
    })?;
    Ok(())
}

/// A short table of a `run` report for the terminal.
pub fn table(report: &Value) -> String {
    let mut out = String::new();
    let m = &report["mesh"];
    out.push_str(&format!(
        "{} cells, {} points, {} faces, {} boundary faces, mesh {} MiB\n",
        m["cells"], m["points"], m["faces"], m["boundary_faces"], m["mesh_mib"]
    ));
    for s in report["stages"].as_array().into_iter().flatten() {
        let extra = s
            .get("triangles")
            .map(|t| format!("  {t} triangles"))
            .unwrap_or_default();
        out.push_str(&format!(
            "  {:<24} {:>10.1} ms{extra}\n",
            s["stage"].as_str().unwrap_or(""),
            s["median_ms"].as_f64().unwrap_or(0.0)
        ));
        for (name, ms) in s
            .get("phases")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            out.push_str(&format!(
                "    {name:<22} {:>10.1} ms\n",
                ms.as_f64().unwrap_or(0.0)
            ));
        }
    }
    out.push_str(&format!(
        "  peak memory {} MiB\n",
        report["memory"]["peak_rss_mib"]
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: on Linux and macOS the bench reports the peak memory in
    /// MiB, at least what the process touched. Linux counts the peak in
    /// KiB and macOS in bytes; taking one unit for the other makes the
    /// peak 1024 times too small (below the 64 MiB touched here) or too
    /// large (64 GiB or more). No other test reads the peak's value.
    #[cfg(unix)]
    #[test]
    fn peak_memory_counts_what_the_process_touched() {
        let touched = vec![1u8; 64 << 20];
        std::hint::black_box(&touched);
        let peak = peak_rss_mib().unwrap_or(0.0);
        assert!((64.0..64.0 * 1024.0).contains(&peak), "peak {peak} MiB");
    }

    /// Invariant: `make` writes every format and `run` reads each back to
    /// the same cell count and reports every stage with a time, the field
    /// stages (made through their tools) drawing something. A stage
    /// that failed silently, or a format the readers refuse, would leave
    /// the benchmark's numbers meaningless.
    #[test]
    fn make_and_run_every_format() {
        let dir = std::env::temp_dir().join(format!("murmel-bench-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let formats: Vec<String> = FORMATS.iter().map(|s| s.to_string()).collect();
        let made = make(500, &dir, &formats, &|_, _, _| {}).unwrap();
        let cells = made["cells"].as_u64().unwrap();
        assert!(cells >= 500);
        for f in made["files"].as_array().unwrap() {
            let p = PathBuf::from(f["path"].as_str().unwrap());
            let report = run(
                &p,
                &Options {
                    render: false,
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(report["mesh"]["cells"].as_u64(), Some(cells), "{f}");
            let stages = report["stages"].as_array().unwrap();
            assert!(stages.len() >= 9);
            assert!(stages.iter().all(|s| s["median_ms"].as_f64().is_some()));
            // the field stages ran through their tools
            for name in ["field.color", "slice.step", "iso.step", "slice.step.arrows"] {
                let st = stages.iter().find(|s| s["stage"] == name);
                assert!(
                    st.is_some_and(|s| s["triangles"].as_u64() > Some(0)),
                    "{name}"
                );
            }
            assert!(!table(&report).is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
