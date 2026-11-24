//! The frames of rotating machinery for the turbomachinery quantities of
//! [`crate::derive`] (the maintainer's card of 2026-11-24: the rotation
//! comes from FoehnCraft's case beside the result, else it is typed in
//! the Derived fields window and can be kept as your default).
//!
//! The notation is that of turbomachinery, as FoehnCraft writes it (its
//! decision D183): **c** is the absolute velocity (the one in the file),
//! **u** = **Ω** × (**x** − **x**₀) the velocity of the frame (the blade
//! speed) and **w** = **c** − **u** the relative velocity. A cell in a
//! turning zone is seen in the frame of its zone, any other cell in the
//! fixed frame (**w** = **c**, **u** = 0), so a pump shows the relative
//! flow in the impeller and the absolute flow in the volute; a `frame`
//! zone takes its frame in every cell. The cylindrical components are
//! taken about the axis of the cell's frame (outside the zones about that
//! of the first zone), which points along **Ω**: the blade speed and a
//! swirl in the direction of rotation are positive. The flow angles are
//! in degrees, measured from the circumferential direction: α between
//! **c** and **u**, β between **w** and −**u** (90° for a flow without
//! swirl relative to the blades, the convention of pump design).

use std::f64::consts::TAU;
use std::path::Path;

use murmel_mesh::Mesh;
use serde::{Deserialize, Serialize};

use crate::defaults::Kind;
use crate::derive::Source;
use crate::session::Session;

/// The most turning zones a rotation has.
pub const MAX_ZONES: usize = 16;
/// The fastest rotation accepted (rpm), a check against typing slips.
pub const MAX_RPM: f64 = 1e7;
/// The farthest axis point accepted (m).
pub const MAX_ORIGIN: f64 = 1e9;

/// One zone that turns about an axis, as FoehnCraft's `[[mrf]]` and
/// `[[sliding]]` tables give it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turning {
    /// The cell zone of the mesh that turns; left out, every cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    /// A point on the axis (m).
    pub origin: [f64; 3],
    /// The direction of the axis (any length): the rotation is
    /// right-handed about it for a positive speed.
    pub axis: [f64; 3],
    /// The speed of rotation (revolutions per minute).
    pub rpm: f64,
}

impl Default for Turning {
    fn default() -> Self {
        Turning {
            zone: None,
            origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            rpm: 0.0,
        }
    }
}

impl Turning {
    /// **Ω** (rad/s), along the axis.
    pub fn omega(&self) -> [f64; 3] {
        let n = norm(self.axis);
        let w = self.rpm * TAU / 60.0;
        self.axis.map(|a| w * a / n)
    }

    /// The unit vector of the axis along **Ω** (the given axis when the
    /// zone does not turn).
    pub fn unit_axis(&self) -> [f64; 3] {
        let o = self.omega();
        let a = if norm(o) > 0.0 { o } else { self.axis };
        let n = norm(a);
        a.map(|v| v / n)
    }

    /// The velocity of the zone at `x`, **Ω** × (**x** − **x**₀).
    pub fn at(&self, x: [f64; 3]) -> [f64; 3] {
        cross(self.omega(), sub(x, self.origin))
    }

    fn check(&self) -> Result<(), String> {
        let n = norm(self.axis);
        if !(n > 0.0 && n.is_finite()) {
            return Err(format!(
                "the axis {:?} must be a vector that is not zero",
                self.axis
            ));
        }
        if !(self.rpm.is_finite() && self.rpm.abs() <= MAX_RPM) {
            return Err(format!(
                "{} rpm must be finite and at most {MAX_RPM}",
                self.rpm
            ));
        }
        if !self
            .origin
            .iter()
            .all(|x| x.is_finite() && x.abs() <= MAX_ORIGIN)
        {
            return Err(format!("the axis point {:?} must be finite", self.origin));
        }
        if self.zone.as_deref().is_some_and(str::is_empty) {
            return Err("a zone name cannot be empty".into());
        }
        Ok(())
    }
}

/// The turning zones of a result and the frame its cells are seen in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rotation {
    /// The zones, the first holding a cell taking it.
    pub zones: Vec<Turning>,
    /// The zone whose frame every cell is seen in (FoehnCraft's
    /// `[output] frame`); left out, each cell in its own zone's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
}

impl Default for Rotation {
    /// Every cell about the z axis through the origin, not turning: the
    /// cylindrical components of a pipe or a fan's inflow.
    fn default() -> Self {
        Rotation {
            zones: vec![Turning::default()],
            frame: None,
        }
    }
}

impl Rotation {
    /// Why this rotation cannot be used, if it cannot.
    pub fn check(&self) -> Result<(), String> {
        if self.zones.is_empty() {
            return Err("a rotation needs a turning zone".into());
        }
        if self.zones.len() > MAX_ZONES {
            return Err(format!("at most {MAX_ZONES} turning zones"));
        }
        for t in &self.zones {
            t.check()?;
        }
        if let Some(f) = &self.frame
            && !self.zones.iter().any(|t| t.zone.as_ref() == Some(f))
        {
            return Err(format!("the frame {f} is not one of the turning zones"));
        }
        Ok(())
    }

    /// The zone (an index into `zones`) whose frame each cell of `mesh`
    /// is seen in, `None` for the fixed frame. Refused when a zone is
    /// not one of the mesh's.
    pub fn frames(&self, mesh: &Mesh) -> Result<Vec<Option<u8>>, String> {
        self.check()?;
        // The mesh zone index of each turning zone (`None`: every cell).
        let mut index: Vec<Option<u32>> = Vec::with_capacity(self.zones.len());
        for t in &self.zones {
            match &t.zone {
                None => index.push(None),
                Some(z) => match mesh.zones.iter().position(|n| n == z) {
                    Some(i) => index.push(Some(i as u32)),
                    None => {
                        return Err(if mesh.zones.is_empty() {
                            format!("the mesh has no cell zones, so no zone {z}")
                        } else {
                            format!(
                                "the mesh has no zone {z}; its zones are {}",
                                mesh.zones.join(", ")
                            )
                        });
                    }
                },
            }
        }
        let n = mesh.cell_count();
        if let Some(f) = &self.frame {
            let i = self
                .zones
                .iter()
                .position(|t| t.zone.as_ref() == Some(f))
                .map(|i| i as u8);
            return Ok(vec![i; n]);
        }
        Ok((0..n)
            .map(|c| {
                let z = mesh.cell_zone.get(c).copied();
                index
                    .iter()
                    .position(|i| i.is_none() || *i == z)
                    .map(|i| i as u8)
            })
            .collect())
    }
}

/// The velocities of rotating machinery at a point: **c**, **u**, **w**,
/// the cylindrical components and the flow angles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turbo {
    /// **w** = **c** − **u** (m/s).
    pub w: [f64; 3],
    /// **u** = **Ω** × (**x** − **x**₀) of the frame, 0 in the fixed one.
    pub u: [f64; 3],
    /// c_r, away from the axis.
    pub c_r: f64,
    /// c_u, positive in the direction of rotation.
    pub c_u: f64,
    /// c_a, along **Ω**.
    pub c_a: f64,
    /// c_m = (c_r² + c_a²)^½.
    pub c_m: f64,
    /// w_u = c_u − u_u.
    pub w_u: f64,
    /// α = atan2(c_m, c_u) (°).
    pub alpha: f64,
    /// β = atan2(c_m, −w_u) (°).
    pub beta: f64,
}

/// The velocities of rotating machinery of velocity `c` at `x`, seen in
/// `frame` (the fixed one for `None`), with the cylindrical components
/// about the axis of `axis`. On the axis itself the radial and
/// circumferential directions are taken as zero.
pub fn turbo(c: [f64; 3], x: [f64; 3], frame: Option<&Turning>, axis: &Turning) -> Turbo {
    let u = frame.map_or([0.0; 3], |t| t.at(x));
    let w = sub(c, u);
    let a = axis.unit_axis();
    let d = sub(x, axis.origin);
    let da = dot(d, a);
    let rv = [d[0] - da * a[0], d[1] - da * a[1], d[2] - da * a[2]];
    let r = norm(rv);
    let er = if r > 0.0 { rv.map(|v| v / r) } else { [0.0; 3] };
    let et = cross(a, er);
    let (c_r, c_u, c_a) = (dot(c, er), dot(c, et), dot(c, a));
    let c_m = c_r.hypot(c_a);
    let w_u = dot(w, et);
    Turbo {
        w,
        u,
        c_r,
        c_u,
        c_a,
        c_m,
        w_u,
        alpha: c_m.atan2(c_u).to_degrees(),
        beta: c_m.atan2(-w_u).to_degrees(),
    }
}

/// The rotation FoehnCraft used for the result at `result`, from
/// `case.resolved.toml` in the same folder: its `[[mrf]]` and
/// `[[sliding]]` tables (zone, origin, axis, `omega` in rad/s or `rpm`)
/// and `[output] frame`. `None` without a turning zone; a table that
/// cannot be read is left out.
pub fn case_rotation(result: &Path) -> Option<Rotation> {
    let file = result.parent()?.join("case.resolved.toml");
    let text = std::fs::read_to_string(file).ok()?;
    let case: toml::Table = toml::from_str(&text).ok()?;
    let number = |v: &toml::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64));
    let vector = |t: &toml::Table, key: &str, or: [f64; 3]| -> Option<[f64; 3]> {
        match t.get(key) {
            None => Some(or),
            Some(v) => {
                let a = v.as_array()?;
                if a.len() != 3 {
                    return None;
                }
                let x: Vec<f64> = a.iter().filter_map(number).collect();
                (x.len() == 3).then(|| [x[0], x[1], x[2]])
            }
        }
    };
    let mut zones = Vec::new();
    for key in ["mrf", "sliding"] {
        let Some(list) = case.get(key).and_then(toml::Value::as_array) else {
            continue;
        };
        for t in list.iter().filter_map(toml::Value::as_table) {
            let rpm = match (
                t.get("omega").and_then(number),
                t.get("rpm").and_then(number),
            ) {
                (Some(w), None) => w * 60.0 / TAU,
                (None, Some(r)) => r,
                _ => continue,
            };
            let (Some(origin), Some(axis)) = (
                vector(t, "origin", [0.0; 3]),
                vector(t, "axis", [0.0, 0.0, 1.0]),
            ) else {
                continue;
            };
            let turning = Turning {
                zone: t.get("zone").and_then(|z| z.as_str()).map(str::to_string),
                origin,
                axis,
                rpm,
            };
            if turning.check().is_ok() {
                zones.push(turning);
            }
        }
    }
    if zones.is_empty() {
        return None;
    }
    zones.truncate(MAX_ZONES);
    let frame = case
        .get("output")
        .and_then(|o| o.get("frame"))
        .and_then(|f| f.as_str())
        .filter(|f| zones.iter().any(|t| t.zone.as_deref() == Some(f)))
        .map(str::to_string);
    Some(Rotation { zones, frame })
}

/// The rotation a new turbomachinery field of `s` starts from, and where
/// it came from: FoehnCraft's case beside the result, else your default
/// speed and axis (`[defaults.rotation]`) about the origin for every
/// cell, else every cell about the z axis, not turning.
pub fn start_rotation(s: &Session) -> (Rotation, Source) {
    if let Some(r) = s.mesh.as_ref().and_then(|m| case_rotation(&m.path)) {
        return (r, Source::Case);
    }
    let t = s.defaults.apply(Kind::Rotation, Turning::default());
    let from = if s.defaults.of(Kind::Rotation).is_some() {
        Source::Defaults
    } else {
        Source::BuiltIn
    };
    (
        Rotation {
            zones: vec![t],
            frame: None,
        },
        from,
    )
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs()))
    }

    /// Invariant: the identities of turbomachinery hold at any point:
    /// **c** = **u** + **w**, c_m² = c_r² + c_a², w_u = c_u − |**u**|
    /// (the blade speed is circumferential), and a fluid turning with the
    /// rotor has **w** = 0. Breaks when the frame velocity takes the
    /// wrong sign or the circumferential direction is the other way.
    #[test]
    fn turbomachinery_identities() {
        let t = Turning {
            zone: None,
            origin: [0.1, -0.2, 0.0],
            axis: [0.0, 0.0, 2.0],
            rpm: 3000.0,
        };
        let x = [0.4, 0.3, 0.7];
        let c = [1.5, -2.0, 3.0];
        let v = turbo(c, x, Some(&t), &t);
        for k in 0..3 {
            assert!(close(v.u[k] + v.w[k], c[k]));
        }
        assert!(close(v.c_m * v.c_m, v.c_r * v.c_r + v.c_a * v.c_a));
        assert!(close(v.c_a, 3.0));
        assert!(close(v.w_u, v.c_u - norm(v.u)), "{v:?}");
        // Solid-body rotation with the rotor: no relative velocity, and
        // the absolute flow is all swirl (α = 0).
        let v = turbo(t.at(x), x, Some(&t), &t);
        assert!(norm(v.w) < 1e-12);
        assert!(v.c_u > 0.0 && v.alpha.abs() < 1e-9 && v.c_r.abs() < 1e-9);
        // Axial flow without swirl seen from a rotor: β = atan(c_a / U).
        let v = turbo([0.0, 0.0, 5.0], x, Some(&t), &t);
        let blade = norm(t.at(x));
        assert!(close(v.beta, (5.0f64).atan2(blade).to_degrees()));
        assert!(close(v.alpha, 90.0));
        // A negative speed turns the axis: the swirl along it stays
        // positive.
        let back = Turning { rpm: -3000.0, ..t };
        let v = turbo(back.at(x), x, Some(&back), &back);
        assert!(v.c_u > 0.0);
    }

    /// Invariant: each cell is seen in the frame of the first zone that
    /// holds it, the fixed frame outside them, and in the named frame
    /// everywhere when one is given; a zone the mesh does not have is
    /// refused. Breaks when zone names are matched by index.
    #[test]
    fn frames_of_the_cells() {
        let mut m = Mesh::default();
        m.zones = vec!["stator".into(), "rotor".into()];
        m.cell_zone = vec![0, 1, 1, 0];
        m.cell_faces = vec![0; 5];
        let rotor = Turning {
            zone: Some("rotor".into()),
            rpm: 100.0,
            ..Default::default()
        };
        let mut r = Rotation {
            zones: vec![rotor.clone()],
            frame: None,
        };
        assert_eq!(r.frames(&m).unwrap(), [None, Some(0), Some(0), None]);
        r.frame = Some("rotor".into());
        assert_eq!(r.frames(&m).unwrap(), [Some(0); 4]);
        r.zones[0].zone = Some("impeller".into());
        r.frame = None;
        assert!(r.frames(&m).unwrap_err().contains("stator, rotor"));
        // Every cell.
        assert_eq!(Rotation::default().frames(&m).unwrap(), [Some(0); 4]);
        // A bad axis or frame.
        let mut bad = Rotation::default();
        bad.zones[0].axis = [0.0; 3];
        assert!(bad.check().is_err());
        let bad = Rotation {
            frame: Some("rotor".into()),
            ..Default::default()
        };
        assert!(bad.check().is_err());
    }

    /// Invariant: FoehnCraft's resolved case gives its turning zones,
    /// with `omega` in rad/s taken as rpm and the output frame kept when
    /// it names one of them. Breaks when a table is read from the wrong
    /// key.
    #[test]
    fn rotation_from_the_case() {
        let dir = std::env::temp_dir().join(format!("murmel-rot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let result = dir.join("fields.vtkhdf");
        assert_eq!(case_rotation(&result), None);
        std::fs::write(
            dir.join("case.resolved.toml"),
            "[output]\nframe = \"rotor\"\n[[mrf]]\nzone = \"rotor\"\norigin = [0.0, 0.0, 0.0]\naxis = [0, 0, 1]\nomega = 104.71975511965977\n[[sliding]]\nzone = \"fan\"\norigin = [1.0, 0.0, 0.0]\naxis = [1.0, 0.0, 0.0]\nrpm = -50.0\n",
        )
        .unwrap();
        let r = case_rotation(&result).unwrap();
        assert_eq!(r.zones.len(), 2);
        assert!(close(r.zones[0].rpm, 1000.0));
        assert_eq!(r.zones[1].zone.as_deref(), Some("fan"));
        assert_eq!(r.zones[1].rpm, -50.0);
        assert_eq!(r.frame.as_deref(), Some("rotor"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
