//! The menu bar in the title row, drawn from the table of
//! `murmel_tools::menus` (in the window on every system, the maintainer's
//! card of 2026-11-25), and the windows only the menus open: Undo history
//! and About.

use egui::{Button, RichText, Role, Ui, WidgetInfo};
use murmel_tools::commands::{Command, Run};
use murmel_tools::menus::{Entry, menus, resolve};
use serde_json::json;

use crate::app::MurmelApp;
use crate::commands::key_text;
use crate::focus::Region;
use crate::style::color;

/// The repository, for the Help menu's pages.
pub const REPO: &str = "https://github.com/chiefenne/MurmelCraft";

/// What a click in a menu asks for.
pub(crate) enum Picked {
    Command(Command),
    /// A file of File ▸ Open recent.
    Recent(murmel_engine::recent::Entry),
}

/// Whether a command needs an open mesh: every tool but the settings,
/// the journal, the charts and those that open something; of the window
/// actions only the clipboard copy of the view.
pub(crate) fn needs_mesh(c: &Command) -> bool {
    match c.tool() {
        Some(t) => {
            !(t.starts_with("settings.")
                || t.starts_with("journal.")
                || t.starts_with("charts.")
                || t.starts_with("recent.")
                || matches!(t, "mesh.open" | "session.load" | "series.open"))
        }
        None => matches!(c.id.as_str(), "window.copy_image" | "window.print"),
    }
}

impl MurmelApp {
    /// Whether a command applies now, and if not, why (a translation
    /// key).
    pub(crate) fn applies(&self, c: &Command) -> Result<(), &'static str> {
        if needs_mesh(c) && self.session.mesh.is_none() {
            return Err("menu-needs-mesh");
        }
        if matches!(c.run, Run::Dynamic(..)) && c.input(&self.session).is_none() {
            return Err("menu-not-now");
        }
        Ok(())
    }

    /// The name of a menu item: Undo and Redo name the step they walk.
    fn item_label(&self, c: &Command) -> String {
        let step = match c.id.as_str() {
            "journal.undo" => self.session.journal.undo.last(),
            "journal.redo" => self.session.journal.redo.last(),
            _ => None,
        };
        match step.and_then(|s| murmel_tools::find(&s.tool)) {
            Some(t) => {
                let key = if c.id == "journal.undo" {
                    "menu-undo-step"
                } else {
                    "menu-redo-step"
                };
                self.i18n
                    .tr_args(key, &[("step", self.i18n.tr(t.label_key).into())])
            }
            None => self.i18n.tr(&c.label_key),
        }
    }

    /// The menu bar: one button per menu; a click on an item runs its
    /// command after the bar is drawn.
    pub(crate) fn menu_bar(&mut self, ui: &mut Ui) {
        let mut run: Option<Picked> = None;
        let name = self.i18n.tr("menu-bar");
        let bar = egui::MenuBar::new().ui(ui, |ui| {
            for (n, m) in menus().iter().enumerate() {
                let r = ui.menu_button(self.i18n.tr(&m.label_key()), |ui| {
                    ui.set_min_width(220.0);
                    self.menu_entries(ui, &m.entries, &mut run);
                });
                if n == 0 {
                    self.focus.offer(Region::Title, &r.response);
                }
            }
        });
        bar.response
            .widget_info(|| WidgetInfo::labeled(Role::MenuBar, true, name.clone()));
        match run {
            Some(Picked::Command(c)) => {
                let ctx = ui.ctx().clone();
                self.run_command(&ctx, &c);
            }
            Some(Picked::Recent(e)) => self.open_recent(e),
            None => {}
        }
    }

    /// Open a file of File ▸ Open recent again, as it was opened.
    pub(crate) fn open_recent(&mut self, e: murmel_engine::recent::Entry) {
        use murmel_engine::recent::Kind;
        match e.kind {
            Kind::Mesh | Kind::Session => self.open_path(e.path),
            k => {
                self.call(k.tool(), json!({ "path": e.path }));
            }
        }
    }

    /// The files opened lately, each a button named by its file and
    /// folder; a file gone since is greyed out.
    fn recent_entries(&self, ui: &mut Ui, run: &mut Option<Picked>) {
        let files = self
            .session
            .recent_dir()
            .map(murmel_engine::recent::load)
            .unwrap_or_default();
        if files.is_empty() {
            ui.add_enabled(false, Button::new(self.i18n.tr("recent-empty")));
        }
        for e in files {
            let name = e
                .path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| e.path.display().to_string());
            let dir = e
                .path
                .parent()
                .and_then(|d| d.file_name())
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default();
            let there = e.path.exists();
            let label = if there {
                name
            } else {
                self.i18n
                    .tr_args("recent-missing", &[("name", name.into())])
            };
            let secondary = color(self.palette(ui.ctx()).text_secondary);
            // Where it comes from, beside its folder (the maintainer's
            // card of 2026-11-25: one list, a tag per entry).
            use murmel_engine::origin::Origin;
            use murmel_engine::recent::Kind;
            let tag = match (e.kind, e.origin) {
                (Kind::Mesh, Some(o)) => Some(self.origin_tag(o, e.imported)),
                (Kind::Mesh, None) => None,
                (Kind::Session, _) => Some(self.i18n.tr("origin-session")),
                (Kind::Series | Kind::Parcels | Kind::Charts, _) => {
                    Some(self.origin_tag(Origin::Foehncraft, false))
                }
            };
            let side = match tag {
                Some(t) if dir.is_empty() => t,
                Some(t) => format!("{dir} · {t}"),
                None => dir,
            };
            let b = ui
                .add_enabled(
                    there,
                    Button::new(label).shortcut_text(RichText::new(side).color(secondary)),
                )
                .on_hover_text(e.path.display().to_string())
                .on_disabled_hover_text(e.path.display().to_string());
            if b.clicked() {
                *run = Some(Picked::Recent(e));
            }
        }
    }

    fn menu_entries(&self, ui: &mut Ui, entries: &[Entry], run: &mut Option<Picked>) {
        for e in entries {
            match e {
                Entry::Separator => {
                    ui.separator();
                }
                Entry::Recent => self.recent_entries(ui, run),
                Entry::Sub(key, sub) => {
                    ui.menu_button(self.i18n.tr(key), |ui| {
                        ui.set_min_width(220.0);
                        self.menu_entries(ui, sub, run);
                    });
                }
                _ => {
                    for c in resolve(e, &self.commands) {
                        self.menu_item(ui, c, run);
                    }
                }
            }
        }
    }

    fn menu_item(&self, ui: &mut Ui, c: &Command, run: &mut Option<Picked>) {
        let label = self.item_label(c);
        let keys = c.keys.first().map(|k| key_text(k)).unwrap_or_default();
        let secondary = color(self.palette(ui.ctx()).text_secondary);
        let why = self.applies(c).err();
        let b = ui.add_enabled(
            why.is_none(),
            Button::new(label).shortcut_text(RichText::new(keys).color(secondary)),
        );
        let b = match why {
            Some(key) => b.on_disabled_hover_text(self.i18n.tr(key)),
            None => self.tip(b, &c.id),
        };
        if b.clicked() {
            *run = Some(Picked::Command(c.clone()));
        }
    }

    /// Copy the view to the clipboard, rendered at the viewport's size in
    /// pixels.
    pub(crate) fn copy_image(&mut self, ctx: &egui::Context) {
        let ppp = ctx.pixels_per_point() as f64;
        let [w, h] = self.session.viewport;
        let side = |v: f64| ((v * ppp).round() as u32).clamp(1, murmel_render::MAX_SIDE);
        match murmel_render::render_session(&self.session, side(w), side(h)) {
            Ok(shot) => ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied(
                [shot.width as usize, shot.height as usize],
                &shot.rgba,
            )),
            Err(e) => self.set_error(Some(murmel_tools::Diagnostic::new(e.code, e.message))),
        }
    }

    /// Edit ▸ Undo history: the steps to undo (newest last) and to redo;
    /// a click on one walks there with journal.undo or journal.redo.
    pub(crate) fn history_window(&mut self, ctx: &egui::Context) {
        if !self.history_open {
            return;
        }
        let p = self.palette(ctx).clone();
        let mut open = true;
        let mut walk: Option<(&'static str, usize)> = None;
        let label = |tool: &str| {
            murmel_tools::find(tool)
                .map(|t| self.i18n.tr(t.label_key))
                .unwrap_or_else(|| tool.to_string())
        };
        let undo: Vec<String> = self
            .session
            .journal
            .undo
            .iter()
            .map(|s| label(&s.tool))
            .collect();
        let redo: Vec<String> = self
            .session
            .journal
            .redo
            .iter()
            .rev()
            .map(|s| label(&s.tool))
            .collect();
        egui::Window::new(self.i18n.tr("undo-history-title"))
            .open(&mut open)
            .collapsible(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                if undo.is_empty() && redo.is_empty() {
                    ui.label(
                        RichText::new(self.i18n.tr("undo-history-empty"))
                            .color(color(p.text_secondary)),
                    );
                    return;
                }
                ui.label(
                    RichText::new(self.i18n.tr("undo-history-tip"))
                        .small()
                        .color(color(p.text_secondary)),
                );
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        let n = undo.len();
                        for (k, name) in undo.iter().enumerate() {
                            // Undoing back to step k takes n − k calls.
                            if ui.selectable_label(false, name).clicked() {
                                walk = Some(("journal.undo", n - k));
                            }
                        }
                        ui.add_enabled(
                            false,
                            Button::selectable(true, self.i18n.tr("undo-history-now")),
                        );
                        for (k, name) in redo.iter().enumerate() {
                            let r = ui.selectable_label(
                                false,
                                RichText::new(name).color(color(p.text_secondary)),
                            );
                            if r.clicked() {
                                walk = Some(("journal.redo", k + 1));
                            }
                        }
                    });
            });
        if let Some((tool, n)) = walk {
            for _ in 0..n {
                self.call(tool, json!({}));
            }
        }
        self.history_open &= open;
    }

    /// Help ▸ About: version, licence and the repository.
    pub(crate) fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let p = self.palette(ctx).clone();
        let mut open = true;
        egui::Window::new(self.i18n.tr("about-title"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    if let Some(t) = &self.logo {
                        ui.add(egui::Image::new(t).fit_to_exact_size(egui::vec2(96.0, 96.0)));
                    }
                    ui.heading(self.i18n.tr("app-name"));
                    ui.label(self.i18n.tr_args(
                        "about-version",
                        &[("version", env!("CARGO_PKG_VERSION").into())],
                    ));
                    ui.add_space(6.0);
                    ui.label(self.i18n.tr("about-text"));
                    ui.label(
                        RichText::new(self.i18n.tr("about-licence"))
                            .small()
                            .color(color(p.text_secondary)),
                    );
                    ui.add_space(6.0);
                    ui.hyperlink_to(self.i18n.tr("about-repo"), REPO);
                });
            });
        self.about_open &= open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: opening, saving and the settings never wait for a mesh,
    /// and the tools that act on the model always do, so a menu item is
    /// greyed out exactly when it cannot work. Breaks when Open is
    /// greyed out on an empty window, or Fit is offered without a mesh
    /// (its call would only report VIEW001).
    #[test]
    fn mesh_is_needed_only_by_model_commands() {
        let all = murmel_tools::commands::commands();
        let by = |id: &str| all.iter().find(|c| c.id == id).unwrap();
        for id in [
            "window.open",
            "window.import_openfoam",
            "window.import_vtk",
            "window.session_load",
            "window.series_open",
            "window.settings",
            "window.page_setup",
            "window.quit",
            "window.about",
            "settings.set_theme:dark",
            "journal.undo",
        ] {
            assert!(!needs_mesh(by(id)), "{id}");
        }
        for id in [
            "view.fit",
            "mesh.close",
            "window.export_image",
            "window.copy_image",
            "window.print",
            "window.export_pdf",
            "clip.set:x",
            "window.measure_distance",
        ] {
            assert!(needs_mesh(by(id)), "{id}");
        }
    }
}
