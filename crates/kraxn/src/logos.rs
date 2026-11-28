//! The logos in Kraxn's window: Kraxn's own and those of the three
//! programs (`assets/logo`, all rights with the maintainer), as textures.

use std::collections::HashMap;

use egui::{ColorImage, TextureHandle, TextureOptions};

const KRAXN: &[u8] = include_bytes!("../../../assets/logo/kraxn-256.png");
const PRODUCTS: &[(&str, &[u8])] = &[
    (
        "murmelcraft",
        include_bytes!("../../../assets/logo/murmelcraft-256.png"),
    ),
    (
        "koflcraft",
        include_bytes!("../../../assets/logo/koflcraft-256.png"),
    ),
    (
        "foehncraft",
        include_bytes!("../../../assets/logo/foehncraft-256.png"),
    ),
];

/// Kraxn's logo as the window's icon (task bar, Dock, title bar).
pub fn icon() -> Option<egui::IconData> {
    let img = decode(KRAXN).ok()?;
    Some(egui::IconData {
        width: img.size[0] as u32,
        height: img.size[1] as u32,
        rgba: img
            .pixels
            .iter()
            .flat_map(|c| c.to_srgba_unmultiplied())
            .collect(),
    })
}

/// A PNG as an egui image (8-bit RGBA or RGB).
pub fn decode(bytes: &[u8]) -> Result<ColorImage, String> {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; r.output_buffer_size().ok_or("png too large")?];
    let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let size = [info.width as usize, info.height as usize];
    let px = &buf[..info.buffer_size()];
    Ok(match info.color_type {
        png::ColorType::Rgba => ColorImage::from_rgba_unmultiplied(size, px),
        png::ColorType::Rgb => ColorImage::from_rgb(size, px),
        png::ColorType::GrayscaleAlpha => {
            let rgba: Vec<u8> = px
                .chunks(2)
                .flat_map(|c| [c[0], c[0], c[0], c[1]])
                .collect();
            ColorImage::from_rgba_unmultiplied(size, &rgba)
        }
        _ => {
            let rgb: Vec<u8> = px.iter().flat_map(|&g| [g, g, g]).collect();
            ColorImage::from_rgb(size, &rgb)
        }
    })
}

pub struct Logos {
    pub kraxn: TextureHandle,
    pub products: HashMap<String, TextureHandle>,
}

impl Logos {
    pub fn load(ctx: &egui::Context) -> Result<Self, String> {
        let tex = |name: &str, bytes: &[u8]| -> Result<TextureHandle, String> {
            Ok(ctx.load_texture(name, decode(bytes)?, TextureOptions::LINEAR))
        };
        let mut products = HashMap::new();
        for (id, bytes) in PRODUCTS {
            products.insert(id.to_string(), tex(id, bytes)?);
        }
        Ok(Self {
            kraxn: tex("kraxn", KRAXN)?,
            products,
        })
    }
}

#[cfg(test)]
mod tests {
    /// Invariant: every logo the window shows decodes to a square image.
    /// Breaks if a logo file is replaced by one png cannot read or of
    /// another shape (the cards would stretch it).
    #[test]
    fn logos_decode_square() {
        for b in std::iter::once(super::KRAXN).chain(super::PRODUCTS.iter().map(|p| p.1)) {
            let i = super::decode(b).unwrap();
            assert_eq!(i.size[0], i.size[1]);
            assert!(i.size[0] >= 128);
        }
    }
}
