//! A page as PDF (`export.pdf`), for reports and for File ▸ Print: the
//! view rendered to fill the page inside its margins, with the legend and
//! axes of export.image, and above it a title and a line naming the file
//! and the date. The page comes from `[page]` in settings.toml (File ▸
//! Page setup), each value overridable per call; the maintainer's card of
//! 2026-11-25 chose "PDF page + Print + Page setup".
//!
//! The writer is MurmelCraft's own and small: one page, the picture as a
//! deflated RGB image, the text in Helvetica (one of the fonts every PDF
//! reader has, so nothing is embedded) with the Windows Latin-1 encoding.

use std::path::{Path, PathBuf};

use murmel_engine::settings::{DPI, MAX_MARGIN_MM, Orientation, PageSetup, Paper};
use murmel_engine::{Session, Theme};
use murmel_render::MAX_SIDE;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::export::{Look, draw_overlays, i18n};
use crate::tools::{need_mesh, object, parse};
use crate::{Diagnostic, Run};

/// Room above the picture for the header, in points.
const HEADER: f64 = 34.0;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportPdf {
    path: PathBuf,
    paper: Option<Paper>,
    orientation: Option<Orientation>,
    margin_mm: Option<u32>,
    header: Option<bool>,
    dpi: Option<u32>,
    theme: Option<Theme>,
    title: Option<String>,
    #[serde(default = "yes")]
    legend: bool,
    #[serde(default = "yes")]
    axes: bool,
}

fn yes() -> bool {
    true
}

/// The page setup a call asks for: `[page]` with the call's values.
fn setup(i: &ExportPdf, s: &Session) -> Result<PageSetup, Diagnostic> {
    let mut p = s.page;
    p.paper = i.paper.unwrap_or(p.paper);
    p.orientation = i.orientation.unwrap_or(p.orientation);
    p.margin_mm = i.margin_mm.unwrap_or(p.margin_mm);
    p.header = i.header.unwrap_or(p.header);
    p.dpi = i.dpi.unwrap_or(p.dpi);
    p.theme = i.theme.unwrap_or(p.theme);
    check(&p)?;
    Ok(p)
}

/// Refuse a margin or resolution out of range.
pub fn check(p: &PageSetup) -> Result<(), Diagnostic> {
    if p.margin_mm > MAX_MARGIN_MM {
        return Err(Diagnostic::new(
            "EXPORT001",
            format!("a margin of {} mm is over {MAX_MARGIN_MM} mm", p.margin_mm),
        ));
    }
    if !(DPI.0..=DPI.1).contains(&p.dpi) {
        return Err(Diagnostic::new(
            "EXPORT001",
            format!("{} dpi is outside {} to {}", p.dpi, DPI.0, DPI.1),
        ));
    }
    Ok(())
}

/// Where the picture goes on the page (points, from the bottom left)
/// and its size in pixels at the resolution (less when a side would pass
/// the renderer's limit).
pub fn layout(p: &PageSetup) -> ([f64; 4], u32, u32, f64) {
    let (w, h) = p.size();
    let m = p.margin_mm as f64 * 72.0 / 25.4;
    let top = if p.header { HEADER } else { 0.0 };
    let (bw, bh) = ((w - 2.0 * m).max(36.0), (h - 2.0 * m - top).max(36.0));
    let mut dpi = p.dpi as f64;
    let px = |v: f64, dpi: f64| (v / 72.0 * dpi).round() as u32;
    let biggest = px(bw, dpi).max(px(bh, dpi));
    if biggest > MAX_SIDE {
        dpi *= MAX_SIDE as f64 / biggest as f64;
    }
    ([m, m, bw, bh], px(bw, dpi).max(1), px(bh, dpi).max(1), dpi)
}

pub fn export_pdf(input: Value, s: &mut Session) -> Run {
    let i: ExportPdf = parse(input)?;
    need_mesh(s)?;
    crate::series::ensure_steps_range(s)?;
    let p = setup(&i, s)?;
    let (rect, width, height, dpi) = layout(&p);
    let look = Look {
        width,
        height,
        // A point of the window is a point on paper: the legend's text
        // prints at the size it has on screen.
        scale: dpi / 72.0,
        transparent: false,
        legend: i.legend,
        axes: i.axes,
        time: s.series.is_some(),
        line_width: (dpi / 72.0).max(1.0),
    };
    let i18n = i18n()?;
    let keep = s.theme;
    s.theme = p.theme;
    let shot = murmel_render::render_session_with(s, &look.shoot())
        .map_err(|e| Diagnostic::new(e.code, e.message))
        .and_then(|mut shot| {
            draw_overlays(s, &look, &i18n, &mut shot.rgba)?;
            Ok(shot)
        });
    s.theme = keep;
    let shot = shot?;
    let mesh = s.mesh.as_ref().map(|m| m.path.clone());
    let title = i.title.clone().unwrap_or_else(|| {
        mesh.as_deref()
            .and_then(Path::file_name)
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let line = format!(
        "MurmelCraft {} · {} · {}",
        env!("CARGO_PKG_VERSION"),
        today(),
        mesh.as_deref()
            .map(|m| m.display().to_string())
            .unwrap_or_default()
    );
    let (pw, ph) = p.size();
    let page = Page {
        size: [pw, ph],
        image: Image {
            width: shot.width,
            height: shot.height,
            rgb: rgb(&shot.rgba),
            at: rect,
        },
        title: p.header.then_some(title.as_str()),
        line: p.header.then_some(line.as_str()),
    };
    let bytes = write(&page);
    std::fs::write(&i.path, bytes)
        .map_err(|e| Diagnostic::new("EXPORT008", format!("{}: {e}", i.path.display())))?;
    Ok(json!({
        "files": [i.path],
        "page": p,
        "page_points": [pw, ph],
        "pixels": [shot.width, shot.height],
        "dpi": dpi,
        "adapter": shot.adapter,
    }))
}

pub fn export_pdf_schema() -> Value {
    object(
        json!({
            "path": {"type": "string", "description": "The PDF file to write"},
            "paper": {"enum": ["a4", "a3", "letter"], "description": "Left out: [page] in settings.toml (a4)"},
            "orientation": {"enum": ["landscape", "portrait"], "description": "Left out: [page] (landscape)"},
            "margin_mm": {"type": "integer", "minimum": 0, "maximum": MAX_MARGIN_MM, "description": "Margin on every side; left out: [page] (12)"},
            "header": {"type": "boolean", "description": "Title and a line naming the file and the date above the view; left out: [page] (true)"},
            "dpi": {"type": "integer", "minimum": DPI.0, "maximum": DPI.1, "description": "The picture's resolution; left out: [page] (200)"},
            "theme": {"enum": ["dark", "light", "system"], "description": "Left out: [page] (light, as dark prints badly)"},
            "title": {"type": "string", "description": "The header's title; left out: the mesh's file name"},
            "legend": {"type": "boolean", "default": true},
            "axes": {"type": "boolean", "default": true}
        }),
        &["path"],
    )
}

/// `settings.set_page`'s input.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetPage {
    paper: Option<Paper>,
    orientation: Option<Orientation>,
    margin_mm: Option<u32>,
    header: Option<bool>,
    dpi: Option<u32>,
    theme: Option<Theme>,
    #[serde(default)]
    reset: bool,
}

pub fn settings_set_page(input: Value, s: &mut Session) -> Run {
    let i: SetPage = parse(input)?;
    let mut p = if i.reset {
        PageSetup::default()
    } else {
        s.page
    };
    p.paper = i.paper.unwrap_or(p.paper);
    p.orientation = i.orientation.unwrap_or(p.orientation);
    p.margin_mm = i.margin_mm.unwrap_or(p.margin_mm);
    p.header = i.header.unwrap_or(p.header);
    p.dpi = i.dpi.unwrap_or(p.dpi);
    p.theme = i.theme.unwrap_or(p.theme);
    check(&p)?;
    // Applied live first; see settings.set_bindings.
    s.page = p;
    let path = crate::tools::save_session_settings(s)?;
    Ok(json!({"page": s.page, "files": [path]}))
}

pub fn settings_set_page_schema() -> Value {
    object(
        json!({
            "paper": {"enum": ["a4", "a3", "letter"]},
            "orientation": {"enum": ["landscape", "portrait"]},
            "margin_mm": {"type": "integer", "minimum": 0, "maximum": MAX_MARGIN_MM},
            "header": {"type": "boolean"},
            "dpi": {"type": "integer", "minimum": DPI.0, "maximum": DPI.1},
            "theme": {"enum": ["dark", "light", "system"]},
            "reset": {"type": "boolean", "description": "Start from the built-in page (A4 landscape, 12 mm, header, 200 dpi, light)"}
        }),
        &[],
    )
}

/// Today's date (UTC) as YYYY-MM-DD.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = civil(secs / 86_400);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The calendar date of a day counted from 1970-01-01 (proleptic
/// Gregorian, by eras of 400 years).
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}

/// RGBA without its alpha.
fn rgb(rgba: &[u8]) -> Vec<u8> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

/// A picture on the page.
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Rows from the top, three bytes a pixel.
    pub rgb: Vec<u8>,
    /// Left, bottom, width, height in points.
    pub at: [f64; 4],
}

/// One page.
pub struct Page<'a> {
    /// Width and height in points.
    pub size: [f64; 2],
    pub image: Image,
    pub title: Option<&'a str>,
    pub line: Option<&'a str>,
}

/// Text as a PDF string in the Windows Latin-1 encoding: `(`, `)` and `\`
/// escaped, a character the encoding lacks as `?`.
pub fn pdf_string(text: &str) -> Vec<u8> {
    let mut out = vec![b'('];
    for c in text.chars() {
        let b = match c {
            '(' | ')' | '\\' => {
                out.push(b'\\');
                c as u8
            }
            ' '..='~' => c as u8,
            '€' => 0x80,
            '…' => 0x85,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '\u{a0}'..='\u{ff}' => c as u32 as u8,
            _ => b'?',
        };
        out.push(b);
    }
    out.push(b')');
    out
}

/// Text for the document's information (title, producer): UTF-16 with
/// its byte order mark, written in hex, so any character survives.
pub fn text_string(text: &str) -> Vec<u8> {
    let mut out = String::from("<FEFF");
    for u in text.encode_utf16() {
        out.push_str(&format!("{u:04X}"));
    }
    out.push('>');
    out.into_bytes()
}

fn deflate(bytes: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(bytes, 6)
}

/// The page as a PDF file.
pub fn write(page: &Page<'_>) -> Vec<u8> {
    let [pw, ph] = page.size;
    let im = &page.image;
    let [x, y, w, h] = im.at;
    let mut content = format!("q {w:.2} 0 0 {h:.2} {x:.2} {y:.2} cm /Im1 Do Q\n").into_bytes();
    let top = ph - y.min(ph);
    let mut text = |size: f64, base: f64, s: &str| {
        content.extend(format!("BT /F1 {size} Tf {x:.2} {base:.2} Td ").as_bytes());
        content.extend(pdf_string(s));
        content.extend(b" Tj ET\n");
    };
    if let Some(t) = page.title {
        text(14.0, top - 16.0, t);
    }
    if let Some(l) = page.line {
        text(8.0, top - 28.0, l);
    }
    let content = deflate(&content);
    let pixels = deflate(&im.rgb);
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] \
             /Resources << /Font << /F1 5 0 R >> /XObject << /Im1 6 0 R >> >> \
             /Contents 4 0 R >>"
        )
        .into_bytes(),
        stream(
            &format!("<< /Length {} /Filter /FlateDecode >>", content.len()),
            &content,
        ),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
        stream(
            &format!(
                "<< /Type /XObject /Subtype /Image /Width {} /Height {} \
                 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>",
                im.width,
                im.height,
                pixels.len()
            ),
            &pixels,
        ),
    ];
    let mut info = b"<< /Producer ".to_vec();
    info.extend(text_string(&format!(
        "MurmelCraft {}",
        env!("CARGO_PKG_VERSION")
    )));
    if let Some(t) = page.title {
        info.extend(b" /Title ");
        info.extend(text_string(t));
    }
    info.extend(b" >>");
    objects.push(info);
    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut at = Vec::new();
    for (k, o) in objects.iter().enumerate() {
        at.push(out.len());
        out.extend(format!("{} 0 obj\n", k + 1).as_bytes());
        out.extend(o);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for a in at {
        out.extend(format!("{a:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info {} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1,
            objects.len()
        )
        .as_bytes(),
    );
    out
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut v = dict.as_bytes().to_vec();
    v.extend(b"\nstream\n");
    v.extend(data);
    v.extend(b"\nendstream");
    v
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// The offsets of the objects the cross-reference table gives.
    pub(crate) fn xref_offsets(pdf: &[u8]) -> Vec<usize> {
        let find = |hay: &[u8], what: &[u8]| hay.windows(what.len()).rposition(|w| w == what);
        let at = find(pdf, b"startxref\n").unwrap() + 10;
        let tail = std::str::from_utf8(&pdf[at..]).unwrap();
        let start: usize = tail.lines().next().unwrap().parse().unwrap();
        let table = std::str::from_utf8(&pdf[start..at]).unwrap();
        table
            .lines()
            .skip(3)
            .take_while(|l| l.ends_with(" n "))
            .map(|l| l[..10].parse().unwrap())
            .collect()
    }

    /// Invariant: the file is a PDF whose cross-reference table points at
    /// each object, with the page's size, the image's pixels deflated to
    /// what they were, and the text in the reader's encoding. Breaks when
    /// an offset is off by a byte (readers then repair or refuse the
    /// file) or the picture is cut short.
    #[test]
    fn a_page_reads_back() {
        let rgb: Vec<u8> = (0..2 * 3 * 3).map(|k| k as u8 * 10).collect();
        let page = Page {
            size: [842.0, 595.0],
            image: Image {
                width: 2,
                height: 3,
                rgb: rgb.clone(),
                at: [34.0, 34.0, 774.0, 493.0],
            },
            title: Some("channel (a) – 2 µm"),
            line: Some("MurmelCraft"),
        };
        let pdf = write(&page);
        assert!(pdf.starts_with(b"%PDF-1.4\n"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let offsets = xref_offsets(&pdf);
        assert_eq!(offsets.len(), 7);
        for (k, at) in offsets.iter().enumerate() {
            let head = format!("{} 0 obj\n", k + 1);
            assert!(pdf[*at..].starts_with(head.as_bytes()), "object {}", k + 1);
        }
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/MediaBox [0 0 842.00 595.00]"));
        assert!(text.contains("/Width 2 /Height 3"));
        // The image's stream inflates to its pixels.
        let img = offsets[5];
        let body = &pdf[img..];
        let s = body.windows(7).position(|w| w == b"stream\n").unwrap() + 7;
        let e = body.windows(10).position(|w| w == b"\nendstream").unwrap();
        let back = miniz_oxide::inflate::decompress_to_vec_zlib(&body[s..e]).unwrap();
        assert_eq!(back, rgb);
        assert_eq!(
            pdf_string("a(b)\\ – µ °"),
            b"(a\\(b\\)\\\\ \x96 \xb5 \xb0)".to_vec()
        );
        assert_eq!(pdf_string("✓"), b"(?)".to_vec());
        assert_eq!(text_string("a–"), b"<FEFF00612013>".to_vec());
    }

    /// Invariant: the picture fills the page inside the margins and under
    /// the header, at the resolution asked, and a page that would need
    /// more pixels than the renderer draws gets a lower resolution. Breaks
    /// when the margins or the header overlap the view.
    #[test]
    fn the_view_fills_the_page() {
        let p = PageSetup::default();
        let ([x, y, w, h], pw, ph, dpi) = layout(&p);
        let (sw, sh) = p.size();
        assert!(
            (sw - 841.89).abs() < 0.01 && (sh - 595.28).abs() < 0.01,
            "A4 landscape"
        );
        let m = 12.0 * 72.0 / 25.4;
        assert!((x - m).abs() < 1e-9 && (y - m).abs() < 1e-9);
        assert!((x + w - (sw - m)).abs() < 1e-9);
        assert!((y + h - (sh - m - HEADER)).abs() < 1e-9);
        assert_eq!(dpi, 200.0);
        assert_eq!(pw, (w / 72.0 * 200.0).round() as u32);
        assert_eq!(ph, (h / 72.0 * 200.0).round() as u32);
        let big = PageSetup {
            paper: Paper::A3,
            dpi: 600,
            ..p
        };
        let (_, bw, bh, d) = layout(&big);
        assert!(bw <= MAX_SIDE && bh <= MAX_SIDE && d < 600.0);
        assert!(check(&PageSetup { dpi: 1000, ..p }).is_err());
        assert!(check(&PageSetup { margin_mm: 80, ..p }).is_err());
    }

    /// Invariant: day 0 is 1970-01-01, and leap days fall where the
    /// calendar has them.
    #[test]
    fn dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(civil(20_782), (2026, 11, 25));
    }
}
