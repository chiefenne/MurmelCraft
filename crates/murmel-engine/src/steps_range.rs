//! The colour range of a field over every step of the open time series
//! (range mode [`RangeMode::Steps`]; the maintainer's card of 2026-11-24,
//! "Range over all steps"): the smallest and largest value the field has
//! at any step, so a colour means the same value at every step of a
//! playing series and of a movie of it.
//!
//! Each step's fields file is read as the probe histories read it
//! ([`crate::history`]: [`murmel_mesh::read_results_with`], no hash), the
//! derived and calculated fields of the open step are made again of its
//! values, and the field's range is taken as [`FieldSet::values`] takes
//! it (cells, points or boundary faces, the whole volume when the file
//! says), without the node values. The step shown is not read again. A
//! step without a fields file, or whose mesh has other cells or points,
//! adds nothing. Steps are read one after the other.
//!
//! Ranges are kept in a [`Cache`] that copies of the session share: the
//! window reads on a copy, on a thread of its own, and the range found so
//! far is kept after each step, so the legend widens while it reads.
//! Until a range is complete, [`Session::field_range`] spans the range
//! found so far and the step shown.
//!
//! [`RangeMode::Steps`]: crate::field::RangeMode::Steps
//! [`Session::field_range`]: crate::session::Session::field_range

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use murmel_mesh::Mesh;

use crate::field::{FieldChoice, FieldSet};
use crate::history::{Failure, Progress};
use crate::series::Series;

/// The range found over the steps read so far.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepsRange {
    /// `None` while no step read has a finite value.
    pub range: Option<(f64, f64)>,
    /// Steps read (with a value or not) and steps in the series.
    pub read: usize,
    pub total: usize,
}

impl StepsRange {
    /// Every step is read.
    pub fn complete(&self) -> bool {
        self.read >= self.total
    }
}

/// The ranges read, by [`key`]; shared by copies of a session.
pub type Cache = Arc<Mutex<HashMap<String, StepsRange>>>;

/// What the range of `choice` over `series` is read of: the series'
/// files, the derived and calculated fields, and the field and its
/// component. The step shown is left out, so playing reads nothing
/// again.
pub fn key(series: &Series, fields: &FieldSet, choice: &FieldChoice) -> String {
    format!(
        "{}|{:?}|{:?}|{}|{:?}",
        series.path.display(),
        series
            .entries
            .iter()
            .map(|e| (e.step, &e.fields))
            .collect::<Vec<_>>(),
        fields.made(),
        choice.name,
        choice.component
    )
}

/// The range kept for `key`, complete or not.
pub fn cached(cache: &Cache, key: &str) -> Option<StepsRange> {
    let c = cache.lock().unwrap_or_else(PoisonError::into_inner);
    c.get(key).copied()
}

fn keep(cache: &Cache, key: &str, r: StepsRange) {
    let mut c = cache.lock().unwrap_or_else(PoisonError::into_inner);
    // A cache of many fields and runs stays small: ranges are a few
    // numbers each, but a long session could try thousands of formulas.
    if c.len() > 4096 && !c.contains_key(key) {
        c.clear();
    }
    c.insert(key.to_string(), r);
}

fn widen(a: Option<(f64, f64)>, b: Option<(f64, f64)>) -> Option<(f64, f64)> {
    match (a, b) {
        (Some((a0, a1)), Some((b0, b1))) => Some((a0.min(b0), a1.max(b1))),
        (a, None) => a,
        (None, b) => b,
    }
}

/// Read the range of `choice` over `series`, `fields` being the open
/// step's fields of `mesh`, keeping what is found after each step in
/// `cache` under [`key`]. A range kept complete is returned as it is.
pub fn read(
    mesh: &Arc<Mesh>,
    fields: &FieldSet,
    series: &Series,
    choice: &FieldChoice,
    cache: &Cache,
    progress: &Progress,
) -> Result<StepsRange, Failure> {
    let k = key(series, fields, choice);
    if let Some(r) = cached(cache, &k).filter(StepsRange::complete) {
        return Ok(r);
    }
    if fields.get(&choice.name).is_none() {
        return Err(Failure::Nothing(format!(
            "the open mesh has no field {}",
            choice.name
        )));
    }
    let total = series.entries.len();
    progress.set_total(total);
    let mut found = StepsRange {
        range: None,
        read: 0,
        total,
    };
    for (index, e) in series.entries.iter().enumerate() {
        if progress.cancelled() {
            return Err(Failure::Cancelled);
        }
        let r = if index == series.current {
            fields.range_of(mesh, choice)
        } else if let Some(path) = &e.fields {
            match murmel_mesh::read_results_with(path, false) {
                Ok((m, results))
                    if m.cell_count() == mesh.cell_count()
                        && m.points.len() == mesh.points.len() =>
                {
                    let mut set = FieldSet::new(results);
                    if !fields.made().is_empty() {
                        set.share_topology(fields);
                        for made in fields.made() {
                            if let Ok((s, _)) = set.remake(mesh, made.clone()) {
                                set = s;
                            }
                        }
                    }
                    set.range_of(mesh, choice)
                }
                // Another mesh, or a file that does not read: the step
                // adds nothing (the player says why when it shows it).
                _ => None,
            }
        } else {
            None
        };
        found.range = widen(found.range, r);
        found.read = index + 1;
        keep(cache, &k, found);
        progress.set_done(index + 1);
    }
    Ok(found)
}

/// The range of `choice` to colour with now: the range kept for the
/// series (complete or so far) together with `shown`, the step shown's
/// own; `shown` alone when nothing is kept.
pub fn now(
    cache: &Cache,
    series: &Series,
    fields: &FieldSet,
    choice: &FieldChoice,
    shown: Option<(f64, f64)>,
) -> Option<(f64, f64)> {
    let kept = cached(cache, &key(series, fields, choice)).and_then(|r| r.range);
    widen(kept, shown)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Invariant: ranges join to the smallest start and the largest end,
    /// and a missing side leaves the other. Breaks if a step without a
    /// value narrowed the range.
    #[test]
    fn ranges_widen() {
        assert_eq!(
            widen(Some((1.0, 2.0)), Some((-1.0, 1.5))),
            Some((-1.0, 2.0))
        );
        assert_eq!(widen(None, Some((1.0, 2.0))), Some((1.0, 2.0)));
        assert_eq!(widen(Some((1.0, 2.0)), None), Some((1.0, 2.0)));
        assert_eq!(widen(None, None), None);
        let r = StepsRange {
            range: None,
            read: 2,
            total: 3,
        };
        assert!(!r.complete());
    }
}
