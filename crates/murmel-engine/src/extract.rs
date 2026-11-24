//! Slices and iso-surfaces (field results, stage 2; the maintainer's
//! cards of 2026-11-24).
//!
//! - A **slice** is a plane through the cells shown, placed and moved
//!   like a clip plane, that cuts nothing away: it draws the section of
//!   every cell it crosses in the colours of the mesh (the field shown in
//!   its style, or the colour mode's cell colours). A **series** is
//!   `count` parallel planes `spacing` apart, the middle one at the
//!   slice's origin. Its faces are the plane's cut through the cells'
//!   tetrahedra (below), so the values inside a large cell follow the
//!   cell's own value and not only its nodes'; the outline of each cell's
//!   section is found as for a flat clip ([`crate::surface`]): the points
//!   where its edges cross the plane, named by the edge, joined face by
//!   face into closed loops.
//! - An **iso-surface** is where one field takes one value. Each cell is
//!   cut into tetrahedra, one per edge of each face (the cell's centroid,
//!   the face's centre and the edge's two nodes), and each tetrahedron is
//!   cut by linear interpolation of its four values (marching
//!   tetrahedra: A. Doi and A. Koide, "An efficient method of
//!   triangulating equi-valued surfaces by using tetrahedral cells",
//!   IEICE Transactions E74(1), 1991). The centroid carries the cell's
//!   own value (the solver's); an inner face's centre the two cells'
//!   values interpolated linearly between their centroids (J. H.
//!   Ferziger and M. Perić, "Computational Methods for Fluid Dynamics",
//!   section 4.4, the usual face value of finite volumes), a boundary
//!   face's centre the mean of its nodes; a node its node value
//!   ([`crate::field`]). Tetrahedra have no ambiguous
//!   cases, and neighbouring cells share the face centres and edges where
//!   points are made, so the surface closes across cells. A point's normal
//!   is the area-weighted mean of its triangles' normals, each turned
//!   towards growing values.
//!
//! Clip planes apply to both: crinkle planes through the cells shown,
//! flat planes by clipping the polygons (Sutherland and Hodgman). While
//! slices or iso-surfaces show, the mesh draws as a faint outline unless
//! the user asks for its faces ([`MeshLook`], decided 2026-11-24).

use std::collections::{HashMap, HashSet};

use murmel_mesh::{Bounds, Component, Mesh, NO_CELL};
use serde::{Deserialize, Serialize};

use crate::clip::{Clip, ClipMode};
use crate::display::{Display, Item};
use crate::field::{FieldChoice, FieldSet, FieldValues};
use crate::surface::Surface;
use crate::theme::{Rgb, Tokens};

/// Most slices (a series counts once) and iso-surfaces one display holds.
pub const MAX_SLICES: usize = 8;
pub const MAX_ISOS: usize = 8;
/// Most planes in one series.
pub const MAX_SERIES: u32 = 64;

fn one() -> u32 {
    1
}

fn is_one(n: &u32) -> bool {
    *n == 1
}

fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

/// A slice, or a series of parallel slices (`slice.set`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slice {
    /// A point of the plane (of the middle plane of a series).
    pub origin: [f64; 3],
    /// Unit normal.
    pub normal: [f64; 3],
    /// Off keeps the slice but draws nothing (the tree's eye).
    pub enabled: bool,
    /// Planes in the series, 1 to [`MAX_SERIES`].
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub count: u32,
    /// Distance between neighbouring planes of a series.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub spacing: f64,
}

impl Slice {
    pub fn new(origin: [f64; 3], normal: [f64; 3]) -> Self {
        Self {
            origin,
            normal,
            enabled: true,
            count: 1,
            spacing: 0.0,
        }
    }

    /// The slice as a plane, for what slices share with clip planes
    /// (offsets, the handles' pivot and turns).
    pub fn plane(&self) -> Clip {
        Clip {
            origin: self.origin,
            normal: self.normal,
            mode: ClipMode::Flat,
            enabled: self.enabled,
        }
    }

    /// Take the place and direction of `plane`.
    pub fn set_plane(&mut self, plane: &Clip) {
        self.origin = plane.origin;
        self.normal = plane.normal;
    }

    /// A point of each plane of the series, in order along the normal,
    /// the middle one at `origin`.
    pub fn plane_origins(&self) -> Vec<[f64; 3]> {
        let n = self.count.clamp(1, MAX_SERIES);
        (0..n)
            .map(|i| {
                let s = (i as f64 - (n - 1) as f64 / 2.0) * self.spacing;
                [0, 1, 2].map(|k| self.origin[k] + s * self.normal[k])
            })
            .collect()
    }

    /// `count` planes through `bounds`, each in the middle of one of
    /// `count` equal slabs of the box along the normal.
    pub fn spread(&mut self, bounds: &Bounds, count: u32) {
        let count = count.clamp(1, MAX_SERIES);
        let mut plane = self.plane();
        let (_, [lo, hi]) = plane.offset(bounds);
        plane.set_offset(bounds, 0.5 * (lo + hi));
        self.origin = plane.origin;
        self.count = count;
        self.spacing = if count > 1 {
            (hi - lo) / count as f64
        } else {
            0.0
        };
    }
}

/// How an iso-surface is coloured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsoColor {
    /// One colour of its own (the default).
    #[default]
    Solid,
    /// The field on the colour bar, in its style (when colour mode field
    /// shows one; else solid).
    Field,
}

impl IsoColor {
    pub const ALL: [IsoColor; 2] = [IsoColor::Solid, IsoColor::Field];

    pub fn key(self) -> &'static str {
        match self {
            IsoColor::Solid => "solid",
            IsoColor::Field => "field",
        }
    }
}

/// An iso-surface: where `field` takes `value` (`iso.set`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Iso {
    pub field: FieldChoice,
    pub value: f64,
    pub enabled: bool,
    #[serde(default)]
    pub color: IsoColor,
}

/// What the mesh looks like while slices or iso-surfaces show
/// (`display.mesh_look`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeshLook {
    /// Its feature edges only, faint, so what is inside stands out
    /// (decided 2026-11-24).
    #[default]
    Outline,
    /// As the render mode says.
    Faces,
}

impl MeshLook {
    pub const ALL: [MeshLook; 2] = [MeshLook::Outline, MeshLook::Faces];

    pub fn key(self) -> &'static str {
        match self {
            MeshLook::Outline => "outline",
            MeshLook::Faces => "faces",
        }
    }
}

/// The solid colour of iso-surface `k`: the user's, else the patch hues
/// in turn.
pub fn iso_color(display: &Display, tokens: &Tokens, k: usize) -> Rgb {
    display
        .colors
        .get(&Item::Iso(k as u32))
        .copied()
        .unwrap_or_else(|| {
            let p = &tokens.mesh.patch;
            if p.is_empty() {
                tokens.mesh.solid
            } else {
                p[k % p.len()]
            }
        })
}

/// A corner of a polygon to draw: where it is, its node value (NaN when
/// none), its normal, and for an edge's cut point the edge (so the
/// outlines of neighbouring sections are drawn once).
#[derive(Clone, Copy, Debug)]
struct Vx {
    p: [f64; 3],
    v: f64,
    n: [f64; 3],
    key: Option<(u32, u32)>,
}

fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [0, 1, 2].map(|k| a[k] + t * (b[k] - a[k]))
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt();
    if l > 0.0 && l.is_finite() {
        a.map(|x| x / l)
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// The part of `poly` that the flat clip planes keep (Sutherland and
/// Hodgman), node values and normals interpolated at the cuts.
fn clip_flat(poly: Vec<Vx>, planes: &[&Clip]) -> Vec<Vx> {
    let mut v = poly;
    for plane in planes {
        if v.len() < 3 {
            break;
        }
        let mut out = Vec::with_capacity(v.len() + 2);
        for i in 0..v.len() {
            let (a, b) = (v[i], v[(i + 1) % v.len()]);
            let (da, db) = (plane.distance(a.p), plane.distance(b.p));
            let (ka, kb) = (da <= 0.0, db <= 0.0);
            if ka {
                out.push(a);
            }
            if ka != kb {
                let t = da / (da - db);
                out.push(Vx {
                    p: lerp3(a.p, b.p, t),
                    v: lerp(a.v, b.v, t),
                    n: unit(lerp3(a.n, b.n, t)),
                    key: None,
                });
            }
        }
        v = out;
    }
    v
}

/// The smallest and largest of the values added.
#[derive(Default)]
struct Span(Option<(f64, f64)>);

impl Span {
    fn add(&mut self, x: f64) {
        if x.is_finite() {
            self.0 = Some(match self.0 {
                None => (x, x),
                Some((a, b)) => (a.min(x), b.max(x)),
            });
        }
    }
}

/// Writes polygons into a [`Surface`] the way the mesh's are written
/// (positions relative to its origin, a fan about the centre).
struct Out<'a> {
    s: &'a mut Surface,
    span: Span,
    /// Edges drawn already, by the cut points' edges.
    seen: HashSet<((u32, u32), (u32, u32))>,
}

/// How one polygon is coloured.
struct Paint {
    /// Linear face colour, and the colour of its outline (None: no
    /// outline).
    color: [f32; 3],
    edge: Option<[f32; 3]>,
    /// The polygon's flat value (colour mode flat) when it has field
    /// values.
    flat: Option<f64>,
    sel_mix: f32,
    cell: u32,
}

impl Out<'_> {
    fn poly(&mut self, verts: &[Vx], paint: &Paint) {
        if verts.len() < 3 {
            return;
        }
        let o = self.s.origin;
        let vo = self.s.value_origin;
        // Field values only when every corner has one.
        let valued = paint
            .flat
            .filter(|f| f.is_finite() && verts.iter().all(|v| v.v.is_finite()));
        if let Some(f) = valued {
            self.span.add(f);
            verts.iter().for_each(|v| self.span.add(v.v));
        }
        let value = |node: f64| -> [f32; 4] {
            match valued {
                Some(f) => [(f - vo) as f32, (node - vo) as f32, paint.sel_mix, 1.0],
                None => [0.0, 0.0, paint.sel_mix, 0.0],
            }
        };
        let rel = |p: [f64; 3]| [0, 1, 2].map(|k| (p[k] - o[k]) as f32);
        let base = self.s.positions.len() as u32;
        let push = |s: &mut Surface, p: [f64; 3], n: [f64; 3], node: f64| {
            s.positions.push(rel(p));
            s.normals.push(n.map(|x| x as f32));
            s.colors.push(paint.color);
            s.values.push(value(node));
        };
        let tris_before = self.s.indices.len() / 3;
        if verts.len() == 3 {
            for v in verts {
                push(self.s, v.p, v.n, v.v);
            }
            self.s.indices.extend([base, base + 1, base + 2]);
        } else {
            let k = verts.len() as f64;
            let c = [0, 1, 2].map(|d| verts.iter().map(|v| v.p[d]).sum::<f64>() / k);
            let n = unit([0, 1, 2].map(|d| verts.iter().map(|v| v.n[d]).sum::<f64>()));
            let mean = verts.iter().map(|v| v.v).sum::<f64>() / k;
            push(self.s, c, n, mean);
            for v in verts {
                push(self.s, v.p, v.n, v.v);
            }
            let m = verts.len() as u32;
            for i in 0..m {
                self.s
                    .indices
                    .extend([base, base + 1 + i, base + 1 + (i + 1) % m]);
            }
        }
        let tris = self.s.indices.len() / 3 - tris_before;
        self.s
            .tri_cell
            .extend(std::iter::repeat_n(paint.cell, tris));
        self.s.tri_face.extend(std::iter::repeat_n(NO_CELL, tris));
        let mut vb = self.s.visible_bounds.unwrap_or_else(Bounds::empty);
        verts.iter().for_each(|v| vb.add(v.p));
        self.s.visible_bounds = Some(vb);
        if let Some(ec) = paint.edge {
            self.outline(verts, ec);
        }
    }

    /// The outline of a polygon; an edge between two cut points of the
    /// same mesh edges is drawn once.
    fn outline(&mut self, verts: &[Vx], ec: [f32; 3]) {
        if verts.len() < 2 {
            return;
        }
        let o = self.s.origin;
        let rel = |p: [f64; 3]| [0, 1, 2].map(|k| (p[k] - o[k]) as f32);
        for j in 0..verts.len() {
            let (a, b) = (verts[j], verts[(j + 1) % verts.len()]);
            if let (Some(ka), Some(kb)) = (a.key, b.key)
                && !self.seen.insert((ka.min(kb), ka.max(kb)))
            {
                continue;
            }
            self.s.edge_positions.push(rel(a.p));
            self.s.edge_positions.push(rel(b.p));
            self.s.edge_colors.push(ec);
            self.s.edge_colors.push(ec);
        }
    }
}

/// The section of cell `c` by a plane with node distances `dist`:
/// closed loops of cut points, each named by its edge (lower node
/// first). A node on the plane counts as behind it, as for clip planes.
fn section(mesh: &Mesh, c: usize, dist: &[f64]) -> Vec<Vec<(u32, u32)>> {
    let behind = |i: u32| dist.get(i as usize).is_none_or(|&d| d <= 0.0);
    let (mut any_in, mut any_out) = (false, false);
    for f in mesh.cell(c) {
        for &i in f {
            if behind(i) {
                any_in = true;
            } else {
                any_out = true;
            }
        }
    }
    if !(any_in && any_out) {
        return Vec::new();
    }
    let key = |a: u32, b: u32| (a.min(b), a.max(b));
    let mut next: HashMap<(u32, u32), Vec<(u32, u32)>> = HashMap::new();
    for f in mesh.cell(c) {
        let n = f.len();
        let mut cuts: Vec<((u32, u32), bool)> = Vec::new();
        for i in 0..n {
            let (a, b) = (f[i], f[(i + 1) % n]);
            let (ka, kb) = (behind(a), behind(b));
            if ka != kb {
                cuts.push((key(a, b), ka));
            }
        }
        for (i, &(v, leaving)) in cuts.iter().enumerate() {
            if !leaving {
                continue;
            }
            if let Some(&(w, _)) = (1..cuts.len())
                .map(|j| &cuts[(i + j) % cuts.len()])
                .find(|(_, l)| !l)
            {
                next.entry(v).or_default().push(w);
                next.entry(w).or_default().push(v);
            }
        }
    }
    let mut loops = Vec::new();
    let mut used: HashSet<(u32, u32)> = HashSet::new();
    let mut starts: Vec<(u32, u32)> = next.keys().copied().collect();
    starts.sort_unstable();
    for start in starts {
        if used.contains(&start) {
            continue;
        }
        let mut ring = vec![start];
        used.insert(start);
        let mut cur = start;
        while let Some(&n) = next
            .get(&cur)
            .and_then(|ns| ns.iter().find(|n| !used.contains(n)))
        {
            ring.push(n);
            used.insert(n);
            cur = n;
        }
        if ring.len() >= 3 {
            loops.push(ring);
        }
    }
    loops
}

/// Whether `item` is selected.
fn selected(display: &Display, item: Item) -> bool {
    display.selection.contains(&item)
}

/// Draw the shown slices and iso-surfaces of `display` into `s` (the
/// mesh's surface, whose origin and value origin they share). `shown`
/// are the cells shown; `fields` gives the iso-surfaces' fields.
pub fn append(
    s: &mut Surface,
    mesh: &Mesh,
    fields: Option<&FieldSet>,
    tokens: &Tokens,
    display: &Display,
    shown: &[bool],
) {
    let flat: Vec<&Clip> = display
        .active_clips()
        .filter(|c| c.mode == ClipMode::Flat)
        .collect();
    let field = display.field_values();
    let edges = display.render_mode.edges();
    let sel = tokens.mesh.selection;
    let mix = tokens.mesh.selection_mix.clamp(0.0, 1.0);
    let darken = 1.0 - tokens.mesh.edge_darken;
    let mut out = Out {
        s,
        span: Span(None),
        seen: HashSet::new(),
    };
    for (k, slice) in display.slices.iter().enumerate() {
        if !slice.enabled {
            continue;
        }
        let picked = selected(display, Item::Slice(k as u32));
        let normal = unit(slice.normal);
        for origin in slice.plane_origins() {
            let plane = Clip {
                origin,
                normal,
                mode: ClipMode::Flat,
                enabled: true,
            };
            // How the section of cell `c` is coloured: the field shown,
            // else the colour mode's cell colour; and its outline's colour.
            let paint = |c: usize| {
                let rgb = if field.is_some() {
                    tokens.mesh.field.none
                } else {
                    display.cell_color(mesh, tokens, c)
                };
                let face = if picked { mixed(rgb, sel, mix) } else { rgb };
                let edge = if picked {
                    sel
                } else if field.is_some() {
                    tokens.mesh.field.edge
                } else {
                    Rgb(rgb.0.map(|v| (v as f32 * darken).round() as u8))
                };
                Paint {
                    color: face.linear(),
                    edge: edges.then(|| edge.linear()),
                    flat: field.map(|f| f.flat(c as u32, NO_CELL)),
                    sel_mix: if picked { mix } else { 0.0 },
                    cell: c as u32,
                }
            };
            // The faces: the plane through the cells' tetrahedra, so the
            // values inside a cell follow its own value (as for
            // iso-surfaces), not only its nodes'.
            for t in level_set(mesh, fields, Level::Plane(&plane), field, shown) {
                let verts: Vec<Vx> = t.verts.iter().map(|v| Vx { n: normal, ..*v }).collect();
                let verts = clip_flat(verts, &flat);
                out.poly(
                    &verts,
                    &Paint {
                        edge: None,
                        ..paint(t.cell as usize)
                    },
                );
            }
            // The outlines of the cells' sections.
            if !edges {
                continue;
            }
            let dist: Vec<f64> = mesh.points.iter().map(|&p| plane.distance(p)).collect();
            for (c, &on) in shown.iter().enumerate() {
                if !on {
                    continue;
                }
                let Some(ec) = paint(c).edge else { continue };
                for ring in section(mesh, c, &dist) {
                    let verts: Vec<Vx> = ring
                        .iter()
                        .map(|&(a, b)| {
                            let (da, db) = (dist[a as usize], dist[b as usize]);
                            let t = if da == db { 0.0 } else { da / (da - db) };
                            Vx {
                                p: lerp3(mesh.points[a as usize], mesh.points[b as usize], t),
                                v: f64::NAN,
                                n: normal,
                                key: Some((a, b)),
                            }
                        })
                        .collect();
                    out.outline(&clip_flat(verts, &flat), ec);
                }
            }
        }
    }
    if let Some(fields) = fields {
        for (k, iso) in display.isos.iter().enumerate() {
            if !iso.enabled {
                continue;
            }
            let Some(values) = fields.values_cached(mesh, &iso.field) else {
                continue;
            };
            let picked = selected(display, Item::Iso(k as u32));
            // Coloured by the field on the colour bar, or solid.
            let colour = match iso.color {
                IsoColor::Field => field,
                IsoColor::Solid => None,
            };
            let rgb = if colour.is_some() {
                tokens.mesh.field.none
            } else {
                iso_color(display, tokens, k)
            };
            let face = if picked { mixed(rgb, sel, mix) } else { rgb };
            let tris = level_set(
                mesh,
                Some(fields),
                Level::Field(&values, iso.value),
                colour,
                shown,
            );
            for t in tris {
                let paint = Paint {
                    color: face.linear(),
                    edge: None,
                    flat: colour.map(|f| f.flat(t.cell, NO_CELL)),
                    sel_mix: if picked { mix } else { 0.0 },
                    cell: t.cell,
                };
                let verts = clip_flat(t.verts.to_vec(), &flat);
                out.poly(&verts, &paint);
            }
        }
    }
    let span = out.span.0;
    if let Some((lo, hi)) = span {
        s.field_range = Some(match s.field_range {
            Some((a, b)) => (a.min(lo), b.max(hi)),
            None => (lo, hi),
        });
    }
}

fn mixed(a: Rgb, b: Rgb, mix: f32) -> Rgb {
    Rgb(std::array::from_fn(|k| {
        (a.0[k] as f32 * (1.0 - mix) + b.0[k] as f32 * mix).round() as u8
    }))
}

/// A point of the tetrahedra of a cell: a node, the centre of a distinct
/// face, or the centroid of a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Pt {
    Node(u32),
    Face(u32),
    Cell(u32),
}

/// One triangle of an iso-surface, in cell `cell`.
struct Tri {
    cell: u32,
    verts: [Vx; 3],
}

/// What a level set is of: where a field takes a value (an
/// iso-surface), or a plane (a slice: where the distance to it is 0).
#[derive(Clone, Copy)]
enum Level<'a> {
    Field(&'a FieldValues, f64),
    Plane(&'a Clip),
}

/// The level set `level` in the cells shown, by marching tetrahedra on
/// the cells' tetrahedra (centroid, face centre, edge), with the values
/// of `colour` (the field it is coloured by) at its corners. A slice cut
/// this way shows the same piecewise linear field as an iso-surface, so
/// the cells' own values show inside large cells, and an iso-surface
/// meets a slice on the slice's contour of that value.
fn level_set(
    mesh: &Mesh,
    fields: Option<&FieldSet>,
    level: Level<'_>,
    colour: Option<&FieldValues>,
    shown: &[bool],
) -> Vec<Tri> {
    let iso = match level {
        Level::Field(_, v) => v,
        Level::Plane(_) => 0.0,
    };
    if !iso.is_finite() {
        return Vec::new();
    }
    let node_of = |f: &FieldValues, i: u32| f.node.get(i as usize).copied().unwrap_or(f64::NAN);
    let centroid = |c: usize| match fields {
        Some(f) => f.centroid(mesh, c),
        None => crate::inspect::cell_centroid(mesh, c),
    };
    // The value at the centre `fc` of distinct face `u`: between the two
    // cells' values by their centroids' distances to it, or for a
    // boundary face (or a cell without a value) the mean of its nodes.
    let face_value = |f: &FieldValues, u: u32, face: &[u32], fc: [f64; 3]| -> f64 {
        let nodes = || face.iter().map(|&n| node_of(f, n)).sum::<f64>() / face.len() as f64;
        let (Some(&o), Some(&n)) = (
            mesh.face_owner.get(u as usize),
            mesh.face_neighbour.get(u as usize),
        ) else {
            return nodes();
        };
        if o == NO_CELL || n == NO_CELL {
            return nodes();
        }
        let (vo, vn) = (
            f.cell.get(o as usize).copied().unwrap_or(f64::NAN),
            f.cell.get(n as usize).copied().unwrap_or(f64::NAN),
        );
        let dist = |c: u32| {
            let x = centroid(c as usize);
            (0..3).map(|d| (x[d] - fc[d]).powi(2)).sum::<f64>().sqrt()
        };
        let (d_o, d_n) = (dist(o), dist(n));
        if !(vo.is_finite() && vn.is_finite()) || d_o + d_n <= 0.0 {
            return nodes();
        }
        (d_n * vo + d_o * vn) / (d_o + d_n)
    };
    let node_level = |n: u32| match level {
        Level::Field(v, _) => node_of(v, n),
        Level::Plane(p) => p.distance(mesh.points[n as usize]),
    };
    // A point of the surface on the segment between two tetrahedron
    // points, made once: (position, colour value).
    let mut made: HashMap<(Pt, Pt), u32> = HashMap::new();
    let mut points: Vec<([f64; 3], f64)> = Vec::new();
    let mut normals: Vec<[f64; 3]> = Vec::new();
    // Triangles by point index, with their cell.
    let mut tris: Vec<([u32; 3], u32)> = Vec::new();
    for (c, &on) in shown.iter().enumerate() {
        if !on {
            continue;
        }
        let range = mesh.cell_face_range(c);
        // Quick test: the cell's values all on one side.
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for i in range.clone() {
            for &n in mesh.faces.get(i) {
                let v = node_level(n);
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        let vc = match level {
            Level::Field(v, _) => {
                let own = v.cell.get(c).copied().unwrap_or(f64::NAN);
                if own.is_finite() {
                    own
                } else {
                    0.5 * (lo + hi)
                }
            }
            // A plane's distance is linear: the nodes bound it.
            Level::Plane(_) => 0.5 * (lo + hi),
        };
        lo = lo.min(vc);
        hi = hi.max(vc);
        if !lo.is_finite() || !hi.is_finite() || lo == hi || iso < lo || iso > hi {
            continue;
        }
        let centre = centroid(c);
        let vc = match level {
            Level::Plane(p) => p.distance(centre),
            Level::Field(..) => vc,
        };
        let cc = colour.map_or(f64::NAN, |f| f.cell.get(c).copied().unwrap_or(f64::NAN));
        for i in range {
            let f = mesh.faces.get(i);
            if f.len() < 3 {
                continue;
            }
            let u = mesh.cell_face_id.get(i).copied().unwrap_or(NO_CELL);
            let k = f.len() as f64;
            let fc =
                [0, 1, 2].map(|d| f.iter().map(|&n| mesh.points[n as usize][d]).sum::<f64>() / k);
            let vf = match level {
                Level::Field(v, _) => face_value(v, u, f, fc),
                Level::Plane(p) => p.distance(fc),
            };
            let cf = colour.map_or(f64::NAN, |cv| face_value(cv, u, f, fc));
            // A face without a distinct id (should not happen) is made
            // per cell.
            let fpt = if u == NO_CELL {
                Pt::Cell(u32::MAX - c as u32)
            } else {
                Pt::Face(u)
            };
            for j in 0..f.len() {
                let (a, b) = (f[j], f[(j + 1) % f.len()]);
                let tet = [
                    (Pt::Cell(c as u32), centre, vc, cc),
                    (fpt, fc, vf, cf),
                    (
                        Pt::Node(a),
                        mesh.points[a as usize],
                        node_level(a),
                        colour.map_or(f64::NAN, |cv| node_of(cv, a)),
                    ),
                    (
                        Pt::Node(b),
                        mesh.points[b as usize],
                        node_level(b),
                        colour.map_or(f64::NAN, |cv| node_of(cv, b)),
                    ),
                ];
                if tet.iter().any(|t| !t.2.is_finite()) {
                    continue;
                }
                let above: Vec<usize> = (0..4).filter(|&q| tet[q].2 > iso).collect();
                let below: Vec<usize> = (0..4).filter(|&q| tet[q].2 <= iso).collect();
                if above.is_empty() || below.is_empty() {
                    continue;
                }
                let mut point = |p: usize, q: usize| -> u32 {
                    let (lo, hi) = if tet[p].0 <= tet[q].0 { (p, q) } else { (q, p) };
                    *made.entry((tet[lo].0, tet[hi].0)).or_insert_with(|| {
                        let (vl, vh) = (tet[lo].2, tet[hi].2);
                        let t = if vh == vl {
                            0.5
                        } else {
                            (iso - vl) / (vh - vl)
                        };
                        let t = t.clamp(0.0, 1.0);
                        points.push((
                            lerp3(tet[lo].1, tet[hi].1, t),
                            lerp(tet[lo].3, tet[hi].3, t),
                        ));
                        normals.push([0.0; 3]);
                        (points.len() - 1) as u32
                    })
                };
                // One point above (or below): a triangle; two each: a
                // quad of the four crossed edges.
                let corners: Vec<u32> = if above.len() == 1 || below.len() == 1 {
                    let (lone, rest) = if above.len() == 1 {
                        (above[0], below.clone())
                    } else {
                        (below[0], above.clone())
                    };
                    rest.iter().map(|&q| point(lone, q)).collect()
                } else {
                    let (a0, a1, b0, b1) = (above[0], above[1], below[0], below[1]);
                    vec![point(a0, b0), point(a0, b1), point(a1, b1), point(a1, b0)]
                };
                // Turn towards growing values: from the mean of the
                // points below to the mean of those above.
                let mean = |qs: &[usize]| {
                    let n = qs.len() as f64;
                    [0, 1, 2].map(|d| qs.iter().map(|&q| tet[q].1[d]).sum::<f64>() / n)
                };
                let up = sub(mean(&above), mean(&below));
                let mut add = |t: [u32; 3]| {
                    let (p0, p1, p2) = (
                        points[t[0] as usize].0,
                        points[t[1] as usize].0,
                        points[t[2] as usize].0,
                    );
                    let n = cross(sub(p1, p0), sub(p2, p0));
                    let (t, n) = if dot(n, up) < 0.0 {
                        ([t[0], t[2], t[1]], n.map(|x| -x))
                    } else {
                        (t, n)
                    };
                    if dot(n, n) == 0.0 {
                        return;
                    }
                    for &i in &t {
                        let m = &mut normals[i as usize];
                        *m = [m[0] + n[0], m[1] + n[1], m[2] + n[2]];
                    }
                    tris.push((t, c as u32));
                };
                add([corners[0], corners[1], corners[2]]);
                if corners.len() == 4 {
                    add([corners[0], corners[2], corners[3]]);
                }
            }
        }
    }
    tris.into_iter()
        .map(|(t, cell)| Tri {
            cell,
            verts: t.map(|i| {
                let (p, v) = points[i as usize];
                Vx {
                    p,
                    v,
                    n: unit(normals[i as usize]),
                    key: None,
                }
            }),
        })
        .collect()
}

/// The field an iso-surface shows, with the component a scalar takes.
pub fn normalized(choice: &FieldChoice, vector: bool) -> FieldChoice {
    FieldChoice {
        name: choice.name.clone(),
        component: if vector {
            choice.component
        } else {
            Component::Magnitude
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::ColorBy;

    fn channel() -> (Mesh, FieldSet) {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
        let (m, r) = murmel_mesh::read_results(&p).unwrap();
        (m, FieldSet::new(r))
    }

    fn by_field(m: &Mesh, set: &FieldSet, name: &str) -> Display {
        let mut d = Display {
            color_by: ColorBy::Field,
            field: Some(FieldChoice {
                name: name.into(),
                component: Component::Magnitude,
            }),
            ..Display::default()
        };
        crate::field::ensure_field(&mut d, m, set);
        d
    }

    fn build(m: &Mesh, set: &FieldSet, d: &Display) -> Surface {
        Surface::build_with(m, Some(set), &Tokens::builtin().unwrap(), d)
    }

    /// The triangles of `s` that show slices or iso-surfaces (no face of
    /// the mesh), as world points.
    fn extract_tris(s: &Surface) -> Vec<[[f64; 3]; 3]> {
        s.indices
            .as_chunks::<3>()
            .0
            .iter()
            .enumerate()
            .filter(|(t, _)| s.tri_face[*t] == NO_CELL)
            .map(|(_, t)| {
                t.map(|i| {
                    let p = s.positions[i as usize];
                    [0, 1, 2].map(|k| p[k] as f64 + s.origin[k])
                })
            })
            .collect()
    }

    fn area(t: &[[f64; 3]; 3]) -> f64 {
        let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
        0.5 * dot(n, n).sqrt()
    }

    /// Invariant: a slice across the duct covers its whole section once
    /// (the areas of the cells' sections add up to the box's y-z face,
    /// the duct being a box), every point lies on the plane, each
    /// section polygon carries its cell's own value for colour mode flat,
    /// and a series of 4 covers 4 sections. While it shows, the mesh
    /// draws no faces (its faint outline only), unless the user asks for
    /// them. A section that missed cells, doubled them or left the plane
    /// would change the area; the flat-clip tests check sections of the
    /// mesh's own faces, not slices.
    #[test]
    fn slices_cover_the_section() {
        let (m, set) = channel();
        let b = m.bounds();
        let section = (b.max[1] - b.min[1]) * (b.max[2] - b.min[2]);
        let mut d = by_field(&m, &set, "pressure");
        let mut slice = Slice::new(b.center(), [1.0, 0.0, 0.0]);
        // Off the cell faces, so no node lies on the plane.
        slice.origin[0] += 0.013 * (b.max[0] - b.min[0]);
        d.slices.push(slice);
        let s = build(&m, &set, &d);
        assert!(s.tri_face.iter().all(|&f| f == NO_CELL), "mesh faces drawn");
        assert!(!s.edge_positions.is_empty(), "no outline");
        let tris = extract_tris(&s);
        let a: f64 = tris.iter().map(area).sum();
        assert!((a - section).abs() < 1e-3 * section, "{a} vs {section}");
        for t in &tris {
            for p in t {
                assert!((p[0] - slice.origin[0]).abs() < 1e-5);
            }
        }
        // Colour mode flat reads the cell's value.
        let values = d.field_values.clone().unwrap();
        for (t, &c) in s.tri_cell.iter().enumerate() {
            let v = s.values[s.indices[3 * t] as usize];
            let want = values.cell[c as usize] - s.value_origin;
            assert!((v[0] as f64 - want).abs() < 1e-3 * want.abs().max(1.0));
            assert_eq!(v[3], 1.0);
        }
        // A series of 4 through the whole duct.
        d.slices[0].spread(&b, 4);
        assert_eq!(d.slices[0].plane_origins().len(), 4);
        let s4 = build(&m, &set, &d);
        let a4: f64 = extract_tris(&s4).iter().map(area).sum();
        assert!((a4 - 4.0 * section).abs() < 4e-3 * section, "{a4}");
        // The mesh's faces back on request.
        d.mesh_look = MeshLook::Faces;
        let with = build(&m, &set, &d);
        assert!(with.tri_face.iter().any(|&f| f != NO_CELL));
    }

    /// Invariant: a slice shows a cell's own value inside it, not only
    /// what its nodes say: across the duct (one core cell wide) the
    /// speed's largest value on the slice is the core cell's, above every
    /// node value (which inverse distance pulls down from the walls). An
    /// iso-surface of speed crosses the slice where the slice's
    /// interpolated speed takes that value, since both cut the same
    /// tetrahedra. Breaks if slices go back to node values only (the
    /// duct's peak vanished from smooth slices in the first build) or the
    /// two use different values inside a cell; the area test only checks
    /// geometry.
    #[test]
    fn slices_show_the_cells_own_values() {
        let (m, set) = channel();
        let b = m.bounds();
        let mut d = by_field(&m, &set, "speed");
        let mut slice = Slice::new(b.center(), [1.0, 0.0, 0.0]);
        slice.origin[0] += 0.013 * (b.max[0] - b.min[0]);
        d.slices.push(slice);
        let s = build(&m, &set, &d);
        let values = d.field_values.clone().unwrap();
        let top_node = values.node.iter().copied().fold(f64::MIN, f64::max);
        let top_cell = values.cell.iter().copied().fold(f64::MIN, f64::max);
        let top_slice = s
            .values
            .iter()
            .filter(|v| v[3] == 1.0)
            .map(|v| v[1] as f64 + s.value_origin)
            .fold(f64::MIN, f64::max);
        assert!(top_node < top_cell - 0.1, "{top_node} vs {top_cell}");
        assert!(
            top_slice > top_node + 0.5 * (top_cell - top_node),
            "slice peak {top_slice}, nodes {top_node}, cells {top_cell}"
        );
        // An iso-surface of speed 1.2 meets the plane where the slice
        // reads 1.2: sample the slice's triangles at the points where
        // the iso-surface's edges cross the plane.
        let mut d2 = by_field(&m, &set, "speed");
        d2.isos.push(Iso {
            field: FieldChoice {
                name: "speed".into(),
                component: Component::Magnitude,
            },
            value: 1.2,
            enabled: true,
            color: IsoColor::Solid,
        });
        let iso = build(&m, &set, &d2);
        let x = slice.origin[0];
        let crossings: Vec<[f64; 3]> = extract_tris(&iso)
            .iter()
            .flat_map(|t| {
                (0..3).filter_map(move |k| {
                    let (a, b) = (t[k], t[(k + 1) % 3]);
                    let (da, db) = (a[0] - x, b[0] - x);
                    (da * db < 0.0).then(|| {
                        let u = da / (da - db);
                        [0, 1, 2].map(|i| a[i] + u * (b[i] - a[i]))
                    })
                })
            })
            .collect();
        assert!(!crossings.is_empty());
        // Barycentric value of the slice at point q, from the triangle
        // containing it (y-z plane).
        let slice_value = |q: [f64; 3]| -> Option<f64> {
            let o = s.origin;
            for t in s.indices.as_chunks::<3>().0 {
                let p = t.map(|i| {
                    let x = s.positions[i as usize];
                    [x[1] as f64 + o[1], x[2] as f64 + o[2]]
                });
                let v = t.map(|i| s.values[i as usize][1] as f64 + s.value_origin);
                let det = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                    - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
                if det.abs() < 1e-18 {
                    continue;
                }
                let l1 = ((q[1] - p[0][0]) * (p[2][1] - p[0][1])
                    - (p[2][0] - p[0][0]) * (q[2] - p[0][1]))
                    / det;
                let l2 = ((p[1][0] - p[0][0]) * (q[2] - p[0][1])
                    - (q[1] - p[0][0]) * (p[1][1] - p[0][1]))
                    / det;
                if l1 >= -1e-6 && l2 >= -1e-6 && l1 + l2 <= 1.0 + 1e-6 {
                    return Some(v[0] + l1 * (v[1] - v[0]) + l2 * (v[2] - v[0]));
                }
            }
            None
        };
        let mut worst: f64 = 0.0;
        let mut found = 0;
        for q in &crossings {
            if let Some(v) = slice_value(*q) {
                worst = worst.max((v - 1.2).abs());
                found += 1;
            }
        }
        assert!(
            found * 10 >= crossings.len() * 9,
            "{found} of {} crossings on the slice",
            crossings.len()
        );
        assert!(worst < 0.05, "the slice reads up to {worst} off 1.2");
    }

    /// Invariant: flat clip planes cut slices too, crinkle planes and
    /// hidden cells leave their cells out. Breaks when slices ignore the
    /// clip planes (they would poke out of a cut model).
    #[test]
    fn clip_planes_cut_slices() {
        let (m, set) = channel();
        let b = m.bounds();
        let mut d = by_field(&m, &set, "speed");
        d.slices.push(Slice::new(b.center(), [0.0, 1.0, 0.0]));
        let c = b.center();
        d.clips.push(Clip {
            origin: c,
            normal: [1.0, 0.0, 0.0],
            mode: ClipMode::Flat,
            enabled: true,
        });
        let s = build(&m, &set, &d);
        let tris = extract_tris(&s);
        assert!(!tris.is_empty());
        assert!(tris.iter().flatten().all(|p| p[0] <= c[0] + 1e-6));
        let half: f64 = tris.iter().map(area).sum();
        let full = (b.max[0] - b.min[0]) * (b.max[2] - b.min[2]);
        assert!((half - 0.5 * full).abs() < 1e-3 * full, "{half} vs {full}");
    }

    /// Invariant: the iso-surface of a field linear in x (the pressure,
    /// 50 - 100 x/L at the cell centres) is a plane across the duct: its
    /// points lie close to where the field crosses the value, its area is
    /// the section's, its normals point to growing pressure (-x), and it
    /// closes across cells: every edge of it is shared by two triangles
    /// except on the duct's walls. Breaks when a tetrahedron case is
    /// wrong (holes), points are made twice with different positions
    /// (cracks), or the orientation is not turned to the gradient.
    #[test]
    fn iso_surface_of_a_linear_field() {
        let (m, set) = channel();
        let b = m.bounds();
        let mut d = by_field(&m, &set, "speed");
        d.isos.push(Iso {
            field: FieldChoice {
                name: "pressure".into(),
                component: Component::Magnitude,
            },
            value: 3.0,
            enabled: true,
            color: IsoColor::Solid,
        });
        let s = build(&m, &set, &d);
        let tris = extract_tris(&s);
        assert!(!tris.is_empty());
        // Where the cell-centre pressure is 3: x/L = 0.47.
        let len = b.max[0] - b.min[0];
        let x = b.min[0] + 0.47 * len;
        assert!(
            tris.iter().flatten().all(|p| (p[0] - x).abs() < 0.05 * len),
            "points far from x = {x}"
        );
        let a: f64 = tris.iter().map(area).sum();
        let section = (b.max[1] - b.min[1]) * (b.max[2] - b.min[2]);
        // Node values by inverse distance are not exactly linear on the
        // graded mesh, so the surface waves a little (2.7 % more area
        // here); a hole or a doubled cell would be far more.
        assert!((a - section).abs() < 0.05 * section, "{a} vs {section}");
        let first = s.indices.len() / 3 - tris.len();
        let nx: f64 = (first..s.indices.len() / 3)
            .map(|t| s.normals[s.indices[3 * t] as usize][0] as f64)
            .sum();
        assert!(nx < 0.0, "normals do not point to growing pressure");
        // Edges by their end points (exact: shared points are made once).
        let key = |p: [f64; 3]| p.map(|v| (v * 1e9).round() as i64);
        let mut count: HashMap<([i64; 3], [i64; 3]), usize> = HashMap::new();
        for t in &tris {
            for j in 0..3 {
                let (a, b) = (key(t[j]), key(t[(j + 1) % 3]));
                *count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let on_wall = |p: [i64; 3]| {
            let p = p.map(|v| v as f64 * 1e-9);
            (1..3).any(|k| (p[k] - b.min[k]).abs() < 1e-6 || (p[k] - b.max[k]).abs() < 1e-6)
        };
        for ((a, c), n) in count {
            assert!(
                n == 2 || (n == 1 && on_wall(a) && on_wall(c)),
                "edge used {n} times"
            );
        }
        // Coloured by the field shown: values on, solid: off.
        let solid = s.values[s.indices[3 * first] as usize][3];
        assert_eq!(solid, 0.0);
        d.isos[0].color = IsoColor::Field;
        let s = build(&m, &set, &d);
        let first = s.indices.len() / 3 - extract_tris(&s).len();
        assert_eq!(s.values[s.indices[3 * first] as usize][3], 1.0);
    }

    /// Invariant: removing a slice or an iso-surface keeps the selection
    /// on the same items (those after it move up one) and an
    /// iso-surface's own colour with it. Breaks when ids shift wrongly
    /// and the handle or Properties show another item.
    #[test]
    fn removing_keeps_ids() {
        let mut d = Display::default();
        for k in 0..3 {
            d.slices
                .push(Slice::new([k as f64, 0.0, 0.0], [1.0, 0.0, 0.0]));
            d.isos.push(Iso {
                field: FieldChoice {
                    name: "p".into(),
                    component: Component::Magnitude,
                },
                value: k as f64,
                enabled: true,
                color: IsoColor::Solid,
            });
        }
        d.colors.insert(Item::Iso(2), Rgb([1, 2, 3]));
        d.selection = vec![Item::Slice(2), Item::Iso(0), Item::Iso(2)];
        assert!(d.remove_slice(1).is_some());
        assert_eq!(
            d.selection,
            vec![Item::Slice(1), Item::Iso(0), Item::Iso(2)]
        );
        assert!(d.remove_iso(0).is_some());
        assert_eq!(d.selection, vec![Item::Slice(1), Item::Iso(1)]);
        assert_eq!(d.colors.get(&Item::Iso(1)), Some(&Rgb([1, 2, 3])));
        assert_eq!(d.current_slice(), Some(1));
        assert_eq!(d.isos[d.current_iso().unwrap()].value, 2.0);
    }
}
