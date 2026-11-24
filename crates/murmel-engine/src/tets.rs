//! The field inside a cell: piecewise linear on the cell's tetrahedra,
//! one per edge of each face (the cell's centroid, the face's centre and
//! the edge's two nodes). Slices and iso-surfaces cut these tetrahedra
//! ([`crate::extract`]) and streamlines are traced through them
//! ([`crate::stream`]), so all three show the same field.
//!
//! The centroid carries the cell's own value (the solver's); an inner
//! face's centre the two cells' values interpolated linearly between
//! their centroids (J. H. Ferziger and M. Perić, "Computational Methods
//! for Fluid Dynamics", section 4.4, the usual face value of finite
//! volumes), a boundary face's centre the mean of its nodes; a node its
//! node value ([`crate::field`]).

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
/// `face`): between the two cells' values by their centroids' distances
/// to it, or for a boundary face (or a cell without a value) the mean of
/// its nodes.
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
    if o == NO_CELL || n == NO_CELL {
        return nodes();
    }
    let (vo, vn) = (cell_value(f, o as usize), cell_value(f, n as usize));
    let dist = |c: u32| {
        let x = centroid(mesh, fields, c as usize);
        (0..3).map(|d| (x[d] - fc[d]).powi(2)).sum::<f64>().sqrt()
    };
    let (d_o, d_n) = (dist(o), dist(n));
    if !(vo.is_finite() && vn.is_finite()) || d_o + d_n <= 0.0 {
        return nodes();
    }
    (d_n * vo + d_o * vn) / (d_o + d_n)
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
