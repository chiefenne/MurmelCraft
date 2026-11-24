//! Cell gradients of a cell field by weighted least squares: each cell's
//! gradient is the one that best predicts the values of the cells that
//! share a node with it (and of its boundary faces, where the file has
//! them), each difference weighted by the inverse square of its distance
//! (D. J. Mavriplis, "Revisiting the least-squares procedure for gradient
//! reconstruction on unstructured meshes", AIAA paper 2003-3986, which
//! also finds the wider, node-sharing stencil more robust than the cells
//! across the faces alone). The gradient is exact for a field linear in
//! space, on any cell shape, and needs no face to be shared: a mesh
//! whose neighbouring cells split a face differently (a polyhedron's
//! face in triangles beside a prism's quad) still gets its gradients.
//!
//! The values between cells are rebuilt from these gradients
//! ([`crate::field`], [`crate::tets`]): a node takes the mean of what its
//! cells predict there, a face centre the mean of what its two cells
//! predict, so a linear field stays exactly linear and its iso-surfaces
//! come out flat (decided 2026-11-24; before, inverse distance weighting
//! made them wave cell by cell). Where the cells around a cell do not
//! span all three directions (a mesh one cell thick), the gradient is
//! zero along the missing one.

use murmel_mesh::{Mesh, NO_CELL};

/// The cells that share a node with a cell (the cell itself may be
/// among them): `near(c, out)` fills `out`.
pub(crate) type Near<'a> = &'a (dyn Fn(usize, &mut Vec<u32>) + Sync);

/// The gradient of `values` (one per cell, NaN for none) in every cell,
/// with the centroids `centre`, fitted to the cells that share a node
/// with each (`near`). `boundary` gives the values on the boundary faces
/// and their centres, by boundary face, when the file has them. A cell
/// without a value, or without any neighbour with one, has a zero
/// gradient.
pub(crate) fn cell_gradients(
    mesh: &Mesh,
    centre: &[[f64; 3]],
    values: &[f64],
    boundary: Option<(&[f64], &[[f64; 3]])>,
    near: Near<'_>,
) -> Vec<[f64; 3]> {
    let n = mesh.cell_count().min(values.len()).min(centre.len());
    murmel_mesh::par::map(n, 4096, |r| {
        let mut around = Vec::new();
        r.map(|c| {
            around.clear();
            near(c, &mut around);
            gradient_of(mesh, centre, values, boundary, c, &around)
        })
        .collect::<Vec<_>>()
    })
    .into_iter()
    .flatten()
    .collect()
}

/// The values of one field component, by cell, and its values on the
/// boundary faces with their centres when the file has them.
pub(crate) type Input<'a> = (&'a [f64], Option<(&'a [f64], &'a [[f64; 3]])>);

/// [`cell_gradients`] of each of `inputs` (each the same as alone), the
/// cells around each cell found once for all of them.
pub(crate) fn cell_gradients_many(
    mesh: &Mesh,
    centre: &[[f64; 3]],
    inputs: &[Input<'_>],
    near: Near<'_>,
) -> Vec<Vec<[f64; 3]>> {
    let ns: Vec<usize> = inputs
        .iter()
        .map(|(v, _)| mesh.cell_count().min(v.len()).min(centre.len()))
        .collect();
    let n = ns.iter().copied().max().unwrap_or(0);
    let parts = murmel_mesh::par::map(n, 4096, |r| {
        let mut around = Vec::new();
        let mut out: Vec<Vec<[f64; 3]>> =
            inputs.iter().map(|_| Vec::with_capacity(r.len())).collect();
        for c in r {
            around.clear();
            near(c, &mut around);
            for (k, &(values, boundary)) in inputs.iter().enumerate() {
                if c < ns[k] {
                    out[k].push(gradient_of(mesh, centre, values, boundary, c, &around));
                }
            }
        }
        out
    });
    let mut all: Vec<Vec<[f64; 3]>> = ns.iter().map(|&n| Vec::with_capacity(n)).collect();
    for part in parts {
        for (k, g) in part.into_iter().enumerate() {
            all[k].extend(g);
        }
    }
    all
}

/// The gradient of `values` in cell `c` alone, fitted to the cells
/// `around` it (see [`cell_gradients`]).
pub(crate) fn gradient_of(
    mesh: &Mesh,
    centre: &[[f64; 3]],
    values: &[f64],
    boundary: Option<(&[f64], &[[f64; 3]])>,
    c: usize,
    around: &[u32],
) -> [f64; 3] {
    let Some(&v) = values.get(c) else {
        return [0.0; 3];
    };
    if !v.is_finite() {
        return [0.0; 3];
    }
    let x = centre[c];
    let cells = around
        .iter()
        .map(|&o| o as usize)
        .filter(|&o| o != c)
        .filter_map(|o| Some((*centre.get(o)?, *values.get(o)?)));
    let faces = boundary.into_iter().flat_map(|(bv, bc)| {
        mesh.cell(c).filter_map(move |f| {
            let k = *mesh.face_boundary.get(f.id as usize)?;
            if k == NO_CELL {
                return None;
            }
            Some((*bc.get(k as usize)?, *bv.get(k as usize)?))
        })
    });
    fit(x, v, cells.chain(faces))
}

/// The gradient about `x`, where the value is `v`, that best predicts
/// the `samples` (a place and its value), each difference weighted by
/// the inverse square of its distance. A sample at `x` itself or
/// without a finite value is skipped.
pub(crate) fn fit(
    x: [f64; 3],
    v: f64,
    samples: impl IntoIterator<Item = ([f64; 3], f64)>,
) -> [f64; 3] {
    // The normal equations: a g = b, a symmetric.
    let mut a = [[0.0f64; 3]; 3];
    let mut b = [0.0f64; 3];
    for (to, value) in samples {
        let d = [to[0] - x[0], to[1] - x[1], to[2] - x[2]];
        let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if !(value.is_finite() && dd > 0.0 && dd.is_finite()) {
            continue;
        }
        let w = 1.0 / dd;
        let dv = value - v;
        for i in 0..3 {
            for j in 0..3 {
                a[i][j] += w * d[i] * d[j];
            }
            b[i] += w * d[i] * dv;
        }
    }
    solve(a, b)
}

/// The solution of the symmetric system `a g = b`, with a tiny ridge so
/// a direction no neighbour spans gets no gradient instead of a division
/// by zero (the weights make `a` dimensionless, its trace about the
/// number of neighbours).
fn solve(mut a: [[f64; 3]; 3], b: [f64; 3]) -> [f64; 3] {
    let trace = a[0][0] + a[1][1] + a[2][2];
    if trace.is_nan() || trace <= 0.0 {
        return [0.0; 3];
    }
    for (i, row) in a.iter_mut().enumerate() {
        row[i] += 1e-9 * trace;
    }
    let det3 = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let det = det3(&a);
    if det == 0.0 || !det.is_finite() {
        return [0.0; 3];
    }
    let g = [0, 1, 2].map(|k| {
        let mut m = a;
        for (i, row) in m.iter_mut().enumerate() {
            row[k] = b[i];
        }
        det3(&m) / det
    });
    if g.iter().all(|x| x.is_finite()) {
        g
    } else {
        [0.0; 3]
    }
}

/// The value cell `c` (value `v`, centroid `x`, gradient `g`) predicts
/// at `at`.
pub(crate) fn predict(v: f64, x: [f64; 3], g: [f64; 3], at: [f64; 3]) -> f64 {
    v + g[0] * (at[0] - x[0]) + g[1] * (at[1] - x[1]) + g[2] * (at[2] - x[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the least-squares gradient of a field linear in space
    /// is its exact gradient in every cell of a polyhedral mesh, inner
    /// cells and cells at the walls alike (where fewer neighbours span
    /// the directions). Breaks when a neighbour is taken from the wrong
    /// side of a face or a weight leaves the fit biased.
    #[test]
    fn exact_for_a_linear_field() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/openfoam/dual_polyhedra");
        let m = murmel_mesh::read(&p).unwrap();
        let centre: Vec<[f64; 3]> = (0..m.cell_count())
            .map(|c| crate::inspect::cell_centroid(&m, c))
            .collect();
        let g0 = [2.0, -3.0, 0.5];
        let f = |x: [f64; 3]| 1.0 + g0[0] * x[0] + g0[1] * x[1] + g0[2] * x[2];
        let values: Vec<f64> = centre.iter().map(|&x| f(x)).collect();
        let mut of_point: Vec<Vec<u32>> = vec![Vec::new(); m.points.len()];
        for c in 0..m.cell_count() {
            for p in m.cell_points(c) {
                of_point[p as usize].push(c as u32);
            }
        }
        let near = |c: usize, out: &mut Vec<u32>| {
            for p in m.cell_points(c) {
                out.extend_from_slice(&of_point[p as usize]);
            }
        };
        let g = cell_gradients(&m, &centre, &values, None, &near);
        assert_eq!(g.len(), m.cell_count());
        for (c, gc) in g.iter().enumerate() {
            for k in 0..3 {
                assert!((gc[k] - g0[k]).abs() < 1e-6, "cell {c}: {gc:?}");
            }
        }
    }

    /// Invariant: a direction no neighbour spans (a mesh one cell thick
    /// along z) gets a zero gradient, not a blow-up. Breaks when the
    /// ridge is dropped and the solve divides by a zero determinant.
    #[test]
    fn flat_directions_get_no_gradient() {
        let a = [[2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]];
        let g = solve(a, [4.0, 1.0, 0.0]);
        assert!((g[0] - 2.0).abs() < 1e-6 && (g[1] - 1.0).abs() < 1e-6);
        assert_eq!(g[2], 0.0);
        assert_eq!(solve([[0.0; 3]; 3], [0.0; 3]), [0.0; 3]);
    }
}
