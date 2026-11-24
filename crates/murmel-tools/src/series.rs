//! The tools of time series (D268, first milestone): `series.open`,
//! `series.go` and `series.play`. The series is read by
//! [`murmel_engine::series`]; a step opens its fields file with
//! [`Session::open_step`] (the display and the camera stay) and its
//! parcel file with [`crate::parcel::open_parcels`] (the parcel settings
//! stay).

use std::path::PathBuf;

use murmel_engine::Session;
use murmel_engine::series::{MAX_RATE, MIN_RATE, Series};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{display_json, object, parse};
use crate::{Diagnostic, Run};

fn no_series() -> Diagnostic {
    Diagnostic::new(
        "SERIES002",
        "no time series is open; open a series.json or an output folder with series.open",
    )
}

fn bad(m: String) -> Diagnostic {
    Diagnostic::new("SERIES003", m)
}

/// The series as tools report it (null without one).
pub(crate) fn series_json(s: &Session) -> Value {
    let Some(r) = &s.series else {
        return Value::Null;
    };
    let e = r.entry();
    let first = &r.entries[0];
    let last = &r.entries[r.len() - 1];
    json!({
        "path": r.path,
        "source": r.source.key(),
        "case": r.case,
        "count": r.len(),
        "index": r.current,
        "step": e.step,
        "time": shown_time(s),
        "files": {"fields": e.fields, "parcels": e.parcels},
        "steps": [first.step, last.step],
        "times": r.times().map(|(a, b)| [a, b]),
        "playback": {"playing": r.playback.playing, "rate": r.playback.rate, "loop": r.playback.looping},
    })
}

/// The time of the step shown: the manifest's, else the parcel file's,
/// else the fields file's.
pub fn shown_time(s: &Session) -> Option<f64> {
    let r = s.series.as_ref()?;
    let e = r.entry();
    e.time
        .or_else(|| {
            let p = s.display.parcel_data.as_ref()?;
            let v = s.display.parcels.as_ref()?;
            (Some(&v.path) == e.parcels.as_ref())
                .then_some(p.time)
                .flatten()
        })
        .or_else(|| {
            let m = s.mesh.as_ref()?;
            (Some(&m.path) == e.fields.as_ref())
                .then_some(m.fields.results.time)
                .flatten()
        })
}

fn done(s: &Session) -> Run {
    let mut out = display_json(s);
    out["series"] = series_json(s);
    out["parcels"] = crate::parcel::parcels_json(s);
    if let Some(m) = &s.mesh {
        out["mesh"] = json!(m.path);
    }
    Ok(out)
}

/// Show entry `k` of the open series: its parcel file (when it has one;
/// a step without keeps the parcel settings and draws none) and its
/// fields file (when it has one; a step without keeps the mesh shown).
pub fn go_to(s: &mut Session, k: usize) -> Result<(), Diagnostic> {
    let Some(r) = s.series.as_mut() else {
        return Err(no_series());
    };
    let Some(e) = r.entries.get(k).cloned() else {
        return Err(bad(format!(
            "index {k} is outside the series' 0 to {}",
            r.len() - 1
        )));
    };
    r.current = k;
    // With no mesh open, a step without a fields file shows the mesh of
    // the nearest step that has one (after it, else before).
    let near = r.entries[k..]
        .iter()
        .chain(r.entries[..k].iter().rev())
        .find_map(|e| e.fields.clone());
    let series = s.series.take();
    let made = step(s, &e, near);
    s.series = series;
    made
}

fn step(
    s: &mut Session,
    e: &murmel_engine::series::Entry,
    near: Option<PathBuf>,
) -> Result<(), Diagnostic> {
    let mesh_err = |e: murmel_mesh::MeshError| Diagnostic::new(e.code, e.message);
    if s.mesh.is_none() {
        let Some(f) = e.fields.clone().or(near) else {
            return Err(Diagnostic::new(
                "VIEW001",
                "the series has no fields file and no mesh is open; open the mesh first",
            ));
        };
        s.open(&f).map_err(mesh_err)?;
        if let Some(p) = &e.parcels {
            crate::parcel::open_parcels(s, p.clone())?;
            s.redraw();
        }
        return Ok(());
    }
    // The parcels first, so that a fields file of the same mesh draws
    // them in the same build.
    let parcels_before = s.display.parcels.is_some();
    match &e.parcels {
        Some(p) => crate::parcel::open_parcels(s, p.clone())?,
        None => s.display.parcel_data = None,
    }
    let fields = e
        .fields
        .as_ref()
        .filter(|f| s.mesh.as_ref().is_some_and(|m| &m.path != *f));
    match fields {
        Some(f) => {
            s.open_step(f).map_err(mesh_err)?;
            // A mesh of other cells opens afresh, without the parcels.
            if s.display.parcels.is_none() && (parcels_before || e.parcels.is_some()) {
                if let Some(p) = &e.parcels {
                    crate::parcel::open_parcels(s, p.clone())?;
                }
                s.redraw();
            }
        }
        None => s.redraw(),
    }
    Ok(())
}

/// Advance the player one step: the next entry, or stop at the end of a
/// series that does not loop. False when it stopped.
pub fn tick(s: &mut Session) -> Result<bool, Diagnostic> {
    let Some(r) = s.series.as_mut() else {
        return Ok(false);
    };
    match r.next_played() {
        Some(k) => {
            go_to(s, k)?;
            Ok(true)
        }
        None => {
            r.playback.playing = false;
            Ok(false)
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenInput {
    path: PathBuf,
    #[serde(default)]
    step: Option<u64>,
}

pub(crate) fn series_open(input: Value, s: &mut Session) -> Run {
    let i: OpenInput = parse(input)?;
    let r = Series::read(&i.path).map_err(|e| Diagnostic::new("SERIES001", e.0))?;
    let k = match i.step {
        Some(st) => r.index_of_step(st).ok_or_else(|| missing_step(&r, st))?,
        None => 0,
    };
    let keep = s.series.replace(r);
    let opened = i.path.clone();
    if let Err(d) = go_to(s, k) {
        s.series = keep;
        return Err(d);
    }
    // The charts of the run's tables, when its folder has them (D268).
    crate::charts::open_beside(s, &opened);
    let mut out = done(s)?;
    out["charts"] = crate::charts::charts_json(s);
    Ok(out)
}

fn missing_step(r: &Series, st: u64) -> Diagnostic {
    let steps: Vec<String> = r
        .entries
        .iter()
        .take(12)
        .map(|e| e.step.to_string())
        .collect();
    let more = if r.len() > 12 { ", …" } else { "" };
    bad(format!(
        "the series has no step {st}; it has {}{more}",
        steps.join(", ")
    ))
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum End {
    First,
    Last,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoInput {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    step: Option<u64>,
    #[serde(default)]
    time: Option<f64>,
    #[serde(default)]
    by: Option<i64>,
    #[serde(default)]
    to: Option<End>,
}

pub(crate) fn series_go(input: Value, s: &mut Session) -> Run {
    let i: GoInput = parse(input)?;
    let r = s.series.as_ref().ok_or_else(no_series)?;
    let given = [
        i.index.is_some(),
        i.step.is_some(),
        i.time.is_some(),
        i.by.is_some(),
        i.to.is_some(),
    ];
    if given.iter().filter(|g| **g).count() != 1 {
        return Err(Diagnostic::new(
            "TOOL002",
            "give exactly one of index, step, time, by and to",
        ));
    }
    let k = if let Some(k) = i.index {
        if k >= r.len() {
            return Err(bad(format!(
                "index {k} is outside the series' 0 to {}",
                r.len() - 1
            )));
        }
        k
    } else if let Some(st) = i.step {
        r.index_of_step(st).ok_or_else(|| missing_step(r, st))?
    } else if let Some(t) = i.time {
        if !t.is_finite() {
            return Err(Diagnostic::new("TOOL002", "time must be finite"));
        }
        r.index_near_time(t).ok_or_else(|| {
            bad("the series' manifest gives no times; go by index, step or by".into())
        })?
    } else if let Some(b) = i.by {
        r.index_by(b)
    } else {
        match i.to {
            Some(End::First) => 0,
            _ => r.len() - 1,
        }
    };
    go_to(s, k)?;
    done(s)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayInput {
    #[serde(default)]
    playing: Option<bool>,
    #[serde(default)]
    rate: Option<f64>,
    #[serde(default, rename = "loop")]
    looping: Option<bool>,
}

pub(crate) fn series_play(input: Value, s: &mut Session) -> Run {
    let i: PlayInput = parse(input)?;
    let r = s.series.as_mut().ok_or_else(no_series)?;
    if let Some(x) = i.rate {
        if !(x.is_finite() && (MIN_RATE..=MAX_RATE).contains(&x)) {
            return Err(bad(format!(
                "rate {x} is outside {MIN_RATE} to {MAX_RATE} steps a second"
            )));
        }
        r.playback.rate = x;
    }
    if let Some(l) = i.looping {
        r.playback.looping = l;
    }
    if let Some(p) = i.playing {
        r.playback.playing = p;
    }
    done(s)
}

// ---- schemas ------------------------------------------------------------

pub(crate) fn series_open_schema() -> Value {
    object(
        json!({
            "path": {"type": "string", "description": "FoehnCraft's series.json, or an output folder (its series.json, else its fields_<step>.vtkhdf and spray/parcels_<step>.vtkhdf)"},
            "step": {"type": "integer", "minimum": 0, "description": "The step to show first (default: the first)"}
        }),
        &["path"],
    )
}

pub(crate) fn series_go_schema() -> Value {
    object(
        json!({
            "index": {"type": "integer", "minimum": 0, "description": "The entry, counted from 0"},
            "step": {"type": "integer", "minimum": 0, "description": "The time step"},
            "time": {"type": "number", "description": "The entry nearest this time (seconds; needs the manifest's times)"},
            "by": {"type": "integer", "description": "Entries on (negative: back); wraps round while the player loops"},
            "to": {"enum": ["first", "last"]}
        }),
        &[],
    )
}

pub(crate) fn series_play_schema() -> Value {
    object(
        json!({
            "playing": {"type": "boolean", "description": "Play or pause (the window plays; the command line steps with series.go)"},
            "rate": {"type": "number", "minimum": MIN_RATE, "maximum": MAX_RATE, "description": "Steps a second"},
            "loop": {"type": "boolean", "description": "Start again at the first step after the last"}
        }),
        &[],
    )
}
