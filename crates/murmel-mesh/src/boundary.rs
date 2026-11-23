//! Face topology from the cells: the distinct faces with their owner and
//! neighbour cells, the faces used by exactly one cell (the boundary of a
//! file that has no boundary polygons), the owner cell of every boundary
//! polygon and the distinct face it is. Faces are matched by their sorted
//! point ids, so a face matches whatever node it starts at and whichever
//! way it turns.

use std::collections::HashMap;

use crate::{Mesh, Patch, PatchRole, Polygons};

/// No cell (or no face) is known.
pub const NO_CELL: u32 = u32::MAX;

fn key(nodes: &[u32]) -> Vec<u32> {
    let mut k = nodes.to_vec();
    k.sort_unstable();
    k
}

/// Fill the face topology of `mesh` (`cell_face_id`, `face_owner`,
/// `face_neighbour`, `face_first`, `boundary_owner`, `boundary_face_id`,
/// `face_boundary`, `unique_faces`) and extract the boundary when the file
/// had none.
pub(crate) fn link(mesh: &mut Mesh) {
    let cells = mesh.cell_count();
    if cells == 0 {
        let n = mesh.boundary.len();
        mesh.boundary_owner = vec![NO_CELL; n];
        mesh.unique_faces = n;
        mesh.cell_face_id = Vec::new();
        mesh.face_owner = vec![NO_CELL; n];
        mesh.face_neighbour = vec![NO_CELL; n];
        mesh.face_first = vec![NO_CELL; n];
        mesh.boundary_face_id = (0..n as u32).collect();
        mesh.face_boundary = (0..n as u32).collect();
        return;
    }
    let mut ids: HashMap<Vec<u32>, u32> = HashMap::with_capacity(mesh.faces.len() / 2 + 1);
    let mut cell_face_id = Vec::with_capacity(mesh.faces.len());
    let (mut owner, mut neighbour, mut first) = (Vec::new(), Vec::new(), Vec::new());
    let mut uses: Vec<u8> = Vec::new();
    for c in 0..cells {
        for f in mesh.cell_faces[c]..mesh.cell_faces[c + 1] {
            let next = owner.len() as u32;
            let u = *ids.entry(key(mesh.faces.get(f as usize))).or_insert(next);
            if u == next {
                owner.push(c as u32);
                neighbour.push(NO_CELL);
                first.push(f);
                uses.push(1);
            } else {
                let i = u as usize;
                uses[i] = uses[i].saturating_add(1);
                if neighbour[i] == NO_CELL && owner[i] != c as u32 {
                    neighbour[i] = c as u32;
                }
            }
            cell_face_id.push(u);
        }
    }
    mesh.unique_faces = owner.len();
    let n_unique = owner.len();
    if mesh.boundary.is_empty() {
        let mut b = Polygons::new();
        let mut bid = Vec::new();
        let mut bowner = Vec::new();
        for u in 0..n_unique {
            if uses[u] == 1 {
                b.push(mesh.faces.get(first[u] as usize).iter().copied());
                bid.push(u as u32);
                bowner.push(owner[u]);
            }
        }
        mesh.boundary_patch = vec![0; b.len()];
        mesh.boundary_owner = bowner;
        mesh.boundary_face_id = bid;
        mesh.boundary = b;
        mesh.patches = vec![Patch {
            name: "boundary".into(),
            role: PatchRole::Unspecified,
        }];
        mesh.boundary_extracted = true;
    } else {
        mesh.boundary_face_id = mesh
            .boundary
            .iter()
            .map(|f| ids.get(&key(f)).copied().unwrap_or(NO_CELL))
            .collect();
        mesh.boundary_owner = mesh
            .boundary_face_id
            .iter()
            .map(|&u| {
                if u == NO_CELL {
                    NO_CELL
                } else {
                    owner[u as usize]
                }
            })
            .collect();
    }
    let mut face_boundary = vec![NO_CELL; n_unique];
    for (b, &u) in mesh.boundary_face_id.iter().enumerate() {
        if u != NO_CELL {
            face_boundary[u as usize] = b as u32;
        }
    }
    mesh.cell_face_id = cell_face_id;
    mesh.face_owner = owner;
    mesh.face_neighbour = neighbour;
    mesh.face_first = first;
    mesh.face_boundary = face_boundary;
}
