//! Fast reading of large arrays of numbers, shared by the readers.
//!
//! Ascii text is cut into one piece per thread at a separator, the tokens
//! of each piece are counted, and then every piece is parsed in place
//! into its part of the result (no intermediate vectors). Integers are
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
pub(crate) struct Sep([bool; 256]);

impl Sep {
    const fn of(chars: &[u8]) -> Sep {
        let mut t = [false; 256];
        let mut i = 0;
        while i < chars.len() {
            t[chars[i] as usize] = true;
            i += 1;
        }
        Sep(t)
    }

    #[inline]
    pub(crate) fn has(&self, c: u8) -> bool {
        self.0[c as usize]
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

/// The tokens of `text`: runs of bytes that are not separators.
fn count_tokens(text: &[u8], sep: &Sep) -> usize {
    let mut n = 0;
    let mut gap = true;
    for &c in text {
        let s = sep.has(c);
        n += usize::from(gap && !s);
        gap = s;
    }
    n
}

/// Parse the tokens of `text` into `out` (sized by `count_tokens`).
fn parse_into<T: Number>(text: &[u8], sep: &Sep, out: &mut [T]) -> Result<(), String> {
    let (mut i, n) = (0, text.len());
    for x in out.iter_mut() {
        while i < n && sep.has(text[i]) {
            i += 1;
        }
        let start = i;
        while i < n && !sep.has(text[i]) {
            i += 1;
        }
        let t = &text[start..i];
        *x = T::parse(t).ok_or_else(|| String::from_utf8_lossy(t).into_owned())?;
    }
    Ok(())
}

/// Parse every token of `text` (tokens between `sep` characters). A token
/// that is not a `T` is returned as the error.
pub(crate) fn parse<T: Number>(text: &[u8], sep: &Sep) -> Result<Vec<T>, String> {
    let mut out = Vec::new();
    parse_append(text, sep, &mut out)?;
    Ok(out)
}

/// Parse every token of `text` onto the end of `out` (a streaming reader
/// hands its text over piece by piece, each ending at a separator). A
/// token that is not a `T` is returned as the error.
pub(crate) fn parse_append<T: Number>(
    text: &[u8],
    sep: &Sep,
    out: &mut Vec<T>,
) -> Result<(), String> {
    let ps = pieces(text, sep);
    let sizes: Vec<usize> = par::collect(ps.len(), 1, |r| {
        r.map(|i| count_tokens(&text[ps[i].clone()], sep)).collect()
    });
    let at = out.len();
    out.resize(at + sizes.iter().sum::<usize>(), T::default());
    // Each piece parses into its own part of `out`.
    let mut parts: Vec<(&[u8], &mut [T])> = Vec::with_capacity(ps.len());
    let mut rest: &mut [T] = &mut out[at..];
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
