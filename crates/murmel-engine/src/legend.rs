//! The colour legend of the view (UI spec, Colour by): one model for the
//! window's legend and for exported pictures and movies, so both show the
//! same entries, ramp and ticks. [`Legend::of`] reads it from the session;
//! front ends only draw it.
//!
//! Ticks follow Heckbert's nice numbers ("Nice numbers for graph labels",
//! Graphics Gems, Academic Press 1990, pp. 61–63): steps of 1, 2 or 5
//! times a power of ten.

use murmel_mesh::{Mesh, PatchRole};

use crate::display::{ColorBy, Display, Item, all_items};
use crate::i18n::I18n;
use crate::quality::Metric;
use crate::session::Session;
use crate::theme::{Rgb, Tokens};

/// Most ticks a ramp gets from [`Legend::of`].
pub const MAX_TICKS: usize = 6;

/// A string of the legend: a translation key, or a name from the mesh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Label {
    /// A key of `assets/i18n/en/murmel.ftl` (`role-wall`, `kind-prism`).
    Key(String),
    /// A name from the mesh file (patch and zone names), shown as it is.
    Text(String),
}

impl Label {
    /// The string to show: the translation of a key, a name as it is.
    pub fn text(&self, i18n: &I18n) -> String {
        match self {
            Label::Key(k) => i18n.tr(k),
            Label::Text(t) => t.clone(),
        }
    }
}

/// One swatch of a legend of categories.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub label: Label,
    pub color: Rgb,
}

/// A continuous colour bar: the colour map over a value range.
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    /// Colours at positions 0 (low end) to 1 (high end), ascending;
    /// colours between two stops are mixed linearly ([`Ramp::sample`]).
    pub stops: Vec<(f32, Rgb)>,
    /// The values at the low and the high end of the bar.
    pub lo: f64,
    pub hi: f64,
    /// Tick values inside `lo..=hi` ([`ticks`]).
    pub ticks: Vec<f64>,
    /// Unit printed after the title (`°`, `m/s`).
    pub unit: Option<String>,
    /// Banded colouring: the range shows as this many flat bands.
    pub bands: Option<usize>,
    /// Values where contour lines are drawn (also marked on the bar).
    pub contours: Vec<f64>,
    /// Colour mode quality: cells better than this value are hidden.
    pub threshold: Option<f64>,
}

/// What the legend shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// Named colours (patches, roles, cell types, zones).
    Categories(Vec<Entry>),
    /// A colour bar with ticks (quality, later field values).
    Ramp(Ramp),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Legend {
    pub title: Label,
    pub body: Body,
}

impl Legend {
    /// The legend of the session's colouring, or `None` when there is
    /// nothing to explain (no mesh, no entry, quality not computed yet).
    pub fn of(s: &Session) -> Option<Legend> {
        let m = s.mesh.as_ref()?;
        let d = &s.display;
        if d.color_by == ColorBy::Quality {
            return quality(d, &s.tokens);
        }
        let entries = categories(&m.mesh, d, &s.tokens);
        (!entries.is_empty()).then(|| Legend {
            title: Label::Key(format!("color-by-{}", d.color_by.key())),
            body: Body::Categories(entries),
        })
    }
}

/// The swatches of colour modes other than quality, as the window has
/// always listed them: in solid the mesh and every item with a colour of
/// its own, in role the roles the patches have, in patch, cell type and
/// zone the items of that kind; hidden items are left out.
fn categories(mesh: &Mesh, d: &Display, t: &Tokens) -> Vec<Entry> {
    let shown = |i: Item| -> Option<Entry> {
        if d.is_hidden(i) {
            return None;
        }
        d.item_color(mesh, t, i).map(|color| Entry {
            label: item_label(mesh, i),
            color,
        })
    };
    match d.color_by {
        ColorBy::Solid => std::iter::once(Item::Mesh)
            .chain(all_items(mesh))
            .filter_map(shown)
            .collect(),
        ColorBy::Role => PatchRole::ALL
            .into_iter()
            .filter(|r| mesh.patches.iter().any(|p| p.role == *r))
            .map(|r| Entry {
                label: Label::Key(format!("role-{}", r.name())),
                color: t.role_color(r),
            })
            .collect(),
        ColorBy::Patch | ColorBy::CellType | ColorBy::Zone => {
            all_items(mesh).into_iter().filter_map(shown).collect()
        }
        ColorBy::Quality => Vec::new(),
    }
}

/// The name of an item: translated for the mesh and the cell types, as in
/// the file for patches and zones.
pub fn item_label(mesh: &Mesh, i: Item) -> Label {
    match i {
        Item::Mesh => Label::Key("props-area-mesh".into()),
        Item::Kind(k) => Label::Key(format!("kind-{}", k.name())),
        other => Label::Text(
            other
                .id(mesh)
                .split_once(':')
                .map_or(String::new(), |(_, n)| n.to_string()),
        ),
    }
}

/// The quality ramp over the shown metric's range, good (low) to bad.
fn quality(d: &Display, t: &Tokens) -> Option<Legend> {
    let (lo, hi) = d.quality_range()?;
    let metric: Metric = d.metric;
    Some(Legend {
        title: Label::Key(format!("metric-{}", metric.key())),
        body: Body::Ramp(Ramp {
            stops: even_stops(&t.mesh.quality),
            lo,
            hi,
            ticks: ticks(lo, hi, MAX_TICKS),
            unit: metric.unit().map(str::to_string),
            bands: None,
            contours: Vec::new(),
            threshold: d.threshold,
        }),
    })
}

/// Colours spread evenly over 0..1, the way the quality ramp of the
/// tokens is laid out.
pub fn even_stops(colors: &[Rgb]) -> Vec<(f32, Rgb)> {
    let n = colors.len();
    colors
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let t = if n > 1 {
                i as f32 / (n - 1) as f32
            } else {
                0.0
            };
            (t, c)
        })
        .collect()
}

impl Ramp {
    /// The colour at `t` (0 low end, 1 high end), mixed linearly in sRGB
    /// between the two stops around it, as `Tokens::quality_color` mixes
    /// the faces' colours.
    pub fn sample(&self, t: f64) -> Rgb {
        let Some(&(_, first)) = self.stops.first() else {
            return Rgb([0, 0, 0]);
        };
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        } as f32;
        let mut prev = (self.stops[0].0, first);
        for &(at, c) in &self.stops {
            if t <= at {
                let span = at - prev.0;
                if span <= 0.0 {
                    return c;
                }
                let f = ((t - prev.0) / span) as f64;
                let (a, b) = (prev.1.0, c.0);
                return Rgb([0, 1, 2]
                    .map(|k| (a[k] as f64 + (b[k] as f64 - a[k] as f64) * f).round() as u8));
            }
            prev = (at, c);
        }
        prev.1
    }

    /// The printed form of tick `x`: as many decimals as the step between
    /// the ticks needs (0, 0.2, 0.4, not 0.20), a true minus sign.
    pub fn tick_label(&self, x: f64) -> String {
        tick_label(x, tick_step(&self.ticks))
    }

    /// Where `value` lies along the bar, 0 at `lo` and 1 at `hi`.
    pub fn position(&self, value: f64) -> f64 {
        if self.hi > self.lo {
            (value - self.lo) / (self.hi - self.lo)
        } else {
            0.0
        }
    }
}

/// Heckbert's nice number near `x` > 0: 1, 2, 5 or 10 times a power of
/// ten, rounded to the nearest one (`round`) or the next one up.
pub fn nice_number(x: f64, round: bool) -> f64 {
    let e = x.log10().floor();
    let f = x / 10f64.powf(e);
    let nf = if round {
        if f < 1.5 {
            1.0
        } else if f < 3.0 {
            2.0
        } else if f < 7.0 {
            5.0
        } else {
            10.0
        }
    } else if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    };
    nf * 10f64.powf(e)
}

/// Tick values for a bar from `lo` to `hi`: Heckbert's loose labelling
/// with about `max_count` ticks, kept to those inside the range (the bar
/// ends exactly at `lo` and `hi`). At most `max_count` values, ascending,
/// rounded to the step's decimals so they print short.
pub fn ticks(lo: f64, hi: f64, max_count: usize) -> Vec<f64> {
    if !lo.is_finite() || !hi.is_finite() || max_count == 0 {
        return Vec::new();
    }
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    if hi - lo <= f64::EPSILON * lo.abs().max(1.0) || max_count == 1 {
        return vec![lo];
    }
    let range = nice_number(hi - lo, false);
    let mut step = nice_number(range / (max_count.max(2) - 1) as f64, true);
    loop {
        let out = ticks_with(lo, hi, step);
        if out.len() <= max_count {
            return out;
        }
        // Too many inside the range: take the next nice step up.
        step = nice_number(step * 1.5, false);
    }
}

/// The step between ticks (0 for fewer than two).
fn tick_step(ticks: &[f64]) -> f64 {
    match ticks {
        [a, b, ..] => (b - a).abs(),
        _ => 0.0,
    }
}

/// `x` with the decimals a tick `step` apart needs; very large or small
/// values in exponent form.
pub fn tick_label(x: f64, step: f64) -> String {
    if !x.is_finite() {
        return format!("{x}");
    }
    let a = x.abs().max(step);
    let s = if a != 0.0 && !(1e-4..1e7).contains(&a) {
        format!("{x:.2e}")
    } else {
        let decimals = if step > 0.0 {
            (-step.log10().floor()).max(0.0) as usize
        } else {
            let mag = if x == 0.0 {
                0.0
            } else {
                x.abs().log10().floor()
            };
            (3.0 - mag).clamp(0.0, 9.0) as usize
        };
        let s = format!("{x:.decimals$}");
        if step > 0.0 || !s.contains('.') {
            s
        } else {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
    };
    let s = if s
        .trim_start_matches('-')
        .chars()
        .all(|c| c == '0' || c == '.')
    {
        s.trim_start_matches('-').to_string()
    } else {
        s
    };
    s.replace('-', "\u{2212}")
}

fn ticks_with(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    let decimals = (-step.log10().floor()).max(0.0) as i32;
    let scale = 10f64.powi(decimals);
    let tol = step * 1e-9;
    let first = (lo / step).ceil() as i64;
    let last = (hi / step).floor() as i64;
    (first..=last)
        .map(|k| ((k as f64 * step) * scale).round() / scale)
        .filter(|&x| x >= lo - tol && x <= hi + tol)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: ticks are 1, 2 or 5 times a power of ten apart, inside
    /// the range, at most `max_count`, and print without float noise
    /// (0.3, not 0.30000000000000004). A change that drops the rounding
    /// or the range filter breaks it; the window and exports both print
    /// these values, so no other test sees them.
    #[test]
    fn nice_ticks() {
        assert_eq!(ticks(0.0, 1.0, 6), vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
        assert_eq!(
            ticks(0.0, 100.0, 6),
            vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]
        );
        assert_eq!(ticks(3.2, 71.9, 6), vec![20.0, 40.0, 60.0]);
        assert_eq!(ticks(0.1, 0.7, 6), vec![0.2, 0.4, 0.6]);
        let tenths = ticks(0.0, 0.9, 10);
        assert_eq!(tenths.len(), 10);
        assert_eq!(tenths[3], 0.3);
        assert_eq!(tenths[7], 0.7);
        for (lo, hi, n) in [
            (1.0, 1.7, 5),
            (-3.0, 12.0, 6),
            (1e-5, 3e-5, 4),
            (0.0, 89.9, 6),
        ] {
            let t = ticks(lo, hi, n);
            assert!(!t.is_empty() && t.len() <= n, "{lo}..{hi}: {t:?}");
            assert!(
                t.iter().all(|&x| x >= lo - 1e-12 && x <= hi + 1e-12),
                "{t:?}"
            );
            assert!(t.windows(2).all(|w| w[1] > w[0]), "{t:?}");
        }
        assert_eq!(ticks(5.0, 5.0, 6), vec![5.0]);
        assert_eq!(ticks(f64::NAN, 1.0, 6), Vec::<f64>::new());
        assert_eq!(nice_number(23.0, true), 20.0);
        assert_eq!(nice_number(23.0, false), 50.0);
        assert_eq!(nice_number(0.18, true), 0.2);
    }

    /// Invariant: tick labels carry the step's decimals and a true minus
    /// (the window and exports print the same strings).
    #[test]
    fn tick_labels() {
        assert_eq!(tick_label(0.4, 0.2), "0.4");
        assert_eq!(tick_label(0.0, 0.2), "0.0");
        assert_eq!(tick_label(40.0, 20.0), "40");
        assert_eq!(tick_label(-2.5, 0.5), "\u{2212}2.5");
        assert_eq!(tick_label(-0.0, 1.0), "0");
        assert_eq!(tick_label(2e-5, 1e-5), "2.00e\u{2212}5");
        assert_eq!(tick_label(12.3456, 0.0), "12.35");
    }

    /// Invariant: the ramp samples its stops exactly at their positions
    /// and mixes linearly between them, like the faces' quality colours.
    #[test]
    fn ramp_samples_its_stops() {
        let t = Tokens::builtin().unwrap();
        let r = Ramp {
            stops: even_stops(&t.mesh.quality),
            lo: 0.0,
            hi: 1.0,
            ticks: Vec::new(),
            unit: None,
            bands: None,
            contours: Vec::new(),
            threshold: None,
        };
        for x in [0.0, 0.1, 0.37, 0.5, 0.82, 1.0] {
            assert_eq!(r.sample(x), t.quality_color(x), "at {x}");
        }
        assert_eq!(r.position(0.25), 0.25);
    }

    /// Invariant: the legend lists what the window has always listed: in
    /// patch colouring every patch by its name from the file, without the
    /// hidden ones; in role colouring the roles the patches have; in
    /// quality colouring the ramp over the metric's range with the unit
    /// and threshold. No mesh, no legend.
    #[test]
    fn legend_of_the_channel() {
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir = None;
        assert_eq!(Legend::of(&s), None);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        s.open(&path).unwrap();
        let mesh = s.mesh.as_ref().unwrap().mesh.clone();

        s.display.color_by = ColorBy::Patch;
        let Some(Legend {
            title,
            body: Body::Categories(e),
        }) = Legend::of(&s)
        else {
            panic!("no categories")
        };
        assert_eq!(title, Label::Key("color-by-patch".into()));
        assert_eq!(e.len(), mesh.patches.len());
        assert_eq!(e[0].label, Label::Text(mesh.patches[0].name.clone()));
        let first = Item::parse(&format!("patch:{}", mesh.patches[0].name), &mesh).unwrap();
        s.display.hidden.insert(first);
        let Some(Body::Categories(e)) = Legend::of(&s).map(|l| l.body) else {
            panic!("no categories")
        };
        assert_eq!(e.len(), mesh.patches.len() - 1);

        s.display.color_by = ColorBy::Role;
        let Some(Body::Categories(e)) = Legend::of(&s).map(|l| l.body) else {
            panic!("no roles")
        };
        assert!(
            e.iter()
                .all(|x| matches!(&x.label, Label::Key(k) if k.starts_with("role-")))
        );

        s.display.color_by = ColorBy::Quality;
        s.display.threshold = Some(20.0);
        s.redraw();
        let Some(Legend {
            title,
            body: Body::Ramp(r),
        }) = Legend::of(&s)
        else {
            panic!("no ramp")
        };
        assert_eq!(title, Label::Key("metric-non_orthogonality".into()));
        assert_eq!(Some((r.lo, r.hi)), s.display.quality_range());
        assert_eq!(r.unit.as_deref(), Some("\u{b0}"));
        assert_eq!(r.threshold, Some(20.0));
        assert!(!r.ticks.is_empty() && r.ticks.len() <= MAX_TICKS);
        assert_eq!(r.sample(0.0), s.tokens.quality_color(0.0));
        let i18n = I18n::english().unwrap();
        assert_eq!(title.text(&i18n), "Non-orthogonality");
    }
}
