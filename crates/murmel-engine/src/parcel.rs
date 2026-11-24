//! Spray parcels (FoehnCraft SP1, viewed as decided in D268, first
//! milestone): the Lagrangian parcels of one parcel file shown with the
//! mesh and its fields.
//!
//! - **What.** The points of a `spray/parcels_<step>.vtkhdf` file
//!   ([`murmel_mesh::parcels`]), each drawn as a small shaded sphere (an
//!   icosahedron with the sphere's normals at its corners, so it shades
//!   round at twenty triangles), lit like the mesh. They go into the
//!   mesh's [`crate::surface::Surface`], so pictures and movies draw them
//!   as the window does.
//! - **Size.** [`Size::Fixed`]: every parcel the same, a radius of
//!   1/400 of the mesh's diagonal times the scale. [`Size::Diameter`]:
//!   the radius follows the parcel's droplet diameter, the largest
//!   diameter in the file getting the fixed radius (times the scale) and
//!   the others in proportion, so the relative sizes are true while the
//!   droplets, often a thousandth of the domain, stay visible.
//! - **Colour.** A parcel field (any scalar, or a vector's magnitude or
//!   component) with a colour bar of its own, its map, direction and
//!   range kept with the parcels (a parcel's `velocity` is not the gas's:
//!   they do not share the field's style); or one solid colour
//!   (`display.set_color` on `parcels`).
//! - **Which.** The parcels kept by every clip plane that cuts, within
//!   the diameter range and of the injector asked for, when one is. At
//!   most [`MAX_DRAWN`] are drawn: more are thinned evenly by their order
//!   in the file, and the surface says so.
//! - **The mesh.** While parcels show, the mesh draws as its faint
//!   outline (the parcels are inside it) unless `display.mesh_look` asks
//!   for its faces, as with slices and streamlines.

use std::path::PathBuf;
use std::sync::Arc;

use murmel_mesh::{Bounds, Component, Mesh, NO_CELL, Parcels};
use serde::{Deserialize, Serialize};

use crate::display::{Display, Item};
use crate::field::{FieldChoice, RangeMode};
use crate::surface::Surface;
use crate::theme::{Rgb, Tokens};

/// The most parcels drawn; more are thinned evenly (the surface says so).
pub const MAX_DRAWN: usize = 200_000;
/// The largest scale factor.
pub const MAX_SCALE: f64 = 100.0;
/// The fixed radius, as a share of the mesh's diagonal.
pub const RADIUS: f64 = 1.0 / 400.0;
/// The field parcels are coloured by when they open: the droplet
/// diameter, else the first field of the file.
pub const DEFAULT_FIELD: &str = "diameter";
/// The field the diameter size and filter read.
pub const DIAMETER: &str = "diameter";
/// The field the injector filter reads.
pub const INJECTOR: &str = "injector";

fn one() -> f64 {
    1.0
}

fn yes() -> bool {
    true
}

/// How large a parcel is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Size {
    /// Every parcel the same.
    #[default]
    Fixed,
    /// In proportion to the droplet diameter, the largest the fixed size.
    Diameter,
}

impl Size {
    pub const ALL: [Size; 2] = [Size::Fixed, Size::Diameter];

    pub fn key(self) -> &'static str {
        match self {
            Size::Fixed => "fixed",
            Size::Diameter => "diameter",
        }
    }
}

/// The parcels shown (`parcels.open`, `parcels.set`): the file and how
/// it is drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParcelView {
    /// The parcel file.
    pub path: PathBuf,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The field they are coloured by; `None`: solid.
    #[serde(default)]
    pub color: Option<FieldChoice>,
    #[serde(default)]
    pub size: Size,
    #[serde(default = "one")]
    pub scale: f64,
    /// Only parcels whose diameter lies in [min, max] (metres).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diameter: Option<[f64; 2]>,
    /// Only parcels of this injector (the file's `injector` field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injector: Option<u32>,
    /// The colour map (`colormap::ids`); `None` as the field colouring
    /// chooses (rainbow desaturated, rainbow uniform about zero).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map: Option<String>,
    #[serde(default)]
    pub reverse: bool,
    /// Field: all parcels of the file; shown: the parcels drawn;
    /// symmetric; fixed.
    #[serde(default)]
    pub range: RangeMode,
}

impl ParcelView {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            enabled: true,
            color: None,
            size: Size::Fixed,
            scale: 1.0,
            diameter: None,
            injector: None,
            map: None,
            reverse: false,
            range: RangeMode::Field,
        }
    }

    /// The colouring a file opens with: its diameter, else its first
    /// field, else solid.
    pub fn default_color(p: &Parcels) -> Option<FieldChoice> {
        p.field(DEFAULT_FIELD)
            .or_else(|| p.fields.first())
            .map(|f| FieldChoice {
                name: f.name.clone(),
                component: Component::Magnitude,
            })
    }

    /// The fixed radius on `mesh`, times the scale.
    pub fn radius(&self, mesh: &Mesh) -> f64 {
        RADIUS * 2.0 * mesh.bounds().radius() * self.scale
    }

    /// The style the colour bar and the colours take.
    pub fn style(&self) -> crate::field::FieldStyle {
        crate::field::FieldStyle {
            map: self.map.clone(),
            reverse: self.reverse,
            range: self.range,
            ..Default::default()
        }
    }
}

/// The colour value of parcel `i`.
fn value(p: &Parcels, choice: &FieldChoice, i: usize) -> f64 {
    p.field(&choice.name)
        .map_or(f64::NAN, |f| f.scalar(i, choice.component))
}

/// Which parcels pass the filters (diameter range, injector) and every
/// clip plane that cuts, in file order.
pub fn kept(v: &ParcelView, p: &Parcels, display: &Display) -> Vec<usize> {
    let planes: Vec<_> = display.active_clips().copied().collect();
    let d = v.diameter.and_then(|_| p.field(DIAMETER));
    let inj = v.injector.and_then(|_| p.field(INJECTOR));
    (0..p.len())
        .filter(|&i| {
            if let (Some([lo, hi]), Some(f)) = (v.diameter, d) {
                let x = f.scalar(i, Component::Magnitude);
                if !(x >= lo && x <= hi) {
                    return false;
                }
            }
            if let (Some(want), Some(f)) = (v.injector, inj)
                && f.scalar(i, Component::Magnitude) != want as f64
            {
                return false;
            }
            planes.iter().all(|c| c.keeps(p.points[i]))
        })
        .collect()
}

/// The smallest and largest finite value of `choice` over `which`.
fn range_of(
    p: &Parcels,
    choice: &FieldChoice,
    which: impl Iterator<Item = usize>,
) -> Option<(f64, f64)> {
    let mut r: Option<(f64, f64)> = None;
    for i in which {
        let x = value(p, choice, i);
        if x.is_finite() {
            r = Some(r.map_or((x, x), |(a, b)| (a.min(x), b.max(x))));
        }
    }
    r
}

/// The colour range of the parcels' field, `None` when solid or when no
/// parcel has a value. `shown` is the range of the parcels drawn.
pub fn color_range(display: &Display, shown: Option<(f64, f64)>) -> Option<(f64, f64)> {
    let v = display.parcels.as_ref()?;
    let choice = v.color.as_ref()?;
    let p = display.parcel_data.as_deref()?;
    let whole = || range_of(p, choice, 0..p.len());
    let r = match v.range {
        // Parcels have no range over the steps; the step's whole range.
        RangeMode::Field | RangeMode::Steps => whole()?,
        RangeMode::Shown => shown?,
        RangeMode::Symmetric => {
            let (a, b) = whole()?;
            let m = a.abs().max(b.abs());
            (-m, m)
        }
        RangeMode::Fixed { min, max } => (min, max),
    };
    Some(crate::field::widen(r))
}

/// The solid colour of the parcels.
pub fn solid_color(display: &Display, tokens: &Tokens) -> Rgb {
    display
        .colors
        .get(&Item::Parcels)
        .copied()
        .unwrap_or(tokens.mesh.field.parcel)
}

/// The corners and triangles of an icosahedron about the origin, its
/// corners on the unit sphere.
fn icosahedron() -> ([[f64; 3]; 12], [[u32; 3]; 20]) {
    let t = (1.0 + 5f64.sqrt()) / 2.0;
    let raw = [
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ];
    let n = (1.0 + t * t).sqrt();
    let v = raw.map(|p: [f64; 3]| p.map(|x| x / n));
    let f = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    (v, f)
}

/// Draw the parcels of `display` into `s` as shaded spheres.
pub fn append(s: &mut Surface, mesh: &Mesh, tokens: &Tokens, display: &Display) {
    let (Some(v), Some(p)) = (display.parcels.as_ref(), display.parcel_data.as_deref()) else {
        return;
    };
    if !v.enabled || p.is_empty() {
        return;
    }
    let kept = kept(v, p, display);
    let stride = kept.len().div_ceil(MAX_DRAWN).max(1);
    let drawn: Vec<usize> = kept.iter().copied().step_by(stride).collect();
    s.parcels = drawn.len();
    s.parcels_capped = stride > 1;
    let shown = v
        .color
        .as_ref()
        .and_then(|c| range_of(p, c, drawn.iter().copied()));
    s.parcel_range = shown;
    let picked = display.selection.contains(&Item::Parcels);
    let sel = tokens.mesh.selection;
    let mix = tokens.mesh.selection_mix.clamp(0.0, 1.0);
    let mixed = |c: Rgb| -> Rgb {
        if picked {
            Rgb(std::array::from_fn(|k| {
                (c.0[k] as f32 * (1.0 - mix) + sel.0[k] as f32 * mix).round() as u8
            }))
        } else {
            c
        }
    };
    let solid = mixed(solid_color(display, tokens)).linear();
    let none = mixed(tokens.mesh.field.none).linear();
    let paint = v.color.as_ref().and_then(|choice| {
        let (lo, hi) = color_range(display, shown)?;
        let map = crate::field::map_for(&v.style(), (lo, hi))?;
        Some((choice, map, lo, hi))
    });
    let base = v.radius(mesh);
    let dmax = p
        .field(DIAMETER)
        .and_then(|f| {
            f.values
                .iter()
                .copied()
                .filter(|x| x.is_finite() && *x > 0.0)
                .reduce(f64::max)
        })
        .filter(|_| v.size == Size::Diameter);
    let (corners, tris) = icosahedron();
    let o = s.origin;
    let mut vb = s.visible_bounds.unwrap_or_else(Bounds::empty);
    s.positions.reserve(12 * drawn.len());
    s.indices.reserve(60 * drawn.len());
    for &i in &drawn {
        let c = match &paint {
            Some((choice, map, lo, hi)) => {
                let x = value(p, choice, i);
                if x.is_finite() {
                    mixed(map.sample((x - lo) / (hi - lo), v.reverse)).linear()
                } else {
                    none
                }
            }
            None => solid,
        };
        let r = match dmax {
            Some(dm) => {
                let d = p
                    .field(DIAMETER)
                    .map_or(f64::NAN, |f| f.scalar(i, Component::Magnitude));
                if d.is_finite() && d > 0.0 {
                    base * d / dm
                } else {
                    base
                }
            }
            None => base,
        };
        let centre = p.points[i];
        vb.add(centre);
        let start = s.positions.len() as u32;
        for n in &corners {
            let q = [0, 1, 2].map(|k| centre[k] + r * n[k] - o[k]);
            s.positions.push(q.map(|x| x as f32));
            s.normals.push(n.map(|x| x as f32));
            s.colors.push(c);
            s.values.push([0.0; 4]);
        }
        for t in &tris {
            s.indices.extend(t.map(|k| start + k));
            s.tri_cell.push(NO_CELL);
            s.tri_face.push(NO_CELL);
        }
    }
    if !vb.is_empty() {
        s.visible_bounds = Some(vb);
    }
}

/// What the Properties panel and `parcels.set` say of the parcels: the
/// file, its time and step, the counts and the range of each field.
pub fn summary(display: &Display, surface: &Surface) -> Option<serde_json::Value> {
    let v = display.parcels.as_ref()?;
    let p = display.parcel_data.as_deref()?;
    let fields: Vec<serde_json::Value> = p
        .fields
        .iter()
        .map(|f| {
            let choice = FieldChoice {
                name: f.name.clone(),
                component: Component::Magnitude,
            };
            let r = range_of(p, &choice, 0..p.len());
            serde_json::json!({
                "name": f.name,
                "components": f.components,
                "unit": f.unit,
                "min": r.map(|r| r.0),
                "max": r.map(|r| r.1),
            })
        })
        .collect();
    Some(serde_json::json!({
        "id": "parcels",
        "path": v.path,
        "blake3": p.blake3,
        "format": p.attrs.get("format"),
        "case": p.attrs.get("case"),
        "liquid": p.attrs.get("liquid"),
        "time": p.time,
        "step": p.step,
        "parcels": p.len(),
        "drawn": surface.parcels,
        "capped": surface.parcels_capped,
        "enabled": v.enabled,
        "color": v.color,
        "size": v.size.key(),
        "scale": v.scale,
        "diameter": v.diameter,
        "injector": v.injector,
        "map": v.map,
        "reverse": v.reverse,
        "range": v.range,
        "color_range": color_range(display, surface.parcel_range),
        "fields": fields,
    }))
}

/// Shares the parcels read once between every copy of the display.
pub fn shared(p: Parcels) -> Arc<Parcels> {
    Arc::new(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use murmel_mesh::{Field, Location};

    fn parcels() -> Parcels {
        let points = vec![[0.1, 0.1, 0.1], [0.5, 0.5, 0.5], [0.9, 0.9, 0.9]];
        let f = |name: &str, values: Vec<f64>| Field {
            name: name.into(),
            unit: None,
            location: Location::Point,
            components: 1,
            values,
        };
        Parcels {
            points,
            fields: vec![
                f("diameter", vec![1e-4, 2e-4, 4e-4]),
                f("injector", vec![0.0, 1.0, 1.0]),
                Field {
                    name: "velocity".into(),
                    unit: None,
                    location: Location::Point,
                    components: 3,
                    values: vec![3.0, 4.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0],
                },
            ],
            ..Default::default()
        }
    }

    /// Invariant: an icosahedron's twenty triangles all face out (their
    /// winding gives an outward normal) and its corners lie on the unit
    /// sphere; a wrong triangle shows as a hole under back-face culling
    /// or a dark facet. No other test looks at the glyph's geometry.
    #[test]
    fn icosahedron_faces_out() {
        let (v, f) = icosahedron();
        for c in &v {
            assert!((c.iter().map(|x| x * x).sum::<f64>() - 1.0).abs() < 1e-12);
        }
        for t in &f {
            let [a, b, c] = t.map(|k| v[k as usize]);
            let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let mid = [0, 1, 2].map(|k| a[k] + b[k] + c[k]);
            assert!(n[0] * mid[0] + n[1] * mid[1] + n[2] * mid[2] > 0.0, "{t:?}");
        }
    }

    /// Invariant: the filters keep the parcels in the diameter range and
    /// of the injector asked for, and a clip plane drops those on its far
    /// side; the colour range follows the range mode (the whole file, the
    /// parcels drawn, symmetric, fixed). Breaking a filter shows parcels
    /// the user excluded.
    #[test]
    fn filters_and_ranges() {
        let p = parcels();
        let mut d = Display::default();
        let mut v = ParcelView::new("p.vtkhdf".into());
        v.color = Some(FieldChoice {
            name: "velocity".into(),
            component: Component::Magnitude,
        });
        assert_eq!(kept(&v, &p, &d), vec![0, 1, 2]);
        v.diameter = Some([1.5e-4, 1.0]);
        assert_eq!(kept(&v, &p, &d), vec![1, 2]);
        v.injector = Some(0);
        assert!(kept(&v, &p, &d).is_empty());
        v.injector = Some(1);
        v.diameter = None;
        assert_eq!(kept(&v, &p, &d), vec![1, 2]);
        d.clips.push(crate::clip::Clip {
            origin: [0.7, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
            mode: crate::clip::ClipMode::Crinkle,
            enabled: true,
        });
        assert_eq!(kept(&v, &p, &d), vec![1]);
        d.parcels = Some(v.clone());
        d.parcel_data = Some(Arc::new(p.clone()));
        assert_eq!(color_range(&d, Some((1.0, 1.0))), Some((1.0, 5.0)));
        d.parcels.as_mut().unwrap().range = RangeMode::Shown;
        assert_eq!(color_range(&d, Some((1.0, 2.0))), Some((1.0, 2.0)));
        d.parcels.as_mut().unwrap().range = RangeMode::Symmetric;
        assert_eq!(color_range(&d, None), Some((-5.0, 5.0)));
        d.parcels.as_mut().unwrap().range = RangeMode::Fixed { min: 2.0, max: 3.0 };
        assert_eq!(color_range(&d, None), Some((2.0, 3.0)));
        d.parcels.as_mut().unwrap().color = None;
        assert_eq!(color_range(&d, None), None);
    }
}
