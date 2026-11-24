//! The 3-D Hilbert index that orders the cells of a blocked VTKHDF file
//! (`docs/blocked-vtkhdf.md` § 2), from John Skilling, "Programming the
//! Hilbert curve", AIP Conference Proceedings 707, 381 (2004): his
//! `AxestoTranspose` turns the axes into the transposed index, whose bits
//! read across the axes from the top bit down are the index.
//!
//! Points close along the curve are close in space, so a run of cells in
//! this order is a compact block: a region of interest then touches few
//! blocks.

/// Bits per axis of the file's index: 3 × 21 = 63 bits.
pub const BITS: u32 = 21;

/// The Hilbert index of the integer point `x` (each axis below
/// 2^`bits`), `3 × bits` bits long.
pub fn index(x: [u32; 3], bits: u32) -> u64 {
    debug_assert!((1..=BITS).contains(&bits));
    let mut x = x;
    let m = 1u32 << (bits - 1);
    // inverse undo
    let mut q = m;
    while q > 1 {
        let p = q - 1;
        for i in 0..3 {
            if x[i] & q != 0 {
                x[0] ^= p; // invert
            } else {
                let t = (x[0] ^ x[i]) & p; // exchange
                x[0] ^= t;
                x[i] ^= t;
            }
        }
        q >>= 1;
    }
    // Gray encode
    for i in 1..3 {
        x[i] ^= x[i - 1];
    }
    let mut t = 0;
    let mut q = m;
    while q > 1 {
        if x[2] & q != 0 {
            t ^= q - 1;
        }
        q >>= 1;
    }
    for v in &mut x {
        *v ^= t;
    }
    // the transposed index read bit plane by bit plane
    let mut h = 0u64;
    for b in (0..bits).rev() {
        for v in x {
            h = (h << 1) | u64::from((v >> b) & 1);
        }
    }
    h
}

/// Scales points of a bounding box to the integer grid of [`BITS`] bits,
/// each axis on its own, as floor((x − min) / (max − min) × (2²¹ − 1))
/// (KoflCraft's writer does the same, so both order a mesh alike); an
/// axis of zero extent maps to 0.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    lo: [f64; 3],
    scale: [f64; 3],
}

impl Grid {
    pub fn new(lo: [f64; 3], hi: [f64; 3]) -> Grid {
        let top = f64::from((1u32 << BITS) - 1);
        let scale = [0, 1, 2].map(|k| {
            let d = hi[k] - lo[k];
            if d > 0.0 && d.is_finite() {
                top / d
            } else {
                0.0
            }
        });
        Grid { lo, scale }
    }

    /// The Hilbert index of point `p` (63 bits).
    pub fn index(&self, p: [f64; 3]) -> u64 {
        let top = f64::from((1u32 << BITS) - 1);
        let q = [0, 1, 2].map(|k| {
            let v = ((p[k] - self.lo[k]) * self.scale[k]).floor();
            // NaN and points outside the box land on the nearest face
            if v >= 0.0 { v.min(top) as u32 } else { 0 }
        });
        index(q, BITS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: on a 2^b grid the index visits every point exactly once
    /// (a bijection onto 0 … 8^b − 1), and two points one apart along the
    /// curve are neighbours of the grid (one step along one axis). This is
    /// what makes a run of the curve a compact block; a wrong bit
    /// operation breaks the adjacency at once.
    #[test]
    fn the_curve_visits_every_point_once_in_unit_steps() {
        for bits in 1..=4u32 {
            let n = 1u32 << bits;
            let mut at = vec![None; (n * n * n) as usize];
            for x in 0..n {
                for y in 0..n {
                    for z in 0..n {
                        let h = index([x, y, z], bits) as usize;
                        assert!(h < at.len(), "index {h} out of range for {bits} bits");
                        assert!(at[h].is_none(), "index {h} twice");
                        at[h] = Some([x, y, z]);
                    }
                }
            }
            for w in at.windows(2) {
                let (a, b) = (w[0].unwrap(), w[1].unwrap());
                let d: u32 = (0..3).map(|k| a[k].abs_diff(b[k])).sum();
                assert_eq!(d, 1, "{a:?} then {b:?} at {bits} bits");
            }
        }
    }

    /// Invariant: the full 21-bit index stays within 63 bits, starts at
    /// the origin, and the grid maps the box corners to 0 and 2^21 − 1 on
    /// every axis (a flat axis to 0).
    #[test]
    fn the_file_index_has_63_bits() {
        let top = (1u32 << BITS) - 1;
        assert_eq!(index([0, 0, 0], BITS), 0);
        assert!(index([top, top, top], BITS) < 1 << 63);
        let g = Grid::new([0.0, -1.0, 5.0], [2.0, 1.0, 5.0]);
        assert_eq!(g.index([0.0, -1.0, 5.0]), 0);
        let far = g.index([2.0, 1.0, 5.0]);
        assert_eq!(far, index([top, top, 0], BITS));
        assert_eq!(g.index([f64::NAN, 9.0, -3.0]), index([0, top, 0], BITS));
    }
}
