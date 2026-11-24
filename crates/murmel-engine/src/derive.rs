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
//! | `enstrophy` | `enstrophy` | ½ ω·ω | 1/s² |
//! | `strain-rate` | `strain_rate` | √(2 S:S), the shear rate | 1/s |
//! | `relative-velocity` | `relative_velocity` (vector) | **w** = **c** − **Ω** × (**x** − **x**₀) | m/s |
//! | `frame-velocity` | `frame_velocity` (vector) | **u** = **Ω** × (**x** − **x**₀) | m/s |
//! | `velocity-radial`, `-circumferential`, `-axial`, `-meridional` | `velocity_radial` … | c_r, c_u, c_a, c_m = (c_r² + c_a²)^½ about the axis of the cell's frame | m/s |
//! | `relative-velocity-circumferential` | `relative_velocity_circumferential` | w_u = c_u − u_u | m/s |
//! | `flow-angle`, `relative-flow-angle` | `flow_angle`, `relative_flow_angle` | α = atan2(c_m, c_u), β = atan2(c_m, −w_u) | ° |
//! | `relative-total-pressure` | `relative_total_pressure` | p + ½ ρ w·w | Pa |
//! | `rothalpy` | `rothalpy` | I = p/ρ + ½ w·w − ½ u·u | m²/s² |
//! | `dissipation-rate`, `specific-dissipation-rate` | `dissipation_rate`, `specific_dissipation_rate` | ε = C_μ k ω, ω = ε/(C_μ k) | m²/s³, 1/s |
//! | `time-scale`, `length-scale` | `time_scale`, `length_scale` | T = k/ε = 1/(C_μ ω) (the bounded T of ζ-f where the file has it), L = k^{3/2}/ε | s, m |
//! | `viscosity-ratio` | `viscosity_ratio` | ν_t/ν | - |
//! | `reynolds-stress-xx` … `-xz` | `reynolds_stress_xx` … | ⅔ k δᵢⱼ − 2 ν_t Sᵢⱼ (Boussinesq) | m²/s² |
//! | `wall-distance` | `wall_distance` | the distance of the cell centroid to the nearest wall face ([`crate::walls`]) | m |
//!
//! The turbomachinery quantities take the turning zones of
//! [`crate::turbo`]; those, the turbulence ones and the wall distance
//! are computed in the cells, as FoehnCraft writes them. FoehnCraft's
//! other fields (y⁺, wall shear stress, the wall treatment's u_τ and y*,
//! the invariant features, the wall normal of elliptic blending and the
//! curvature correction) belong to its turbulence model and come from
//! `foehn post derive`.
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

/// A quantity MurmelCraft derives itself: one field each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Quantity {
    Vorticity,
    QCriterion,
    Lambda2,
    Enstrophy,
    StrainRate,
    TotalPressure,
    PressureCoefficient,
    RelativeVelocity,
    FrameVelocity,
    VelocityRadial,
    VelocityCircumferential,
    VelocityAxial,
    VelocityMeridional,
    RelativeVelocityCircumferential,
    FlowAngle,
    RelativeFlowAngle,
    RelativeTotalPressure,
    Rothalpy,
    DissipationRate,
    SpecificDissipationRate,
    TimeScale,
    LengthScale,
    ViscosityRatio,
    ReynoldsStressXx,
    ReynoldsStressYy,
    ReynoldsStressZz,
    ReynoldsStressXy,
    ReynoldsStressYz,
    ReynoldsStressXz,
    WallDistance,
}

/// The groups of the catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    /// Vorticity, Q, λ₂, enstrophy and strain rate: where the flow turns.
    Vortices,
    /// Total pressure and its coefficient.
    Pressure,
    /// The velocities of rotating machinery ([`crate::turbo`]).
    Turbomachinery,
    /// The scales of a two-equation turbulence model.
    Turbulence,
    /// The distance to the walls.
    Wall,
}

impl Group {
    pub const ALL: [Group; 5] = [
        Group::Vortices,
        Group::Pressure,
        Group::Turbomachinery,
        Group::Turbulence,
        Group::Wall,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Group::Vortices => "vortices",
            Group::Pressure => "pressure",
            Group::Turbomachinery => "turbomachinery",
            Group::Turbulence => "turbulence",
            Group::Wall => "wall",
        }
    }
}

/// A field a quantity is computed from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    /// A vector of three components.
    Velocity,
    /// A scalar.
    Pressure,
    /// The turbulent kinetic energy k (m²/s²).
    K,
    /// The second variable of the turbulence model: ω (1/s) or ε
    /// (m²/s³), told apart by name, else by unit.
    Dissipation,
    /// The eddy viscosity ν_t (m²/s).
    NuT,
}

impl Input {
    pub const ALL: [Input; 5] = [
        Input::Velocity,
        Input::Pressure,
        Input::K,
        Input::Dissipation,
        Input::NuT,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Input::Velocity => "velocity",
            Input::Pressure => "pressure",
            Input::K => "k",
            Input::Dissipation => "dissipation",
            Input::NuT => "nu_t",
        }
    }

    /// The names the field is found by when none is given, in order.
    fn names(self) -> &'static [&'static str] {
        match self {
            Input::Velocity => &["velocity", "U", "u", "vel", "Velocity"],
            Input::Pressure => &["pressure", "p", "static_pressure", "Pressure"],
            Input::K => &["k", "turbulent_kinetic_energy", "tke", "K"],
            Input::Dissipation => &[
                "omega",
                "epsilon",
                "specific_dissipation",
                "turbulent_dissipation",
                "Omega",
                "Epsilon",
            ],
            Input::NuT => &[
                "nu_t",
                "nut",
                "turbulent_viscosity",
                "eddy_viscosity",
                "Nut",
            ],
        }
    }

    fn components(self) -> usize {
        match self {
            Input::Velocity => 3,
            _ => 1,
        }
    }
}

impl Quantity {
    pub const ALL: [Quantity; 30] = [
        Quantity::Vorticity,
        Quantity::QCriterion,
        Quantity::Lambda2,
        Quantity::Enstrophy,
        Quantity::StrainRate,
        Quantity::TotalPressure,
        Quantity::PressureCoefficient,
        Quantity::RelativeVelocity,
        Quantity::FrameVelocity,
        Quantity::VelocityRadial,
        Quantity::VelocityCircumferential,
        Quantity::VelocityAxial,
        Quantity::VelocityMeridional,
        Quantity::RelativeVelocityCircumferential,
        Quantity::FlowAngle,
        Quantity::RelativeFlowAngle,
        Quantity::RelativeTotalPressure,
        Quantity::Rothalpy,
        Quantity::DissipationRate,
        Quantity::SpecificDissipationRate,
        Quantity::TimeScale,
        Quantity::LengthScale,
        Quantity::ViscosityRatio,
        Quantity::ReynoldsStressXx,
        Quantity::ReynoldsStressYy,
        Quantity::ReynoldsStressZz,
        Quantity::ReynoldsStressXy,
        Quantity::ReynoldsStressYz,
        Quantity::ReynoldsStressXz,
        Quantity::WallDistance,
    ];

    /// The name in tools and session files: FoehnCraft's `[output]
    /// derived` name for a quantity of one field, else the field's name
    /// with dashes.
    pub fn key(self) -> &'static str {
        match self {
            Quantity::Vorticity => "vorticity",
            Quantity::QCriterion => "q-criterion",
            Quantity::Lambda2 => "lambda2",
            Quantity::Enstrophy => "enstrophy",
            Quantity::StrainRate => "strain-rate",
            Quantity::TotalPressure => "total-pressure",
            Quantity::PressureCoefficient => "pressure-coefficient",
            Quantity::RelativeVelocity => "relative-velocity",
            Quantity::FrameVelocity => "frame-velocity",
            Quantity::VelocityRadial => "velocity-radial",
            Quantity::VelocityCircumferential => "velocity-circumferential",
            Quantity::VelocityAxial => "velocity-axial",
            Quantity::VelocityMeridional => "velocity-meridional",
            Quantity::RelativeVelocityCircumferential => "relative-velocity-circumferential",
            Quantity::FlowAngle => "flow-angle",
            Quantity::RelativeFlowAngle => "relative-flow-angle",
            Quantity::RelativeTotalPressure => "relative-total-pressure",
            Quantity::Rothalpy => "rothalpy",
            Quantity::DissipationRate => "dissipation-rate",
            Quantity::SpecificDissipationRate => "specific-dissipation-rate",
            Quantity::TimeScale => "time-scale",
            Quantity::LengthScale => "length-scale",
            Quantity::ViscosityRatio => "viscosity-ratio",
            Quantity::ReynoldsStressXx => "reynolds-stress-xx",
            Quantity::ReynoldsStressYy => "reynolds-stress-yy",
            Quantity::ReynoldsStressZz => "reynolds-stress-zz",
            Quantity::ReynoldsStressXy => "reynolds-stress-xy",
            Quantity::ReynoldsStressYz => "reynolds-stress-yz",
            Quantity::ReynoldsStressXz => "reynolds-stress-xz",
            Quantity::WallDistance => "wall-distance",
        }
    }

    pub fn parse(s: &str) -> Option<Quantity> {
        Quantity::ALL.into_iter().find(|q| q.key() == s)
    }

    /// The name of the field it makes: FoehnCraft's dataset name (a
    /// vector without its `_x`, `_y`, `_z`, as MurmelCraft reads it).
    pub fn field(self) -> &'static str {
        match self {
            Quantity::Vorticity => "vorticity",
            Quantity::QCriterion => "q_criterion",
            Quantity::Lambda2 => "lambda2",
            Quantity::Enstrophy => "enstrophy",
            Quantity::StrainRate => "strain_rate",
            Quantity::TotalPressure => "total_pressure",
            Quantity::PressureCoefficient => "pressure_coefficient",
            Quantity::RelativeVelocity => "relative_velocity",
            Quantity::FrameVelocity => "frame_velocity",
            Quantity::VelocityRadial => "velocity_radial",
            Quantity::VelocityCircumferential => "velocity_circumferential",
            Quantity::VelocityAxial => "velocity_axial",
            Quantity::VelocityMeridional => "velocity_meridional",
            Quantity::RelativeVelocityCircumferential => "relative_velocity_circumferential",
            Quantity::FlowAngle => "flow_angle",
            Quantity::RelativeFlowAngle => "relative_flow_angle",
            Quantity::RelativeTotalPressure => "relative_total_pressure",
            Quantity::Rothalpy => "rothalpy",
            Quantity::DissipationRate => "dissipation_rate",
            Quantity::SpecificDissipationRate => "specific_dissipation_rate",
            Quantity::TimeScale => "time_scale",
            Quantity::LengthScale => "length_scale",
            Quantity::ViscosityRatio => "viscosity_ratio",
            Quantity::ReynoldsStressXx => "reynolds_stress_xx",
            Quantity::ReynoldsStressYy => "reynolds_stress_yy",
            Quantity::ReynoldsStressZz => "reynolds_stress_zz",
            Quantity::ReynoldsStressXy => "reynolds_stress_xy",
            Quantity::ReynoldsStressYz => "reynolds_stress_yz",
            Quantity::ReynoldsStressXz => "reynolds_stress_xz",
            Quantity::WallDistance => "wall_distance",
        }
    }

    /// Its unit, for a pressure in Pa.
    pub fn unit(self) -> &'static str {
        use Quantity::*;
        match self {
            Vorticity | StrainRate | SpecificDissipationRate => "1/s",
            QCriterion | Lambda2 | Enstrophy => "1/s²",
            TotalPressure | RelativeTotalPressure => "Pa",
            PressureCoefficient | ViscosityRatio => "-",
            RelativeVelocity
            | FrameVelocity
            | VelocityRadial
            | VelocityCircumferential
            | VelocityAxial
            | VelocityMeridional
            | RelativeVelocityCircumferential => "m/s",
            FlowAngle | RelativeFlowAngle => "°",
            Rothalpy => "m²/s²",
            DissipationRate => "m²/s³",
            TimeScale => "s",
            LengthScale | WallDistance => "m",
            ReynoldsStressXx | ReynoldsStressYy | ReynoldsStressZz | ReynoldsStressXy
            | ReynoldsStressYz | ReynoldsStressXz => "m²/s²",
        }
    }

    /// The formula, as the catalogue shows it.
    pub fn formula(self) -> &'static str {
        use Quantity::*;
        match self {
            Vorticity => "ω = ∇ × u",
            QCriterion => "Q = ½ (‖Ω‖² − ‖S‖²)",
            Lambda2 => "λ₂ of S² + Ω²",
            Enstrophy => "½ ω·ω",
            StrainRate => "√(2 S:S)",
            TotalPressure => "p + ½ ρ u·u",
            PressureCoefficient => "(p − p_ref) / (½ ρ_ref U_ref²)",
            RelativeVelocity => "w = c − Ω × (x − x₀)",
            FrameVelocity => "u = Ω × (x − x₀)",
            VelocityRadial => "c_r = c·e_r",
            VelocityCircumferential => "c_u = c·e_θ",
            VelocityAxial => "c_a = c·e_a",
            VelocityMeridional => "c_m = √(c_r² + c_a²)",
            RelativeVelocityCircumferential => "w_u = c_u − u_u",
            FlowAngle => "α = atan2(c_m, c_u)",
            RelativeFlowAngle => "β = atan2(c_m, −w_u)",
            RelativeTotalPressure => "p + ½ ρ w·w",
            Rothalpy => "I = p/ρ + ½ w·w − ½ u·u",
            DissipationRate => "ε = C_μ k ω",
            SpecificDissipationRate => "ω = ε / (C_μ k)",
            TimeScale => "T = k/ε = 1/(C_μ ω)",
            LengthScale => "L = k^1.5 / ε",
            ViscosityRatio => "ν_t / ν",
            ReynoldsStressXx => "2/3 k − 2 ν_t S_xx",
            ReynoldsStressYy => "2/3 k − 2 ν_t S_yy",
            ReynoldsStressZz => "2/3 k − 2 ν_t S_zz",
            ReynoldsStressXy => "−2 ν_t S_xy",
            ReynoldsStressYz => "−2 ν_t S_yz",
            ReynoldsStressXz => "−2 ν_t S_xz",
            WallDistance => "d = distance to the nearest wall face",
        }
    }

    pub fn group(self) -> Group {
        use Quantity::*;
        match self {
            Vorticity | QCriterion | Lambda2 | Enstrophy | StrainRate => Group::Vortices,
            TotalPressure | PressureCoefficient => Group::Pressure,
            RelativeVelocity
            | FrameVelocity
            | VelocityRadial
            | VelocityCircumferential
            | VelocityAxial
            | VelocityMeridional
            | RelativeVelocityCircumferential
            | FlowAngle
            | RelativeFlowAngle
            | RelativeTotalPressure
            | Rothalpy => Group::Turbomachinery,
            DissipationRate
            | SpecificDissipationRate
            | TimeScale
            | LengthScale
            | ViscosityRatio
            | ReynoldsStressXx
            | ReynoldsStressYy
            | ReynoldsStressZz
            | ReynoldsStressXy
            | ReynoldsStressYz
            | ReynoldsStressXz => Group::Turbulence,
            WallDistance => Group::Wall,
        }
    }

    pub fn inputs(self) -> &'static [Input] {
        use Quantity::*;
        match self {
            Vorticity | QCriterion | Lambda2 | Enstrophy | StrainRate => &[Input::Velocity],
            TotalPressure | RelativeTotalPressure | Rothalpy => &[Input::Pressure, Input::Velocity],
            PressureCoefficient => &[Input::Pressure],
            FrameVelocity | WallDistance => &[],
            RelativeVelocity
            | VelocityRadial
            | VelocityCircumferential
            | VelocityAxial
            | VelocityMeridional
            | RelativeVelocityCircumferential
            | FlowAngle
            | RelativeFlowAngle => &[Input::Velocity],
            DissipationRate | SpecificDissipationRate | TimeScale | LengthScale => {
                &[Input::K, Input::Dissipation]
            }
            ViscosityRatio => &[Input::NuT],
            ReynoldsStressXx | ReynoldsStressYy | ReynoldsStressZz | ReynoldsStressXy
            | ReynoldsStressYz | ReynoldsStressXz => &[Input::Velocity, Input::K, Input::NuT],
        }
    }

    /// The reference values it uses, by their keys in [`Reference`].
    pub fn references(self) -> &'static [&'static str] {
        use Quantity::*;
        match self {
            TotalPressure | RelativeTotalPressure | Rothalpy => &["density"],
            PressureCoefficient => &["density", "pressure", "velocity"],
            DissipationRate | SpecificDissipationRate | TimeScale | LengthScale => &["c_mu"],
            ViscosityRatio => &["viscosity"],
            _ => &[],
        }
    }

    /// Whether it takes the turning zones of a [`crate::turbo::Rotation`].
    pub fn turns(self) -> bool {
        self.group() == Group::Turbomachinery
    }

    /// Whether it takes the wall patches.
    pub fn walls(self) -> bool {
        self == Quantity::WallDistance
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
    /// The kinematic viscosity ν of the fluid (m²/s), above 0, for the
    /// viscosity ratio.
    #[serde(default = "air_viscosity")]
    pub viscosity: f64,
    /// C_μ of the turbulence model (β* of SST), above 0, for the scales
    /// of ω and ε.
    #[serde(default = "standard_c_mu")]
    pub c_mu: f64,
}

/// Built-in kinematic viscosity: air at sea level in the ICAO standard
/// atmosphere, μ = 1.7894e-5 Pa s over ρ = 1.225 kg/m³ (m²/s).
pub const AIR_VISCOSITY: f64 = 1.7894e-5 / AIR_DENSITY;
/// Built-in C_μ: 0.09, that of every model of FoehnCraft (β* of SST).
pub const C_MU: f64 = 0.09;

fn air_viscosity() -> f64 {
    AIR_VISCOSITY
}

fn standard_c_mu() -> f64 {
    C_MU
}

/// The keys of [`Reference`], in the order of [`start_reference`]'s
/// sources.
pub const REFERENCE_KEYS: [&str; 5] = ["density", "pressure", "velocity", "viscosity", "c_mu"];

impl Default for Reference {
    fn default() -> Self {
        Reference {
            density: AIR_DENSITY,
            pressure: 0.0,
            velocity: 1.0,
            viscosity: AIR_VISCOSITY,
            c_mu: C_MU,
        }
    }
}

impl Reference {
    /// The value of key `k` of [`REFERENCE_KEYS`].
    pub fn get(&self, k: usize) -> f64 {
        [
            self.density,
            self.pressure,
            self.velocity,
            self.viscosity,
            self.c_mu,
        ][k]
    }

    /// Value `k` of [`REFERENCE_KEYS`], to edit.
    pub fn get_mut(&mut self, k: usize) -> &mut f64 {
        match k {
            0 => &mut self.density,
            1 => &mut self.pressure,
            2 => &mut self.velocity,
            3 => &mut self.viscosity,
            _ => &mut self.c_mu,
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
        if !(ok(self.viscosity) && self.viscosity > 0.0) {
            return Err(format!(
                "kinematic viscosity {} must be above 0 and finite",
                self.viscosity
            ));
        }
        if !(ok(self.c_mu) && self.c_mu > 0.0) {
            return Err(format!("C_mu {} must be above 0 and finite", self.c_mu));
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
/// velocity`, else `[output.coefficients] velocity`; ν from `[fluid]
/// viscosity` (μ) over `[fluid] density` when both are numbers; C_μ from
/// `[turbulence.constants]` (`beta_star` for SST, `c_mu_eq` for the ζ-f
/// family, else `c_mu`). Each `None` the case does not give, in the
/// order of [`REFERENCE_KEYS`].
pub fn case_reference(result: &Path) -> Option<[Option<f64>; 5]> {
    let file = result.parent()?.join("case.resolved.toml");
    let text = std::fs::read_to_string(file).ok()?;
    let case: toml::Table = toml::from_str(&text).ok()?;
    let value = |path: &[&str]| -> Option<&toml::Value> {
        let mut at = case.get(path[0])?;
        for k in &path[1..] {
            at = at.get(k)?;
        }
        Some(at)
    };
    let number = |path: &[&str]| -> Option<f64> {
        let at = value(path)?;
        at.as_float()
            .or_else(|| at.as_integer().map(|i| i as f64))
            .filter(|x| x.is_finite())
    };
    let fluid_density = number(&["fluid", "density"]).filter(|&x| x > 0.0);
    let density = number(&["output", "reference", "density"])
        .filter(|&x| x > 0.0)
        .or(fluid_density);
    let pressure = number(&["output", "reference", "pressure"]);
    let velocity = number(&["output", "reference", "velocity"])
        .or_else(|| number(&["output", "coefficients", "velocity"]))
        .filter(|&x| x > 0.0);
    let viscosity = number(&["fluid", "viscosity"])
        .filter(|&x| x > 0.0)
        .zip(fluid_density)
        .map(|(mu, rho)| mu / rho);
    let model = value(&["turbulence", "model"]).and_then(|m| m.as_str());
    let key = match model {
        Some("k-omega-sst") => "beta_star",
        Some("k-epsilon-zeta-f" | "k-epsilon-zeta-f0" | "elliptic-blending-k-epsilon") => "c_mu_eq",
        _ => "c_mu",
    };
    let c_mu = number(&["turbulence", "constants", key]).filter(|&x| x > 0.0);
    let found = [density, pressure, velocity, viscosity, c_mu];
    found.iter().any(Option::is_some).then_some(found)
}

/// The reference values a new derived field of `s` starts from, each
/// with where it came from (in the order of [`REFERENCE_KEYS`]):
/// FoehnCraft's case beside the result, else the user's default, else
/// built in (air at sea level, p_ref 0, U_ref the largest speed of the
/// velocity, 1 m/s without one, C_μ 0.09).
pub fn start_reference(s: &Session) -> (Reference, [Source; 5]) {
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
        velocity: speed.unwrap_or(1.0),
        ..Reference::default()
    };
    let mine = s.defaults.apply(Kind::Reference, built);
    let mut r = mine;
    let mut from = [Source::BuiltIn; 5];
    for (k, key) in REFERENCE_KEYS.into_iter().enumerate() {
        if let Some(x) = case.and_then(|c| c[k]) {
            *r.get_mut(k) = x;
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
    /// The turbulent kinetic energy; left out, found by its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k: Option<String>,
    /// ω or ε; left out, found by its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dissipation: Option<String>,
    /// The eddy viscosity; left out, found by its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nu_t: Option<String>,
    /// The reference values and constants (pressure and turbulence
    /// quantities).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<Reference>,
    /// The turning zones (turbomachinery quantities); left out, every
    /// cell about the z axis, not turning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<crate::turbo::Rotation>,
    /// The wall patches (wall distance); left out, the patches whose
    /// role is wall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walls: Option<Vec<String>>,
}

/// The ζ-f family's bounded turbulence time scale, which FoehnCraft
/// writes beside k and ε; the time scale takes it where it is.
const TURBULENT_TIME_SCALE: &str = "turbulent_time_scale";

impl Recipe {
    pub fn new(quantity: Quantity) -> Recipe {
        Recipe {
            quantity,
            velocity: None,
            pressure: None,
            k: None,
            dissipation: None,
            nu_t: None,
            reference: None,
            rotation: None,
            walls: None,
        }
    }

    /// The field it makes.
    pub fn field(&self) -> &'static str {
        self.quantity.field()
    }

    /// The field named for `input`, if one is.
    pub fn named(&self, input: Input) -> Option<&String> {
        match input {
            Input::Velocity => self.velocity.as_ref(),
            Input::Pressure => self.pressure.as_ref(),
            Input::K => self.k.as_ref(),
            Input::Dissipation => self.dissipation.as_ref(),
            Input::NuT => self.nu_t.as_ref(),
        }
    }

    /// Name `field` for `input`.
    pub fn set_input(&mut self, input: Input, field: Option<String>) {
        let slot = match input {
            Input::Velocity => &mut self.velocity,
            Input::Pressure => &mut self.pressure,
            Input::K => &mut self.k,
            Input::Dissipation => &mut self.dissipation,
            Input::NuT => &mut self.nu_t,
        };
        *slot = field;
    }

    /// The field it reads for `input`: the one named, else the first
    /// field of the result that fits by name and shape.
    pub fn input(&self, set: &FieldSet, input: Input) -> Option<String> {
        if let Some(n) = self.named(input) {
            return Some(n.clone());
        }
        find_input(set, input)
    }

    /// The fields of `set` it reads: its inputs, the density fields the
    /// pressure quantities take and the time scale of ζ-f.
    pub fn reads(&self, set: &FieldSet) -> Vec<String> {
        let mut out: Vec<String> = self
            .quantity
            .inputs()
            .iter()
            .filter_map(|&i| self.input(set, i))
            .collect();
        if self.quantity.references().contains(&"density") {
            for n in ["density", "rho"] {
                if set.get(n).is_some() {
                    out.push(n.into());
                }
            }
        }
        if self.quantity == Quantity::TimeScale && set.get(TURBULENT_TIME_SCALE).is_some() {
            out.push(TURBULENT_TIME_SCALE.into());
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
    /// Turning zones that cannot be used: why (FIELD016).
    Rotation(String),
    /// No wall face to measure the distance to: why (FIELD017).
    Walls(String),
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
            Refusal::Rotation(_) => "FIELD016",
            Refusal::Walls(_) => "FIELD017",
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
            | Refusal::Cycle(why)
            | Refusal::Rotation(why)
            | Refusal::Walls(why) => why.clone(),
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
/// named [`Quantity::field`]: on cells for the quantities of the
/// velocity gradient, rotating machinery, turbulence and walls (a node
/// field read there as the mean of each cell's nodes), and wherever the
/// inputs are for the total pressure and its coefficient.
pub fn compute(mesh: &Mesh, set: &FieldSet, recipe: &Recipe) -> Result<Vec<Field>, Refusal> {
    use Quantity::*;
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
    let gradient = || -> Result<Vec<[[f64; 3]; 3]>, Refusal> {
        let u = need(Input::Velocity)?;
        u.iter()
            .find(|f| f.location != Location::Boundary)
            .and_then(|f| set.velocity_gradient(mesh, f))
            .ok_or_else(|| {
                Refusal::Shape(
                    Input::Velocity,
                    "is only on the boundary faces; the gradient needs it in the cells or at the nodes"
                        .into(),
                )
            })
    };
    let cell = |components: usize, values: Vec<f64>, unit: Option<String>| Field {
        name: name.into(),
        unit: Some(unit.unwrap_or_else(|| q.unit().into())),
        location: Location::Cell,
        components,
        values,
    };
    match q {
        Vorticity | QCriterion | Lambda2 | Enstrophy | StrainRate => {
            let g = gradient()?;
            let (components, values) = match q {
                Vorticity => (3, g.iter().flat_map(|g| vorticity(*g)).collect()),
                QCriterion => (1, g.iter().map(|g| kinematics(*g).0).collect()),
                Lambda2 => (1, g.iter().map(|g| kinematics(*g).1).collect()),
                Enstrophy => (
                    1,
                    g.iter()
                        .map(|g| 0.5 * vorticity(*g).iter().map(|w| w * w).sum::<f64>())
                        .collect(),
                ),
                _ => (1, g.iter().map(|g| strain_rate(*g)).collect()),
            };
            Ok(vec![cell(components, values, None)])
        }
        TotalPressure | PressureCoefficient => pressure_fields(set, recipe, &reference, need),
        _ if q.turns() => {
            let rotation = recipe.rotation.clone().unwrap_or_default();
            let frames = rotation.frames(mesh).map_err(Refusal::Rotation)?;
            let x = set.places(mesh, Location::Cell);
            let c = if q.inputs().contains(&Input::Velocity) {
                Some(cell_input(mesh, set, Input::Velocity, &need(Input::Velocity)?)?.0)
            } else {
                None
            };
            // p, its unit and ρ in each cell for the pressure ones.
            let pressure = if q.inputs().contains(&Input::Pressure) {
                let (p, f) = cell_input(mesh, set, Input::Pressure, &need(Input::Pressure)?)?;
                let kin = kinematic(f.unit.as_deref());
                let rho = (!kin)
                    .then(|| {
                        ["density", "rho"].into_iter().find_map(|n| {
                            let fs: Vec<&Field> = [Location::Cell, Location::Point]
                                .into_iter()
                                .filter_map(|l| set.at(n, l))
                                .filter(|f| f.components == 1 && FieldSet::fits(f, mesh))
                                .collect();
                            cell_input(mesh, set, Input::Pressure, &fs)
                                .ok()
                                .map(|r| r.0)
                        })
                    })
                    .flatten();
                Some((p, kin, f.unit.clone(), rho))
            } else {
                None
            };
            let n = mesh.cell_count();
            let k = if matches!(q, RelativeVelocity | FrameVelocity) {
                3
            } else {
                1
            };
            let values: Vec<f64> = murmel_mesh::par::map(n, 4096, |r| {
                let mut out = Vec::with_capacity(r.len() * k);
                for i in r {
                    let cv = c
                        .as_ref()
                        .map_or([0.0; 3], |c| [c[3 * i], c[3 * i + 1], c[3 * i + 2]]);
                    let frame = frames[i].map(|z| &rotation.zones[z as usize]);
                    let axis = frame.unwrap_or(&rotation.zones[0]);
                    let t = crate::turbo::turbo(cv, x[i], frame, axis);
                    let half = |v: [f64; 3]| 0.5 * (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
                    match q {
                        RelativeVelocity => out.extend(t.w),
                        FrameVelocity => out.extend(t.u),
                        VelocityRadial => out.push(t.c_r),
                        VelocityCircumferential => out.push(t.c_u),
                        VelocityAxial => out.push(t.c_a),
                        VelocityMeridional => out.push(t.c_m),
                        RelativeVelocityCircumferential => out.push(t.w_u),
                        FlowAngle => out.push(t.alpha),
                        RelativeFlowAngle => out.push(t.beta),
                        _ => {
                            let Some((p, kin, _, rho)) = pressure.as_ref() else {
                                out.push(f64::NAN);
                                continue;
                            };
                            let rho = if *kin {
                                1.0
                            } else {
                                rho.as_ref().map_or(reference.density, |r| r[i])
                            };
                            out.push(if q == RelativeTotalPressure {
                                p[i] + rho * half(t.w)
                            } else {
                                p[i] / rho + half(t.w) - half(t.u)
                            });
                        }
                    }
                }
                out
            })
            .concat();
            // A kinematic pressure gives a relative total pressure in its
            // own unit.
            let unit = match (&pressure, q) {
                (Some((_, true, u, _)), RelativeTotalPressure) => u.clone(),
                _ => None,
            };
            Ok(vec![cell(k, values, unit)])
        }
        DissipationRate | SpecificDissipationRate | TimeScale | LengthScale => {
            let (kv, _) = cell_input(mesh, set, Input::K, &need(Input::K)?)?;
            let wf = need(Input::Dissipation)?;
            let (wv, f) = cell_input(mesh, set, Input::Dissipation, &wf)?;
            let epsilon = is_epsilon(&f.name, f.unit.as_deref());
            let bounded = (q == TimeScale)
                .then(|| {
                    let fs: Vec<&Field> = [Location::Cell, Location::Point]
                        .into_iter()
                        .filter_map(|l| set.at(TURBULENT_TIME_SCALE, l))
                        .filter(|f| f.components == 1 && FieldSet::fits(f, mesh))
                        .collect();
                    cell_input(mesh, set, Input::K, &fs).ok().map(|r| r.0)
                })
                .flatten();
            let slot = match q {
                DissipationRate => 0,
                SpecificDissipationRate => 1,
                TimeScale => 2,
                _ => 3,
            };
            let values = (0..kv.len())
                .map(|i| {
                    scales(
                        kv[i],
                        wv[i],
                        epsilon,
                        bounded.as_ref().map(|t| t[i]),
                        reference.c_mu,
                    )[slot]
                })
                .collect();
            Ok(vec![cell(1, values, None)])
        }
        ViscosityRatio => {
            let (nt, _) = cell_input(mesh, set, Input::NuT, &need(Input::NuT)?)?;
            let values = nt.iter().map(|v| v / reference.viscosity).collect();
            Ok(vec![cell(1, values, None)])
        }
        ReynoldsStressXx | ReynoldsStressYy | ReynoldsStressZz | ReynoldsStressXy
        | ReynoldsStressYz | ReynoldsStressXz => {
            let g = gradient()?;
            let (kv, _) = cell_input(mesh, set, Input::K, &need(Input::K)?)?;
            let (nt, _) = cell_input(mesh, set, Input::NuT, &need(Input::NuT)?)?;
            let slot = match q {
                ReynoldsStressXx => 0,
                ReynoldsStressYy => 1,
                ReynoldsStressZz => 2,
                ReynoldsStressXy => 3,
                ReynoldsStressYz => 4,
                _ => 5,
            };
            let values = (0..g.len())
                .map(|i| reynolds_stress(g[i], kv[i], nt[i])[slot])
                .collect();
            Ok(vec![cell(1, values, None)])
        }
        WallDistance => {
            let walls = recipe.walls.clone().unwrap_or_else(|| {
                mesh.patches
                    .iter()
                    .filter(|p| p.role == murmel_mesh::PatchRole::Wall)
                    .map(|p| p.name.clone())
                    .collect()
            });
            let patches = || {
                mesh.patches
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            if walls.is_empty() {
                return Err(Refusal::Walls(format!(
                    "no patch is a wall; name the wall patches (the mesh's patches: {})",
                    patches()
                )));
            }
            if let Some(w) = walls
                .iter()
                .find(|w| !mesh.patches.iter().any(|p| &p.name == *w))
            {
                return Err(Refusal::Walls(format!(
                    "the mesh has no patch {w}; its patches are {}",
                    patches()
                )));
            }
            let x = set.places(mesh, Location::Cell);
            let d = crate::walls::wall_distance(mesh, &walls, x).ok_or_else(|| {
                Refusal::Walls(format!("the patches {} have no face", walls.join(", ")))
            })?;
            Ok(vec![cell(1, d, None)])
        }
        _ => unreachable!("every quantity has its branch"),
    }
}

/// The total pressure or its coefficient wherever the pressure is.
fn pressure_fields<'a>(
    set: &'a FieldSet,
    recipe: &Recipe,
    reference: &Reference,
    need: impl Fn(Input) -> Result<Vec<&'a Field>, Refusal>,
) -> Result<Vec<Field>, Refusal> {
    let q = recipe.quantity;
    let name = q.field();
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

/// The values of `input` in each cell from its fields `found` (the cell
/// field, else the mean of each cell's nodes), with the field taken.
fn cell_input<'a>(
    mesh: &Mesh,
    set: &FieldSet,
    input: Input,
    found: &[&'a Field],
) -> Result<(Vec<f64>, &'a Field), Refusal> {
    if let Some(f) = found.iter().find(|f| f.location == Location::Cell) {
        return Ok((f.values.clone(), f));
    }
    if let Some(f) = found.iter().find(|f| f.location == Location::Point) {
        return Ok((set.cell_means(mesh, &f.values, f.components), f));
    }
    Err(Refusal::Shape(
        input,
        "is only on the boundary faces; it is needed in the cells or at the nodes".into(),
    ))
}

/// Whether the second turbulence variable `name` (unit `unit`) is ε,
/// not ω: by its name, else by its unit (m²/s³).
fn is_epsilon(name: &str, unit: Option<&str>) -> bool {
    let n = name.to_lowercase();
    if n.contains("eps") || n.contains("turbulent_dissipation") {
        return true;
    }
    if n.contains("omega") || n.contains("specific") {
        return false;
    }
    unit.is_some_and(|u| {
        let u: String = u.chars().filter(|c| !c.is_whitespace()).collect();
        matches!(
            u.as_str(),
            "m2/s3" | "m²/s³" | "m^2/s^3" | "m2s-3" | "m2.s-3"
        )
    })
}

/// ε, ω, the time scale and L = k^{3/2}/ε of one cell from k, the
/// second variable `w` (ε when `epsilon`, else ω), the bounded time
/// scale of ζ-f where there is one, and C_μ (β* of SST), as FoehnCraft's
/// `turbulence-scales` computes them; every denominator is floored at
/// 10⁻³⁰⁰ (a fluid at rest).
pub fn scales(k: f64, w: f64, epsilon: bool, bounded: Option<f64>, c_mu: f64) -> [f64; 4] {
    let tiny = 1e-300;
    let k = k.max(0.0);
    let (eps, omega) = if epsilon {
        (w, w / (c_mu * k).max(tiny))
    } else {
        (c_mu * k * w, w)
    };
    let t = bounded.unwrap_or(if epsilon {
        k / eps.max(tiny)
    } else {
        1.0 / (c_mu * omega).max(tiny)
    });
    [eps, omega, t, k * k.sqrt() / eps.max(tiny)]
}

/// The Boussinesq Reynolds stress ⅔ k δᵢⱼ − 2 ν_t Sᵢⱼ of the velocity
/// gradient `g`, components xx, yy, zz, xy, yz, xz.
pub fn reynolds_stress(g: [[f64; 3]; 3], k: f64, nu_t: f64) -> [f64; 6] {
    let s = |i: usize, j: usize| 0.5 * (g[i][j] + g[j][i]);
    let iso = 2.0 / 3.0 * k.max(0.0);
    [
        iso - 2.0 * nu_t * s(0, 0),
        iso - 2.0 * nu_t * s(1, 1),
        iso - 2.0 * nu_t * s(2, 2),
        -2.0 * nu_t * s(0, 1),
        -2.0 * nu_t * s(1, 2),
        -2.0 * nu_t * s(0, 2),
    ]
}

/// √(2 S:S) of the velocity gradient `g`, the shear rate.
pub fn strain_rate(g: [[f64; 3]; 3]) -> f64 {
    let mut ss = 0.0;
    for i in 0..3 {
        for j in 0..3 {
            let s = 0.5 * (g[i][j] + g[j][i]);
            ss += s * s;
        }
    }
    (2.0 * ss).sqrt()
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
            "[fluid]\ndensity = 998.2\nviscosity = 0.001\n[output.reference]\npressure = 101325.0\n[output.coefficients]\nvelocity = 2\n[turbulence]\nmodel = \"k-omega-sst\"\n[turbulence.constants]\nbeta_star = 0.1\nc_mu = 0.5\n",
        )
        .unwrap();
        assert_eq!(
            case_reference(&result),
            Some([
                Some(998.2),
                Some(101325.0),
                Some(2.0),
                Some(0.001 / 998.2),
                Some(0.1)
            ])
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

    fn cells(m: &Mesh) -> Vec<[f64; 3]> {
        (0..m.cell_count())
            .map(|c| crate::inspect::cell_centroid(m, c))
            .collect()
    }

    fn scalar_cells(name: &str, unit: &str, values: Vec<f64>) -> Field {
        Field {
            name: name.into(),
            unit: Some(unit.into()),
            location: Location::Cell,
            components: 1,
            values,
        }
    }

    /// Invariant: enstrophy is ½ ω·ω and the strain rate √(2 S:S) of
    /// a velocity linear in space, in every cell. Breaks when the
    /// strain sums the antisymmetric part.
    #[test]
    fn enstrophy_and_strain_rate() {
        let (m, set) = polyhedra(|m| {
            vec![Field {
                name: "velocity".into(),
                unit: Some("m/s".into()),
                location: Location::Cell,
                components: 3,
                values: cells(m).iter().flat_map(|&x| linear(x)).collect(),
            }]
        });
        let w = vorticity(G);
        let want = 0.5 * w.iter().map(|x| x * x).sum::<f64>();
        let e = compute(&m, &set, &Recipe::new(Quantity::Enstrophy)).unwrap();
        assert!(e[0].values.iter().all(|v| (v - want).abs() < 1e-6));
        let mut ss = 0.0;
        for i in 0..3 {
            for j in 0..3 {
                ss += (0.5 * (G[i][j] + G[j][i])).powi(2);
            }
        }
        let s = compute(&m, &set, &Recipe::new(Quantity::StrainRate)).unwrap();
        assert_eq!(s[0].unit.as_deref(), Some("1/s"));
        assert!(
            s[0].values
                .iter()
                .all(|v| (v - (2.0 * ss).sqrt()).abs() < 1e-6)
        );
    }

    /// Invariant: a fluid turning with the rotor as a solid body has no
    /// relative velocity, all its absolute velocity is swirl (α = 0),
    /// its frame velocity is Ω × x, and its rothalpy is p/ρ − ½ u·u.
    /// Breaks when a cell is seen in the wrong frame or **u** takes the
    /// wrong sign.
    #[test]
    fn turbomachinery_of_a_solid_body_rotation() {
        let rpm = 600.0;
        let w = rpm * std::f64::consts::TAU / 60.0;
        let (m, set) = polyhedra(|m| {
            let x = cells(m);
            vec![
                Field {
                    name: "velocity".into(),
                    unit: Some("m/s".into()),
                    location: Location::Cell,
                    components: 3,
                    values: x.iter().flat_map(|p| [-w * p[1], w * p[0], 0.0]).collect(),
                },
                scalar_cells("pressure", "Pa", vec![100.0; x.len()]),
            ]
        });
        let rotation = crate::turbo::Rotation {
            zones: vec![crate::turbo::Turning {
                rpm,
                ..Default::default()
            }],
            frame: None,
        };
        let make = |q: Quantity| {
            let mut r = Recipe::new(q);
            r.rotation = Some(rotation.clone());
            r.reference = Some(Reference {
                density: 2.0,
                ..Default::default()
            });
            compute(&m, &set, &r).unwrap().remove(0)
        };
        let x = cells(&m);
        let rel = make(Quantity::RelativeVelocity);
        assert!(rel.is_vector());
        assert!(rel.values.iter().all(|v| v.abs() < 1e-9));
        let frame = make(Quantity::FrameVelocity);
        for (c, p) in x.iter().enumerate() {
            let u = frame.value(c).unwrap();
            assert!((u[0] + w * p[1]).abs() < 1e-9 && (u[1] - w * p[0]).abs() < 1e-9);
        }
        let alpha = make(Quantity::FlowAngle);
        assert_eq!(alpha.unit.as_deref(), Some("°"));
        let cu = make(Quantity::VelocityCircumferential);
        let roth = make(Quantity::Rothalpy);
        let rtp = make(Quantity::RelativeTotalPressure);
        for (c, p) in x.iter().enumerate() {
            let r2 = p[0] * p[0] + p[1] * p[1];
            if r2 < 1e-6 {
                continue;
            }
            assert!(alpha.values[c].abs() < 1e-6, "{}", alpha.values[c]);
            assert!((cu.values[c] - w * r2.sqrt()).abs() < 1e-9);
            assert!((roth.values[c] - (50.0 - 0.5 * w * w * r2)).abs() < 1e-9);
            assert!((rtp.values[c] - 100.0).abs() < 1e-9);
        }
        // A zone the mesh does not have.
        let mut r = Recipe::new(Quantity::VelocityAxial);
        r.rotation = Some(crate::turbo::Rotation {
            zones: vec![crate::turbo::Turning {
                zone: Some("impeller".into()),
                ..Default::default()
            }],
            frame: None,
        });
        assert_eq!(compute(&m, &set, &r).map_err(|e| e.code()), Err("FIELD016"));
    }

    /// Invariant: the turbulence scales follow from k and ω, or from k
    /// and ε, as FoehnCraft computes them; the viscosity ratio is ν_t/ν
    /// and the Reynolds stress the Boussinesq estimate, exact for a
    /// linear velocity. Breaks when ω and ε are told apart wrongly.
    #[test]
    fn turbulence_scales() {
        let fields = |second: &str, unit: &str, v: f64| {
            let (m, set) = polyhedra(|m| {
                let n = m.cell_count();
                vec![
                    Field {
                        name: "velocity".into(),
                        unit: Some("m/s".into()),
                        location: Location::Cell,
                        components: 3,
                        values: cells(m).iter().flat_map(|&x| linear(x)).collect(),
                    },
                    scalar_cells("k", "m2/s2", vec![2.0; n]),
                    scalar_cells(second, unit, vec![v; n]),
                    scalar_cells("nu_t", "m2/s", vec![0.01; n]),
                ]
            });
            (m, set)
        };
        let get = |m: &Mesh, set: &FieldSet, q: Quantity| {
            let mut r = Recipe::new(q);
            r.reference = Some(Reference {
                viscosity: 1e-5,
                ..Default::default()
            });
            compute(m, set, &r).unwrap().remove(0).values[3]
        };
        let (m, set) = fields("omega", "1/s", 10.0);
        let eps = get(&m, &set, Quantity::DissipationRate);
        assert!((eps - 0.09 * 2.0 * 10.0).abs() < 1e-12);
        assert!((get(&m, &set, Quantity::SpecificDissipationRate) - 10.0).abs() < 1e-12);
        assert!((get(&m, &set, Quantity::TimeScale) - 1.0 / 0.9).abs() < 1e-12);
        assert!((get(&m, &set, Quantity::LengthScale) - 2f64.powf(1.5) / eps).abs() < 1e-12);
        assert!((get(&m, &set, Quantity::ViscosityRatio) - 1000.0).abs() < 1e-9);
        let xy = get(&m, &set, Quantity::ReynoldsStressXy);
        assert!((xy - (-2.0 * 0.01 * 0.5 * (G[0][1] + G[1][0]))).abs() < 1e-8);
        let xx = get(&m, &set, Quantity::ReynoldsStressXx);
        assert!((xx - (4.0 / 3.0 - 2.0 * 0.01 * G[0][0])).abs() < 1e-8);
        // ε by its name.
        let (m, set) = fields("epsilon", "m2/s3", 1.8);
        assert!((get(&m, &set, Quantity::SpecificDissipationRate) - 10.0).abs() < 1e-12);
        assert!((get(&m, &set, Quantity::TimeScale) - 2.0 / 1.8).abs() < 1e-12);
        // Without ν_t.
        let (m, set) = polyhedra(|m| vec![scalar_cells("k", "m2/s2", vec![1.0; m.cell_count()])]);
        assert_eq!(
            compute(&m, &set, &Recipe::new(Quantity::ViscosityRatio)),
            Err(Refusal::Missing(Input::NuT))
        );
        assert!(is_epsilon("eps", None) && !is_epsilon("omega", Some("m2/s3")));
        assert!(is_epsilon("w2", Some("m²/s³")) && !is_epsilon("w2", Some("1/s")));
    }

    /// Invariant: the wall distance of each cell is that of its centroid
    /// to the nearest named wall: in a box with walls at y = min and y =
    /// max, the smaller of the two gaps. Without a wall patch it is
    /// refused. Breaks when faces of other patches are taken.
    #[test]
    fn wall_distance_in_a_box() {
        let (m, set) = polyhedra(|m| vec![scalar_cells("p", "Pa", vec![0.0; m.cell_count()])]);
        let (lo, hi) = m
            .points
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
                (a.min(p[1]), b.max(p[1]))
            });
        let mut r = Recipe::new(Quantity::WallDistance);
        r.walls = Some(vec!["ymin".into(), "ymax".into()]);
        let d = compute(&m, &set, &r).unwrap().remove(0);
        assert_eq!(d.unit.as_deref(), Some("m"));
        for (c, x) in cells(&m).iter().enumerate() {
            let want = (x[1] - lo).min(hi - x[1]);
            assert!(
                (d.values[c] - want).abs() < 1e-9,
                "{c}: {} {want}",
                d.values[c]
            );
        }
        assert_eq!(
            compute(&m, &set, &Recipe::new(Quantity::WallDistance)).map_err(|e| e.code()),
            Err("FIELD017")
        );
        r.walls = Some(vec!["lid".into()]);
        assert_eq!(compute(&m, &set, &r).map_err(|e| e.code()), Err("FIELD017"));
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
            ..Default::default()
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
