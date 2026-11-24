//! The field inside a cell: piecewise linear on the cell's tetrahedra,
//! one per edge of each face (the cell's centroid, the face's centre and
//! the edge's two nodes). Slices and iso-surfaces cut these tetrahedra
//! ([`crate::extract`]) and streamlines are traced through them
//! ([`crate::stream`]), so all three show the same field.
//!
//! The centroid carries the cell's own value (the solver's); an inner
//! face's centre what the two cells predict there from their gradients
//! ([`crate::gradient`]), the nearer cell weighted more (as the usual
//! face value of finite volumes weights the two values: J. H. Ferziger
//! and M. Perić, "Computational Methods for Fluid Dynamics", section
//! 4.4); a boundary face's centre what its cell predicts there; a node
//! its node value ([`crate::field`]). So a field linear in space is exactly linear
//! on every tetrahedron, and its slices and iso-surfaces are exact.

use murmel_mesh::{Mesh, NO_CELL};

use crate::field::{FieldSet, FieldValues};

/// The value of `f` at mesh point `i` (NaN where it has none).
pub(crate) fn node_value(f: &FieldValues, i: u32) -> f64 {
    f.node.get(i as usize).copied().unwrap_or(f64::NAN)
}

/// The value of `f` in cell `c` (NaN where it has none).
pub(crate) fn cell_value(f: &FieldValues, c: usize) -> f64 {
    f.cell.get(c).copied().unwrap_or(f64::NAN)
}

/// The mean of the points `nodes`.
pub(crate) fn face_centre(mesh: &Mesh, nodes: &[u32]) -> [f64; 3] {
    let k = nodes.len().max(1) as f64;
    [0, 1, 2].map(|d| {
        nodes
            .iter()
            .map(|&n| mesh.points.get(n as usize).map_or(f64::NAN, |p| p[d]))
            .sum::<f64>()
            / k
    })
}

/// The centroid of cell `c`: cached by the field set when there is one.
pub(crate) fn centroid(mesh: &Mesh, fields: Option<&FieldSet>, c: usize) -> [f64; 3] {
    match fields {
        Some(f) => f.centroid(mesh, c),
        None => crate::inspect::cell_centroid(mesh, c),
    }
}

/// The value of `f` at the centre `fc` of distinct face `u` (points
/// `face`): what its cells predict there from their gradients, the two
/// of an inner face weighted by their centroids' distances to it, kept
/// within the cells' values and the face's node values; without
/// gradients, the two cells' values so weighted; the mean of its nodes
/// where no cell has a value (and a boundary face without gradients).
pub(crate) fn face_value(
    mesh: &Mesh,
    fields: Option<&FieldSet>,
    f: &FieldValues,
    u: u32,
    face: &[u32],
    fc: [f64; 3],
) -> f64 {
    let nodes = || face.iter().map(|&n| node_value(f, n)).sum::<f64>() / face.len() as f64;
    let (Some(&o), Some(&n)) = (
        mesh.face_owner.get(u as usize),
        mesh.face_neighbour.get(u as usize),
    ) else {
        return nodes();
    };
    let value = |c: u32| {
        if c == NO_CELL {
            f64::NAN
        } else {
            cell_value(f, c as usize)
        }
    };
    let (vo, vn) = (value(o), value(n));
    let dist = |c: u32| {
        let x = centroid(mesh, fields, c as usize);
        (0..3).map(|d| (x[d] - fc[d]).powi(2)).sum::<f64>().sqrt()
    };
    // What a cell predicts at the centre from its gradient.
    let predict = |c: u32, v: f64| {
        let g = f.grad.get(c as usize)?;
        let r =
            crate::gradient::predict(v, centroid(mesh, fields, c as usize), g.map(f64::from), fc);
        r.is_finite().then_some(r)
    };
    // Within the cells' values and the face's node values, so no new
    // extreme appears (a linear field's is the mean of its nodes').
    let bound = |x: f64, cells: &[f64]| {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let at_nodes = face.iter().map(|&k| node_value(f, k));
        for v in cells.iter().copied().chain(at_nodes) {
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        if lo <= hi { x.clamp(lo, hi) } else { x }
    };
    match (vo.is_finite(), vn.is_finite()) {
        (true, true) => {
            let (d_o, d_n) = (dist(o), dist(n));
            if d_o + d_n <= 0.0 {
                return nodes();
            }
            match (predict(o, vo), predict(n, vn)) {
                (Some(ro), Some(rn)) => bound((d_n * ro + d_o * rn) / (d_o + d_n), &[vo, vn]),
                _ => (d_n * vo + d_o * vn) / (d_o + d_n),
            }
        }
        // A boundary face (or a neighbour without a value): its one
        // cell's prediction, so a large face's centre follows the cell
        // and not only its rim.
        (true, false) => predict(o, vo).map_or_else(nodes, |r| bound(r, &[vo])),
        (false, true) => predict(n, vn).map_or_else(nodes, |r| bound(r, &[vn])),
        (false, false) => nodes(),
    }
}

/// The barycentric coordinates of `p` in the tetrahedron `t`, or `None`
/// when it is flat.
pub(crate) fn barycentric(t: &[[f64; 3]; 4], p: [f64; 3]) -> Option<[f64; 4]> {
    let e = |i: usize| [0, 1, 2].map(|d| t[i][d] - t[0][d]);
    let (a, b, c) = (e(1), e(2), e(3));
    let r = [0, 1, 2].map(|d| p[d] - t[0][d]);
    let det = |x: [f64; 3], y: [f64; 3], z: [f64; 3]| {
        x[0] * (y[1] * z[2] - y[2] * z[1]) - x[1] * (y[0] * z[2] - y[2] * z[0])
            + x[2] * (y[0] * z[1] - y[1] * z[0])
    };
    let d = det(a, b, c);
    let scale = [a, b, c]
        .iter()
        .map(|v| v.iter().map(|x| x.abs()).fold(0.0, f64::max))
        .fold(0.0, f64::max);
    if !d.is_finite() || d.abs() <= 1e-14 * scale.powi(3) {
        return None;
    }
    let l1 = det(r, b, c) / d;
    let l2 = det(a, r, c) / d;
    let l3 = det(a, b, r) / d;
    Some([1.0 - l1 - l2 - l3, l1, l2, l3])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: barycentric coordinates sum to one, give each corner
    /// its own weight 1, and are all positive inside only.
    #[test]
    fn barycentric_coordinates() {
        let t = [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
            [0.0, 0.0, 2.0],
        ];
        for (i, &p) in t.iter().enumerate() {
            let l = barycentric(&t, p).unwrap();
            for (j, &w) in l.iter().enumerate() {
                assert!((w - if i == j { 1.0 } else { 0.0 }).abs() < 1e-12);
            }
        }
        let l = barycentric(&t, [0.25, 0.25, 0.25]).unwrap();
        assert!(l.iter().all(|&w| w > 0.0));
        assert!((l.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        let l = barycentric(&t, [3.0, 0.1, 0.1]).unwrap();
        assert!(l.iter().any(|&w| w < 0.0));
        let flat = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        assert!(barycentric(&flat, [0.2, 0.2, 0.0]).is_none());
    }
}
