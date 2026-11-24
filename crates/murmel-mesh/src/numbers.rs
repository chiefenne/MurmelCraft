//! Fast reading of large arrays of numbers, shared by the readers.
//!
//! Ascii text is cut into one piece per thread at a separator, the tokens
//! of each piece are counted, and then every piece is parsed in place
//! into its part of the result (no intermediate vectors), whose memory is
//! first touched by those threads (see [`Filling`]). Integers are
//! parsed by hand; floating-point numbers by the standard library
//! (Eisel and Lemire's method). Base64 text is decoded in pieces of whole
//! four-character groups, and little-endian values are converted with
//! one decision per array instead of one per value.

use std::ops::Range;

use base64::Engine as _;

use crate::par;

/// A number type the readers parse.
pub(crate) trait Number: Copy + Default + Send + Sync {
    fn parse(token: &[u8]) -> Option<Self>;
}

/// The digits of a token as a magnitude, with its sign.
fn integer(t: &[u8]) -> Option<(bool, u64)> {
    let (neg, digits) = match t.first()? {
        b'-' => (true, &t[1..]),
        b'+' => (false, &t[1..]),
        _ => (false, t),
    };
    if digits.is_empty() || digits.len() > 19 {
        return None;
    }
    let mut v: u64 = 0;
    for &c in digits {
        let d = c.wrapping_sub(b'0');
        if d > 9 {
            return None;
        }
        v = v * 10 + u64::from(d);
    }
    Some((neg, v))
}

impl Number for i64 {
    fn parse(t: &[u8]) -> Option<i64> {
        let (neg, v) = integer(t)?;
        if neg {
            0i64.checked_sub_unsigned(v)
        } else {
            i64::try_from(v).ok()
        }
    }
}

impl Number for u32 {
    fn parse(t: &[u8]) -> Option<u32> {
        match integer(t)? {
            (false, v) => u32::try_from(v).ok(),
            (true, 0) => Some(0),
            _ => None,
        }
    }
}

impl Number for f64 {
    fn parse(t: &[u8]) -> Option<f64> {
        std::str::from_utf8(t).ok()?.parse().ok()
    }
}

/// The bytes that separate numbers, as a table (one lookup per byte).
/// Every separator is a control byte, a space or a parenthesis, which the
/// token count relies on (see [`count_tokens`]).
pub(crate) struct Sep {
    table: [bool; 256],
    /// `(` and `)` separate too.
    paren: bool,
}

impl Sep {
    const fn of(chars: &[u8]) -> Sep {
        let mut table = [false; 256];
        let mut paren = false;
        let mut i = 0;
        while i < chars.len() {
            table[chars[i] as usize] = true;
            paren |= chars[i] == b'(';
            i += 1;
        }
        Sep { table, paren }
    }

    #[inline]
    pub(crate) fn has(&self, c: u8) -> bool {
        self.table[c as usize]
    }
}

/// Whitespace between the numbers of a VTK or OpenFOAM array.
pub(crate) static SPACE: Sep = Sep::of(b" \n\r\t\x0c");

/// Whitespace and parentheses: OpenFOAM's `(x y z)` and `4(a b c d)` read
/// as a flat stream of numbers.
pub(crate) static SPACE_PAREN: Sep = Sep::of(b" \n\r\t\x0c()");

/// Pieces of `text`, one per thread, each starting at a separator.
fn pieces(text: &[u8], sep: &Sep) -> Vec<Range<usize>> {
    let n = text.len();
    let parts = par::ranges(n, 1 << 20);
    let mut cuts = vec![0];
    for r in parts.iter().skip(1) {
        let mut p = r.start.max(*cuts.last().unwrap_or(&0));
        while p < n && !sep.has(text[p]) {
            p += 1;
        }
        cuts.push(p);
    }
    cuts.push(n);
    cuts.windows(2)
        .map(|w| w[0]..w[1])
        .filter(|r| !r.is_empty())
        .collect()
}

/// The tokens of `text`: runs of bytes that are not separators, counted
/// sixteen or more bytes at a time. A byte counts as a separator when it
/// is a control byte, a space or (with `sep.paren`) a parenthesis: a test
/// with no table lookup, which the compiler turns into vector compares.
/// That is every separator and more, yet the count can only differ from
/// the table's when a control byte that is not a separator sits in the
/// text, and then [`parse_into`] fails on it: the token holding it is not
/// a number, or it is left over after the last token.
fn count_tokens(text: &[u8], sep: &Sep) -> usize {
    if sep.paren {
        count::<true>(text)
    } else {
        count::<false>(text)
    }
}

/// About how many numbers `bytes` bytes of text like `sample` hold: the
/// tokens of its first megabyte scaled up, at most one for every two
/// bytes (a number and its separator).
pub(crate) fn estimate(sample: &[u8], sep: &Sep, bytes: usize) -> usize {
    let sample = &sample[..sample.len().min(1 << 20)];
    if sample.is_empty() {
        return 0;
    }
    let tokens = count_tokens(sample, sep) as u128;
    let n = tokens * bytes as u128 / sample.len() as u128;
    usize::try_from(n)
        .unwrap_or(usize::MAX)
        .min(bytes.div_ceil(2))
}

/// A byte that may separate numbers (see [`count_tokens`]).
#[inline(always)]
fn loose<const PAREN: bool>(c: u8) -> bool {
    c <= b' ' || (PAREN && (c | 1) == b')')
}

/// The runs of bytes that are not [`loose`] separators: the first byte if
/// it starts one, and every byte that follows a separator. Counted in
/// blocks of 128 pairs into a byte (at most 64 starts fit), so the pairs
/// compare as vectors.
fn count<const PAREN: bool>(text: &[u8]) -> usize {
    let Some(&first) = text.first() else {
        return 0;
    };
    let mut n = usize::from(!loose::<PAREN>(first));
    let (before, after) = (&text[..text.len() - 1], &text[1..]);
    for (a, b) in before.chunks(128).zip(after.chunks(128)) {
        let mut k: u8 = 0;
        for (&x, &y) in a.iter().zip(b) {
            k += u8::from(loose::<PAREN>(x) & !loose::<PAREN>(y));
        }
        n += usize::from(k);
    }
    n
}

/// Parse the tokens of `text` into `out` (sized by `count_tokens`); any
/// text left after them must be separators.
fn parse_into<T: Number>(text: &[u8], sep: &Sep, out: &mut [T]) -> Result<(), String> {
    let (mut i, n) = (0, text.len());
    let token = |start: usize| {
        let end = text[start..]
            .iter()
            .position(|&c| sep.has(c))
            .map_or(n, |k| start + k);
        &text[start..end]
    };
    for x in out.iter_mut() {
        while i < n && sep.has(text[i]) {
            i += 1;
        }
        let t = token(i);
        i += t.len();
        *x = T::parse(t).ok_or_else(|| String::from_utf8_lossy(t).into_owned())?;
    }
    match text[i..].iter().position(|&c| !sep.has(c)) {
        Some(k) => Err(String::from_utf8_lossy(token(i + k)).into_owned()),
        None => Ok(()),
    }
}

/// Parse every token of `text` (tokens between `sep` characters) into
/// slots of `S`. A token that is not a number is returned as the error,
/// as is a count of values that does not fill whole slots.
pub(crate) fn parse<S: Slot>(text: &[u8], sep: &Sep) -> Result<Vec<S>, String> {
    let mut out = Filling::new(0);
    parse_append(text, sep, &mut out)?;
    out.finish()
        .map_err(|flat| format!("{} values, not groups of {}", flat.len(), S::N))
}

/// How parsed numbers are kept: one value per slot, or the coordinates of
/// a point together, seen as one flat run of values while parsing.
pub(crate) trait Slot: Copy + Default + Send + Sync {
    type Value: Number;
    /// Values per slot.
    const N: usize;
    fn flat(slots: &mut [Self]) -> &mut [Self::Value];
}

macro_rules! one_value {
    ($($t:ty),*) => {$(
        impl Slot for $t {
            type Value = $t;
            const N: usize = 1;
            fn flat(slots: &mut [$t]) -> &mut [$t] {
                slots
            }
        }
    )*};
}
one_value!(f64, u32, i64);

impl Slot for [f64; 3] {
    type Value = f64;
    const N: usize = 3;
    fn flat(slots: &mut [[f64; 3]]) -> &mut [f64] {
        slots.as_flattened_mut()
    }
}

/// Numbers parsed into the front of a buffer whose rest the allocator
/// handed over zeroed and nobody has touched yet. Growing never fills the
/// new part on one thread (as `Vec::resize` does): a zeroed allocation of
/// this size comes as fresh pages from the system, and the threads that
/// parse into it are the first to write them, in parallel. Sized from the
/// count a file declares, it never grows at all.
pub(crate) struct Filling<S> {
    buf: Vec<S>,
    /// Values written (not slots).
    len: usize,
}

impl<S: Slot> Filling<S> {
    /// Room for `values` values, untouched until parsed into.
    pub(crate) fn new(values: usize) -> Filling<S> {
        Filling {
            buf: vec![S::default(); values.div_ceil(S::N)],
            len: 0,
        }
    }

    /// Room for `values` values in all, as [`Filling::new`] gives, for a
    /// count learnt once the first values are read.
    pub(crate) fn expect(&mut self, values: usize) {
        self.grow(values.div_ceil(S::N));
    }

    /// The next `k` values, to be written; the buffer at least doubles
    /// when they do not fit.
    fn room(&mut self, k: usize) -> &mut [S::Value] {
        let need = self.len + k;
        let slots = need.div_ceil(S::N);
        if slots > self.buf.len() {
            self.grow(slots.max(2 * self.buf.len()));
        }
        let at = self.len;
        self.len = need;
        &mut S::flat(&mut self.buf)[at..need]
    }

    /// A buffer of `slots` slots, at least, holding the values so far
    /// (copied over on all threads).
    fn grow(&mut self, slots: usize) {
        if slots <= self.buf.len() {
            return;
        }
        let mut grown = vec![S::default(); slots];
        let old = &self.buf[..self.len.div_ceil(S::N)];
        par::chunks_mut(&mut grown[..old.len()], 1 << 16, |at, dst| {
            dst.copy_from_slice(&old[at..at + dst.len()]);
        });
        self.buf = grown;
    }

    /// The slots filled, or every value read when they do not fill whole
    /// slots (a point cut short).
    pub(crate) fn finish(mut self) -> Result<Vec<S>, Vec<S::Value>> {
        if !self.len.is_multiple_of(S::N) {
            return Err(S::flat(&mut self.buf)[..self.len].to_vec());
        }
        // Room beyond the values (a guess too large) is given back.
        self.buf.truncate(self.len / S::N);
        self.buf.shrink_to_fit();
        Ok(self.buf)
    }
}

/// Parse every token of `text` onto the end of `out` (a streaming reader
/// hands its text over piece by piece, each ending at a separator). A
/// token that is not a number is returned as the error.
pub(crate) fn parse_append<S: Slot>(
    text: &[u8],
    sep: &Sep,
    out: &mut Filling<S>,
) -> Result<(), String> {
    let ps = pieces(text, sep);
    let sizes: Vec<usize> = par::collect(ps.len(), 1, |r| {
        r.map(|i| count_tokens(&text[ps[i].clone()], sep)).collect()
    });
    // Each piece parses into its own part of `out`.
    let mut parts: Vec<(&[u8], &mut [S::Value])> = Vec::with_capacity(ps.len());
    let mut rest = out.room(sizes.iter().sum());
    for (r, &k) in ps.iter().zip(&sizes) {
        let (head, tail) = rest.split_at_mut(k);
        rest = tail;
        parts.push((&text[r.clone()], head));
    }
    // One piece (a short array, or one thread) is parsed here.
    let results: Vec<Result<(), String>> = if parts.len() <= 1 {
        parts
            .into_iter()
            .map(|(piece, slot)| parse_into(piece, sep, slot))
            .collect()
    } else {
        std::thread::scope(|s| {
            let handles: Vec<_> = parts
                .into_iter()
                .map(|(piece, slot)| s.spawn(move || parse_into(piece, sep, slot)))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
                .collect()
        })
    };
    for r in results {
        r?;
    }
    Ok(())
}

/// Decode base64 `b` (no whitespace, whole groups of four characters)
/// onto the end of `out`, and say how many bytes it wrote. Only the last
/// group may hold padding.
pub(crate) fn base64_append(b: &[u8], out: &mut Vec<u8>) -> Result<usize, String> {
    if !b.len().is_multiple_of(4) {
        return Err(format!(
            "{} base64 characters, not a multiple of 4",
            b.len()
        ));
    }
    let quads = b.len() / 4;
    let ranges = par::ranges(quads, 1 << 18);
    let at = out.len();
    out.resize(at + quads * 3, 0);
    let mut parts: Vec<(&[u8], &mut [u8])> = Vec::with_capacity(ranges.len());
    let mut rest: &mut [u8] = &mut out[at..];
    for r in &ranges {
        let (head, tail) = rest.split_at_mut(r.len() * 3);
        rest = tail;
        parts.push((&b[r.start * 4..r.end * 4], head));
    }
    let engine = &base64::engine::general_purpose::STANDARD;
    let decode =
        |src: &[u8], dst: &mut [u8]| engine.decode_slice(src, dst).map_err(|e| e.to_string());
    let written: Vec<Result<usize, String>> = if parts.len() <= 1 {
        parts
            .into_iter()
            .map(|(src, dst)| decode(src, dst))
            .collect()
    } else {
        std::thread::scope(|s| {
            let handles: Vec<_> = parts
                .into_iter()
                .map(|(src, dst)| s.spawn(move || decode(src, dst)))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
                .collect()
        })
    };
    let mut total = 0;
    for (i, w) in written.into_iter().enumerate() {
        let w = w?;
        // Only the last piece may end short (its padding).
        if i + 1 < ranges.len() && w != ranges[i].len() * 3 {
            return Err("padding inside the data".into());
        }
        total += w;
    }
    out.truncate(at + total);
    Ok(total)
}

/// The little-endian values of `bytes`, each `size` bytes, converted by
/// `f` (chosen once per array by the caller).
pub(crate) fn le<T: Send>(bytes: &[u8], size: usize, f: fn(&[u8]) -> T) -> Vec<T> {
    let n = bytes.len() / size.max(1);
    par::collect(n, 1 << 16, |r| {
        bytes[r.start * size..r.end * size]
            .chunks_exact(size)
            .map(f)
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the parallel parsers read what `str::parse` reads, at
    /// one thread and at several (pieces cut inside long runs of
    /// whitespace and between parentheses), and name the first bad token.
    /// A piece cut inside a token would split one number into two.
    #[test]
    fn parse_like_the_standard_library() {
        let ints: Vec<i64> = (0..5000).map(|i| (i * 7919) % 100_003 - 50_000).collect();
        let text: String = ints
            .iter()
            .enumerate()
            .map(|(i, v)| format!("{v}{}", if i % 7 == 0 { "\n   \t" } else { " " }))
            .collect();
        let floats: Vec<f64> = (0..3000).map(|i| (i as f64).sqrt() * 1e-3 - 0.5).collect();
        let ftext: String = floats.iter().map(|v| format!("{v:e} ")).collect();
        for threads in [1, 4] {
            par::with_threads(threads, || {
                assert_eq!(parse::<i64>(text.as_bytes(), &SPACE).unwrap(), ints);
                assert_eq!(parse::<f64>(ftext.as_bytes(), &SPACE).unwrap(), floats);
                let foam = b"3(0 1 2)\n4(4 5 6 7)\n(1.5 -2 3e-2)";
                assert_eq!(
                    parse::<f64>(foam, &SPACE_PAREN).unwrap(),
                    [3., 0., 1., 2., 4., 4., 5., 6., 7., 1.5, -2., 0.03]
                );
                assert_eq!(parse::<i64>(b"1 2 x3 4", &SPACE).unwrap_err(), "x3");
                assert_eq!(parse::<u32>(b"1 -2", &SPACE).unwrap_err(), "-2");
                assert_eq!(
                    parse::<u32>(b"4294967296", &SPACE).unwrap_err(),
                    "4294967296"
                );
                assert_eq!(parse::<i64>(b"", &SPACE).unwrap(), Vec::<i64>::new());
            });
        }
        assert_eq!(i64::parse(b"-9223372036854775808"), Some(i64::MIN));
        assert_eq!(i64::parse(b"9223372036854775808"), None);
        assert_eq!(i64::parse(b"3.0"), None);
    }

    /// Invariant: the vector token count agrees with the separator table
    /// on every byte that separates, and a control byte that does not
    /// separate is an error wherever it sits (inside a token, alone, or
    /// after the last token). A count that skipped such a byte would read
    /// a broken file without a word.
    #[test]
    fn stray_control_bytes_are_errors() {
        for sep in [&SPACE, &SPACE_PAREN] {
            for c in 0..=255u8 {
                if sep.has(c) {
                    assert!(if sep.paren {
                        loose::<true>(c)
                    } else {
                        loose::<false>(c)
                    });
                }
            }
        }
        for threads in [1, 3] {
            par::with_threads(threads, || {
                assert_eq!(parse::<i64>(b"1 2 \x01", &SPACE).unwrap_err(), "\x01");
                assert_eq!(parse::<i64>(b"1 \x01 2", &SPACE).unwrap_err(), "\x01");
                assert_eq!(parse::<i64>(b"1\x012 3", &SPACE).unwrap_err(), "1\x012");
                assert_eq!(
                    parse::<u32>(b"3(0 1\x02)", &SPACE_PAREN).unwrap_err(),
                    "1\x02"
                );
                assert_eq!(parse::<f64>(b"\x00", &SPACE).unwrap_err(), "\x00");
            });
        }
        // Long runs cross the blocks the count works in.
        let text: String = (0..1000)
            .map(|i| format!("{i}{}", " ".repeat(1 + i % 300)))
            .collect();
        assert_eq!(count_tokens(text.as_bytes(), &SPACE), 1000);
    }

    /// Invariant: numbers parse into points three by three however the
    /// text arrives, into room sized right, too small (it grows, keeping
    /// what was read) or too large (the rest is dropped), and a count that
    /// does not fill whole points comes back as plain values. A growth
    /// that lost the front of the buffer would move every point.
    #[test]
    fn filling_grows_and_groups() {
        let xyz: Vec<f64> = (0..3000).map(|i| f64::from(i) * 0.25 - 7.0).collect();
        let text: String = xyz.iter().map(|v| format!("{v} ")).collect();
        let bytes = text.as_bytes();
        let cuts: Vec<usize> = (1..9)
            .map(|k| {
                let c = k * bytes.len() / 9;
                c + bytes[c..].iter().position(|&b| b == b' ').unwrap_or(0)
            })
            .collect();
        for threads in [1, 4] {
            for room in [0, 1, 2999, 3000, 9000] {
                par::with_threads(threads, || {
                    let mut out = Filling::<[f64; 3]>::new(room);
                    let mut from = 0;
                    for &c in cuts.iter().chain([&bytes.len()]) {
                        parse_append(&bytes[from..c], &SPACE, &mut out).unwrap();
                        from = c;
                    }
                    let points = out.finish().unwrap();
                    assert_eq!(points.as_flattened(), xyz.as_slice());
                    let mut cut = Filling::<[f64; 3]>::new(room);
                    parse_append(b"1 2 3 4", &SPACE, &mut cut).unwrap();
                    assert_eq!(cut.finish().unwrap_err(), [1., 2., 3., 4.]);
                });
            }
        }
        assert!(parse::<[f64; 3]>(b"1 2", &SPACE).is_err());
        assert_eq!(
            parse::<[f64; 3]>(b"(1 2 3)(4 5 6)", &SPACE_PAREN).unwrap(),
            [[1., 2., 3.], [4., 5., 6.]]
        );
    }

    /// Invariant: base64 decodes in pieces to what one decode gives, piece
    /// after piece onto the same vector, with its padding only at the
    /// end. A piece decoded into the wrong part of the result, or padding
    /// accepted inside the data, would change the bytes.
    #[test]
    fn base64_in_pieces() {
        let data: Vec<u8> = (0..100_001u32).map(|i| (i * 31 % 251) as u8).collect();
        let text = base64::engine::general_purpose::STANDARD.encode(&data);
        for threads in [1, 5] {
            par::with_threads(threads, || {
                let mut out = Vec::new();
                assert_eq!(
                    base64_append(text.as_bytes(), &mut out).unwrap(),
                    data.len()
                );
                assert_eq!(out, data);
                let mut out = vec![7u8];
                let cut = 4 * 1001;
                base64_append(&text.as_bytes()[..cut], &mut out).unwrap();
                base64_append(&text.as_bytes()[cut..], &mut out).unwrap();
                assert_eq!(out[1..], data);
                assert!(base64_append(b"QUJD=QUJD", &mut out).is_err());
                assert!(base64_append(b"QUJ=QUJD", &mut out).is_err());
                assert!(base64_append(b"QUJDQ", &mut out).is_err());
            });
        }
        let v = le(&[1, 0, 0, 0, 2, 0, 0, 0], 4, |c| {
            u32::from_le_bytes([c[0], c[1], c[2], c[3]])
        });
        assert_eq!(v, [1, 2]);
    }
}
