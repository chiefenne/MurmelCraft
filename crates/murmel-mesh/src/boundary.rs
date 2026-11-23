//! Face topology from the cells: the faces used by exactly one cell (the
//! boundary of a file that has no boundary polygons), the owner cell of
//! every boundary polygon and the number of distinct faces. Faces are
//! matched by their sorted point ids.

use std::collections::HashMap;

use crate::{Mesh, Patch, PatchRole, Polygons};

/// No owner cell is known for a boundary face.
pub const NO_CELL: u32 = u32::MAX;

fn key(nodes: &[u32]) -> Vec<u32> {
    let mut k = nodes.to_vec();
    k.sort_unstable();
    k
}

/// Fill `boundary_owner` and `unique_faces`, and extract the boundary
/// when the file had none.
pub(crate) fn link(mesh: &mut Mesh) {
    let cells = mesh.cell_count();
    if cells == 0 {
        mesh.boundary_owner = vec![NO_CELL; mesh.boundary.len()];
        mesh.unique_faces = mesh.boundary.len();
        return;
    }
    // key -> (uses, first cell-face index, its cell)
    let mut seen: HashMap<Vec<u32>, (u32, u32, u32)> = HashMap::with_capacity(mesh.faces.len());
    for c in 0..cells {
        for f in mesh.cell_faces[c]..mesh.cell_faces[c + 1] {
            seen.entry(key(mesh.faces.get(f as usize)))
                .and_modify(|e| e.0 += 1)
                .or_insert((1, f, c as u32));
        }
    }
    mesh.unique_faces = seen.len();
    if mesh.boundary.is_empty() {
        let mut once: Vec<(u32, u32)> = seen
            .values()
            .filter(|e| e.0 == 1)
            .map(|e| (e.1, e.2))
            .collect();
        once.sort_unstable();
        let mut b = Polygons::new();
        for &(f, _) in &once {
            b.push(mesh.faces.get(f as usize).iter().copied());
        }
        mesh.boundary_patch = vec![0; b.len()];
        mesh.boundary_owner = once.iter().map(|e| e.1).collect();
        mesh.boundary = b;
        mesh.patches = vec![Patch {
            name: "boundary".into(),
            role: PatchRole::Unspecified,
        }];
        mesh.boundary_extracted = true;
        return;
    }
    mesh.boundary_owner = mesh
        .boundary
        .iter()
        .map(|f| seen.get(&key(f)).map_or(NO_CELL, |e| e.2))
        .collect();
}
