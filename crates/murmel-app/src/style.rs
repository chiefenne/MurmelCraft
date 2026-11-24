//! egui style from the design tokens (UI spec, "Design tokens, fonts and
//! icons"): Inter and JetBrains Mono (SIL OFL), Lucide icons (ISC), the
//! dark and light palettes, a 4 px grid and 6 px radius. Inter's Greek
//! letters, sub- and superscripts and a few maths signs come from a
//! subset of their own (`Inter-Greek.otf`, for the formulas and symbols
//! of derived fields), after the Latin subset, so Latin text draws as
//! before.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
    Visuals,
};
use murmel_engine::theme::Palette;
use murmel_engine::{Rgb, Tokens};

pub const SEMIBOLD: &str = "semibold";
pub const ICONS: &str = "icons";

pub fn color(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0[0], c.0[1], c.0[2])
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
        "inter-greek",
        include_bytes!("../../../assets/fonts/Inter-Greek.otf"),
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
    prop.insert(1, "inter-greek".into());
    prop.insert(2, "lucide".into());
    let mono = f.families.entry(FontFamily::Monospace).or_default();
    mono.insert(0, "jetbrains-mono".into());
    f.families.insert(
        FontFamily::Name(SEMIBOLD.into()),
        vec![
            "inter-semibold".into(),
            "inter".into(),
            "inter-greek".into(),
            "lucide".into(),
        ],
    );
    f.families.insert(
        FontFamily::Name(ICONS.into()),
        vec!["lucide".into(), "inter".into()],
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
    v.panel_fill = color(p.bg_panel);
    v.window_fill = color(p.bg_raised);
    v.extreme_bg_color = color(p.bg_app);
    v.faint_bg_color = color(p.bg_raised);
    v.window_stroke = Stroke::new(1.0, color(p.line));
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(8);
    v.hyperlink_color = color(p.accent);
    v.selection.bg_fill = color(p.accent).gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, color(p.accent));
    v.error_fg_color = color(p.status_error);
    v.warn_fg_color = color(p.status_warn);
    v.override_text_color = Some(color(p.text_primary));
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = color(p.bg_panel);
    w.noninteractive.weak_bg_fill = color(p.bg_panel);
    w.noninteractive.bg_stroke = Stroke::new(1.0, color(p.line));
    w.noninteractive.fg_stroke = Stroke::new(1.0, color(p.text_secondary));
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = color(p.bg_raised);
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.5, color(p.text_secondary));
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = color(p.bg_raised);
    w.hovered.weak_bg_fill = color(p.bg_raised);
    w.hovered.bg_stroke = Stroke::new(1.0, color(p.line));
    w.hovered.fg_stroke = Stroke::new(1.5, color(p.text_primary));
    w.hovered.corner_radius = r;
    w.active.bg_fill = color(p.accent).gamma_multiply(0.25);
    w.active.weak_bg_fill = color(p.accent).gamma_multiply(0.25);
    w.active.bg_stroke = Stroke::new(1.0, color(p.accent));
    w.active.fg_stroke = Stroke::new(1.5, color(p.accent));
    w.active.corner_radius = r;
    w.open = w.active;
    v
}

/// Install fonts and both themes' styles.
pub fn install(ctx: &egui::Context, tokens: &Tokens) {
    ctx.set_fonts(fonts());
    for (theme, dark) in [(egui::Theme::Dark, true), (egui::Theme::Light, false)] {
        ctx.style_mut_of(theme, |s| {
            s.visuals = visuals(tokens.palette(dark), dark);
            s.text_styles = [
                (TextStyle::Body, FontId::proportional(13.0)),
                (TextStyle::Button, FontId::proportional(13.0)),
                (TextStyle::Small, FontId::proportional(12.0)),
                (TextStyle::Monospace, FontId::monospace(12.0)),
                (
                    TextStyle::Heading,
                    FontId::new(15.0, FontFamily::Name(SEMIBOLD.into())),
                ),
            ]
            .into();
            s.spacing.item_spacing = egui::vec2(8.0, 4.0);
            s.spacing.button_padding = egui::vec2(8.0, 4.0);
            s.spacing.interact_size.y = 28.0;
            s.spacing.menu_margin = egui::Margin::same(8);
        });
    }
}

/// Lucide glyphs (codepoints of lucide-static 0.460.0, kept in the subset
/// font `assets/icons/lucide-subset.ttf`).
pub mod icon {
    pub const FOLDER_OPEN: &str = "\u{E247}";
    pub const SCAN: &str = "\u{E257}";
    pub const HOUSE: &str = "\u{E0F8}";
    pub const BOX: &str = "\u{E065}";
    pub const SQUARE: &str = "\u{E16B}";
    pub const SUN: &str = "\u{E17C}";
    pub const MOON: &str = "\u{E122}";
    pub const MONITOR: &str = "\u{E121}";
    pub const SETTINGS: &str = "\u{E158}";
    pub const ALERT: &str = "\u{E07B}";
    pub const EYE: &str = "\u{E0BE}";
    pub const EYE_OFF: &str = "\u{E0BF}";
    pub const INFO: &str = "\u{E0FE}";
    pub const LAYERS: &str = "\u{E104}";
    pub const SEARCH: &str = "\u{E155}";
}

/// The MurmelCraft logo (assets/logo, supplied by the maintainer) as RGBA.
pub fn logo_rgba(size: u32) -> Option<(u32, u32, Vec<u8>)> {
    let bytes: &[u8] = if size <= 128 {
        include_bytes!("../../../assets/logo/murmelcraft-128.png")
    } else {
        include_bytes!("../../../assets/logo/murmelcraft-256.png")
    };
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    (info.color_type == png::ColorType::Rgba).then_some((info.width, info.height, buf))
}

#[cfg(test)]
mod tests {
    /// Invariant: both logo sizes decode to square RGBA with transparent
    /// corners (the rounded square is cut out of the white page).
    #[test]
    fn logo_decodes_with_transparent_corners() {
        for size in [128, 256] {
            let (w, h, rgba) = super::logo_rgba(size).unwrap();
            assert_eq!((w, h), (size, size));
            assert_eq!(rgba[3], 0, "top-left corner must be transparent");
            let mid = ((h / 2 * w + w / 2) * 4 + 3) as usize;
            assert_eq!(rgba[mid], 255);
        }
    }
}
