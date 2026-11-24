//! Export of pictures (`export.image`): the view rendered offscreen at a
//! chosen size or scale, on a transparent background if asked, with the
//! colour legend and the axis triad drawn over it, written as PNG.
//!
//! Decided by the maintainer on cards (2026-11-24): the legend and the
//! axes are each optional, the background can be transparent, and lines,
//! legend and text grow with the scale, so a 2x picture is a sharp copy
//! of the window ("same look as on screen").

use std::path::PathBuf;

use murmel_engine::i18n::I18n;
use murmel_engine::{Legend, Session, Theme};
use murmel_render::overlay::{self, Overlays};
use murmel_render::{MAX_SIDE, Shoot};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{camera_json, need_mesh, object, parse};
use crate::{Diagnostic, Run};

/// Smallest and largest export scale (pixels per point of the window).
pub const SCALE: (f64, f64) = (0.25, 8.0);

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportImage {
    path: PathBuf,
    width: Option<u32>,
    height: Option<u32>,
    scale: Option<f64>,
    #[serde(default)]
    transparent: bool,
    #[serde(default = "yes")]
    legend: bool,
    #[serde(default = "yes")]
    axes: bool,
    dpi: Option<f64>,
    theme: Option<Theme>,
    line_width: Option<f64>,
}

/// The size of a picture and its scale (pixels per point): the viewport
/// times `scale` (1 by default); a width or height given sets the size
/// (the other side keeps the viewport's aspect when missing) and, without
/// a scale, the scale follows the height (the camera frames the view by
/// its height, so the legend keeps its size next to the mesh).
pub fn picture_size(
    viewport: [f64; 2],
    width: Option<u32>,
    height: Option<u32>,
    scale: Option<f64>,
) -> Result<(u32, u32, f64), Diagnostic> {
    let [vw, vh] = [viewport[0].max(1.0), viewport[1].max(1.0)];
    if let Some(sc) = scale
        && !(SCALE.0..=SCALE.1).contains(&sc)
    {
        return Err(Diagnostic::new(
            "EXPORT001",
            format!("scale {sc} is outside {} to {}", SCALE.0, SCALE.1),
        ));
    }
    let round = |x: f64| x.round().max(1.0) as u32;
    let (w, h) = match (width, height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, round(w as f64 * vh / vw)),
        (None, Some(h)) => (round(h as f64 * vw / vh), h),
        (None, None) => {
            let sc = scale.unwrap_or(1.0);
            (round(vw * sc), round(vh * sc))
        }
    };
    if !(1..=MAX_SIDE).contains(&w) || !(1..=MAX_SIDE).contains(&h) {
        return Err(Diagnostic::new(
            "RENDER003",
            format!("{w} x {h} is outside 1..{MAX_SIDE}"),
        ));
    }
    let sc = scale.unwrap_or(h as f64 / vh).clamp(SCALE.0, SCALE.1);
    Ok((w, h, sc))
}

/// The width of edges in pixels: as given, else the scale (lines never
/// get thinner than one pixel).
pub fn line_width(given: Option<f64>, scale: f64) -> Result<f64, Diagnostic> {
    let w = given.unwrap_or(scale.max(1.0));
    if (1.0..=32.0).contains(&w) {
        Ok(w)
    } else {
        Err(Diagnostic::new(
            "EXPORT001",
            format!("line width {w} is outside 1 to 32 pixels"),
        ))
    }
}

/// What an exported picture shows over the mesh, and how.
pub struct Look {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub transparent: bool,
    pub legend: bool,
    pub axes: bool,
    pub line_width: f64,
}

impl Look {
    pub fn shoot(&self) -> Shoot {
        Shoot {
            width: self.width,
            height: self.height,
            transparent: self.transparent,
            line_width: self.line_width as f32,
        }
    }
}

/// Draw the legend and triad `look` asks for over `rgba`.
pub fn draw_overlays(
    s: &Session,
    look: &Look,
    i18n: &I18n,
    rgba: &mut [u8],
) -> Result<(), Diagnostic> {
    let legend = look.legend.then(|| Legend::of(s)).flatten();
    let o = Overlays {
        legend: legend.as_ref(),
        triad: look.axes.then_some(&s.camera),
        i18n,
        tokens: &s.tokens,
        dark: s.theme.is_dark(None),
        scale: look.scale as f32,
    };
    overlay::draw(rgba, look.width, look.height, &o).map_err(|e| Diagnostic::new(e.code, e.message))
}

pub fn i18n() -> Result<I18n, Diagnostic> {
    I18n::english().map_err(|e| Diagnostic::new("RENDER005", e))
}

pub fn export_image(input: Value, s: &mut Session) -> Run {
    let i: ExportImage = parse(input)?;
    need_mesh(s)?;
    let (width, height, scale) = picture_size(s.viewport, i.width, i.height, i.scale)?;
    let line_width = line_width(i.line_width, scale)?;
    if let Some(d) = i.dpi
        && !(1.0..=10_000.0).contains(&d)
    {
        return Err(Diagnostic::new(
            "EXPORT001",
            format!("dpi {d} is outside 1 to 10000"),
        ));
    }
    let look = Look {
        width,
        height,
        scale,
        transparent: i.transparent,
        legend: i.legend,
        axes: i.axes,
        line_width,
    };
    let i18n = i18n()?;
    let keep = s.theme;
    if let Some(t) = i.theme {
        s.theme = t;
    }
    let shot = murmel_render::render_session_with(s, &look.shoot())
        .map_err(|e| Diagnostic::new(e.code, e.message))
        .and_then(|mut shot| {
            draw_overlays(s, &look, &i18n, &mut shot.rgba)?;
            Ok(shot)
        });
    s.theme = keep;
    let shot = shot?;
    murmel_render::write_png_with(&i.path, shot.width, shot.height, &shot.rgba, i.dpi)
        .map_err(|e| Diagnostic::new(e.code, e.message))?;
    Ok(json!({
        "files": [i.path],
        "width": shot.width,
        "height": shot.height,
        "scale": scale,
        "line_width": line_width,
        "transparent": i.transparent,
        "legend": i.legend,
        "axes": i.axes,
        "dpi": i.dpi,
        "adapter": shot.adapter,
        "camera": camera_json(s),
    }))
}

pub fn export_image_schema() -> Value {
    object(
        json!({
            "path": {"type": "string", "description": "The PNG file to write"},
            "width": {"type": "integer", "minimum": 1, "maximum": MAX_SIDE, "description": "Pixels; without it the viewport's width times the scale (or the height's aspect)"},
            "height": {"type": "integer", "minimum": 1, "maximum": MAX_SIDE, "description": "Pixels; without it the viewport's height times the scale (or the width's aspect)"},
            "scale": {"type": "number", "minimum": SCALE.0, "maximum": SCALE.1, "description": "Pixels per point of the window: the size without width and height, and the size of lines, legend and text (default: 1, or the height over the viewport's height)"},
            "transparent": {"type": "boolean", "default": false, "description": "Leave the background out (alpha 0)"},
            "legend": {"type": "boolean", "default": true, "description": "Draw the colour legend (top left)"},
            "axes": {"type": "boolean", "default": true, "description": "Draw the axis triad (bottom left)"},
            "dpi": {"type": "number", "minimum": 1, "maximum": 10000, "description": "Resolution stored in the PNG (pHYs), for print"},
            "theme": {"enum": ["dark", "light", "system"]},
            "line_width": {"type": "number", "minimum": 1, "maximum": 32, "description": "Edge width in pixels (default: the scale)"}
        }),
        &["path"],
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Invariant: without a size the picture is the viewport times the
    /// scale; one side given keeps the viewport's aspect; the scale of
    /// lines and legend follows the height unless given; scales and sizes
    /// out of range are refused with their codes. A change of these rules
    /// changes every exported picture's size or look.
    #[test]
    fn sizes_and_scales() {
        let vp = [1280.0, 720.0];
        assert_eq!(
            picture_size(vp, None, None, None).unwrap(),
            (1280, 720, 1.0)
        );
        assert_eq!(
            picture_size(vp, None, None, Some(2.0)).unwrap(),
            (2560, 1440, 2.0)
        );
        assert_eq!(
            picture_size(vp, Some(3840), None, None).unwrap(),
            (3840, 2160, 3.0)
        );
        assert_eq!(
            picture_size(vp, None, Some(360), None).unwrap(),
            (640, 360, 0.5)
        );
        assert_eq!(
            picture_size(vp, Some(1920), Some(1080), Some(1.0)).unwrap(),
            (1920, 1080, 1.0)
        );
        assert_eq!(
            picture_size(vp, None, None, Some(9.0)).unwrap_err().code,
            "EXPORT001"
        );
        assert_eq!(
            picture_size(vp, None, None, Some(8.0)).unwrap_err().code,
            "RENDER003"
        );
        assert_eq!(
            picture_size(vp, Some(0), Some(10), None).unwrap_err().code,
            "RENDER003"
        );
    }
}
