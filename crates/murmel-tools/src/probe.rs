//! The tools of probe histories (the maintainer's card of 2026-11-24,
//! "Follow steps, plus history plot"): `probe.history` reads every
//! probe's field values at every step of the open series
//! ([`murmel_engine::history`]) and `probe.export` writes them as CSV.
//! The window reads a history on a copy of the session, on a thread of
//! its own ([`with_progress`] counts its steps and takes its Cancel); the
//! copies share the history read last, so the export after a plot reads
//! nothing again.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use murmel_engine::Session;
use murmel_engine::display::Item;
use murmel_engine::history::{self, Failure, History, Point, Progress};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{need_mesh, object, parse};
use crate::{Diagnostic, Run};

thread_local! {
    static PROGRESS: RefCell<Option<Arc<Progress>>> = const { RefCell::new(None) };
}

/// Clears the progress of this thread when the call ends.
struct Reset;

impl Drop for Reset {
    fn drop(&mut self) {
        PROGRESS.with(|c| *c.borrow_mut() = None);
    }
}

/// Run `f` (a call of `probe.history` or `probe.export` on this thread)
/// with `p` counting its steps and taking its Cancel.
pub fn with_progress<R>(p: Arc<Progress>, f: impl FnOnce() -> R) -> R {
    PROGRESS.with(|c| *c.borrow_mut() = Some(p));
    let _reset = Reset;
    f()
}

fn progress() -> Arc<Progress> {
    PROGRESS.with(|c| c.borrow().clone()).unwrap_or_default()
}

/// The probes inside the mesh, in the tree's order.
pub fn points(s: &Session) -> Vec<Point> {
    s.display
        .probes
        .iter()
        .filter_map(|p| {
            Some(Point {
                n: p.n,
                cell: p.at.cell?,
                point: p.at.point,
            })
        })
        .collect()
}

/// What the history of the open mesh's probes is read of
/// ([`history::key`]); `None` without a mesh.
pub fn key_of(s: &Session) -> Option<String> {
    let m = s.mesh.as_ref()?;
    Some(history::key(
        &m.path,
        &m.fields,
        s.series.as_ref(),
        &points(s),
    ))
}

/// The history kept for the open mesh's probes, if it is still theirs.
pub fn kept(s: &Session) -> Option<Arc<History>> {
    history::cached(&s.probe_history, &key_of(s)?)
}

/// The history of the open mesh's probes: the one kept when nothing it
/// was read of changed, else read now (and kept).
pub fn history_of(s: &Session) -> Result<Arc<History>, Diagnostic> {
    need_mesh(s)?;
    let Some(m) = &s.mesh else {
        return Err(Diagnostic::new("VIEW001", "no mesh is open"));
    };
    let pts = points(s);
    let key = history::key(&m.path, &m.fields, s.series.as_ref(), &pts);
    if let Some(h) = history::cached(&s.probe_history, &key) {
        return Ok(h);
    }
    let h = history::read(&m.mesh, &m.fields, s.series.as_ref(), &pts, &progress()).map_err(
        |f| match f {
            Failure::Nothing(why) => Diagnostic::new("PROBE001", why),
            Failure::Cancelled => Diagnostic::new("PROBE002", "the history was cancelled"),
        },
    )?;
    let h = Arc::new(h);
    history::keep(&s.probe_history, key, h.clone());
    Ok(h)
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Pick {
    /// Probe ids (`probe:<n>`); all inside the mesh when left out.
    probes: Option<Vec<String>>,
    /// Field names; all cell and node fields when left out.
    fields: Option<Vec<String>>,
}

/// `h` with only the probes and fields `pick` names; refused for a
/// probe or field the history lacks.
fn picked(s: &Session, h: &History, pick: &Pick) -> Result<History, Diagnostic> {
    let mut out = h.clone();
    if let Some(ids) = &pick.probes {
        let mut keep = Vec::new();
        for id in ids {
            let n = match s.mesh.as_ref().and_then(|m| Item::parse(id, &m.mesh)) {
                Some(Item::Probe(n)) => n,
                _ => {
                    return Err(Diagnostic::new(
                        "VIEW002",
                        format!("{id} is not a probe (probe:<n>)"),
                    ));
                }
            };
            let Some(i) = h.probes.iter().position(|&p| p == n) else {
                return Err(Diagnostic::new(
                    "PROBE001",
                    format!("{id} is not a probe inside the mesh"),
                ));
            };
            keep.push(i);
        }
        out.probes = keep.iter().map(|&i| h.probes[i]).collect();
        for (r, src) in out.rows.iter_mut().zip(&h.rows) {
            r.values = keep.iter().map(|&i| src.values[i].clone()).collect();
        }
    }
    if let Some(names) = &pick.fields {
        let mut keep = Vec::new();
        for n in names {
            let Some(k) = h.column(n) else {
                let all: Vec<&str> = h.columns.iter().map(|c| c.name.as_str()).collect();
                return Err(Diagnostic::new(
                    "FIELD001",
                    format!(
                        "no cell or node field {n}; the fields are {}",
                        all.join(", ")
                    ),
                ));
            };
            keep.push(k);
        }
        out.columns = keep.iter().map(|&k| h.columns[k].clone()).collect();
        for r in &mut out.rows {
            r.values = r
                .values
                .iter()
                .map(|by_col| keep.iter().map(|&k| by_col[k].clone()).collect())
                .collect();
        }
    }
    Ok(out)
}

/// A probe's name as the tree shows it.
pub fn probe_name(s: &Session, n: u32) -> String {
    s.display
        .probe(n)
        .and_then(|p| p.name.clone())
        .unwrap_or_else(|| format!("Probe {n}"))
}

pub(crate) fn probe_history_schema() -> Value {
    object(
        json!({
            "probes": {"type": "array", "items": {"type": "string"}, "description": "probe:<n> ids; all probes inside the mesh when left out"},
            "fields": {"type": "array", "items": {"type": "string"}, "description": "Field names; all cell and node fields when left out"},
        }),
        &[],
    )
}

pub(crate) fn probe_history(input: Value, s: &mut Session) -> Run {
    let pick: Pick = parse(input)?;
    let h = history_of(s)?;
    let h = picked(s, &h, &pick)?;
    let probes: Vec<Value> = h
        .probes
        .iter()
        .map(|&n| json!({"id": format!("probe:{n}"), "name": probe_name(s, n)}))
        .collect();
    Ok(json!({
        "probes": probes,
        "columns": h.columns,
        "rows": h.rows,
        "steps": h.rows.len(),
        "series": s.series.is_some(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Export {
    path: PathBuf,
    probes: Option<Vec<String>>,
    fields: Option<Vec<String>>,
}

pub(crate) fn probe_export_schema() -> Value {
    object(
        json!({
            "path": {"type": "string", "description": "The CSV file to write"},
            "probes": {"type": "array", "items": {"type": "string"}, "description": "probe:<n> ids; all probes inside the mesh when left out"},
            "fields": {"type": "array", "items": {"type": "string"}, "description": "Field names; all cell and node fields when left out"},
        }),
        &["path"],
    )
}

pub(crate) fn probe_export(input: Value, s: &mut Session) -> Run {
    let i: Export = parse(input)?;
    let h = history_of(s)?;
    let h = picked(
        s,
        &h,
        &Pick {
            probes: i.probes,
            fields: i.fields,
        },
    )?;
    let csv = h.csv(&|n| probe_name(s, n));
    std::fs::write(&i.path, csv)
        .map_err(|e| Diagnostic::new("PROBE003", format!("{}: {e}", i.path.display())))?;
    Ok(json!({
        "files": [i.path],
        "steps": h.rows.len(),
        "probes": h.probes.len(),
        "fields": h.columns.len(),
    }))
}
