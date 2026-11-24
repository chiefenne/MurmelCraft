//! MurmelCraft, the desktop mesh viewer of KoflCraft and FoehnCraft.
//!
//! `murmelcraft [MESH]` opens the window, optionally with a mesh: a
//! `.vtu` file or an OpenFOAM case, `constant` or `polyMesh` folder, or
//! with a session file (`.murmel`). While agents are allowed (Settings),
//! `murmel mcp --live` drives the window.

#[cfg(test)]
mod a11y_tests;
mod agents;
mod app;
mod clip_handle;
mod color_edit;
mod commands;
mod cube;
mod export_dialog;
mod field;
mod focus;
mod i18n;
mod measure;
mod panels;
mod quality;
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
