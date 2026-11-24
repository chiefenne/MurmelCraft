//! Design tokens (`assets/tokens.toml`) and the theme setting. egui's
//! Visuals and the wgpu viewport both take their colours from here.

use std::collections::BTreeMap;

use murmel_mesh::{CellKind, PatchRole};
use serde::{Deserialize, Serialize};

const TOKENS: &str = include_str!("../../../assets/tokens.toml");

/// An sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    pub fn parse(hex: &str) -> Option<Rgb> {
        let h = hex.strip_prefix('#')?;
        if h.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        Some(Rgb([(v >> 16) as u8, (v >> 8) as u8, v as u8]))
    }

    /// Linear RGB, for the renderer: the sRGB transfer function, looked
    /// up in a table of the 256 values (a surface asks for two colours
    /// per polygon, millions of times).
    pub fn linear(self) -> [f32; 3] {
        static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
        let table = TABLE.get_or_init(|| {
            std::array::from_fn(|c| {
                let c = c as f32 / 255.0;
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            })
        });
        self.0.map(|c| table[c as usize])
    }

    pub fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0[0], self.0[1], self.0[2])
    }
}

impl Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("bad colour {s}")))
    }
}

/// The colours of one theme.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Palette {
    pub bg_app: Rgb,
    pub bg_panel: Rgb,
    pub bg_raised: Rgb,
    pub line: Rgb,
    pub text_primary: Rgb,
    pub text_secondary: Rgb,
    pub viewport_top: Rgb,
    pub viewport_bottom: Rgb,
    pub accent: Rgb,
    pub status_ok: Rgb,
    pub status_warn: Rgb,
    pub status_error: Rgb,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct MeshTokens {
    /// The one colour of every face in colour mode `solid` (the default).
    pub solid: Rgb,
    pub patch: Vec<Rgb>,
    pub edge_darken: f32,
    pub logo: Rgb,
    pub axis_x: Rgb,
    pub axis_y: Rgb,
    pub axis_z: Rgb,
    pub selection: Rgb,
    pub selection_mix: f32,
    /// The quality ramp, good to bad.
    pub quality: Vec<Rgb>,
    pub role: RoleTokens,
    pub kind: KindTokens,
    pub field: FieldTokens,
}

/// Colours of field colouring that no colour map gives.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct FieldTokens {
    /// Faces without a value.
    pub none: Rgb,
    /// Mesh edges drawn over a field.
    pub edge: Rgb,
    /// Contour lines, and how much of their colour they mix in.
    pub line: Rgb,
    pub line_mix: f32,
    /// The mesh's faint outline while slices or iso-surfaces show.
    pub outline: Rgb,
    /// Vector arrows in their own colour.
    pub arrow: Rgb,
    /// Streamlines in their own colour.
    pub stream: Rgb,
    /// Spray parcels in their own colour.
    pub parcel: Rgb,
    /// The boxes of an overview's blocks.
    pub block: Rgb,
    /// The boxes of the blocks the regions read.
    pub block_read: Rgb,
}

/// Colours of the patch roles.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct RoleTokens {
    pub wall: Rgb,
    pub inlet: Rgb,
    pub outlet: Rgb,
    pub symmetry: Rgb,
    pub interface: Rgb,
    pub unspecified: Rgb,
}

/// Colours of the cell types.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct KindTokens {
    pub polyhedron: Rgb,
    pub prism: Rgb,
    pub hexahedron: Rgb,
    pub tetrahedron: Rgb,
    pub pyramid: Rgb,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Tokens {
    pub dark: Palette,
    pub light: Palette,
    pub mesh: MeshTokens,
    pub gizmo: GizmoTokens,
    pub measure: MeasureTokens,
    pub chart: ChartTokens,
}

/// The colours of charts (D268): the series hues per theme, in order.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ChartTokens {
    pub dark: ChartColors,
    pub light: ChartColors,
}

impl ChartTokens {
    pub fn colors(&self, dark: bool) -> &ChartColors {
        if dark { &self.dark } else { &self.light }
    }
}

/// Chart colours in one theme.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ChartColors {
    /// The series hues in their fixed order.
    pub series: Vec<Rgb>,
}

/// The colours of measurements and probes drawn over the viewport, per
/// theme.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct MeasureTokens {
    pub dark: MeasureColors,
    pub light: MeasureColors,
}

impl MeasureTokens {
    pub fn colors(&self, dark: bool) -> &MeasureColors {
        if dark { &self.dark } else { &self.light }
    }
}

/// Measurement colours in one theme.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct MeasureColors {
    /// Distance and angle lines, their points and the snap marker.
    pub line: Rgb,
    /// Probe markers.
    pub probe: Rgb,
    /// The halo under lines and markers, so they read on any face.
    pub halo: Rgb,
}

/// The shading of the clip plane's handles in the viewport: the accent
/// darkened towards `shade` on the sides turned from the light and
/// lifted towards `shine` in the highlight.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct GizmoTokens {
    pub shade: Rgb,
    pub shade_mix: f32,
    pub shine: Rgb,
    pub shine_mix: f32,
    pub dark: GizmoColors,
    pub light: GizmoColors,
}

impl GizmoTokens {
    pub fn colors(&self, dark: bool) -> &GizmoColors {
        if dark { &self.dark } else { &self.light }
    }
}

/// The handles' colours in one theme.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct GizmoColors {
    /// The slide arrow.
    pub slide: Rgb,
    /// The tilt arcs and the rotation plane shown while one is hovered
    /// or dragged.
    pub tilt: Rgb,
}

impl Tokens {
    /// The tokens built into this binary.
    pub fn builtin() -> Result<Tokens, String> {
        let t: Tokens = toml::from_str(TOKENS).map_err(|e| format!("tokens.toml: {e}"))?;
        if t.mesh.patch.is_empty() {
            return Err("tokens.toml: empty patch palette".into());
        }
        if t.mesh.quality.len() < 2 {
            return Err("tokens.toml: the quality ramp needs two colours".into());
        }
        for c in [&t.chart.dark, &t.chart.light] {
            if c.series.len() < crate::chart::MAX_SERIES {
                return Err(format!(
                    "tokens.toml: [chart] needs {} series colours",
                    crate::chart::MAX_SERIES
                ));
            }
        }
        Ok(t)
    }

    pub fn palette(&self, dark: bool) -> &Palette {
        if dark { &self.dark } else { &self.light }
    }

    pub fn patch_color(&self, patch: usize) -> Rgb {
        self.mesh.patch[patch % self.mesh.patch.len()]
    }

    pub fn role_color(&self, role: PatchRole) -> Rgb {
        let r = &self.mesh.role;
        match role {
            PatchRole::Wall => r.wall,
            PatchRole::Inlet => r.inlet,
            PatchRole::Outlet => r.outlet,
            PatchRole::Symmetry => r.symmetry,
            PatchRole::Interface => r.interface,
            PatchRole::Unspecified => r.unspecified,
        }
    }

    /// The quality ramp at `t` (0 good, 1 bad), linear between stops.
    pub fn quality_color(&self, t: f64) -> Rgb {
        let q = &self.mesh.quality;
        let Some(&last) = q.last() else {
            return self.mesh.solid;
        };
        if q.len() < 2 || !t.is_finite() {
            return q[0];
        }
        let x = t.clamp(0.0, 1.0) * (q.len() - 1) as f64;
        let i = (x.floor() as usize).min(q.len() - 2);
        let f = x - i as f64;
        if i + 1 >= q.len() {
            return last;
        }
        let (a, b) = (q[i].0, q[i + 1].0);
        Rgb([0, 1, 2].map(|k| (a[k] as f64 + (b[k] as f64 - a[k] as f64) * f).round() as u8))
    }

    pub fn kind_color(&self, kind: CellKind) -> Rgb {
        let k = &self.mesh.kind;
        match kind {
            CellKind::Polyhedron => k.polyhedron,
            CellKind::Prism => k.prism,
            CellKind::Hexahedron => k.hexahedron,
            CellKind::Tetrahedron => k.tetrahedron,
            CellKind::Pyramid => k.pyramid,
        }
    }
}

/// The user's mesh colours (`[colors]` in settings.toml): each one set
/// replaces its built-in token, so every mesh colour is configurable.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshColors {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid: Option<Rgb>,
    /// The patch palette, in patch order (cycled).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patch: Vec<Rgb>,
    /// Role name (wall, inlet, …) to colour.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub role: BTreeMap<String, Rgb>,
    /// Cell type name (polyhedron, prism, …) to colour.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kind: BTreeMap<String, Rgb>,
    /// Edges take the face colour darkened by this fraction (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_darken: Option<f32>,
    /// The quality ramp, good to bad (two colours or more).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quality: Vec<Rgb>,
    /// The clip handles' colours per theme (`dark`, `light`), each a map
    /// of `slide` and `tilt` to a colour.
    #[serde(default, skip_serializing_if = "UserGizmo::is_empty")]
    pub gizmo: UserGizmo,
    /// The measurement colours per theme (`dark`, `light`), each a map of
    /// `line`, `probe` and `halo` to a colour (same shape as `gizmo`).
    #[serde(default, skip_serializing_if = "UserGizmo::is_empty")]
    pub measure: UserGizmo,
    /// Field colouring and the colours of what draws inside the mesh
    /// (`none`, `edge`, `line`, `outline`, `arrow`, `stream`, `parcel`,
    /// `block`, `block_read`) to a colour.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub field: BTreeMap<String, Rgb>,
}

/// The user's clip handle colours (`[colors.gizmo.dark]` and
/// `[colors.gizmo.light]` in settings.toml).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserGizmo {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dark: BTreeMap<String, Rgb>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub light: BTreeMap<String, Rgb>,
}

impl UserGizmo {
    pub fn is_empty(&self) -> bool {
        self.dark.is_empty() && self.light.is_empty()
    }
}

impl MeshColors {
    /// Lay `other` over these colours: what it sets wins.
    pub fn merge(&mut self, other: MeshColors) {
        if other.solid.is_some() {
            self.solid = other.solid;
        }
        if !other.patch.is_empty() {
            self.patch = other.patch;
        }
        self.role.extend(other.role);
        self.kind.extend(other.kind);
        if other.edge_darken.is_some() {
            self.edge_darken = other.edge_darken;
        }
        if !other.quality.is_empty() {
            self.quality = other.quality;
        }
        self.gizmo.dark.extend(other.gizmo.dark);
        self.gizmo.light.extend(other.gizmo.light);
        self.measure.dark.extend(other.measure.dark);
        self.measure.light.extend(other.measure.light);
        self.field.extend(other.field);
    }
}

impl Tokens {
    /// These tokens with the user's mesh colours laid over them; an
    /// unknown role or cell type, or an edge darkening outside 0 to 1, is
    /// refused.
    pub fn with_colors(&self, c: &MeshColors) -> Result<Tokens, String> {
        let mut t = self.clone();
        let m = &mut t.mesh;
        if let Some(s) = c.solid {
            m.solid = s;
        }
        if !c.patch.is_empty() {
            m.patch = c.patch.clone();
        }
        for (name, &rgb) in &c.role {
            let r = &mut m.role;
            *match name.as_str() {
                "wall" => &mut r.wall,
                "inlet" => &mut r.inlet,
                "outlet" => &mut r.outlet,
                "symmetry" => &mut r.symmetry,
                "interface" => &mut r.interface,
                "unspecified" => &mut r.unspecified,
                _ => return Err(format!("unknown role {name}")),
            } = rgb;
        }
        for (name, &rgb) in &c.kind {
            let k = &mut m.kind;
            *match name.as_str() {
                "polyhedron" => &mut k.polyhedron,
                "prism" => &mut k.prism,
                "hexahedron" => &mut k.hexahedron,
                "tetrahedron" => &mut k.tetrahedron,
                "pyramid" => &mut k.pyramid,
                _ => return Err(format!("unknown cell type {name}")),
            } = rgb;
        }
        if let Some(d) = c.edge_darken {
            if !(0.0..=1.0).contains(&d) {
                return Err(format!("edge_darken {d} is not between 0 and 1"));
            }
            m.edge_darken = d;
        }
        match c.quality.len() {
            0 => {}
            1 => return Err("the quality ramp needs two colours or more".into()),
            _ => m.quality = c.quality.clone(),
        }
        for (theme, user, g) in [
            ("dark", &c.gizmo.dark, &mut t.gizmo.dark),
            ("light", &c.gizmo.light, &mut t.gizmo.light),
        ] {
            for (name, &rgb) in user {
                *match name.as_str() {
                    "slide" => &mut g.slide,
                    "tilt" => &mut g.tilt,
                    _ => return Err(format!("unknown handle {name} (gizmo.{theme})")),
                } = rgb;
            }
        }
        for (theme, user, g) in [
            ("dark", &c.measure.dark, &mut t.measure.dark),
            ("light", &c.measure.light, &mut t.measure.light),
        ] {
            for (name, &rgb) in user {
                *match name.as_str() {
                    "line" => &mut g.line,
                    "probe" => &mut g.probe,
                    "halo" => &mut g.halo,
                    _ => return Err(format!("unknown measure colour {name} (measure.{theme})")),
                } = rgb;
            }
        }
        for (name, &rgb) in &c.field {
            let f = &mut t.mesh.field;
            *match name.as_str() {
                "none" => &mut f.none,
                "edge" => &mut f.edge,
                "line" => &mut f.line,
                "outline" => &mut f.outline,
                "arrow" => &mut f.arrow,
                "stream" => &mut f.stream,
                "parcel" => &mut f.parcel,
                "block" => &mut f.block,
                "block_read" => &mut f.block_read,
                _ => return Err(format!("unknown field colour {name}")),
            } = rgb;
        }
        Ok(t)
    }
}

/// The theme setting (decided 2026-11-23: dark by default).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Dark,
    Light,
    System,
}

impl Theme {
    /// Whether the dark palette applies, given the system's preference.
    pub fn is_dark(self, system_dark: Option<bool>) -> bool {
        match self {
            Theme::Dark => true,
            Theme::Light => false,
            Theme::System => system_dark.unwrap_or(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the built-in tokens parse and carry the spec's accents.
    #[test]
    fn builtin_tokens() {
        let t = Tokens::builtin().unwrap();
        assert_eq!(t.dark.accent.hex(), "#DE74B6");
        assert_eq!(t.light.accent.hex(), "#AD3285");
        assert_eq!(t.light.accent, t.mesh.logo);
        assert_eq!(t.mesh.patch.len(), 12);
        assert_eq!(t.patch_color(12), t.patch_color(0));
        assert_eq!(t.mesh.solid.hex(), "#6EC64A");
    }

    /// Invariant: user colours replace exactly the tokens they name, and a
    /// name that is no role or cell type is refused rather than ignored.
    #[test]
    fn user_colours_replace_tokens() {
        let t = Tokens::builtin().unwrap();
        let red = Rgb([255, 0, 0]);
        let c = MeshColors {
            solid: Some(red),
            role: [("inlet".to_string(), red)].into(),
            kind: [("prism".to_string(), red)].into(),
            edge_darken: Some(0.3),
            ..MeshColors::default()
        };
        let u = t.with_colors(&c).unwrap();
        assert_eq!(u.mesh.solid, red);
        assert_eq!(u.role_color(PatchRole::Inlet), red);
        assert_eq!(u.role_color(PatchRole::Wall), t.role_color(PatchRole::Wall));
        assert_eq!(u.kind_color(CellKind::Prism), red);
        assert_eq!(u.mesh.edge_darken, 0.3);
        assert_eq!(u.mesh.patch, t.mesh.patch);
        let bad = MeshColors {
            role: [("door".to_string(), red)].into(),
            ..MeshColors::default()
        };
        assert!(t.with_colors(&bad).is_err());
        let text = toml::to_string(&c).unwrap();
        assert_eq!(toml::from_str::<MeshColors>(&text).unwrap(), c);
    }

    /// Invariant: the tilt arcs differ from the slide arrow in both
    /// themes (the maintainer's Mac test asked for it), the arrow keeps
    /// the accent, and the user's handle colours replace exactly the one
    /// they name in the one theme they name; an unknown handle is
    /// refused. Breaks if the tokens or `with_colors` lose the gizmo
    /// colours; the window draws them but no window test reads colours.
    #[test]
    fn gizmo_colours() {
        let t = Tokens::builtin().unwrap();
        for dark in [true, false] {
            let g = t.gizmo.colors(dark);
            assert_eq!(g.slide, t.palette(dark).accent);
            assert_ne!(g.tilt, g.slide);
        }
        let red = Rgb([255, 0, 0]);
        let c = MeshColors {
            gizmo: UserGizmo {
                light: [("tilt".to_string(), red)].into(),
                ..UserGizmo::default()
            },
            ..MeshColors::default()
        };
        let u = t.with_colors(&c).unwrap();
        assert_eq!(u.gizmo.light.tilt, red);
        assert_eq!(u.gizmo.light.slide, t.gizmo.light.slide);
        assert_eq!(u.gizmo.dark, t.gizmo.dark);
        let text = toml::to_string(&c).unwrap();
        assert_eq!(toml::from_str::<MeshColors>(&text).unwrap(), c);
        let bad = MeshColors {
            gizmo: UserGizmo {
                dark: [("ring".to_string(), red)].into(),
                ..UserGizmo::default()
            },
            ..MeshColors::default()
        };
        assert!(t.with_colors(&bad).is_err());
    }

    /// Invariant (WCAG 2.2 AA, 1.4.3 and 1.4.11): in both themes, primary
    /// and secondary text reach 4.5:1 on every background text sits on
    /// (app, panel, raised); the error text of the status bar reaches
    /// 4.5:1 on the app background; the accent (focus ring, selection
    /// outline) and the status dots reach 3:1 on the backgrounds they are
    /// drawn on, the focus ring also on both ends of the viewport
    /// gradient. Breaks when a token is lightened or darkened past these;
    /// no other test reads the palettes' contrast.
    #[test]
    fn contrast_is_aa() {
        fn lum(c: Rgb) -> f32 {
            let [r, g, b] = c.linear();
            0.2126 * r + 0.7152 * g + 0.0722 * b
        }
        fn ratio(a: Rgb, b: Rgb) -> f32 {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        }
        let t = Tokens::builtin().unwrap();
        for (theme, p) in [("dark", &t.dark), ("light", &t.light)] {
            let check = |what: &str, fg: Rgb, bg: (&str, Rgb), min: f32| {
                let r = ratio(fg, bg.1);
                assert!(
                    r >= min,
                    "{theme}: {what} on {} is {r:.2}:1, below {min}",
                    bg.0
                );
            };
            let text_bgs = [
                ("bg_app", p.bg_app),
                ("bg_panel", p.bg_panel),
                ("bg_raised", p.bg_raised),
            ];
            for bg in text_bgs {
                check("text_primary", p.text_primary, bg, 4.5);
                check("text_secondary", p.text_secondary, bg, 4.5);
                check("accent", p.accent, bg, 3.0);
            }
            for bg in [
                ("viewport_top", p.viewport_top),
                ("viewport_bottom", p.viewport_bottom),
            ] {
                check("accent", p.accent, bg, 3.0);
                check("text_secondary", p.text_secondary, bg, 4.5);
            }
            check("status_error", p.status_error, ("bg_app", p.bg_app), 4.5);
            for (what, c) in [
                ("status_ok", p.status_ok),
                ("status_warn", p.status_warn),
                ("status_error", p.status_error),
            ] {
                check(what, c, ("bg_panel", p.bg_panel), 3.0);
            }
        }
    }

    /// OKLab of a linear colour (Ottosson 2020).
    fn oklab([r, g, b]: [f32; 3]) -> [f32; 3] {
        let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_99 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        [
            0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        ]
    }

    /// Full-severity colour-vision simulations (Machado, Oliveira and
    /// Fernandes 2009) on linear RGB, with normal vision first.
    const VISION: [[[f32; 3]; 3]; 4] = [
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [
            [0.152_286, 1.052_583, -0.204_868],
            [0.114_503, 0.786_281, 0.099_216],
            [-0.003_882, -0.048_116, 1.051_998],
        ],
        [
            [0.367_322, 0.860_646, -0.227_968],
            [0.280_085, 0.672_501, 0.047_413],
            [-0.011_820, 0.042_940, 0.968_881],
        ],
        [
            [1.255_528, -0.076_749, -0.178_779],
            [-0.078_411, 0.930_809, 0.147_602],
            [0.004_733, 0.691_367, 0.303_900],
        ],
    ];

    /// The smallest OKLab distance between any two of `colors` under the
    /// simulations `vision`.
    fn min_distance(colors: &[Rgb], vision: &[[[f32; 3]; 3]]) -> f32 {
        let mut best = f32::MAX;
        for m in vision {
            let lab: Vec<[f32; 3]> = colors
                .iter()
                .map(|c| {
                    let v = c.linear();
                    oklab(m.map(|row| (0..3).map(|k| row[k] * v[k]).sum::<f32>().clamp(0.0, 1.0)))
                })
                .collect();
            for i in 0..lab.len() {
                for j in i + 1..lab.len() {
                    let d = (0..3).map(|k| (lab[i][k] - lab[j][k]).powi(2)).sum::<f32>();
                    best = best.min(d.sqrt());
                }
            }
        }
        best
    }

    /// Invariant (UI spec: palettes tested for colour-blind safety): the
    /// role and cell-type colours stay apart (OKLab distance at least 0.1)
    /// in normal vision and under simulated protanopia, deuteranopia and
    /// tritanopia; the first four patch colours stay at least 0.05 apart
    /// under all four, and all twelve at least 0.07 in normal vision.
    #[test]
    fn palettes_are_colour_blind_safe() {
        let t = Tokens::builtin().unwrap();
        let roles: Vec<Rgb> = PatchRole::ALL.iter().map(|&r| t.role_color(r)).collect();
        let kinds: Vec<Rgb> = CellKind::ALL.iter().map(|&k| t.kind_color(k)).collect();
        assert!(min_distance(&roles, &VISION) >= 0.1);
        assert!(min_distance(&kinds, &VISION) >= 0.1);
        assert!(min_distance(&t.mesh.patch[..4], &VISION) >= 0.05);
        assert!(min_distance(&t.mesh.patch, &VISION[..1]) >= 0.07);
    }

    /// Invariant (UI spec: quality is sequential and colour-blind safe):
    /// the quality ramp's OKLab lightness falls by at least 0.04 from each
    /// stop to the next under normal vision and the three simulations, so
    /// worse always reads darker; the ends map to the first and last stop.
    /// Breaks when a stop is reordered or a hue-only step is added; the
    /// palette test above checks only categorical colours.
    #[test]
    fn quality_ramp_is_sequential() {
        let t = Tokens::builtin().unwrap();
        let q = &t.mesh.quality;
        for m in &VISION {
            let l: Vec<f32> = q
                .iter()
                .map(|c| {
                    let v = c.linear();
                    oklab(m.map(|row| (0..3).map(|k| row[k] * v[k]).sum::<f32>().clamp(0.0, 1.0)))
                        [0]
                })
                .collect();
            for w in l.windows(2) {
                assert!(w[0] - w[1] >= 0.04, "{l:?}");
            }
        }
        assert_eq!(t.quality_color(0.0), q[0]);
        assert_eq!(t.quality_color(1.0), *q.last().unwrap());
        assert_eq!(t.quality_color(f64::NAN), q[0]);
        let one = MeshColors {
            quality: vec![q[0]],
            ..MeshColors::default()
        };
        assert!(t.with_colors(&one).is_err());
    }
}
