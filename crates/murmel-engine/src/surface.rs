//! The boundary of a mesh as triangles and edge lines, ready for the GPU.
//! Built once per mesh and display (colouring, hidden items, selection);
//! orbiting never re-triangulates.
//!
//! Each boundary polygon is a fan about its node average (exact for the
//! star-shaped faces KoflCraft writes, including merged faces), with one
//! flat normal from Newell's method. Positions are relative to `origin`
//! (the centre of the bounds) so they keep their precision in `f32`.

use std::collections::HashSet;

use murmel_mesh::{Bounds, Mesh, NO_CELL};

use crate::display::{ColorBy, Display};
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
        let face_rgb = |f: usize| -> Rgb {
            let patch = mesh.boundary_patch.get(f).copied().unwrap_or(0) as usize;
            match display.color_by {
                ColorBy::Patch => tokens.patch_color(patch),
                ColorBy::Role => tokens.role_color(
                    mesh.patches
                        .get(patch)
                        .map_or(murmel_mesh::PatchRole::Unspecified, |p| p.role),
                ),
                ColorBy::CellType => match mesh.boundary_owner.get(f).copied() {
                    Some(c) if c != NO_CELL => mesh
                        .cell_kind
                        .get(c as usize)
                        .map_or(tokens.mesh.role.unspecified, |&k| tokens.kind_color(k)),
                    _ => tokens.mesh.role.unspecified,
                },
            }
        };
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
        for (unselected, f) in order {
            let face = mesh.boundary.get(f);
            let rgb = face_rgb(f);
            let (color, ec) = if unselected {
                (
                    rgb.linear(),
                    Rgb(rgb.0.map(|v| (v as f32 * darken).round() as u8)).linear(),
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
                if seen.insert((a.min(b), a.max(b))) {
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
        for n in &s.normals {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((l - 1.0).abs() < 1e-5);
        }
    }
}
