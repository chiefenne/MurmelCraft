//! Your defaults (the maintainer's words of 2026-11-24, "each
//! configurable value can be set interactively as default in the gui",
//! and their card): any value of an item's look or of how it is
//! computed can be saved as the value new items of that kind start from,
//! from the window (right-click a value, an item's row or a Properties
//! row) or by `settings.set_default`. Items already open keep theirs, so
//! a saved session never changes on its own; Settings lists the defaults
//! and resets each (`settings.reset_default`). They live in
//! `settings.toml` under `[defaults.<kind>]`, one key per value.
//!
//! Only values that make sense on any mesh can be defaults: an item's
//! look, density, count and mode, never where it is, which field it
//! shows or a length in metres; the reference values of derived
//! pressure fields (density, pressure, velocity) belong to the fluid,
//! not the mesh, so they can be too. [`Kind::keys`] lists them; a value is
//! checked against a fresh item of its kind and the kind's limits before
//! it is kept, and a value edited by hand that no longer fits is left out
//! when an item is made.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::camera::Projection;
use crate::clip::{Clip, ClipMode};
use crate::derive::Reference;
use crate::display::{Item, RenderMode};
use crate::extract::{Iso, MeshLook, Slice};
use crate::field::{FieldChoice, FieldStyle};
use crate::glyph::Arrows;
use crate::parcel::ParcelView;
use crate::region::{Region, Shape};
use crate::session::Session;
use crate::stream::{Seed, Stream};

/// What a default is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Arrows,
    Slice,
    Iso,
    Stream,
    Clip,
    Region,
    /// The colouring of a field newly shown (mode, bands, contour lines,
    /// colour map, range mode).
    FieldStyle,
    /// How the mesh is drawn when the window starts.
    View,
    /// The reference values derived pressure fields start from when the
    /// result's case does not give them.
    Reference,
    /// The look of spray parcels.
    Parcels,
    /// The speed and axis of rotation the turbomachinery fields start
    /// from when the result's case does not give them (the axis point
    /// and zone belong to one mesh).
    Rotation,
}

impl Kind {
    pub const ALL: [Kind; 11] = [
        Kind::Arrows,
        Kind::Slice,
        Kind::Iso,
        Kind::Stream,
        Kind::Clip,
        Kind::Region,
        Kind::FieldStyle,
        Kind::View,
        Kind::Reference,
        Kind::Parcels,
        Kind::Rotation,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Kind::Arrows => "arrows",
            Kind::Slice => "slice",
            Kind::Iso => "iso",
            Kind::Stream => "stream",
            Kind::Clip => "clip",
            Kind::Region => "region",
            Kind::FieldStyle => "field_style",
            Kind::View => "view",
            Kind::Reference => "reference",
            Kind::Parcels => "parcels",
            Kind::Rotation => "rotation",
        }
    }

    pub fn from_key(k: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|x| x.key() == k)
    }

    /// The values of this kind that can be defaults, by their names in
    /// the kind's tool and in session files.
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            Kind::Arrows => &["surface", "in_plane", "density", "scale", "length", "color"],
            Kind::Slice => &["count"],
            Kind::Iso => &["color"],
            Kind::Stream => &["count", "direction", "look", "arrowheads"],
            Kind::Clip => &["mode"],
            Kind::Region => &["cut"],
            Kind::FieldStyle => &[
                "mode",
                "bands",
                "lines",
                "line_count",
                "map",
                "reverse",
                "range",
            ],
            Kind::View => &["render_mode", "mesh_look", "projection", "show_blocks"],
            Kind::Reference => &crate::derive::REFERENCE_KEYS,
            Kind::Rotation => &["rpm", "axis"],
            Kind::Parcels => &["size", "scale", "map", "reverse"],
        }
    }

    /// The kind of a model item, `None` for items that are part of the
    /// mesh (patches, zones, cells) or have nothing to default.
    pub fn of_item(item: Item) -> Option<Kind> {
        Some(match item {
            Item::Arrows => Kind::Arrows,
            Item::Slice(_) => Kind::Slice,
            Item::Iso(_) => Kind::Iso,
            Item::Stream(_) => Kind::Stream,
            Item::Clip(_) => Kind::Clip,
            Item::Region(_) => Kind::Region,
            Item::Parcels => Kind::Parcels,
            _ => return None,
        })
    }

    /// A fresh item of this kind as JSON, to check a value against.
    fn sample(self) -> Value {
        let o = [0.0; 3];
        let x = [1.0, 0.0, 0.0];
        let v = match self {
            Kind::Arrows => serde_json::to_value(Arrows::new("v")),
            Kind::Slice => serde_json::to_value(Slice::new(o, x)),
            Kind::Iso => serde_json::to_value(Iso {
                field: FieldChoice {
                    name: "v".into(),
                    component: crate::field::Component::Magnitude,
                },
                value: 0.0,
                enabled: true,
                color: Default::default(),
            }),
            Kind::Stream => serde_json::to_value(Stream::new("v", Seed::Line { from: o, to: x })),
            Kind::Clip => serde_json::to_value(Clip {
                origin: o,
                normal: x,
                mode: ClipMode::Crinkle,
                enabled: true,
            }),
            Kind::Region => serde_json::to_value(Region::new(Shape::Sphere {
                center: o,
                radius: 1.0,
            })),
            Kind::FieldStyle => serde_json::to_value(FieldStyle::default()),
            Kind::View => serde_json::to_value(View::default()),
            Kind::Reference => serde_json::to_value(Reference::default()),
            Kind::Parcels => serde_json::to_value(ParcelView::new("p".into())),
            Kind::Rotation => serde_json::to_value(crate::turbo::Turning::default()),
        };
        v.unwrap_or(Value::Null)
    }

    /// Whether `value` fits key `key` of this kind: its type, and the
    /// limits the kind's tool checks.
    fn fits(self, key: &str, value: &Value) -> Result<(), String> {
        if !self.keys().contains(&key) {
            return Err(format!(
                "{key} cannot be a default of {}; these can: {}",
                self.key(),
                self.keys().join(", ")
            ));
        }
        // settings.toml has no null: a value at its built-in default is
        // forgotten instead.
        if value.is_null() {
            return Err(format!("{}.{key}: no value", self.key()));
        }
        let mut item = self.sample();
        if let Value::Object(o) = &mut item {
            o.insert(key.to_string(), value.clone());
        }
        let typed = match self {
            Kind::Arrows => serde_json::from_value::<Arrows>(item).map(|_| ()),
            Kind::Slice => serde_json::from_value::<Slice>(item).map(|_| ()),
            Kind::Iso => serde_json::from_value::<Iso>(item).map(|_| ()),
            Kind::Stream => serde_json::from_value::<Stream>(item).map(|_| ()),
            Kind::Clip => serde_json::from_value::<Clip>(item).map(|_| ()),
            Kind::Region => serde_json::from_value::<Region>(item).map(|_| ()),
            Kind::FieldStyle => serde_json::from_value::<FieldStyle>(item).map(|_| ()),
            Kind::View => serde_json::from_value::<View>(item).map(|_| ()),
            Kind::Reference => serde_json::from_value::<Reference>(item).map(|_| ()),
            Kind::Parcels => serde_json::from_value::<ParcelView>(item).map(|_| ()),
            Kind::Rotation => serde_json::from_value::<crate::turbo::Turning>(item)
                .map_err(|e| e.to_string())
                .and_then(|t| {
                    crate::turbo::Rotation {
                        zones: vec![t],
                        frame: None,
                    }
                    .check()
                })
                .map_err(serde::de::Error::custom),
        };
        typed.map_err(|e| format!("{}.{key} = {value}: {e}", self.key()))?;
        let number = value.as_f64().unwrap_or(f64::NAN);
        let within = |lo: f64, hi: f64| {
            if (lo..=hi).contains(&number) {
                Ok(())
            } else {
                Err(format!(
                    "{}.{key} = {value} is outside {lo} to {hi}",
                    self.key()
                ))
            }
        };
        match (self, key) {
            (Kind::Arrows, "density") => within(
                crate::glyph::MIN_DENSITY as f64,
                crate::glyph::MAX_DENSITY as f64,
            ),
            (Kind::Arrows, "scale") if number <= 0.0 => {
                Err(format!("arrows.scale = {value} must be above 0"))
            }
            (Kind::Arrows, "scale") => within(0.0, crate::glyph::MAX_SCALE),
            (Kind::Parcels, "scale") if number <= 0.0 => {
                Err(format!("parcels.scale = {value} must be above 0"))
            }
            (Kind::Parcels, "scale") => within(0.0, crate::parcel::MAX_SCALE),
            (Kind::Slice, "count") => within(1.0, crate::extract::MAX_SERIES as f64),
            (Kind::Stream, "count") => within(
                crate::stream::MIN_COUNT as f64,
                crate::stream::MAX_COUNT as f64,
            ),
            (Kind::FieldStyle, "bands") => within(
                crate::field::MIN_BANDS as f64,
                crate::field::MAX_BANDS as f64,
            ),
            (Kind::FieldStyle, "line_count") => within(1.0, crate::field::MAX_LINES as f64),
            (Kind::FieldStyle | Kind::Parcels, "map") => match value.as_str() {
                Some(m) if crate::colormap::get(m).is_none() => Err(format!(
                    "no colour map {m}; there are {}",
                    crate::colormap::ids().join(", ")
                )),
                _ => Ok(()),
            },
            (Kind::Reference, "density" | "velocity" | "viscosity" | "c_mu") if number <= 0.0 => {
                Err(format!("reference.{key} = {value} must be above 0"))
            }
            (Kind::Reference, _) => {
                within(-crate::derive::MAX_REFERENCE, crate::derive::MAX_REFERENCE)
            }
            // A fixed range belongs to one field.
            (Kind::FieldStyle, "range") if value.is_object() => Err(
                "a fixed range belongs to one field; a default range is field, shown or symmetric"
                    .into(),
            ),
            _ => Ok(()),
        }
    }
}

/// How the mesh is drawn, as a default for when the window starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub render_mode: RenderMode,
    pub mesh_look: MeshLook,
    pub projection: Projection,
    /// Whether the boxes of an overview's blocks show.
    #[serde(default)]
    pub show_blocks: bool,
}

impl View {
    /// The session's view now.
    pub fn of(s: &Session) -> View {
        View {
            render_mode: s.display.render_mode,
            mesh_look: s.display.mesh_look,
            projection: s.camera.projection,
            show_blocks: s.display.show_blocks,
        }
    }

    /// Put this view in place in `s`.
    pub fn put(self, s: &mut Session) {
        s.display.render_mode = self.render_mode;
        s.display.mesh_look = self.mesh_look;
        s.camera.projection = self.projection;
        s.display.show_blocks = self.show_blocks;
    }
}

/// Start `s` from the default view (the window, the command line and
/// the MCP server call it once their settings are read).
pub fn start_view(s: &mut Session) {
    let v = s.defaults.apply(Kind::View, View::of(s));
    v.put(s);
}

/// The defaults by kind and key (`[defaults.<kind>]` in settings.toml).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Defaults(pub BTreeMap<String, Map<String, Value>>);

impl Defaults {
    pub fn is_empty(&self) -> bool {
        self.0.values().all(Map::is_empty)
    }

    /// The default of `key` of `kind`, if one is saved.
    pub fn get(&self, kind: Kind, key: &str) -> Option<&Value> {
        self.0.get(kind.key())?.get(key)
    }

    /// The defaults of `kind`.
    pub fn of(&self, kind: Kind) -> Option<&Map<String, Value>> {
        self.0.get(kind.key()).filter(|m| !m.is_empty())
    }

    /// `item` with the defaults of `kind` laid over it, one value at a
    /// time; a value that does not fit (a file edited by hand) is left
    /// out, so the item stays whole.
    pub fn apply<T: Serialize + DeserializeOwned>(&self, kind: Kind, item: T) -> T {
        let Some(over) = self.of(kind) else {
            return item;
        };
        let Ok(mut json) = serde_json::to_value(&item) else {
            return item;
        };
        let mut out = item;
        for (key, value) in over {
            if kind.fits(key, value).is_err() {
                continue;
            }
            let mut next = json.clone();
            if let Value::Object(o) = &mut next {
                o.insert(key.clone(), value.clone());
            }
            if let Ok(t) = serde_json::from_value::<T>(next.clone()) {
                out = t;
                json = next;
            }
        }
        out
    }

    /// Keep `value` as the default of `key` of `kind`, once it fits.
    pub fn set(&mut self, kind: Kind, key: &str, value: Value) -> Result<(), String> {
        kind.fits(key, &value)?;
        self.0
            .entry(kind.key().to_string())
            .or_default()
            .insert(key.to_string(), value);
        Ok(())
    }

    /// Forget the default of `key` of `kind`, every default of `kind`
    /// (`key` `None`), or all (`kind` `None`); how many were forgotten.
    pub fn reset(&mut self, kind: Option<Kind>, key: Option<&str>) -> usize {
        let before = self.count();
        match (kind, key) {
            (None, _) => self.0.clear(),
            (Some(k), None) => {
                self.0.remove(k.key());
            }
            (Some(k), Some(key)) => {
                if let Some(m) = self.0.get_mut(k.key()) {
                    m.remove(key);
                }
            }
        }
        self.0.retain(|_, m| !m.is_empty());
        before - self.count()
    }

    /// How many values are saved.
    pub fn count(&self) -> usize {
        self.0.values().map(Map::len).sum()
    }

    /// The defaults as rows of (kind, key, value), in kind order.
    pub fn rows(&self) -> Vec<(Kind, String, Value)> {
        Kind::ALL
            .into_iter()
            .flat_map(|k| {
                self.of(k)
                    .into_iter()
                    .flatten()
                    .map(move |(key, v)| (k, key.clone(), v.clone()))
            })
            .collect()
    }
}

/// The values of `keys` (all the kind's when empty) of `item` as JSON
/// (`item` is an item of `kind`, already serialised). A key the item
/// leaves out is at its built-in default: `None` for it, so saving it
/// forgets the default instead.
pub fn values_of(kind: Kind, item: &Value, keys: &[String]) -> Vec<(String, Option<Value>)> {
    let wanted: Vec<String> = if keys.is_empty() {
        kind.keys().iter().map(|k| k.to_string()).collect()
    } else {
        keys.to_vec()
    };
    wanted
        .into_iter()
        .map(|k| {
            let v = item.get(&k).cloned();
            (k, v)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyph::{ArrowColor, Length};
    use serde_json::json;

    /// Invariant: a saved default is where a new item of its kind starts
    /// (here arrows: density, length and colour) while the rest keeps the
    /// built-in values; a value that does not fit (a key that is not a
    /// default, a wrong type, outside the tool's limits, a fixed range)
    /// is refused when set and left out when applied (a file edited by
    /// hand), and reset forgets one value, a kind or all. Breaks when
    /// defaults reach values tied to one mesh or a bad file breaks every
    /// new item.
    #[test]
    fn defaults_start_new_items() {
        let mut d = Defaults::default();
        assert!(d.is_empty());
        d.set(Kind::Arrows, "density", json!(60)).unwrap();
        d.set(Kind::Arrows, "length", json!("equal")).unwrap();
        d.set(Kind::Arrows, "color", json!("solid")).unwrap();
        let a = d.apply(Kind::Arrows, Arrows::new("velocity"));
        assert_eq!(a.density, 60);
        assert_eq!(a.length, Length::Equal);
        assert_eq!(a.color, ArrowColor::Solid);
        assert_eq!(a.field, "velocity");
        assert_eq!(a.scale, 1.0);
        for (kind, key, value) in [
            (Kind::Arrows, "field", json!("p")),
            (Kind::Arrows, "density", json!("many")),
            (Kind::Arrows, "density", json!(201)),
            (Kind::Arrows, "scale", json!(0.0)),
            (Kind::Slice, "origin", json!([0, 0, 0])),
            (Kind::FieldStyle, "map", json!("lava")),
            (
                Kind::FieldStyle,
                "range",
                json!({"fixed": {"min": 0, "max": 1}}),
            ),
            (Kind::FieldStyle, "map", Value::Null),
            (Kind::Stream, "count", json!(0)),
        ] {
            assert!(d.set(kind, key, value.clone()).is_err(), "{key} = {value}");
        }
        // Edited by hand: the bad value is left out, the rest applies.
        d.0.get_mut("arrows")
            .unwrap()
            .insert("scale".into(), json!(-3));
        let a = d.apply(Kind::Arrows, Arrows::new("velocity"));
        assert_eq!((a.density, a.scale), (60, 1.0));
        // Another kind is untouched; the field style and view apply too.
        d.set(Kind::FieldStyle, "bands", json!(12)).unwrap();
        d.set(Kind::FieldStyle, "range", json!("shown")).unwrap();
        d.set(Kind::View, "render_mode", json!("wireframe"))
            .unwrap();
        let st = d.apply(Kind::FieldStyle, FieldStyle::default());
        assert_eq!(st.bands, 12);
        assert_eq!(st.range, crate::field::RangeMode::Shown);
        let v = d.apply(Kind::View, View::default());
        assert_eq!(v.render_mode, RenderMode::Wireframe);
        let s = d.apply(Kind::Slice, Slice::new([0.0; 3], [0.0, 0.0, 1.0]));
        assert_eq!(s, Slice::new([0.0; 3], [0.0, 0.0, 1.0]));
        // Through TOML and back, as settings.toml keeps them.
        let text = toml::to_string(&d).unwrap();
        let back: Defaults = toml::from_str(&text).unwrap();
        assert_eq!(back.rows(), d.rows());
        // Reset one, a kind, all.
        assert_eq!(d.reset(Some(Kind::Arrows), Some("color")), 1);
        assert!(d.get(Kind::Arrows, "color").is_none());
        assert_eq!(d.reset(Some(Kind::Arrows), None), 3);
        assert!(d.of(Kind::Arrows).is_none());
        assert_eq!(d.reset(None, None), 3);
        assert!(d.is_empty());
    }

    /// Invariant: every key a kind offers fits a fresh item of that kind
    /// with that item's own value, and every kind's key is reachable from
    /// its tool's name. Breaks when a field is renamed and its default
    /// silently stops applying.
    #[test]
    fn every_key_fits_its_kind() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_key(kind.key()), Some(kind));
            let sample = kind.sample();
            let full = match kind {
                // Values a fresh item leaves out (at their defaults).
                Kind::Arrows => json!({"surface": false, "in_plane": false, "density": 40,
                                      "scale": 1.0, "length": "floor", "color": "magnitude"}),
                Kind::Slice => json!({"count": 1}),
                Kind::Iso => json!({"color": "solid"}),
                Kind::Stream => json!({"count": 20, "direction": "both", "look": "lines",
                                      "arrowheads": false}),
                Kind::Region => json!({"cut": "crinkle"}),
                Kind::FieldStyle => json!({"map": "viridis"}),
                Kind::Parcels => json!({"size": "diameter", "scale": 2.0, "map": "inferno",
                                       "reverse": true}),
                _ => sample.clone(),
            };
            for key in kind.keys() {
                let value = sample
                    .get(*key)
                    .or_else(|| full.get(*key))
                    .unwrap_or_else(|| panic!("{}.{key} not in the item", kind.key()));
                kind.fits(key, value)
                    .unwrap_or_else(|e| panic!("{}.{key}: {e}", kind.key()));
            }
        }
    }
}
