//! The export window (File, "Export image…", Ctrl + Shift + E): the size
//! (the viewport at 1x, 2x or 4x, Full HD, 4K or custom pixels), the colour
//! legend and the axis triad (each optional), a transparent background and
//! the resolution stored for print. "Export…" asks for the file and calls
//! `export.image`, so the CLI and agents get the same picture.

use std::path::PathBuf;

use egui::{Key, Modifiers, RichText};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::style::color;

/// The picture's size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeChoice {
    /// The viewport times this scale.
    Scale(u8),
    FullHd,
    Uhd,
    Custom,
}

impl SizeChoice {
    pub const ALL: [SizeChoice; 6] = [
        SizeChoice::Scale(1),
        SizeChoice::Scale(2),
        SizeChoice::Scale(4),
        SizeChoice::FullHd,
        SizeChoice::Uhd,
        SizeChoice::Custom,
    ];
}

/// The export window's settings, kept while the window is closed.
#[derive(Clone, Debug)]
pub struct ExportDialog {
    pub open: bool,
    pub size: SizeChoice,
    pub custom: [u32; 2],
    pub legend: bool,
    pub axes: bool,
    pub transparent: bool,
    /// Resolution stored in the PNG; `None` stores none (screen).
    pub dpi: Option<u32>,
    /// What the last export did, shown in the window.
    pub last: Option<String>,
    /// Write here instead of asking with the file dialog (window tests).
    pub path: Option<PathBuf>,
}

impl Default for ExportDialog {
    fn default() -> Self {
        Self {
            open: false,
            size: SizeChoice::Scale(2),
            custom: [1920, 1080],
            legend: true,
            axes: true,
            transparent: false,
            dpi: None,
            last: None,
            path: None,
        }
    }
}

impl ExportDialog {
    /// The input of `export.image`, without the path.
    pub fn input(&self) -> Value {
        let mut v = json!({
            "legend": self.legend,
            "axes": self.axes,
            "transparent": self.transparent,
        });
        match self.size {
            SizeChoice::Scale(k) => v["scale"] = json!(k),
            SizeChoice::FullHd => {
                v["width"] = json!(1920);
                v["height"] = json!(1080);
            }
            SizeChoice::Uhd => {
                v["width"] = json!(3840);
                v["height"] = json!(2160);
            }
            SizeChoice::Custom => {
                v["width"] = json!(self.custom[0]);
                v["height"] = json!(self.custom[1]);
            }
        }
        if let Some(d) = self.dpi {
            v["dpi"] = json!(d);
        }
        v
    }
}

impl MurmelApp {
    fn size_name(&self, c: SizeChoice) -> String {
        match c {
            SizeChoice::Scale(k) => self
                .i18n
                .tr_args("export-size-scale", &[("n", (k as i64).into())]),
            SizeChoice::FullHd => self.i18n.tr("export-size-full-hd"),
            SizeChoice::Uhd => self.i18n.tr("export-size-uhd"),
            SizeChoice::Custom => self.i18n.tr("export-size-custom"),
        }
    }

    fn dpi_name(&self, d: Option<u32>) -> String {
        match d {
            None => self.i18n.tr("export-dpi-screen"),
            Some(n) => self
                .i18n
                .tr_args("export-dpi-n", &[("n", (n as i64).into())]),
        }
    }

    pub(crate) fn export_window(&mut self, ctx: &egui::Context) {
        if !self.export.open {
            return;
        }
        let p = self.palette(ctx).clone();
        let mut ed = self.export.clone();
        let mut open = true;
        let mut go = false;
        let pixels = murmel_tools::export::picture_size(
            self.session.viewport,
            ed.input()["width"].as_u64().map(|w| w as u32),
            ed.input()["height"].as_u64().map(|h| h as u32),
            ed.input()["scale"].as_f64(),
        );
        egui::Window::new(self.i18n.tr("export-title"))
            .id(egui::Id::unique("export-image"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                egui::Grid::new("export-grid")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        let name = self.i18n.tr("export-size");
                        let label = ui.label(&name);
                        egui::ComboBox::from_id_salt("export-size")
                            .selected_text(self.size_name(ed.size))
                            .width(190.0)
                            .show_ui(ui, |ui| {
                                for c in SizeChoice::ALL {
                                    ui.selectable_value(&mut ed.size, c, self.size_name(c));
                                }
                            })
                            .response
                            .labelled_by(label.id);
                        ui.end_row();
                        if ed.size == SizeChoice::Custom {
                            for (k, key) in [(0, "export-width"), (1, "export-height")] {
                                let name = self.i18n.tr(key);
                                let label = ui.label(&name);
                                ui.add(
                                    egui::DragValue::new(&mut ed.custom[k])
                                        .range(16..=murmel_render::MAX_SIDE)
                                        .suffix(" px"),
                                )
                                .labelled_by(label.id);
                                ui.end_row();
                            }
                        }
                        ui.label("");
                        let note = match &pixels {
                            Ok((w, h, _)) => self.i18n.tr_args(
                                "export-pixels",
                                &[("w", (*w as i64).into()), ("h", (*h as i64).into())],
                            ),
                            Err(d) => d.message.clone(),
                        };
                        ui.label(RichText::new(note).small().color(color(p.text_secondary)));
                        ui.end_row();
                        let name = self.i18n.tr("export-dpi");
                        let label = ui.label(&name);
                        egui::ComboBox::from_id_salt("export-dpi")
                            .selected_text(self.dpi_name(ed.dpi))
                            .width(190.0)
                            .show_ui(ui, |ui| {
                                for d in [None, Some(150), Some(300)] {
                                    ui.selectable_value(&mut ed.dpi, d, self.dpi_name(d));
                                }
                            })
                            .response
                            .labelled_by(label.id);
                        ui.end_row();
                    });
                ui.add_space(4.0);
                ui.checkbox(&mut ed.legend, self.i18n.tr("export-legend"));
                ui.checkbox(&mut ed.axes, self.i18n.tr("export-axes"));
                ui.checkbox(&mut ed.transparent, self.i18n.tr("export-transparent"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let ok = pixels.is_ok() && self.session.mesh.is_some();
                    if ui
                        .add_enabled(ok, egui::Button::new(self.i18n.tr("export-save")))
                        .clicked()
                    {
                        go = true;
                    }
                });
                if let Some(last) = &ed.last {
                    ui.label(RichText::new(last).small().color(color(p.text_secondary)));
                }
            });
        let esc = !ctx.text_edit_focused()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        ed.open = open && !esc;
        self.export = ed;
        if go {
            self.export_image();
        }
    }

    /// Ask for the file (unless the window test set one) and export.
    fn export_image(&mut self) {
        let path = match self.export.path.clone() {
            Some(p) => Some(p),
            None => rfd::FileDialog::new()
                .set_title(self.i18n.tr("export-file-title"))
                .add_filter("PNG", &["png"])
                .set_file_name("murmelcraft.png")
                .save_file(),
        };
        let Some(path) = path else { return };
        let mut input = self.export.input();
        input["path"] = json!(path);
        let file = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        self.export.last = self.call("export.image", input).map(|r| {
            self.i18n.tr_args(
                "export-done",
                &[
                    ("file", file.into()),
                    ("w", r["width"].as_i64().unwrap_or(0).into()),
                    ("h", r["height"].as_i64().unwrap_or(0).into()),
                ],
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the window's choices map to the tool's input: a scale
    /// for the viewport sizes, pixels for HD, 4K and custom, the dpi only
    /// when one is chosen. Breaks when a choice sends the wrong field; the
    /// window test exports one choice only.
    #[test]
    fn choices_to_input() {
        let mut d = ExportDialog::default();
        assert_eq!(d.input()["scale"], 2);
        assert!(d.input().get("dpi").is_none());
        d.size = SizeChoice::Uhd;
        d.dpi = Some(300);
        d.transparent = true;
        let v = d.input();
        assert_eq!(
            (v["width"].clone(), v["height"].clone()),
            (json!(3840), json!(2160))
        );
        assert_eq!(v["dpi"], 300);
        assert_eq!(v["transparent"], true);
        assert!(v.get("scale").is_none());
        d.size = SizeChoice::Custom;
        d.custom = [800, 600];
        assert_eq!(d.input()["width"], 800);
    }
}
