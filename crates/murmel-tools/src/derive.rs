//! The tools of derived fields: `field.derive` computes one (or removes
//! it), `field.calculate` computes the field of a formula of your own,
//! `settings.keep_formula` and `settings.forget_formula` keep your
//! formulas for every result, and `field.catalogue` lists what can be
//! derived and calculated from the open result. The computing is the
//! engine's ([`murmel_engine::derive`], [`murmel_engine::calc`]).

use murmel_engine::Session;
use murmel_engine::calc::{self, FUNCTIONS, Formula, MAX_FORMULA, MAX_NAME};
use murmel_engine::derive::{
    self, Group, Input, Quantity, REFERENCE_KEYS, Recipe, Refusal, Source,
};
use murmel_engine::turbo::{self, Rotation};
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
    viscosity: Option<f64>,
    c_mu: Option<f64>,
}

impl DeriveInput {
    /// The fields named for the inputs, in the order of [`Input::ALL`].
    fn inputs(&mut self) -> [(Input, Option<String>); 13] {
        [
            (Input::Velocity, self.velocity.take()),
            (Input::Pressure, self.pressure.take()),
            (Input::K, self.k.take()),
            (Input::Dissipation, self.dissipation.take()),
            (Input::NuT, self.nu_t.take()),
            (Input::RmsVelocity, self.rms_velocity.take()),
            (Input::CovarianceXy, self.covariance_xy.take()),
            (Input::CovarianceYz, self.covariance_yz.take()),
            (Input::CovarianceXz, self.covariance_xz.take()),
            (Input::MeanK, self.mean_k.take()),
            (Input::HeatFluxX, self.heat_flux_x.take()),
            (Input::HeatFluxY, self.heat_flux_y.take()),
            (Input::HeatFluxZ, self.heat_flux_z.take()),
        ]
    }
}

impl PartReference {
    fn get(&self, k: usize) -> Option<f64> {
        [
            self.density,
            self.pressure,
            self.velocity,
            self.viscosity,
            self.c_mu,
        ][k]
    }
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
    k: Option<String>,
    #[serde(default)]
    dissipation: Option<String>,
    #[serde(default)]
    nu_t: Option<String>,
    #[serde(default)]
    rms_velocity: Option<String>,
    #[serde(default)]
    covariance_xy: Option<String>,
    #[serde(default)]
    covariance_yz: Option<String>,
    #[serde(default)]
    covariance_xz: Option<String>,
    #[serde(default)]
    mean_k: Option<String>,
    #[serde(default)]
    heat_flux_x: Option<String>,
    #[serde(default)]
    heat_flux_y: Option<String>,
    #[serde(default)]
    heat_flux_z: Option<String>,
    #[serde(default)]
    reference: Option<PartReference>,
    #[serde(default)]
    rotation: Option<Rotation>,
    #[serde(default)]
    walls: Option<Vec<String>>,
    #[serde(default)]
    mean: Option<bool>,
    #[serde(default)]
    remove: bool,
}

pub(crate) fn field_derive_schema() -> Value {
    object(
        json!({
            "quantity": {"enum": Quantity::ALL.map(Quantity::key).to_vec()},
            "velocity": {"type": "string", "description": "the velocity field (a vector); left out: found by its name"},
            "pressure": {"type": "string", "description": "the pressure field (a scalar); left out: found by its name"},
            "k": {"type": "string", "description": "the turbulent kinetic energy (a scalar); left out: found by its name"},
            "dissipation": {"type": "string", "description": "omega or epsilon (a scalar; epsilon by its name, else by the unit m2/s3); left out: found by its name"},
            "nu_t": {"type": "string", "description": "the eddy viscosity (a scalar); left out: found by its name"},
            "rms_velocity": {"type": "string", "description": "the RMS of the velocity (a vector, statistics.vtkhdf rms_velocity); left out: found by its name"},
            "covariance_xy": {"type": "string", "description": "<u'v'> (a scalar, covariance_velocity_xy); left out: found by its name"},
            "covariance_yz": {"type": "string", "description": "<v'w'> (a scalar, covariance_velocity_yz); left out: found by its name"},
            "covariance_xz": {"type": "string", "description": "<u'w'> (a scalar, covariance_velocity_xz); left out: found by its name"},
            "mean_k": {"type": "string", "description": "the mean modelled k (a scalar, mean_k); left out: found by its name"},
            "heat_flux_x": {"type": "string", "description": "<u'T'> (a scalar, covariance_velocity_x_temperature); left out: found by its name"},
            "heat_flux_y": {"type": "string", "description": "<v'T'> (a scalar, covariance_velocity_y_temperature); left out: found by its name"},
            "heat_flux_z": {"type": "string", "description": "<w'T'> (a scalar, covariance_velocity_z_temperature); left out: found by its name"},
            "reference": object(json!({
                "density": {"type": "number", "exclusiveMinimum": 0},
                "pressure": {"type": "number"},
                "velocity": {"type": "number", "exclusiveMinimum": 0},
                "viscosity": {"type": "number", "exclusiveMinimum": 0, "description": "kinematic viscosity (m2/s)"},
                "c_mu": {"type": "number", "exclusiveMinimum": 0, "description": "C_mu of the turbulence model (beta* of SST)"}
            }), &[]),
            "rotation": object(json!({
                "zones": {"type": "array", "minItems": 1, "maxItems": turbo::MAX_ZONES, "items": object(json!({
                    "zone": {"type": "string", "description": "the cell zone that turns; left out: every cell"},
                    "origin": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
                    "axis": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
                    "rpm": {"type": "number"}
                }), &["origin", "axis", "rpm"])},
                "frame": {"type": "string", "description": "the zone whose frame every cell is seen in; left out: each cell in its own zone's"}
            }), &["zones"]),
            "walls": {"type": "array", "items": {"type": "string"}, "description": "the wall patches (wall distance); left out: the case's walls, else the patches whose role is wall"},
            "mean": {"type": "boolean", "description": "of the mean flow of a transient run's statistics (mean_velocity, mean_pressure, mean_k), making mean_<field>; left out: when the result has the means and your default says so, or it has no instantaneous fields"},
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
    let mut i: DeriveInput = parse(input)?;
    let named = i.inputs();
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
        if named.iter().any(|(_, f)| f.is_some())
            || i.reference.is_some()
            || i.rotation.is_some()
            || i.walls.is_some()
        {
            return Err(Diagnostic::new(
                "TOOL002",
                "remove takes the quantity alone",
            ));
        }
        let name = q.field_of(i.mean.unwrap_or(false));
        s.underive(name).map_err(refused)?;
        return Ok(json!({"removed": name}));
    }
    let mean = i.mean.unwrap_or_else(|| derive::start_mean(s, q));
    let own = s.fields().and_then(|f| f.recipe(q.field_of(mean))).cloned();
    let mut recipe = Recipe::new(q);
    recipe.mean = mean;
    for (input, field) in named {
        if field.is_some() && !q.inputs().contains(&input) {
            return Err(Diagnostic::new(
                "TOOL002",
                format!("{} takes no {}", q.key(), input.key()),
            ));
        }
        recipe.set_input(input, field);
    }
    for (given, what) in [
        (i.rotation.is_some() && !q.turns(), "rotation"),
        (i.walls.is_some() && !q.walls(), "walls"),
    ] {
        if given {
            return Err(Diagnostic::new(
                "TOOL002",
                format!("{} takes no {what}", q.key()),
            ));
        }
    }
    let mut sources = None;
    if !q.references().is_empty() {
        // Values given win; the rest are the field's own when it is
        // made again, else where new fields start from.
        let (start, from) = match own.as_ref().and_then(|r| r.reference) {
            Some(r) => (r, [None; 5]),
            None => {
                let (r, f) = derive::start_reference(s);
                (r, f.map(Some))
            }
        };
        let part = i.reference.unwrap_or_default();
        let mut r = start;
        for k in 0..REFERENCE_KEYS.len() {
            if let Some(x) = part.get(k) {
                *r.get_mut(k) = x;
            }
        }
        sources = Some(
            REFERENCE_KEYS
                .into_iter()
                .enumerate()
                .filter(|(_, key)| q.references().contains(key))
                .map(|(k, key)| {
                    let src = if part.get(k).is_some() {
                        json!("given")
                    } else {
                        from[k].map_or(json!("kept"), |f| json!(f))
                    };
                    (key.to_string(), src)
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
    let mut rotation_from = None;
    if q.turns() {
        recipe.rotation = Some(
            match (i.rotation, own.as_ref().and_then(|r| r.rotation.clone())) {
                (Some(r), _) => {
                    rotation_from = Some(json!("given"));
                    r
                }
                (None, Some(r)) => {
                    rotation_from = Some(json!("kept"));
                    r
                }
                (None, None) => {
                    let (r, from) = turbo::start_rotation(s);
                    rotation_from = Some(json!(from));
                    r
                }
            },
        );
    }
    if q.walls() {
        recipe.walls = i
            .walls
            .or_else(|| own.as_ref().and_then(|r| r.walls.clone()))
            .or_else(|| {
                let m = s.mesh.as_ref()?;
                let w = murmel_engine::walls::find_walls(&m.mesh, Some(&m.path));
                (!w.is_empty()).then_some(w)
            });
    }
    let name = s.derive(recipe).map_err(refused)?;
    let mut out = derived_json(s, name);
    if let Some(src) = sources {
        out["reference_from"] = Value::Object(src);
    }
    if let Some(r) = rotation_from {
        out["rotation_from"] = r;
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
            let mean = derive::start_mean(s, q);
            let field = q.field_of(mean);
            let recipe = set.recipe(field).cloned().unwrap_or_else(|| {
                let mut r = Recipe::new(q);
                r.mean = mean;
                r
            });
            let missing: Vec<&str> = q
                .inputs()
                .iter()
                .filter(|&&i| recipe.input(set, i).is_none())
                .map(|i| i.key())
                .collect();
            let in_file =
                set.recipe(field).is_none() && set.fields().iter().any(|f| f.name == field);
            json!({
                "quantity": q.key(),
                "group": q.group(),
                "field": field,
                "mean": mean,
                "averages": q.averages(),
                "formula": q.formula(),
                "unit": q.unit(),
                "inputs": q.inputs().iter().map(|&i| input_json(s, &recipe, i)).collect::<Vec<_>>(),
                "references": q.references(),
                "turns": q.turns(),
                "walls": q.walls(),
                "missing": missing,
                "available": missing.is_empty() && !in_file,
                "in_file": in_file,
                "derived": set.recipe(field),
            })
        })
        .collect();
    let (r, from) = derive::start_reference(s);
    let (rotation, rotation_from) = turbo::start_rotation(s);
    let (walls, patches, mesh_zones) = s.mesh.as_ref().map_or_else(Default::default, |m| {
        (
            murmel_engine::walls::find_walls(&m.mesh, Some(&m.path)),
            m.mesh
                .patches
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>(),
            m.mesh.zones.clone(),
        )
    });
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
            "viscosity": r.viscosity,
            "c_mu": r.c_mu,
            "from": REFERENCE_KEYS
                .into_iter()
                .zip(from)
                .map(|(k, f)| (k.to_string(), json!(f)))
                .collect::<serde_json::Map<_, _>>(),
        },
        "rotation": {
            "zones": rotation.zones,
            "frame": rotation.frame,
            "from": rotation_from,
            "mesh_zones": mesh_zones,
        },
        "walls": {
            "patches": walls,
            "mesh_patches": patches,
        },
        "sources": [Source::Case, Source::Defaults, Source::BuiltIn],
    }))
}
