//! The tools of slices and iso-surfaces (field results, stage 2):
//! `slice.set`, `slice.rotate`, `slice.clear`, `iso.set`, `iso.clear`
//! and `display.mesh_look`; and of vector arrows (stage 3): `arrows.set`
//! and `arrows.clear`. The geometry is the engine's
//! ([`murmel_engine::extract`], [`murmel_engine::glyph`]).

use murmel_engine::extract::{IsoColor, MAX_ISOS, MAX_SERIES, MAX_SLICES, MeshLook};
use murmel_engine::glyph::{Arrows, MAX_DENSITY, MAX_SCALE, MIN_DENSITY};
use murmel_engine::{Item, ScreenAxis, Session};
use murmel_mesh::Component;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{display_json, field_choice, finite, need_fields, need_mesh, object, parse};
use crate::{Diagnostic, Run};

fn bounds(s: &Session) -> murmel_mesh::Bounds {
    s.mesh
        .as_ref()
        .map(|m| m.mesh.bounds())
        .unwrap_or_else(murmel_mesh::Bounds::empty)
}

/// Slice `k` (0-based) with its id and planes.
pub(crate) fn slice_json(s: &Session, k: usize) -> Value {
    let (Some(c), Some(m)) = (s.display.slices.get(k), &s.mesh) else {
        return Value::Null;
    };
    let (offset, range) = c.plane().offset(&m.mesh.bounds());
    json!({
        "id": Item::Slice(k as u32).id(&m.mesh),
        "slice": k + 1,
        "enabled": c.enabled,
        "origin": c.origin,
        "normal": c.normal,
        "offset": offset,
        "range": range,
        "count": c.count,
        "spacing": c.spacing,
        "current": s.display.current_slice() == Some(k),
    })
}

/// Iso-surface `k` (0-based) with its id.
pub(crate) fn iso_json(s: &Session, k: usize) -> Value {
    let (Some(c), Some(m)) = (s.display.isos.get(k), &s.mesh) else {
        return Value::Null;
    };
    let f = m.fields.get(&c.field.name);
    let vector = f.is_some_and(|f| f.is_vector());
    json!({
        "id": Item::Iso(k as u32).id(&m.mesh),
        "iso": k + 1,
        "enabled": c.enabled,
        "field": c.field.name,
        "component": c.field.component,
        "title": c.field.title(vector),
        "unit": f.and_then(|f| f.unit.clone()),
        "value": c.value,
        "color": c.color,
        "solid": murmel_engine::extract::iso_color(&s.display, &s.tokens, k).hex(),
        "current": s.display.current_iso() == Some(k),
    })
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Axis {
    X,
    Y,
    Z,
    View,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SliceSet {
    slice: Option<u32>,
    #[serde(default)]
    new: bool,
    axis: Option<Axis>,
    normal: Option<[f64; 3]>,
    origin: Option<[f64; 3]>,
    offset: Option<f64>,
    #[serde(default)]
    flip: bool,
    enabled: Option<bool>,
    count: Option<u32>,
    spacing: Option<f64>,
}

/// The index of slice `n` (counted from 1), or of the current slice.
fn slice_index(s: &Session, n: Option<u32>) -> Result<Option<usize>, Diagnostic> {
    match n {
        None => Ok(s.display.current_slice()),
        Some(n) if n >= 1 && (n as usize) <= s.display.slices.len() => Ok(Some(n as usize - 1)),
        Some(n) => Err(Diagnostic::new(
            "SLICE001",
            format!(
                "no slice {n}; there are {} (slice:1 is the first)",
                s.display.slices.len()
            ),
        )),
    }
}

/// Redraw, and keep the slice or iso-surface just added current.
fn done(s: &mut Session) -> Run {
    s.redraw();
    Ok(display_json(s))
}

pub(crate) fn slice_set(input: Value, s: &mut Session) -> Run {
    let i: SliceSet = parse(input)?;
    need_mesh(s)?;
    let both = |a: bool, b: bool, what: &str| {
        if a && b {
            Err(Diagnostic::new(
                "TOOL002",
                format!("give either {what}, not both"),
            ))
        } else {
            Ok(())
        }
    };
    both(i.axis.is_some(), i.normal.is_some(), "axis or normal")?;
    both(i.origin.is_some(), i.offset.is_some(), "origin or offset")?;
    both(i.new, i.slice.is_some(), "new or slice")?;
    let b = bounds(s);
    let at = if i.new {
        None
    } else {
        slice_index(s, i.slice)?
    };
    if at.is_none() && s.display.slices.len() >= MAX_SLICES {
        return Err(Diagnostic::new(
            "SLICE002",
            format!("{MAX_SLICES} slices are the most"),
        ));
    }
    let mut c = at
        .map(|k| s.display.slices[k])
        .unwrap_or_else(|| murmel_engine::Slice::new(b.center(), [1.0, 0.0, 0.0]));
    if let Some(a) = i.axis {
        // As for clip planes: the normal points towards the eye.
        let to_eye = s.camera.to_eye().to_array();
        let toward = |k: usize| {
            let mut n = [0.0; 3];
            n[k] = if to_eye[k] < -1e-9 { -1.0 } else { 1.0 };
            n
        };
        c.normal = match a {
            Axis::X => toward(0),
            Axis::Y => toward(1),
            Axis::Z => toward(2),
            Axis::View => to_eye,
        };
        c.enabled = true;
    }
    if let Some(n) = i.normal {
        for v in n {
            finite(v, "normal")?;
        }
        c.normal = murmel_engine::clip::unit(n)
            .ok_or_else(|| Diagnostic::new("TOOL002", "normal must not be zero"))?;
        c.enabled = true;
    }
    if i.flip {
        c.normal = c.normal.map(|v| -v);
    }
    if let Some(o) = i.origin {
        for v in o {
            finite(v, "origin")?;
        }
        c.origin = o;
    }
    if let Some(off) = i.offset {
        let mut p = c.plane();
        p.set_offset(&b, finite(off, "offset")?);
        c.origin = p.origin;
    }
    if let Some(e) = i.enabled {
        c.enabled = e;
    }
    let series = |why: String| Diagnostic::new("SLICE003", why);
    match (i.count, i.spacing) {
        (Some(n), _) if !(1..=MAX_SERIES).contains(&n) => {
            return Err(series(format!("count {n} is outside 1 to {MAX_SERIES}")));
        }
        (_, Some(d)) if !(d.is_finite() && d > 0.0) => {
            return Err(series(format!("spacing {d} must be positive")));
        }
        // A count alone spreads the planes through the mesh.
        (Some(n), None) => c.spread(&b, n),
        (Some(n), Some(d)) => {
            c.count = n;
            c.spacing = if n > 1 { d } else { 0.0 };
        }
        (None, Some(d)) => {
            if c.count > 1 {
                c.spacing = d;
            }
        }
        (None, None) => {}
    }
    match at {
        Some(k) => s.display.slices[k] = c,
        None => {
            s.display.selection.retain(|i| !matches!(i, Item::Slice(_)));
            s.display.slices.push(c);
        }
    }
    done(s)
}

/// The axis of `slice.rotate`, as for `clip.rotate`.
#[derive(Deserialize)]
#[serde(untagged)]
enum TiltAxis {
    Screen(ScreenAxis),
    World([f64; 3]),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SliceRotate {
    axis: TiltAxis,
    deg: f64,
    pivot: Option<[f64; 3]>,
    slice: Option<u32>,
}

pub(crate) fn slice_rotate(input: Value, s: &mut Session) -> Run {
    let i: SliceRotate = parse(input)?;
    need_mesh(s)?;
    let Some(k) = slice_index(s, i.slice)? else {
        return Err(Diagnostic::new(
            "SLICE001",
            "there is no slice to tilt; add one with slice.set first",
        ));
    };
    let deg = finite(i.deg, "deg")?;
    let axis = match i.axis {
        TiltAxis::Screen(a) => {
            let (right, up) = s.camera.basis();
            match a {
                ScreenAxis::X => right,
                ScreenAxis::Y => up,
                ScreenAxis::Z => s.camera.to_eye(),
            }
            .to_array()
        }
        TiltAxis::World(v) => {
            for x in v {
                finite(x, "axis")?;
            }
            v
        }
    };
    let b = bounds(s);
    let mut plane = s.display.slices[k].plane();
    let pivot = match i.pivot {
        Some(p) => {
            for x in p {
                finite(x, "pivot")?;
            }
            p
        }
        None => plane.pivot(&b),
    };
    plane
        .rotate(axis, deg, pivot)
        .ok_or_else(|| Diagnostic::new("TOOL002", "axis must not be zero"))?;
    s.display.slices[k].set_plane(&plane);
    done(s)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Clear {
    slice: Option<u32>,
    iso: Option<u32>,
    #[serde(default)]
    all: bool,
}

pub(crate) fn slice_clear(input: Value, s: &mut Session) -> Run {
    let i: Clear = parse(input)?;
    if i.iso.is_some() {
        return Err(Diagnostic::new(
            "TOOL002",
            "slice.clear takes slice, not iso",
        ));
    }
    if i.all {
        if i.slice.is_some() {
            return Err(Diagnostic::new(
                "TOOL002",
                "give either all or slice, not both",
            ));
        }
        s.display.slices.clear();
        s.display.selection.retain(|i| !matches!(i, Item::Slice(_)));
    } else if let Some(k) = slice_index(s, i.slice)? {
        s.display.remove_slice(k);
    }
    done(s)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IsoSet {
    iso: Option<u32>,
    #[serde(default)]
    new: bool,
    field: Option<String>,
    component: Option<Component>,
    value: Option<f64>,
    color: Option<IsoColor>,
    enabled: Option<bool>,
}

fn iso_index(s: &Session, n: Option<u32>) -> Result<Option<usize>, Diagnostic> {
    match n {
        None => Ok(s.display.current_iso()),
        Some(n) if n >= 1 && (n as usize) <= s.display.isos.len() => Ok(Some(n as usize - 1)),
        Some(n) => Err(Diagnostic::new(
            "ISO001",
            format!(
                "no iso-surface {n}; there are {} (iso:1 is the first)",
                s.display.isos.len()
            ),
        )),
    }
}

/// The middle of a field's values over the whole field.
fn middle(s: &Session, choice: &murmel_engine::FieldChoice) -> Option<f64> {
    let m = s.mesh.as_ref()?;
    let v = m.fields.values_cached(&m.mesh, choice)?;
    let (lo, hi) = murmel_engine::field::widen(v.range?);
    Some(0.5 * (lo + hi))
}

pub(crate) fn iso_set(input: Value, s: &mut Session) -> Run {
    let i: IsoSet = parse(input)?;
    need_fields(s)?;
    if i.new && i.iso.is_some() {
        return Err(Diagnostic::new(
            "TOOL002",
            "give either new or iso, not both",
        ));
    }
    let at = if i.new { None } else { iso_index(s, i.iso)? };
    if at.is_none() && s.display.isos.len() >= MAX_ISOS {
        return Err(Diagnostic::new(
            "ISO002",
            format!("{MAX_ISOS} iso-surfaces are the most"),
        ));
    }
    let old = at.map(|k| s.display.isos[k].clone());
    // The field: the one named, else the iso-surface's own, else the one
    // on the colour bar (or the file's first).
    let name = i
        .field
        .clone()
        .or_else(|| old.as_ref().map(|o| o.field.name.clone()));
    let component = i.component.or_else(|| {
        old.as_ref()
            .filter(|o| i.field.as_ref().is_none_or(|f| *f == o.field.name))
            .map(|o| o.field.component)
    });
    let field = field_choice(s, name.as_deref(), component)?;
    let same_field = old.as_ref().is_some_and(|o| o.field == field);
    let value = match i.value {
        Some(v) => finite(v, "value")?,
        None => match old.as_ref().filter(|_| same_field) {
            Some(o) => o.value,
            None => middle(s, &field).ok_or_else(|| {
                Diagnostic::new("FIELD002", format!("{} has no values", field.name))
            })?,
        },
    };
    let iso = murmel_engine::Iso {
        field,
        value,
        enabled: i
            .enabled
            .or(old.as_ref().map(|o| o.enabled))
            .unwrap_or(true),
        color: i
            .color
            .or(old.as_ref().map(|o| o.color))
            .unwrap_or_default(),
    };
    match at {
        Some(k) => s.display.isos[k] = iso,
        None => {
            s.display.selection.retain(|i| !matches!(i, Item::Iso(_)));
            s.display.isos.push(iso);
        }
    }
    done(s)
}

pub(crate) fn iso_clear(input: Value, s: &mut Session) -> Run {
    let i: Clear = parse(input)?;
    if i.slice.is_some() {
        return Err(Diagnostic::new("TOOL002", "iso.clear takes iso, not slice"));
    }
    if i.all {
        if i.iso.is_some() {
            return Err(Diagnostic::new(
                "TOOL002",
                "give either all or iso, not both",
            ));
        }
        while !s.display.isos.is_empty() {
            s.display.remove_iso(0);
        }
    } else if let Some(k) = iso_index(s, i.iso)? {
        s.display.remove_iso(k);
    }
    done(s)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Look {
    look: MeshLook,
}

pub(crate) fn mesh_look(input: Value, s: &mut Session) -> Run {
    let i: Look = parse(input)?;
    s.display.mesh_look = i.look;
    done(s)
}

/// The arrows with their id, grid spacing and what the last build drew
/// (null without arrows).
pub(crate) fn arrows_json(s: &Session) -> Value {
    let (Some(a), Some(m)) = (s.display.arrows.as_ref(), &s.mesh) else {
        return Value::Null;
    };
    let unit = m.fields.get(&a.field).and_then(|f| f.unit.clone());
    let surface = &m.surface;
    json!({
        "id": Item::Arrows.id(&m.mesh),
        "field": a.field,
        "enabled": a.enabled,
        "surface": a.surface,
        "in_plane": a.in_plane,
        "density": a.density,
        "spacing": a.spacing(&m.mesh),
        "scale": a.scale,
        "color": a.color,
        "solid": s.display.item_color(&m.mesh, &s.tokens, Item::Arrows).map(|c| c.hex()),
        "unit": unit,
        "drawn": surface.arrows,
        "capped": surface.arrows_capped,
        "longest": (surface.arrow_longest > 0.0).then_some(surface.arrow_longest),
        "slices_shown": s.display.slices.iter().filter(|x| x.enabled).count(),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArrowsSet {
    field: Option<String>,
    enabled: Option<bool>,
    surface: Option<bool>,
    in_plane: Option<bool>,
    density: Option<u32>,
    scale: Option<f64>,
    color: Option<IsoColor>,
}

/// The vector field the arrows draw: the one named, else their own, else
/// the field shown when it is a vector, else the file's first vector.
fn arrow_field(s: &Session, name: Option<&str>) -> Result<String, Diagnostic> {
    let set = need_fields(s)?;
    let vectors: Vec<&str> = set
        .names()
        .into_iter()
        .filter(|n| set.get(n).is_some_and(|f| f.is_vector()))
        .collect();
    let name = name
        .map(str::to_string)
        .or_else(|| s.display.arrows.as_ref().map(|a| a.field.clone()))
        .or_else(|| {
            s.display
                .field
                .as_ref()
                .map(|c| c.name.clone())
                .filter(|n| vectors.contains(&n.as_str()))
        })
        .or_else(|| vectors.first().map(|n| n.to_string()));
    let Some(name) = name else {
        return Err(Diagnostic::new(
            "ARROW001",
            format!(
                "the open mesh has no vector field (one with x, y and z components); it has {}",
                set.names().join(", ")
            ),
        ));
    };
    match set.get(&name) {
        Some(f) if f.is_vector() => Ok(name),
        Some(_) => Err(Diagnostic::new(
            "ARROW001",
            format!(
                "{name} is a scalar; arrows need a vector field ({})",
                if vectors.is_empty() {
                    "the open mesh has none".to_string()
                } else {
                    vectors.join(", ")
                }
            ),
        )),
        None => Err(Diagnostic::new(
            "FIELD002",
            format!(
                "no field {name}; the open mesh has {}",
                set.names().join(", ")
            ),
        )),
    }
}

pub(crate) fn arrows_set(input: Value, s: &mut Session) -> Run {
    let i: ArrowsSet = parse(input)?;
    need_fields(s)?;
    let field = arrow_field(s, i.field.as_deref())?;
    if let Some(d) = i.density
        && !(MIN_DENSITY..=MAX_DENSITY).contains(&d)
    {
        return Err(Diagnostic::new(
            "ARROW002",
            format!("density {d} is outside {MIN_DENSITY} to {MAX_DENSITY}"),
        ));
    }
    if let Some(x) = i.scale
        && !(x.is_finite() && x > 0.0 && x <= MAX_SCALE)
    {
        return Err(Diagnostic::new(
            "ARROW002",
            format!("scale {x} is outside 0 (not included) to {MAX_SCALE}"),
        ));
    }
    let mut a = s
        .display
        .arrows
        .clone()
        .unwrap_or_else(|| Arrows::new(field.clone()));
    a.field = field;
    if let Some(x) = i.enabled {
        a.enabled = x;
    }
    if let Some(x) = i.surface {
        a.surface = x;
    }
    if let Some(x) = i.in_plane {
        a.in_plane = x;
    }
    if let Some(x) = i.density {
        a.density = x;
    }
    if let Some(x) = i.scale {
        a.scale = x;
    }
    if let Some(x) = i.color {
        a.color = x;
    }
    s.display.arrows = Some(a);
    s.redraw();
    let mut out = display_json(s);
    out["arrows"] = arrows_json(s);
    Ok(out)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArrowsClear {}

pub(crate) fn arrows_clear(input: Value, s: &mut Session) -> Run {
    let ArrowsClear {} = parse(input)?;
    s.display.arrows = None;
    s.display.selection.retain(|&i| i != Item::Arrows);
    s.display.colors.remove(&Item::Arrows);
    done(s)
}

// ---- schemas ------------------------------------------------------------

fn vec3_schema() -> Value {
    json!({"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3})
}

pub(crate) fn slice_set_schema() -> Value {
    object(
        json!({
            "slice": {"type": "integer", "minimum": 1},
            "new": {"type": "boolean"},
            "axis": {"enum": ["x", "y", "z", "view"]},
            "normal": vec3_schema(),
            "origin": vec3_schema(),
            "offset": {"type": "number"},
            "flip": {"type": "boolean"},
            "enabled": {"type": "boolean"},
            "count": {"type": "integer", "minimum": 1, "maximum": MAX_SERIES},
            "spacing": {"type": "number", "exclusiveMinimum": 0}
        }),
        &[],
    )
}

pub(crate) fn slice_rotate_schema() -> Value {
    object(
        json!({
            "axis": {"oneOf": [{"enum": ["x", "y", "z"]}, vec3_schema()]},
            "deg": {"type": "number"},
            "pivot": vec3_schema(),
            "slice": {"type": "integer", "minimum": 1}
        }),
        &["axis", "deg"],
    )
}

pub(crate) fn slice_clear_schema() -> Value {
    object(
        json!({"slice": {"type": "integer", "minimum": 1}, "all": {"type": "boolean"}}),
        &[],
    )
}

pub(crate) fn iso_set_schema() -> Value {
    object(
        json!({
            "iso": {"type": "integer", "minimum": 1},
            "new": {"type": "boolean"},
            "field": {"type": "string"},
            "component": {"enum": ["magnitude", "x", "y", "z"]},
            "value": {"type": "number"},
            "color": {"enum": ["solid", "field"]},
            "enabled": {"type": "boolean"}
        }),
        &[],
    )
}

pub(crate) fn iso_clear_schema() -> Value {
    object(
        json!({"iso": {"type": "integer", "minimum": 1}, "all": {"type": "boolean"}}),
        &[],
    )
}

pub(crate) fn arrows_set_schema() -> Value {
    object(
        json!({
            "field": {"type": "string"},
            "enabled": {"type": "boolean"},
            "surface": {"type": "boolean"},
            "in_plane": {"type": "boolean"},
            "density": {"type": "integer", "minimum": MIN_DENSITY, "maximum": MAX_DENSITY},
            "scale": {"type": "number", "exclusiveMinimum": 0, "maximum": MAX_SCALE},
            "color": {"enum": ["solid", "field"]}
        }),
        &[],
    )
}

pub(crate) fn arrows_clear_schema() -> Value {
    object(json!({}), &[])
}

pub(crate) fn mesh_look_schema() -> Value {
    object(json!({"look": {"enum": ["outline", "faces"]}}), &["look"])
}
