//! File ▸ Export PDF page…, File ▸ Print… and File ▸ Page setup… (the
//! maintainer's card of 2026-11-25: "PDF page + Print + Page setup").
//! Print writes the page with `export.pdf` to the temporary folder and
//! hands it to the system: on macOS Preview's print dialog (or Preview
//! itself if it cannot be asked to print), elsewhere the PDF viewer.

use std::path::{Path, PathBuf};

use egui::{RichText, Role, WidgetInfo};
use murmel_engine::Theme;
use murmel_engine::settings::{DPI, MAX_MARGIN_MM, Orientation, PageSetup, Paper};
use serde_json::json;

use crate::app::MurmelApp;
use crate::style::color;

/// Where Print writes its page.
fn print_path() -> PathBuf {
    std::env::temp_dir().join(format!("murmelcraft-print-{}.pdf", std::process::id()))
}

/// Hand a PDF to the system to print, on a thread of its own.
fn print_file(path: &Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        #[cfg(target_os = "macos")]
        {
            let p = path
                .display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            let script = format!(
                "tell application \"Preview\"\nactivate\nprint POSIX file \"{p}\" with print dialog\nend tell"
            );
            let asked = std::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .status()
                .is_ok_and(|s| s.success());
            if !asked {
                let _ = std::process::Command::new("open")
                    .args(["-a", "Preview"])
                    .arg(&path)
                    .status();
            }
        }
        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", ""])
                .arg(&path)
                .status();
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = std::process::Command::new("xdg-open").arg(&path).status();
        }
    });
}

impl MurmelApp {
    /// File ▸ Export PDF page…: where to, then export.pdf with Page setup.
    pub(crate) fn export_pdf_dialog(&mut self) {
        let name = self
            .session
            .mesh
            .as_ref()
            .and_then(|m| m.path.file_stem())
            .map(|s| format!("{}.pdf", s.to_string_lossy()))
            .unwrap_or_else(|| "murmelcraft.pdf".into());
        let picked = rfd::FileDialog::new()
            .set_title(self.i18n.tr("cmd-export-pdf"))
            .add_filter("PDF", &["pdf"])
            .set_file_name(name)
            .save_file();
        if let Some(path) = picked {
            self.call("export.pdf", json!({ "path": path }));
        }
    }

    /// File ▸ Print…: the page as Page setup has it, to the system.
    pub(crate) fn print_page(&mut self) {
        let path = print_path();
        if self.call("export.pdf", json!({ "path": path })).is_some() {
            print_file(&path);
        }
    }

    /// File ▸ Page setup…: the page of Print and Export PDF page, kept in
    /// settings.toml.
    pub(crate) fn page_setup_window(&mut self, ctx: &egui::Context) {
        if !self.page_setup_open {
            return;
        }
        let secondary = color(self.palette(ctx).text_secondary);
        let now = self.session.page;
        let mut p = now;
        let mut open = true;
        let mut reset = false;
        let mut action = None;
        egui::Window::new(self.i18n.tr("page-title"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                egui::Grid::new("page-grid")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        let name = self.i18n.tr("page-paper");
                        ui.label(&name);
                        egui::ComboBox::from_id_salt("page-paper")
                            .selected_text(self.i18n.tr(&format!("paper-{}", p.paper.key())))
                            .show_ui(ui, |ui| {
                                for k in Paper::ALL {
                                    let t = self.i18n.tr(&format!("paper-{}", k.key()));
                                    ui.selectable_value(&mut p.paper, k, t);
                                }
                            })
                            .response
                            .widget_info(|| {
                                WidgetInfo::labeled(Role::ComboBox, true, name.clone())
                            });
                        ui.end_row();
                        ui.label(self.i18n.tr("page-orientation"));
                        ui.horizontal(|ui| {
                            for (o, key) in [
                                (Orientation::Landscape, "page-landscape"),
                                (Orientation::Portrait, "page-portrait"),
                            ] {
                                ui.radio_value(&mut p.orientation, o, self.i18n.tr(key));
                            }
                        });
                        ui.end_row();
                        let name = self.i18n.tr("page-margin");
                        ui.label(&name);
                        ui.add(
                            egui::DragValue::new(&mut p.margin_mm)
                                .range(0..=MAX_MARGIN_MM)
                                .suffix(" mm"),
                        )
                        .widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name.clone()));
                        ui.end_row();
                        let name = self.i18n.tr("page-dpi");
                        ui.label(&name);
                        ui.add(
                            egui::DragValue::new(&mut p.dpi)
                                .range(DPI.0..=DPI.1)
                                .speed(5.0)
                                .suffix(" dpi"),
                        )
                        .widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name.clone()));
                        ui.end_row();
                        ui.label(self.i18n.tr("page-theme"));
                        ui.horizontal(|ui| {
                            for t in [Theme::Light, Theme::Dark] {
                                let key = format!("theme-{}", crate::app::theme_key(t));
                                ui.radio_value(&mut p.theme, t, self.i18n.tr(&key));
                            }
                        });
                        ui.end_row();
                    });
                ui.checkbox(&mut p.header, self.i18n.tr("page-header"));
                let (w, h) = p.size();
                ui.label(
                    RichText::new(self.i18n.tr_args(
                        "page-size",
                        &[
                            ("width", format!("{:.0}", w * 25.4 / 72.0).into()),
                            ("height", format!("{:.0}", h * 25.4 / 72.0).into()),
                        ],
                    ))
                    .small()
                    .color(secondary),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let has_mesh = self.session.mesh.is_some();
                    if ui
                        .add_enabled(has_mesh, egui::Button::new(self.i18n.tr("page-print")))
                        .clicked()
                    {
                        action = Some(true);
                    }
                    if ui
                        .add_enabled(has_mesh, egui::Button::new(self.i18n.tr("page-export")))
                        .clicked()
                    {
                        action = Some(false);
                    }
                    if !now.is_default() && ui.button(self.i18n.tr("page-reset")).clicked() {
                        reset = true;
                    }
                });
            });
        self.page_setup_open &= open;
        if reset {
            self.call("settings.set_page", json!({"reset": true}));
        } else if p != now {
            self.set_page(&now, &p);
        }
        match action {
            Some(true) => self.print_page(),
            Some(false) => self.export_pdf_dialog(),
            None => {}
        }
    }

    /// Save the values of the page that changed, and only those.
    fn set_page(&mut self, now: &PageSetup, p: &PageSetup) {
        let mut input = json!({});
        if p.paper != now.paper {
            input["paper"] = json!(p.paper);
        }
        if p.orientation != now.orientation {
            input["orientation"] = json!(p.orientation);
        }
        if p.margin_mm != now.margin_mm {
            input["margin_mm"] = json!(p.margin_mm);
        }
        if p.dpi != now.dpi {
            input["dpi"] = json!(p.dpi);
        }
        if p.header != now.header {
            input["header"] = json!(p.header);
        }
        if p.theme != now.theme {
            input["theme"] = json!(p.theme);
        }
        self.call("settings.set_page", input);
    }
}
