//! Derived fields (the maintainer's cards of 2026-11-24: MurmelCraft
//! computes the common ones itself, FoehnCraft the rest with `foehn post
//! derive`; a catalogue window lists them with their formula, unit and
//! inputs): quantities computed from the fields of the open result,
//! which then join its fields as if the file had them, for colouring,
//! slices, iso-surfaces, arrows, streamlines and probes alike.
//!
//! | quantity | field | definition | unit |
//! |---|---|---|---|
//! | `vorticity` | `vorticity` (vector) | ω = ∇ × u | 1/s |
//! | `q-criterion` | `q_criterion` | Q = ½ (‖Ω‖² − ‖S‖²); vortices where Q > 0 (J. C. R. Hunt, A. A. Wray and P. Moin, "Eddies, streams, and convergence zones in turbulent flows", CTR report CTR-S88, 1988) | 1/s² |
//! | `lambda2` | `lambda2` | the middle eigenvalue of S² + Ω²; vortices where λ₂ < 0 (J. Jeong and F. Hussain, "On the identification of a vortex", J. Fluid Mech. 285, 1995) | 1/s² |
//! | `total-pressure` | `total_pressure` | p + ½ ρ u·u | Pa |
//! | `pressure-coefficient` | `pressure_coefficient` | (p − p_ref) / (½ ρ_ref U_ref²) | - |
//!
//! The names and units are the ones FoehnCraft writes, so a result with
//! FoehnCraft's own derived fields and one derived here read alike; a
//! field the file already has is not computed again.
//!
//! The velocity gradient G of a cell, Gᵢⱼ = ∂uᵢ/∂xⱼ, is the
//! least-squares gradient of each component ([`crate::gradient`], with
//! the velocity on the boundary faces where the file has it), its
//! symmetric part S = ½ (G + Gᵀ) the rate of strain and its
//! antisymmetric part Ω = ½ (G − Gᵀ) the rate of rotation; ‖·‖ is the
//! Frobenius norm. A velocity given at the nodes gets each cell's
//! gradient fitted to the cell's own nodes, about their mean. The
//! eigenvalues of λ₂ come from cyclic Jacobi rotations (G. H. Golub and
//! C. F. Van Loan, "Matrix Computations", §8.5), exact to round-off also
//! where two of them are equal, as in every vortex of a plane flow.
//!
//! Pressure quantities are taken wherever the pressure is (cells, nodes,
//! boundary faces) and, for the total pressure, the velocity too. A
//! pressure in m²/s² is a kinematic one (p/ρ, as incompressible solvers
//! write it): its total pressure is p + ½ u·u, its coefficient
//! (p − p_ref) / (½ U_ref²). A density field of the file (`density`,
//! `rho`) at the same place wins over the reference density.

use std::path::Path;

use murmel_mesh::Mesh;
use murmel_mesh::field::{Component, Field, Location};
use serde::{Deserialize, Serialize};

use crate::defaults::Kind;
use crate::field::FieldSet;
use crate::session::Session;

/// A quantity MurmelCraft derives itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Quantity {
    Vorticity,
    QCriterion,
    Lambda2,
    TotalPressure,
    PressureCoefficient,
}

/// The groups of the catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    /// Vorticity, Q and λ₂: where the flow turns.
    Vortices,
    /// Total pressure and its coefficient.
    Pressure,
}

impl Group {
    pub const ALL: [Group; 2] = [Group::Vortices, Group::Pressure];

    pub fn key(self) -> &'static str {
        match self {
            Group::Vortices => "vortices",
            Group::Pressure => "pressure",
        }
    }
}

/// A field a quantity is computed from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    /// A vector of three components.
    Velocity,
    /// A scalar.
    Pressure,
}

impl Input {
    pub fn key(self) -> &'static str {
        match self {
            Input::Velocity => "velocity",
            Input::Pressure => "pressure",
        }
    }

    /// The names the field is found by when none is given, in order.
    fn names(self) -> &'static [&'static str] {
        match self {
            Input::Velocity => &["velocity", "U", "u", "vel", "Velocity"],
            Input::Pressure => &["pressure", "p", "static_pressure", "Pressure"],
        }
    }

    fn components(self) -> usize {
        match self {
            Input::Velocity => 3,
            Input::Pressure => 1,
        }
    }
}

impl Quantity {
    pub const ALL: [Quantity; 5] = [
        Quantity::Vorticity,
        Quantity::QCriterion,
        Quantity::Lambda2,
        Quantity::TotalPressure,
        Quantity::PressureCoefficient,
    ];

    /// The name in tools and session files (FoehnCraft's `[output]
    /// derived` names).
    pub fn key(self) -> &'static str {
        match self {
            Quantity::Vorticity => "vorticity",
            Quantity::QCriterion => "q-criterion",
            Quantity::Lambda2 => "lambda2",
            Quantity::TotalPressure => "total-pressure",
            Quantity::PressureCoefficient => "pressure-coefficient",
        }
    }

    pub fn parse(s: &str) -> Option<Quantity> {
        Quantity::ALL.into_iter().find(|q| q.key() == s)
    }

    /// The name of the field it makes.
    pub fn field(self) -> &'static str {
        match self {
            Quantity::Vorticity => "vorticity",
            Quantity::QCriterion => "q_criterion",
            Quantity::Lambda2 => "lambda2",
            Quantity::TotalPressure => "total_pressure",
            Quantity::PressureCoefficient => "pressure_coefficient",
        }
    }

    /// Its unit, for a pressure in Pa.
    pub fn unit(self) -> &'static str {
        match self {
            Quantity::Vorticity => "1/s",
            Quantity::QCriterion | Quantity::Lambda2 => "1/s²",
            Quantity::TotalPressure => "Pa",
            Quantity::PressureCoefficient => "-",
        }
    }

    /// The formula, as the catalogue shows it.
    pub fn formula(self) -> &'static str {
        match self {
            Quantity::Vorticity => "ω = ∇ × u",
            Quantity::QCriterion => "Q = ½ (‖Ω‖² − ‖S‖²)",
            Quantity::Lambda2 => "λ₂ of S² + Ω²",
            Quantity::TotalPressure => "p + ½ ρ u·u",
            Quantity::PressureCoefficient => "(p − p_ref) / (½ ρ_ref U_ref²)",
        }
    }

    pub fn group(self) -> Group {
        match self {
            Quantity::Vorticity | Quantity::QCriterion | Quantity::Lambda2 => Group::Vortices,
            Quantity::TotalPressure | Quantity::PressureCoefficient => Group::Pressure,
        }
    }

    pub fn inputs(self) -> &'static [Input] {
        match self {
            Quantity::Vorticity | Quantity::QCriterion | Quantity::Lambda2 => &[Input::Velocity],
            Quantity::TotalPressure => &[Input::Pressure, Input::Velocity],
            Quantity::PressureCoefficient => &[Input::Pressure],
        }
    }

    /// The reference values it uses, by their keys in [`Reference`].
    pub fn references(self) -> &'static [&'static str] {
        match self {
            Quantity::Vorticity | Quantity::QCriterion | Quantity::Lambda2 => &[],
            Quantity::TotalPressure => &["density"],
            Quantity::PressureCoefficient => &["density", "pressure", "velocity"],
        }
    }

    /// The quantity that makes field `name`, if one does.
    pub fn of_field(name: &str) -> Option<Quantity> {
        Quantity::ALL.into_iter().find(|q| q.field() == name)
    }
}

/// Built-in reference density: air at sea level in the ICAO standard
/// atmosphere (kg/m³).
pub const AIR_DENSITY: f64 = 1.225;
/// The largest reference value accepted (a check against typing slips).
pub const MAX_REFERENCE: f64 = 1e12;

/// The reference values of the pressure quantities.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    /// ρ_ref (kg/m³), above 0.
    pub density: f64,
    /// p_ref, in the unit of the pressure.
    pub pressure: f64,
    /// U_ref (m/s), above 0.
    pub velocity: f64,
}

impl Default for Reference {
    fn default() -> Self {
        Reference {
            density: AIR_DENSITY,
            pressure: 0.0,
            velocity: 1.0,
        }
    }
}

impl Reference {
    /// Why these values cannot be used, if they cannot.
    pub fn check(&self) -> Result<(), String> {
        let ok = |x: f64| x.is_finite() && x.abs() <= MAX_REFERENCE;
        if !(ok(self.density) && self.density > 0.0) {
            return Err(format!(
                "reference density {} must be above 0 and finite",
                self.density
            ));
        }
        if !(ok(self.velocity) && self.velocity > 0.0) {
            return Err(format!(
                "reference velocity {} must be above 0 and finite",
                self.velocity
            ));
        }
        if !ok(self.pressure) {
            return Err(format!(
                "reference pressure {} must be finite",
                self.pressure
            ));
        }
        Ok(())
    }
}

/// Where the reference values a new derived field starts from come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// FoehnCraft's `case.resolved.toml` beside the result.
    Case,
    /// The user's defaults (`settings.toml`, `[defaults.reference]`).
    Defaults,
    /// Built in: air at sea level, p_ref 0, U_ref the largest speed of
    /// the velocity (1 m/s without one).
    BuiltIn,
}

/// The reference values FoehnCraft used for the result at `result`, from
/// `case.resolved.toml` in the same folder (FoehnCraft writes the case
/// there with every default filled in): ρ_ref from `[output.reference]
/// density`, else `[fluid] density` when it is a number; p_ref from
/// `[output.reference] pressure`; U_ref from `[output.reference]
/// velocity`, else `[output.coefficients] velocity`. Each `None` the
/// case does not give.
pub fn case_reference(result: &Path) -> Option<[Option<f64>; 3]> {
    let file = result.parent()?.join("case.resolved.toml");
    let text = std::fs::read_to_string(file).ok()?;
    let case: toml::Table = toml::from_str(&text).ok()?;
    let number = |path: &[&str]| -> Option<f64> {
        let mut at = case.get(path[0])?;
        for k in &path[1..] {
            at = at.get(k)?;
        }
        at.as_float()
            .or_else(|| at.as_integer().map(|i| i as f64))
            .filter(|x| x.is_finite())
    };
    let density = number(&["output", "reference", "density"])
        .or_else(|| number(&["fluid", "density"]))
        .filter(|&x| x > 0.0);
    let pressure = number(&["output", "reference", "pressure"]);
    let velocity = number(&["output", "reference", "velocity"])
        .or_else(|| number(&["output", "coefficients", "velocity"]))
        .filter(|&x| x > 0.0);
    let found = [density, pressure, velocity];
    found.iter().any(Option::is_some).then_some(found)
}

/// The reference values a new derived field of `s` starts from, each
/// with where it came from: FoehnCraft's case beside the result, else
/// the user's default, else built in (air at sea level, p_ref 0, U_ref
/// the largest speed of the velocity, 1 m/s without one).
pub fn start_reference(s: &Session) -> (Reference, [Source; 3]) {
    let case = s.mesh.as_ref().and_then(|m| case_reference(&m.path));
    let speed = s
        .fields()
        .and_then(|f| {
            let n = find_input(f, Input::Velocity)?;
            f.get(&n)?.range(Component::Magnitude)
        })
        .map(|(_, hi)| hi)
        .filter(|&x| x > 0.0 && x.is_finite() && x <= MAX_REFERENCE);
    let built = Reference {
        density: AIR_DENSITY,
        pressure: 0.0,
        velocity: speed.unwrap_or(1.0),
    };
    let mine = s.defaults.apply(Kind::Reference, built);
    let mut r = mine;
    let mut from = [Source::BuiltIn; 3];
    for (k, key) in ["density", "pressure", "velocity"].into_iter().enumerate() {
        if let Some(x) = case.and_then(|c| c[k]) {
            match k {
                0 => r.density = x,
                1 => r.pressure = x,
                _ => r.velocity = x,
            }
            from[k] = Source::Case;
        } else if s.defaults.get(Kind::Reference, key).is_some() {
            from[k] = Source::Defaults;
        }
    }
    // A case value out of range (a typing slip there) is not taken.
    if r.check().is_err() {
        return (
            mine,
            from.map(|f| {
                if f == Source::Case {
                    Source::BuiltIn
                } else {
                    f
                }
            }),
        );
    }
    (r, from)
}

/// How to compute one derived field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub quantity: Quantity,
    /// The velocity field; left out, found by its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity: Option<String>,
    /// The pressure field; left out, found by its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure: Option<String>,
    /// The reference values (total pressure, pressure coefficient).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<Reference>,
}

impl Recipe {
    pub fn new(quantity: Quantity) -> Recipe {
        Recipe {
            quantity,
            velocity: None,
            pressure: None,
            reference: None,
        }
    }

    /// The field it makes.
    pub fn field(&self) -> &'static str {
        self.quantity.field()
    }

    /// The field it reads for `input`: the one named, else the first
    /// field of the result that fits by name and shape.
    pub fn input(&self, set: &FieldSet, input: Input) -> Option<String> {
        let named = match input {
            Input::Velocity => &self.velocity,
            Input::Pressure => &self.pressure,
        };
        if let Some(n) = named {
            return Some(n.clone());
        }
        find_input(set, input)
    }

    /// The fields of `set` it reads: its inputs, and the density fields
    /// the pressure quantities take.
    pub fn reads(&self, set: &FieldSet) -> Vec<String> {
        let mut out: Vec<String> = self
            .quantity
            .inputs()
            .iter()
            .filter_map(|&i| self.input(set, i))
            .collect();
        if !self.quantity.references().is_empty() {
            for n in ["density", "rho"] {
                if set.get(n).is_some() {
                    out.push(n.into());
                }
            }
        }
        out
    }
}

/// A field made from the result's fields: a derived quantity, or a
/// formula of your own ([`crate::calc`]). Sessions keep them in the
/// order they were made, each after the fields it reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Made {
    Quantity(Recipe),
    Formula(crate::calc::Formula),
}

impl Made {
    /// The field it makes.
    pub fn field(&self) -> &str {
        match self {
            Made::Quantity(r) => r.field(),
            Made::Formula(f) => &f.name,
        }
    }

    /// The fields of `set` it reads.
    pub fn reads(&self, set: &FieldSet) -> Vec<String> {
        match self {
            Made::Quantity(r) => r.reads(set),
            Made::Formula(f) => crate::calc::reads(&f.formula, set),
        }
    }

    /// Its fields, made from those of `set`.
    pub fn compute(&self, mesh: &Mesh, set: &FieldSet) -> Result<Vec<Field>, Refusal> {
        match self {
            Made::Quantity(r) => compute(mesh, set, r),
            Made::Formula(f) => crate::calc::compute(mesh, set, f),
        }
    }
}

/// The field of `set` that `input` reads when none is named: by name,
/// with the right number of components, on cells or nodes (a field on
/// the boundary faces alone does not do), and not itself derived.
pub fn find_input(set: &FieldSet, input: Input) -> Option<String> {
    let fits = |f: &Field| {
        f.components == input.components()
            && f.location != Location::Boundary
            && set.maker(&f.name).is_none()
    };
    for name in input.names() {
        if set.fields().iter().any(|f| f.name == *name && fits(f)) {
            return Some((*name).to_string());
        }
    }
    let lower: Vec<String> = input.names().iter().map(|n| n.to_lowercase()).collect();
    set.fields()
        .iter()
        .find(|f| fits(f) && lower.contains(&f.name.to_lowercase()))
        .map(|f| f.name.clone())
}

/// The fields of `set` that can be `input` (for the catalogue's choice):
/// the file's and your formulas' fields, not the derived quantities'.
pub fn candidates(set: &FieldSet, input: Input) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in set.fields() {
        if f.components == input.components()
            && f.location != Location::Boundary
            && set.recipe(&f.name).is_none()
            && !out.contains(&f.name)
        {
            out.push(f.name.clone());
        }
    }
    out
}

/// Why a field cannot be derived.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    /// No field for this input (FIELD005).
    Missing(Input),
    /// The input field has the wrong shape or place (FIELD006).
    Shape(Input, String),
    /// A reference value is out of range (FIELD007).
    Reference(String),
    /// The file has a field of this name (FIELD008).
    InFile(String),
    /// No mesh is open, or it has no fields (FIELD001).
    NoFields,
    /// No derived field of this name: none was made, or it is the
    /// file's own (FIELD009).
    NotDerived(String),
    /// Items made from the derived field, by id, and the fields made
    /// from it, by name (FIELD010).
    InUse(String, Vec<String>),
    /// A formula that cannot be read (FIELD011).
    Syntax(String),
    /// A name in a formula that is no field, built-in name or function
    /// (FIELD012).
    Unknown(String),
    /// A formula whose types or places do not fit together: a vector
    /// where a scalar goes, fields with no place in common (FIELD013).
    Type(String),
    /// A name a formula's field cannot have (FIELD014).
    BadName(String),
    /// A formula that reads itself, or a field made from it (FIELD015).
    Cycle(String),
    /// Making a field again would break field `.0`, made from it: why.
    Breaks(String, Box<Refusal>),
}

impl Refusal {
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::Missing(_) => "FIELD005",
            Refusal::Shape(..) => "FIELD006",
            Refusal::Reference(_) => "FIELD007",
            Refusal::InFile(_) => "FIELD008",
            Refusal::NoFields => "FIELD001",
            Refusal::NotDerived(_) => "FIELD009",
            Refusal::InUse(..) => "FIELD010",
            Refusal::Syntax(_) => "FIELD011",
            Refusal::Unknown(_) => "FIELD012",
            Refusal::Type(_) => "FIELD013",
            Refusal::BadName(_) => "FIELD014",
            Refusal::Cycle(_) => "FIELD015",
            Refusal::Breaks(_, r) => r.code(),
        }
    }

    pub fn message(&self) -> String {
        match self {
            Refusal::Missing(i) => format!(
                "no {} field: name one (field.list lists the fields) or open a result with one",
                i.key()
            ),
            Refusal::Shape(i, why) => format!("{} field {why}", i.key()),
            Refusal::Reference(why) => why.clone(),
            Refusal::InFile(n) => {
                format!("the result's file has a field {n}; it is not computed again")
            }
            Refusal::NoFields => "the open mesh has no fields".into(),
            Refusal::NotDerived(n) => format!("no derived or calculated field {n}"),
            Refusal::InUse(n, ids) => format!(
                "{n} is in use by {}; remove or change them first",
                ids.join(", ")
            ),
            Refusal::Syntax(why)
            | Refusal::Unknown(why)
            | Refusal::Type(why)
            | Refusal::BadName(why)
            | Refusal::Cycle(why) => why.clone(),
            Refusal::Breaks(n, r) => {
                format!("{n} is made from it and would fail: {}", r.message())
            }
        }
    }
}

/// Whether `unit` is that of a kinematic pressure (m²/s²).
fn kinematic(unit: Option<&str>) -> bool {
    let Some(u) = unit else {
        return false;
    };
    let u: String = u.chars().filter(|c| !c.is_whitespace()).collect();
    matches!(
        u.as_str(),
        "m2/s2" | "m²/s²" | "m^2/s^2" | "m2s-2" | "m2.s-2"
    )
}

/// The input field `name` of `set` for `input`, checked: the right
/// number of components, at the places given first in `places`.
fn input_fields<'a>(
    set: &'a FieldSet,
    mesh: &Mesh,
    input: Input,
    name: &str,
) -> Result<Vec<&'a Field>, Refusal> {
    let found: Vec<&Field> = [Location::Cell, Location::Point, Location::Boundary]
        .into_iter()
        .filter_map(|l| set.at(name, l))
        .collect();
    if found.is_empty() {
        return Err(Refusal::Missing(input));
    }
    if let Some(f) = found.iter().find(|f| f.components != input.components()) {
        return Err(Refusal::Shape(
            input,
            format!(
                "{} has {} component(s); it needs {}",
                f.name,
                f.components,
                input.components()
            ),
        ));
    }
    let fit: Vec<&Field> = found
        .into_iter()
        .filter(|f| FieldSet::fits(f, mesh))
        .collect();
    if fit.is_empty() {
        return Err(Refusal::Shape(
            input,
            format!("{name} does not have a value for every cell, node or face"),
        ));
    }
    Ok(fit)
}

/// The fields `recipe` makes on `mesh` from the fields of `set`, with the
/// reference values `reference` when it has none of its own. Each is
/// named [`Quantity::field`], on cells for the quantities of the
/// velocity gradient, and wherever the inputs are for the pressure ones.
pub fn compute(mesh: &Mesh, set: &FieldSet, recipe: &Recipe) -> Result<Vec<Field>, Refusal> {
    if set.is_empty() {
        return Err(Refusal::NoFields);
    }
    let q = recipe.quantity;
    let name = q.field();
    if set.fields().iter().any(|f| f.name == name) && set.recipe(name).is_none() {
        return Err(Refusal::InFile(name.into()));
    }
    let reference = recipe.reference.unwrap_or_default();
    if !q.references().is_empty() {
        reference.check().map_err(Refusal::Reference)?;
    }
    let need = |input: Input| -> Result<Vec<&Field>, Refusal> {
        let n = recipe.input(set, input).ok_or(Refusal::Missing(input))?;
        input_fields(set, mesh, input, &n)
    };
    match q {
        Quantity::Vorticity | Quantity::QCriterion | Quantity::Lambda2 => {
            let u = need(Input::Velocity)?;
            let Some(g) = u
                .iter()
                .find(|f| f.location != Location::Boundary)
                .and_then(|f| set.velocity_gradient(mesh, f))
            else {
                return Err(Refusal::Shape(
                    Input::Velocity,
                    "is only on the boundary faces; the gradient needs it in the cells or at the nodes"
                        .into(),
                ));
            };
            let (components, values) = match q {
                Quantity::Vorticity => (3, g.iter().flat_map(|g| vorticity(*g)).collect()),
                Quantity::QCriterion => (1, g.iter().map(|g| kinematics(*g).0).collect()),
                _ => (1, g.iter().map(|g| kinematics(*g).1).collect()),
            };
            Ok(vec![Field {
                name: name.into(),
                unit: Some(q.unit().into()),
                location: Location::Cell,
                components,
                values,
            }])
        }
        Quantity::TotalPressure | Quantity::PressureCoefficient => {
            let p = need(Input::Pressure)?;
            let u = if q == Quantity::TotalPressure {
                need(Input::Velocity)?
            } else {
                Vec::new()
            };
            let mut out = Vec::new();
            for pf in &p {
                let uf = u.iter().find(|f| f.location == pf.location);
                if q == Quantity::TotalPressure && uf.is_none() {
                    continue;
                }
                let kin = kinematic(pf.unit.as_deref());
                let rho_field = (!kin)
                    .then(|| {
                        ["density", "rho"].into_iter().find_map(|n| {
                            set.at(n, pf.location)
                                .filter(|f| f.components == 1 && f.len() == pf.len())
                        })
                    })
                    .flatten();
                let rho = |i: usize| {
                    if kin {
                        1.0
                    } else {
                        rho_field.map_or(reference.density, |f| f.values[i])
                    }
                };
                let dynamic = if kin {
                    0.5 * reference.velocity * reference.velocity
                } else {
                    0.5 * reference.density * reference.velocity * reference.velocity
                };
                let values: Vec<f64> = (0..pf.len())
                    .map(|i| {
                        let pv = pf.values[i];
                        match uf {
                            Some(uf) => {
                                let v = uf.value(i).unwrap_or(&[f64::NAN; 3]);
                                let uu: f64 = v.iter().map(|x| x * x).sum();
                                pv + 0.5 * rho(i) * uu
                            }
                            None => (pv - reference.pressure) / dynamic,
                        }
                    })
                    .collect();
                let unit = match (q, kin) {
                    (Quantity::TotalPressure, true) => pf.unit.clone(),
                    _ => Some(q.unit().into()),
                };
                out.push(Field {
                    name: name.into(),
                    unit,
                    location: pf.location,
                    components: 1,
                    values,
                });
            }
            if out.is_empty() {
                return Err(Refusal::Shape(
                    Input::Velocity,
                    "is not where the pressure is (cells, nodes or boundary faces)".into(),
                ));
            }
            Ok(out)
        }
    }
}

/// ω = ∇ × u of the velocity gradient `g` (gᵢⱼ = ∂uᵢ/∂xⱼ).
pub fn vorticity(g: [[f64; 3]; 3]) -> [f64; 3] {
    [g[2][1] - g[1][2], g[0][2] - g[2][0], g[1][0] - g[0][1]]
}

/// Q and λ₂ of the velocity gradient `g`.
pub fn kinematics(g: [[f64; 3]; 3]) -> (f64, f64) {
    let mut s = [[0.0; 3]; 3];
    let mut w = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            s[i][j] = 0.5 * (g[i][j] + g[j][i]);
            w[i][j] = 0.5 * (g[i][j] - g[j][i]);
        }
    }
    let norm2 = |m: &[[f64; 3]; 3]| m.iter().flatten().map(|x| x * x).sum::<f64>();
    let q = 0.5 * (norm2(&w) - norm2(&s));
    let mut m = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = (0..3).map(|k| s[i][k] * s[k][j] + w[i][k] * w[k][j]).sum();
        }
    }
    // S² + Ω² is symmetric; round-off is evened out before the rotations.
    for i in 0..3 {
        for j in i + 1..3 {
            let a = 0.5 * (m[i][j] + m[j][i]);
            m[i][j] = a;
            m[j][i] = a;
        }
    }
    (q, symmetric_eigenvalues(m)[1])
}

/// The eigenvalues of the symmetric matrix `a`, ascending, by cyclic
/// Jacobi rotations (Golub and Van Loan, §8.5): each rotation zeroes one
/// off-diagonal entry, and the sweeps repeat until the off-diagonal part
/// is round-off.
pub fn symmetric_eigenvalues(mut a: [[f64; 3]; 3]) -> [f64; 3] {
    for _ in 0..32 {
        let off = a[0][1] * a[0][1] + a[0][2] * a[0][2] + a[1][2] * a[1][2];
        let diag = a[0][0] * a[0][0] + a[1][1] * a[1][1] + a[2][2] * a[2][2];
        if off == 0.0 || off <= 1e-32 * diag || !off.is_finite() {
            break;
        }
        for (p, r) in [(0, 1), (0, 2), (1, 2)] {
            let apr = a[p][r];
            if apr == 0.0 {
                continue;
            }
            let theta = (a[r][r] - a[p][p]) / (2.0 * apr);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for row in a.iter_mut() {
                let (x, y) = (row[p], row[r]);
                row[p] = c * x - s * y;
                row[r] = s * x + c * y;
            }
            for k in 0..3 {
                let (x, y) = (a[p][k], a[r][k]);
                a[p][k] = c * x - s * y;
                a[r][k] = s * x + c * y;
            }
        }
    }
    let mut e = [a[0][0], a[1][1], a[2][2]];
    e.sort_by(f64::total_cmp);
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the eigenvalues are those of the matrix, ascending,
    /// also for equal ones and for a full matrix. Breaks when a rotation
    /// is applied on the wrong side or its angle has the wrong sign.
    #[test]
    fn eigenvalues_of_symmetric_matrices() {
        let e = symmetric_eigenvalues([[2.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 2.0]]);
        assert_eq!(e, [-1.0, 2.0, 2.0]);
        // [[2,1,0],[1,2,0],[0,0,5]]: 1, 3, 5.
        let e = symmetric_eigenvalues([[2.0, 1.0, 0.0], [1.0, 2.0, 0.0], [0.0, 0.0, 5.0]]);
        for (a, b) in e.iter().zip([1.0, 3.0, 5.0]) {
            assert!((a - b).abs() < 1e-12, "{e:?}");
        }
        // A full matrix: its trace and determinant are kept.
        let m = [[4.0, 1.0, -2.0], [1.0, 2.0, 0.5], [-2.0, 0.5, 3.0]];
        let e = symmetric_eigenvalues(m);
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        assert!((e.iter().sum::<f64>() - 9.0).abs() < 1e-12);
        assert!((e.iter().product::<f64>() - det).abs() < 1e-10);
        assert!(e[0] <= e[1] && e[1] <= e[2]);
    }

    /// Invariant: a fluid turning as a solid body at rate w has
    /// vorticity 2w along the axis, Q = w² > 0 and λ₂ = −w² < 0; a
    /// pure shear has as much strain as rotation, so Q = 0, and λ₂ = 0.
    /// Breaks when ω takes a gradient's transpose or S and Ω swap.
    #[test]
    fn vortex_and_shear() {
        let w = 3.0;
        // u = (−w y, w x, 0).
        let g = [[0.0, -w, 0.0], [w, 0.0, 0.0], [0.0, 0.0, 0.0]];
        assert_eq!(vorticity(g), [0.0, 0.0, 2.0 * w]);
        let (q, l2) = kinematics(g);
        assert!(
            (q - w * w).abs() < 1e-12 && (l2 + w * w).abs() < 1e-12,
            "{q} {l2}"
        );
        // u = (a y, 0, 0).
        let a = 2.0;
        let g = [[0.0, a, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
        assert_eq!(vorticity(g), [0.0, 0.0, -a]);
        let (q, l2) = kinematics(g);
        assert!(q.abs() < 1e-12 && l2.abs() < 1e-12, "{q} {l2}");
    }

    /// Invariant: the units that mark a kinematic pressure are told from
    /// those of a pressure in Pa.
    #[test]
    fn kinematic_pressure_units() {
        assert!(kinematic(Some("m2/s2")));
        assert!(kinematic(Some("m²/s²")));
        assert!(!kinematic(Some("Pa")));
        assert!(!kinematic(None));
    }

    /// Invariant: FoehnCraft's resolved case gives the reference values
    /// it used, and a folder without one gives none. Breaks when a key
    /// is read from the wrong table.
    #[test]
    fn reference_from_the_case() {
        let dir = std::env::temp_dir().join(format!("murmel-case-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let result = dir.join("fields.vtkhdf");
        assert_eq!(case_reference(&result), None);
        std::fs::write(
            dir.join("case.resolved.toml"),
            "[fluid]\ndensity = 998.2\n[output.reference]\npressure = 101325.0\n[output.coefficients]\nvelocity = 2\n",
        )
        .unwrap();
        assert_eq!(
            case_reference(&result),
            Some([Some(998.2), Some(101325.0), Some(2.0)])
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The polyhedral fixture with `fields`.
    fn polyhedra(fields: impl Fn(&Mesh) -> Vec<Field>) -> (Mesh, FieldSet) {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/openfoam/dual_polyhedra");
        let m = murmel_mesh::read(&p).unwrap();
        let f = fields(&m);
        let set = FieldSet::new(murmel_mesh::field::Results {
            fields: f,
            ..Default::default()
        });
        (m, set)
    }

    /// A constant velocity gradient: u = g x.
    const G: [[f64; 3]; 3] = [[0.2, -1.5, 0.3], [2.0, -0.4, 0.0], [0.5, 0.7, 0.2]];

    fn linear(x: [f64; 3]) -> [f64; 3] {
        [0, 1, 2].map(|i| (0..3).map(|j| G[i][j] * x[j]).sum::<f64>() + i as f64)
    }

    /// Invariant: a velocity linear in space, given in the cells or at
    /// the nodes, has its exact vorticity, Q and λ₂ in every cell of a
    /// polyhedral mesh, at the walls too. Breaks when a gradient
    /// component lands in the wrong row or column of G.
    #[test]
    fn exact_for_a_linear_velocity() {
        let want_w = vorticity(G);
        let (want_q, want_l2) = kinematics(G);
        for location in [Location::Cell, Location::Point] {
            let (m, set) = polyhedra(|m| {
                let at: Vec<[f64; 3]> = match location {
                    Location::Cell => (0..m.cell_count())
                        .map(|c| crate::inspect::cell_centroid(m, c))
                        .collect(),
                    _ => m.points.clone(),
                };
                vec![Field {
                    name: "U".into(),
                    unit: Some("m/s".into()),
                    location,
                    components: 3,
                    values: at.iter().flat_map(|&x| linear(x)).collect(),
                }]
            });
            let w = compute(&m, &set, &Recipe::new(Quantity::Vorticity)).unwrap();
            assert_eq!(w.len(), 1);
            assert_eq!((w[0].location, w[0].components), (Location::Cell, 3));
            assert_eq!(w[0].len(), m.cell_count());
            for c in 0..m.cell_count() {
                let v = w[0].value(c).unwrap();
                for k in 0..3 {
                    assert!(
                        (v[k] - want_w[k]).abs() < 1e-6,
                        "{location:?} cell {c}: {v:?}"
                    );
                }
            }
            for (q, want) in [(Quantity::QCriterion, want_q), (Quantity::Lambda2, want_l2)] {
                let f = compute(&m, &set, &Recipe::new(q)).unwrap();
                assert_eq!(f[0].unit.as_deref(), Some("1/s²"));
                assert_eq!(f[0].values.len(), m.cell_count());
                for (c, v) in f[0].values.iter().enumerate() {
                    assert!((v - want).abs() < 1e-6, "{q:?} {c}");
                }
            }
        }
    }

    /// Invariant: total pressure is p + ½ ρ u·u and the pressure
    /// coefficient (p − p_ref) / (½ ρ_ref U_ref²) wherever the pressure
    /// is; a kinematic pressure takes no density, and a density field
    /// wins over the reference one. Breaks when the reference is
    /// applied twice or the unit test of a kinematic pressure fails.
    #[test]
    fn pressure_quantities() {
        let cell = |name: &str, unit: &str, k: usize, v: Vec<f64>| Field {
            name: name.into(),
            unit: Some(unit.into()),
            location: Location::Cell,
            components: k,
            values: v,
        };
        let (m, set) = polyhedra(|m| {
            let n = m.cell_count();
            vec![
                cell(
                    "velocity",
                    "m/s",
                    3,
                    (0..n).flat_map(|_| [3.0, 4.0, 0.0]).collect(),
                ),
                cell("pressure", "Pa", 1, (0..n).map(|c| c as f64).collect()),
            ]
        });
        let r = Reference {
            density: 2.0,
            pressure: 10.0,
            velocity: 5.0,
        };
        let mut recipe = Recipe::new(Quantity::TotalPressure);
        recipe.reference = Some(r);
        let t = compute(&m, &set, &recipe).unwrap();
        assert_eq!(t[0].unit.as_deref(), Some("Pa"));
        assert!((t[0].values[7] - (7.0 + 0.5 * 2.0 * 25.0)).abs() < 1e-12);
        recipe.quantity = Quantity::PressureCoefficient;
        let cp = compute(&m, &set, &recipe).unwrap();
        assert!((cp[0].values[7] - (7.0 - 10.0) / (0.5 * 2.0 * 25.0)).abs() < 1e-12);

        // Kinematic: p/ρ in m²/s².
        let (m, set) = polyhedra(|m| {
            let n = m.cell_count();
            vec![
                cell(
                    "U",
                    "m/s",
                    3,
                    (0..n).flat_map(|_| [3.0, 4.0, 0.0]).collect(),
                ),
                cell("p", "m2/s2", 1, vec![1.0; n]),
                cell("rho", "kg/m3", 1, vec![1000.0; n]),
            ]
        });
        let mut recipe = Recipe::new(Quantity::TotalPressure);
        let t = compute(&m, &set, &recipe).unwrap();
        assert_eq!(t[0].unit.as_deref(), Some("m2/s2"));
        assert!((t[0].values[0] - 13.5).abs() < 1e-12);
        recipe.quantity = Quantity::PressureCoefficient;
        recipe.reference = Some(r);
        let cp = compute(&m, &set, &recipe).unwrap();
        assert!((cp[0].values[0] - (1.0 - 10.0) / 12.5).abs() < 1e-12);

        // A density field in Pa: it wins.
        let (m, set) = polyhedra(|m| {
            let n = m.cell_count();
            vec![
                cell(
                    "U",
                    "m/s",
                    3,
                    (0..n).flat_map(|_| [3.0, 4.0, 0.0]).collect(),
                ),
                cell("p", "Pa", 1, vec![1.0; n]),
                cell("rho", "kg/m3", 1, vec![1000.0; n]),
            ]
        });
        let t = compute(&m, &set, &Recipe::new(Quantity::TotalPressure)).unwrap();
        assert!((t[0].values[0] - (1.0 + 0.5 * 1000.0 * 25.0)).abs() < 1e-9);
    }

    /// Invariant: what cannot be derived is refused with its reason: no
    /// velocity, a scalar given as the velocity, a bad reference, a
    /// field the file has already.
    #[test]
    fn refusals() {
        let scalar = |name: &str| Field {
            name: name.into(),
            unit: None,
            location: Location::Cell,
            components: 1,
            values: Vec::new(),
        };
        let (m, set) = polyhedra(|m| {
            let mut p = scalar("pressure");
            p.values = vec![0.0; m.cell_count()];
            let mut w = scalar("q_criterion");
            w.values = vec![0.0; m.cell_count()];
            vec![p, w]
        });
        assert_eq!(
            compute(&m, &set, &Recipe::new(Quantity::Vorticity)),
            Err(Refusal::Missing(Input::Velocity))
        );
        let mut r = Recipe::new(Quantity::Lambda2);
        r.velocity = Some("pressure".into());
        assert_eq!(compute(&m, &set, &r).map_err(|e| e.code()), Err("FIELD006"));
        let mut r = Recipe::new(Quantity::PressureCoefficient);
        r.reference = Some(Reference {
            velocity: 0.0,
            ..Default::default()
        });
        assert_eq!(compute(&m, &set, &r).map_err(|e| e.code()), Err("FIELD007"));
        assert_eq!(
            compute(&m, &set, &Recipe::new(Quantity::QCriterion)).map_err(|e| e.code()),
            Err("FIELD008")
        );
    }

    /// Invariant: a derived field joins the open result's fields, is
    /// made again with new reference values, leaves with `underive`,
    /// and a session file brings it back on loading. Breaks when the
    /// recipe is not written or the cached values of the old field are
    /// kept.
    #[test]
    fn derived_fields_in_the_session() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
        let mut s = Session::new(crate::theme::Tokens::builtin().unwrap());
        s.open(&path).unwrap();
        assert_eq!(s.derive(Recipe::new(Quantity::Vorticity)), Ok("vorticity"));
        let f = s.fields().unwrap();
        assert!(f.get("vorticity").is_some_and(|f| f.is_vector()));
        assert_eq!(f.made().len(), 1);
        s.display.color_by = crate::display::ColorBy::Field;
        s.display.field = Some(crate::field::FieldChoice {
            name: "vorticity".into(),
            component: Component::Magnitude,
        });
        s.redraw();
        assert!(s.field_values().is_some());

        let mut cp = Recipe::new(Quantity::PressureCoefficient);
        cp.reference = Some(Reference::default());
        s.derive(cp.clone()).unwrap();
        let first = s
            .fields()
            .unwrap()
            .get("pressure_coefficient")
            .unwrap()
            .values
            .clone();
        cp.reference = Some(Reference {
            velocity: 2.0,
            ..Default::default()
        });
        s.derive(cp).unwrap();
        let f = s.fields().unwrap();
        assert_eq!(f.made().len(), 2);
        assert_eq!(
            f.fields()
                .iter()
                .filter(|x| x.name == "pressure_coefficient")
                .count(),
            f.fields().iter().filter(|x| x.name == "pressure").count(),
            "one per place the pressure is"
        );
        let second = &f.get("pressure_coefficient").unwrap().values;
        assert!((first[3] / second[3] - 4.0).abs() < 1e-9);

        let file =
            std::env::temp_dir().join(format!("murmel-derive-{}.murmel", std::process::id()));
        let saved = crate::session_file::SessionFile::of(&s, &file);
        assert_eq!(saved.derived.len(), 2);
        let text = saved.to_json();
        let mut t = Session::new(crate::theme::Tokens::builtin().unwrap());
        t.open(&path).unwrap();
        let missing = t.restore(crate::session_file::SessionFile::parse(&text).unwrap());
        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(t.fields().unwrap().made(), s.fields().unwrap().made());
        assert_eq!(
            t.display.field.as_ref().map(|c| c.name.as_str()),
            Some("vorticity")
        );
        assert!(t.field_values().is_some());

        // An iso-surface of vorticity keeps it.
        s.display.isos.push(crate::extract::Iso {
            field: s.display.field.clone().unwrap(),
            value: 1.0,
            enabled: true,
            color: Default::default(),
        });
        assert_eq!(
            s.underive("vorticity").map_err(|e| e.code()),
            Err("FIELD010")
        );
        s.display.isos.clear();
        assert_eq!(s.underive("vorticity"), Ok(()));
        assert_eq!(
            s.underive("vorticity").map_err(|e| e.code()),
            Err("FIELD009")
        );
        assert_eq!(
            s.underive("pressure").map_err(|e| e.code()),
            Err("FIELD009")
        );
        assert!(s.fields().unwrap().get("vorticity").is_none());
        assert_eq!(
            s.display.field.as_ref().map(|c| c.name.as_str()),
            s.fields().unwrap().names().first().copied()
        );
        assert!(s.field_values().is_some());
    }

    /// Invariant: reference values out of range are refused before
    /// anything is computed.
    #[test]
    fn reference_checks() {
        assert!(Reference::default().check().is_ok());
        for bad in [
            Reference {
                density: 0.0,
                ..Default::default()
            },
            Reference {
                velocity: -1.0,
                ..Default::default()
            },
            Reference {
                pressure: f64::NAN,
                ..Default::default()
            },
            Reference {
                density: f64::INFINITY,
                ..Default::default()
            },
        ] {
            assert!(bad.check().is_err(), "{bad:?}");
        }
    }
}
