//! FoehnCraft's own derived fields (the maintainer's card of 2026-11-24,
//! "Run foehn post derive for me"): the fields that belong to its
//! turbulence model and wall treatment, which only FoehnCraft computes
//! the same way it solved the flow. MurmelCraft finds the run beside the
//! result (its case and restart file), asks `foehn post derive` for the
//! fields, and reads the file it writes as the next step of the same
//! mesh ([`crate::session::Session::open_same_step`]), so the view, the
//! derived and calculated fields and the items stay. The file holds the
//! solution and only the derived fields asked for, so the FoehnCraft
//! fields the result had are asked for again ([`kept`]).
//!
//! | name | fields | what | unit |
//! |---|---|---|---|
//! | `y-plus` | `y_plus` (walls) | y⁺ = d u_τ / ν of the wall-adjacent cells | - |
//! | `wall-shear-stress` | `wall_shear_stress` (walls, vector) | the tangential viscous stress of the fluid on the wall | Pa |
//! | `turbulence-wall` | `friction_velocity`, `y_star`, `blending` (walls) | u_τ and y* of the wall treatment, and its viscous weight | m/s, -, - |
//! | `turbulence-features` | `feature_lambda1` … `feature_curvature` | the invariant features of FoehnCraft's D166 | - |
//! | `wall-normal` | `wall_normal` (vector) | n = ∇α/\|∇α\| of the elliptic-blending k-ε | - |
//! | `curvature-correction` | `rotation_function` | f_r1 of SST's curvature correction | - |
//! | `solid-fraction` | `solid_fraction` | χ, the share of each cell inside the case's immersed bodies (`[immersed]`) | - |
//!
//! Where the run is: the restart file of the result's step beside it
//! (`restart.h5` for `fields.vtkhdf`, `restart_<N>.h5` for
//! `fields_<N>.vtkhdf`), and the case file the run's record names
//! (`run.jsonl`, its header's `case_file` and `overrides`), else the
//! `case.resolved.toml` FoehnCraft wrote beside the result.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::derive::Group;

/// A derived field only FoehnCraft computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    YPlus,
    WallShearStress,
    TurbulenceWall,
    TurbulenceFeatures,
    WallNormal,
    CurvatureCorrection,
    SolidFraction,
}

impl Model {
    pub const ALL: [Model; 7] = [
        Model::YPlus,
        Model::WallShearStress,
        Model::TurbulenceWall,
        Model::TurbulenceFeatures,
        Model::WallNormal,
        Model::CurvatureCorrection,
        Model::SolidFraction,
    ];

    /// FoehnCraft's name (`[output] derived`, `foehn post derive
    /// --fields`).
    pub fn key(self) -> &'static str {
        match self {
            Model::YPlus => "y-plus",
            Model::WallShearStress => "wall-shear-stress",
            Model::TurbulenceWall => "turbulence-wall",
            Model::TurbulenceFeatures => "turbulence-features",
            Model::WallNormal => "wall-normal",
            Model::CurvatureCorrection => "curvature-correction",
            Model::SolidFraction => "solid-fraction",
        }
    }

    pub fn parse(s: &str) -> Option<Model> {
        Model::ALL.into_iter().find(|m| m.key() == s)
    }

    /// The fields it gives, as MurmelCraft reads them (a vector's `_x`,
    /// `_y`, `_z` and `_magnitude` as one field).
    pub fn fields(self) -> &'static [&'static str] {
        match self {
            Model::YPlus => &["y_plus"],
            Model::WallShearStress => &["wall_shear_stress"],
            Model::TurbulenceWall => &["friction_velocity", "y_star", "blending"],
            Model::TurbulenceFeatures => &[
                "feature_lambda1",
                "feature_lambda2",
                "feature_lambda3",
                "feature_lambda4",
                "feature_lambda5",
                "feature_wall_reynolds",
                "feature_time_ratio",
                "feature_viscosity_ratio",
                "feature_shielding",
                "feature_q",
                "feature_pressure_gradient",
                "feature_curvature",
            ],
            Model::WallNormal => &["wall_normal"],
            Model::CurvatureCorrection => &["rotation_function"],
            Model::SolidFraction => &["solid_fraction"],
        }
    }

    /// The unit of its fields, as the catalogue shows it.
    pub fn unit(self) -> &'static str {
        match self {
            Model::WallShearStress => "Pa",
            Model::TurbulenceWall => "m/s, -, -",
            _ => "-",
        }
    }

    /// The group of the catalogue it is listed in.
    pub fn group(self) -> Group {
        match self {
            Model::YPlus | Model::WallShearStress | Model::TurbulenceWall => Group::Wall,
            Model::SolidFraction => Group::Wall,
            _ => Group::Turbulence,
        }
    }

    /// Whether its fields are on the walls (the boundary faces).
    pub fn on_walls(self) -> bool {
        matches!(
            self,
            Model::YPlus | Model::WallShearStress | Model::TurbulenceWall
        )
    }
}

/// How to ask FoehnCraft for its fields of one result.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Plan {
    /// The case file of the run.
    pub case: PathBuf,
    /// The run's `--set key=value` overrides.
    pub sets: Vec<String>,
    /// The restart file of the result's step.
    pub restart: PathBuf,
    /// The file FoehnCraft writes: `derived.vtkhdf` or
    /// `derived_<N>.vtkhdf` beside the result.
    pub file: PathBuf,
}

impl Plan {
    /// The arguments of `foehn` for `models`, writing to `file`, and
    /// for the FoehnCraft fields `keep` (keys, [`kept`]) the open result
    /// has already, so the file it writes holds them too.
    pub fn args(&self, models: &[Model], keep: &[&str], file: &Path) -> Vec<String> {
        let mut keys: Vec<&str> = keep.to_vec();
        for m in models {
            if !keys.contains(&m.key()) {
                keys.push(m.key());
            }
        }
        let mut a: Vec<String> = vec![
            "post".into(),
            "derive".into(),
            self.case.display().to_string(),
            "--restart".into(),
            self.restart.display().to_string(),
            "--fields".into(),
            keys.join(","),
            "--file".into(),
            file.display().to_string(),
        ];
        for s in &self.sets {
            a.push("--set".into());
            a.push(s.clone());
        }
        a
    }
}

/// Every field FoehnCraft derives (its key in `--fields` and
/// `[output] derived`) with one of the fields it writes, as MurmelCraft
/// reads them: `foehn post derive` writes only the fields it is asked
/// for beside the solution, so a file that replaces the open result
/// asks again for those the result has.
pub const FOEHN_FIELDS: [(&str, &str); 22] = [
    ("vorticity", "vorticity"),
    ("q-criterion", "q_criterion"),
    ("lambda2", "lambda2"),
    ("enstrophy", "enstrophy"),
    ("strain-rate", "strain_rate"),
    ("pressure-coefficient", "pressure_coefficient"),
    ("total-pressure", "total_pressure"),
    ("relative-velocity", "relative_velocity"),
    ("frame-velocity", "frame_velocity"),
    ("turbomachinery", "velocity_radial"),
    ("relative-total-pressure", "relative_total_pressure"),
    ("rothalpy", "rothalpy"),
    ("wall-distance", "wall_distance"),
    ("wall-shear-stress", "wall_shear_stress"),
    ("y-plus", "y_plus"),
    ("turbulence-features", "feature_lambda1"),
    ("turbulence-scales", "dissipation_rate"),
    ("reynolds-stress", "reynolds_stress_xx"),
    ("wall-normal", "wall_normal"),
    ("turbulence-wall", "friction_velocity"),
    ("curvature-correction", "rotation_function"),
    ("solid-fraction", "solid_fraction"),
];

/// The FoehnCraft keys of the fields among `file_fields` (the fields the
/// result's file has, not those MurmelCraft made).
pub fn kept<'a>(file_fields: impl IntoIterator<Item = &'a str>) -> Vec<&'static str> {
    let names: Vec<&str> = file_fields.into_iter().collect();
    FOEHN_FIELDS
        .iter()
        .filter(|(_, f)| names.contains(f))
        .map(|(k, _)| *k)
        .collect()
}

/// Why FoehnCraft cannot be asked for a result's fields.
#[derive(Clone, Debug, PartialEq)]
pub enum Missing {
    /// The result is not a FoehnCraft fields file (`fields.vtkhdf`,
    /// `fields_<N>.vtkhdf`, `derived.vtkhdf`, `derived_<N>.vtkhdf`).
    NotFoehnCraft(PathBuf),
    /// No restart file for the result's step.
    Restart(PathBuf),
    /// Neither the run's case file nor `case.resolved.toml`.
    Case(PathBuf),
}

impl Missing {
    pub fn message(&self) -> String {
        match self {
            Missing::NotFoehnCraft(p) => format!(
                "{} is not a FoehnCraft result (fields.vtkhdf or fields_<N>.vtkhdf)",
                p.display()
            ),
            Missing::Restart(p) => format!(
                "no restart file {} beside the result; FoehnCraft writes it at the end of a run",
                p.display()
            ),
            Missing::Case(d) => format!(
                "the run's case file is not found (run.jsonl names none that exists, and there is no case.resolved.toml in {})",
                d.display()
            ),
        }
    }
}

/// The step part of a FoehnCraft result's name: `""` for
/// `fields.vtkhdf`, `"_12"` for `fields_12.vtkhdf` (also of the
/// `derived` files FoehnCraft writes after the run).
pub fn step_of(result: &Path) -> Option<String> {
    let name = result.file_name()?.to_str()?;
    let stem = name.strip_suffix(".vtkhdf")?;
    let rest = stem
        .strip_prefix("fields")
        .or_else(|| stem.strip_prefix("derived"))?;
    let ok = rest.is_empty()
        || rest
            .strip_prefix('_')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    ok.then(|| rest.to_string())
}

/// The case file and overrides the run's record (`run.jsonl` in `dir`)
/// names in its header, when that file exists: an absolute path, else
/// one below `dir` or the folders above it (a run is started from the
/// case's folder and writes into a folder beside or below it).
fn recorded_case(dir: &Path) -> Option<(PathBuf, Vec<String>)> {
    use std::io::BufRead;
    let f = std::fs::File::open(dir.join("run.jsonl")).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(f).read_line(&mut line).ok()?;
    let head: serde_json::Value = serde_json::from_str(&line).ok()?;
    if head.get("type").and_then(|t| t.as_str()) != Some("header") {
        return None;
    }
    let case = PathBuf::from(head.get("case_file")?.as_str()?);
    let sets: Vec<String> = head
        .get("overrides")
        .and_then(|o| o.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let found = if case.is_absolute() {
        case.is_file().then_some(case)
    } else {
        dir.ancestors()
            .take(3)
            .map(|d| d.join(&case))
            .find(|p| p.is_file())
    }?;
    Some((found, sets))
}

/// How to ask FoehnCraft for the fields of the result at `result`.
pub fn plan(result: &Path) -> Result<Plan, Missing> {
    let step = step_of(result).ok_or_else(|| Missing::NotFoehnCraft(result.to_path_buf()))?;
    let dir = result
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let restart = dir.join(format!("restart{step}.h5"));
    if !restart.is_file() {
        return Err(Missing::Restart(restart));
    }
    let (case, sets) = match recorded_case(&dir) {
        Some(c) => c,
        None => {
            let resolved = dir.join("case.resolved.toml");
            if !resolved.is_file() {
                return Err(Missing::Case(dir));
            }
            (resolved, Vec::new())
        }
    };
    Ok(Plan {
        case,
        sets,
        restart,
        file: dir.join(format!("derived{step}.vtkhdf")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the step of a result's name is found for FoehnCraft's
    /// fields and derived files only.
    #[test]
    fn steps_of_result_names() {
        for (name, step) in [
            ("fields.vtkhdf", Some("")),
            ("fields_12.vtkhdf", Some("_12")),
            ("derived_3.vtkhdf", Some("_3")),
            ("fields_x.vtkhdf", None),
            ("fields_.vtkhdf", None),
            ("box.vtkhdf", None),
            ("fields.vtu", None),
        ] {
            assert_eq!(step_of(Path::new(name)).as_deref(), step, "{name}");
        }
    }

    /// Invariant: the plan takes the restart file of the result's step,
    /// the case the run's record names (found above the output folder)
    /// with its overrides, else case.resolved.toml, and writes the
    /// derived file of the same step; what is missing is said. Breaks
    /// when a relative case file is looked for in the wrong folder.
    #[test]
    fn plan_of_a_run() {
        let root = std::env::temp_dir().join(format!("murmel-foehn-{}", std::process::id()));
        let out = root.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let result = out.join("fields_7.vtkhdf");
        assert_eq!(
            plan(&result),
            Err(Missing::Restart(out.join("restart_7.h5")))
        );
        std::fs::write(out.join("restart_7.h5"), b"").unwrap();
        assert_eq!(plan(&result), Err(Missing::Case(out.clone())));
        std::fs::write(out.join("case.resolved.toml"), b"").unwrap();
        let p = plan(&result).unwrap();
        assert_eq!(p.case, out.join("case.resolved.toml"));
        assert_eq!(p.file, out.join("derived_7.vtkhdf"));
        std::fs::write(root.join("pump.toml"), b"").unwrap();
        std::fs::write(
            out.join("run.jsonl"),
            "{\"type\":\"header\",\"case_file\":\"pump.toml\",\"overrides\":[\"run.threads=4\"]}\n{\"type\":\"row\"}\n",
        )
        .unwrap();
        let p = plan(&result).unwrap();
        assert_eq!(p.case, root.join("pump.toml"));
        assert_eq!(p.sets, ["run.threads=4"]);
        let keep = kept(["velocity", "q_criterion", "y_plus"]);
        assert_eq!(keep, ["q-criterion", "y-plus"]);
        let args = p.args(&[Model::YPlus, Model::WallShearStress], &keep, &p.file);
        assert_eq!(&args[..2], ["post", "derive"]);
        assert!(
            args.windows(2)
                .any(|w| w == ["--fields", "q-criterion,y-plus,wall-shear-stress"])
        );
        // every model is among FoehnCraft's fields, with its first field
        for m in Model::ALL {
            assert!(
                FOEHN_FIELDS.contains(&(m.key(), m.fields()[0])),
                "{}",
                m.key()
            );
        }
        assert!(args.windows(2).any(|w| w == ["--set", "run.threads=4"]));
        assert_eq!(
            plan(&out.join("box.vtkhdf")).map_err(|e| matches!(e, Missing::NotFoehnCraft(_))),
            Err(true)
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
