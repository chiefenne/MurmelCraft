//! Field results: the values a solver wrote on the cells, the points or
//! the boundary faces of a mesh (FoehnCraft's `fields.vtkhdf`, or the
//! data arrays of a `.vtu`).
//!
//! A field has one value per cell, point or boundary face, with one or
//! three components. FoehnCraft writes vectors as three scalar datasets
//! (`velocity_x`, `velocity_y`, `velocity_z`); [`group_vectors`] joins
//! them into one vector field (`velocity`), so the viewer can colour by
//! its magnitude.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Where a field's values sit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Location {
    /// One value per volume cell (finite-volume results).
    Cell,
    /// One value per mesh point.
    Point,
    /// One value per boundary face (`Mesh::boundary` order), such as the
    /// wall shear stress.
    Boundary,
}

impl Location {
    pub fn name(self) -> &'static str {
        match self {
            Location::Cell => "cell",
            Location::Point => "point",
            Location::Boundary => "boundary",
        }
    }
}

/// A scalar out of a field: the value of a scalar field, or the
/// magnitude or one component of a vector field.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Component {
    /// The value of a scalar field, the length of a vector.
    #[default]
    Magnitude,
    X,
    Y,
    Z,
}

impl Component {
    pub const ALL: [Component; 4] = [Component::Magnitude, Component::X, Component::Y, Component::Z];

    pub fn name(self) -> &'static str {
        match self {
            Component::Magnitude => "magnitude",
            Component::X => "x",
            Component::Y => "y",
            Component::Z => "z",
        }
    }

    pub fn parse(s: &str) -> Option<Component> {
        Component::ALL.into_iter().find(|c| c.name() == s)
    }

    fn index(self) -> Option<usize> {
        match self {
            Component::Magnitude => None,
            Component::X => Some(0),
            Component::Y => Some(1),
            Component::Z => Some(2),
        }
    }
}

/// One field.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    /// The unit as the file names it (`m/s`, `Pa`), when it does.
    pub unit: Option<String>,
    pub location: Location,
    /// 1 for a scalar, 3 for a vector.
    pub components: usize,
    /// `components` values per entity, entity after entity.
    pub values: Vec<f64>,
}

impl Field {
    /// The number of cells, points or faces the field has values for.
    pub fn len(&self) -> usize {
        self.values.len() / self.components.max(1)
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn is_vector(&self) -> bool {
        self.components == 3
    }

    /// The values of entity `i` (one or three).
    pub fn value(&self, i: usize) -> Option<&[f64]> {
        let k = self.components.max(1);
        self.values.get(i * k..i * k + k)
    }

    /// Scalar `c` of entity `i`: the value of a scalar field (any
    /// component), the magnitude or one component of a vector. NaN when
    /// `i` is out of range.
    pub fn scalar(&self, i: usize, c: Component) -> f64 {
        let Some(v) = self.value(i) else {
            return f64::NAN;
        };
        if v.len() == 1 {
            return v[0];
        }
        match c.index() {
            Some(k) => v.get(k).copied().unwrap_or(f64::NAN),
            None => v.iter().map(|x| x * x).sum::<f64>().sqrt(),
        }
    }

    /// Scalar `c` of every entity.
    pub fn scalars(&self, c: Component) -> Vec<f64> {
        (0..self.len()).map(|i| self.scalar(i, c)).collect()
    }

    /// The smallest and largest finite value of scalar `c`, or `None`
    /// when there is none.
    pub fn range(&self, c: Component) -> Option<(f64, f64)> {
        range_of((0..self.len()).map(|i| self.scalar(i, c)))
    }
}

/// The smallest and largest finite value, or `None`.
pub fn range_of(values: impl IntoIterator<Item = f64>) -> Option<(f64, f64)> {
    let mut r: Option<(f64, f64)> = None;
    for v in values.into_iter().filter(|v| v.is_finite()) {
        r = Some(match r {
            None => (v, v),
            Some((a, b)) => (a.min(v), b.max(v)),
        });
    }
    r
}

/// The fields of a result file and what it says about them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Results {
    pub fields: Vec<Field>,
    /// The simulated time of a transient result.
    pub time: Option<f64>,
    /// The time step of a transient result.
    pub step: Option<u64>,
    /// The outer iteration of a steady result.
    pub iteration: Option<u64>,
    /// Further text attributes of the file (`foehncraft_version`, `case`).
    pub attrs: BTreeMap<String, String>,
}

impl Results {
    pub fn get(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// FoehnCraft's `units` attribute ("velocity_x m/s; pressure Pa; …") as
/// a table from dataset name to unit.
pub fn parse_units(s: &str) -> BTreeMap<String, String> {
    s.split(';')
        .filter_map(|e| {
            let e = e.trim();
            let (n, u) = e.split_once(char::is_whitespace)?;
            Some((n.trim().to_string(), u.trim().to_string()))
        })
        .filter(|(n, u)| !n.is_empty() && !u.is_empty())
        .collect()
}

/// Join scalar fields `<stem>_x`, `<stem>_y` and `<stem>_z` (same
/// location and length) into one vector field `<stem>` at the place of
/// the first; its unit is theirs when they agree. Other fields keep their
/// order. A field already called `<stem>` keeps the components apart.
pub fn group_vectors(fields: Vec<Field>) -> Vec<Field> {
    let names: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
    let find = |n: &str, loc: Location| {
        fields
            .iter()
            .position(|f| f.name == n && f.location == loc && f.components == 1)
    };
    let mut joined: BTreeMap<usize, [usize; 3]> = BTreeMap::new();
    let mut used = vec![false; fields.len()];
    for (i, f) in fields.iter().enumerate() {
        if f.components != 1 || used[i] {
            continue;
        }
        let Some(stem) = f.name.strip_suffix("_x") else {
            continue;
        };
        if stem.is_empty() || names.iter().any(|n| n == stem) {
            continue;
        }
        let (Some(y), Some(z)) = (
            find(&format!("{stem}_y"), f.location),
            find(&format!("{stem}_z"), f.location),
        ) else {
            continue;
        };
        if fields[y].len() != f.len() || fields[z].len() != f.len() {
            continue;
        }
        used[i] = true;
        used[y] = true;
        used[z] = true;
        joined.insert(i, [i, y, z]);
    }
    let mut out = Vec::with_capacity(fields.len());
    for (i, f) in fields.iter().enumerate() {
        if let Some(&[x, y, z]) = joined.get(&i) {
            let (fx, fy, fz) = (&fields[x], &fields[y], &fields[z]);
            let mut values = Vec::with_capacity(3 * fx.len());
            for k in 0..fx.len() {
                values.extend([fx.values[k], fy.values[k], fz.values[k]]);
            }
            let unit = (fx.unit == fy.unit && fy.unit == fz.unit)
                .then(|| fx.unit.clone())
                .flatten();
            out.push(Field {
                name: fx.name.trim_end_matches("_x").to_string(),
                unit,
                location: fx.location,
                components: 3,
                values,
            });
        } else if !used[i] {
            out.push(f.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(name: &str, unit: &str, v: &[f64]) -> Field {
        Field {
            name: name.into(),
            unit: Some(unit.into()),
            location: Location::Cell,
            components: 1,
            values: v.to_vec(),
        }
    }

    /// Invariant: FoehnCraft's three velocity datasets become one vector
    /// `velocity` in m/s at the place of `velocity_x`, other fields keep
    /// their order, and the magnitude is the length of the vector. Without
    /// the join a probe would list three unrelated scalars and colour by
    /// speed could not be derived.
    #[test]
    fn joins_xyz_into_a_vector() {
        let f = group_vectors(vec![
            scalar("velocity_x", "m/s", &[3.0, 0.0]),
            scalar("velocity_y", "m/s", &[4.0, 1.0]),
            scalar("velocity_z", "m/s", &[0.0, 0.0]),
            scalar("pressure", "Pa", &[1.0, 2.0]),
        ]);
        let names: Vec<&str> = f.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["velocity", "pressure"]);
        let v = &f[0];
        assert_eq!(v.components, 3);
        assert_eq!(v.unit.as_deref(), Some("m/s"));
        assert_eq!(v.scalar(0, Component::Magnitude), 5.0);
        assert_eq!(v.scalar(1, Component::Y), 1.0);
        assert_eq!(v.range(Component::Magnitude), Some((1.0, 5.0)));
        // a scalar answers every component with its value
        assert_eq!(f[1].scalar(1, Component::Z), 2.0);
    }

    /// Invariant: a set without all three components, or whose stem is
    /// itself a field, is left alone.
    #[test]
    fn leaves_incomplete_sets_alone() {
        let f = group_vectors(vec![
            scalar("u_x", "m/s", &[1.0]),
            scalar("u_y", "m/s", &[1.0]),
            scalar("w", "1", &[1.0]),
            scalar("w_x", "1", &[1.0]),
            scalar("w_y", "1", &[1.0]),
            scalar("w_z", "1", &[1.0]),
        ]);
        assert_eq!(f.len(), 6);
    }

    /// Invariant: the units attribute splits into name and unit at the
    /// first blank, units with blanks or slashes intact.
    #[test]
    fn parses_units() {
        let u = parse_units("coordinates m; velocity_x m/s; k m2/s2;  ; y_plus 1");
        assert_eq!(u.get("velocity_x").map(String::as_str), Some("m/s"));
        assert_eq!(u.get("k").map(String::as_str), Some("m2/s2"));
        assert_eq!(u.len(), 4);
    }
}
