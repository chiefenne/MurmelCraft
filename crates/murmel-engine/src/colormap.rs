//! The colour maps of field colouring (`assets/colormaps.toml`, written
//! by `assets/colormaps.py`; sources and licences in
//! `assets/PROVENANCE.md`). A map is a list of sRGB colours at even steps
//! from the low end to the high end; colours between two stops are mixed
//! in sRGB, as the sources of the data do.
//!
//! Decided by the maintainer (2026-11-24): viridis for one-sided ranges,
//! the diverging cool-to-warm map (Moreland 2009) for symmetric ranges.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::theme::Rgb;

const MAPS: &str = include_str!("../../../assets/colormaps.toml");

/// The map of one-sided ranges when the user chose none.
pub const DEFAULT: &str = "viridis";
/// The map of symmetric ranges when the user chose none.
pub const DIVERGING: &str = "cool_warm";
/// Texels of the colour table the renderer samples.
pub const LUT_SIZE: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Sequential,
    Diverging,
    Rainbow,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Colormap {
    /// The id tools take (`viridis`); the window translates
    /// `colormap-<id>`.
    pub id: String,
    pub kind: Kind,
    /// Colours at even steps, low end first (two or more).
    pub stops: Vec<Rgb>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    kind: Kind,
    stops: Vec<Rgb>,
}

/// Every colour map, in the order of the file (viridis first).
pub fn all() -> &'static [Colormap] {
    static MAPS_PARSED: OnceLock<Vec<Colormap>> = OnceLock::new();
    MAPS_PARSED.get_or_init(|| parse(MAPS).unwrap_or_default())
}

fn parse(text: &str) -> Result<Vec<Colormap>, String> {
    // Keep the order of the file: read the table names in order first.
    let order: Vec<String> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix('[')?.strip_suffix(']').map(str::to_string))
        .collect();
    let mut table: BTreeMap<String, Entry> =
        toml::from_str(text).map_err(|e| format!("colormaps.toml: {e}"))?;
    let mut out = Vec::new();
    for id in order {
        if let Some(e) = table.remove(&id) {
            if e.stops.len() < 2 {
                return Err(format!("colormaps.toml: {id} needs two colours"));
            }
            out.push(Colormap {
                id,
                kind: e.kind,
                stops: e.stops,
            });
        }
    }
    Ok(out)
}

/// The map called `id`.
pub fn get(id: &str) -> Option<&'static Colormap> {
    all().iter().find(|m| m.id == id)
}

/// The ids of every map.
pub fn ids() -> Vec<&'static str> {
    all().iter().map(|m| m.id.as_str()).collect()
}

impl Colormap {
    /// The colour at `t` (0 low end, 1 high end; clamped), turned end
    /// for end when `reverse`.
    pub fn sample(&self, t: f64, reverse: bool) -> Rgb {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        let t = if reverse { 1.0 - t } else { t };
        let n = self.stops.len() - 1;
        let x = t * n as f64;
        let i = (x.floor() as usize).min(n.saturating_sub(1));
        let f = x - i as f64;
        let (a, b) = (self.stops[i], self.stops[(i + 1).min(n)]);
        Rgb(std::array::from_fn(|k| {
            (a.0[k] as f64 + f * (b.0[k] as f64 - a.0[k] as f64)).round() as u8
        }))
    }

    /// The legend's stops: (position, colour), low end first.
    pub fn legend_stops(&self, reverse: bool) -> Vec<(f32, Rgb)> {
        let n = self.stops.len() - 1;
        (0..=n)
            .map(|i| {
                let t = i as f64 / n as f64;
                (t as f32, self.sample(t, reverse))
            })
            .collect()
    }

    /// The table the renderer samples: `LUT_SIZE` sRGB texels with alpha.
    pub fn lut(&self, reverse: bool) -> Vec<[u8; 4]> {
        (0..LUT_SIZE)
            .map(|i| {
                let c = self.sample(i as f64 / (LUT_SIZE - 1) as f64, reverse);
                [c.0[0], c.0[1], c.0[2], 255]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the file parses into the eight maps in its order, the
    /// two the maintainer chose as defaults among them, each with even
    /// stops; viridis runs from its dark violet to its yellow and
    /// cool_warm from Moreland's blue through a light grey to his red. A
    /// map dropped or misread from the file fails here.
    #[test]
    fn maps_parse_in_order() {
        let ids = ids();
        assert_eq!(
            ids,
            ["viridis", "cool_warm", "plasma", "magma", "inferno", "turbo", "rainbow", "grey"]
        );
        let v = get(DEFAULT).unwrap();
        assert_eq!(v.sample(0.0, false), Rgb([0x44, 0x01, 0x54]));
        assert_eq!(v.sample(1.0, false), Rgb([0xFD, 0xE7, 0x25]));
        assert_eq!(v.sample(1.0, true), Rgb([0x44, 0x01, 0x54]));
        let c = get(DIVERGING).unwrap();
        assert_eq!(c.kind, Kind::Diverging);
        assert_eq!(c.sample(0.0, false), Rgb([59, 76, 192]));
        assert_eq!(c.sample(1.0, false), Rgb([180, 4, 38]));
        let mid = c.sample(0.5, false);
        assert!(mid.0.iter().all(|&k| k > 200), "{mid:?}");
        assert_eq!(v.lut(false).len(), LUT_SIZE);
    }

    /// Invariant (UI spec: colour maps read in order): every sequential
    /// map grows in lightness from end to end, so the colours keep the
    /// order of the values in grey print and for colour-blind viewers.
    #[test]
    fn sequential_maps_grow_lighter() {
        let lum = |c: Rgb| {
            let l = c.linear();
            0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2]
        };
        for m in all().iter().filter(|m| m.kind == Kind::Sequential) {
            for w in m.stops.windows(2) {
                assert!(lum(w[1]) >= lum(w[0]) - 1e-4, "{} not monotone", m.id);
            }
        }
    }
}
