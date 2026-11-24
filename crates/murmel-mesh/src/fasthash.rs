//! A fast hash for integer keys: the surface's edge sets (millions of
//! point-id pairs per build) and the global point ids the VTKHDF reader
//! joins blocks by. It is the multiply-rotate word hash Firefox
//! introduced and rustc uses ("FxHash": rotate the state left by 5, xor
//! the word in, multiply by a fixed odd constant). It is not resistant to
//! chosen keys, which is fine for ids from a mesh; the standard library's
//! SipHash spends most of the edge pass and of the join hashing.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

#[derive(Clone, Copy, Default)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for w in words {
            self.add(u64::from_le_bytes(*w));
        }
        if !rest.is_empty() {
            let mut w = [0u8; 8];
            w[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(w));
        }
    }

    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    fn write_isize(&mut self, i: isize) {
        self.add(i as u64);
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}

pub type FastSet<K> = HashSet<K, BuildHasherDefault<FxHasher>>;
pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

/// An empty set with room for `n` keys.
pub fn set_with_capacity<K>(n: usize) -> FastSet<K> {
    FastSet::with_capacity_and_hasher(n, BuildHasherDefault::default())
}

/// An empty map with room for `n` keys.
pub fn map_with_capacity<K, V>(n: usize) -> FastMap<K, V> {
    FastMap::with_capacity_and_hasher(n, BuildHasherDefault::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the set behaves as a set (each key once, every key
    /// found) and different small keys hash apart. Breaks if a write
    /// drops part of a key, which would merge distinct edges and leave
    /// lines out of the surface.
    #[test]
    fn keys_stay_distinct() {
        let mut s: FastSet<(u32, u32)> = set_with_capacity(16);
        for a in 0..200u32 {
            for b in 0..20u32 {
                assert!(s.insert((a, b)));
            }
        }
        assert!(!s.insert((7, 3)));
        assert_eq!(s.len(), 4000);
        let h = |bytes: &[u8]| {
            let mut x = FxHasher::default();
            x.write(bytes);
            x.finish()
        };
        assert_ne!(
            h(&[1, 0, 0, 0, 0, 0, 0, 0, 2]),
            h(&[1, 0, 0, 0, 0, 0, 0, 0, 3])
        );
    }
}
