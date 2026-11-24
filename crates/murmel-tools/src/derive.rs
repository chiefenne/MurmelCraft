//! The tools of derived fields: `field.derive` computes one (or removes
//! it) and `field.catalogue` lists what can be derived from the open
//! result. The computing is the engine's ([`murmel_engine::derive`]).

use murmel_engine::Session;
use murmel_engine::derive::{self, Group, Input, Quantity, Recipe, Reference, Refusal, Source};
use murmel_mesh::Component;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{need_fields, need_mesh, object, parse};
use crate::{Diagnostic, Run};

fn refused(r: Refusal) -> Diagnostic {
    Diagnostic::new(r.code(), r.message())
}

/// Reference values given in part; the rest start as
/// [`derive::start_reference`] says.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartReference {
    density: Option<f64>,
    pressure: Option<f64>,
    velocity: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeriveInput {
    quantity: String,
    #[serde(default)]
    velocity: Option<String>,
    #[serde(default)]
    pressure: Option<String>,
    #[serde(default)]
    reference: Option<PartReference>,
    #[serde(default)]
    remove: bool,
}

pub(crate) fn field_derive_schema() -> Value {
    object(
        json!({
            "quantity": {"enum": Quantity::ALL.map(Quantity::key)},
            "velocity": {"type": "string", "description": "the velocity field (a vector); left out: found by its name"},
            "pressure": {"type": "string", "description": "the pressure field (a scalar); left out: found by its name"},
            "reference": object(json!({
                "density": {"type": "number", "exclusiveMinimum": 0},
                "pressure": {"type": "number"},
                "velocity": {"type": "number", "exclusiveMinimum": 0}
            }), &[]),
            "remove": {"type": "boolean"}
        }),
        &["quantity"],
    )
}

/// The derived field `name` of `s` as JSON: its recipe, places, unit and
/// range.
fn derived_json(s: &Session, name: &str) -> Value {
    let Some(set) = s.fields() else {
        return Value::Null;
    };
    let places: Vec<Value> = set
        .fields()
        .iter()
        .filter(|f| f.name == name)
        .map(|f| {
            json!({
                "location": f.location,
                "components": f.components,
                "unit": f.unit,
                "range": f.range(Component::Magnitude).map(|(a, b)| [a, b]),
            })
        })
        .collect();
    json!({
        "field": name,
        "recipe": set.recipe(name),
        "places": places,
    })
}

pub(crate) fn field_derive(input: Value, s: &mut Session) -> Run {
    let i: DeriveInput = parse(input)?;
    let q = Quantity::parse(&i.quantity).ok_or_else(|| {
        Diagnostic::new(
            "TOOL002",
            format!(
                "no quantity {}; there are {}",
                i.quantity,
                Quantity::ALL.map(Quantity::key).join(", ")
            ),
        )
    })?;
    need_fields(s)?;
    if i.remove {
        if i.velocity.is_some() || i.pressure.is_some() || i.reference.is_some() {
            return Err(Diagnostic::new(
                "TOOL002",
                "remove takes the quantity alone",
            ));
        }
        s.underive(q.field()).map_err(refused)?;
        return Ok(json!({"removed": q.field()}));
    }
    let mut recipe = Recipe::new(q);
    recipe.velocity = i.velocity;
    recipe.pressure = i.pressure;
    let mut sources = None;
    if !q.references().is_empty() {
        // Values given win; the rest are the field's own when it is
        // made again, else where new fields start from.
        let set = s.fields();
        let (start, from) = match set
            .and_then(|f| f.recipe(q.field()))
            .and_then(|r| r.reference)
        {
            Some(r) => (r, [None; 3]),
            None => {
                let (r, f) = derive::start_reference(s);
                (r, f.map(Some))
            }
        };
        let part = i.reference.unwrap_or_default();
        let r = Reference {
            density: part.density.unwrap_or(start.density),
            pressure: part.pressure.unwrap_or(start.pressure),
            velocity: part.velocity.unwrap_or(start.velocity),
        };
        let given = [part.density, part.pressure, part.velocity].map(|x| x.is_some());
        sources = Some(
            ["density", "pressure", "velocity"]
                .into_iter()
                .zip(given.into_iter().zip(from))
                .map(|(k, (given, from))| {
                    let src = if given {
                        json!("given")
                    } else {
                        from.map_or(json!("kept"), |f| json!(f))
                    };
                    (k.to_string(), src)
                })
                .collect::<serde_json::Map<_, _>>(),
        );
        recipe.reference = Some(r);
    } else if i.reference.is_some() {
        return Err(Diagnostic::new(
            "TOOL002",
            format!("{} takes no reference values", q.key()),
        ));
    }
    let name = s.derive(recipe).map_err(refused)?;
    let mut out = derived_json(s, name);
    if let Some(src) = sources {
        out["reference_from"] = Value::Object(src);
    }
    Ok(out)
}

/// What the catalogue says of `input` for quantity `q`: the field it
/// reads now and the fields that could be it.
fn input_json(s: &Session, recipe: &Recipe, input: Input) -> Value {
    let Some(set) = s.fields() else {
        return Value::Null;
    };
    json!({
        "input": input.key(),
        "field": recipe.input(set, input),
        "candidates": derive::candidates(set, input),
    })
}

pub(crate) fn field_catalogue(_: Value, s: &mut Session) -> Run {
    need_mesh(s)?;
    let Some(set) = s.fields() else {
        return Ok(json!({"quantities": []}));
    };
    let quantities: Vec<Value> = Quantity::ALL
        .into_iter()
        .map(|q| {
            let recipe = set
                .recipe(q.field())
                .cloned()
                .unwrap_or_else(|| Recipe::new(q));
            let missing: Vec<&str> = q
                .inputs()
                .iter()
                .filter(|&&i| recipe.input(set, i).is_none())
                .map(|i| i.key())
                .collect();
            let in_file =
                set.recipe(q.field()).is_none() && set.fields().iter().any(|f| f.name == q.field());
            json!({
                "quantity": q.key(),
                "group": q.group(),
                "field": q.field(),
                "formula": q.formula(),
                "unit": q.unit(),
                "inputs": q.inputs().iter().map(|&i| input_json(s, &recipe, i)).collect::<Vec<_>>(),
                "references": q.references(),
                "missing": missing,
                "available": missing.is_empty() && !in_file,
                "in_file": in_file,
                "derived": set.recipe(q.field()),
            })
        })
        .collect();
    let (r, from) = derive::start_reference(s);
    Ok(json!({
        "groups": Group::ALL.map(Group::key),
        "quantities": quantities,
        "reference": {
            "density": r.density,
            "pressure": r.pressure,
            "velocity": r.velocity,
            "from": {
                "density": from[0],
                "pressure": from[1],
                "velocity": from[2],
            },
        },
        "sources": [Source::Case, Source::Defaults, Source::BuiltIn],
    }))
}
