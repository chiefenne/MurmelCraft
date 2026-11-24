//! Overlays of exported pictures and movies (`export.image`,
//! `export.movie`): the colour legend ([`murmel_engine::Legend`]) and the
//! axis triad, drawn on the CPU with vello_cpu (Apache-2.0 OR MIT) over the
//! rendered image, so they need no window. They follow the window's
//! legend and triad (`panels.rs`, `viewport.rs`): the same tokens, the
//! same sizes in points, times `scale` pixels per point, Inter for text
//! (SIL OFL, assets/fonts).

use murmel_engine::i18n::I18n;
use murmel_engine::legend::{Body, Entry, Legend, Ramp};
use murmel_engine::{Camera, Rgb, Tokens};
use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Affine, BezPath, Cap, Line, Rect, RoundedRect, Shape, Stroke};
use vello_cpu::peniko::{Blob, FontData};
use vello_cpu::{
    CompositeMode, Glyph, PixmapMut, RasterizerSettings, RenderContext, RenderMode, Resources,
};

use crate::RenderError;

const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

/// Inset of the legend from the top left corner, points (the window's
/// legend sits under the view note, which exports leave out).
const LEGEND_AT: (f64, f64) = (16.0, 16.0);
/// Text sizes of the window's legend and triad, points.
const TEXT: f32 = 12.0;
const SMALL: f32 = 11.0;
/// Height of the colour bar of a ramp, points.
const BAR: f64 = 160.0;
/// Width of the colour bar, points.
const BAR_W: f64 = 14.0;
/// Opacity of the legend's panel, as in the window.
const PANEL_ALPHA: f32 = 0.92;

/// What to draw over a picture.
pub struct Overlays<'a> {
    pub legend: Option<&'a Legend>,
    /// The camera whose axes the triad shows (bottom left).
    pub triad: Option<&'a Camera>,
    pub i18n: &'a I18n,
    pub tokens: &'a Tokens,
    pub dark: bool,
    /// Pixels per point: 1 draws the window's sizes, 2 twice as large.
    pub scale: f32,
}

impl Overlays<'_> {
    pub fn is_empty(&self) -> bool {
        self.legend.is_none() && self.triad.is_none()
    }
}

/// Draw `o` over `rgba` (`width` × `height` RGBA8 rows, sRGB, straight
/// alpha) in place.
pub fn draw(rgba: &mut [u8], width: u32, height: u32, o: &Overlays<'_>) -> Result<(), RenderError> {
    if o.is_empty() {
        return Ok(());
    }
    let size_err = || {
        RenderError::new(
            "RENDER003",
            format!("{width} x {height} is outside 1..{}", crate::MAX_SIDE),
        )
    };
    let w = u16::try_from(width).map_err(|_| size_err())?;
    let h = u16::try_from(height).map_err(|_| size_err())?;
    if w == 0 || h == 0 || rgba.len() != 4 * width as usize * height as usize {
        return Err(size_err());
    }
    let font = FontRef::new(INTER)
        .map_err(|e| RenderError::new("RENDER005", format!("Inter-Regular.ttf: {e}")))?;
    let mut p = Painter {
        ctx: RenderContext::new(w, h),
        res: Resources::new(),
        data: FontData::new(Blob::from(INTER.to_vec()), 0),
        font,
        scale: o.scale.max(0.05) as f64,
    };
    let palette = o.tokens.palette(o.dark);
    if let Some(l) = o.legend {
        let title = l.title.text(o.i18n);
        match &l.body {
            Body::Categories(e) => p.categories(&title, e, o, palette),
            Body::Ramp(r) => p.ramp(&title, r, palette),
        }
    }
    if let Some(cam) = o.triad {
        p.triad(cam, o.tokens, height as f64 / p.scale);
    }
    premultiply(rgba);
    let target = PixmapMut::new(w, h, rgba)
        .ok_or_else(|| RenderError::new("RENDER002", "overlay buffer"))?;
    p.ctx.render_with(
        target,
        &mut p.res,
        RasterizerSettings {
            render_mode: RenderMode::OptimizeSpeed,
            composite_mode: CompositeMode::SrcOver,
            ..Default::default()
        },
    );
    unpremultiply(rgba);
    Ok(())
}

struct Painter<'f> {
    ctx: RenderContext,
    res: Resources,
    data: FontData,
    font: FontRef<'f>,
    /// Pixels per point.
    scale: f64,
}

fn paint(c: Rgb, alpha: f32) -> AlphaColor<Srgb> {
    AlphaColor::from_rgba8(
        c.0[0],
        c.0[1],
        c.0[2],
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

impl Painter<'_> {
    /// Glyphs of `text` at `pt` points, from x = 0 on the baseline, and the
    /// advance width in points. Simple left-to-right layout (legend text
    /// is short labels and numbers).
    fn layout(&self, text: &str, pt: f32) -> (Vec<Glyph>, f64) {
        let px = pt * self.scale as f32;
        let map = self.font.charmap();
        let metrics = self
            .font
            .glyph_metrics(Size::new(px), LocationRef::default());
        let mut x = 0.0f32;
        let mut glyphs = Vec::with_capacity(text.len());
        for ch in text.chars() {
            let id = map.map(ch).unwrap_or(GlyphId::NOTDEF);
            glyphs.push(Glyph {
                id: id.to_u32(),
                x,
                y: 0.0,
            });
            x += metrics.advance_width(id).unwrap_or(0.0);
        }
        (glyphs, x as f64 / self.scale)
    }

    fn text_width(&self, text: &str, pt: f32) -> f64 {
        self.layout(text, pt).1
    }

    /// Ascent and cap height of the font at `pt` points, in points.
    fn ascent_cap(&self, pt: f32) -> (f64, f64) {
        let m = self.font.metrics(Size::new(pt), LocationRef::default());
        let cap = m.cap_height.unwrap_or(0.7 * pt);
        (m.ascent as f64, cap as f64)
    }

    /// Draw `text` with its baseline at (`x`, `y`) points.
    fn text(&mut self, text: &str, pt: f32, x: f64, y: f64, color: AlphaColor<Srgb>) {
        let (glyphs, _) = self.layout(text, pt);
        let s = self.scale;
        self.ctx.set_transform(Affine::translate((x * s, y * s)));
        self.ctx.set_paint(color);
        self.ctx
            .glyph_run(&mut self.res, &self.data)
            .font_size(pt * s as f32)
            .fill_glyphs(glyphs.into_iter());
        self.ctx.reset_transform();
    }

    /// Text placed with its top at `top` like an egui label of `pt`.
    fn label(&mut self, text: &str, pt: f32, x: f64, top: f64, color: AlphaColor<Srgb>) {
        let (ascent, _) = self.ascent_cap(pt);
        // egui rows are about 1.2 em; Inter's ascent is about 0.97 em.
        let lead = (1.2 * pt as f64 - ascent) * 0.35;
        self.text(text, pt, x, top + lead + ascent, color);
    }

    fn rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: AlphaColor<Srgb>) {
        let s = self.scale;
        self.ctx.set_paint(color);
        self.ctx
            .fill_rect(&Rect::new(x0 * s, y0 * s, x1 * s, y1 * s));
    }

    fn rounded(&mut self, r: Rect, radius: f64, color: AlphaColor<Srgb>) {
        let s = self.scale;
        let path =
            RoundedRect::new(r.x0 * s, r.y0 * s, r.x1 * s, r.y1 * s, radius * s).to_path(0.1);
        self.ctx.set_paint(color);
        self.ctx.fill_path(&path);
    }

    fn line(&mut self, a: (f64, f64), b: (f64, f64), width: f64, color: AlphaColor<Srgb>) {
        let s = self.scale;
        let path: BezPath = Line::new((a.0 * s, a.1 * s), (b.0 * s, b.1 * s)).to_path(0.1);
        self.ctx
            .set_stroke(Stroke::new(width * s).with_caps(Cap::Round));
        self.ctx.set_paint(color);
        self.ctx.stroke_path(&path);
    }

    /// The window's legend of categories: a title, then one row per
    /// entry with a 10-point swatch, 18 points apart.
    fn categories(
        &mut self,
        title: &str,
        entries: &[Entry],
        o: &Overlays<'_>,
        palette: &murmel_engine::theme::Palette,
    ) {
        let names: Vec<String> = entries.iter().map(|e| e.label.text(o.i18n)).collect();
        let line = 18.0;
        let widest = names
            .iter()
            .map(|n| self.text_width(n, TEXT) + 18.0)
            .fold(self.text_width(title, TEXT), f64::max);
        let (x, y) = LEGEND_AT;
        let rows = names.len() + 1;
        self.rounded(
            Rect::new(x, y, x + widest + 20.0, y + line * rows as f64 + 10.0),
            6.0,
            paint(palette.bg_panel, PANEL_ALPHA),
        );
        let mut top = y + 5.0;
        self.label(
            title,
            TEXT,
            x + 10.0,
            top,
            paint(palette.text_secondary, 1.0),
        );
        for (e, name) in entries.iter().zip(&names) {
            top += line;
            self.rounded(
                Rect::new(x + 10.0, top + 3.0, x + 20.0, top + 13.0),
                2.0,
                paint(e.color, 1.0),
            );
            self.label(name, TEXT, x + 28.0, top, paint(palette.text_primary, 1.0));
        }
    }

    /// A vertical colour bar, high values on top, with the ticks to its
    /// right; flat bands when the ramp is banded, marks for contour
    /// levels and the quality threshold.
    fn ramp(&mut self, title: &str, r: &Ramp, palette: &murmel_engine::theme::Palette) {
        let title = match &r.unit {
            Some(u) if !u.is_empty() => format!("{title} ({u})"),
            _ => title.to_string(),
        };
        let labels: Vec<(f64, String)> = r.ticks.iter().map(|&t| (t, r.tick_label(t))).collect();
        let label_w = labels
            .iter()
            .map(|(_, l)| self.text_width(l, SMALL))
            .fold(0.0, f64::max);
        let pad = 10.0;
        let width = (BAR_W + 8.0 + label_w).max(self.text_width(&title, TEXT)) + 2.0 * pad;
        let (x, y) = LEGEND_AT;
        let bar_top = y + pad + 22.0;
        let height = 22.0 + BAR + 2.0 * pad + 6.0;
        self.rounded(
            Rect::new(x, y, x + width, y + height),
            6.0,
            paint(palette.bg_panel, PANEL_ALPHA),
        );
        self.label(
            &title,
            TEXT,
            x + pad,
            y + pad - 3.0,
            paint(palette.text_secondary, 1.0),
        );
        let (bx0, bx1) = (x + pad, x + pad + BAR_W);
        let at = |v: f64| bar_top + BAR * (1.0 - r.position(v).clamp(0.0, 1.0));
        // The bar: flat bands, or one slice per pixel row.
        match r.bands.filter(|&n| n > 0) {
            Some(n) => {
                for k in 0..n {
                    let (t0, t1) = (k as f64 / n as f64, (k + 1) as f64 / n as f64);
                    let c = r.sample((t0 + t1) / 2.0);
                    self.rect(
                        bx0,
                        bar_top + BAR * (1.0 - t1),
                        bx1,
                        bar_top + BAR * (1.0 - t0),
                        paint(c, 1.0),
                    );
                }
            }
            None => {
                let rows = (BAR * self.scale).ceil().max(1.0) as usize;
                for k in 0..rows {
                    let t = (k as f64 + 0.5) / rows as f64;
                    let (y0, y1) = (
                        bar_top + BAR * (1.0 - (k + 1) as f64 / rows as f64),
                        bar_top + BAR * (1.0 - k as f64 / rows as f64),
                    );
                    self.rect(bx0, y0, bx1, y1, paint(r.sample(t), 1.0));
                }
            }
        }
        let ink = paint(palette.text_primary, 1.0);
        for &c in &r.contours {
            if r.position(c).is_finite() && (0.0..=1.0).contains(&r.position(c)) {
                let yy = at(c);
                self.line((bx0, yy), (bx1, yy), 1.0, paint(palette.text_primary, 0.8));
            }
        }
        if let Some(t) = r.threshold {
            let yy = at(t);
            self.line(
                (bx0 - 3.0, yy),
                (bx1 + 3.0, yy),
                2.0,
                paint(palette.accent, 1.0),
            );
        }
        let (_, cap) = self.ascent_cap(SMALL);
        for (v, l) in &labels {
            let yy = at(*v);
            self.line((bx1, yy), (bx1 + 4.0, yy), 1.0, ink);
            self.text(l, SMALL, bx1 + 8.0, yy + cap / 2.0, ink);
        }
    }

    /// The window's axis triad: 28 points long from 48 points inside the
    /// bottom left corner, X red, Y green, Z blue, the axis pointing away
    /// drawn first.
    fn triad(&mut self, cam: &Camera, tokens: &Tokens, height_pt: f64) {
        let t = &tokens.mesh;
        let origin = (48.0, height_pt - 48.0);
        let (right, up) = cam.basis();
        let to_eye = cam.to_eye();
        let len = 28.0;
        let mut axes = [
            ([1.0, 0.0, 0.0], t.axis_x, "X"),
            ([0.0, 1.0, 0.0], t.axis_y, "Y"),
            ([0.0, 0.0, 1.0], t.axis_z, "Z"),
        ];
        let dot = |a: [f64; 3], b: glam::DVec3| a[0] * b.x + a[1] * b.y + a[2] * b.z;
        axes.sort_by(|a, b| dot(a.0, to_eye).total_cmp(&dot(b.0, to_eye)));
        let (_, cap) = self.ascent_cap(SMALL);
        for (dir, c, label) in axes {
            let (sx, sy) = (dot(dir, right), dot(dir, up));
            let end = (origin.0 + sx * len, origin.1 - sy * len);
            self.line(origin, end, 2.0, paint(c, 1.0));
            let at = (
                origin.0 + (end.0 - origin.0) * 1.3,
                origin.1 + (end.1 - origin.1) * 1.3,
            );
            let w = self.text_width(label, SMALL);
            self.text(
                label,
                SMALL,
                at.0 - w / 2.0,
                at.1 + cap / 2.0,
                paint(c, 1.0),
            );
        }
    }
}

/// Straight to premultiplied alpha, in sRGB (vello_cpu's pixel format).
fn premultiply(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a == 255 {
            continue;
        }
        for c in &mut px[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
}

/// Premultiplied to straight alpha, in sRGB.
fn unpremultiply(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a == 255 || a == 0 {
            continue;
        }
        for c in &mut px[..3] {
            *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Invariant: premultiplying and back keeps opaque pixels exactly and
    /// translucent ones within rounding; a fully transparent pixel stays
    /// transparent. A swapped division shows here before any picture.
    #[test]
    fn alpha_round_trip() {
        let mut px = vec![200, 100, 50, 255, 200, 100, 50, 128, 0, 0, 0, 0];
        let keep = px.clone();
        premultiply(&mut px);
        assert_eq!(&px[..4], &keep[..4]);
        unpremultiply(&mut px);
        for (a, b) in px.iter().zip(&keep) {
            assert!(a.abs_diff(*b) <= 1, "{px:?}");
        }
    }
}
