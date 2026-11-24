//! Your defaults (`settings.set_default`, `settings.reset_default`; the
//! model is [`murmel_engine::defaults`]): a value saved here is where new
//! items of its kind start; items already open keep theirs. The tools
//! that add items lay the defaults over a fresh item before their own
//! input ([`fresh`]), and a field shown for the first time takes the
//! default field style ([`adopt_style`]).

use murmel_engine::defaults::{Kind, View, values_of};
use murmel_engine::{FieldStyle, Item, Session};
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::tools::{object, parse, save_session_settings};
use crate::{Diagnostic, Run};

/// `item` with the user's defaults of `kind` laid over it.
pub(crate) fn fresh<T: Serialize + DeserializeOwned>(s: &Session, kind: Kind, item: T) -> T {
    s.defaults.apply(kind, item)
}

/// Give field `name` the default field style when it has no style of its
/// own yet and a default style is saved, so a field shown for the first
/// time starts from it; fields with a style keep it.
pub(crate) fn adopt_style(s: &mut Session, name: &str) {
    let series = s.series.is_some();
    if (s.defaults.of(Kind::FieldStyle).is_none() && !series)
        || s.display.field_styles.contains_key(name)
    {
        return;
    }
    let st = fresh(s, Kind::FieldStyle, FieldStyle::default());
    s.display.field_styles.insert(name.to_string(), st);
    // While a series is open, a field first shown spans every step.
    crate::series::steps_by_default_one(s, Some(name));
}

fn not_a_default(m: String) -> Diagnostic {
    Diagnostic::new("SET003", m)
}

/// An item of the display as JSON, as its tool and session files name
/// its values.
fn item_json(s: &Session, item: Item) -> Option<Value> {
    let d = &s.display;
    let v = match item {
        Item::Arrows => serde_json::to_value(d.arrows.as_ref()?),
        Item::Slice(k) => serde_json::to_value(d.slices.get(k as usize)?),
        Item::Iso(k) => serde_json::to_value(d.isos.get(k as usize)?),
        Item::Stream(k) => serde_json::to_value(d.streams.get(k as usize)?),
        Item::Clip(k) => serde_json::to_value(d.clips.get(k as usize)?),
        Item::Region(k) => serde_json::to_value(d.regions.get(k as usize)?),
        _ => return None,
    };
    v.ok()
}

fn kinds() -> String {
    Kind::ALL.map(Kind::key).join(", ")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetDefault {
    kind: Option<String>,
    item: Option<String>,
    field: Option<String>,
    #[serde(default)]
    keys: Vec<String>,
    values: Option<Map<String, Value>>,
}

pub(crate) fn settings_set_default(input: Value, s: &mut Session) -> Run {
    let i: SetDefault = parse(input)?;
    if i.values.is_some() && !i.keys.is_empty() {
        return Err(Diagnostic::new(
            "TOOL002",
            "give values, or keys to take from the item, not both",
        ));
    }
    // The kind, and the values now of what is named: an item, the view
    // or a field's style.
    let (kind, current) = match (&i.item, &i.kind) {
        (Some(_), Some(_)) => {
            return Err(Diagnostic::new(
                "TOOL002",
                "give either item or kind, not both",
            ));
        }
        (Some(id), None) => {
            let m = s
                .mesh
                .as_ref()
                .ok_or_else(|| Diagnostic::new("VIEW001", "no mesh is open"))?;
            let item = Item::parse(id, &m.mesh)
                .ok_or_else(|| Diagnostic::new("TOOL002", format!("no item {id}")))?;
            let kind = Kind::of_item(item).ok_or_else(|| {
                not_a_default(format!(
                    "{id} is part of the mesh; defaults are for {}",
                    kinds()
                ))
            })?;
            let v = item_json(s, item)
                .ok_or_else(|| Diagnostic::new("TOOL002", format!("no item {id}")))?;
            (kind, Some(v))
        }
        (None, Some(k)) => {
            let kind = Kind::from_key(k)
                .ok_or_else(|| not_a_default(format!("no kind {k}; there are {}", kinds())))?;
            let current = match kind {
                Kind::View => serde_json::to_value(View::of(s)).ok(),
                Kind::FieldStyle => i
                    .field
                    .clone()
                    .or_else(|| s.display.field.as_ref().map(|c| c.name.clone()))
                    .and_then(|name| serde_json::to_value(s.display.style_of(&name)).ok()),
                Kind::Player => s
                    .series
                    .as_ref()
                    .and_then(|r| serde_json::to_value(r.playback).ok()),
                _ => None,
            };
            (kind, current)
        }
        (None, None) => {
            return Err(Diagnostic::new(
                "TOOL002",
                format!(
                    "give item (an item's id, such as arrows or slice:1) or kind ({})",
                    kinds()
                ),
            ));
        }
    };
    if i.field.is_some() && kind != Kind::FieldStyle {
        return Err(Diagnostic::new(
            "TOOL002",
            "field names the field whose style to keep; it goes with kind field_style",
        ));
    }
    if let Some(k) = i.keys.iter().find(|k| !kind.keys().contains(&k.as_str())) {
        return Err(not_a_default(format!(
            "{k} cannot be a default of {}; these can: {}",
            kind.key(),
            kind.keys().join(", ")
        )));
    }
    let mut d = s.defaults.clone();
    let mut saved = Map::new();
    let mut forgot: Vec<String> = Vec::new();
    match i.values {
        Some(values) => {
            for (k, v) in values {
                d.set(kind, &k, v.clone()).map_err(not_a_default)?;
                saved.insert(k, v);
            }
        }
        None => {
            let current = current.ok_or_else(|| {
                Diagnostic::new(
                    "TOOL002",
                    format!(
                        "{} needs values, or {} to take them from",
                        kind.key(),
                        if kind == Kind::FieldStyle {
                            "a field (or one shown)"
                        } else {
                            "item"
                        }
                    ),
                )
            })?;
            // A value the item leaves out is at its built-in default:
            // saving it forgets the default instead.
            for (k, v) in values_of(kind, &current, &i.keys) {
                match v {
                    Some(v) => {
                        d.set(kind, &k, v.clone()).map_err(not_a_default)?;
                        saved.insert(k, v);
                    }
                    None => {
                        d.reset(Some(kind), Some(&k));
                        forgot.push(k);
                    }
                }
            }
        }
    }
    s.defaults = d;
    let path = save_session_settings(s)?;
    Ok(json!({
        "kind": kind.key(),
        "saved": saved,
        "forgot": forgot,
        "defaults": s.defaults,
        "files": [path],
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetDefault {
    kind: Option<String>,
    key: Option<String>,
}

pub(crate) fn settings_reset_default(input: Value, s: &mut Session) -> Run {
    let i: ResetDefault = parse(input)?;
    let kind = match &i.kind {
        Some(k) => Some(
            Kind::from_key(k)
                .ok_or_else(|| not_a_default(format!("no kind {k}; there are {}", kinds())))?,
        ),
        None => None,
    };
    if let Some(key) = &i.key {
        let Some(kind) = kind else {
            return Err(Diagnostic::new("TOOL002", "key goes with kind"));
        };
        if !kind.keys().contains(&key.as_str()) {
            return Err(not_a_default(format!(
                "{key} cannot be a default of {}; these can: {}",
                kind.key(),
                kind.keys().join(", ")
            )));
        }
    }
    let forgot = s.defaults.reset(kind, i.key.as_deref());
    let path = save_session_settings(s)?;
    Ok(json!({"forgot": forgot, "defaults": s.defaults, "files": [path]}))
}

/// The kinds and the values each can keep as a default.
pub(crate) fn defaultable() -> Value {
    Value::Object(
        Kind::ALL
            .into_iter()
            .map(|k| (k.key().to_string(), json!(k.keys())))
            .collect(),
    )
}

pub(crate) fn set_default_schema() -> Value {
    object(
        json!({
            "kind": {"enum": Kind::ALL.map(Kind::key)},
            "item": {"type": "string"},
            "field": {"type": "string"},
            "keys": {"type": "array", "items": {"type": "string"}},
            "values": {"type": "object"}
        }),
        &[],
    )
}

pub(crate) fn reset_default_schema() -> Value {
    object(
        json!({
            "kind": {"enum": Kind::ALL.map(Kind::key)},
            "key": {"type": "string"}
        }),
        &[],
    )
}
