//! The wall distance of [`crate::derive`]: the exact distance of each
//! cell centroid to the nearest face of the wall patches, as FoehnCraft
//! writes `wall_distance` (its decisions D48 and D108).
//!
//! The wall faces are split into triangles (a fan about each face's
//! node mean, so a warped polygon is covered as the renderer draws it)
//! and put in a bounding volume hierarchy, split at the median of the
//! longest axis of the triangle centres; a query walks it nearest box
//! first and skips a box farther than the best distance found, with the
//! closest point of a triangle by its Voronoi regions (C. Ericson,
//! "Real-Time Collision Detection", 2005, §5.1.5). The cells are queried
//! on all threads.
//!
//! The walls are the patches the result's case gives a `wall` condition
//! (FoehnCraft's `[boundary.<name>] type = "wall"` in
//! `case.resolved.toml` beside the result), else the patches whose role
//! is wall, or those you choose.

use std::path::Path;

use murmel_mesh::{Mesh, PatchRole};

/// The triangles of a leaf, at most.
const LEAF: usize = 4;

/// The wall patches FoehnCraft's case beside `result` names: those of
/// `[boundary.<name>]` with `type = "wall"`. `None` without the case or
/// a boundary table.
pub fn case_walls(result: &Path) -> Option<Vec<String>> {
    let file = result.parent()?.join("case.resolved.toml");
    let text = std::fs::read_to_string(file).ok()?;
    let case: toml::Table = toml::from_str(&text).ok()?;
    let b = case.get("boundary")?.as_table()?;
    Some(
        b.iter()
            .filter(|(_, t)| t.get("type").and_then(|v| v.as_str()) == Some("wall"))
            .map(|(n, _)| n.clone())
            .collect(),
    )
}

/// The wall patches of `mesh` when none are named: those of the case
/// beside `result` that the mesh has, else the patches whose role is
/// wall.
pub fn find_walls(mesh: &Mesh, result: Option<&Path>) -> Vec<String> {
    if let Some(named) = result.and_then(case_walls) {
        let have: Vec<String> = named
            .into_iter()
            .filter(|n| mesh.patches.iter().any(|p| &p.name == n))
            .collect();
        if !have.is_empty() {
            return have;
        }
    }
    mesh.patches
        .iter()
        .filter(|p| p.role == PatchRole::Wall)
        .map(|p| p.name.clone())
        .collect()
}

/// A node of the hierarchy: its box, and its children or triangles.
struct Node {
    lo: [f64; 3],
    hi: [f64; 3],
    /// Children (`left`, `left + 1`) for an inner node; for a leaf the
    /// first triangle and how many.
    first: u32,
    count: u32,
}

/// The triangles of the walls in a bounding volume hierarchy.
pub struct WallTree {
    tris: Vec<[[f64; 3]; 3]>,
    nodes: Vec<Node>,
}

impl WallTree {
    /// The tree of the boundary faces of `mesh` on the patches `walls`;
    /// `None` when they have no face.
    pub fn new(mesh: &Mesh, walls: &[String]) -> Option<WallTree> {
        let on: Vec<bool> = mesh
            .patches
            .iter()
            .map(|p| walls.iter().any(|w| w == &p.name))
            .collect();
        let mut tris: Vec<[[f64; 3]; 3]> = Vec::new();
        for b in 0..mesh.boundary.len() {
            let Some(&p) = mesh.boundary_patch.get(b) else {
                continue;
            };
            if !on.get(p as usize).copied().unwrap_or(false) {
                continue;
            }
            let nodes = mesh.boundary.get(b);
            let at = |i: u32| mesh.points[i as usize];
            match nodes.len() {
                0..=2 => {}
                3 => tris.push([at(nodes[0]), at(nodes[1]), at(nodes[2])]),
                n => {
                    let mut c = [0.0; 3];
                    for &i in nodes {
                        for (d, x) in c.iter_mut().zip(at(i)) {
                            *d += x / n as f64;
                        }
                    }
                    for k in 0..n {
                        tris.push([c, at(nodes[k]), at(nodes[(k + 1) % n])]);
                    }
                }
            }
        }
        if tris.is_empty() {
            return None;
        }
        let mut order: Vec<u32> = (0..tris.len() as u32).collect();
        let centre: Vec<[f64; 3]> = tris
            .iter()
            .map(|t| [0, 1, 2].map(|d| (t[0][d] + t[1][d] + t[2][d]) / 3.0))
            .collect();
        let mut nodes = vec![Node {
            lo: [0.0; 3],
            hi: [0.0; 3],
            first: 0,
            count: 0,
        }];
        // (node, range of `order`)
        let mut stack = vec![(0usize, 0usize, order.len())];
        while let Some((at, a, b)) = stack.pop() {
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for &t in &order[a..b] {
                for p in tris[t as usize] {
                    for d in 0..3 {
                        lo[d] = lo[d].min(p[d]);
                        hi[d] = hi[d].max(p[d]);
                    }
                }
            }
            nodes[at].lo = lo;
            nodes[at].hi = hi;
            if b - a <= LEAF {
                nodes[at].first = a as u32;
                nodes[at].count = (b - a) as u32;
                continue;
            }
            let ext = [0, 1, 2].map(|d| hi[d] - lo[d]);
            let axis = (0..3)
                .max_by(|&i, &j| ext[i].total_cmp(&ext[j]))
                .unwrap_or(0);
            let mid = a + (b - a) / 2;
            order[a..b].select_nth_unstable_by(mid - a, |&i, &j| {
                centre[i as usize][axis].total_cmp(&centre[j as usize][axis])
            });
            let left = nodes.len();
            for _ in 0..2 {
                nodes.push(Node {
                    lo: [0.0; 3],
                    hi: [0.0; 3],
                    first: 0,
                    count: 0,
                });
            }
            nodes[at].first = left as u32;
            nodes[at].count = 0;
            stack.push((left, a, mid));
            stack.push((left + 1, mid, b));
        }
        let tris = order.iter().map(|&i| tris[i as usize]).collect();
        Some(WallTree { tris, nodes })
    }

    /// The distance of `x` to the nearest wall triangle.
    pub fn distance(&self, x: [f64; 3]) -> f64 {
        let mut best = f64::INFINITY;
        let mut stack: Vec<(f64, usize)> = vec![(box_d2(&self.nodes[0], x), 0)];
        while let Some((d2, at)) = stack.pop() {
            if d2 >= best {
                continue;
            }
            let n = &self.nodes[at];
            if n.count > 0 {
                for t in &self.tris[n.first as usize..(n.first + n.count) as usize] {
                    let p = closest_on_triangle(x, t);
                    best = best.min(dist2(x, p));
                }
                continue;
            }
            let (l, r) = (n.first as usize, n.first as usize + 1);
            let (dl, dr) = (box_d2(&self.nodes[l], x), box_d2(&self.nodes[r], x));
            // The nearer child is taken first, so it goes on top.
            if dl < dr {
                stack.push((dr, r));
                stack.push((dl, l));
            } else {
                stack.push((dl, l));
                stack.push((dr, r));
            }
        }
        best.sqrt()
    }
}

fn box_d2(n: &Node, x: [f64; 3]) -> f64 {
    (0..3)
        .map(|d| {
            let e = (n.lo[d] - x[d]).max(0.0).max(x[d] - n.hi[d]);
            e * e
        })
        .sum()
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = sub(a, b);
    dot(d, d)
}

fn along(a: [f64; 3], d: [f64; 3], t: f64) -> [f64; 3] {
    [a[0] + t * d[0], a[1] + t * d[1], a[2] + t * d[2]]
}

/// The point of triangle `t` nearest `p` (Ericson, §5.1.5): the vertex,
/// edge or face region `p` projects into. A degenerate triangle falls
/// back to the nearest point of its edges.
fn closest_on_triangle(p: [f64; 3], t: &[[f64; 3]; 3]) -> [f64; 3] {
    let [a, b, c] = *t;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return along(a, ab, d1 / (d1 - d3));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return along(a, ac, d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return along(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let sum = va + vb + vc;
    if sum == 0.0 || !sum.is_finite() {
        // A sliver: the nearest point of its three edges.
        let seg = |a: [f64; 3], b: [f64; 3]| {
            let d = sub(b, a);
            let l = dot(d, d);
            let s = if l > 0.0 {
                (dot(sub(p, a), d) / l).clamp(0.0, 1.0)
            } else {
                0.0
            };
            along(a, d, s)
        };
        return [seg(a, b), seg(b, c), seg(c, a)]
            .into_iter()
            .min_by(|x, y| dist2(p, *x).total_cmp(&dist2(p, *y)))
            .unwrap_or(a);
    }
    let v = vb / sum;
    let w = vc / sum;
    [
        a[0] + ab[0] * v + ac[0] * w,
        a[1] + ab[1] * v + ac[1] * w,
        a[2] + ab[2] * v + ac[2] * w,
    ]
}

/// The distance of each of `points` to the nearest face of the patches
/// `walls` of `mesh`, on all threads; `None` when those patches have no
/// face.
pub fn wall_distance(mesh: &Mesh, walls: &[String], points: &[[f64; 3]]) -> Option<Vec<f64>> {
    let tree = WallTree::new(mesh, walls)?;
    Some(
        murmel_mesh::par::map(points.len(), 2048, |r| {
            r.map(|i| tree.distance(points[i])).collect::<Vec<_>>()
        })
        .concat(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the nearest point of a triangle is a vertex, an edge
    /// point or the foot on the face, whichever region the point is in.
    /// Breaks when a region test has its sign the wrong way.
    #[test]
    fn closest_points_of_a_triangle() {
        let t = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let near = |p: [f64; 3], want: [f64; 3]| {
            let got = closest_on_triangle(p, &t);
            assert!(dist2(got, want) < 1e-24, "{p:?}: {got:?} not {want:?}");
        };
        near([0.2, 0.2, 3.0], [0.2, 0.2, 0.0]);
        near([-1.0, -1.0, 0.0], [0.0; 3]);
        near([2.0, -1.0, 1.0], [1.0, 0.0, 0.0]);
        near([0.5, -2.0, 0.0], [0.5, 0.0, 0.0]);
        near([1.0, 1.0, 0.0], [0.5, 0.5, 0.0]);
        near([-1.0, 0.5, 0.0], [0.0, 0.5, 0.0]);
        // A sliver.
        let s = [[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        assert_eq!(closest_on_triangle([1.5, 1.0, 0.0], &s), [1.5, 0.0, 0.0]);
    }

    /// Invariant: the tree finds the same nearest distance as trying
    /// every triangle. Breaks when a box is skipped too early or a
    /// child range is lost in the split.
    #[test]
    fn tree_matches_brute_force() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/openfoam/dual_polyhedra");
        let m = murmel_mesh::read(&p).unwrap();
        let all: Vec<String> = m.patches.iter().map(|p| p.name.clone()).collect();
        let tree = WallTree::new(&m, &all).unwrap();
        let mut seed = 7u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * 3.0 - 1.0
        };
        for _ in 0..200 {
            let x = [next(), next(), next()];
            let brute = tree
                .tris
                .iter()
                .map(|t| dist2(x, closest_on_triangle(x, t)))
                .fold(f64::INFINITY, f64::min)
                .sqrt();
            assert!((tree.distance(x) - brute).abs() < 1e-12, "{x:?}");
        }
    }
}
