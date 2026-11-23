//! The window: title bar, toolbar, viewport and status bar (V0.1 of the
//! UI spec). The window holds no state of its own beyond what it draws:
//! every action is a call of a registered tool on the session.

use std::path::PathBuf;

use egui::{Align, Color32, Layout, RichText, Role, Sense, Ui, WidgetInfo};
use murmel_engine::{Session, Theme, Tokens, settings};
use murmel_tools::Diagnostic;
use serde_json::{Value, json};

use crate::i18n::{I18n, count};
use crate::style::{self, color, icon};
use crate::viewport::GpuView;

pub struct MurmelApp {
    pub(crate) session: Session,
    pub(crate) i18n: I18n,
    pub(crate) gpu: Option<GpuView>,
    pub(crate) error: Option<Diagnostic>,
    pub(crate) drag: Option<murmel_engine::navigation::Action>,
    pub(crate) settings_open: bool,
    pub(crate) installed_theme: Option<Theme>,
}

impl MurmelApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Result<Self, String> {
        let tokens = Tokens::builtin()?;
        style::install(&cc.egui_ctx, &tokens);
        let mut session = Session::new(tokens);
        let mut error = None;
        if let Some(dir) = settings::config_dir() {
            let (st, nav, problems) = settings::load(&dir, cfg!(target_os = "macos"));
            session.theme = st.theme;
            session.navigation = nav;
            if let Some(p) = problems.into_iter().next() {
                error = Some(Diagnostic::new("SET001", p));
            }
        }
        let gpu = cc.wgpu_render_state.as_ref().map(GpuView::new);
        let mut app = Self {
            session,
            i18n: I18n::english()?,
            gpu,
            error,
            drag: None,
            settings_open: false,
            installed_theme: None,
        };
        if let Some(p) = open {
            app.open_path(p);
        }
        Ok(app)
    }

    /// Call a tool on the session; a failure goes to the status bar.
    pub(crate) fn call(&mut self, id: &str, input: Value) -> Option<Value> {
        let e = murmel_tools::call(id, input, &mut self.session);
        if e.ok() {
            if id == "mesh.open" {
                self.error = None;
            }
            Some(e.result)
        } else {
            self.error = e.diagnostics.into_iter().next();
            None
        }
    }

    pub(crate) fn open_path(&mut self, path: PathBuf) {
        self.call("mesh.open", json!({ "path": path }));
    }

    fn open_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title(self.i18n.tr("toolbar-open-tip"))
            .add_filter("VTK unstructured grid", &["vtu"])
            .pick_file();
        if let Some(p) = picked {
            self.open_path(p);
        }
    }

    pub(crate) fn dark(&self, ctx: &egui::Context) -> bool {
        ctx.theme() == egui::Theme::Dark
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        if self.installed_theme == Some(self.session.theme) {
            return;
        }
        ctx.set_theme(match self.session.theme {
            Theme::Dark => egui::ThemePreference::Dark,
            Theme::Light => egui::ThemePreference::Light,
            Theme::System => egui::ThemePreference::System,
        });
        self.installed_theme = Some(self.session.theme);
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::Key;
        let (open, fit, home, cycle, zin, zout, arrows, shift) = ctx.input_mut(|i| {
            let open = i.consume_key(egui::Modifiers::COMMAND, Key::O);
            let fit = i.consume_key(egui::Modifiers::NONE, Key::F);
            let home = i.consume_key(egui::Modifiers::NONE, Key::Home);
            let cycle = i.consume_key(egui::Modifiers::NONE, Key::Num5);
            let zin = i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals);
            let zout = i.key_pressed(Key::Minus);
            let arrows = [
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::ArrowUp,
                Key::ArrowDown,
            ]
            .map(|k| i.key_pressed(k));
            (open, fit, home, cycle, zin, zout, arrows, i.modifiers.shift)
        });
        if open {
            self.open_dialog();
        }
        if fit {
            self.call("view.fit", json!({}));
        }
        if home {
            self.call("view.standard", json!({"view": "home"}));
        }
        if cycle {
            let next = self.session.camera.projection.next();
            self.call("view.set_camera", json!({ "projection": next }));
        }
        if zin {
            self.call("view.zoom", json!({"factor": 0.8}));
        }
        if zout {
            self.call("view.zoom", json!({"factor": 1.25}));
        }
        let [l, r, u, d] = arrows;
        let (dx, dy) = ((r as i32 - l as i32) as f64, (d as i32 - u as i32) as f64);
        if dx != 0.0 || dy != 0.0 {
            if shift {
                let step = 0.05 * self.session.viewport[1];
                self.call("view.pan", json!({"dx_px": dx * step, "dy_px": dy * step}));
            } else {
                self.call(
                    "view.orbit",
                    json!({"yaw_deg": -15.0 * dx, "pitch_deg": 15.0 * dy}),
                );
            }
        }
    }

    fn palette(&self, ctx: &egui::Context) -> &murmel_engine::theme::Palette {
        self.session.tokens.palette(self.dark(ctx))
    }

    fn title_bar(&mut self, ui: &mut Ui) {
        let p = self.palette(ui.ctx()).clone();
        let logo = color(self.session.tokens.mesh.logo);
        ui.horizontal_centered(|ui| {
            // Placeholder mark until the marmot logo arrives.
            let (r, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), Sense::hover());
            ui.painter().rect_filled(r, 6.0, logo);
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "M",
                egui::FontId::new(13.0, egui::FontFamily::Name(style::SEMIBOLD.into())),
                Color32::WHITE,
            );
            ui.label(
                RichText::new(self.i18n.tr("app-name"))
                    .family(egui::FontFamily::Name(style::SEMIBOLD.into()))
                    .size(14.0),
            );
            let file = self
                .session
                .mesh
                .as_ref()
                .map(|m| {
                    let name = m.path.file_name().map(|f| f.to_string_lossy().into_owned());
                    let dir = m
                        .path
                        .parent()
                        .and_then(|d| d.file_name())
                        .map(|d| d.to_string_lossy().into_owned());
                    match (dir, name) {
                        (Some(d), Some(n)) => format!("{d} / {n}"),
                        (_, Some(n)) => n,
                        _ => String::new(),
                    }
                })
                .unwrap_or_else(|| self.i18n.tr("app-no-file"));
            ui.label(RichText::new(file).color(color(p.text_secondary)));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let settings = self.i18n.tr("settings-title");
                if icon_button(
                    ui,
                    icon::SETTINGS,
                    &settings,
                    &self.i18n.tr("settings-tip"),
                    self.settings_open,
                )
                .clicked()
                {
                    self.settings_open = !self.settings_open;
                }
                let (glyph, next) = match self.session.theme {
                    Theme::Dark => (icon::MOON, Theme::Light),
                    Theme::Light => (icon::SUN, Theme::System),
                    Theme::System => (icon::MONITOR, Theme::Dark),
                };
                let name = self
                    .i18n
                    .tr(&format!("theme-{}", theme_key(self.session.theme)));
                if icon_button(ui, glyph, &name, &self.i18n.tr("settings-theme"), false).clicked() {
                    self.call("settings.set_theme", json!({ "theme": next }));
                }
            });
        });
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let secondary = color(self.palette(ui.ctx()).text_secondary);
        ui.horizontal_centered(|ui| {
            let open = self.i18n.tr("toolbar-open");
            let b = ui
                .add(egui::Button::new(
                    RichText::new(format!("{}  {open}", icon::FOLDER_OPEN)).size(13.0),
                ))
                .on_hover_text(self.i18n.tr("toolbar-open-tip"));
            if b.clicked() {
                self.open_dialog();
            }
            ui.separator();
            ui.label(
                RichText::new(self.i18n.tr("toolbar-group-view").to_uppercase())
                    .small()
                    .color(secondary),
            );
            let fit = self.i18n.tr("toolbar-fit");
            if icon_button(
                ui,
                icon::SCAN,
                &fit,
                &self.i18n.tr("toolbar-fit-tip"),
                false,
            )
            .clicked()
            {
                self.call("view.fit", json!({}));
            }
            let home = self.i18n.tr("toolbar-home");
            if icon_button(
                ui,
                icon::HOUSE,
                &home,
                &self.i18n.tr("toolbar-home-tip"),
                false,
            )
            .clicked()
            {
                self.call("view.standard", json!({"view": "home"}));
            }
            let proj = self.session.camera.projection;
            let glyph = if self.session.camera.is_orthographic() {
                icon::SQUARE
            } else {
                icon::BOX
            };
            let short = self.i18n.tr(&format!("projection-short-{}", proj.key()));
            let long = self.i18n.tr(&format!("projection-{}", proj.key()));
            let b = ui
                .add(egui::Button::new(RichText::new(format!(
                    "{glyph}  {short}"
                ))))
                .on_hover_text(format!(
                    "{long}\n{}",
                    self.i18n.tr("toolbar-projection-tip")
                ));
            b.widget_info(|| WidgetInfo::labeled(Role::Button, true, long.clone()));
            if b.clicked() {
                self.call("view.set_camera", json!({ "projection": proj.next() }));
            }
        });
    }

    fn hint(&self) -> String {
        use murmel_engine::navigation::{Action, Button, Gesture};
        let nav = &self.session.navigation;
        let describe = |action: Action| -> Option<String> {
            let b = nav.of(action).next()?;
            let mut parts = Vec::new();
            for (on, key) in [
                (b.mods.cmd, "mod-cmd"),
                (b.mods.ctrl, "mod-ctrl"),
                (b.mods.alt, "mod-alt"),
                (b.mods.shift, "mod-shift"),
            ] {
                if on {
                    parts.push(self.i18n.tr(key));
                }
            }
            parts.push(self.i18n.tr(match (b.gesture, b.button) {
                (Gesture::Scroll, _) => "input-scroll",
                (Gesture::Pinch, _) => "input-pinch",
                (_, Some(Button::Secondary)) => "input-secondary",
                (_, Some(Button::Middle)) => "input-middle",
                _ => "input-primary",
            }));
            Some(parts.join(" + "))
        };
        let mut out = Vec::new();
        for (a, key) in [
            (Action::Orbit, "hint-orbit"),
            (Action::Pan, "hint-pan"),
            (Action::Zoom, "hint-zoom"),
        ] {
            if let Some(input) = describe(a) {
                out.push(self.i18n.tr_args(key, &[("input", input.into())]));
            }
        }
        out.join(" · ")
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let p = self.palette(ui.ctx()).clone();
        let secondary = color(p.text_secondary);
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new(self.hint()).small().color(secondary));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(self.i18n.tr("status-units"))
                        .small()
                        .color(secondary),
                );
                if let Some(g) = &self.gpu {
                    ui.label(RichText::new(g.adapter.clone()).small().color(secondary));
                }
                let ms = ui.input(|i| i.stable_dt) * 1000.0;
                ui.label(
                    RichText::new(
                        self.i18n
                            .tr_args("status-frame", &[("ms", format!("{ms:.1}").into())]),
                    )
                    .monospace()
                    .color(secondary),
                );
                if let Some(e) = &self.error {
                    let text = self.i18n.tr_args(
                        "status-error",
                        &[
                            ("code", e.code.into()),
                            ("message", e.message.clone().into()),
                        ],
                    );
                    ui.label(
                        RichText::new(format!("{}  {text}", icon::ALERT))
                            .small()
                            .color(color(p.status_error)),
                    )
                    .on_hover_text(e.fix.unwrap_or(""));
                } else if let Some(m) = &self.session.mesh {
                    let s = (m.mesh.cell_count(), m.mesh.boundary.len());
                    ui.label(
                        RichText::new(self.i18n.tr_args(
                            "status-cells",
                            &[("cells", count(s.0).into()), ("faces", count(s.1).into())],
                        ))
                        .small(),
                    );
                }
            });
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let title = self.i18n.tr("settings-title");
        egui::Window::new(title)
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.heading(self.i18n.tr("settings-theme"));
                for t in [Theme::Dark, Theme::Light, Theme::System] {
                    let label = self.i18n.tr(&format!("theme-{}", theme_key(t)));
                    if ui.radio(self.session.theme == t, label).clicked() {
                        self.call("settings.set_theme", json!({ "theme": t }));
                    }
                }
                ui.add_space(8.0);
                ui.heading(self.i18n.tr("settings-navigation"));
                let current = self.session.navigation.preset.clone();
                for p in murmel_engine::navigation::PRESETS {
                    let label = self.i18n.tr(&format!("preset-{p}"));
                    if ui.radio(current == *p, label).clicked() {
                        self.call("settings.set_bindings", json!({ "preset": p }));
                    }
                }
                if current == "custom" {
                    let _ = ui.radio(true, self.i18n.tr("preset-custom"));
                }
                let mut z = self.session.navigation.zoom_to_cursor;
                if ui
                    .checkbox(&mut z, self.i18n.tr("settings-zoom-to-cursor"))
                    .changed()
                {
                    self.call("settings.set_bindings", json!({ "zoom_to_cursor": z }));
                }
                let mut inv = self.session.navigation.invert_zoom;
                if ui
                    .checkbox(&mut inv, self.i18n.tr("settings-invert-zoom"))
                    .changed()
                {
                    self.call("settings.set_bindings", json!({ "invert_zoom": inv }));
                }
                if let Some(d) = &self.session.config_dir {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(
                            self.i18n.tr_args(
                                "settings-file",
                                &[("path", d.display().to_string().into())],
                            ),
                        )
                        .small(),
                    );
                }
            });
        self.settings_open &= open;
    }
}

pub(crate) fn theme_key(t: Theme) -> &'static str {
    match t {
        Theme::Dark => "dark",
        Theme::Light => "light",
        Theme::System => "system",
    }
}

/// A 28 px icon button with an accessible name and a tooltip.
pub(crate) fn icon_button(
    ui: &mut Ui,
    glyph: &str,
    name: &str,
    tip: &str,
    selected: bool,
) -> egui::Response {
    let b = ui
        .add(
            egui::Button::new(RichText::new(glyph).size(16.0))
                .selected(selected)
                .min_size(egui::vec2(28.0, 28.0)),
        )
        .on_hover_text(tip);
    let name = name.to_string();
    b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
    b
}

impl eframe::App for MurmelApp {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_theme(&ctx);
        self.keys(&ctx);
        let dropped: Option<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .find_map(|f| Some(f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()))
        });
        if let Some(p) = dropped {
            self.open_path(p);
        }
        let p = self.palette(&ctx).clone();
        let bar = |fill: murmel_engine::Rgb| {
            egui::Frame::NONE
                .fill(color(fill))
                .inner_margin(egui::Margin::symmetric(12, 0))
                .stroke(egui::Stroke::new(1.0, color(p.line)))
        };
        egui::Panel::top("title")
            .exact_size(36.0)
            .frame(bar(p.bg_app))
            .show(ui, |ui| self.title_bar(ui));
        egui::Panel::top("toolbar")
            .exact_size(40.0)
            .frame(bar(p.bg_panel))
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(bar(p.bg_app))
            .show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(color(p.viewport_bottom)))
            .show(ui, |ui| self.viewport(ui, frame));
        self.settings_window(&ctx);
    }
}
