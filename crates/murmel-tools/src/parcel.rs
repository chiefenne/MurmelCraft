//! The tools of spray parcels (D268, first milestone): `parcels.open`,
//! `parcels.set` and `parcels.clear`. The drawing is the engine's
//! ([`murmel_engine::parcel`]); the file is read by
//! [`murmel_mesh::parcels`].

use std::path::PathBuf;
use std::sync::Arc;

use murmel_engine::field::{FieldChoice, RangeMode};
use murmel_engine::parcel::{DIAMETER, INJECTOR, MAX_SCALE, ParcelView, Size};
use murmel_engine::{Item, Session};
use murmel_mesh::{Component, Parcels};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{RangeKey, display_json, need_mesh, object, parse};
use crate::{Diagnostic, Run};

/// The parcels with what the last build drew (null without parcels).
pub(crate) fn parcels_json(s: &Session) -> Value {
    let Some(m) = &s.mesh else {
        return Value::Null;
    };
    murmel_engine::parcel::summary(&s.display, &m.surface).unwrap_or(Value::Null)
}

fn done(s: &mut Session) -> Run {
    s.redraw();
    let mut out = display_json(s);
    out["parcels"] = parcels_json(s);
    Ok(out)
}

fn need_parcels(s: &Session) -> Result<(), Diagnostic> {
    need_mesh(s)?;
    if s.display.parcels.is_none() || s.display.parcel_data.is_none() {
        return Err(Diagnostic::new(
            "PARCEL001",
            "no parcels are open; open a parcel file with parcels.open",
        ));
    }
    Ok(())
}

/// Read a parcel file into the display: the settings of the parcels
/// shown stay (the next file of the same run looks the same), a colour
/// field the new file lacks falls back to its default.
pub fn open_parcels(s: &mut Session, path: PathBuf) -> Result<(), Diagnostic> {
    let p = murmel_mesh::read_parcels(&path).map_err(|e| Diagnostic::new(e.code, e.message))?;
    let mut v = s
        .display
        .parcels
        .clone()
        .map(|old| ParcelView {
            path: path.clone(),
            ..old
        })
        .unwrap_or_else(|| {
            let mut v = crate::defaults::fresh(
                s,
                murmel_engine::defaults::Kind::Parcels,
                ParcelView::new(path.clone()),
            );
            v.path = path.clone();
            v.color = ParcelView::default_color(&p);
            v
        });
    if v.color.as_ref().is_some_and(|c| p.field(&c.name).is_none()) {
        v.color = ParcelView::default_color(&p);
    }
    s.display.parcels = Some(v);
    s.display.parcel_data = Some(Arc::new(p));
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenInput {
    path: PathBuf,
}

pub(crate) fn parcels_open(input: Value, s: &mut Session) -> Run {
    let i: OpenInput = parse(input)?;
    need_mesh(s)?;
    open_parcels(s, i.path)?;
    done(s)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetInput {
    #[serde(default)]
    enabled: Option<bool>,
    /// A parcel field, or `solid`.
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    component: Option<Component>,
    #[serde(default)]
    size: Option<Size>,
    #[serde(default)]
    scale: Option<f64>,
    /// [min, max] in metres, or null for every diameter.
    #[serde(default, deserialize_with = "some")]
    diameter: Option<Option<[f64; 2]>>,
    /// An injector number, or null for every injector.
    #[serde(default, deserialize_with = "some")]
    injector: Option<Option<u32>>,
    #[serde(default)]
    map: Option<String>,
    #[serde(default)]
    reverse: Option<bool>,
    #[serde(default)]
    range: Option<RangeKey>,
    #[serde(default)]
    min: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
}

/// A key given as null is `Some(None)`, one left out `None`.
fn some<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

fn bad(m: String) -> Diagnostic {
    Diagnostic::new("PARCEL003", m)
}

/// The fields of the file, for messages that list what exists.
fn listing(p: &Parcels) -> String {
    let names = p.field_names();
    if names.is_empty() {
        "the file has no parcel fields".into()
    } else {
        format!("the file has {}", names.join(", "))
    }
}

pub(crate) fn parcels_set(input: Value, s: &mut Session) -> Run {
    let i: SetInput = parse(input)?;
    need_parcels(s)?;
    let (Some(mut v), Some(p)) = (s.display.parcels.clone(), s.display.parcel_data.clone()) else {
        return Err(Diagnostic::new("PARCEL001", "no parcels are open"));
    };
    if let Some(e) = i.enabled {
        v.enabled = e;
    }
    match i.color.as_deref() {
        Some("solid") => v.color = None,
        Some(name) => {
            if p.field(name).is_none() {
                return Err(Diagnostic::new(
                    "PARCEL002",
                    format!("no parcel field {name}; {}, or solid", listing(&p)),
                ));
            }
            v.color = Some(FieldChoice {
                name: name.to_string(),
                component: i.component.unwrap_or_default(),
            });
        }
        None => {
            if let (Some(c), Some(choice)) = (i.component, v.color.as_mut()) {
                choice.component = c;
            }
        }
    }
    if let Some(sz) = i.size {
        if sz == Size::Diameter && p.field(DIAMETER).is_none() {
            return Err(Diagnostic::new(
                "PARCEL002",
                format!("size diameter needs a {DIAMETER} field; {}", listing(&p)),
            ));
        }
        v.size = sz;
    }
    if let Some(x) = i.scale {
        if !(x.is_finite() && x > 0.0 && x <= MAX_SCALE) {
            return Err(bad(format!(
                "scale {x} is outside 0 (not included) to {MAX_SCALE}"
            )));
        }
        v.scale = x;
    }
    if let Some(d) = i.diameter {
        if let Some([lo, hi]) = d {
            if p.field(DIAMETER).is_none() {
                return Err(Diagnostic::new(
                    "PARCEL002",
                    format!(
                        "the diameter filter needs a {DIAMETER} field; {}",
                        listing(&p)
                    ),
                ));
            }
            if !(lo.is_finite() && hi.is_finite() && lo >= 0.0 && lo <= hi) {
                return Err(bad(format!(
                    "the diameter range {lo} to {hi} needs finite 0 <= min <= max (metres)"
                )));
            }
        }
        v.diameter = d;
    }
    if let Some(k) = i.injector {
        if k.is_some() && p.field(INJECTOR).is_none() {
            return Err(Diagnostic::new(
                "PARCEL002",
                format!(
                    "the injector filter needs an {INJECTOR} field; {}",
                    listing(&p)
                ),
            ));
        }
        v.injector = k;
    }
    if let Some(m) = &i.map {
        if m == "auto" {
            v.map = None;
        } else if murmel_engine::colormap::get(m).is_some() {
            v.map = Some(m.clone());
        } else {
            return Err(bad(format!(
                "no colour map {m}; there are auto, {}",
                murmel_engine::colormap::ids().join(", ")
            )));
        }
    }
    if let Some(r) = i.reverse {
        v.reverse = r;
    }
    let range = match (i.range, i.min.is_some() || i.max.is_some()) {
        (Some(r), _) => Some(r),
        (None, true) => Some(RangeKey::Fixed),
        (None, false) => None,
    };
    match range {
        Some(RangeKey::Field) => v.range = RangeMode::Field,
        Some(RangeKey::Shown) => v.range = RangeMode::Shown,
        Some(RangeKey::Symmetric) => v.range = RangeMode::Symmetric,
        Some(RangeKey::Fixed) => {
            let now = s.mesh.as_ref().and_then(|m| {
                murmel_engine::parcel::color_range(&s.display, m.surface.parcel_range)
            });
            let (min, max) = match (i.min.or(now.map(|r| r.0)), i.max.or(now.map(|r| r.1))) {
                (Some(a), Some(b)) => (a, b),
                _ => return Err(bad("give min and max of the fixed range".into())),
            };
            if !(min.is_finite() && max.is_finite() && min < max) {
                return Err(bad(format!(
                    "the fixed range {min} to {max} needs finite min < max"
                )));
            }
            v.range = RangeMode::Fixed { min, max };
        }
        None => {}
    }
    s.display.parcels = Some(v);
    done(s)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClearInput {}

pub(crate) fn parcels_clear(input: Value, s: &mut Session) -> Run {
    let ClearInput {} = parse(input)?;
    s.display.parcels = None;
    s.display.parcel_data = None;
    s.display.selection.retain(|&i| i != Item::Parcels);
    s.display.colors.remove(&Item::Parcels);
    done(s)
}

// ---- schemas ------------------------------------------------------------

pub(crate) fn parcels_open_schema() -> Value {
    object(
        json!({"path": {"type": "string", "description": "A parcel file: FoehnCraft's spray/parcels_<step>.vtkhdf (VTKHDF PolyData)"}}),
        &["path"],
    )
}

pub(crate) fn parcels_set_schema() -> Value {
    object(
        json!({
            "enabled": {"type": "boolean"},
            "color": {"type": "string", "description": "A parcel field, or solid"},
            "component": {"enum": ["magnitude", "x", "y", "z"]},
            "size": {"enum": Size::ALL.map(Size::key)},
            "scale": {"type": "number", "exclusiveMinimum": 0, "maximum": MAX_SCALE},
            "diameter": {"oneOf": [
                {"type": "null"},
                {"type": "array", "items": {"type": "number", "minimum": 0}, "minItems": 2, "maxItems": 2}
            ]},
            "injector": {"oneOf": [{"type": "null"}, {"type": "integer", "minimum": 0}]},
            "map": {"type": "string"},
            "reverse": {"type": "boolean"},
            "range": {"enum": ["field", "shown", "symmetric", "fixed"]},
            "min": {"type": "number"},
            "max": {"type": "number"}
        }),
        &[],
    )
}

pub(crate) fn parcels_clear_schema() -> Value {
    object(json!({}), &[])
}
