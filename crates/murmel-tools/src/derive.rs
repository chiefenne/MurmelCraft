//! The tools of derived fields: `field.derive` computes one (or removes
//! it), `field.calculate` computes the field of a formula of your own,
//! `settings.keep_formula` and `settings.forget_formula` keep your
//! formulas for every result, and `field.catalogue` lists what can be
//! derived and calculated from the open result. The computing is the
//! engine's ([`murmel_engine::derive`], [`murmel_engine::calc`]).

use murmel_engine::Session;
use murmel_engine::calc::{self, FUNCTIONS, Formula, MAX_FORMULA, MAX_NAME};
use murmel_engine::derive::{self, Group, Input, Quantity, Recipe, Reference, Refusal, Source};
use murmel_mesh::Component;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{need_fields, need_mesh, object, parse, save_session_settings};
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

/// The derived or calculated field `name` of `s` as JSON: its recipe
/// or formula, and its places, each with its unit, range and how many
/// entities have no value.
fn derived_json(s: &Session, name: &str) -> Value {
    let Some(set) = s.fields() else {
        return Value::Null;
    };
    let places: Vec<Value> = set
        .fields()
        .iter()
        .filter(|f| f.name == name)
        .map(|f| {
            let k = f.components.max(1);
            let none = f
                .values
                .chunks(k)
                .filter(|v| v.iter().any(|x| !x.is_finite()))
                .count();
            json!({
                "location": f.location,
                "components": f.components,
                "unit": f.unit,
                "range": f.range(Component::Magnitude).map(|(a, b)| [a, b]),
                "no_value": none,
            })
        })
        .collect();
    let mut out = json!({
        "field": name,
        "places": places,
    });
    if let Some(r) = set.recipe(name) {
        out["recipe"] = json!(r);
    }
    if let Some(f) = set.formula(name) {
        out["formula"] = json!(f);
        out["reads"] = json!(calc::reads(&f.formula, set));
    }
    out
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CalculateInput {
    name: String,
    #[serde(default)]
    formula: Option<String>,
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    remove: bool,
}

pub(crate) fn field_calculate_schema() -> Value {
    object(
        json!({
            "name": {"type": "string", "maxLength": MAX_NAME, "description": "the field it makes: a letter or _ first, then letters, digits and _"},
            "formula": {"type": "string", "maxLength": MAX_FORMULA, "description": "left out: your formula of this name (settings.keep_formula)"},
            "unit": {"type": "string", "maxLength": MAX_NAME},
            "remove": {"type": "boolean"}
        }),
        &["name"],
    )
}

pub(crate) fn field_calculate(input: Value, s: &mut Session) -> Run {
    let i: CalculateInput = parse(input)?;
    need_fields(s)?;
    if i.remove {
        if i.formula.is_some() || i.unit.is_some() {
            return Err(Diagnostic::new("TOOL002", "remove takes the name alone"));
        }
        if s.fields().and_then(|f| f.formula(&i.name)).is_none() {
            return Err(refused(Refusal::NotDerived(i.name)));
        }
        s.underive(&i.name).map_err(refused)?;
        return Ok(json!({"removed": i.name}));
    }
    let f = match i.formula {
        Some(formula) => Formula {
            name: i.name,
            formula,
            unit: i.unit,
        },
        None => {
            let Some(mut kept) = s.formulas.iter().find(|f| f.name == i.name).cloned() else {
                return Err(Diagnostic::new(
                    "TOOL002",
                    format!(
                        "give a formula, or the name of one of your formulas ({})",
                        if s.formulas.is_empty() {
                            "you have none yet; settings.keep_formula keeps one".to_string()
                        } else {
                            s.formulas
                                .iter()
                                .map(|f| f.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    ),
                ));
            };
            if i.unit.is_some() {
                kept.unit = i.unit;
            }
            kept
        }
    };
    let name = f.name.clone();
    let names = s.calculate(f).map_err(refused)?;
    let mut out = derived_json(s, &name);
    out["made_again"] = json!(names.get(1..).unwrap_or_default());
    Ok(out)
}

/// The most formulas you can keep.
pub const MAX_FORMULAS: usize = 200;

pub(crate) fn keep_formula_schema() -> Value {
    object(
        json!({
            "name": {"type": "string", "maxLength": MAX_NAME},
            "formula": {"type": "string", "maxLength": MAX_FORMULA},
            "unit": {"type": "string", "maxLength": MAX_NAME}
        }),
        &["name", "formula"],
    )
}

pub(crate) fn settings_keep_formula(input: Value, s: &mut Session) -> Run {
    let f: Formula = parse(input)?;
    calc::check(&f).map_err(refused)?;
    let full = s.formulas.len() >= MAX_FORMULAS;
    match s.formulas.iter_mut().find(|k| k.name == f.name) {
        Some(k) => *k = f,
        None if full => {
            return Err(Diagnostic::new(
                "TOOL002",
                format!("you keep {MAX_FORMULAS} formulas already; forget one first"),
            ));
        }
        None => s.formulas.push(f),
    }
    let path = save_session_settings(s)?;
    Ok(json!({"formulas": s.formulas, "files": [path]}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ForgetFormula {
    name: String,
}

pub(crate) fn forget_formula_schema() -> Value {
    object(json!({"name": {"type": "string"}}), &["name"])
}

pub(crate) fn settings_forget_formula(input: Value, s: &mut Session) -> Run {
    let i: ForgetFormula = parse(input)?;
    let before = s.formulas.len();
    s.formulas.retain(|f| f.name != i.name);
    if s.formulas.len() == before {
        return Err(Diagnostic::new(
            "TOOL002",
            format!("you keep no formula {}", i.name),
        ));
    }
    let path = save_session_settings(s)?;
    Ok(json!({"formulas": s.formulas, "files": [path]}))
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
    let formulas: Vec<Value> = s
        .formulas
        .iter()
        .map(|f| {
            json!({
                "name": f.name,
                "formula": f.formula,
                "unit": f.unit,
                "computed": set.formula(&f.name).is_some_and(|m| m.formula == f.formula),
            })
        })
        .collect();
    let calculated: Vec<Value> = set
        .made()
        .iter()
        .filter_map(|m| match m {
            derive::Made::Formula(f) => Some(json!({
                "name": f.name,
                "formula": f.formula,
                "unit": f.unit,
                "reads": calc::reads(&f.formula, set),
                "kept": s.formulas.iter().any(|k| k == f),
            })),
            derive::Made::Quantity(_) => None,
        })
        .collect();
    Ok(json!({
        "groups": Group::ALL.map(Group::key),
        "quantities": quantities,
        "formulas": formulas,
        "calculated": calculated,
        "functions": FUNCTIONS,
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
