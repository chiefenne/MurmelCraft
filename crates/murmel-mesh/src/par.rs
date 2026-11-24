//! Data parallelism on scoped threads of the standard library (no thread
//! pool, no new dependency): split work into one contiguous range per
//! thread and join. The readers, the face topology and the engine's
//! surface use it for loops over millions of items.

use std::cell::Cell;
use std::ops::Range;

thread_local! {
    static THREADS: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Run `f` with work split for `n` threads on this thread, however few
/// items there are (tests compare one thread with several on small
/// meshes).
pub fn with_threads<R>(n: usize, f: impl FnOnce() -> R) -> R {
    let before = THREADS.with(|t| t.replace(Some(n.max(1))));
    let r = f();
    THREADS.with(|t| t.set(before));
    r
}

/// The threads to use: the machine's parallelism, or `MURMEL_THREADS`
/// when set (1 turns the parallel paths into plain loops, for tests and
/// for measuring).
pub fn threads() -> usize {
    if let Some(n) = THREADS.with(Cell::get) {
        return n;
    }
    if let Some(n) = std::env::var("MURMEL_THREADS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
    {
        return n.max(1);
    }
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// `0..n` cut into at most `threads()` ranges of at least `min` items.
pub fn ranges(n: usize, min: usize) -> Vec<Range<usize>> {
    let min = if THREADS.with(Cell::get).is_some() {
        1
    } else {
        min.max(1)
    };
    let parts = threads().min(n / min).max(1);
    let step = n.div_ceil(parts);
    (0..parts)
        .map(|i| (i * step).min(n)..((i + 1) * step).min(n))
        .filter(|r| !r.is_empty() || n == 0)
        .collect()
}

/// `f` over the ranges of `0..n` (see [`ranges`]), each on its own
/// thread, results in range order.
pub fn map<R: Send>(n: usize, min: usize, f: impl Fn(Range<usize>) -> R + Sync) -> Vec<R> {
    let rs = ranges(n, min);
    if rs.len() <= 1 {
        return rs.into_iter().map(&f).collect();
    }
    let f = &f;
    std::thread::scope(|s| {
        let handles: Vec<_> = rs.into_iter().map(|r| s.spawn(move || f(r))).collect();
        handles
            .into_iter()
            .map(|h| match h.join() {
                Ok(r) => r,
                Err(p) => std::panic::resume_unwind(p),
            })
            .collect()
    })
}

/// `f` over contiguous pieces of `data`, each with the index of its first
/// item, each on its own thread.
pub fn chunks_mut<T: Send>(data: &mut [T], min: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let rs = ranges(data.len(), min);
    if rs.len() <= 1 {
        f(0, data);
        return;
    }
    let f = &f;
    std::thread::scope(|s| {
        let mut rest = data;
        let mut start = 0;
        let mut handles = Vec::with_capacity(rs.len());
        for r in rs {
            let (head, tail) = rest.split_at_mut(r.len());
            rest = tail;
            let at = start;
            start += r.len();
            handles.push(s.spawn(move || f(at, head)));
        }
        for h in handles {
            if let Err(p) = h.join() {
                std::panic::resume_unwind(p);
            }
        }
    });
}

/// `data` cut at `cuts` (increasing, the first 0 and the last
/// `data.len()`) into consecutive pieces.
pub fn split_mut<'a, T>(data: &'a mut [T], cuts: &[usize]) -> Vec<&'a mut [T]> {
    let mut out = Vec::with_capacity(cuts.len().saturating_sub(1));
    let mut rest = data;
    for w in cuts.windows(2) {
        let (head, tail) = rest.split_at_mut(w[1] - w[0]);
        out.push(head);
        rest = tail;
    }
    out
}

/// The pieces of `f` over the ranges of `0..n`, joined in order.
pub fn collect<T: Send>(n: usize, min: usize, f: impl Fn(Range<usize>) -> Vec<T> + Sync) -> Vec<T> {
    let parts = map(n, min, f);
    let total = parts.iter().map(Vec::len).sum();
    let mut out = Vec::with_capacity(total);
    for p in parts {
        out.extend(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the ranges cover `0..n` once each, in order, and the
    /// parallel helpers give what a plain loop gives. A gap or overlap
    /// would drop or repeat items in every reader that uses them.
    #[test]
    fn ranges_cover_once_and_helpers_match_a_loop() {
        for n in [0, 1, 7, 1000, 12_345] {
            let rs = ranges(n, 10);
            let mut next = 0;
            for r in &rs {
                assert_eq!(r.start, next);
                next = r.end;
            }
            assert_eq!(next, n);
            let squares = collect(n, 10, |r| r.map(|i| i * i).collect());
            assert_eq!(squares, (0..n).map(|i| i * i).collect::<Vec<_>>());
            let mut v = vec![0usize; n];
            chunks_mut(&mut v, 10, |at, s| {
                for (k, x) in s.iter_mut().enumerate() {
                    *x = at + k;
                }
            });
            assert_eq!(v, (0..n).collect::<Vec<_>>());
        }
    }
}
