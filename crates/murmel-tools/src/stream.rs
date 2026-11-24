//! The tools of streamlines (field results, stage 4): `stream.set` and
//! `stream.clear`. Tracing and drawing are the engine's
//! ([`murmel_engine::stream`]).

use murmel_engine::stream::{
    Direction, Look, MAX_COUNT, MAX_STREAMS, MIN_COUNT, Seed, Stream, color_range,
};
use murmel_engine::{FieldChoice, Item, Session};
use murmel_mesh::Component;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{display_json, need_fields, object, parse};
use crate::{Diagnostic, Run};

/// Streamline set `k` (0-based) with its id, its settings and how its
/// lines were traced.
pub(crate) fn stream_json(s: &Session, k: usize) -> Value {
    let (Some(st), Some(m)) = (s.display.streams.get(k), &s.mesh) else {
        return Value::Null;
    };
    let traced = s
        .display
        .stream_traces
        .as_ref()
        .and_then(|t| t.0.get(k).cloned().flatten())
        .filter(|t| t.fits(st));
    let color = st.color.as_ref().map(|c| {
        let f = m.fields.get(&c.name);
        let vector = f.is_some_and(|f| f.is_vector());
        json!({
            "field": c.name,
            "component": c.component,
            "title": c.title(vector),
            "unit": f.and_then(|f| f.unit.clone()),
            "range": color_range(&s.display, &m.mesh, Some(&m.fields), k),
        })
    });
    json!({
        "id": Item::Stream(k as u32).id(&m.mesh),
        "stream": k + 1,
        "enabled": st.enabled,
        "field": st.field,
        "seed": st.seed,
        "count": st.count,
        "direction": st.direction,
        "max_length": st.length_limit(&m.mesh),
        "look": st.look,
        "tube_radius": st.tube_radius(&m.mesh),
        "arrowheads": st.arrowheads,
        "color": color,
        "solid": murmel_engine::stream::solid_color(&s.display, &s.tokens, k).hex(),
        "current": s.display.current_stream() == Some(k),
        "traced": traced.map(|t| json!({
            "seeds": t.seeds,
            "seeds_outside": t.outside,
            "lines": t.lines.len(),
            "points": t.points,
            "ended": t.ends,
            "problem": t.problem,
        })),
    })
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Line,
    Sphere,
    Patch,
    Plane,
}

impl Kind {
    fn key(self) -> &'static str {
        match self {
            Kind::Line => "line",
            Kind::Sphere => "sphere",
            Kind::Patch => "patch",
            Kind::Plane => "plane",
        }
    }
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Colour {
    Solid,
    Field,
}

/// A value, or `null` to go back to the default.
#[derive(Deserialize)]
#[serde(untagged)]
enum OrDefault {
    Value(f64),
    Default(()),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamSet {
    stream: Option<u32>,
    #[serde(default)]
    new: bool,
    field: Option<String>,
    seed: Option<Kind>,
    from: Option<[f64; 3]>,
    to: Option<[f64; 3]>,
    centre: Option<[f64; 3]>,
    radius: Option<f64>,
    patch: Option<String>,
    origin: Option<[f64; 3]>,
    normal: Option<[f64; 3]>,
    count: Option<u32>,
    direction: Option<Direction>,
    max_length: Option<OrDefault>,
    look: Option<Look>,
    tube_radius: Option<OrDefault>,
    arrowheads: Option<bool>,
    color: Option<Colour>,
    color_field: Option<String>,
    component: Option<Component>,
    enabled: Option<bool>,
}

/// The index of set `n` (counted from 1), or of the current set.
fn stream_index(s: &Session, n: Option<u32>) -> Result<Option<usize>, Diagnostic> {
    match n {
        None => Ok(s.display.current_stream()),
        Some(n) if n >= 1 && (n as usize) <= s.display.streams.len() => Ok(Some(n as usize - 1)),
        Some(n) => Err(Diagnostic::new(
            "STREAM001",
            format!(
                "no streamline set {n}; there are {} (stream:1 is the first)",
                s.display.streams.len()
            ),
        )),
    }
}

fn bad(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new("STREAM002", message)
}

/// The vector field traced: the one named, else the set's own, else the
/// field shown when it is a vector, else the file's first vector.
fn vector_field(s: &Session, name: Option<&str>, own: Option<&str>) -> Result<String, Diagnostic> {
    let set = need_fields(s)?;
    let vectors: Vec<&str> = set
        .names()
        .into_iter()
        .filter(|n| set.get(n).is_some_and(|f| f.is_vector()))
        .collect();
    let name = name
        .or(own)
        .map(str::to_string)
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
            "STREAM003",
            format!(
                "the open mesh has no vector field (one with x, y and z components); it has {}",
                set.names().join(", ")
            ),
        ));
    };
    match set.get(&name) {
        Some(f) if f.is_vector() => Ok(name),
        Some(_) => Err(Diagnostic::new(
            "STREAM003",
            format!(
                "{name} is a scalar; streamlines need a vector field ({})",
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

/// The seed shape `i` asks for: a new shape of the kind named (placed for
/// the mesh), or `old` changed by the values given; a value of another
/// shape is refused.
fn seed_of(s: &Session, i: &StreamSet, old: Option<&Seed>) -> Result<Seed, Diagnostic> {
    let Some(m) = &s.mesh else {
        return Err(bad("no mesh is open"));
    };
    let kind = i.seed.map(Kind::key);
    let mut seed = match (kind, old) {
        (Some(k), Some(o)) if o.key() == k => o.clone(),
        (Some(k), _) => Seed::default_for(k, &m.mesh)
            .ok_or_else(|| bad(format!("the mesh has nothing to seed a {k} on")))?,
        (None, Some(o)) => o.clone(),
        (None, None) => {
            let k = if i.centre.is_some() || i.radius.is_some() {
                "sphere"
            } else if i.patch.is_some() {
                "patch"
            } else if i.origin.is_some() || i.normal.is_some() {
                "plane"
            } else {
                "line"
            };
            Seed::default_for(k, &m.mesh)
                .ok_or_else(|| bad(format!("the mesh has nothing to seed a {k} on")))?
        }
    };
    let shape = seed.key();
    let wrong = |what: &str, of: &str| {
        bad(format!(
            "{what} belongs to a {of} seed; this set seeds on a {shape} (give seed: \"{of}\" to change it)"
        ))
    };
    match &mut seed {
        Seed::Line { from, to } => {
            if let Some(p) = i.from {
                *from = p;
            }
            if let Some(p) = i.to {
                *to = p;
            }
        }
        Seed::Sphere { centre, radius } => {
            if let Some(p) = i.centre {
                *centre = p;
            }
            if let Some(r) = i.radius {
                *radius = r;
            }
        }
        Seed::Patch { patch } => {
            if let Some(p) = &i.patch {
                patch.clone_from(p);
            }
        }
        Seed::Plane { origin, normal } => {
            if let Some(p) = i.origin {
                *origin = p;
            }
            if let Some(n) = i.normal {
                *normal = n;
            }
        }
    }
    if shape != "line" && (i.from.is_some() || i.to.is_some()) {
        return Err(wrong("from and to", "line"));
    }
    if shape != "sphere" && (i.centre.is_some() || i.radius.is_some()) {
        return Err(wrong("centre and radius", "sphere"));
    }
    if shape != "patch" && i.patch.is_some() {
        return Err(wrong("patch", "patch"));
    }
    if shape != "plane" && (i.origin.is_some() || i.normal.is_some()) {
        return Err(wrong("origin and normal", "plane"));
    }
    if let Some(why) = seed.problem(&m.mesh) {
        return Err(bad(match &seed {
            Seed::Patch { .. } => format!(
                "{why}; the mesh has {}",
                m.mesh
                    .patches
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => why,
        }));
    }
    Ok(seed)
}

pub(crate) fn stream_set(input: Value, s: &mut Session) -> Run {
    let i: StreamSet = parse(input)?;
    need_fields(s)?;
    if i.new && i.stream.is_some() {
        return Err(Diagnostic::new(
            "TOOL002",
            "give either new or stream, not both",
        ));
    }
    let at = if i.new {
        None
    } else {
        stream_index(s, i.stream)?
    };
    if at.is_none() && s.display.streams.len() >= MAX_STREAMS {
        return Err(Diagnostic::new(
            "STREAM001",
            format!("{MAX_STREAMS} streamline sets are the most"),
        ));
    }
    let old = at.map(|k| s.display.streams[k].clone());
    let field = vector_field(
        s,
        i.field.as_deref(),
        old.as_ref().map(|o| o.field.as_str()),
    )?;
    let seed = seed_of(s, &i, old.as_ref().map(|o| &o.seed))?;
    let mut st = old.clone().unwrap_or_else(|| {
        crate::defaults::fresh(
            s,
            murmel_engine::defaults::Kind::Stream,
            Stream::new(field.clone(), seed.clone()),
        )
    });
    // A new traced field takes its magnitude as the colour when the set
    // was coloured by the old one's.
    if st.field != field
        && st
            .color
            .as_ref()
            .is_some_and(|c| c.name == st.field && i.color_field.is_none())
    {
        st.color = Some(FieldChoice {
            name: field.clone(),
            component: Component::Magnitude,
        });
    }
    st.field = field;
    st.seed = seed;
    if let Some(n) = i.count {
        if !(MIN_COUNT..=MAX_COUNT).contains(&n) {
            return Err(bad(format!(
                "count {n} is outside {MIN_COUNT} to {MAX_COUNT}"
            )));
        }
        st.count = n;
    }
    if let Some(d) = i.direction {
        st.direction = d;
    }
    match i.max_length {
        Some(OrDefault::Value(l)) => {
            if !(l.is_finite() && l > 0.0) {
                return Err(bad(format!("max_length {l} must be above 0")));
            }
            st.max_length = Some(l);
        }
        Some(OrDefault::Default(())) => st.max_length = None,
        None => {}
    }
    if let Some(l) = i.look {
        st.look = l;
    }
    match i.tube_radius {
        Some(OrDefault::Value(r)) => {
            if !(r.is_finite() && r > 0.0) {
                return Err(bad(format!("tube_radius {r} must be above 0")));
            }
            st.radius = Some(r);
        }
        Some(OrDefault::Default(())) => st.radius = None,
        None => {}
    }
    if let Some(a) = i.arrowheads {
        st.arrowheads = a;
    }
    if let Some(e) = i.enabled {
        st.enabled = e;
    }
    // The colour: solid, or a field (the one named, else the set's own,
    // else the traced field's magnitude).
    let wants_field = i.color == Some(Colour::Field) || i.color_field.is_some();
    if i.color == Some(Colour::Solid) {
        if i.color_field.is_some() || i.component.is_some() {
            return Err(Diagnostic::new(
                "TOOL002",
                "color solid takes no color_field or component",
            ));
        }
        st.color = None;
    } else if wants_field || i.component.is_some() {
        let name = i
            .color_field
            .clone()
            .or_else(|| st.color.as_ref().map(|c| c.name.clone()))
            .unwrap_or_else(|| st.field.clone());
        let set = need_fields(s)?;
        let Some(f) = set.get(&name) else {
            return Err(Diagnostic::new(
                "FIELD002",
                format!(
                    "no field {name}; the open mesh has {}",
                    set.names().join(", ")
                ),
            ));
        };
        let component = match i.component {
            Some(c) if f.is_vector() => c,
            Some(c) if c != Component::Magnitude => {
                return Err(Diagnostic::new(
                    "FIELD003",
                    format!("{name} is a scalar: it has no component {c:?}"),
                ));
            }
            _ => st
                .color
                .as_ref()
                .filter(|c| c.name == name)
                .map_or(Component::Magnitude, |c| c.component),
        };
        st.color = Some(FieldChoice { name, component });
    }
    if let Some(why) = st.problem_alone() {
        return Err(bad(why));
    }
    if let Some(c) = &st.color {
        crate::defaults::adopt_style(s, &c.name.clone());
    }
    let k = match at {
        Some(k) => {
            s.display.streams[k] = st;
            k
        }
        None => {
            s.display
                .selection
                .retain(|i| !matches!(i, Item::Stream(_)));
            s.display.streams.push(st);
            s.display.streams.len() - 1
        }
    };
    s.redraw();
    let mut out = display_json(s);
    out["stream"] = stream_json(s, k);
    Ok(out)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamClear {
    stream: Option<u32>,
    #[serde(default)]
    all: bool,
}

pub(crate) fn stream_clear(input: Value, s: &mut Session) -> Run {
    let i: StreamClear = parse(input)?;
    if i.all {
        if i.stream.is_some() {
            return Err(Diagnostic::new(
                "TOOL002",
                "give either all or stream, not both",
            ));
        }
        while !s.display.streams.is_empty() {
            s.display.remove_stream(s.display.streams.len() - 1);
        }
    } else {
        match stream_index(s, i.stream)? {
            Some(k) => {
                s.display.remove_stream(k);
            }
            None => {
                return Err(Diagnostic::new(
                    "STREAM001",
                    "there are no streamlines to remove",
                ));
            }
        }
    }
    s.redraw();
    Ok(display_json(s))
}

// ---- schemas ------------------------------------------------------------

fn vec3_schema() -> Value {
    json!({"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3})
}

fn positive_or_null() -> Value {
    json!({"oneOf": [{"type": "number", "exclusiveMinimum": 0}, {"type": "null"}]})
}

pub(crate) fn stream_set_schema() -> Value {
    object(
        json!({
            "stream": {"type": "integer", "minimum": 1},
            "new": {"type": "boolean"},
            "field": {"type": "string"},
            "seed": {"enum": ["line", "sphere", "patch", "plane"]},
            "from": vec3_schema(),
            "to": vec3_schema(),
            "centre": vec3_schema(),
            "radius": {"type": "number", "exclusiveMinimum": 0},
            "patch": {"type": "string"},
            "origin": vec3_schema(),
            "normal": vec3_schema(),
            "count": {"type": "integer", "minimum": MIN_COUNT, "maximum": MAX_COUNT},
            "direction": {"enum": ["forward", "backward", "both"]},
            "max_length": positive_or_null(),
            "look": {"enum": ["lines", "tubes"]},
            "tube_radius": positive_or_null(),
            "arrowheads": {"type": "boolean"},
            "color": {"enum": ["solid", "field"]},
            "color_field": {"type": "string"},
            "component": {"enum": ["magnitude", "x", "y", "z"]},
            "enabled": {"type": "boolean"}
        }),
        &[],
    )
}

pub(crate) fn stream_clear_schema() -> Value {
    object(
        json!({"stream": {"type": "integer", "minimum": 1}, "all": {"type": "boolean"}}),
        &[],
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use murmel_engine::{Item, Session, Tokens};
    use serde_json::json;

    use crate::call;

    fn channel() -> Session {
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir = None;
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
        assert!(call("mesh.open", json!({ "path": path }), &mut s).ok());
        s
    }

    fn code(e: &crate::Envelope) -> String {
        e.diagnostics
            .first()
            .map(|d| d.code.to_string())
            .unwrap_or_default()
    }

    /// Invariant: `stream.set` traces every seed shape of the duct's
    /// velocity and reports what it traced (seeds placed, lines, how they
    /// ended); a new set becomes the current one and the next call
    /// without `stream` changes it; a scalar field, a count out of range,
    /// a shape's points given to another shape and a ninth set are
    /// refused with their codes and change nothing; `stream.clear` removes
    /// one set or all, and each call is one undo step. Breaks if the tool
    /// stops tracing, reports a partial result as whole, or edits the
    /// wrong set; the engine tests trace without the tool.
    #[test]
    fn stream_set_traces_every_seed_shape() {
        let mut s = channel();
        for (k, seed) in ["line", "sphere", "patch", "plane"].into_iter().enumerate() {
            let e = call(
                "stream.set",
                json!({"new": true, "seed": seed, "count": 16}),
                &mut s,
            );
            assert!(e.ok(), "{seed}: {:?}", e.diagnostics);
            let st = &e.result["stream"];
            assert_eq!(st["id"], format!("stream:{}", k + 1));
            assert_eq!(st["seed"]["shape"], seed, "{st}");
            assert_eq!(st["current"], true);
            let t = &st["traced"];
            let lines = t["lines"].as_u64().unwrap();
            assert!(lines > 0, "{seed}: {t}");
            assert_eq!(
                t["seeds"].as_u64().unwrap(),
                lines,
                "{seed}: one line per seed"
            );
            let ended = &t["ended"];
            let sum: u64 = ["left", "slow", "length", "steps", "lost"]
                .iter()
                .map(|k| ended[k].as_u64().unwrap())
                .sum();
            // Traced both ways: two ends per line.
            assert_eq!(sum, 2 * lines, "{seed}: {ended}");
            assert!(st["color"]["range"].is_array(), "{st}");
        }
        assert_eq!(s.display.current_stream(), Some(3));

        // Without `stream`, the current set changes.
        let e = call(
            "stream.set",
            json!({"look": "tubes", "direction": "forward"}),
            &mut s,
        );
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(s.display.streams[3].look, murmel_engine::Look::Tubes);
        assert_eq!(s.display.streams[0].look, murmel_engine::Look::Lines);
        let ended = &e.result["stream"]["traced"]["ended"];
        assert!(ended["left"].as_u64().unwrap() > 0, "{ended}");

        // Refusals change nothing.
        let before = s.display.streams.clone();
        for (input, want) in [
            (json!({"field": "pressure"}), "STREAM003"),
            (json!({"count": 0}), "STREAM002"),
            (json!({"count": 5000}), "STREAM002"),
            (json!({"stream": 1, "centre": [0.0, 0.0, 0.0]}), "STREAM002"),
            (json!({"stream": 9}), "STREAM001"),
            (
                json!({"stream": 1, "seed": "patch", "patch": "nowhere"}),
                "STREAM002",
            ),
        ] {
            let e = call("stream.set", input.clone(), &mut s);
            assert!(!e.ok(), "{input}");
            assert_eq!(code(&e), want, "{input}: {:?}", e.diagnostics);
        }
        for _ in 4..murmel_engine::stream::MAX_STREAMS {
            assert!(call("stream.set", json!({"new": true}), &mut s).ok());
        }
        let e = call("stream.set", json!({"new": true}), &mut s);
        assert_eq!(code(&e), "STREAM001");
        assert_eq!(&s.display.streams[..4], &before[..]);

        // Clear one, then all; undo brings them back.
        let n = s.display.streams.len();
        assert!(call("stream.clear", json!({"stream": 2}), &mut s).ok());
        assert_eq!(s.display.streams.len(), n - 1);
        assert!(call("stream.clear", json!({"all": true}), &mut s).ok());
        assert!(s.display.streams.is_empty());
        assert_eq!(code(&call("stream.clear", json!({}), &mut s)), "STREAM001");
        assert!(call("journal.undo", json!({}), &mut s).ok());
        assert_eq!(s.display.streams.len(), n - 1);
        assert!(call("journal.undo", json!({}), &mut s).ok());
        assert_eq!(s.display.streams.len(), n);
    }

    /// Invariant: the colour follows the tool: `color_field` colours a
    /// set by another field (a vector by its magnitude unless a component
    /// is given), `solid` takes the token or the item's own colour, and
    /// tracing another vector field moves a colour that followed the old
    /// one; selecting a set makes it the current one. Breaks if the
    /// colour choice is lost or a scalar takes a component.
    #[test]
    fn stream_colours() {
        let mut s = channel();
        assert!(call("stream.set", json!({"new": true, "count": 8}), &mut s).ok());
        assert!(call("stream.set", json!({"new": true, "count": 8}), &mut s).ok());
        let e = call(
            "stream.set",
            json!({"stream": 1, "color_field": "pressure"}),
            &mut s,
        );
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(e.result["stream"]["color"]["title"], "pressure");
        let e = call(
            "stream.set",
            json!({"stream": 1, "color_field": "velocity", "component": "y"}),
            &mut s,
        );
        assert_eq!(e.result["stream"]["color"]["title"], "velocity_y");
        let e = call(
            "stream.set",
            json!({"stream": 1, "color_field": "pressure", "component": "x"}),
            &mut s,
        );
        assert_eq!(code(&e), "FIELD003");
        let e = call("stream.set", json!({"stream": 1, "color": "solid"}), &mut s);
        assert!(e.ok());
        assert!(e.result["stream"]["color"].is_null());
        assert_eq!(
            e.result["stream"]["solid"],
            s.tokens.mesh.field.stream.hex()
        );
        let e = call(
            "display.set_color",
            json!({"item": "stream:1", "color": "#102030"}),
            &mut s,
        );
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(
            call("stream.set", json!({"stream": 1}), &mut s).result["stream"]["solid"],
            "#102030"
        );
        assert!(call("select.item", json!({"item": "stream:1"}), &mut s).ok());
        assert_eq!(s.display.current_stream(), Some(0));
        assert_eq!(s.display.selection, [Item::Stream(0)]);
    }

    /// Invariant (session format 1.9): streamline sets come back from a
    /// session file with the same settings and the same traced lines; a
    /// set seeded on a patch the mesh does not have is skipped and
    /// reported. Breaks if a setting is not saved or restored; the tool
    /// tests above never save.
    #[test]
    fn streams_round_trip_through_a_session() {
        let dir =
            std::env::temp_dir().join(format!("murmel-stream-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("streams.murmel");
        let mut s = channel();
        let a = json!({"new": true, "seed": "sphere", "count": 12, "look": "tubes",
                       "arrowheads": true, "direction": "backward", "max_length": 2.5});
        assert!(call("stream.set", a, &mut s).ok());
        let b = json!({"new": true, "seed": "patch", "patch": "inlet", "count": 9,
                       "color_field": "k", "enabled": false});
        assert!(call("stream.set", b, &mut s).ok());
        let e = call("session.save", json!({"path": file}), &mut s);
        assert!(e.ok(), "{:?}", e.diagnostics);
        let text = std::fs::read_to_string(&file).unwrap();
        let version = murmel_engine::session_file::SESSION_VERSION;
        assert!(
            text.contains(&format!("\"format_version\": \"{version}\"")),
            "{text}"
        );

        let mut t = Session::new(Tokens::builtin().unwrap());
        t.config_dir = None;
        let e = call("session.load", json!({"path": file}), &mut t);
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(t.display.streams, s.display.streams);
        let lines = |s: &Session| {
            s.display
                .stream_traces
                .as_ref()
                .map(|t| {
                    t.0.iter()
                        .map(|x| x.as_ref().map_or(0, |x| x.lines.len()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        assert_eq!(lines(&t), lines(&s));

        let text = text.replace("\"patch\": \"inlet\"", "\"patch\": \"nowhere\"");
        std::fs::write(&file, text).unwrap();
        let mut u = Session::new(Tokens::builtin().unwrap());
        u.config_dir = None;
        let e = call("session.load", json!({"path": file}), &mut u);
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(e.result["skipped"], json!(["stream:2"]));
        assert_eq!(u.display.streams.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
