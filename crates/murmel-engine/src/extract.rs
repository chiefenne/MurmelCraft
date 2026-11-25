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
//!   own value (the solver's); an inner face's centre and a node what
//!   the cells around predict from their gradients ([`crate::tets`],
//!   [`crate::field`]), as does a boundary face's centre from its cell,
//!   so a field linear in space gives an exactly flat surface. Tetrahedra
//!   have no ambiguous
//!   cases, and neighbouring cells share the face centres and edges where
//!   points are made, so the surface closes across cells. A point's normal
//!   is the field's slope there, interpolated along its edge from the
//!   cell gradients ([`crate::gradient`]: a node has its cells' by
//!   inverse distance, a centroid and a face centre the mean of their
//!   nodes'), so the shading is smooth across the facets; where the slope
//!   is zero or disagrees with the facets, the area-weighted mean of its
//!   triangles' normals. Both point towards growing values.
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
use crate::glyph::ArrowColor;
use crate::region::Bound;
use crate::surface::{IsoPart, SlicePart, Surface};
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
/// none), its normal, for an edge's cut point the edge (so the outlines
/// of neighbouring sections are drawn once), and the vector of the
/// arrows there (NaN when none).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Vx {
    pub(crate) p: [f64; 3],
    pub(crate) v: f64,
    pub(crate) n: [f64; 3],
    pub(crate) key: Option<(u32, u32)>,
    pub(crate) w: [f64; 3],
}

/// No vector.
pub(crate) const NO_VECTOR: [f64; 3] = [f64::NAN; 3];

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

/// The part of `poly` that the flat clip planes and the borders of the
/// region cutting its cell flat keep (Sutherland and Hodgman), node
/// values and normals interpolated at the cuts.
fn clip_flat(poly: Vec<Vx>, planes: &[Bound], region: &[Bound]) -> Vec<Vx> {
    let mut v = poly;
    for plane in planes.iter().chain(region) {
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
                    w: lerp3(a.w, b.w, t),
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

    /// Indexed triangles (an arrow) in one colour and value `value`
    /// (NaN: none).
    fn mesh(
        &mut self,
        verts: &[([f64; 3], [f64; 3])],
        tris: &[[u32; 3]],
        value: f64,
        paint: &Paint,
    ) {
        let o = self.s.origin;
        let vo = self.s.value_origin;
        let valued = paint.flat.filter(|_| value.is_finite()).map(|_| value);
        let val = match valued {
            Some(f) => {
                self.span.add(f);
                [(f - vo) as f32, (f - vo) as f32, paint.sel_mix, 1.0]
            }
            None => [0.0, 0.0, paint.sel_mix, 0.0],
        };
        let base = self.s.positions.len() as u32;
        let mut vb = self.s.visible_bounds.unwrap_or_else(Bounds::empty);
        for (p, n) in verts {
            self.s
                .positions
                .push([0, 1, 2].map(|k| (p[k] - o[k]) as f32));
            self.s.normals.push(n.map(|x| x as f32));
            self.s.colors.push(paint.color);
            self.s.values.push(val);
            vb.add(*p);
        }
        self.s.visible_bounds = Some(vb);
        for t in tris {
            self.s.indices.extend(t.map(|i| base + i));
        }
        self.s
            .tri_cell
            .extend(std::iter::repeat_n(paint.cell, tris.len()));
        self.s
            .tri_face
            .extend(std::iter::repeat_n(NO_CELL, tris.len()));
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

/// The closed loops of a cell's section, each by the edges it crosses.
type Rings = Vec<Vec<(u32, u32)>>;

/// The section of cell `c` by a plane with node distances `dist`:
/// closed loops of cut points, each named by its edge (lower node
/// first). A node on the plane counts as behind it, as for clip planes.
fn section(mesh: &Mesh, c: usize, dist: &dyn Fn(u32) -> Option<f64>) -> Rings {
    let behind = |i: u32| dist(i).is_none_or(|d| d <= 0.0);
    let (mut any_in, mut any_out) = (false, false);
    for i in mesh.cell_points(c) {
        {
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
            let (a, b) = (f.at(i), f.at((i + 1) % n));
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
    let flat: Vec<Bound> = display
        .active_clips()
        .filter(|c| c.mode == ClipMode::Flat)
        .map(Bound::from)
        .collect();
    let regions = display
        .region_cells
        .as_deref()
        .filter(|rc| rc.cuts_flat() && rc.regions == display.regions);
    let borders = |c: u32| regions.map_or(&[][..], |rc| rc.borders_of(c as usize));
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
    // The arrows' field (its three components and magnitude), their grid
    // spacing and where they go.
    let phase = murmel_mesh::profile::span("extract.vector");
    let arrows = display.arrows.as_ref().filter(|a| a.enabled).and_then(|a| {
        let fields = fields?;
        let comp = |component| {
            fields.values_cached(
                mesh,
                &FieldChoice {
                    name: a.field.clone(),
                    component,
                },
            )
        };
        let vector = [
            comp(Component::X)?,
            comp(Component::Y)?,
            comp(Component::Z)?,
        ];
        // The largest magnitude, without the magnitude's node values.
        let longest = fields
            .range(
                mesh,
                &FieldChoice {
                    name: a.field.clone(),
                    component: Component::Magnitude,
                },
            )?
            .1;
        Some((a, vector, longest, a.spacing(mesh)))
    });
    drop(phase);
    let mut samples: Vec<crate::glyph::Sample> = Vec::new();
    let anchor = mesh.bounds().center();
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
            let vector = arrows.as_ref().map(|a| &a.1);
            let mut pieces: Vec<(u32, Vec<Vx>)> = Vec::new();
            let phase = murmel_mesh::profile::span("extract.slice");
            let (f0, e0) = (
                out.s.indices.len() as u32,
                out.s.edge_positions.len() as u32,
            );
            for t in level_set(mesh, fields, Level::Plane(&plane), field, vector, shown) {
                let verts: Vec<Vx> = t.verts.iter().map(|v| Vx { n: normal, ..*v }).collect();
                let verts = clip_flat(verts, &flat, borders(t.cell));
                out.poly(
                    &verts,
                    &Paint {
                        edge: None,
                        ..paint(t.cell as usize)
                    },
                );
                if arrows.is_some() {
                    pieces.push((t.cell, verts));
                }
            }
            drop(phase);
            out.s.slice_parts.push(SlicePart {
                slice: k as u32,
                faces: f0..out.s.indices.len() as u32,
                edges: e0..e0,
            });
            if let Some((a, _, _, _)) = &arrows {
                let h = a.plane_spacing(mesh, &plane);
                crate::glyph::on_plane(&mut samples, &plane, anchor, h, a.in_plane, &pieces);
            }
            // The outlines of the cells' sections.
            if !edges {
                continue;
            }
            let _phase = murmel_mesh::profile::span("extract.outline");
            let dist = |i: u32| mesh.points.get(i as usize).map(|&p| plane.distance(p));
            // The sections of the cells shown, found on all threads; of
            // the cells near the plane only ([`crate::bins`]).
            let ring_of = |c: usize| (c, section(mesh, c, &dist));
            let sections: Vec<(usize, Rings)> = match fields.filter(|_| crate::bins::on()) {
                Some(set) => {
                    let near = set.bins(mesh).near_plane(|x| plane.distance(x));
                    murmel_mesh::par::collect(near.len(), 1 << 10, |r| {
                        near[r]
                            .iter()
                            .map(|&c| c as usize)
                            .filter(|&c| shown.get(c).copied().unwrap_or(false))
                            .map(ring_of)
                            .filter(|(_, rings)| !rings.is_empty())
                            .collect()
                    })
                }
                None => murmel_mesh::par::collect(shown.len(), 1 << 12, |r| {
                    r.filter(|&c| shown[c])
                        .map(ring_of)
                        .filter(|(_, rings)| !rings.is_empty())
                        .collect()
                }),
            };
            for (c, rings) in sections {
                let Some(ec) = paint(c).edge else { continue };
                for ring in rings {
                    let verts: Vec<Vx> = ring
                        .iter()
                        .map(|&(a, b)| {
                            let (da, db) =
                                (dist(a).unwrap_or(f64::NAN), dist(b).unwrap_or(f64::NAN));
                            let t = if da == db { 0.0 } else { da / (da - db) };
                            Vx {
                                p: lerp3(mesh.points[a as usize], mesh.points[b as usize], t),
                                v: f64::NAN,
                                n: normal,
                                key: Some((a, b)),
                                w: NO_VECTOR,
                            }
                        })
                        .collect();
                    out.outline(&clip_flat(verts, &flat, borders(c as u32)), ec);
                }
            }
            if let Some(p) = out.s.slice_parts.last_mut() {
                p.edges.end = out.s.edge_positions.len() as u32;
            }
        }
    }
    let phase = murmel_mesh::profile::span("extract.iso");
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
                None,
                shown,
            );
            let f0 = out.s.indices.len() as u32;
            for t in tris {
                let paint = Paint {
                    color: face.linear(),
                    edge: None,
                    flat: colour.map(|f| f.flat(t.cell, NO_CELL)),
                    sel_mix: if picked { mix } else { 0.0 },
                    cell: t.cell,
                };
                let verts = clip_flat(t.verts.to_vec(), &flat, borders(t.cell));
                out.poly(&verts, &paint);
            }
            out.s.iso_parts.push(IsoPart {
                iso: k as u32,
                faces: f0..out.s.indices.len() as u32,
            });
        }
    }
    drop(phase);
    let _phase = murmel_mesh::profile::span("extract.arrows");
    if let Some((a, vector, longest, h)) = &arrows {
        if a.surface {
            let v = [&*vector[0], &*vector[1], &*vector[2]];
            crate::glyph::on_surface(&mut samples, mesh, display, shown, *h, v, field);
        }
        let picked = selected(display, Item::Arrows);
        let colour = match a.color {
            ArrowColor::Field => field,
            ArrowColor::Solid | ArrowColor::Magnitude => None,
        };
        let rgb = if colour.is_some() {
            tokens.mesh.field.none
        } else {
            display
                .colors
                .get(&Item::Arrows)
                .copied()
                .unwrap_or(tokens.mesh.field.arrow)
        };
        let size = |w: [f64; 3]| dot(w, w).sqrt();
        // The magnitude drawn 0.9 of a spacing long: the field's largest,
        // so the arrows keep their length while a slice moves; with the
        // part in the plane only, the largest of the arrows drawn (the
        // part in a plane is often a small fraction of the whole).
        let longest = if a.in_plane {
            samples
                .iter()
                .map(|x| size(x.w))
                .filter(|m| m.is_finite())
                .fold(0.0, f64::max)
        } else {
            *longest
        };
        out.s.arrow_longest = longest;
        // The arrows drawn, with their magnitude and length.
        let mut drawn: Vec<(&crate::glyph::Sample, f64, f64)> = Vec::new();
        for x in &samples {
            let m = size(x.w);
            // No value, or no flow to point along (noise below a
            // millionth of the largest).
            if !m.is_finite() || m <= 1e-6 * longest || longest <= 0.0 {
                continue;
            }
            let len = a.length.share(m, longest) * 0.9 * x.h * a.scale;
            // Too short to see.
            if len <= 0.02 * x.h {
                continue;
            }
            if drawn.len() == crate::glyph::MAX_ARROWS {
                out.s.arrows_capped = true;
                break;
            }
            drawn.push((x, m, len));
        }
        let range = drawn
            .iter()
            .fold(None, |r: Option<(f64, f64)>, &(_, m, _)| {
                Some(r.map_or((m, m), |(lo, hi)| (lo.min(m), hi.max(m))))
            });
        out.s.arrow_range = range;
        // Coloured by their own magnitude: the colours here, from their
        // field's style over its range.
        let own = crate::glyph::color_range(display, mesh, fields, range)
            .and_then(|r| crate::glyph::OwnColors::new(display, a, r));
        let a0 = out.s.indices.len() as u32;
        for &(x, m, len) in &drawn {
            let dir = x.w.map(|c| c / m);
            let (verts, tris) = crate::glyph::arrow(x.centre(dir, len, x.h), dir, len, x.h);
            let rgb = own.as_ref().map_or(rgb, |o| o.of(m));
            let face = if picked { mixed(rgb, sel, mix) } else { rgb };
            let paint = Paint {
                color: face.linear(),
                edge: None,
                flat: colour.map(|_| x.v),
                sel_mix: if picked { mix } else { 0.0 },
                cell: x.cell,
            };
            out.mesh(&verts, &tris, x.v, &paint);
        }
        out.s.arrows = drawn.len();
        out.s.arrow_faces = a0..out.s.indices.len() as u32;
    }
    let span = out.span.0;
    if let Some((lo, hi)) = span {
        s.field_range = Some(match s.field_range {
            Some((a, b)) => (a.min(lo), b.max(hi)),
            None => (lo, hi),
        });
    }
}

pub(crate) fn mixed(a: Rgb, b: Rgb, mix: f32) -> Rgb {
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
    vector: Option<&[std::sync::Arc<FieldValues>; 3]>,
    shown: &[bool],
) -> Vec<Tri> {
    // The values carried to the surface's points: the colour, then the
    // vector's components (NaN where none).
    let chans: [Option<&FieldValues>; 4] = [
        colour,
        vector.map(|v| &*v[0]),
        vector.map(|v| &*v[1]),
        vector.map(|v| &*v[2]),
    ];
    let iso = match level {
        Level::Field(_, v) => v,
        Level::Plane(_) => 0.0,
    };
    if !iso.is_finite() {
        return Vec::new();
    }
    let node_of = crate::tets::node_value;
    let node_level = |n: u32| match level {
        Level::Field(v, _) => node_of(v, n),
        Level::Plane(p) => p.distance(mesh.points[n as usize]),
    };
    // An iso-surface's normals follow the field's slope ([`crate::
    // gradient`]) where it has one, so the shading is smooth across the
    // facets; a slice's and a field's without gradients follow the
    // facets.
    let slope = match (level, fields) {
        (Level::Field(v, _), Some(set)) if !v.grad.is_empty() => Some((v, set)),
        _ => None,
    };
    // Which cells the level set crosses, with the value at each one's
    // centroid, on all threads: the bounds of the values a cell's
    // tetrahedra can take, and the value at its centroid.
    let bounds = |c: usize| -> (f64, f64, f64) {
        // Quick test: the cell's values all on one side.
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut add = |n: u32| {
            let v = node_level(n);
            lo = lo.min(v);
            hi = hi.max(v);
        };
        match fields {
            Some(set) => set.nodes_of_cell(mesh, c).iter().for_each(|&n| add(n)),
            None => mesh.cell_points(c).for_each(add),
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
        // A face centre's value lies between its cells' and its nodes'
        // ([`crate::tets::face_value`]), so the neighbours bound it too:
        // a cell passed over here while a face centre crosses would
        // leave a hole.
        if let Level::Field(v, _) = level {
            for f in mesh.cell(c) {
                let u = f.id as usize;
                if u >= mesh.face_owner.len() {
                    continue;
                }
                let other = mesh.across(u, c as u32);
                if other != NO_CELL
                    && let Some(&x) = v.cell.get(other as usize)
                    && x.is_finite()
                {
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
        }
        (lo, hi, vc)
    };
    let crosses = |(lo, hi): (f64, f64)| lo.is_finite() && hi.is_finite() && lo != hi;
    let crossing = |c: usize| -> Option<f64> {
        let (lo, hi, vc) = bounds(c);
        (crosses((lo, hi)) && iso >= lo && iso <= hi).then_some(vc)
    };
    let phase = murmel_mesh::profile::span("extract.crossed");
    // Only the cells of the bins near the level set ([`crate::bins`]);
    // all of them without fields.
    let near: Option<Vec<u32>> = fields.filter(|_| crate::bins::on()).map(|set| {
        let bins = set.bins(mesh);
        match level {
            Level::Plane(p) => bins.near_plane(|x| p.distance(x)),
            Level::Field(v, iso) => {
                let spans = v.spans.get_or_init(|| {
                    bins.spans(|c| {
                        let (lo, hi, _) = bounds(c);
                        crosses((lo, hi)).then_some((lo, hi))
                    })
                });
                bins.near_value(spans, iso)
            }
        }
    });
    let crossed: Vec<(u32, f64)> = match &near {
        Some(near) => murmel_mesh::par::collect(near.len(), 1 << 10, |r| {
            near[r]
                .iter()
                .filter(|&&c| shown.get(c as usize).copied().unwrap_or(false))
                .filter_map(|&c| crossing(c as usize).map(|vc| (c, vc)))
                .collect()
        }),
        None => murmel_mesh::par::collect(shown.len(), 1 << 12, |r| {
            r.filter(|&c| shown[c])
                .filter_map(|c| crossing(c).map(|vc| (c as u32, vc)))
                .collect()
        }),
    };
    // The crossed cells cut into their triangles, run by run on all
    // threads; each run names its points by the segment they lie on.
    let cut = |run: std::ops::Range<usize>| -> Piece {
        let mut piece = Piece::default();
        let mut made: murmel_mesh::fasthash::FastMap<(Pt, Pt), u32> =
            murmel_mesh::fasthash::FastMap::default();
        for &(c, vc) in &crossed[run] {
            cut_cell(
                &mut piece,
                &mut made,
                c as usize,
                vc,
                &Cutting {
                    mesh,
                    fields,
                    level,
                    iso,
                    chans,
                    slope,
                },
            );
        }
        piece
    };
    drop(phase);
    let phase = murmel_mesh::profile::span("extract.cut");
    let pieces = murmel_mesh::par::map(crossed.len(), 64, cut);
    drop(phase);
    let _phase = murmel_mesh::profile::span("extract.join");
    // Joined in cell order, as one thread would have made them: a point
    // two runs made is the first run's, and each triangle's normal and
    // side are found from the points kept.
    let mut made: murmel_mesh::fasthash::FastMap<(Pt, Pt), u32> =
        murmel_mesh::fasthash::FastMap::default();
    let total: usize = pieces.iter().map(|p| p.points.len()).sum();
    let mut points: Vec<([f64; 3], [f64; 4])> = Vec::with_capacity(total);
    let mut slopes: Vec<[f64; 3]> = Vec::new();
    let mut normals: Vec<[f64; 3]> = Vec::with_capacity(total);
    let mut tris: Vec<([u32; 3], u32)> = Vec::new();
    for piece in pieces {
        let mut to = Vec::with_capacity(piece.points.len());
        for (i, key) in piece.keys.iter().enumerate() {
            let mut add = || {
                points.push(piece.points[i]);
                normals.push([0.0; 3]);
                if let Some(&g) = piece.slopes.get(i) {
                    slopes.push(g);
                }
                (points.len() - 1) as u32
            };
            // A segment from a centroid lies in its cell alone.
            let g = if matches!(key.1, Pt::Cell(_)) {
                add()
            } else {
                *made.entry(*key).or_insert_with(add)
            };
            to.push(g);
        }
        for (t, cell, up) in piece.tris {
            let t = t.map(|i| to[i as usize]);
            let (p0, p1, p2) = (
                points[t[0] as usize].0,
                points[t[1] as usize].0,
                points[t[2] as usize].0,
            );
            let n = cross(sub(p1, p0), sub(p2, p0));
            // Turn towards growing values.
            let (t, n) = if dot(n, up) < 0.0 {
                ([t[0], t[2], t[1]], n.map(|x| -x))
            } else {
                (t, n)
            };
            if dot(n, n) == 0.0 {
                continue;
            }
            for &i in &t {
                let m = &mut normals[i as usize];
                *m = [m[0] + n[0], m[1] + n[1], m[2] + n[2]];
            }
            tris.push((t, cell));
        }
    }
    tris.into_iter()
        .map(|(t, cell)| Tri {
            cell,
            verts: t.map(|i| {
                let (p, v) = points[i as usize];
                // The slope where it is clear and agrees with the
                // facets' side, else the facets' mean.
                let facets = normals[i as usize];
                let n = match slopes.get(i as usize) {
                    Some(&g) if dot(g, g) > 0.0 && dot(g, facets) > 0.0 => g,
                    _ => facets,
                };
                Vx {
                    p,
                    v: v[0],
                    n: unit(n),
                    key: None,
                    w: [v[1], v[2], v[3]],
                }
            }),
        })
        .collect()
}

/// The points and triangles of a run of crossed cells: each point with
/// the segment it lies on (`keys`), its position and channel values and,
/// for an iso-surface with gradients, the field's slope there; each
/// triangle by its points, with its cell and the direction of growing
/// values it should face.
#[derive(Default)]
struct Piece {
    keys: Vec<(Pt, Pt)>,
    points: Vec<([f64; 3], [f64; 4])>,
    slopes: Vec<[f64; 3]>,
    tris: Vec<([u32; 3], u32, [f64; 3])>,
}

/// What cutting a cell needs to know of the level set.
struct Cutting<'a> {
    mesh: &'a Mesh,
    fields: Option<&'a FieldSet>,
    level: Level<'a>,
    iso: f64,
    chans: [Option<&'a FieldValues>; 4],
    slope: Option<(&'a FieldValues, &'a FieldSet)>,
}

/// Cut cell `c` (`vc` the level at its centroid) into its tetrahedra
/// and those by the level set, into `piece` (`made` names the points
/// made in it).
fn cut_cell(
    piece: &mut Piece,
    made: &mut murmel_mesh::fasthash::FastMap<(Pt, Pt), u32>,
    c: usize,
    vc: f64,
    k: &Cutting<'_>,
) {
    let Cutting {
        mesh,
        fields,
        level,
        iso,
        chans,
        slope,
    } = *k;
    let node_of = crate::tets::node_value;
    let face_value = |f: &FieldValues, u: u32, face: &[u32], fc: [f64; 3]| -> f64 {
        crate::tets::face_value(mesh, fields, f, u, face, fc)
    };
    let node_level = |n: u32| match level {
        Level::Field(v, _) => node_of(v, n),
        Level::Plane(p) => p.distance(mesh.points[n as usize]),
    };
    let node_slope =
        |n: u32| slope.map_or([0.0; 3], |(v, set)| set.node_gradient(mesh, &v.grad, n));
    let centre = crate::tets::centroid(mesh, fields, c);
    let vc = match level {
        Level::Plane(p) => p.distance(centre),
        Level::Field(..) => vc,
    };
    let cc = chans.map(|ch| ch.map_or(f64::NAN, |f| f.cell.get(c).copied().unwrap_or(f64::NAN)));
    for f in mesh.cell(c) {
        if f.len() < 3 {
            continue;
        }
        let u = f.id;
        let fc = crate::tets::face_centre(mesh, f.nodes);
        let vf = match level {
            Level::Field(v, _) => face_value(v, u, f.nodes, fc),
            Level::Plane(p) => p.distance(fc),
        };
        let cf = chans.map(|ch| ch.map_or(f64::NAN, |cv| face_value(cv, u, f.nodes, fc)));
        // A face without a distinct id (should not happen) is made
        // per cell.
        let fpt = if u == NO_CELL {
            Pt::Cell(u32::MAX - c as u32)
        } else {
            Pt::Face(u)
        };
        for j in 0..f.len() {
            let (a, b) = (f.at(j), f.at((j + 1) % f.len()));
            let tet = [
                (Pt::Cell(c as u32), centre, vc, cc),
                (fpt, fc, vf, cf),
                (
                    Pt::Node(a),
                    mesh.points[a as usize],
                    node_level(a),
                    chans.map(|ch| ch.map_or(f64::NAN, |cv| node_of(cv, a))),
                ),
                (
                    Pt::Node(b),
                    mesh.points[b as usize],
                    node_level(b),
                    chans.map(|ch| ch.map_or(f64::NAN, |cv| node_of(cv, b))),
                ),
            ];
            // The slope at each (made only for a crossed edge's ends):
            // a node's from its cells, a centroid's and a face centre's
            // the mean of their nodes', so one cell's gradient never
            // creases the shading on its own.
            let mean_slope = |ns: &mut dyn Iterator<Item = u32>| {
                let (mut k, mut g) = (0.0, [0.0; 3]);
                for n in ns {
                    let s = node_slope(n);
                    g = [g[0] + s[0], g[1] + s[1], g[2] + s[2]];
                    k += 1.0;
                }
                if k > 0.0 { g.map(|x| x / k) } else { g }
            };
            let slope_at = |q: usize| match (slope, q) {
                (None, _) => [0.0; 3],
                (_, 0) => mean_slope(&mut mesh.cell_points(c)),
                (_, 1) => mean_slope(&mut f.nodes.iter().copied()),
                (_, 2) => node_slope(a),
                _ => node_slope(b),
            };
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
                    let (a, b) = (tet[lo].3, tet[hi].3);
                    piece.keys.push((tet[lo].0, tet[hi].0));
                    piece.points.push((
                        lerp3(tet[lo].1, tet[hi].1, t),
                        std::array::from_fn(|k| lerp(a[k], b[k], t)),
                    ));
                    if slope.is_some() {
                        piece.slopes.push(lerp3(slope_at(lo), slope_at(hi), t));
                    }
                    (piece.points.len() - 1) as u32
                })
            };
            // One point above (or below): a triangle; two each: a quad
            // of the four crossed edges.
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
            // Towards growing values: from the mean of the points below
            // to the mean of those above.
            let mean = |qs: &[usize]| {
                let n = qs.len() as f64;
                [0, 1, 2].map(|d| qs.iter().map(|&q| tet[q].1[d]).sum::<f64>() / n)
            };
            let up = sub(mean(&above), mean(&below));
            piece
                .tris
                .push(([corners[0], corners[1], corners[2]], c as u32, up));
            if corners.len() == 4 {
                piece
                    .tris
                    .push(([corners[0], corners[2], corners[3]], c as u32, up));
            }
        }
    }
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

    /// The honeycomb with a scalar `p` and a swirling vector `U` in its
    /// cells (as `murmel bench` gives every mesh).
    fn honeycomb_with_fields(n: usize) -> (Mesh, FieldSet) {
        let mesh = murmel_mesh::synth::honeycomb(n);
        let b = mesh.bounds();
        let (c, h) = (b.center(), [0, 1, 2].map(|k| 0.5 * (b.max[k] - b.min[k])));
        let at: Vec<[f64; 3]> = (0..mesh.cell_count())
            .map(|i| {
                let q = crate::inspect::cell_centroid(&mesh, i);
                [0, 1, 2].map(|k| (q[k] - c[k]) / h[k])
            })
            .collect();
        let p = at
            .iter()
            .map(|q| (3.0 * q[0]).sin() * (2.0 * q[1]).cos() + 0.5 * q[2])
            .collect();
        let u = at
            .iter()
            .flat_map(|q| [-q[1], q[0], 0.2 + 0.1 * q[2]])
            .collect();
        let f = |name: &str, components, values| murmel_mesh::field::Field {
            name: name.into(),
            unit: None,
            location: murmel_mesh::field::Location::Cell,
            components,
            values,
        };
        let set = FieldSet::new(murmel_mesh::field::Results {
            fields: vec![f("p", 1, p), f("U", 3, u)],
            ..Default::default()
        });
        (mesh, set)
    }

    /// Invariant (fields on large meshes, docs/performance.md): slices
    /// and iso-surfaces cut on all threads, testing only the cells of the
    /// bins near them ([`crate::bins`]), are the same, bit for bit, as
    /// cut on one thread testing every cell: faces, values, normals,
    /// outlines, arrows. Breaks when the bins leave out a crossed cell (a
    /// hole), when runs joined out of order number points differently,
    /// or when a point two runs make is kept twice. The other tests look
    /// at what a slice covers, not at how it was cut.
    #[test]
    fn cut_on_threads_near_the_bins_as_on_one_thread() {
        let (m, set) = honeycomb_with_fields(16);
        let b = m.bounds();
        let c = b.center();
        let mut d = by_field(&m, &set, "p");
        let mut series = Slice::new(
            [c[0] + 0.013 * (b.max[0] - b.min[0]), c[1], c[2]],
            [1.0, 0.0, 0.0],
        );
        series.count = 3;
        series.spacing = 0.2 * (b.max[0] - b.min[0]);
        d.slices = vec![series, Slice::new(c, [0.3, 0.8, 0.52])];
        let iso = |name: &str, value, color| Iso {
            field: FieldChoice {
                name: name.into(),
                component: Component::Magnitude,
            },
            value,
            enabled: true,
            color,
        };
        d.isos = vec![
            iso("p", 0.25, IsoColor::Field),
            iso("U", 0.6, IsoColor::Solid),
        ];
        d.arrows = Some(crate::glyph::Arrows::new("U"));
        d.clips = vec![Clip {
            origin: c,
            normal: [0.0, 0.0, 1.0],
            mode: ClipMode::Flat,
            enabled: true,
        }];
        let one =
            murmel_mesh::par::with_threads(1, || crate::bins::without(|| build(&m, &set, &d)));
        let many = murmel_mesh::par::with_threads(5, || build(&m, &set, &d));
        assert!(one.triangles() > 1000 && one.edges() > 1000 && one.arrows > 0);
        assert_eq!(one.positions, many.positions);
        assert_eq!(one.normals, many.normals);
        assert_eq!(one.values, many.values);
        assert_eq!(one.indices, many.indices);
        assert_eq!(one.tri_cell, many.tri_cell);
        assert_eq!(one.edge_positions, many.edge_positions);
        assert_eq!(one.edge_colors, many.edge_colors);
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

    /// Invariant (large-mesh stage 1b): a region cutting flat clips the
    /// slices of its cells to its border, as a flat clip plane does: a
    /// slab of width w across the duct leaves a slice w wide, exactly,
    /// and a sphere keeps its slice inside it; with whole cells the slice
    /// covers the slab's cells, wider than w. Breaks when slices ignore
    /// the regions' flat cut (they would poke out of the cut cells).
    #[test]
    fn flat_regions_cut_slices() {
        use crate::region::{Region, Shape};
        let (m, set) = channel();
        let b = m.bounds();
        let c = b.center();
        let w = 0.1 * (b.max[0] - b.min[0]);
        let slab = Shape::Slab {
            origin: c,
            normal: [1.0, 0.0, 0.0],
            thickness: w,
        };
        // Large enough to hold nodes of the coarse duct (else it shows
        // whole cells).
        let sphere = Shape::Sphere {
            center: c,
            radius: 0.45 * (b.max[1] - b.min[1]),
        };
        let depth = b.max[2] - b.min[2];
        let slice_area = |shape: &Shape, cut: ClipMode| {
            let mut d = by_field(&m, &set, "speed");
            d.slices.push(Slice::new(c, [0.0, 1.0, 0.0]));
            d.regions = vec![Region {
                cut,
                ..Region::new(shape.clone())
            }];
            crate::region::ensure(&mut d, &m);
            assert_eq!(d.region_cells.as_ref().unwrap().whole_instead, [false]);
            let tris = extract_tris(&build(&m, &set, &d));
            assert!(!tris.is_empty(), "{shape:?} {cut:?}");
            if cut == ClipMode::Flat {
                for bd in shape.borders() {
                    assert!(
                        tris.iter().flatten().all(|&p| bd.distance(p) <= 1e-6),
                        "{shape:?} pokes out"
                    );
                }
            }
            tris.iter().map(area).sum::<f64>()
        };
        let flat = slice_area(&slab, ClipMode::Flat);
        assert!(
            (flat - w * depth).abs() < 1e-3 * w * depth,
            "{flat} vs {}",
            w * depth
        );
        let whole = slice_area(&slab, ClipMode::Crinkle);
        assert!(whole > flat * 1.01, "whole cells {whole} vs flat {flat}");
        slice_area(&sphere, ClipMode::Flat);
    }

    /// A box of `n`³ hexahedra with its inner points moved by up to a
    /// third of a cell (a fixed pattern), as a mesher's cells are
    /// uneven; written as a `.vtu` and read back.
    fn uneven_box(n: usize) -> Mesh {
        let h = 1.0 / n as f64;
        let id = |i: usize, j: usize, k: usize| (k * (n + 1) + j) * (n + 1) + i;
        let mut pts = String::new();
        for k in 0..=n {
            for j in 0..=n {
                for i in 0..=n {
                    let inner = |q: usize| q > 0 && q < n;
                    let jig = |s: usize| {
                        let x = ((id(i, j, k) * 2654435761 + s * 40503) % 1000) as f64;
                        (x / 1000.0 - 0.5) * 0.66 * h
                    };
                    let p = [(i, 0), (j, 1), (k, 2)]
                        .map(|(q, s)| q as f64 * h + if inner(q) { jig(s) } else { 0.0 });
                    pts.push_str(&format!("{} {} {} ", p[0], p[1], p[2]));
                }
            }
        }
        let (mut conn, mut offs, mut types) = (String::new(), String::new(), String::new());
        let mut c = 0;
        for k in 0..n {
            for j in 0..n {
                for i in 0..n {
                    for (a, b, d) in [
                        (0, 0, 0),
                        (1, 0, 0),
                        (1, 1, 0),
                        (0, 1, 0),
                        (0, 0, 1),
                        (1, 0, 1),
                        (1, 1, 1),
                        (0, 1, 1),
                    ] {
                        conn.push_str(&format!("{} ", id(i + a, j + b, k + d)));
                    }
                    c += 8;
                    offs.push_str(&format!("{c} "));
                    types.push_str("12 ");
                }
            }
        }
        let xml = format!(
            r#"<?xml version="1.0"?>
<VTKFile type="UnstructuredGrid" version="1.0" byte_order="LittleEndian">
<UnstructuredGrid><Piece NumberOfPoints="{}" NumberOfCells="{}">
<Points><DataArray type="Float64" NumberOfComponents="3" format="ascii">{pts}</DataArray></Points>
<Cells>
<DataArray type="Int64" Name="connectivity" format="ascii">{conn}</DataArray>
<DataArray type="Int64" Name="offsets" format="ascii">{offs}</DataArray>
<DataArray type="UInt8" Name="types" format="ascii">{types}</DataArray>
</Cells></Piece></UnstructuredGrid></VTKFile>
"#,
            (n + 1).pow(3),
            n.pow(3)
        );
        let dir = std::env::temp_dir().join(format!("murmel-uneven-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("uneven.vtu");
        std::fs::write(&path, xml).unwrap();
        let m = murmel_mesh::read(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        m
    }

    /// Invariant: an iso-surface closes and never folds: the duct's speed
    /// at 1.2 through the one large cell in its core leaves no hole
    /// (every edge of it is shared by two triangles except on the
    /// duct's sides), and no triangle faces against the normals of its
    /// corners (the shading would draw it dark). Breaks when a cell whose
    /// face centre crosses the value is passed over (its neighbour's
    /// value bounds that centre, so the quick test must count it: holes
    /// in the tube), or a boundary face's centre stops following its
    /// cell (the core's inlet and outlet faces took the mean of their
    /// rims, too low there, and capped the tube with folded triangles).
    #[test]
    fn iso_surface_closes_without_folds() {
        let (m, set) = channel();
        let b = m.bounds();
        let mut d = by_field(&m, &set, "velocity");
        d.isos.push(Iso {
            field: FieldChoice {
                name: "speed".into(),
                component: Component::Magnitude,
            },
            value: 1.2,
            enabled: true,
            color: IsoColor::Solid,
        });
        let s = build(&m, &set, &d);
        let mut seen = 0;
        for (t, tri) in s.indices.as_chunks::<3>().0.iter().enumerate() {
            if s.tri_face[t] != NO_CELL {
                continue;
            }
            seen += 1;
            let p = tri.map(|i| {
                let q = s.positions[i as usize];
                [0, 1, 2].map(|k| q[k] as f64 + s.origin[k])
            });
            let facet = unit(cross(sub(p[1], p[0]), sub(p[2], p[0])));
            for &i in tri {
                let n = unit(s.normals[i as usize].map(f64::from));
                assert!(dot(facet, n) > 0.0, "a triangle at {:?} folds", p[0]);
            }
        }
        assert!(seen > 100, "{seen} triangles");
        let key = |p: [f64; 3]| p.map(|v| (v * 1e5).round() as i64);
        let mut count: HashMap<([i64; 3], [i64; 3]), usize> = HashMap::new();
        for t in extract_tris(&s) {
            for j in 0..3 {
                let (a, c) = (key(t[j]), key(t[(j + 1) % 3]));
                *count.entry((a.min(c), a.max(c))).or_default() += 1;
            }
        }
        let on_side = |p: [i64; 3]| {
            let p = p.map(|v| v as f64 * 1e-5);
            (0..3).any(|k| (p[k] - b.min[k]).abs() < 1e-4 || (p[k] - b.max[k]).abs() < 1e-4)
        };
        for ((a, c), n) in count {
            assert!(
                n == 2 || (on_side(a) && on_side(c)),
                "a hole: an edge used {n} times at {a:?}"
            );
        }
    }

    /// Invariant: the iso-surface of a smooth round field (the distance
    /// from a point off the middle, at the centroids of 20³ uneven
    /// hexahedra; the sphere spans 12 cells) is smooth and round: its
    /// vertex normals, which the shading shows, lie within 2° of the
    /// true ones on average and 4° for 95 % of them, and its points
    /// within 0.6 % of the radius, RMS. Measured: 1.0°, 2.2° and 0.43 %
    /// with values rebuilt from cell gradients and normals from the
    /// field's slope; the facets' own normals were about three times as
    /// far off, and inverse distance weighting gave 9.2°, 18.9° and
    /// 1.5 %, the waves that looked rough. Breaks when the values between
    /// cells stop following the field's slope or the normals go back to
    /// the facets.
    #[test]
    fn iso_surface_of_a_round_field_is_smooth() {
        use murmel_mesh::field::{Field, Location, Results};
        let m = uneven_box(20);
        let c = [0.52, 0.47, 0.49];
        let dist = |x: [f64; 3]| dot(sub(x, c), sub(x, c)).sqrt();
        let values: Vec<f64> = (0..m.cell_count())
            .map(|k| dist(crate::inspect::cell_centroid(&m, k)))
            .collect();
        let set = FieldSet::new(Results {
            fields: vec![Field {
                name: "r".into(),
                unit: None,
                location: Location::Cell,
                components: 1,
                values,
            }],
            ..Results::default()
        });
        let mut d = by_field(&m, &set, "r");
        let radius = 0.3;
        d.isos.push(Iso {
            field: FieldChoice {
                name: "r".into(),
                component: Component::Magnitude,
            },
            value: radius,
            enabled: true,
            color: IsoColor::Solid,
        });
        let s = build(&m, &set, &d);
        let (mut off, mut angles) = (Vec::new(), Vec::new());
        for (t, tri) in s.indices.as_chunks::<3>().0.iter().enumerate() {
            if s.tri_face[t] != NO_CELL {
                continue;
            }
            for &i in tri {
                let p = s.positions[i as usize];
                let x = [0, 1, 2].map(|k| p[k] as f64 + s.origin[k]);
                off.push(dist(x) / radius - 1.0);
                let n = s.normals[i as usize].map(f64::from);
                let along = dot(unit(sub(x, c)), unit(n)).abs().min(1.0);
                angles.push(along.acos().to_degrees());
            }
        }
        assert!(!off.is_empty());
        let rms = (off.iter().map(|e| e * e).sum::<f64>() / off.len() as f64).sqrt();
        angles.sort_by(f64::total_cmp);
        let mean = angles.iter().sum::<f64>() / angles.len() as f64;
        let p95 = angles[angles.len() * 95 / 100];
        assert!(rms < 0.006, "RMS {rms}");
        assert!(
            mean < 2.0 && p95 < 4.0,
            "normals off by {mean}° on average, {p95}° at 95 %"
        );
    }

    /// Invariant: the iso-surface of a field linear in x (the pressure,
    /// 50 - 100 x/L at the cell centres) is exactly the plane across the
    /// duct where the field crosses the value: every point on it, its
    /// area the section's (values rebuilt from cell gradients; inverse
    /// distance weighting made it wave, 2.3 % more area), its normals
    /// pointing to growing pressure (-x), and it closes across cells:
    /// every edge of it is shared by two triangles except on the duct's
    /// walls. Breaks when a tetrahedron case is wrong (holes), points are
    /// made twice with different positions (cracks), the orientation is
    /// not turned to the gradient, or the values between cells stop
    /// following a linear field.
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
            tris.iter().flatten().all(|p| (p[0] - x).abs() < 1e-6 * len),
            "points off the plane x = {x}"
        );
        let a: f64 = tris.iter().map(area).sum();
        let section = (b.max[1] - b.min[1]) * (b.max[2] - b.min[2]);
        assert!((a - section).abs() < 1e-6 * section, "{a} vs {section}");
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

    /// The arrows of `s` (drawn last, one block of vertices each) as
    /// (tail, tip) world points.
    fn arrows_of(s: &Surface) -> Vec<([f64; 3], [f64; 3])> {
        let each = crate::glyph::arrow([0.0; 3], [1.0, 0.0, 0.0], 1.0, 1.0)
            .0
            .len();
        // The cone's first triangle ends at the tip.
        let tip = each - 3 * crate::glyph::SIDES + 2;
        let world = |i: usize| {
            let p = s.positions[i];
            [0, 1, 2].map(|k| p[k] as f64 + s.origin[k])
        };
        let first = s.positions.len() - s.arrows * each;
        (0..s.arrows)
            .map(|k| (world(first + k * each), world(first + k * each + tip)))
            .collect()
    }

    fn velocity_arrows() -> crate::glyph::Arrows {
        crate::glyph::Arrows::new("velocity")
    }

    /// Invariant: arrows on a slice sit on the even grid of the plane
    /// (anchored at the mesh's centre, one spacing apart: the slice's
    /// section of the box over the density, 40 by default), one per grid
    /// point, centred on it; with length mode magnitude their length is
    /// the magnitude over the field's largest, times 0.9 of a spacing
    /// times the scale; they point along the vector (the duct's flow,
    /// along x). With a floor (the default) none is shorter than a
    /// quarter of the longest, with equal lengths all are 0.9 of a
    /// spacing. Breaks when arrows follow the cells (crowding where the
    /// mesh is fine), when a grid point is drawn twice (it lies on an
    /// edge between two cells' pieces) or when the length stops
    /// following the magnitude.
    #[test]
    fn arrows_on_a_slice_follow_the_grid_and_the_magnitude() {
        let (m, set) = channel();
        let b = m.bounds();
        let mut d = by_field(&m, &set, "speed");
        let mut slice = Slice::new(b.center(), [0.0, 0.0, 1.0]);
        slice.origin[2] += 0.013;
        d.slices.push(slice);
        let mut a = velocity_arrows();
        a.length = crate::glyph::Length::Magnitude;
        d.arrows = Some(a.clone());
        let s = build(&m, &set, &d);
        let plane = Clip {
            origin: slice.origin,
            normal: slice.normal,
            mode: ClipMode::Flat,
            enabled: true,
        };
        let h = a.plane_spacing(&m, &plane);
        // The duct is 4 long: 40 arrows along it.
        assert!((h - 4.0 / 40.0).abs() < 1e-12, "{h}");
        let arrows = arrows_of(&s);
        // One per grid point over x and y, a few left out at the walls
        // (too short to see).
        let most = (((b.max[0] - b.min[0]) / h).floor() as usize + 1)
            * (((b.max[1] - b.min[1]) / h).floor() as usize + 1);
        assert!(
            (most * 3 / 4..=most).contains(&arrows.len()),
            "{} arrows of {most}",
            arrows.len()
        );
        assert!(!s.arrows_capped);
        let (u, v) = crate::clip::plane_axes([0.0, 0.0, 1.0]);
        let anchor = b.center();
        let mut seen = HashSet::new();
        let mut longest: f64 = 0.0;
        let length = |t: &[f64; 3], p: &[f64; 3]| dot(sub(*p, *t), sub(*p, *t)).sqrt();
        for (tail, tip) in &arrows {
            let c = [0, 1, 2].map(|k| 0.5 * (tail[k] + tip[k]));
            assert!(
                (c[2] - slice.origin[2]).abs() < 1e-4,
                "off the plane: {c:?}"
            );
            let g = [u, v].map(|e| dot(sub(c, anchor), e) / h);
            for x in g {
                assert!((x - x.round()).abs() < 1e-3, "off the grid: {g:?}");
            }
            assert!(seen.insert(g.map(|x| x.round() as i64)), "twice: {g:?}");
            let d = sub(*tip, *tail);
            let len = length(tail, tip);
            assert!(len <= 0.9 * h * (1.0 + 1e-4), "{len}");
            longest = longest.max(len);
            // The core flow (not the slow eddies at the walls).
            if len > 0.5 * 0.9 * h {
                assert!(d[0] / len > 0.99, "not along the flow: {d:?}");
            }
        }
        assert!(longest > 0.75 * 0.9 * h, "{longest}");
        let shortest = arrows
            .iter()
            .map(|(t, p)| length(t, p))
            .fold(f64::INFINITY, f64::min);
        assert!(shortest < 0.2 * 0.9 * h, "{shortest}");
        // Twice the scale, twice the length.
        d.arrows.as_mut().unwrap().scale = 2.0;
        let s2 = build(&m, &set, &d);
        let longest2 = arrows_of(&s2)
            .iter()
            .map(|(a, b)| length(a, b))
            .fold(0.0, f64::max);
        assert!((longest2 - 2.0 * longest).abs() < 1e-4, "{longest2}");
        d.arrows.as_mut().unwrap().scale = 1.0;
        // A floor: the slow arrows a quarter of the longest; equal: all
        // the same, and more of them (none too short to see).
        d.arrows.as_mut().unwrap().length = crate::glyph::Length::Floor;
        let floor = arrows_of(&build(&m, &set, &d));
        assert!(floor.len() >= arrows.len());
        for (t, p) in &floor {
            let len = length(t, p);
            assert!(len >= 0.25 * 0.9 * h * (1.0 - 1e-4), "{len}");
        }
        d.arrows.as_mut().unwrap().length = crate::glyph::Length::Equal;
        for (t, p) in &arrows_of(&build(&m, &set, &d)) {
            assert!((length(t, p) - 0.9 * h).abs() < 1e-4);
        }
        // Off: none.
        d.arrows.as_mut().unwrap().enabled = false;
        assert_eq!(build(&m, &set, &d).arrows, 0);
    }

    /// Invariant: arrows coloured by their own magnitude (the default)
    /// take their field's colour map over its magnitude's range, one
    /// colour per arrow and no field value (so the mesh's colour bar does
    /// not recolour them): the fastest at the map's top, the slowest
    /// near its bottom; solid arrows share one colour. Breaks when own
    /// colours are left to the mesh's field paint or ignore the style.
    #[test]
    fn arrows_coloured_by_their_magnitude() {
        let (m, set) = channel();
        let b = m.bounds();
        let mut d = by_field(&m, &set, "speed");
        d.slices.push(Slice::new(b.center(), [0.0, 0.0, 1.0]));
        d.arrows = Some(velocity_arrows());
        let s = build(&m, &set, &d);
        assert!(s.arrows > 0);
        let each = crate::glyph::arrow([0.0; 3], [1.0, 0.0, 0.0], 1.0, 1.0)
            .0
            .len();
        let first = s.positions.len() - s.arrows * each;
        let colours: Vec<[f32; 3]> = (0..s.arrows).map(|k| s.colors[first + k * each]).collect();
        assert!((first..s.positions.len()).all(|i| s.values[i][3] == 0.0));
        let range = crate::glyph::color_range(&d, &m, Some(&set), s.arrow_range).unwrap();
        let style = d.style_of("velocity");
        let map = crate::field::map_for(&style, range).unwrap();
        let top = map.sample(1.0, false).linear();
        let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 0.02);
        let fastest = s.arrow_range.unwrap().1;
        assert!(fastest > 0.9 * range.1, "{fastest} of {range:?}");
        assert!(colours.iter().any(|c| {
            near(
                *c,
                map.sample((fastest - range.0) / (range.1 - range.0), false)
                    .linear(),
            )
        }));
        assert!(!colours.iter().all(|c| near(*c, top)));
        let distinct: HashSet<[u32; 3]> = colours.iter().map(|c| c.map(f32::to_bits)).collect();
        assert!(distinct.len() > 5, "{} colours", distinct.len());
        d.arrows.as_mut().unwrap().color = crate::glyph::ArrowColor::Solid;
        let s = build(&m, &set, &d);
        let first = s.positions.len() - s.arrows * each;
        let distinct: HashSet<[u32; 3]> = (first..s.positions.len())
            .map(|i| s.colors[i].map(f32::to_bits))
            .collect();
        assert_eq!(distinct.len(), 1);
    }

    /// Invariant: arrows of the part in the plane lie in the slice and
    /// point along that part (across the duct, y), the longest of them
    /// 0.9 of a spacing; the full vector on the same slice points along
    /// the flow (x). Breaks when the projection is lost or in-plane
    /// arrows keep the full field's scale (the duct's cross flow is 2 %
    /// of its speed, too short to see).
    #[test]
    fn in_plane_arrows_lie_in_the_slice() {
        let (m, set) = channel();
        let b = m.bounds();
        let mut d = by_field(&m, &set, "speed");
        let mut slice = Slice::new(b.center(), [1.0, 0.0, 0.0]);
        slice.origin[0] += 0.013;
        d.slices.push(slice);
        let mut a = velocity_arrows();
        a.density = 100;
        a.in_plane = true;
        d.arrows = Some(a.clone());
        let h = a.plane_spacing(
            &m,
            &Clip {
                origin: slice.origin,
                normal: slice.normal,
                mode: ClipMode::Flat,
                enabled: true,
            },
        );
        let s = build(&m, &set, &d);
        let arrows = arrows_of(&s);
        assert!(arrows.len() > 50, "{}", arrows.len());
        let mut longest: f64 = 0.0;
        for (tail, tip) in &arrows {
            assert!((tail[0] - slice.origin[0]).abs() < 1e-4);
            assert!((tip[0] - slice.origin[0]).abs() < 1e-4);
            let d = sub(*tip, *tail);
            let len = dot(d, d).sqrt();
            longest = longest.max(len);
            assert!(d[1] / len > 0.99, "not across the duct: {d:?}");
        }
        assert!((longest - 0.9 * h).abs() < 1e-4, "{longest}");
        assert!(s.arrow_longest > 0.0 && s.arrow_longest < 0.1);
        d.arrows.as_mut().unwrap().in_plane = false;
        let full = arrows_of(&build(&m, &set, &d));
        assert!(full.iter().all(|(a, b)| b[0] - a[0] > 0.0));
    }

    /// Invariant: on the surface, arrows go at the centres of boundary
    /// faces shown, at most one per cube of the grid, with the face's
    /// cell value (the outlet's core flow, not the node values a coarse
    /// mesh pulls down to a third); each arrow lies outside the mesh, off
    /// the face, starting at the outlet and ending at the inlet; a coarser
    /// grid gives fewer and a hidden patch none. Breaks when every face
    /// gets an arrow (a fine boundary layer turns black with them),
    /// arrows sink half into the mesh, or hidden patches keep theirs.
    #[test]
    fn surface_arrows_thin_to_one_per_cube() {
        let (m, set) = channel();
        let b = m.bounds();
        let mut d = by_field(&m, &set, "speed");
        let mut a = velocity_arrows();
        a.surface = true;
        a.density = 10;
        d.arrows = Some(a.clone());
        let h = a.spacing(&m);
        let shown = vec![true; m.cell_count()];
        let comp = |c| {
            set.values_cached(
                &m,
                &FieldChoice {
                    name: "velocity".into(),
                    component: c,
                },
            )
            .unwrap()
        };
        let v = [comp(Component::X), comp(Component::Y), comp(Component::Z)];
        let vs = [&*v[0], &*v[1], &*v[2]];
        let mut samples = Vec::new();
        crate::glyph::on_surface(&mut samples, &m, &d, &shown, h, vs, None);
        assert!(!samples.is_empty() && samples.len() < m.boundary.len());
        let mut cubes = HashSet::new();
        for x in &samples {
            let on = (0..3).find(|&k| x.p[k] == b.min[k] || x.p[k] == b.max[k]);
            let Some(k) = on else {
                panic!("not on the boundary: {:?}", x.p);
            };
            let n = x.out.unwrap();
            let sign = if x.p[k] == b.max[k] { 1.0 } else { -1.0 };
            assert!(
                (n[k] - sign).abs() < 1e-9,
                "inward normal {n:?} at {:?}",
                x.p
            );
            let key = [0, 1, 2].map(|k| ((x.p[k] - b.min[k]) / h).floor() as i64);
            assert!(cubes.insert(key), "two arrows in cube {key:?}");
            assert_eq!(x.w, [0, 1, 2].map(|k| v[k].cell[x.cell as usize]));
        }
        // The outlet's middle arrow carries its cell's speed.
        let core = v[0].cell.iter().copied().fold(0.0, f64::max);
        let outlet = samples
            .iter()
            .filter(|x| x.p[0] == b.max[0])
            .map(|x| x.w[0])
            .fold(0.0, f64::max);
        assert!(outlet > 0.8 * core, "{outlet} vs {core}");
        // Drawn: outside the box, off the face. The mesh keeps its faces:
        // arrows alone do not turn it into its outline.
        let s = build(&m, &set, &d);
        assert!(s.tri_face.iter().any(|&f| f != NO_CELL));
        let arrows = arrows_of(&s);
        // All but those with no flow (in the corners): the slow ones
        // keep a quarter of the longest's length (length mode floor).
        let longest = s.arrow_longest;
        let seen = samples
            .iter()
            .filter(|x| dot(x.w, x.w).sqrt() > 1e-6 * longest)
            .count();
        assert_eq!(arrows.len(), seen);
        assert!(seen * 2 > samples.len(), "{seen} of {}", samples.len());
        let inside = |p: [f64; 3]| (0..3).all(|k| p[k] > b.min[k] + 1e-4 && p[k] < b.max[k] - 1e-4);
        for (tail, tip) in &arrows {
            assert!(!inside(*tail) && !inside(*tip), "{tail:?} {tip:?}");
        }
        // Out of the outlet (x = 4), into the inlet (x = 0).
        let lift = 0.12 * h;
        assert!(
            arrows
                .iter()
                .any(|(t, _)| (t[0] - b.max[0] - lift).abs() < 1e-4)
        );
        assert!(
            arrows
                .iter()
                .any(|(_, p)| (p[0] - b.min[0] + lift).abs() < 1e-4)
        );
        d.arrows.as_mut().unwrap().density = 4;
        let coarse = build(&m, &set, &d).arrows;
        assert!(coarse < arrows.len(), "{coarse} vs {}", arrows.len());
        // A hidden patch has none: only the inlet and outlet keep theirs.
        let walls = m.patches.iter().position(|p| p.name == "walls").unwrap();
        d.hidden.insert(Item::Patch(walls as u32));
        let mut left = Vec::new();
        crate::glyph::on_surface(&mut left, &m, &d, &shown, h, vs, None);
        assert!(!left.is_empty());
        assert!(
            left.iter()
                .all(|x| x.p[0] == b.min[0] || x.p[0] == b.max[0])
        );
    }

    /// Invariant: no more than [`crate::glyph::MAX_ARROWS`] arrows are
    /// drawn, and the surface says so when more were wanted. Breaks when
    /// a dense grid on a large slice builds hundreds of thousands of
    /// arrows and stalls the window.
    #[test]
    fn arrows_are_capped() {
        let (m, set) = channel();
        let b = m.bounds();
        let mut d = by_field(&m, &set, "speed");
        let mut slice = Slice::new(b.center(), [0.0, 0.0, 1.0]);
        slice.origin[2] += 0.013;
        d.slices.push(slice);
        let mut a = velocity_arrows();
        a.density = crate::glyph::MAX_DENSITY;
        d.arrows = Some(a);
        let s = build(&m, &set, &d);
        assert_eq!(s.arrows, crate::glyph::MAX_ARROWS);
        assert!(s.arrows_capped);
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
