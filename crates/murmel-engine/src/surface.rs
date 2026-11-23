//! The outside of the cells shown, as triangles and edge lines ready for
//! the GPU. Built once per mesh and display (colouring, hidden items,
//! clip plane, selection); orbiting never re-triangulates.
//!
//! What is drawn: the boundary faces of the cells shown (in their patch's
//! colour), the faces between a cell shown and one that is not (a hidden
//! zone or cell type, or the clip plane, lays them open; they take the
//! cell's colour), and with a flat clip the section of every cell the
//! plane crosses. So hiding the prism layers shows the polyhedral core,
//! and a crinkle clip shows the cells themselves.
//!
//! Each polygon is a fan about its node average (exact for the
//! star-shaped faces KoflCraft writes, including merged faces, and for
//! the convex sections), with one flat normal from Newell's method.
//! Positions are relative to `origin` (the centre of the bounds) so they
//! keep their precision in `f32`.

use std::collections::{HashMap, HashSet};

use murmel_mesh::{Bounds, Mesh, NO_CELL};

use crate::clip::{Clip, ClipMode};
use crate::display::{Display, Item, RenderMode};

/// Feature edges (render mode 4) bend by more than this angle between
/// their two faces (UI spec, Display).
pub const FEATURE_ANGLE_DEG: f64 = 30.0;
use crate::theme::{Rgb, Tokens};

/// The feature-edge group of a face that is on no patch (laid open by the
/// clip or a hidden item), and of a section.
const GROUP_OPEN: u32 = u32::MAX - 1;
const GROUP_SECTION: u32 = u32::MAX - 2;

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
    pub indices: Vec<u32>,
    /// Per triangle: the cell it shows (the owner of a boundary face, the
    /// cell shown beside an open face, the cell of a section;
    /// [`NO_CELL`] when unknown) and the distinct face (`face:<n>`, or
    /// [`NO_CELL`] for a section). Picking reads these.
    pub tri_cell: Vec<u32>,
    pub tri_face: Vec<u32>,
    /// Edge line vertices, two per edge, with their linear colour.
    pub edge_positions: Vec<[f32; 3]>,
    pub edge_colors: Vec<[f32; 3]>,
}

/// A corner of a drawn polygon: a mesh node, or the point where the clip
/// plane crosses the edge between two nodes (smaller id first).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum V {
    Node(u32),
    Cut(u32, u32),
}

fn cut(a: u32, b: u32) -> V {
    V::Cut(a.min(b), a.max(b))
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

/// The clip plane with its per-node side and distance.
struct Cutter<'a> {
    clip: &'a Clip,
    keep: Vec<bool>,
    dist: Vec<f64>,
}

impl Cutter<'_> {
    fn keeps(&self, v: V) -> bool {
        match v {
            V::Node(i) => self.keep[i as usize],
            V::Cut(..) => true,
        }
    }

    /// The part of `poly` on the kept side (Sutherland-Hodgman).
    fn clip_poly(&self, verts: &[V]) -> Vec<V> {
        let mut out = Vec::with_capacity(verts.len() + 2);
        for i in 0..verts.len() {
            let (a, b) = (verts[i], verts[(i + 1) % verts.len()]);
            let (ka, kb) = (self.keeps(a), self.keeps(b));
            if ka {
                out.push(a);
            }
            if ka != kb
                && let (V::Node(x), V::Node(y)) = (a, b)
            {
                out.push(cut(x, y));
            }
        }
        out
    }

    /// The section of cell `c` by the plane: closed loops of cut points.
    /// In each face the kept part runs from the cut where it leaves the
    /// kept side to the next cut where it comes back; those segments,
    /// joined at the shared cut points, close around the cell.
    fn section(&self, mesh: &Mesh, c: usize) -> Vec<Vec<V>> {
        let mut next: HashMap<V, Vec<V>> = HashMap::new();
        for face in mesh.cell(c) {
            let n = face.len();
            let mut cuts: Vec<(V, bool)> = Vec::new();
            for i in 0..n {
                let (a, b) = (face[i], face[(i + 1) % n]);
                let (ka, kb) = (self.keep[a as usize], self.keep[b as usize]);
                if ka != kb {
                    cuts.push((cut(a, b), ka));
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
}

impl Surface {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn edges(&self) -> usize {
        self.edge_positions.len() / 2
    }

    /// Triangulate the outside of the cells of `mesh` that `display`
    /// shows, in the colours it asks for.
    pub fn build(mesh: &Mesh, tokens: &Tokens, display: &Display) -> Surface {
        let bounds = mesh.bounds();
        let origin = bounds.center();
        let mut s = Surface {
            origin,
            radius: bounds.radius(),
            bounds: (!bounds.is_empty()).then_some(bounds),
            ..Surface::default()
        };
        let cutter = display.active_clip().map(|clip| {
            let dist: Vec<f64> = mesh.points.iter().map(|&p| clip.distance(p)).collect();
            Cutter {
                clip,
                keep: dist.iter().map(|&d| d <= 0.0).collect(),
                dist,
            }
        });
        let shown = display.shown_cells(mesh);
        s.cells = shown.iter().filter(|&&v| v).count();
        let polys = polygons(mesh, tokens, display, &shown, cutter.as_ref());

        let pos = |v: V| -> [f64; 3] {
            let p = match v {
                V::Node(i) => mesh.points[i as usize],
                V::Cut(a, b) => {
                    let (pa, pb) = (mesh.points[a as usize], mesh.points[b as usize]);
                    let (da, db) = cutter
                        .as_ref()
                        .map_or((0.0, 1.0), |c| (c.dist[a as usize], c.dist[b as usize]));
                    let t = if da == db { 0.0 } else { da / (da - db) };
                    [0, 1, 2].map(|k| pa[k] + t * (pb[k] - pa[k]))
                }
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
        let mut seen: HashSet<(V, V)> = HashSet::new();
        let pts: Vec<Vec<[f64; 3]>> = polys
            .iter()
            .map(|p| p.verts.iter().map(|&v| pos(v)).collect())
            .collect();
        let normals: Vec<[f32; 3]> = pts.iter().map(|p| newell(p)).collect();
        let features = (display.render_mode == RenderMode::FeatureEdges).then(|| {
            feature_edges(
                polys
                    .iter()
                    .zip(&normals)
                    .map(|(p, n)| (p.verts.as_slice(), p.group, *n)),
                FEATURE_ANGLE_DEG.to_radians().cos(),
            )
        });
        // Lines alone are drawn in the face colour; over faces, darker.
        let lines_only = !display.render_mode.faces();
        for (unselected, _, i) in order {
            let poly = &polys[i];
            let rgb = poly.color;
            let (color, ec) = if unselected {
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
            let push = |s: &mut Surface, p: [f64; 3]| {
                s.positions.push(p.map(|v| v as f32));
                s.normals.push(n);
                s.colors.push(color);
            };
            let tris_before = s.indices.len() / 3;
            if pts.len() == 3 {
                for &p in pts {
                    push(&mut s, p);
                }
                s.indices.extend([base, base + 1, base + 2]);
            } else {
                let k = pts.len() as f64;
                let c = [0, 1, 2].map(|d| pts.iter().map(|p| p[d]).sum::<f64>() / k);
                push(&mut s, c);
                for &p in pts {
                    push(&mut s, p);
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
        s.visible_bounds = (!vb.is_empty()).then_some(vb);
        s
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
    let flat = cutter.filter(|c| c.clip.mode == ClipMode::Flat);
    let mut polys: Vec<Poly> = Vec::new();
    let mut add = |verts: Vec<V>, cell: u32, face: u32, boundary: u32, color: Rgb, group: u32| {
        let verts = match flat {
            Some(c) => c.clip_poly(&verts),
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
        if !display.face_visible(mesh, b)
            || cutter.is_some_and(|c| !face.iter().any(|&i| c.keep[i as usize]))
        {
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
        if so == sn {
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
    if let Some(cutter) = flat {
        for (c, &on) in shown.iter().enumerate() {
            if !on
                || mesh
                    .cell(c)
                    .all(|f| f.iter().all(|&i| cutter.keep[i as usize]))
            {
                continue;
            }
            let color = display.cell_color(mesh, tokens, c);
            for ring in cutter.section(mesh, c) {
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

/// Whether `poly` belongs to the selected item.
fn selected(mesh: &Mesh, display: &Display, poly: &Poly) -> bool {
    let cell = (poly.cell != NO_CELL).then_some(poly.cell as usize);
    match display.selection {
        None | Some(Item::Mesh) => false,
        Some(Item::Patch(p)) => {
            poly.boundary != NO_CELL && mesh.boundary_patch.get(poly.boundary as usize) == Some(&p)
        }
        Some(Item::Zone(z)) => cell.is_some_and(|c| mesh.cell_zone.get(c) == Some(&z)),
        Some(Item::Kind(k)) => cell.is_some_and(|c| mesh.cell_kind.get(c) == Some(&k)),
    }
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
) -> HashSet<(V, V)> {
    struct Seen {
        normal: [f32; 3],
        group: u32,
        count: u32,
        feature: bool,
    }
    let mut edges: HashMap<(V, V), Seen> = HashMap::new();
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
            clip: Some(Clip {
                origin: b.center(),
                normal: [0.0, 1.0, 0.0],
                mode,
                enabled: true,
            }),
            ..Display::default()
        }
    }

    /// Every polygon edge of a display, counted by use (zero-length edges
    /// at nodes on the plane left out).
    fn edge_uses(mesh: &Mesh, d: &Display) -> HashMap<(V, V), usize> {
        let t = Tokens::builtin().unwrap();
        let shown = d.shown_cells(mesh);
        let cutter = d.active_clip().map(|clip| {
            let dist: Vec<f64> = mesh.points.iter().map(|&p| clip.distance(p)).collect();
            Cutter {
                clip,
                keep: dist.iter().map(|&x| x <= 0.0).collect(),
                dist,
            }
        });
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

    /// Invariant: what the viewport draws is a closed surface, every edge
    /// shared by exactly two polygons, for the whole channel, with the
    /// prism layers hidden (the core's faces laid open), and with a
    /// crinkle or a flat clip (the sections close the cut). A wrong
    /// neighbour in the face table, a section that does not close, or a
    /// cut point named differently by two faces would leave a hole.
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
        ] {
            let uses = edge_uses(&m, &d);
            assert!(!uses.is_empty(), "{name}");
            let open: Vec<_> = uses.iter().filter(|(_, n)| **n != 2).take(3).collect();
            assert!(open.is_empty(), "{name}: {open:?}");
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
