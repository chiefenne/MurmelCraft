//! The look of Kraxn's window: MurmelCraft's design tokens
//! (`assets/tokens.toml`, dark and light), Inter and Lucide (SIL OFL,
//! ISC), a 4 px grid and 6 px radius. No colour is written here.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
    Visuals,
};
use serde::Deserialize;

const TOKENS: &str = include_str!("../../../assets/tokens.toml");

pub const SEMIBOLD: &str = "semibold";

/// One theme's colours (the `[dark]` and `[light]` tables of the tokens).
#[derive(Clone, Debug, Deserialize)]
pub struct Palette {
    pub bg_app: Hex,
    pub bg_panel: Hex,
    pub bg_raised: Hex,
    pub line: Hex,
    pub text_primary: Hex,
    pub text_secondary: Hex,
    pub accent: Hex,
    pub status_warn: Hex,
    pub status_error: Hex,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Tokens {
    pub dark: Palette,
    pub light: Palette,
}

/// An sRGB colour written `#RRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hex(pub [u8; 3]);

impl<'de> Deserialize<'de> for Hex {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let h = s.trim_start_matches('#');
        let byte = |i: usize| {
            h.get(i..i + 2)
                .and_then(|b| u8::from_str_radix(b, 16).ok())
                .ok_or_else(|| serde::de::Error::custom(format!("bad colour {s}")))
        };
        if h.len() != 6 {
            return Err(serde::de::Error::custom(format!("bad colour {s}")));
        }
        Ok(Hex([byte(0)?, byte(2)?, byte(4)?]))
    }
}

impl Hex {
    pub fn c(self) -> Color32 {
        Color32::from_rgb(self.0[0], self.0[1], self.0[2])
    }
}

impl Tokens {
    pub fn builtin() -> Result<Self, String> {
        toml::from_str(TOKENS).map_err(|e| format!("tokens.toml: {e}"))
    }

    pub fn palette(&self, dark: bool) -> &Palette {
        if dark { &self.dark } else { &self.light }
    }
}

pub fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    let add = |f: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        f.font_data
            .insert(name.to_string(), Arc::new(FontData::from_static(bytes)));
    };
    add(
        &mut f,
        "inter",
        include_bytes!("../../../assets/fonts/Inter-Regular.ttf"),
    );
    add(
        &mut f,
        "inter-semibold",
        include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf"),
    );
    add(
        &mut f,
        "jetbrains-mono",
        include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"),
    );
    add(
        &mut f,
        "lucide",
        include_bytes!("../../../assets/icons/lucide-subset.ttf"),
    );
    let prop = f.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "inter".into());
    prop.insert(1, "lucide".into());
    let mono = f.families.entry(FontFamily::Monospace).or_default();
    mono.insert(0, "jetbrains-mono".into());
    f.families.insert(
        FontFamily::Name(SEMIBOLD.into()),
        vec!["inter-semibold".into(), "inter".into(), "lucide".into()],
    );
    f
}

fn visuals(p: &Palette, dark: bool) -> Visuals {
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let r = CornerRadius::same(6);
    v.panel_fill = p.bg_panel.c();
    v.window_fill = p.bg_raised.c();
    v.extreme_bg_color = p.bg_app.c();
    v.faint_bg_color = p.bg_raised.c();
    v.window_stroke = Stroke::new(1.0, p.line.c());
    v.window_corner_radius = CornerRadius::same(8);
    v.hyperlink_color = p.accent.c();
    v.selection.bg_fill = p.accent.c().gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, p.accent.c());
    v.error_fg_color = p.status_error.c();
    v.warn_fg_color = p.status_warn.c();
    v.override_text_color = Some(p.text_primary.c());
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.bg_panel.c();
    w.noninteractive.weak_bg_fill = p.bg_panel.c();
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.line.c());
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text_secondary.c());
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = p.bg_raised.c();
    w.inactive.weak_bg_fill = p.bg_raised.c();
    w.inactive.bg_stroke = Stroke::new(1.0, p.line.c());
    w.inactive.fg_stroke = Stroke::new(1.5, p.text_secondary.c());
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = p.bg_raised.c();
    w.hovered.weak_bg_fill = p.bg_raised.c();
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent.c());
    w.hovered.fg_stroke = Stroke::new(1.5, p.text_primary.c());
    w.hovered.corner_radius = r;
    w.active.bg_fill = p.accent.c().gamma_multiply(0.25);
    w.active.weak_bg_fill = p.accent.c().gamma_multiply(0.25);
    w.active.bg_stroke = Stroke::new(1.0, p.accent.c());
    w.active.fg_stroke = Stroke::new(1.5, p.accent.c());
    w.active.corner_radius = r;
    w.open = w.active;
    v
}

/// Fonts and both themes' styles.
pub fn install(ctx: &egui::Context, tokens: &Tokens) {
    ctx.set_fonts(fonts());
    for (theme, dark) in [(egui::Theme::Dark, true), (egui::Theme::Light, false)] {
        ctx.style_mut_of(theme, |s| {
            s.visuals = visuals(tokens.palette(dark), dark);
            s.text_styles = [
                (TextStyle::Body, FontId::proportional(14.0)),
                (TextStyle::Button, FontId::proportional(14.0)),
                (TextStyle::Small, FontId::proportional(12.0)),
                (TextStyle::Monospace, FontId::monospace(12.0)),
                (
                    TextStyle::Heading,
                    FontId::new(20.0, FontFamily::Name(SEMIBOLD.into())),
                ),
            ]
            .into();
            s.spacing.item_spacing = egui::vec2(8.0, 8.0);
            s.spacing.button_padding = egui::vec2(12.0, 6.0);
            s.spacing.interact_size.y = 32.0;
        });
    }
}

/// Lucide glyphs (lucide-static 0.460.0, in `assets/icons/lucide-subset.ttf`).
pub mod icon {
    pub const BOX: &str = "\u{E065}";
    pub const SUN: &str = "\u{E17C}";
    pub const MOON: &str = "\u{E122}";
}

#[cfg(test)]
mod tests {
    /// Invariant: Kraxn reads both palettes from MurmelCraft's tokens
    /// (no colour of its own), and the accent is the logo's magenta in
    /// light. Breaks if the token names change or Kraxn grew a palette.
    #[test]
    fn colours_come_from_the_tokens() {
        let t = super::Tokens::builtin().unwrap();
        assert_eq!(t.light.accent, super::Hex([0xAD, 0x32, 0x85]));
        assert_ne!(t.dark.bg_app, t.light.bg_app);
        assert!(
            !include_str!("window.rs").contains("Color32::from_rgb("),
            "colour literal"
        );
    }
}
