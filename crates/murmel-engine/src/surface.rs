//! The outside of the cells shown, as triangles and edge lines ready for
//! the GPU. Built once per mesh and display (colouring, hidden items,
//! clip plane, selection); orbiting never re-triangulates.
//!
//! What is drawn: the boundary faces of the cells shown (in their patch's
//! colour), the faces between a cell shown and one that is not (a hidden
//! zone or cell type, or the clip plane, lays them open; they take the
//! cell's colour), and with a flat clip the section of every cell the
//! plane crosses. So hiding the prism layers shows the polyhedral core,
//! and a crinkle clip shows the cells themselves. A region that cuts flat
//! clips the cells it cuts to its borders in the same way.
//!
//! Each polygon is a fan about its node average (exact for the
//! star-shaped faces KoflCraft writes, including merged faces, and for
//! the convex sections), with one flat normal from Newell's method.
//! Positions are relative to `origin` (the centre of the bounds) so they
//! keep their precision in `f32`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::fasthash::{FastMap, FastSet, set_with_capacity};

use murmel_mesh::{Bounds, Mesh, NO_CELL};

use crate::clip::ClipMode;
use crate::display::{Display, Item, RenderMode};
use crate::field::FieldSet;
use crate::region::{Bound, NOT_CUT, RegionCells};

/// Feature edges (render mode 4) bend by more than this angle between
/// their two faces (UI spec, Display).
pub const FEATURE_ANGLE_DEG: f64 = 30.0;
use crate::theme::{Rgb, Tokens};

/// The feature-edge group of a face that is on no patch (laid open by the
/// clip or a hidden item), and of a section.
const GROUP_OPEN: u32 = u32::MAX - 1;
const GROUP_SECTION: u32 = u32::MAX - 2;

/// Triangles per pick box ([`Surface::tri_boxes`]).
pub const PICK_RUN: usize = 128;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Surface {
    pub origin: [f64; 3],
    pub radius: f64,
    pub bounds: Option<Bounds>,
    /// Bounds of the faces drawn (hidden items left out).
    pub visible_bounds: Option<Bounds>,
    /// Boundary faces drawn, faces laid open (by the clip plane or hidden
    /// items, sections included), and the cells shown (counted once per
    /// build so the status bar does not walk the mesh every frame).
    pub faces: usize,
    pub open_faces: usize,
    pub cells: usize,
    /// Triangle vertices: position, normal, linear colour.
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 3]>,
    /// Per vertex, for colour mode field: the cell value (colour mode
    /// flat), the node value (smooth and banded), both relative to
    /// `value_origin`, the share of the selection colour to mix in, and
    /// 1 when the face has a value (0: it keeps `colors`).
    pub values: Vec<[f32; 4]>,
    pub value_origin: f64,
    /// The smallest and largest field value drawn (range mode shown).
    pub field_range: Option<(f64, f64)>,
    pub indices: Vec<u32>,
    /// Per triangle: the cell it shows (the owner of a boundary face, the
    /// cell shown beside an open face, the cell of a section;
    /// [`NO_CELL`] when unknown) and the distinct face (`face:<n>`, or
    /// [`NO_CELL`] for a section). Picking reads these.
    pub tri_cell: Vec<u32>,
    pub tri_face: Vec<u32>,
    /// The box (smallest and largest corner, relative to `origin`) of
    /// each run of [`PICK_RUN`] triangles in `indices`, so a pick tests
    /// only the runs its ray passes through ([`crate::pick`]). Triangles
    /// come in face order, so a run covers a small patch of the surface.
    pub tri_boxes: Vec<[[f32; 3]; 2]>,
    /// Edge line vertices, two per edge, with their linear colour.
    pub edge_positions: Vec<[f32; 3]>,
    pub edge_colors: Vec<[f32; 3]>,
    /// The outline of the picked cells and faces (line pairs), drawn over
    /// everything in `outline_color` (linear).
    pub outline: Vec<[f32; 3]>,
    pub outline_color: [f32; 3],
    /// Vector arrows drawn, and whether more were wanted than drawn
    /// ([`crate::glyph::MAX_ARROWS`]).
    pub arrows: usize,
    pub arrows_capped: bool,
    /// The magnitude an arrow 0.9 of a spacing long stands for (times
    /// the scale); 0 without arrows.
    pub arrow_longest: f64,
}

/// A corner of a drawn polygon: a mesh node, or a point where a clip
/// plane crosses an edge (`Cutter::cut`, numbered as found).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum V {
    Node(u32),
    Cut(u32),
}

/// One polygon to draw.
struct Poly {
    verts: Vec<V>,
    cell: u32,
    face: u32,
    boundary: u32,
    color: Rgb,
    group: u32,
}

/// A border that cuts: a clip plane, with the side and distance of every
/// node, or a border of a region cutting flat, whose distances are found
/// as needed (it cuts only the cells along it).
struct Plane {
    bound: Bound,
    /// Whether a clip plane cuts flat (region borders always do).
    flat: bool,
    nodes: Option<(Vec<bool>, Vec<f64>)>,
}

/// The cut points found so far: where each is, which plane made it, and
/// its name (plane, edge).
#[derive(Default)]
struct Cuts {
    pos: Vec<[f64; 3]>,
    plane: Vec<usize>,
    ids: FastMap<(usize, V, V), u32>,
    /// The edge each cut point lies on and where along it (0 at the
    /// first corner), so values interpolate like positions.
    along: Vec<(V, V, f64)>,
}

/// The clip planes that cut, and the borders of regions that cut flat.
/// A point where plane `p` crosses the edge between corners `a` and `b`
/// is named by `(p, a, b)`, so the cuts of neighbouring faces and cells
/// meet at the same named points and the sections close. Flat planes
/// slice in the order they were added, each one the polygons the ones
/// before it left, so a corner where two planes meet a face has one name
/// too; a cell a region cuts is then sliced by that region's borders.
struct Cutter<'a> {
    points: &'a [[f64; 3]],
    planes: Vec<Plane>,
    /// The clip planes are `planes[..clips]`.
    clips: usize,
    /// The flat clip planes, by index into `planes`, in order.
    flat: Vec<usize>,
    /// Which region cuts each cell, and each region's borders, by index
    /// into `planes`.
    regions: Option<(&'a RegionCells, Vec<std::ops::Range<usize>>)>,
    cuts: RefCell<Cuts>,
}

impl<'a> Cutter<'a> {
    /// The planes of `display` that cut, or `None` when none does.
    fn new(mesh: &'a Mesh, display: &'a Display) -> Option<Cutter<'a>> {
        let mut planes: Vec<Plane> = display
            .active_clips()
            .map(|clip| {
                let dist: Vec<f64> = mesh.points.iter().map(|&p| clip.distance(p)).collect();
                Plane {
                    bound: Bound::from(clip),
                    flat: clip.mode == ClipMode::Flat,
                    nodes: Some((dist.iter().map(|&d| d <= 0.0).collect(), dist)),
                }
            })
            .collect();
        let clips = planes.len();
        let flat = (0..clips).filter(|&p| planes[p].flat).collect();
        let regions = display
            .region_cells
            .as_deref()
            .filter(|rc| rc.cuts_flat() && rc.regions == display.regions)
            .map(|rc| {
                let ranges = rc
                    .borders
                    .iter()
                    .map(|bs| {
                        let start = planes.len();
                        planes.extend(bs.iter().map(|&bound| Plane {
                            bound,
                            flat: true,
                            nodes: None,
                        }));
                        start..planes.len()
                    })
                    .collect();
                (rc, ranges)
            });
        if planes.is_empty() {
            return None;
        }
        Some(Cutter {
            points: &mesh.points,
            planes,
            clips,
            flat,
            regions,
            cuts: RefCell::default(),
        })
    }

    /// Whether anything cuts flat.
    fn slices(&self) -> bool {
        !self.flat.is_empty() || self.regions.is_some()
    }

    /// The region that cuts cell `c` flat, or `NOT_CUT`.
    fn cut_by(&self, c: u32) -> u8 {
        self.regions
            .as_ref()
            .and_then(|(rc, _)| rc.cut_by.as_ref()?.get(c as usize).copied())
            .unwrap_or(NOT_CUT)
    }

    /// The planes that slice cell `c` (`NO_CELL`: the clip planes only),
    /// in order.
    fn flat_of(&self, c: u32) -> impl Iterator<Item = usize> + '_ {
        let region = self.regions.as_ref().and_then(|(rc, ranges)| {
            let k = *rc.cut_by.as_ref()?.get(c as usize)?;
            (k != NOT_CUT).then(|| ranges[k as usize].clone())
        });
        self.flat
            .iter()
            .copied()
            .chain(region.into_iter().flatten())
    }

    fn pos(&self, v: V) -> [f64; 3] {
        match v {
            V::Node(i) => self.points[i as usize],
            V::Cut(k) => self.cuts.borrow().pos[k as usize],
        }
    }

    fn dist(&self, p: usize, v: V) -> f64 {
        let plane = &self.planes[p];
        match (v, &plane.nodes) {
            (V::Node(i), Some((_, dist))) => dist[i as usize],
            _ => plane.bound.distance(self.pos(v)),
        }
    }

    /// Whether plane `p` keeps corner `v` (a point on the plane counts as
    /// kept, and so does every cut point the plane made).
    fn keeps(&self, p: usize, v: V) -> bool {
        match (v, &self.planes[p].nodes) {
            (V::Node(i), Some((keep, _))) => keep[i as usize],
            (V::Node(_), None) => self.dist(p, v) <= 0.0,
            (V::Cut(k), _) => self.cuts.borrow().plane[k as usize] == p || self.dist(p, v) <= 0.0,
        }
    }

    /// Whether every clip plane keeps a node of `face`.
    fn keeps_some(&self, face: &[u32]) -> bool {
        self.planes[..self.clips].iter().all(|pl| {
            pl.nodes
                .as_ref()
                .is_none_or(|(keep, _)| face.iter().any(|&i| keep[i as usize]))
        })
    }

    /// The point where plane `p` crosses the edge from `a` to `b`.
    fn cut(&self, p: usize, a: V, b: V) -> V {
        let key = (p, a.min(b), a.max(b));
        if let Some(&k) = self.cuts.borrow().ids.get(&key) {
            return V::Cut(k);
        }
        let (a, b) = (key.1, key.2);
        let (da, db) = (self.dist(p, a), self.dist(p, b));
        let t = if da == db { 0.0 } else { da / (da - db) };
        let (pa, pb) = (self.pos(a), self.pos(b));
        let at = [0, 1, 2].map(|k| pa[k] + t * (pb[k] - pa[k]));
        let mut c = self.cuts.borrow_mut();
        let k = c.pos.len() as u32;
        c.pos.push(at);
        c.plane.push(p);
        c.ids.insert(key, k);
        c.along.push((a, b, t));
        V::Cut(k)
    }

    /// The value at corner `v` of a field with `node` values: a node's
    /// own, a cut point's mixed from the ends of its edge.
    fn value(&self, v: V, node: &[f64]) -> f64 {
        match v {
            V::Node(i) => node.get(i as usize).copied().unwrap_or(f64::NAN),
            V::Cut(k) => {
                let along = self.cuts.borrow().along.get(k as usize).copied();
                match along {
                    Some((a, b, t)) => {
                        let (va, vb) = (self.value(a, node), self.value(b, node));
                        va + t * (vb - va)
                    }
                    None => f64::NAN,
                }
            }
        }
    }

    /// The part of `poly` that plane `p` keeps (Sutherland and Hodgman).
    fn clip_poly(&self, p: usize, verts: &[V]) -> Vec<V> {
        let mut out = Vec::with_capacity(verts.len() + 2);
        for i in 0..verts.len() {
            let (a, b) = (verts[i], verts[(i + 1) % verts.len()]);
            let (ka, kb) = (self.keeps(p, a), self.keeps(p, b));
            if ka {
                out.push(a);
            }
            if ka != kb {
                out.push(self.cut(p, a, b));
            }
        }
        out
    }

    /// The part of `poly`, a polygon of cell `cell`, that every plane
    /// slicing that cell keeps, cut in their order.
    fn clip_flat(&self, cell: u32, verts: Vec<V>) -> Vec<V> {
        let mut v = verts;
        for p in self.flat_of(cell) {
            if v.len() < 3 {
                break;
            }
            v = self.clip_poly(p, &v);
        }
        v
    }

    /// The section of the closed polygons `polys` by plane `p`: closed
    /// loops of cut points. In each polygon the kept part runs from the
    /// cut where it leaves the kept side to the next cut where it comes
    /// back; those segments, joined at the shared cut points, close
    /// around the cell.
    fn ring(&self, p: usize, polys: &[Vec<V>]) -> Vec<Vec<V>> {
        let mut next: HashMap<V, Vec<V>> = HashMap::new();
        for face in polys {
            let n = face.len();
            let mut cuts: Vec<(V, bool)> = Vec::new();
            for i in 0..n {
                let (a, b) = (face[i], face[(i + 1) % n]);
                let (ka, kb) = (self.keeps(p, a), self.keeps(p, b));
                if ka != kb {
                    cuts.push((self.cut(p, a, b), ka));
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
        let mut used: HashSet<V> = HashSet::new();
        let mut starts: Vec<V> = next.keys().copied().collect();
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

    /// The sections of cell `c` by the flat planes: the cell's faces are
    /// cut by each plane in turn, and each plane's section joins the
    /// polygons the next one cuts (convex clipping of a polyhedron by
    /// half-spaces, one plane at a time).
    fn sections(&self, mesh: &Mesh, c: usize) -> Vec<Vec<V>> {
        let planes: Vec<usize> = self.flat_of(c as u32).collect();
        let crossed = |p: usize| mesh.cell_points(c).any(|i| !self.keeps(p, V::Node(i)));
        if !planes.iter().any(|&p| crossed(p)) {
            return Vec::new();
        }
        let mut set: Vec<(Vec<V>, bool)> = mesh
            .cell(c)
            .map(|f| (f.oriented().map(V::Node).collect(), false))
            .collect();
        for p in planes {
            let polys: Vec<Vec<V>> = set.iter().map(|(v, _)| v.clone()).collect();
            if !polys.iter().flatten().any(|&v| !self.keeps(p, v)) {
                continue;
            }
            let rings = self.ring(p, &polys);
            set = set
                .into_iter()
                .map(|(v, s)| (self.clip_poly(p, &v), s))
                .filter(|(v, _)| v.len() >= 3)
                .collect();
            set.extend(rings.into_iter().map(|r| (r, true)));
        }
        set.into_iter()
            .filter(|(_, s)| *s)
            .map(|(v, _)| v)
            .collect()
    }
}

impl Surface {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn edges(&self) -> usize {
        self.edge_positions.len() / 2
    }

    /// Triangulate the outside of the cells of `mesh` that `display`
    /// shows, in the colours it asks for, with its slices (iso-surfaces
    /// need the fields: [`Surface::build_with`]).
    pub fn build(mesh: &Mesh, tokens: &Tokens, display: &Display) -> Surface {
        Self::build_with(mesh, None, tokens, display)
    }

    /// [`Surface::build`] with the slices and iso-surfaces of `display`
    /// ([`crate::extract`]) drawn into the same buffers.
    pub fn build_with(
        mesh: &Mesh,
        fields: Option<&FieldSet>,
        tokens: &Tokens,
        display: &Display,
    ) -> Surface {
        let bounds = mesh.bounds();
        let origin = bounds.center();
        let mut s = Surface {
            origin,
            radius: bounds.radius(),
            bounds: (!bounds.is_empty()).then_some(bounds),
            ..Surface::default()
        };
        let phase = murmel_mesh::profile::span("surface.polygons");
        let cutter = Cutter::new(mesh, display);
        let shown = display.shown_cells(mesh);
        s.cells = shown.iter().filter(|&&v| v).count();
        let polys = polygons(mesh, tokens, display, &shown, cutter.as_ref());
        drop(phase);
        let phase = murmel_mesh::profile::span("surface.points");

        let pos = |v: V| -> [f64; 3] {
            let p = match (v, &cutter) {
                (V::Node(i), _) => mesh.points[i as usize],
                (_, Some(c)) => c.pos(v),
                (V::Cut(_), None) => [0.0; 3],
            };
            [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]]
        };
        let darken = 1.0 - tokens.mesh.edge_darken;
        let sel = tokens.mesh.selection;
        let mix = tokens.mesh.selection_mix.clamp(0.0, 1.0);
        // Selected faces first, so the edges they share with others take
        // the selection colour.
        // Then boundary faces in file order, then open faces: an edge two
        // faces share takes the colour of the first.
        let mut order: Vec<(bool, u32, usize)> = polys
            .iter()
            .enumerate()
            .map(|(i, p)| (!selected(mesh, display, p), p.boundary, i))
            .collect();
        order.sort_unstable();
        s.faces = polys.iter().filter(|p| p.boundary != NO_CELL).count();
        s.open_faces = polys.len() - s.faces;
        let mut vb = Bounds::empty();
        // Each corner starts one edge, and most edges are shared by two
        // polygons.
        let corners: usize = polys.iter().map(|p| p.verts.len()).sum();
        let mut seen: FastSet<(V, V)> = set_with_capacity(corners / 2 + 16);
        let pts: Vec<Vec<[f64; 3]>> = polys
            .iter()
            .map(|p| p.verts.iter().map(|&v| pos(v)).collect())
            .collect();
        let normals: Vec<[f32; 3]> = pts.iter().map(|p| newell(p)).collect();
        // While slices or iso-surfaces show, the mesh is a faint outline
        // of its feature edges (or, in a render mode without faces, its
        // lines): its triangles are left out, so a click picks what is
        // inside.
        let faint = display.outline_mesh();
        // Without its faces, the mesh gives no triangles (the renderer
        // draws faces for slices, iso-surfaces and arrows).
        let extras = display.extracts_shown() || display.arrows_shown();
        let mesh_triangles = !(extras && !display.mesh_faces());
        let features = (display.render_mode == RenderMode::FeatureEdges || faint).then(|| {
            feature_edges(
                polys
                    .iter()
                    .zip(&normals)
                    .map(|(p, n)| (p.verts.as_slice(), p.group, *n)),
                FEATURE_ANGLE_DEG.to_radians().cos(),
            )
        });
        // Colour mode field: the value of each polygon (colour mode flat)
        // and of each corner (smooth, banded).
        let field = display.field_values();
        s.value_origin = field.map_or(0.0, |f| f.origin);
        let corner_values: Vec<Vec<f64>> = match field {
            Some(f) => polys
                .iter()
                .map(|p| {
                    p.verts
                        .iter()
                        .map(|&v| match (v, &cutter) {
                            (V::Node(i), _) => f.node.get(i as usize).copied().unwrap_or(f64::NAN),
                            (_, Some(c)) => c.value(v, &f.node),
                            (V::Cut(_), None) => f64::NAN,
                        })
                        .collect()
                })
                .collect(),
            None => Vec::new(),
        };
        drop(phase);
        let phase = murmel_mesh::profile::span("surface.buffers");
        if mesh_triangles {
            // A triangle is drawn as is; a larger polygon as a fan about
            // its centre.
            let (mut vertices, mut indices) = (0, 0);
            for p in &polys {
                let k = p.verts.len();
                vertices += if k == 3 { 3 } else { k + 1 };
                indices += if k == 3 { 3 } else { 3 * k };
            }
            s.positions.reserve(vertices);
            s.normals.reserve(vertices);
            s.colors.reserve(vertices);
            s.values.reserve(vertices);
            s.indices.reserve(indices);
            s.tri_cell.reserve(indices / 3);
            s.tri_face.reserve(indices / 3);
        }
        if features.is_none() {
            // Feature edges are a few of the edges; the others are all
            // drawn, each once.
            s.edge_positions.reserve(corners);
            s.edge_colors.reserve(corners);
        }
        let mut drawn = Bounds1::default();
        // Lines alone are drawn in the face colour; over faces, darker.
        let lines_only = !display.render_mode.faces();
        for (unselected, _, i) in order {
            let poly = &polys[i];
            let rgb = poly.color;
            // The polygon's flat value and its corners' values, when it
            // has them all.
            let valued = field.and_then(|f| {
                let corners = &corner_values[i];
                if !corners.iter().all(|x| x.is_finite()) {
                    return None;
                }
                let mut flat = f.flat(poly.cell, poly.boundary);
                if !flat.is_finite() {
                    // A face of no cell (a surface mesh) with a point
                    // field: the mean of its corners.
                    flat = corners.iter().sum::<f64>() / corners.len().max(1) as f64;
                }
                flat.is_finite().then_some((flat, corners))
            });
            if let Some((flat, corners)) = valued
                && mesh_triangles
            {
                drawn.add(flat);
                corners.iter().for_each(|&x| drawn.add(x));
            }
            let sel_mix = if unselected { 0.0 } else { mix };
            let (color, ec) = if faint {
                let ec = if unselected {
                    tokens.mesh.field.outline
                } else {
                    sel
                };
                (rgb.linear(), ec.linear())
            } else if field.is_some() {
                let face = if valued.is_some() {
                    rgb
                } else {
                    tokens.mesh.field.none
                };
                let face = if unselected {
                    face
                } else {
                    Rgb(std::array::from_fn(|k| {
                        (face.0[k] as f32 * (1.0 - mix) + sel.0[k] as f32 * mix).round() as u8
                    }))
                };
                let edge = if !unselected {
                    sel
                } else if lines_only {
                    tokens.mesh.field.none
                } else {
                    tokens.mesh.field.edge
                };
                (face.linear(), edge.linear())
            } else if unselected {
                (
                    rgb.linear(),
                    if lines_only {
                        rgb.linear()
                    } else {
                        Rgb(rgb.0.map(|v| (v as f32 * darken).round() as u8)).linear()
                    },
                )
            } else {
                let m = Rgb(std::array::from_fn(|k| {
                    (rgb.0[k] as f32 * (1.0 - mix) + sel.0[k] as f32 * mix).round() as u8
                }));
                (m.linear(), sel.linear())
            };
            let pts = &pts[i];
            for p in pts {
                vb.add([p[0] + origin[0], p[1] + origin[1], p[2] + origin[2]]);
            }
            let n = normals[i];
            let base = s.positions.len() as u32;
            let origin_v = s.value_origin;
            let value = |node: f64| -> [f32; 4] {
                match valued {
                    Some((flat, _)) => [
                        (flat - origin_v) as f32,
                        (node - origin_v) as f32,
                        sel_mix,
                        1.0,
                    ],
                    None => [0.0, 0.0, 0.0, 0.0],
                }
            };
            let corner =
                |j: usize| -> f64 { valued.map_or(0.0, |(_, c)| c.get(j).copied().unwrap_or(0.0)) };
            let push = |s: &mut Surface, p: [f64; 3], v: [f32; 4]| {
                s.positions.push(p.map(|v| v as f32));
                s.normals.push(n);
                s.colors.push(color);
                s.values.push(v);
            };
            let tris_before = s.indices.len() / 3;
            if !mesh_triangles {
                // Edges only.
            } else if pts.len() == 3 {
                for (j, &p) in pts.iter().enumerate() {
                    push(&mut s, p, value(corner(j)));
                }
                s.indices.extend([base, base + 1, base + 2]);
            } else {
                let k = pts.len() as f64;
                let c = [0, 1, 2].map(|d| pts.iter().map(|p| p[d]).sum::<f64>() / k);
                let mean = (0..pts.len()).map(corner).sum::<f64>() / k;
                push(&mut s, c, value(mean));
                for (j, &p) in pts.iter().enumerate() {
                    push(&mut s, p, value(corner(j)));
                }
                let m = pts.len() as u32;
                for i in 0..m {
                    s.indices
                        .extend([base, base + 1 + i, base + 1 + (i + 1) % m]);
                }
            }
            let tris = s.indices.len() / 3 - tris_before;
            s.tri_cell.extend(std::iter::repeat_n(poly.cell, tris));
            s.tri_face.extend(std::iter::repeat_n(poly.face, tris));
            let vs = &poly.verts;
            for j in 0..vs.len() {
                let (a, b) = (vs[j], vs[(j + 1) % vs.len()]);
                let key = (a.min(b), a.max(b));
                if a == b || features.as_ref().is_some_and(|fe| !fe.contains(&key)) {
                    continue;
                }
                if seen.insert(key) {
                    s.edge_positions.push(pts[j].map(|v| v as f32));
                    s.edge_positions
                        .push(pts[(j + 1) % vs.len()].map(|v| v as f32));
                    s.edge_colors.push(ec);
                    s.edge_colors.push(ec);
                }
            }
        }
        drop(phase);
        s.visible_bounds = (!vb.is_empty()).then_some(vb);
        s.field_range = drawn.range();
        let _phase = murmel_mesh::profile::span("surface.extras");
        if extras {
            crate::extract::append(&mut s, mesh, fields, tokens, display, &shown);
        }
        s.outline_color = sel.linear();
        for (a, b) in outline(mesh, display) {
            for i in [a, b] {
                let p = mesh.points[i as usize];
                s.outline.push([0, 1, 2].map(|k| (p[k] - origin[k]) as f32));
            }
        }
        drop(_phase);
        let _phase = murmel_mesh::profile::span("surface.boxes");
        s.tri_boxes = tri_boxes(&s.positions, &s.indices);
        s
    }
}

/// The box of each run of [`PICK_RUN`] triangles, runs split over the
/// threads.
fn tri_boxes(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[[f32; 3]; 2]> {
    let tris = indices.as_chunks::<3>().0;
    let runs = tris.len().div_ceil(PICK_RUN);
    murmel_mesh::par::collect(runs, 1 << 10, |r| {
        r.map(|k| {
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            let end = ((k + 1) * PICK_RUN).min(tris.len());
            for tri in &tris[k * PICK_RUN..end] {
                for &i in tri {
                    if let Some(p) = positions.get(i as usize) {
                        for d in 0..3 {
                            lo[d] = lo[d].min(p[d]);
                            hi[d] = hi[d].max(p[d]);
                        }
                    }
                }
            }
            [lo, hi]
        })
        .collect()
    })
}

/// The smallest and largest of the values added.
#[derive(Default)]
struct Bounds1(Option<(f64, f64)>);

impl Bounds1 {
    fn add(&mut self, x: f64) {
        if x.is_finite() {
            self.0 = Some(match self.0 {
                None => (x, x),
                Some((a, b)) => (a.min(x), b.max(x)),
            });
        }
    }

    fn range(&self) -> Option<(f64, f64)> {
        self.0
    }
}

/// The polygons to draw, before the selection orders them.
fn polygons(
    mesh: &Mesh,
    tokens: &Tokens,
    display: &Display,
    shown: &[bool],
    cutter: Option<&Cutter<'_>>,
) -> Vec<Poly> {
    let flat = cutter.filter(|c| c.slices());
    let mut polys: Vec<Poly> = Vec::new();
    let mut add = |verts: Vec<V>, cell: u32, face: u32, boundary: u32, color: Rgb, group: u32| {
        let verts = match flat {
            Some(c) => c.clip_flat(cell, verts),
            None => verts,
        };
        if verts.len() >= 3 {
            polys.push(Poly {
                verts,
                cell,
                face,
                boundary,
                color,
                group,
            });
        }
    };
    let nodes = |f: &[u32]| f.iter().map(|&i| V::Node(i)).collect::<Vec<_>>();
    let patch = |b: usize| mesh.boundary_patch.get(b).copied().unwrap_or(0);
    // Boundary polygons that match no cell face (or a mesh with no cells):
    // drawn by their patch, and by the clip when it keeps a node.
    for (b, &u) in mesh.boundary_face_id.iter().enumerate() {
        if u != NO_CELL && mesh.cell_count() > 0 {
            continue;
        }
        let face = mesh.boundary.get(b);
        if !display.face_visible(mesh, b) || cutter.is_some_and(|c| !c.keeps_some(face)) {
            continue;
        }
        add(
            nodes(face),
            NO_CELL,
            u,
            b as u32,
            display.face_color(mesh, tokens, b),
            patch(b),
        );
    }
    for u in 0..mesh.face_owner.len().min(mesh.face_neighbour.len()) {
        let (o, n) = (mesh.face_owner[u], mesh.face_neighbour[u]);
        if o == NO_CELL {
            continue;
        }
        let so = shown[o as usize];
        if n == NO_CELL {
            if !so {
                continue;
            }
            match mesh.face_boundary.get(u).copied() {
                Some(b) if b != NO_CELL => {
                    let b = b as usize;
                    if !display.face_visible(mesh, b) {
                        continue;
                    }
                    add(
                        nodes(mesh.boundary.get(b)),
                        o,
                        u as u32,
                        b as u32,
                        display.face_color(mesh, tokens, b),
                        patch(b),
                    );
                }
                _ => add(
                    nodes(mesh.face_nodes(u)),
                    o,
                    u as u32,
                    NO_CELL,
                    display.cell_color(mesh, tokens, o as usize),
                    GROUP_OPEN,
                ),
            }
            continue;
        }
        let sn = shown[n as usize];
        if so && sn {
            // Two cells cut differently (one by a flat region, the other
            // whole or by another region): each side draws the face as
            // its own clip keeps it, so where one side is cut away the
            // other's face closes the surface (the rest lies inside).
            if let Some(cut) = flat
                && cut.cut_by(o) != cut.cut_by(n)
            {
                for c in [o, n] {
                    add(
                        nodes(mesh.face_nodes(u)),
                        c,
                        u as u32,
                        NO_CELL,
                        display.cell_color(mesh, tokens, c as usize),
                        GROUP_OPEN,
                    );
                }
            }
            continue;
        }
        if !so && !sn {
            continue;
        }
        let c = if so { o } else { n };
        add(
            nodes(mesh.face_nodes(u)),
            c,
            u as u32,
            NO_CELL,
            display.cell_color(mesh, tokens, c as usize),
            GROUP_OPEN,
        );
    }
    // Faces isolated on their own (I on picked faces), unless a cell
    // shown beside them already draws them.
    for &item in &display.isolated {
        let Item::Face(u) = item else { continue };
        let u = u as usize;
        let side = |c: Option<&u32>| c.is_some_and(|&c| c != NO_CELL && shown[c as usize]);
        if u >= mesh.unique_faces
            || side(mesh.face_owner.get(u))
            || side(mesh.face_neighbour.get(u))
        {
            continue;
        }
        let owner = mesh.face_owner.get(u).copied().unwrap_or(NO_CELL);
        match mesh.face_boundary.get(u).copied() {
            Some(b) if b != NO_CELL => add(
                nodes(mesh.boundary.get(b as usize)),
                owner,
                u as u32,
                b,
                display.face_color(mesh, tokens, b as usize),
                patch(b as usize),
            ),
            _ => add(
                nodes(mesh.face_nodes(u)),
                owner,
                u as u32,
                NO_CELL,
                if owner == NO_CELL {
                    tokens.mesh.solid
                } else {
                    display.cell_color(mesh, tokens, owner as usize)
                },
                GROUP_OPEN,
            ),
        }
    }
    if let Some(cutter) = flat {
        for (c, &on) in shown.iter().enumerate() {
            if !on {
                continue;
            }
            let rings = cutter.sections(mesh, c);
            if rings.is_empty() {
                continue;
            }
            let color = display.cell_color(mesh, tokens, c);
            for ring in rings {
                polys.push(Poly {
                    verts: ring,
                    cell: c as u32,
                    face: NO_CELL,
                    boundary: NO_CELL,
                    color,
                    group: GROUP_SECTION,
                });
            }
        }
    }
    polys
}

/// Whether `poly` belongs to a selected item.
fn selected(mesh: &Mesh, display: &Display, poly: &Poly) -> bool {
    let cell = (poly.cell != NO_CELL).then_some(poly.cell as usize);
    display.selection.iter().any(|&item| match item {
        Item::Mesh => false,
        Item::Patch(p) => {
            poly.boundary != NO_CELL && mesh.boundary_patch.get(poly.boundary as usize) == Some(&p)
        }
        Item::Zone(z) => cell.is_some_and(|c| mesh.cell_zone.get(c) == Some(&z)),
        Item::Kind(k) => cell.is_some_and(|c| mesh.cell_kind.get(c) == Some(&k)),
        Item::Cell(c) => poly.cell == c,
        Item::Face(u) => poly.face == u,
        Item::Clip(_)
        | Item::Measure(_)
        | Item::Probe(_)
        | Item::Slice(_)
        | Item::Iso(_)
        | Item::Arrows
        | Item::Region(_) => false,
    })
}

/// The outline of the picked cells and faces: every edge of a selected
/// cell (or of the cell holding a selected probe), every edge of a
/// selected face, each once, as line pairs of point ids. Drawn over everything (UI spec: "an accent outline"), so a picked
/// cell stays visible behind others.
fn outline(mesh: &Mesh, display: &Display) -> Vec<(u32, u32)> {
    let mut seen: HashSet<(u32, u32)> = HashSet::new();
    let mut out = Vec::new();
    let mut add_face = |f: &[u32], out: &mut Vec<(u32, u32)>| {
        for i in 0..f.len() {
            let (a, b) = (f[i], f[(i + 1) % f.len()]);
            if seen.insert((a.min(b), a.max(b))) {
                out.push((a, b));
            }
        }
    };
    for &item in &display.selection {
        let item = match item {
            Item::Probe(n) => match display.probe(n).and_then(|p| p.at.cell) {
                Some(c) => Item::Cell(c),
                None => continue,
            },
            other => other,
        };
        match item {
            Item::Cell(c) if (c as usize) < mesh.cell_count() => {
                for f in mesh.cell(c as usize) {
                    add_face(f.nodes, &mut out);
                }
            }
            Item::Face(u) => add_face(mesh.face_nodes(u as usize), &mut out),
            _ => {}
        }
    }
    out
}

/// The feature edges among `polys` (corners, feature group, normal): an
/// edge whose two faces meet at a crease (the cosine of the angle between
/// their normals below `cos_max`), whose faces lie in different groups
/// (patches, open faces, sections), or that has only one face drawn (an
/// open border, also where hidden items were cut away). The normals'
/// orientation is not trusted, so a crease is judged on the absolute
/// cosine.
fn feature_edges<'a>(
    polys: impl Iterator<Item = (&'a [V], u32, [f32; 3])>,
    cos_max: f64,
) -> FastSet<(V, V)> {
    struct Seen {
        normal: [f32; 3],
        group: u32,
        count: u32,
        feature: bool,
    }
    let mut edges: FastMap<(V, V), Seen> = FastMap::default();
    for (face, group, n) in polys {
        for i in 0..face.len() {
            let (a, b) = (face[i], face[(i + 1) % face.len()]);
            let e = edges.entry((a.min(b), a.max(b))).or_insert(Seen {
                normal: n,
                group,
                count: 0,
                feature: false,
            });
            if e.count > 0 {
                let dot: f32 = (0..3).map(|k| e.normal[k] * n[k]).sum();
                e.feature |= e.group != group || (dot.abs() as f64) < cos_max;
            }
            e.count += 1;
        }
    }
    edges
        .into_iter()
        .filter(|(_, e)| e.feature || e.count == 1)
        .map(|(k, _)| k)
        .collect()
}

/// The unit normal of a polygon by Newell's method.
fn newell(p: &[[f64; 3]]) -> [f32; 3] {
    let mut n = [0.0f64; 3];
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l > 0.0 {
        n.map(|v| (v / l) as f32)
    } else {
        [0.0, 0.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{Clip, ClipMode};

    /// Invariant: on the channel (a box of walls, inlet and outlet) the
    /// feature edges are the box's 12 creases and nothing inside a flat
    /// side: all lie on the bounding box's edges, the flat interior edges
    /// are dropped, and every box edge is covered.
    #[test]
    fn channel_feature_edges_are_the_box_edges() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        let mesh = murmel_mesh::read(&p).unwrap();
        let t = Tokens::builtin().unwrap();
        let all = Surface::build(&mesh, &t, &Display::default());
        let d = Display {
            render_mode: RenderMode::FeatureEdges,
            ..Display::default()
        };
        let s = Surface::build(&mesh, &t, &d);
        assert!(
            s.edges() > 0 && s.edges() < all.edges() / 4,
            "{} of {}",
            s.edges(),
            all.edges()
        );
        let b = s.bounds.unwrap();
        let half: Vec<f64> = (0..3).map(|k| 0.5 * (b.max[k] - b.min[k])).collect();
        let mut length = 0.0;
        for e in s.edge_positions.chunks(2) {
            let on_box_edge = |p: [f32; 3]| {
                (0..3)
                    .filter(|&k| ((p[k].abs() as f64) - half[k]).abs() < 1e-6 * (1.0 + half[k]))
                    .count()
                    >= 2
            };
            assert!(on_box_edge(e[0]) && on_box_edge(e[1]), "{e:?}");
            length += (0..3)
                .map(|k| ((e[0][k] - e[1][k]) as f64).powi(2))
                .sum::<f64>()
                .sqrt();
        }
        let want = 4.0 * 2.0 * (half[0] + half[1] + half[2]);
        assert!((length - want).abs() < 1e-6 * want, "{length} vs {want}");
    }

    fn channel() -> Mesh {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        murmel_mesh::read(&p).unwrap()
    }

    fn clip_y(mesh: &Mesh, mode: ClipMode) -> Display {
        let b = mesh.bounds();
        Display {
            clips: vec![Clip {
                origin: b.center(),
                normal: [0.0, 1.0, 0.0],
                mode,
                enabled: true,
            }],
            ..Display::default()
        }
    }

    /// The y plane of `clip_y` and more planes through the centre: x with
    /// mode `b`, and for three planes an oblique one, flat.
    fn clips(mesh: &Mesh, a: ClipMode, b: ClipMode, three: bool) -> Display {
        let mut d = clip_y(mesh, a);
        let c = mesh.bounds().center();
        d.clips.push(Clip {
            origin: c,
            normal: [1.0, 0.0, 0.0],
            mode: b,
            enabled: true,
        });
        if three {
            let n = 1.0 / 3f64.sqrt();
            d.clips.push(Clip {
                origin: [c[0] - 0.1, c[1], c[2]],
                normal: [-n, n, n],
                mode: ClipMode::Flat,
                enabled: true,
            });
        }
        d
    }

    /// Every polygon edge of a display, counted by use (zero-length edges
    /// at nodes on the plane left out).
    fn edge_uses(mesh: &Mesh, d: &Display) -> HashMap<(V, V), usize> {
        let t = Tokens::builtin().unwrap();
        let shown = d.shown_cells(mesh);
        let cutter = Cutter::new(mesh, d);
        let mut uses = HashMap::new();
        for p in polygons(mesh, &t, d, &shown, cutter.as_ref()) {
            for i in 0..p.verts.len() {
                let (a, b) = (p.verts[i], p.verts[(i + 1) % p.verts.len()]);
                if a != b {
                    *uses.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                }
            }
        }
        uses
    }

    /// The mean of cell `c`'s points.
    fn cell_mean(mesh: &Mesh, c: usize) -> [f64; 3] {
        let pts: Vec<[f64; 3]> = mesh
            .cell_points(c)
            .map(|i| mesh.points[i as usize])
            .collect();
        [0, 1, 2].map(|k| pts.iter().map(|p| p[k]).sum::<f64>() / pts.len() as f64)
    }

    /// A display of the channel with `regions` (their cells found).
    fn with_regions(mesh: &Mesh, regions: Vec<crate::Region>, clips: Vec<Clip>) -> Display {
        let mut d = Display {
            regions,
            clips,
            ..Display::default()
        };
        crate::region::ensure(&mut d, mesh);
        d
    }

    /// Invariant (large-mesh stage 1b, the maintainer's card 6): a box,
    /// a slab or a sphere region cutting flat draws a closed surface,
    /// every edge shared by exactly two polygons, and nothing outside
    /// the shape: a box's and a slab's corners lie inside every face's
    /// half-space, a sphere's within the sphere (a cut point on an edge
    /// is inside, the distance being convex). A slab thinner than the
    /// channel's cells still draws its cells' sections. Breaks when a
    /// cell's cut does not close, when a polygon escapes its region's
    /// clip, or when a thin slab draws nothing.
    #[test]
    fn flat_regions_are_closed_and_stay_inside() {
        use crate::region::{Region, Shape};
        let m = channel();
        let b = m.bounds();
        // Inside the fluid (the channel's middle is its obstacle).
        let c = cell_mean(&m, m.cell_count() / 3);
        let r = 0.3 * b.radius();
        let thin = 1e-3 * b.radius();
        let flat = |shape: Shape| Region {
            cut: ClipMode::Flat,
            ..Region::new(shape)
        };
        for shape in [
            Shape::default_box(&b),
            Shape::Box {
                center: c,
                size: [r, r, 0.5 * thin],
            },
            Shape::Sphere {
                center: c,
                radius: r,
            },
            Shape::Slab {
                origin: c,
                normal: [0.6, 0.0, 0.8],
                thickness: 0.5 * r,
            },
            Shape::Slab {
                origin: c,
                normal: [0.0, 1.0, 0.0],
                thickness: thin,
            },
        ] {
            let d = with_regions(&m, vec![flat(shape.clone())], Vec::new());
            let t = Tokens::builtin().unwrap();
            let shown = d.shown_cells(&m);
            let cutter = Cutter::new(&m, &d).expect("a flat region cuts");
            let polys = polygons(&m, &t, &d, &shown, Some(&cutter));
            assert!(
                polys.iter().any(|p| p.group == GROUP_SECTION),
                "{shape:?} draws sections"
            );
            let borders = shape.borders();
            let tol = 1e-9 * b.radius();
            for p in &polys {
                for &v in &p.verts {
                    let at = cutter.pos(v);
                    for bd in &borders {
                        assert!(bd.distance(at) <= tol, "{shape:?}: {at:?} outside");
                    }
                }
            }
            let uses = edge_uses(&m, &d);
            let open: Vec<_> = uses.iter().filter(|(_, n)| **n != 2).take(3).collect();
            assert!(open.is_empty(), "{shape:?}: {open:?}");
        }
    }

    /// Invariant: regions cutting differently side by side still draw a
    /// closed surface: a flat box beside a whole-cell sphere, two flat
    /// regions overlapping, and a flat region under a flat clip plane;
    /// every edge is used an even number of times (each way of cutting
    /// closes its own cells, and the face between two cells cut
    /// differently is drawn by both). Breaks when the face between a
    /// clipped cell and a whole one is left out, which opens a hole into
    /// the whole cell.
    #[test]
    fn mixed_regions_are_closed() {
        use crate::region::{Region, Shape};
        let m = channel();
        let b = m.bounds();
        let c = cell_mean(&m, m.cell_count() / 3);
        let r = 0.3 * b.radius();
        let flat = |shape: Shape| Region {
            cut: ClipMode::Flat,
            ..Region::new(shape)
        };
        let sphere = Shape::Sphere {
            center: [c[0] + 0.5 * r, c[1], c[2]],
            radius: r,
        };
        let cases = [
            (
                "flat box, whole sphere",
                vec![flat(Shape::default_box(&b)), Region::new(sphere.clone())],
                Vec::new(),
            ),
            (
                "flat box, flat sphere",
                vec![flat(Shape::default_box(&b)), flat(sphere.clone())],
                Vec::new(),
            ),
            (
                "flat sphere, flat clip",
                vec![flat(sphere.clone())],
                vec![Clip {
                    origin: c,
                    normal: [0.0, 1.0, 0.0],
                    mode: ClipMode::Flat,
                    enabled: true,
                }],
            ),
        ];
        for (name, regions, clips) in cases {
            let d = with_regions(&m, regions, clips);
            let uses = edge_uses(&m, &d);
            assert!(!uses.is_empty(), "{name}");
            let odd: Vec<_> = uses.iter().filter(|(_, n)| **n % 2 != 0).take(3).collect();
            assert!(odd.is_empty(), "{name}: {odd:?}");
        }
    }

    /// Invariant: what the viewport draws is a closed surface, every edge
    /// shared by exactly two polygons, for the whole channel, with the
    /// prism layers hidden (the core's faces laid open), and with a
    /// crinkle or a flat clip (the sections close the cut), and with two
    /// or three planes at once (a corner where two flat planes meet a face
    /// has one name in the face and in both sections). A wrong neighbour
    /// in the face table, a section that does not close, or a cut point
    /// named differently by two faces would leave a hole.
    #[test]
    fn drawn_surfaces_are_closed() {
        let m = channel();
        let mut no_prisms = Display::default();
        no_prisms
            .hidden
            .insert(Item::Kind(murmel_mesh::CellKind::Prism));
        for (name, d) in [
            ("all", Display::default()),
            ("no prisms", no_prisms),
            ("crinkle", clip_y(&m, ClipMode::Crinkle)),
            ("flat", clip_y(&m, ClipMode::Flat)),
            (
                "flat + flat",
                clips(&m, ClipMode::Flat, ClipMode::Flat, false),
            ),
            (
                "crinkle + flat",
                clips(&m, ClipMode::Crinkle, ClipMode::Flat, false),
            ),
            (
                "three flat",
                clips(&m, ClipMode::Flat, ClipMode::Flat, true),
            ),
        ] {
            let uses = edge_uses(&m, &d);
            assert!(!uses.is_empty(), "{name}");
            let open: Vec<_> = uses.iter().filter(|(_, n)| **n != 2).take(3).collect();
            assert!(open.is_empty(), "{name}: {open:?}");
        }
    }

    /// Invariant: the buffers the renderer uploads are well formed for
    /// every display the render tests draw: one normal and colour per
    /// position, whole triangles and segments, every index in range, all
    /// numbers finite, unit normals. A software rasteriser may read out of
    /// bounds (or crash) on a bad index or a NaN instead of failing
    /// validation.
    #[test]
    fn buffers_are_well_formed() {
        let m = channel();
        let t = Tokens::builtin().unwrap();
        let mut no_prisms = Display::default();
        no_prisms
            .hidden
            .insert(Item::Kind(murmel_mesh::CellKind::Prism));
        for (name, d) in [
            ("all", Display::default()),
            ("no prisms", no_prisms),
            ("crinkle", clip_y(&m, ClipMode::Crinkle)),
            ("flat", clip_y(&m, ClipMode::Flat)),
            (
                "flat + flat",
                clips(&m, ClipMode::Flat, ClipMode::Flat, false),
            ),
            (
                "crinkle + flat",
                clips(&m, ClipMode::Crinkle, ClipMode::Flat, false),
            ),
            (
                "three flat",
                clips(&m, ClipMode::Flat, ClipMode::Flat, true),
            ),
        ] {
            let s = Surface::build(&m, &t, &d);
            let n = s.positions.len();
            assert_eq!(s.normals.len(), n, "{name}");
            assert_eq!(s.colors.len(), n, "{name}");
            assert_eq!(s.indices.len() % 3, 0, "{name}");
            assert!(s.indices.iter().all(|&i| (i as usize) < n), "{name}");
            assert_eq!(s.edge_positions.len() % 2, 0, "{name}");
            assert_eq!(s.edge_colors.len(), s.edge_positions.len(), "{name}");
            let finite = |v: &[[f32; 3]]| v.iter().flatten().all(|x| x.is_finite());
            assert!(finite(&s.positions) && finite(&s.colors), "{name}");
            assert!(
                finite(&s.edge_positions) && finite(&s.edge_colors),
                "{name}"
            );
            let bad: Vec<_> = s
                .normals
                .iter()
                .filter(|v| {
                    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                    (l - 1.0).abs() >= 1e-3
                })
                .take(3)
                .collect();
            assert!(bad.is_empty(), "{name}: {bad:?}");
        }
    }

    /// Invariant: hiding the prism layers keeps the 8 polyhedra of the
    /// core and draws their faces, and the crinkle clip keeps whole cells:
    /// fewer than all, more than half (the cells the plane crosses stay).
    #[test]
    fn hidden_types_and_crinkle_keep_whole_cells() {
        let m = channel();
        let t = Tokens::builtin().unwrap();
        let mut d = Display::default();
        d.hidden.insert(Item::Kind(murmel_mesh::CellKind::Prism));
        let s = Surface::build(&m, &t, &d);
        assert_eq!(s.cells, 8);
        assert!(s.open_faces > 0);
        let s = Surface::build(&m, &t, &clip_y(&m, ClipMode::Crinkle));
        assert!(s.cells > m.cell_count() / 2 && s.cells < m.cell_count());
        assert_eq!(s.tri_cell.len(), s.triangles());
        assert!(s.tri_cell.iter().all(|&c| (c as usize) < m.cell_count()));
    }

    /// Invariant: the flat cut's sections tile the channel's cross-section
    /// at y = centre exactly (width times height of the box), so the
    /// section of every crossed cell is there once and none overlaps.
    #[test]
    fn flat_sections_tile_the_cross_section() {
        let m = channel();
        let t = Tokens::builtin().unwrap();
        let d = clip_y(&m, ClipMode::Flat);
        let s = Surface::build(&m, &t, &d);
        let b = m.bounds();
        let want = (b.max[0] - b.min[0]) * (b.max[2] - b.min[2]);
        let mut area = 0.0;
        for (i, tri) in s.indices.chunks(3).enumerate() {
            if s.tri_face[i] != NO_CELL {
                continue;
            }
            let p: [[f64; 3]; 3] =
                std::array::from_fn(|j| s.positions[tri[j] as usize].map(f64::from));
            let e1 = [0, 1, 2].map(|k| p[1][k] - p[0][k]);
            let e2 = [0, 1, 2].map(|k| p[2][k] - p[0][k]);
            area += 0.5 * crate::clip::cross(e1, e2)[1].abs();
        }
        assert!((area - want).abs() < 1e-4 * want, "{area} vs {want}");
        // Everything drawn is on the kept side (y <= centre, to f32).
        assert!(s.positions.iter().all(|p| p[1] <= 1e-5));
    }

    /// Invariant: a selected probe outlines the cell that holds it, the
    /// same lines as selecting that cell, and a probe outside the mesh
    /// outlines nothing. Breaks if `outline` stops mapping probes to their
    /// cell; the window draws the probe marker with egui, so no render
    /// test would notice the missing outline.
    #[test]
    fn a_selected_probe_outlines_its_cell() {
        let m = channel();
        let t = Tokens::builtin().unwrap();
        let mut cell = Display::default();
        cell.selection.push(Item::Cell(7));
        let want = Surface::build(&m, &t, &cell).outline;
        assert!(!want.is_empty());
        let mut d = Display::default();
        let at = |c: Option<u32>| crate::measure::Anchor {
            cell: c,
            ..crate::measure::Anchor::typed([0.0; 3])
        };
        d.probes.push(crate::measure::Probe {
            n: 1,
            name: None,
            at: at(Some(7)),
            visible: true,
        });
        d.probes.push(crate::measure::Probe {
            n: 2,
            name: None,
            at: at(None),
            visible: true,
        });
        d.selection.push(Item::Probe(1));
        assert_eq!(Surface::build(&m, &t, &d).outline, want);
        d.selection = vec![Item::Probe(2)];
        assert!(Surface::build(&m, &t, &d).outline.is_empty());
    }

    /// Invariant: every boundary polygon becomes a closed fan (n triangles
    /// for n > 3 nodes, one for a triangle) and every polygon edge is drawn
    /// once, so a closed surface has as many edge lines as unique edges.
    #[test]
    fn tetra_surface() {
        let mesh = murmel_mesh::read_vtu_str(
            r#"<VTKFile type="UnstructuredGrid"><UnstructuredGrid><Piece NumberOfPoints="4" NumberOfCells="1">
<Points><DataArray type="Float64" NumberOfComponents="3" format="ascii">0 0 0 1 0 0 0 1 0 0 0 1</DataArray></Points>
<Cells><DataArray type="Int64" Name="connectivity">0 1 2 3</DataArray>
<DataArray type="Int64" Name="offsets">4</DataArray><DataArray type="UInt8" Name="types">10</DataArray></Cells>
</Piece></UnstructuredGrid></VTKFile>"#,
        )
        .unwrap();
        let t = Tokens::builtin().unwrap();
        let s = Surface::build(&mesh, &t, &Display::default());
        assert_eq!(s.triangles(), 4);
        assert_eq!(s.edges(), 6);
        assert_eq!(s.origin, [0.5; 3]);
        assert_eq!(s.faces, 4);
        // Every edge of a tetrahedron is a crease (70.5 degrees).
        let feat = Display {
            render_mode: RenderMode::FeatureEdges,
            ..Display::default()
        };
        assert_eq!(Surface::build(&mesh, &t, &feat).edges(), 6);
        for n in &s.normals {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((l - 1.0).abs() < 1e-5);
        }
    }
}
