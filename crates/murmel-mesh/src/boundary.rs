//! Face topology from the cells: the distinct faces with their owner and
//! neighbour cells, the faces used by exactly one cell (the boundary of a
//! file that has no boundary polygons), the owner cell of every boundary
//! polygon and the distinct face it is. Faces are matched by their sorted
//! point ids, so a face matches whatever node it starts at and whichever
//! way it turns.

use crate::{Mesh, Patch, PatchRole, Polygons, par};

/// No cell (or no face) is known.
pub const NO_CELL: u32 = u32::MAX;

/// The cell faces grouped by their smallest point id: faces with the same
/// points have the same smallest one, so a face's twin is among the few
/// faces of its group (about two per point in a polyhedral mesh). This
/// replaces a hash map keyed by sorted point lists, which allocated a key
/// per face.
struct Buckets {
    /// `slot_face[head[p]..head[p + 1]]` are the faces whose smallest
    /// point is `p`, in face order; the last group holds empty faces.
    head: Vec<u32>,
    slot_face: Vec<u32>,
}

impl Buckets {
    fn new(faces: &Polygons, points: usize) -> Buckets {
        let n = faces.len();
        let smallest: Vec<u32> = par::collect(n, 1 << 15, |r| {
            r.map(|f| {
                let m = faces.get(f).iter().copied().min().unwrap_or(u32::MAX);
                m.min(points as u32)
            })
            .collect()
        });
        let mut head = vec![0u32; points + 2];
        for &m in &smallest {
            head[m as usize + 1] += 1;
        }
        for p in 0..=points {
            head[p + 1] += head[p];
        }
        let mut next = head.clone();
        let mut slot_face = vec![0u32; n];
        for (f, &m) in smallest.iter().enumerate() {
            let s = &mut next[m as usize];
            slot_face[*s as usize] = f as u32;
            *s += 1;
        }
        Buckets { head, slot_face }
    }

    fn group(&self, p: usize) -> &[u32] {
        &self.slot_face[self.head[p] as usize..self.head[p + 1] as usize]
    }
}

/// Whether two faces have the same points (in any order and turn).
fn same_points(a: &[u32], b: &[u32], sa: &mut Vec<u32>, sb: &mut Vec<u32>) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let sum = |f: &[u32]| f.iter().map(|&v| u64::from(v)).sum::<u64>();
    if sum(a) != sum(b) {
        return false;
    }
    sa.clear();
    sa.extend_from_slice(a);
    sa.sort_unstable();
    sb.clear();
    sb.extend_from_slice(b);
    sb.sort_unstable();
    sa == sb
}

/// For every cell face, the first cell face (in face order) with the
/// same points: itself when it is the first.
fn first_twins(faces: &Polygons, buckets: &Buckets, points: usize) -> Vec<u32> {
    let per_slot: Vec<u32> = par::collect(points + 1, 1 << 12, |r| {
        let (s0, s1) = (buckets.head[r.start] as usize, buckets.head[r.end] as usize);
        let mut out: Vec<u32> = Vec::with_capacity(s1 - s0);
        let (mut sa, mut sb) = (Vec::new(), Vec::new());
        for p in r {
            let group = buckets.group(p);
            let base = out.len();
            for (i, &f) in group.iter().enumerate() {
                let face = faces.get(f as usize);
                let twin = (0..i)
                    .find(|&j| same_points(face, faces.get(group[j] as usize), &mut sa, &mut sb))
                    .map_or(f, |j| out[base + j]);
                out.push(twin);
            }
        }
        out
    });
    let mut first = vec![0u32; per_slot.len()];
    for (s, &t) in per_slot.iter().enumerate() {
        first[buckets.slot_face[s] as usize] = t;
    }
    first
}

/// Whether `b` lists the points of `a` the other way round (`a` and `b`
/// hold the same points; a face of fewer than three points has no turn).
fn turned(a: &[u32], b: &[u32]) -> bool {
    let n = b.len();
    if a.len() < 3 || n != a.len() {
        return false;
    }
    match b.iter().position(|&p| p == a[0]) {
        Some(k) => b[(k + 1) % n] != a[1],
        None => false,
    }
}

/// Fill the face topology of `mesh` from the faces of its cells, `polys`
/// (cell `c` has `polys` `cell_faces[c]..cell_faces[c + 1]`, as the file
/// lists them): the distinct faces, each stored once as its owner lists
/// it (`distinct`), `cell_face_id`, `cell_face_flip`, `face_owner`,
/// `face_neighbour`, `boundary_owner`, `boundary_face_id`,
/// `face_boundary` and `unique_faces`; and extract the boundary when the
/// file had none.
pub(crate) fn link(mesh: &mut Mesh, polys: Polygons) {
    let cells = mesh.cell_count();
    if cells == 0 {
        let n = mesh.boundary.len();
        mesh.distinct = Polygons::new();
        mesh.boundary_owner = vec![NO_CELL; n];
        mesh.unique_faces = n;
        mesh.cell_face_id = Vec::new();
        mesh.cell_face_flip = Vec::new();
        mesh.face_owner = vec![NO_CELL; n];
        mesh.face_neighbour = vec![NO_CELL; n];
        mesh.boundary_face_id = (0..n as u32).collect();
        mesh.face_boundary = (0..n as u32).collect();
        return;
    }
    let points = mesh.points.len();
    let buckets = Buckets::new(&polys, points);
    let twin = first_twins(&polys, &buckets, points);
    // Distinct faces numbered in the order their first cell face comes;
    // the owner is the first cell that uses a face, and its listing is
    // the one stored.
    let mut cell_face_id = vec![NO_CELL; polys.len()];
    let mut cell_face_flip = vec![false; polys.len()];
    let (mut owner, mut neighbour, mut first) = (Vec::new(), Vec::new(), Vec::new());
    let mut uses: Vec<u8> = Vec::new();
    for c in 0..cells {
        for f in mesh.cell_faces[c]..mesh.cell_faces[c + 1] {
            let t = twin[f as usize];
            let u = if t == f {
                let u = owner.len() as u32;
                owner.push(c as u32);
                neighbour.push(NO_CELL);
                first.push(f);
                uses.push(1);
                u
            } else {
                let u = cell_face_id[t as usize];
                let i = u as usize;
                uses[i] = uses[i].saturating_add(1);
                if neighbour[i] == NO_CELL && owner[i] != c as u32 {
                    neighbour[i] = c as u32;
                }
                u
            };
            cell_face_id[f as usize] = u;
        }
    }
    drop(twin);
    mesh.unique_faces = owner.len();
    let n_unique = owner.len();
    // Which cell faces list their distinct face the other way round.
    let (ids, firsts, cell_polys) = (&cell_face_id, &first, &polys);
    par::chunks_mut(&mut cell_face_flip, 1 << 15, |at, part| {
        for (k, flip) in part.iter_mut().enumerate() {
            let f = at + k;
            let owners = firsts[ids[f] as usize] as usize;
            *flip = owners != f && turned(cell_polys.get(owners), cell_polys.get(f));
        }
    });
    // The distinct faces as their owners list them, copied in parallel.
    let mut start = Vec::with_capacity(n_unique + 1);
    start.push(0u32);
    let mut at = 0u32;
    for &f in &first {
        at += polys.start[f as usize + 1] - polys.start[f as usize];
        start.push(at);
    }
    let mut nodes = vec![0u32; at as usize];
    let ranges = par::ranges(n_unique, 1 << 15);
    let cuts: Vec<usize> = ranges
        .iter()
        .map(|r| start[r.start] as usize)
        .chain([at as usize])
        .collect();
    let parts = par::split_mut(&mut nodes, &cuts);
    let starts = &start;
    std::thread::scope(|sc| {
        for (r, part) in ranges.iter().zip(parts) {
            let base = starts[r.start] as usize;
            sc.spawn(move || {
                for u in r.clone() {
                    let src = cell_polys.get(firsts[u] as usize);
                    let a = starts[u] as usize - base;
                    part[a..a + src.len()].copy_from_slice(src);
                }
            });
        }
    });
    let distinct = Polygons { start, nodes };
    if mesh.boundary.is_empty() {
        let mut b = Polygons::new();
        let mut bid = Vec::new();
        let mut bowner = Vec::new();
        for u in 0..n_unique {
            if uses[u] == 1 {
                b.push(distinct.get(u).iter().copied());
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
        let (faces, boundary, ids) = (&polys, &mesh.boundary, &cell_face_id);
        mesh.boundary_face_id = par::collect(boundary.len(), 1 << 12, |r| {
            let (mut sa, mut sb) = (Vec::new(), Vec::new());
            r.map(|b| {
                let poly = boundary.get(b);
                let Some(m) = poly.iter().copied().min() else {
                    return NO_CELL;
                };
                if m as usize >= points {
                    return NO_CELL;
                }
                buckets
                    .group(m as usize)
                    .iter()
                    .find(|&&f| same_points(poly, faces.get(f as usize), &mut sa, &mut sb))
                    .map_or(NO_CELL, |&f| ids[f as usize])
            })
            .collect()
        });
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
    mesh.distinct = distinct;
    mesh.cell_face_id = cell_face_id;
    mesh.cell_face_flip = cell_face_flip;
    mesh.face_owner = owner;
    mesh.face_neighbour = neighbour;
    mesh.face_boundary = face_boundary;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// The topology by a hash map of sorted point lists (the method this
    /// module used before the buckets): cell-face ids, owners,
    /// neighbours, first cell faces and boundary matches.
    #[allow(clippy::type_complexity)]
    fn reference(m: &Mesh, polys: &Polygons) -> (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>) {
        let key = |f: &[u32]| {
            let mut k = f.to_vec();
            k.sort_unstable();
            k
        };
        let mut ids: HashMap<Vec<u32>, u32> = HashMap::new();
        let (mut ids_of, mut owner, mut nei, mut first) = (vec![], vec![], vec![], vec![]);
        for c in 0..m.cell_count() {
            for f in m.cell_faces[c]..m.cell_faces[c + 1] {
                let next = owner.len() as u32;
                let u = *ids.entry(key(polys.get(f as usize))).or_insert(next);
                if u == next {
                    owner.push(c as u32);
                    nei.push(NO_CELL);
                    first.push(f);
                } else if nei[u as usize] == NO_CELL && owner[u as usize] != c as u32 {
                    nei[u as usize] = c as u32;
                }
                ids_of.push(u);
            }
        }
        let bound = m
            .boundary
            .iter()
            .map(|f| ids.get(&key(f)).copied().unwrap_or(NO_CELL))
            .collect();
        (ids_of, owner, nei, first, bound)
    }

    /// Whether `a` is `b` started at another point.
    fn same_cycle(a: &[u32], b: &[u32]) -> bool {
        a.len() == b.len()
            && (0..a.len().max(1)).any(|k| a.iter().cycle().skip(k).zip(b).all(|(x, y)| x == y))
    }

    /// Invariant: matching faces in groups of their smallest point gives
    /// exactly the topology of the hash map it replaced, on KoflCraft
    /// meshes and on the honeycomb with every face starting at another
    /// corner, at one and at several threads; each distinct face is
    /// stored once as its owner lists it, and every cell reads back its
    /// faces in its own turn. A face compared by its first point only, a
    /// group cut between threads or a wrong flip would number faces
    /// differently, miss twins, or turn a neighbour's face inside out (its
    /// volume and sections would change; the inspect and surface tests
    /// see only the meshes as a whole).
    #[test]
    fn buckets_match_the_hash_map() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures");
        let mut meshes = vec![
            crate::read(&fixtures.join("channel/foehncraft.vtu")).unwrap(),
            crate::read(&fixtures.join("box/foehncraft.vtu")).unwrap(),
            crate::read(&fixtures.join("openfoam/box")).unwrap(),
        ];
        meshes.push(crate::synth::honeycomb(5));
        for (i, m) in meshes.iter_mut().enumerate() {
            let mut polys = m.cell_polygons();
            if i == 3 {
                for f in 0..polys.len() {
                    let (a, b) = (polys.start[f] as usize, polys.start[f + 1] as usize);
                    polys.nodes[a..b].rotate_left(f % (b - a));
                }
            }
            for threads in [1, 3] {
                // The threads only steer how work is split; both must agree.
                let r = crate::par::with_threads(threads, || {
                    link(m, polys.clone());
                    reference(m, &polys)
                });
                assert_eq!(r.0, m.cell_face_id);
                assert_eq!(r.1, m.face_owner);
                assert_eq!(r.2, m.face_neighbour);
                for (u, &f) in r.3.iter().enumerate() {
                    assert_eq!(m.distinct.get(u), polys.get(f as usize));
                }
                assert_eq!(r.4, m.boundary_face_id);
                let mut f = 0;
                for c in 0..m.cell_count() {
                    for face in m.cell(c) {
                        let own: Vec<u32> = face.oriented().collect();
                        assert!(same_cycle(&own, polys.get(f)), "cell {c} face {f}");
                        f += 1;
                    }
                }
            }
        }
    }
}
