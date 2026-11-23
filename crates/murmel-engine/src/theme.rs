//! Design tokens (`assets/tokens.toml`) and the theme setting. egui's
//! Visuals and the wgpu viewport both take their colours from here.

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

    /// Linear RGB, for the renderer.
    pub fn linear(self) -> [f32; 3] {
        self.0.map(|c| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        })
    }

    pub fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0[0], self.0[1], self.0[2])
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
    pub patch: Vec<Rgb>,
    pub edge_darken: f32,
    pub logo: Rgb,
    pub axis_x: Rgb,
    pub axis_y: Rgb,
    pub axis_z: Rgb,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Tokens {
    pub dark: Palette,
    pub light: Palette,
    pub mesh: MeshTokens,
}

impl Tokens {
    /// The tokens built into this binary.
    pub fn builtin() -> Result<Tokens, String> {
        let t: Tokens = toml::from_str(TOKENS).map_err(|e| format!("tokens.toml: {e}"))?;
        if t.mesh.patch.is_empty() {
            return Err("tokens.toml: empty patch palette".into());
        }
        Ok(t)
    }

    pub fn palette(&self, dark: bool) -> &Palette {
        if dark { &self.dark } else { &self.light }
    }

    pub fn patch_color(&self, patch: usize) -> Rgb {
        self.mesh.patch[patch % self.mesh.patch.len()]
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
        assert_eq!(t.dark.accent.hex(), "#D671B4");
        assert_eq!(t.light.accent.hex(), "#A3337E");
        assert_eq!(t.mesh.patch.len(), 12);
        assert_eq!(t.patch_color(12), t.patch_color(0));
    }

    /// WCAG relative luminance contrast of primary text on the panel and of
    /// the accent on the app background: at least AA (4.5 and 3).
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
        for p in [&t.dark, &t.light] {
            assert!(ratio(p.text_primary, p.bg_panel) >= 4.5);
            assert!(ratio(p.text_secondary, p.bg_panel) >= 4.5);
            assert!(ratio(p.accent, p.bg_app) >= 3.0);
        }
    }
}
