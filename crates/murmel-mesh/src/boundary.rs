//! Face topology from the cells: the distinct faces with their owner and
//! neighbour cells, the faces used by exactly one cell (the boundary of a
//! file that has no boundary polygons), the owner cell of every boundary
//! polygon and the distinct face it is. Faces are matched by their sorted
//! point ids, so a face matches whatever node it starts at and whichever
//! way it turns.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

use crate::{FaceList, Mesh, Patch, PatchRole, Polygons, par};

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
    /// A counting sort by smallest point, on all threads: each run of
    /// faces deals its faces out to parts (consecutive ranges of points)
    /// in face order, and each part then sorts its own faces into its
    /// groups, so every group still lists its faces in face order.
    /// `least` is each face's smallest point when the reader found it
    /// (`u32::MAX` for a face of none); otherwise it is found here.
    fn new(faces: &impl FaceList, points: usize, least: Option<Vec<u32>>) -> Buckets {
        let n = faces.count();
        // Groups 0..=points (the last for empty faces).
        let groups = points + 1;
        let parts = par::ranges(groups, 1 << 14);
        // Every part but the last is `step` groups long.
        let step = parts[0].len().max(1);
        let part = |m: u32| m as usize / step;
        // Each run of faces finds their smallest points, in place, and
        // counts how many of its faces go to each part.
        let runs = par::ranges(n, 1 << 15);
        let run_cuts: Vec<usize> = runs.iter().map(|r| r.start).chain([n]).collect();
        let given = least.as_ref().is_some_and(|l| l.len() == n);
        let mut smallest = match least {
            Some(l) if given => l,
            _ => vec![0u32; n],
        };
        let mut counts = vec![vec![0usize; parts.len()]; runs.len()];
        scoped(
            runs.iter()
                .cloned()
                .zip(par::split_mut(&mut smallest, &run_cuts))
                .zip(counts.iter_mut()),
            |((r, out), count)| {
                for (f, m) in r.zip(out.iter_mut()) {
                    let least = if given {
                        *m
                    } else {
                        faces.face(f).iter().copied().min().unwrap_or(u32::MAX)
                    };
                    *m = least.min(points as u32);
                    count[part(*m)] += 1;
                }
            },
        );
        // The faces dealt out by part, and by run within each part.
        let mut cuts = vec![0];
        let mut part_at = Vec::with_capacity(parts.len() + 1);
        for k in 0..parts.len() {
            part_at.push(*cuts.last().unwrap_or(&0));
            for c in &counts {
                cuts.push(cuts.last().unwrap_or(&0) + c[k]);
            }
        }
        part_at.push(n);
        let mut dealt = vec![0u32; n];
        let mut pieces: Vec<Vec<&mut [u32]>> = runs.iter().map(|_| Vec::new()).collect();
        for (i, piece) in par::split_mut(&mut dealt, &cuts).into_iter().enumerate() {
            pieces[i % runs.len()].push(piece);
        }
        let small = &smallest;
        scoped(runs.iter().cloned().zip(pieces), |(r, mut to)| {
            let mut at = vec![0; to.len()];
            for f in r {
                let k = part(small[f]);
                to[k][at[k]] = f as u32;
                at[k] += 1;
            }
        });
        // Each part sorts its faces into its groups.
        let mut head = vec![0u32; groups + 1];
        let mut slot_face = vec![0u32; n];
        let group_cuts: Vec<usize> = parts.iter().map(|r| r.start).chain([groups]).collect();
        let heads = par::split_mut(&mut head[..groups], &group_cuts);
        let slots = par::split_mut(&mut slot_face, &part_at);
        let (dealt, part_at) = (&dealt, &part_at);
        scoped(
            parts.iter().cloned().zip(heads).zip(slots).enumerate(),
            |(k, ((r, head), slots))| {
                let mine = &dealt[part_at[k]..part_at[k + 1]];
                let mut next = vec![0u32; r.len()];
                for &f in mine {
                    next[small[f as usize] as usize - r.start] += 1;
                }
                let mut at = part_at[k] as u32;
                for (h, c) in head.iter_mut().zip(next.iter_mut()) {
                    *h = at;
                    at += *c;
                    *c = *h - part_at[k] as u32;
                }
                for &f in mine {
                    let g = small[f as usize] as usize - r.start;
                    slots[next[g] as usize] = f;
                    next[g] += 1;
                }
            },
        );
        head[groups] = n as u32;
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
/// same points: itself when it is the first. The groups are matched on
/// all threads, each writing the twins of its own faces.
fn first_twins(faces: &impl FaceList, buckets: &Buckets, points: usize) -> Vec<u32> {
    let mut first = vec![0u32; faces.count()];
    let firsts: &[AtomicU32] = AtomicU32::from_mut_slice(&mut first);
    par::map(points + 1, 1 << 12, |r| {
        let (mut sa, mut sb) = (Vec::new(), Vec::new());
        let mut twins: Vec<u32> = Vec::new();
        for p in r {
            let group = buckets.group(p);
            twins.clear();
            for (i, &f) in group.iter().enumerate() {
                let face = faces.face(f as usize);
                let twin = (0..i)
                    .find(|&j| same_points(face, faces.face(group[j] as usize), &mut sa, &mut sb))
                    .map_or(f, |j| twins[j]);
                twins.push(twin);
                firsts[f as usize].store(twin, Relaxed);
            }
        }
    });
    first
}

/// `f` over `items`, each on a scoped thread of its own (one alone runs
/// here).
fn scoped<T: Send>(items: impl Iterator<Item = T>, f: impl Fn(T) + Sync) {
    let items: Vec<T> = items.collect();
    if items.len() <= 1 {
        items.into_iter().for_each(&f);
        return;
    }
    let f = &f;
    std::thread::scope(|s| {
        let handles: Vec<_> = items.into_iter().map(|x| s.spawn(move || f(x))).collect();
        for h in handles {
            if let Err(p) = h.join() {
                std::panic::resume_unwind(p);
            }
        }
    });
}

/// The distinct faces numbered in the order their first cell face comes,
/// on all threads: each run of cells numbers its own first faces after
/// those of the runs before it, then every cell face takes the number of
/// its first twin. The owner is the cell of the first face; the
/// neighbour the first other cell to use it (the smallest, as cells come
/// in order); `shared` whether any other cell face uses it.
struct Numbered {
    cell_face_id: Vec<u32>,
    owner: Vec<u32>,
    neighbour: Vec<u32>,
    first: Vec<u32>,
    shared: Vec<bool>,
}

fn number(cell_faces: &[u32], twin: &[u32]) -> Numbered {
    let cells = cell_faces.len().saturating_sub(1);
    let runs: Vec<Range<usize>> = par::ranges(cells, 1 << 14);
    let faces_of = |r: &Range<usize>| cell_faces[r.start] as usize..cell_faces[r.end] as usize;
    let counts: Vec<usize> = par::map(cells, 1 << 14, |r| {
        faces_of(&r).filter(|&f| twin[f] as usize == f).count()
    });
    let mut at = vec![0];
    for c in &counts {
        at.push(at.last().unwrap_or(&0) + c);
    }
    let unique = at.last().copied().unwrap_or(0);
    let mut cell_face_id = vec![0u32; twin.len()];
    // Faces of no cell (none in a mesh read whole) have no number.
    let listed = cell_faces.last().map_or(0, |&e| e as usize);
    cell_face_id[listed.min(twin.len())..].fill(NO_CELL);
    let (mut owner, mut first) = (vec![0u32; unique], vec![0u32; unique]);
    let ids: &[AtomicU32] = AtomicU32::from_mut_slice(&mut cell_face_id);
    let parts = par::split_mut(&mut owner, &at)
        .into_iter()
        .zip(par::split_mut(&mut first, &at));
    scoped(
        runs.iter().zip(&at).zip(parts),
        |((r, &base), (owner, first))| {
            let mut u = 0;
            for c in r.clone() {
                for f in cell_faces[c] as usize..cell_faces[c + 1] as usize {
                    if twin[f] as usize == f {
                        ids[f].store((base + u) as u32, Relaxed);
                        owner[u] = c as u32;
                        first[u] = f as u32;
                        u += 1;
                    }
                }
            }
        },
    );
    let mut neighbour = vec![0u32; unique];
    par::chunks_mut(&mut neighbour, 1 << 16, |_, p| p.fill(NO_CELL));
    let mut shared = vec![false; unique];
    let (nb, sh) = (
        AtomicU32::from_mut_slice(&mut neighbour),
        AtomicBool::from_mut_slice(&mut shared),
    );
    let owners = &owner;
    par::map(cells, 1 << 14, |r| {
        for c in r {
            for f in cell_faces[c] as usize..cell_faces[c + 1] as usize {
                let t = twin[f] as usize;
                if t != f {
                    let u = ids[t].load(Relaxed);
                    ids[f].store(u, Relaxed);
                    sh[u as usize].store(true, Relaxed);
                    if owners[u as usize] != c as u32 {
                        nb[u as usize].fetch_min(c as u32, Relaxed);
                    }
                }
            }
        }
    });
    Numbered {
        cell_face_id,
        owner,
        neighbour,
        first,
        shared,
    }
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
/// file had none. `least` is each face's smallest point, when the reader
/// found it (it is found here otherwise).
pub(crate) fn link(mesh: &mut Mesh, polys: impl FaceList, least: Option<Vec<u32>>) {
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
    let t = crate::profile::span("topology.buckets");
    let buckets = Buckets::new(&polys, points, least);
    drop(t);
    let t = crate::profile::span("topology.twins");
    let twin = first_twins(&polys, &buckets, points);
    drop(t);
    let t = crate::profile::span("topology.number");
    let Numbered {
        cell_face_id,
        owner,
        neighbour,
        first,
        shared,
    } = number(&mesh.cell_faces, &twin);
    drop(t);
    let t = crate::profile::span("topology.flip");
    let mut cell_face_flip = vec![false; polys.count()];
    drop(twin);
    mesh.unique_faces = owner.len();
    let n_unique = owner.len();
    // Which cell faces list their distinct face the other way round.
    let (ids, firsts, cell_polys) = (&cell_face_id, &first, &polys);
    par::chunks_mut(&mut cell_face_flip, 1 << 15, |at, part| {
        for (k, flip) in part.iter_mut().enumerate() {
            let f = at + k;
            let owners = firsts[ids[f] as usize] as usize;
            *flip = owners != f && turned(cell_polys.face(owners), cell_polys.face(f));
        }
    });
    drop(t);
    let t = crate::profile::span("topology.distinct");
    // The distinct faces as their owners list them, on all threads: each
    // range of faces adds up its own sizes, then copies its faces after
    // those of the ranges before it.
    let ranges = par::ranges(n_unique, 1 << 15);
    let cuts: Vec<usize> = ranges.iter().map(|r| r.start).chain([n_unique]).collect();
    let mut start = vec![0u32; n_unique + 1];
    let mut sums = vec![0u32; ranges.len()];
    scoped(
        ranges
            .iter()
            .cloned()
            .zip(par::split_mut(&mut start[1..], &cuts))
            .zip(sums.iter_mut()),
        |((r, ends), sum)| {
            let mut at = 0u32;
            for (u, end) in r.zip(ends.iter_mut()) {
                at += cell_polys.face(firsts[u] as usize).len() as u32;
                *end = at;
            }
            *sum = at;
        },
    );
    let mut bases = Vec::with_capacity(sums.len());
    let mut total = 0u32;
    for s in &sums {
        bases.push(total);
        total += s;
    }
    let mut nodes = vec![0u32; total as usize];
    let node_cuts: Vec<usize> = bases
        .iter()
        .map(|&b| b as usize)
        .chain([total as usize])
        .collect();
    scoped(
        ranges
            .iter()
            .cloned()
            .zip(par::split_mut(&mut start[1..], &cuts))
            .zip(par::split_mut(&mut nodes, &node_cuts))
            .zip(&bases),
        |(((r, ends), part), &base)| {
            let mut a = 0;
            for (u, end) in r.zip(ends.iter_mut()) {
                let src = cell_polys.face(firsts[u] as usize);
                part[a..a + src.len()].copy_from_slice(src);
                a += src.len();
                *end += base;
            }
        },
    );
    let distinct = Polygons { start, nodes };
    drop(t);
    let t = crate::profile::span("topology.boundary");
    if mesh.boundary.is_empty() {
        let mut b = Polygons::new();
        let mut bid = Vec::new();
        let mut bowner = Vec::new();
        for u in 0..n_unique {
            if !shared[u] {
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
                    .find(|&&f| same_points(poly, faces.face(f as usize), &mut sa, &mut sb))
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
    drop(t);
    let t = crate::profile::span("topology.rest");
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
    drop(t);
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
    /// corner, at one and at several threads (the faces sorted, matched
    /// and numbered in parts), with the faces' smallest points found here
    /// or handed on as a reader finds them; each distinct face is stored once as its
    /// owner lists it, a boundary found from the cells is the faces used
    /// once, and every cell reads back its faces in its own turn. A face compared by its first point only, a
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
            // Each face's smallest point, as a reader that finds them hands
            // them on.
            let least: Vec<u32> = (0..polys.len())
                .map(|f| polys.get(f).iter().copied().min().unwrap_or(u32::MAX))
                .collect();
            for (threads, given) in [(1, false), (3, false), (1, true), (3, true)] {
                // The threads only steer how work is split, and the smallest
                // points only save finding them; all must agree.
                let least = given.then(|| least.clone());
                let r = crate::par::with_threads(threads, || {
                    link(m, polys.clone(), least);
                    reference(m, &polys)
                });
                assert_eq!(r.0, m.cell_face_id);
                assert_eq!(r.1, m.face_owner);
                assert_eq!(r.2, m.face_neighbour);
                for (u, &f) in r.3.iter().enumerate() {
                    assert_eq!(m.distinct.get(u), polys.get(f as usize));
                }
                assert_eq!(r.4, m.boundary_face_id);
                if m.boundary_extracted {
                    // The boundary is the faces one cell face uses.
                    let mut uses = vec![0; m.unique_faces];
                    for &u in &r.0 {
                        uses[u as usize] += 1;
                    }
                    let once: Vec<u32> = (0..m.unique_faces as u32)
                        .filter(|&u| uses[u as usize] == 1)
                        .collect();
                    assert_eq!(m.boundary_face_id, once);
                }
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
