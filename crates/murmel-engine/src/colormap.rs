//! The colour maps of field colouring (`assets/colormaps.toml`, written
//! by `assets/colormaps.py`; sources and licences in
//! `assets/PROVENANCE.md`). A map is a list of sRGB colours from the low
//! end to the high end, at even steps unless the map gives their
//! positions; colours between two stops are mixed in sRGB, as the sources
//! of the data do.
//!
//! Decided by the maintainer (2026-11-24): Rainbow Desaturated (ParaView's
//! eight control points) for one-sided ranges, the same default as
//! FoehnCraft's and KoflCraft's pictures, with Inferno as the second
//! choice; the diverging cool-to-warm map (Moreland 2009) for symmetric
//! ranges.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::theme::Rgb;

const MAPS: &str = include_str!("../../../assets/colormaps.toml");

/// The map of one-sided ranges when the user chose none.
pub const DEFAULT: &str = "rainbow_desaturated";
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
    /// The id tools take (`rainbow_desaturated`); the window translates
    /// `colormap-<id>`.
    pub id: String,
    pub kind: Kind,
    /// Colours, low end first (two or more).
    pub stops: Vec<Rgb>,
    /// Where each stop sits: 0 for the first, 1 for the last, rising.
    pub at: Vec<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    kind: Kind,
    stops: Vec<Rgb>,
    /// Positions of the stops; left out, the stops are at even steps.
    at: Option<Vec<f64>>,
}

/// Every colour map, in the order of the file (the default first).
pub fn all() -> &'static [Colormap] {
    static MAPS_PARSED: OnceLock<Vec<Colormap>> = OnceLock::new();
    MAPS_PARSED.get_or_init(|| parse(MAPS).unwrap_or_default())
}

fn parse(text: &str) -> Result<Vec<Colormap>, String> {
    // Keep the order of the file: read the table names in order first.
    let order: Vec<String> = text
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix('[')?
                .strip_suffix(']')
                .map(str::to_string)
        })
        .collect();
    let mut table: BTreeMap<String, Entry> =
        toml::from_str(text).map_err(|e| format!("colormaps.toml: {e}"))?;
    let mut out = Vec::new();
    for id in order {
        if let Some(e) = table.remove(&id) {
            let n = e.stops.len();
            if n < 2 {
                return Err(format!("colormaps.toml: {id} needs two colours"));
            }
            let at = match e.at {
                Some(at) => at,
                None => (0..n).map(|i| i as f64 / (n - 1) as f64).collect(),
            };
            let rising = at.windows(2).all(|w| w[1] > w[0]);
            if at.len() != n || at[0] != 0.0 || at[n - 1] != 1.0 || !rising {
                return Err(format!(
                    "colormaps.toml: {id}: at must give one rising position per colour, from 0 to 1"
                ));
            }
            out.push(Colormap {
                id,
                kind: e.kind,
                stops: e.stops,
                at,
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
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let t = if reverse { 1.0 - t } else { t };
        // The segment [at[i], at[i + 1]] that holds t.
        let n = self.stops.len() - 1;
        let i = self.at[1..n].partition_point(|&a| a <= t);
        let span = self.at[i + 1] - self.at[i];
        let f = ((t - self.at[i]) / span).clamp(0.0, 1.0);
        let (a, b) = (self.stops[i], self.stops[i + 1]);
        Rgb(std::array::from_fn(|k| {
            (a.0[k] as f64 + f * (b.0[k] as f64 - a.0[k] as f64)).round() as u8
        }))
    }

    /// The legend's stops: (position, colour), low end first.
    pub fn legend_stops(&self, reverse: bool) -> Vec<(f32, Rgb)> {
        if reverse {
            self.at
                .iter()
                .zip(&self.stops)
                .rev()
                .map(|(&a, &c)| ((1.0 - a) as f32, c))
                .collect()
        } else {
            self.at
                .iter()
                .zip(&self.stops)
                .map(|(&a, &c)| (a as f32, c))
                .collect()
        }
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

    /// Invariant: the file parses into the nine maps in its order, the
    /// maintainer's two preferred maps first (2026-11-24); viridis runs
    /// from its dark violet to its yellow and cool_warm from Moreland's
    /// blue through a light grey to his red. A map dropped or misread
    /// from the file fails here.
    #[test]
    fn maps_parse_in_order() {
        let ids = ids();
        assert_eq!(
            ids,
            [
                "rainbow_desaturated",
                "inferno",
                "cool_warm",
                "viridis",
                "plasma",
                "magma",
                "turbo",
                "rainbow",
                "grey"
            ]
        );
        assert_eq!(DEFAULT, ids[0]);
        let v = get("viridis").unwrap();
        assert_eq!(v.sample(0.0, false), Rgb([0x44, 0x01, 0x54]));
        assert_eq!(v.sample(1.0, false), Rgb([0xFD, 0xE7, 0x25]));
        assert_eq!(v.sample(1.0, true), Rgb([0x44, 0x01, 0x54]));
        assert_eq!(v.at.len(), v.stops.len());
        assert!((v.at[1] - 1.0 / 85.0).abs() < 1e-12);
        let c = get(DIVERGING).unwrap();
        assert_eq!(c.kind, Kind::Diverging);
        assert_eq!(c.sample(0.0, false), Rgb([59, 76, 192]));
        assert_eq!(c.sample(1.0, false), Rgb([180, 4, 38]));
        let mid = c.sample(0.5, false);
        assert!(mid.0.iter().all(|&k| k > 200), "{mid:?}");
        assert_eq!(v.lut(false).len(), LUT_SIZE);
    }

    /// Invariant (maintainer, 2026-11-24): the default is ParaView's
    /// "Rainbow Desaturated", the eight control points written out the
    /// same in MurmelCraft, FoehnCraft and KoflCraft and mixed in RGB. A
    /// changed point or position breaks the match between the three
    /// projects' pictures here.
    #[test]
    fn rainbow_desaturated_is_paraviews() {
        let m = get(DEFAULT).unwrap();
        assert_eq!(m.id, "rainbow_desaturated");
        let points: [(f64, [u8; 3]); 8] = [
            (0.0, [0x47, 0x47, 0xDB]),
            (0.143, [0x00, 0x00, 0x5C]),
            (0.285, [0x00, 0xFF, 0xFF]),
            (0.429, [0x00, 0x80, 0x00]),
            (0.571, [0xFF, 0xFF, 0x00]),
            (0.714, [0xFF, 0x61, 0x00]),
            (0.857, [0x6B, 0x00, 0x00]),
            (1.0, [0xE0, 0x4D, 0x4D]),
        ];
        assert_eq!(m.at, points.map(|p| p.0));
        assert_eq!(m.stops, points.map(|p| Rgb(p.1)));
        for (t, c) in points {
            assert_eq!(m.sample(t, false), Rgb(c), "at {t}");
            assert_eq!(m.sample(1.0 - t, true), Rgb(c), "reversed at {t}");
        }
        // Halfway between two points is their mean: cyan to green.
        let half = m.sample(0.357, false).0;
        assert!(half[0] == 0 && half[1].abs_diff(192) <= 1 && half[2].abs_diff(128) <= 1);
        // The legend draws the points where they sit, both ways round.
        let legend = m.legend_stops(false);
        assert_eq!(legend.len(), 8);
        assert_eq!(legend[2], (0.285, Rgb([0x00, 0xFF, 0xFF])));
        let back = m.legend_stops(true);
        assert_eq!(back[0], (0.0, Rgb([0xE0, 0x4D, 0x4D])));
        assert_eq!(back[7], (1.0, Rgb([0x47, 0x47, 0xDB])));
        assert!(back.windows(2).all(|w| w[1].0 > w[0].0));
        // The renderer's table: first and last texel are the end points.
        let lut = m.lut(false);
        assert_eq!(lut[0], [0x47, 0x47, 0xDB, 255]);
        assert_eq!(lut[LUT_SIZE - 1], [0xE0, 0x4D, 0x4D, 255]);
    }

    /// A map whose positions don't rise from 0 to 1, or don't match its
    /// colours, is refused rather than drawn wrongly.
    #[test]
    fn bad_positions_are_refused() {
        let map = |at: &str| {
            parse(&format!(
                "[m]\nkind = \"rainbow\"\nat = {at}\nstops = [\"#000000\", \"#808080\", \"#FFFFFF\"]\n"
            ))
        };
        assert!(map("[0.0, 0.3, 1.0]").is_ok());
        assert!(map("[0.0, 1.0]").is_err());
        assert!(map("[0.1, 0.3, 1.0]").is_err());
        assert!(map("[0.0, 0.3, 0.9]").is_err());
        assert!(map("[0.0, 0.5, 0.5]").is_err());
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
