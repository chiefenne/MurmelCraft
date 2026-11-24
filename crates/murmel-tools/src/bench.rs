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
use murmel_mesh::synth::{self, Encoding};
use serde_json::{Value, json};

pub const BENCH_FORMAT: &str = "murmelcraft.bench";

/// The file formats `make` writes.
pub const FORMATS: [&str; 4] = ["vtu", "vtu-binary", "polymesh", "polymesh-binary"];

/// Write the honeycomb of at least `cells` cells into `out` in each of
/// `formats` (names from [`FORMATS`]).
pub fn make(cells: usize, out: &Path, formats: &[String]) -> Result<Value, String> {
    for f in formats {
        if !FORMATS.contains(&f.as_str()) {
            return Err(format!("unknown format {f}; one of {}", FORMATS.join(", ")));
        }
    }
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let t = Instant::now();
    let n = synth::n_for_cells(cells);
    let mesh = synth::honeycomb(n);
    let built = ms(t);
    let mut written = Vec::new();
    for f in formats {
        let t = Instant::now();
        let (path, result) = match f.as_str() {
            "vtu" | "vtu-binary" => {
                let enc = if f == "vtu" {
                    Encoding::Ascii
                } else {
                    Encoding::Binary
                };
                let p = out.join(format!("{f}/foehncraft.vtu"));
                let r = std::fs::create_dir_all(out.join(f))
                    .and_then(|_| synth::write_vtu(&mesh, &p, enc));
                (p, r)
            }
            _ => {
                let enc = if f == "polymesh" {
                    Encoding::Ascii
                } else {
                    Encoding::Binary
                };
                let p = out.join(format!("{f}/constant/polyMesh"));
                let r = synth::write_polymesh(&mesh, &p, enc);
                (p, r)
            }
        };
        result.map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(json!({
            "format": f,
            "path": path.display().to_string(),
            "bytes": bytes_of(&path),
            "write_ms": ms(t),
        }));
    }
    Ok(json!({
        "format": BENCH_FORMAT,
        "format_version": "1.0",
        "kind": "make",
        "generator": format!("honeycomb n={n}"),
        "cells": mesh.cell_count(),
        "points": mesh.points.len(),
        "faces": mesh.unique_faces,
        "boundary_faces": mesh.boundary.len(),
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
}

impl Default for Options {
    fn default() -> Self {
        Self {
            repeat: 1,
            render: true,
            size: [1280, 720],
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
/// tells it (Linux `/proc/self/status` `VmHWM`).
pub fn peak_rss_mib() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: f64 = s
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((kb / 1024.0 * 10.0).round() / 10.0)
}

fn mib(bytes: usize) -> f64 {
    (bytes as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0
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
            .map(|f| murmel_engine::session::sha256(f).len())
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
        let s = st.time(name, r, || Surface::build(&mesh, &tokens, &d));
        st.note(surface_note(&s));
    }
    st.time("quality", r, || Quality::compute(&mesh));

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
            Ok::<String, murmel_render::RenderError>(first)
        })
        .and_then(|x| x);
        match out {
            Ok(a) => adapter = json!(a),
            Err(e) => adapter = json!(format!("not rendered: {e}")),
        }
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

    /// Invariant: `make` writes every format and `run` reads each back to
    /// the same cell count and reports every stage with a time. A stage
    /// that failed silently, or a format the readers refuse, would leave
    /// the benchmark's numbers meaningless.
    #[test]
    fn make_and_run_every_format() {
        let dir = std::env::temp_dir().join(format!("murmel-bench-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let formats: Vec<String> = FORMATS.iter().map(|s| s.to_string()).collect();
        let made = make(500, &dir, &formats).unwrap();
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
            assert!(!table(&report).is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
