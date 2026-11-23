//! The boundary of a mesh as triangles and edge lines, ready for the GPU.
//! Built once per mesh and display (colouring, hidden items, selection);
//! orbiting never re-triangulates.
//!
//! Each boundary polygon is a fan about its node average (exact for the
//! star-shaped faces KoflCraft writes, including merged faces), with one
//! flat normal from Newell's method. Positions are relative to `origin`
//! (the centre of the bounds) so they keep their precision in `f32`.

use std::collections::{HashMap, HashSet};

use murmel_mesh::{Bounds, Mesh};

use crate::display::{Display, RenderMode};

/// Feature edges (render mode 4) bend by more than this angle between
/// their two faces (UI spec, Display).
pub const FEATURE_ANGLE_DEG: f64 = 30.0;
use crate::theme::{Rgb, Tokens};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Surface {
    pub origin: [f64; 3],
    pub radius: f64,
    pub bounds: Option<Bounds>,
    /// Bounds of the faces drawn (hidden items left out).
    pub visible_bounds: Option<Bounds>,
    /// Boundary faces drawn, and the cells shown (counted once per build
    /// so the status bar does not walk the mesh every frame).
    pub faces: usize,
    pub cells: usize,
    /// Triangle vertices: position, normal, linear colour.
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// Edge line vertices, two per edge, with their linear colour.
    pub edge_positions: Vec<[f32; 3]>,
    pub edge_colors: Vec<[f32; 3]>,
}

impl Surface {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn edges(&self) -> usize {
        self.edge_positions.len() / 2
    }

    /// Triangulate the boundary faces of `mesh` that `display` shows, in
    /// the colours it asks for.
    pub fn build(mesh: &Mesh, tokens: &Tokens, display: &Display) -> Surface {
        let bounds = mesh.bounds();
        let origin = bounds.center();
        let mut s = Surface {
            origin,
            radius: bounds.radius(),
            bounds: (!bounds.is_empty()).then_some(bounds),
            ..Surface::default()
        };
        let rel = |i: u32| -> [f64; 3] {
            let p = mesh.points[i as usize];
            [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]]
        };
        let darken = 1.0 - tokens.mesh.edge_darken;
        let sel = tokens.mesh.selection;
        let mix = tokens.mesh.selection_mix.clamp(0.0, 1.0);
        let face_rgb = |f: usize| -> Rgb { display.face_color(mesh, tokens, f) };
        // Selected faces first, so the edges they share with others take
        // the selection colour.
        let mut order: Vec<(bool, usize)> = (0..mesh.boundary.len())
            .filter(|&f| display.face_visible(mesh, f))
            .map(|f| (!display.face_selected(mesh, f), f))
            .collect();
        order.sort_unstable();
        s.faces = order.len();
        s.cells = if display.hidden.is_empty() {
            mesh.cell_count()
        } else {
            (0..mesh.cell_count())
                .filter(|&c| display.cell_visible(mesh, c))
                .count()
        };
        let mut vb = Bounds::empty();
        let mut seen: HashSet<(u32, u32)> = HashSet::new();
        let features = (display.render_mode == RenderMode::FeatureEdges).then(|| {
            feature_edges(
                mesh,
                order.iter().map(|&(_, f)| f),
                FEATURE_ANGLE_DEG.to_radians().cos(),
            )
        });
        // Lines alone are drawn in the face colour; over faces, darker.
        let lines_only = !display.render_mode.faces();
        for (unselected, f) in order {
            let face = mesh.boundary.get(f);
            let rgb = face_rgb(f);
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
            for &i in face {
                vb.add(mesh.points[i as usize]);
            }
            let pts: Vec<[f64; 3]> = face.iter().map(|&i| rel(i)).collect();
            let n = newell(&pts);
            let base = s.positions.len() as u32;
            let push = |s: &mut Surface, p: [f64; 3]| {
                s.positions.push(p.map(|v| v as f32));
                s.normals.push(n);
                s.colors.push(color);
            };
            if pts.len() == 3 {
                for &p in &pts {
                    push(&mut s, p);
                }
                s.indices.extend([base, base + 1, base + 2]);
            } else {
                let k = pts.len() as f64;
                let c = [0, 1, 2].map(|d| pts.iter().map(|p| p[d]).sum::<f64>() / k);
                push(&mut s, c);
                for &p in &pts {
                    push(&mut s, p);
                }
                let m = pts.len() as u32;
                for i in 0..m {
                    s.indices
                        .extend([base, base + 1 + i, base + 1 + (i + 1) % m]);
                }
            }
            for i in 0..face.len() {
                let (a, b) = (face[i], face[(i + 1) % face.len()]);
                let key = (a.min(b), a.max(b));
                if features.as_ref().is_some_and(|fe| !fe.contains(&key)) {
                    continue;
                }
                if seen.insert(key) {
                    s.edge_positions.push(rel(a).map(|v| v as f32));
                    s.edge_positions.push(rel(b).map(|v| v as f32));
                    s.edge_colors.push(ec);
                    s.edge_colors.push(ec);
                }
            }
        }
        s.visible_bounds = (!vb.is_empty()).then_some(vb);
        s
    }
}

/// The feature edges among the boundary faces `faces` of `mesh`: an edge
/// whose two faces meet at a crease (the cosine of the angle between their
/// normals below `cos_max`), whose faces lie on different patches, or
/// that has only one face drawn (an open border, also where hidden items
/// were cut away). The normals' orientation is not trusted, so a crease is
/// judged on the absolute cosine.
pub fn feature_edges(
    mesh: &Mesh,
    faces: impl Iterator<Item = usize>,
    cos_max: f64,
) -> HashSet<(u32, u32)> {
    struct Seen {
        normal: [f32; 3],
        patch: u32,
        count: u32,
        feature: bool,
    }
    let mut edges: HashMap<(u32, u32), Seen> = HashMap::new();
    for f in faces {
        let face = mesh.boundary.get(f);
        let pts: Vec<[f64; 3]> = face.iter().map(|&i| mesh.points[i as usize]).collect();
        let n = newell(&pts);
        let patch = mesh.boundary_patch.get(f).copied().unwrap_or(0);
        for i in 0..face.len() {
            let (a, b) = (face[i], face[(i + 1) % face.len()]);
            let e = edges.entry((a.min(b), a.max(b))).or_insert(Seen {
                normal: n,
                patch,
                count: 0,
                feature: false,
            });
            if e.count > 0 {
                let dot: f32 = (0..3).map(|k| e.normal[k] * n[k]).sum();
                e.feature |= e.patch != patch || (dot.abs() as f64) < cos_max;
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
