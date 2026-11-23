//! The boundary of a volume mesh that has no boundary polygons: the faces
//! used by exactly one cell, found by their sorted point ids.

use std::collections::HashMap;

use crate::{Mesh, Patch, PatchRole, Polygons};

pub(crate) fn extract(mesh: &mut Mesh) {
    let mut seen: HashMap<Vec<u32>, (u32, u32)> = HashMap::with_capacity(mesh.faces.len());
    for f in 0..mesh.faces.len() {
        let mut key = mesh.faces.get(f).to_vec();
        key.sort_unstable();
        seen.entry(key)
            .and_modify(|e| e.0 += 1)
            .or_insert((1, f as u32));
    }
    let mut once: Vec<u32> = seen
        .into_values()
        .filter(|&(n, _)| n == 1)
        .map(|(_, f)| f)
        .collect();
    once.sort_unstable();
    let mut b = Polygons::new();
    for f in once {
        b.push(mesh.faces.get(f as usize).iter().copied());
    }
    mesh.boundary_patch = vec![0; b.len()];
    mesh.boundary = b;
    mesh.patches = vec![Patch {
        name: "boundary".into(),
        role: PatchRole::Unspecified,
    }];
    mesh.boundary_extracted = true;
}
