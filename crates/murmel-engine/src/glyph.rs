//! Vector arrows (field results, stage 3; the maintainer's cards of
//! 2026-11-24): shaded 3D arrows of a vector field on the slices and, on
//! request, on the mesh surface shown.
//!
//! - **Where.** On every slice the arrows sit on an even square grid of
//!   the plane, anchored at the mesh's centre, so they do not crowd in
//!   the boundary layers of a graded mesh; the vector there is the one
//!   the slice's tetrahedra interpolate ([`crate::extract`]), the full
//!   vector or, on request, its part in the plane. On the surface the
//!   faces are thinned to one per cube of the same grid (the face nearest
//!   the cube's centre), with the face's own value, else its cell's (the
//!   value colour mode flat shows there: on a coarse outlet the node
//!   values would pull the core flow down to a third). A surface arrow
//!   sits outside the mesh, lifted off the face by its head's radius: it
//!   starts at the face when it points out (an outlet) and ends there
//!   when it points in (an inlet). On a slice the grid spacing is the
//!   largest extent of the slice's section of the mesh's box divided by
//!   the density (40 by default; the maintainer's review of 2026-11-24
//!   found 25 across the whole mesh too sparse), so a cross-section of a
//!   long duct gets as many arrows across as its length; on the surface
//!   it is the mesh's largest extent divided by the density.
//! - **Size.** The length grows with the vector's magnitude: the
//!   largest magnitude of the field is 0.9 of the grid spacing, times
//!   the scale (with the part in the plane only, the largest of the
//!   arrows drawn, the part in a plane being often a small fraction of
//!   the whole). With [`Length::Floor`] no arrow is shorter than a
//!   quarter of that, so the direction still reads where the flow is
//!   slow; with [`Length::Equal`] every arrow has that length and the
//!   magnitude shows in the colour only. Shaft and head keep their
//!   thickness whatever the length, so short arrows stay readable.
//! - **Colour.** Solid in a colour of their own, the colour of the
//!   field on the colour bar at the arrow's point, or their own
//!   magnitude in their field's style with a colour bar of their own
//!   ([`crate::legend::Legend::own_bars`]).
//! - **Look.** A shaft (a cylinder) and a head (a cone), with a cap at
//!   the tail, twelve sides each, lit like the mesh; on a slice centred
//!   on their point. They go into the mesh's [`crate::surface::Surface`], so
//!   pictures and movies draw them as the window does.

use std::collections::{HashMap, HashSet};

use murmel_mesh::Mesh;
use serde::{Deserialize, Serialize};

use crate::clip::{Clip, plane_axes};
use crate::colormap::Colormap;
use crate::display::Display;
use crate::extract::Vx;
use crate::field::{
    Component, FieldChoice, FieldMode, FieldSet, FieldValues, MAX_BANDS, MIN_BANDS, RangeMode,
};
use crate::theme::Rgb;

/// Arrows across a slice's (or on the surface the mesh's) largest
/// extent: the least, the default and the most.
pub const MIN_DENSITY: u32 = 4;
pub const DEFAULT_DENSITY: u32 = 40;
pub const MAX_DENSITY: u32 = 200;
/// The largest scale factor.
pub const MAX_SCALE: f64 = 100.0;
/// The most arrows drawn; more are left out (the surface says so).
pub const MAX_ARROWS: usize = 10_000;
/// Sides of the shaft and the head.
pub(crate) const SIDES: usize = 12;
/// The shortest arrow with [`Length::Floor`], as a share of the longest.
pub const FLOOR: f64 = 0.25;
/// The radii of the shaft and the head, in grid spacings.
const SHAFT_RADIUS: f64 = 0.05;
const HEAD_RADIUS: f64 = 0.12;

fn default_density() -> u32 {
    DEFAULT_DENSITY
}

fn is_default_density(d: &u32) -> bool {
    *d == DEFAULT_DENSITY
}

fn one() -> f64 {
    1.0
}

fn is_one(x: &f64) -> bool {
    *x == 1.0
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// How an arrow's length follows its vector's magnitude.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Length {
    /// As long as the magnitude (slow arrows shrink to dots).
    Magnitude,
    /// As long as the magnitude, but at least [`FLOOR`] of the longest.
    #[default]
    Floor,
    /// All the same length; the magnitude shows in the colour.
    Equal,
}

impl Length {
    pub const ALL: [Length; 3] = [Length::Magnitude, Length::Floor, Length::Equal];

    pub fn key(self) -> &'static str {
        match self {
            Length::Magnitude => "magnitude",
            Length::Floor => "floor",
            Length::Equal => "equal",
        }
    }

    /// The share of the longest length an arrow of magnitude `m` gets,
    /// when `longest` is drawn full length.
    pub fn share(self, m: f64, longest: f64) -> f64 {
        let t = if longest > 0.0 { m / longest } else { 0.0 };
        match self {
            Length::Magnitude => t,
            Length::Floor => t.max(FLOOR),
            Length::Equal => 1.0,
        }
    }
}

/// How the arrows are coloured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrowColor {
    /// One colour of their own.
    Solid,
    /// The field on the colour bar, in its style (when colour mode field
    /// shows one; else solid).
    Field,
    /// Their own magnitude, in their field's style, with a colour bar of
    /// their own.
    #[default]
    Magnitude,
}

impl ArrowColor {
    pub const ALL: [ArrowColor; 3] = [ArrowColor::Solid, ArrowColor::Field, ArrowColor::Magnitude];

    pub fn key(self) -> &'static str {
        match self {
            ArrowColor::Solid => "solid",
            ArrowColor::Field => "field",
            ArrowColor::Magnitude => "magnitude",
        }
    }
}

fn is_default_length(l: &Length) -> bool {
    *l == Length::default()
}

/// The arrows of one vector field (`arrows.set`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arrows {
    /// A field with three components.
    pub field: String,
    /// Off keeps the settings but draws nothing (the tree's eye).
    pub enabled: bool,
    /// Also on the mesh surface shown.
    #[serde(default, skip_serializing_if = "is_false")]
    pub surface: bool,
    /// On slices, only the part of the vector in the slice's plane.
    #[serde(default, skip_serializing_if = "is_false")]
    pub in_plane: bool,
    /// Arrows across a slice's (on the surface the mesh's) largest
    /// extent.
    #[serde(
        default = "default_density",
        skip_serializing_if = "is_default_density"
    )]
    pub density: u32,
    /// Factor on the length (1: the longest arrow is 0.9 of a spacing).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub scale: f64,
    /// How the length follows the magnitude (format 1.10).
    #[serde(default, skip_serializing_if = "is_default_length")]
    pub length: Length,
    #[serde(default)]
    pub color: ArrowColor,
}

impl Arrows {
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            enabled: true,
            surface: false,
            in_plane: false,
            density: DEFAULT_DENSITY,
            scale: 1.0,
            length: Length::default(),
            color: ArrowColor::default(),
        }
    }

    /// The distance between neighbouring arrows on the surface of `mesh`.
    pub fn spacing(&self, mesh: &Mesh) -> f64 {
        let b = mesh.bounds();
        let extent = (0..3).map(|k| b.max[k] - b.min[k]).fold(0.0_f64, f64::max);
        extent / self.density() as f64
    }

    /// The distance between neighbouring arrows on `plane` in `mesh`:
    /// the largest extent of the plane's section of the mesh's box, along
    /// the plane's own axes, divided by the density (the mesh's spacing
    /// when the plane misses the box).
    pub fn plane_spacing(&self, mesh: &Mesh, plane: &Clip) -> f64 {
        let section = plane.box_section(&mesh.bounds());
        let (u, v) = plane_axes(plane.normal);
        let extent = [u, v]
            .iter()
            .map(|ax| {
                let d: Vec<f64> = section.iter().map(|p| dot(*p, *ax)).collect();
                let lo = d.iter().copied().fold(f64::INFINITY, f64::min);
                let hi = d.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                hi - lo
            })
            .fold(0.0_f64, f64::max);
        if extent.is_finite() && extent > 0.0 {
            extent / self.density() as f64
        } else {
            self.spacing(mesh)
        }
    }

    fn density(&self) -> u32 {
        self.density.clamp(MIN_DENSITY, MAX_DENSITY)
    }

    /// The magnitude of the arrows' field.
    pub fn magnitude(&self) -> FieldChoice {
        FieldChoice {
            name: self.field.clone(),
            component: Component::Magnitude,
        }
    }
}

/// The colour range of arrows coloured by their own magnitude, in their
/// field's style: the whole field's magnitude (range mode field and
/// symmetric), or the arrows drawn (`drawn`, range mode shown, and
/// always with the part in the plane only, which the field's range does
/// not describe), or the style's fixed range. `None` when the arrows are
/// not coloured by their magnitude or nothing gives a range.
pub fn color_range(
    display: &Display,
    mesh: &Mesh,
    fields: Option<&FieldSet>,
    drawn: Option<(f64, f64)>,
) -> Option<(f64, f64)> {
    let a = display.arrows.as_ref().filter(|a| a.enabled)?;
    if a.color != ArrowColor::Magnitude {
        return None;
    }
    let style = display.style_of(&a.field);
    let whole = if a.in_plane {
        drawn
    } else {
        fields
            .and_then(|f| f.values_cached(mesh, &a.magnitude()))
            .and_then(|v| v.range)
            .or(drawn)
    };
    let r = match style.range {
        // Over the steps of a series the arrows keep the step's range
        // for now: the steps' ranges are kept for the mesh colouring.
        RangeMode::Field | RangeMode::Steps => whole?,
        RangeMode::Shown => drawn?,
        RangeMode::Symmetric => {
            let (lo, hi) = whole?;
            let m = lo.abs().max(hi.abs());
            (-m, m)
        }
        RangeMode::Fixed { min, max } => (min, max),
    };
    Some(crate::field::widen(r))
}

/// How arrows coloured by their magnitude get their colour: the map of
/// their field's style over the range, banded in colour mode banded.
pub(crate) struct OwnColors {
    map: &'static Colormap,
    reverse: bool,
    lo: f64,
    hi: f64,
    bands: Option<u32>,
}

impl OwnColors {
    pub(crate) fn new(display: &Display, a: &Arrows, (lo, hi): (f64, f64)) -> Option<Self> {
        let style = display.style_of(&a.field);
        let map = crate::field::map_for(&style, (lo, hi))?;
        Some(Self {
            map,
            reverse: style.reverse,
            lo,
            hi,
            bands: (style.mode == FieldMode::Banded)
                .then(|| style.bands.clamp(MIN_BANDS, MAX_BANDS)),
        })
    }

    /// The colour of magnitude `m`.
    pub(crate) fn of(&self, m: f64) -> Rgb {
        let mut t = ((m - self.lo) / (self.hi - self.lo)).clamp(0.0, 1.0);
        if let Some(b) = self.bands.filter(|&b| b > 0) {
            let b = b as f64;
            t = ((t * b).floor().min(b - 1.0) + 0.5) / b;
        }
        self.map.sample(t, self.reverse)
    }
}

/// Where an arrow goes: its point, its vector, the value of the colour
/// field there (NaN when none), its cell, on the surface the face's
/// outward normal, and the spacing of its grid.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sample {
    pub(crate) h: f64,
    pub(crate) p: [f64; 3],
    pub(crate) w: [f64; 3],
    pub(crate) v: f64,
    pub(crate) cell: u32,
    pub(crate) out: Option<[f64; 3]>,
}

impl Sample {
    /// The centre of an arrow `len` long along the unit `dir` for a grid
    /// of spacing `h`: the point itself on a slice; on the surface off the
    /// face by the head's radius, starting at it when the arrow points
    /// out and ending at it when it points in.
    pub(crate) fn centre(&self, dir: [f64; 3], len: f64, h: f64) -> [f64; 3] {
        match self.out {
            None => self.p,
            Some(n) => {
                let lifted = add_scaled(self.p, n, HEAD_RADIUS * h);
                let along = if dot(dir, n) >= 0.0 { 0.5 } else { -0.5 };
                add_scaled(lifted, dir, along * len)
            }
        }
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add_scaled(a: [f64; 3], b: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] + k * b[0], a[1] + k * b[1], a[2] + k * b[2]]
}

/// The arrows of one plane of a slice: the points of the even grid of
/// spacing `h` (anchored at the projection of `anchor`) that lie in the
/// polygons `polys` (the slice's pieces, with their cells, already cut
/// by the clip planes), with the vector and colour value there by
/// linear interpolation in the piece. `in_plane` keeps only the part of
/// the vector in the plane.
pub(crate) fn on_plane(
    out: &mut Vec<Sample>,
    plane: &Clip,
    anchor: [f64; 3],
    h: f64,
    in_plane: bool,
    polys: &[(u32, Vec<Vx>)],
) {
    if !(h > 0.0 && h.is_finite()) {
        return;
    }
    let n = plane.normal;
    let (u, v) = plane_axes(n);
    let a = add_scaled(anchor, n, -plane.distance(anchor));
    let st = |p: [f64; 3]| {
        let d = sub(p, a);
        [dot(d, u) / h, dot(d, v) / h]
    };
    let mut seen: HashSet<(i64, i64)> = HashSet::new();
    for (cell, poly) in polys {
        if poly.len() < 3 {
            continue;
        }
        // A fan of the convex piece.
        for k in 1..poly.len() - 1 {
            let tri = [poly[0], poly[k], poly[k + 1]];
            let q = tri.map(|x| st(x.p));
            let det = (q[1][0] - q[0][0]) * (q[2][1] - q[0][1])
                - (q[2][0] - q[0][0]) * (q[1][1] - q[0][1]);
            if det.abs() < 1e-12 {
                continue;
            }
            let lo = [0, 1].map(|d| q.iter().map(|p| p[d]).fold(f64::INFINITY, f64::min));
            let hi = [0, 1].map(|d| q.iter().map(|p| p[d]).fold(f64::NEG_INFINITY, f64::max));
            for i in (lo[0].ceil() as i64)..=(hi[0].floor() as i64) {
                for j in (lo[1].ceil() as i64)..=(hi[1].floor() as i64) {
                    let (x, y) = (i as f64, j as f64);
                    let l1 = ((x - q[0][0]) * (q[2][1] - q[0][1])
                        - (q[2][0] - q[0][0]) * (y - q[0][1]))
                        / det;
                    let l2 = ((q[1][0] - q[0][0]) * (y - q[0][1])
                        - (x - q[0][0]) * (q[1][1] - q[0][1]))
                        / det;
                    let eps = -1e-9;
                    if l1 < eps || l2 < eps || l1 + l2 > 1.0 - eps || !seen.insert((i, j)) {
                        continue;
                    }
                    let l0 = 1.0 - l1 - l2;
                    let mix = |f: &dyn Fn(&Vx) -> f64| {
                        l0 * f(&tri[0]) + l1 * f(&tri[1]) + l2 * f(&tri[2])
                    };
                    let mut w = [0, 1, 2].map(|d| mix(&|x: &Vx| x.w[d]));
                    if in_plane {
                        w = add_scaled(w, n, -dot(w, n));
                    }
                    let p = add_scaled(add_scaled(a, u, x * h), v, y * h);
                    out.push(Sample {
                        h,
                        p,
                        w,
                        v: mix(&|x: &Vx| x.v),
                        cell: *cell,
                        out: None,
                    });
                }
            }
        }
    }
}

/// The value of `f` on boundary face `b` of cell `cell`: the face's own
/// (a field of the boundary), else the cell's.
fn face_value(f: &FieldValues, b: usize, cell: u32) -> f64 {
    let own = f.boundary.get(b).copied().filter(|v| v.is_finite());
    own.or_else(|| f.cell.get(cell as usize).copied())
        .unwrap_or(f64::NAN)
}

/// The outward unit normal of boundary face `b` (Newell's normal, turned
/// away from its cell's centroid).
fn outward(mesh: &Mesh, b: usize, centre: [f64; 3], cell: u32) -> Option<[f64; 3]> {
    let nodes = mesh.boundary.get(b);
    let mut n = [0.0; 3];
    for i in 0..nodes.len() {
        let a = mesh.points[nodes[i] as usize];
        let c = mesh.points[nodes[(i + 1) % nodes.len()] as usize];
        n[0] += (a[1] - c[1]) * (a[2] + c[2]);
        n[1] += (a[2] - c[2]) * (a[0] + c[0]);
        n[2] += (a[0] - c[0]) * (a[1] + c[1]);
    }
    let l = dot(n, n).sqrt();
    if l.is_nan() || l <= 0.0 || cell as usize >= mesh.cell_count() {
        return None;
    }
    let n = n.map(|x| x / l);
    let inside = crate::inspect::cell_centroid(mesh, cell as usize);
    Some(if dot(n, sub(centre, inside)) < 0.0 {
        n.map(|x| -x)
    } else {
        n
    })
}

/// The arrows of the mesh surface shown: one boundary face per cube of
/// the grid of spacing `h` (the one whose centre is nearest the cube's
/// centre), with the face's vector and colour value.
pub(crate) fn on_surface(
    out: &mut Vec<Sample>,
    mesh: &Mesh,
    display: &Display,
    shown: &[bool],
    h: f64,
    vector: [&FieldValues; 3],
    colour: Option<&FieldValues>,
) {
    if !(h > 0.0 && h.is_finite()) {
        return;
    }
    let origin = mesh.bounds().min;
    // Per cube: the face nearest its centre, and that distance.
    let mut best: HashMap<[i64; 3], (f64, usize, [f64; 3])> = HashMap::new();
    for b in 0..mesh.boundary.len() {
        let owner = mesh.boundary_owner.get(b).copied().unwrap_or(u32::MAX);
        if !shown.get(owner as usize).copied().unwrap_or(false) || !display.face_visible(mesh, b) {
            continue;
        }
        let nodes = mesh.boundary.get(b);
        if nodes.len() < 3 {
            continue;
        }
        let k = nodes.len() as f64;
        let c = [0, 1, 2].map(|d| {
            nodes
                .iter()
                .map(|&i| mesh.points[i as usize][d])
                .sum::<f64>()
                / k
        });
        if !display.clips_keep(c) || !display.region_keeps(owner as usize, c) {
            continue;
        }
        let key = [0, 1, 2].map(|d| ((c[d] - origin[d]) / h).floor() as i64);
        let centre = [0, 1, 2].map(|d| origin[d] + (key[d] as f64 + 0.5) * h);
        let dist = dot(sub(c, centre), sub(c, centre));
        let e = best.entry(key).or_insert((f64::INFINITY, b, c));
        if dist < e.0 {
            *e = (dist, b, c);
        }
    }
    let mut picked: Vec<(usize, [f64; 3])> = best.into_values().map(|(_, b, c)| (b, c)).collect();
    picked.sort_unstable_by_key(|&(b, _)| b);
    for (b, c) in picked {
        let cell = mesh.boundary_owner.get(b).copied().unwrap_or(u32::MAX);
        let Some(n) = outward(mesh, b, c, cell) else {
            continue;
        };
        out.push(Sample {
            h,
            p: c,
            w: vector.map(|f| face_value(f, b, cell)),
            v: colour.map_or(f64::NAN, |f| face_value(f, b, cell)),
            cell,
            out: Some(n),
        });
    }
}

/// Triangles: points with their normals, and the triangles by index.
pub(crate) type Shape = (Vec<([f64; 3], [f64; 3])>, Vec<[u32; 3]>);

/// The triangles of an arrow (positions and normals, triangles by
/// index), centred on `at`, pointing along the unit `dir`, `len` long,
/// for a grid of spacing `h`.
pub(crate) fn arrow(at: [f64; 3], dir: [f64; 3], len: f64, h: f64) -> Shape {
    let shaft_r = SHAFT_RADIUS * h;
    let head_r = HEAD_RADIUS * h;
    let head = (0.3 * h).min(0.45 * len).max(0.0);
    let tail = add_scaled(at, dir, -0.5 * len);
    let neck = add_scaled(tail, dir, len - head);
    let tip = add_scaled(tail, dir, len);
    let (u, v) = plane_axes(dir);
    let ring: Vec<[f64; 3]> = (0..SIDES)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / SIDES as f64;
            add_scaled(u.map(|x| x * a.cos()), v, a.sin())
        })
        .collect();
    let mut verts: Vec<([f64; 3], [f64; 3])> = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    let back = dir.map(|x| -x);
    // The tail's cap.
    let cap = verts.len() as u32;
    verts.push((tail, back));
    for r in &ring {
        verts.push((add_scaled(tail, *r, shaft_r), back));
    }
    for i in 0..SIDES as u32 {
        tris.push([cap, cap + 1 + (i + 1) % SIDES as u32, cap + 1 + i]);
    }
    // The shaft.
    if len - head > 1e-12 {
        let s = verts.len() as u32;
        for r in &ring {
            verts.push((add_scaled(tail, *r, shaft_r), *r));
            verts.push((add_scaled(neck, *r, shaft_r), *r));
        }
        for i in 0..SIDES as u32 {
            let j = (i + 1) % SIDES as u32;
            let (a0, a1, b0, b1) = (s + 2 * i, s + 2 * i + 1, s + 2 * j, s + 2 * j + 1);
            tris.push([a0, b0, b1]);
            tris.push([a0, b1, a1]);
        }
    }
    // The head's back (a ring from the shaft out to the cone's rim).
    let hb = verts.len() as u32;
    for r in &ring {
        verts.push((add_scaled(neck, *r, shaft_r), back));
        verts.push((add_scaled(neck, *r, head_r), back));
    }
    for i in 0..SIDES as u32 {
        let j = (i + 1) % SIDES as u32;
        let (a0, a1, b0, b1) = (hb + 2 * i, hb + 2 * i + 1, hb + 2 * j, hb + 2 * j + 1);
        tris.push([a0, b0, b1]);
        tris.push([a0, b1, a1]);
    }
    // The cone: a normal per side, tilted towards the tip.
    if head > 1e-12 {
        let slope = head_r / head;
        for i in 0..SIDES {
            let j = (i + 1) % SIDES;
            let mid = {
                let m = add_scaled(ring[i], ring[j], 1.0);
                let l = dot(m, m).sqrt().max(1e-12);
                m.map(|x| x / l)
            };
            let normal = |r: [f64; 3]| {
                let n = add_scaled(r, dir, slope);
                let l = dot(n, n).sqrt().max(1e-12);
                n.map(|x| x / l)
            };
            let c = verts.len() as u32;
            verts.push((add_scaled(neck, ring[i], head_r), normal(ring[i])));
            verts.push((add_scaled(neck, ring[j], head_r), normal(ring[j])));
            verts.push((tip, normal(mid)));
            tris.push([c, c + 1, c + 2]);
        }
    }
    (verts, tris)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: an arrow is a closed solid pointing along its
    /// direction: its points span `len` along it, centred on its point,
    /// its triangles face outwards (their normals agree with the
    /// vertices' normals), and a short arrow keeps its head's width. A
    /// wrong winding would light the arrow from inside; the render test
    /// would show it only from one side.
    #[test]
    fn arrows_are_closed_and_point_along() {
        let at = [1.0, 2.0, 3.0];
        let dir = [0.0, 0.6, 0.8];
        for (len, h) in [(1.0, 1.0), (0.1, 1.0)] {
            let (verts, tris) = arrow(at, dir, len, h);
            let along: Vec<f64> = verts.iter().map(|(p, _)| dot(sub(*p, at), dir)).collect();
            let lo = along.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = along.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            assert!((lo + 0.5 * len).abs() < 1e-9 && (hi - 0.5 * len).abs() < 1e-9);
            let width = verts
                .iter()
                .map(|(p, _)| {
                    let d = sub(*p, at);
                    let a = dot(d, dir);
                    (dot(d, d) - a * a).max(0.0).sqrt()
                })
                .fold(0.0, f64::max);
            assert!((width - 0.12 * h).abs() < 1e-9, "head width {width}");
            for t in &tris {
                let [a, b, c] = t.map(|i| verts[i as usize]);
                let n = crate::clip::cross(sub(b.0, a.0), sub(c.0, a.0));
                if dot(n, n) < 1e-24 {
                    continue;
                }
                let mean = add_scaled(add_scaled(a.1, b.1, 1.0), c.1, 1.0);
                assert!(dot(n, mean) > 0.0, "triangle {t:?} faces inwards");
            }
        }
    }
}
