//! MurmelCraft, the desktop mesh viewer of KoflCraft and FoehnCraft.
//!
//! `murmelcraft [MESH.vtu]` opens the window, optionally with a mesh.

mod app;
mod i18n;
mod style;
mod viewport;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let open = std::env::args_os().nth(1).map(PathBuf::from);
    let mut viewport = egui::ViewportBuilder::default();
    if let Some((width, height, rgba)) = style::logo_rgba(256) {
        viewport = viewport.with_icon(egui::IconData {
            rgba,
            width,
            height,
        });
    }
    let options = eframe::NativeOptions {
        viewport: viewport
            .with_title("MurmelCraft")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "MurmelCraft",
        options,
        Box::new(|cc| {
            let app = app::MurmelApp::new(cc, open)?;
            Ok(Box::new(app))
        }),
    )
}
