//! Probe histories: every probe's field values at every time step of the
//! open series (the maintainer's card of 2026-11-24, "Follow steps, plus
//! history plot": the values follow the step shown, and a probe window
//! plots each probe over all steps, read in the background, and exports
//! CSV).
//!
//! Each step's fields file is read as the player reads it
//! ([`murmel_mesh::read_results_with`], no SHA-256), the derived and
//! calculated fields of the open step are made again of its values in
//! the same order (as [`crate::session::Session::open_step`] does), and
//! every probe reads its cell as [`FieldSet::probe_values`] does. The
//! step shown is not read again. A step without a fields file, or whose
//! mesh has other cells or points than the open one, gets a note and no
//! values. Steps are read one after the other, so at most one step's
//! fields are in memory beside the open ones.
//!
//! The last history read is kept in a [`Cache`] that copies of the
//! session share (the window reads on a copy, on a thread of its own),
//! so plotting it and exporting it read the run once.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use murmel_mesh::Mesh;
use murmel_mesh::field::Location;
use serde::Serialize;

use crate::field::FieldSet;
use crate::series::Series;

/// A probe inside the mesh, as the history reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    /// The number of its id `probe:<n>`.
    pub n: u32,
    pub cell: u32,
    pub point: [f64; 3],
}

/// A field the history reads: a cell or node field of the open step,
/// derived and calculated ones included.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Column {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// 1 for a scalar, 3 for a vector.
    pub components: usize,
}

/// One time step of the history.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    /// The entry of the series (0 without one).
    pub index: usize,
    pub step: Option<u64>,
    pub time: Option<f64>,
    /// By probe, then by column: the value (one per component), or
    /// `None` where the step has no value.
    pub values: Vec<Vec<Option<Vec<f64>>>>,
    /// Why the step has no values, when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct History {
    /// The probes read, by the number of their id.
    pub probes: Vec<u32>,
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
}

impl History {
    /// The column of field `name`.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    /// The history as CSV, one line per step: `step`, `time`, then for
    /// each probe (named by `name`) each field's components (`x`, `y`,
    /// `z` of a vector), the unit in brackets. Values are written in
    /// full precision; an empty cell is a value the step lacks.
    pub fn csv(&self, name: &dyn Fn(u32) -> String) -> String {
        let mut head = vec!["step".to_string(), "time".to_string()];
        for &p in &self.probes {
            let probe = name(p);
            for c in &self.columns {
                let unit = c
                    .unit
                    .as_ref()
                    .map(|u| format!(" [{u}]"))
                    .unwrap_or_default();
                if c.components == 1 {
                    head.push(format!("{probe}: {}{unit}", c.name));
                } else {
                    for k in ["x", "y", "z"].iter().take(c.components) {
                        head.push(format!("{probe}: {}_{k}{unit}", c.name));
                    }
                }
            }
        }
        let mut out = head.iter().map(|h| quote(h)).collect::<Vec<_>>().join(",");
        out.push('\n');
        for r in &self.rows {
            let mut line = vec![
                r.step.map(|s| s.to_string()).unwrap_or_default(),
                r.time.map(|t| t.to_string()).unwrap_or_default(),
            ];
            for (p, _) in self.probes.iter().enumerate() {
                for (k, c) in self.columns.iter().enumerate() {
                    let v = r
                        .values
                        .get(p)
                        .and_then(|v| v.get(k))
                        .and_then(Option::as_ref);
                    for j in 0..c.components {
                        line.push(
                            v.and_then(|v| v.get(j))
                                .map(|x| x.to_string())
                                .unwrap_or_default(),
                        );
                    }
                }
            }
            out.push_str(&line.join(","));
            out.push('\n');
        }
        out
    }
}

/// A CSV field, quoted when it holds a comma, a quote or a line break.
fn quote(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// How far a history is, and a request to stop it: shared between the
/// thread that reads it and the window that shows its progress.
#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicUsize,
    total: AtomicUsize,
    cancel: AtomicBool,
}

impl Progress {
    /// Steps read so far.
    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }

    /// Steps to read (0 until the reading starts).
    pub fn total(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }

    /// Ask the reading to stop before its next step.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Why no history was read.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// No probe lies inside the mesh, or the open mesh has no fields.
    Nothing(String),
    /// Stopped by [`Progress::cancel`].
    Cancelled,
}

/// What a history is read of: the series' files (the open mesh without
/// one), the derived and calculated fields and the probes, in one
/// string, so a cached history is used only while none of them changed.
/// The step shown is left out: playing the series reads nothing again.
pub fn key(
    mesh_path: &std::path::Path,
    fields: &FieldSet,
    series: Option<&Series>,
    points: &[Point],
) -> String {
    let source = match series {
        Some(s) => format!(
            "{}|{:?}",
            s.path.display(),
            s.entries
                .iter()
                .map(|e| (e.step, &e.fields))
                .collect::<Vec<_>>()
        ),
        None => mesh_path.display().to_string(),
    };
    format!("{source}|{:?}|{:?}", fields.made(), points)
}

/// The last history read and its [`key`]; shared by copies of a session.
pub type Cache = Arc<Mutex<Option<(String, Arc<History>)>>>;

/// The cached history of `key`, if it is the one kept.
pub fn cached(cache: &Cache, key: &str) -> Option<Arc<History>> {
    let c = cache.lock().unwrap_or_else(PoisonError::into_inner);
    c.as_ref().filter(|(k, _)| k == key).map(|(_, h)| h.clone())
}

/// Keep `history` as the one of `key`.
pub fn keep(cache: &Cache, key: String, history: Arc<History>) {
    *cache.lock().unwrap_or_else(PoisonError::into_inner) = Some((key, history));
}

/// The fields a history reads: the open step's cell and node fields.
fn columns(fields: &FieldSet, mesh: &Mesh) -> Vec<Column> {
    let mut out: Vec<Column> = Vec::new();
    for f in fields.fields() {
        if f.location == Location::Boundary
            || !FieldSet::fits(f, mesh)
            || out.iter().any(|c| c.name == f.name)
        {
            continue;
        }
        out.push(Column {
            name: f.name.clone(),
            unit: f.unit.clone(),
            components: f.components.max(1),
        });
    }
    out
}

/// Every probe's value of every column in `fields`.
fn read_step(
    fields: &FieldSet,
    mesh: &Mesh,
    points: &[Point],
    cols: &[Column],
) -> Vec<Vec<Option<Vec<f64>>>> {
    points
        .iter()
        .map(|p| {
            let vals = fields.probe_values(mesh, p.cell as usize, p.point, None);
            cols.iter()
                .map(|c| {
                    vals.iter()
                        .find(|v| v.name == c.name && v.location != Location::Boundary)
                        .filter(|v| v.value.len() == c.components)
                        .map(|v| v.value.clone())
                })
                .collect()
        })
        .collect()
}

/// Read the history of `points` over `series` (just the open step
/// without one), `fields` being the open step's fields of `mesh`.
pub fn read(
    mesh: &Arc<Mesh>,
    fields: &FieldSet,
    series: Option<&Series>,
    points: &[Point],
    progress: &Progress,
) -> Result<History, Failure> {
    if points.is_empty() {
        return Err(Failure::Nothing(
            "no probe lies inside the mesh; place one with probe.add".into(),
        ));
    }
    let cols = columns(fields, mesh);
    if cols.is_empty() {
        return Err(Failure::Nothing(
            "the open mesh has no cell or node fields to read".into(),
        ));
    }
    let probes = points.iter().map(|p| p.n).collect();
    let none = || vec![vec![None; cols.len()]; points.len()];
    let Some(series) = series else {
        progress.total.store(1, Ordering::Relaxed);
        let row = Row {
            index: 0,
            step: fields.results.step.or(fields.results.iteration),
            time: fields.results.time,
            values: read_step(fields, mesh, points, &cols),
            note: None,
        };
        progress.done.store(1, Ordering::Relaxed);
        return Ok(History {
            probes,
            columns: cols,
            rows: vec![row],
        });
    };
    progress
        .total
        .store(series.entries.len(), Ordering::Relaxed);
    let mut rows = Vec::with_capacity(series.entries.len());
    for (index, e) in series.entries.iter().enumerate() {
        if progress.cancelled() {
            return Err(Failure::Cancelled);
        }
        let mut row = Row {
            index,
            step: Some(e.step),
            time: e.time,
            values: none(),
            note: None,
        };
        if index == series.current {
            row.values = read_step(fields, mesh, points, &cols);
            row.time = row.time.or(fields.results.time);
        } else if let Some(path) = &e.fields {
            match murmel_mesh::read_results_with(path, false) {
                Err(err) => row.note = Some(err.to_string()),
                Ok((m, results))
                    if m.cell_count() != mesh.cell_count()
                        || m.points.len() != mesh.points.len() =>
                {
                    drop(results);
                    row.note = Some(format!(
                        "{} has another mesh ({} cells) than the step shown ({} cells)",
                        path.display(),
                        m.cell_count(),
                        mesh.cell_count()
                    ));
                }
                Ok((_, results)) => {
                    row.time = row.time.or(results.time);
                    let mut set = FieldSet::new(results);
                    set.share_topology(fields);
                    for made in fields.made() {
                        if let Ok((s, _)) = set.remake(mesh, made.clone()) {
                            set = s;
                        }
                    }
                    row.values = read_step(&set, mesh, points, &cols);
                }
            }
        } else {
            row.note = Some("the step has no fields file".into());
        }
        rows.push(row);
        progress.done.store(index + 1, Ordering::Relaxed);
    }
    Ok(History {
        probes,
        columns: cols,
        rows,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Invariant: the CSV has a header naming each probe's field
    /// components with units, a line per step, and empty cells where a
    /// step lacks a value; names with commas are quoted. Breaks if the
    /// columns and the values went out of step.
    #[test]
    fn csv_lines_up() {
        let h = History {
            probes: vec![1, 2],
            columns: vec![
                Column {
                    name: "pressure".into(),
                    unit: Some("Pa".into()),
                    components: 1,
                },
                Column {
                    name: "velocity".into(),
                    unit: None,
                    components: 3,
                },
            ],
            rows: vec![
                Row {
                    index: 0,
                    step: Some(10),
                    time: Some(0.5),
                    values: vec![
                        vec![Some(vec![1.5]), Some(vec![1.0, 2.0, 3.0])],
                        vec![None, Some(vec![4.0, 5.0, 6.0])],
                    ],
                    note: None,
                },
                Row {
                    index: 1,
                    step: Some(20),
                    time: None,
                    values: vec![vec![None, None], vec![None, None]],
                    note: Some("no file".into()),
                },
            ],
        };
        let csv = h.csv(&|n| {
            if n == 1 {
                "Inlet, top".into()
            } else {
                format!("Probe {n}")
            }
        });
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(
            lines[0],
            "step,time,\"Inlet, top: pressure [Pa]\",\"Inlet, top: velocity_x\",\"Inlet, top: velocity_y\",\"Inlet, top: velocity_z\",Probe 2: pressure [Pa],Probe 2: velocity_x,Probe 2: velocity_y,Probe 2: velocity_z"
        );
        assert_eq!(lines[1], "10,0.5,1.5,1,2,3,,4,5,6");
        assert_eq!(lines[2], "20,,,,,,,,,");
        assert_eq!(lines.len(), 3);
    }
}
