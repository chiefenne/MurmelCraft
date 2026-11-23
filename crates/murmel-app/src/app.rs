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
    /// The pointer resting over the viewport, and what `inspect.at` found
    /// under it (the hover tooltip and the cursor coordinates).
    pub(crate) hover: Option<crate::viewport::Hover>,
    pub(crate) settings_open: bool,
    pub(crate) installed_theme: Option<Theme>,
    logo: Option<egui::TextureHandle>,
    /// Side panels shown (T and P toggle them).
    pub(crate) show_tree: bool,
    pub(crate) show_props: bool,
    pub(crate) tree_filter: String,
    pub(crate) focus_filter: bool,
    pub(crate) props: Option<crate::panels::PropsCache>,
    /// The command table (keys, palette, shortcut sheet).
    pub(crate) commands: Vec<murmel_tools::commands::Command>,
    pub(crate) palette: Option<crate::commands::Palette>,
    pub(crate) shortcuts_open: bool,
    /// The 250 ms flight to a new view: the camera it started from and
    /// when (UI spec, View cube).
    pub(crate) flight: Option<(murmel_engine::Camera, f64)>,
    /// The time of the frame, for flights started by a tool call.
    pub(crate) now: f64,
    /// The rotate row under the view cube: axis and degrees.
    pub(crate) rotate_axis: murmel_engine::ScreenAxis,
    pub(crate) rotate_deg: f64,
    /// The quality threshold while its slider is dragged (the surface is
    /// rebuilt when the drag ends).
    pub(crate) threshold_edit: Option<f64>,
    pub(crate) histogram: Option<crate::quality::HistogramCache>,
    /// The world point the current viewport orbit turns about.
    pub(crate) orbit_pivot: Option<[f64; 3]>,
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
            let mut problems = problems;
            if let Err(e) = session.set_colors(st.colors) {
                problems.push(format!("settings.toml colors: {e}"));
            }
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
            hover: None,
            settings_open: false,
            installed_theme: None,
            show_tree: true,
            show_props: true,
            tree_filter: String::new(),
            focus_filter: false,
            props: None,
            commands: murmel_tools::commands::commands(),
            palette: None,
            shortcuts_open: false,
            flight: None,
            now: 0.0,
            rotate_axis: murmel_engine::ScreenAxis::Z,
            rotate_deg: 90.0,
            threshold_edit: None,
            histogram: None,
            orbit_pivot: None,
            logo: style::logo_rgba(128).map(|(w, h, rgba)| {
                cc.egui_ctx.load_texture(
                    "murmelcraft-logo",
                    egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
                    egui::TextureOptions::LINEAR,
                )
            }),
        };
        if let Some(p) = open {
            app.open_path(p);
        }
        Ok(app)
    }

    /// Call a tool on the session; a failure goes to the status bar.
    pub(crate) fn call(&mut self, id: &str, input: Value) -> Option<Value> {
        let before = self.session.camera.clone();
        let e = murmel_tools::call(id, input, &mut self.session);
        // Jumps of the view fly there in 250 ms; drags and wheel steps
        // follow the pointer directly.
        if e.ok()
            && matches!(id, "view.standard" | "view.rotate" | "view.fit")
            && before != self.session.camera
        {
            let from = match &self.flight {
                Some((a, t0)) => murmel_engine::Camera::lerp(
                    a,
                    &before,
                    (self.now - t0) / crate::cube::FLIGHT_SECONDS,
                ),
                None => before,
            };
            self.flight = Some((from, self.now));
        }
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

    pub(crate) fn open_dialog(&mut self) {
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

    pub(crate) fn palette(&self, ctx: &egui::Context) -> &murmel_engine::theme::Palette {
        self.session.tokens.palette(self.dark(ctx))
    }

    fn title_bar(&mut self, ui: &mut Ui) {
        let p = self.palette(ui.ctx()).clone();
        let logo = color(self.session.tokens.mesh.logo);
        ui.horizontal_centered(|ui| {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), Sense::hover());
            match &self.logo {
                Some(t) => {
                    ui.painter().image(
                        t.id(),
                        r,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    ui.painter().rect_filled(r, 6.0, logo);
                }
            }
            let name = self.i18n.tr("app-name");
            resp.widget_info(|| WidgetInfo::labeled(Role::Image, true, name.clone()));
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
                let palette = self.i18n.tr("cmd-palette");
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
                let tip = format!("{palette} ({})", crate::commands::key_text("Ctrl+K"));
                if icon_button(ui, icon::SEARCH, &palette, &tip, self.palette.is_some()).clicked() {
                    self.open_palette(String::new());
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

    /// The toolbar: the panel toggles at the right, the groups from the
    /// left. When the window is too narrow for all groups they scroll
    /// sideways under the toggles instead of drawing over them.
    fn toolbar(&mut self, ui: &mut Ui) {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let props = self.i18n.tr("toolbar-properties");
            if icon_button(
                ui,
                icon::INFO,
                &props,
                &self.i18n.tr("toolbar-properties-tip"),
                self.show_props,
            )
            .clicked()
            {
                self.show_props = !self.show_props;
            }
            let tree = self.i18n.tr("toolbar-tree");
            if icon_button(
                ui,
                icon::LAYERS,
                &tree,
                &self.i18n.tr("toolbar-tree-tip"),
                self.show_tree,
            )
            .clicked()
            {
                self.show_tree = !self.show_tree;
            }
            ui.separator();
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("toolbar-groups")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        let size = egui::vec2(0.0, ui.available_height());
                        ui.allocate_ui_with_layout(
                            size,
                            Layout::left_to_right(Align::Center),
                            |ui| self.toolbar_groups(ui),
                        );
                    });
            });
        });
    }

    /// The groups of the toolbar: Open, View, Display, Clip and Select.
    fn toolbar_groups(&mut self, ui: &mut Ui) {
        let secondary = color(self.palette(ui.ctx()).text_secondary);
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
        ui.separator();
        ui.label(
            RichText::new(self.i18n.tr("toolbar-group-display").to_uppercase())
                .small()
                .color(secondary),
        );
        let mode = self.session.display.render_mode;
        let mut pick_mode = None;
        let current = self.i18n.tr(&format!("render-mode-{}", mode.key()));
        let r = egui::ComboBox::from_id_salt("render-mode")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (i, m) in murmel_engine::RenderMode::ALL.into_iter().enumerate() {
                    let name = self.i18n.tr(&format!("render-mode-{}", m.key()));
                    if ui
                        .selectable_label(m == mode, format!("{}  {name}", i + 1))
                        .clicked()
                    {
                        pick_mode = Some(m);
                    }
                }
            });
        r.response
            .on_hover_text(self.i18n.tr("toolbar-render-mode-tip"));
        if let Some(m) = pick_mode.filter(|&m| m != mode) {
            self.call("display.render_mode", json!({ "mode": m }));
        }
        let by = self.session.display.color_by;
        let metric = self.session.display.metric;
        let mut pick = None;
        let label = self.i18n.tr("toolbar-color-by");
        let current = self.color_by_name();
        let r = egui::ComboBox::from_id_salt("color-by")
            .selected_text(format!("{label}: {current}"))
            .show_ui(ui, |ui| {
                for c in murmel_engine::ColorBy::ALL {
                    if c == murmel_engine::ColorBy::Quality {
                        continue;
                    }
                    let name = self.i18n.tr(&format!("color-by-{}", c.key()));
                    if ui.selectable_label(c == by, name).clicked() {
                        pick = Some(json!({ "by": c }));
                    }
                }
                ui.separator();
                let quality = self.i18n.tr("color-by-quality");
                for m in murmel_engine::Metric::ALL {
                    let name = format!(
                        "{quality}: {}",
                        self.i18n.tr(&format!("metric-{}", m.key()))
                    );
                    let on = by == murmel_engine::ColorBy::Quality && m == metric;
                    if ui.selectable_label(on, name).clicked() && !on {
                        pick = Some(json!({"by": "quality", "metric": m}));
                    }
                }
            });
        r.response
            .on_hover_text(self.i18n.tr("toolbar-color-by-tip"));
        if let Some(input) = pick.filter(|i| i["by"] != json!(by) || i.get("metric").is_some()) {
            self.call("display.color_by", input);
        }
        ui.separator();
        self.clip_group(ui, secondary);
        ui.separator();
        self.select_group(ui, secondary);
    }

    /// The name of the colouring, with the metric in colour mode quality.
    pub(crate) fn color_by_name(&self) -> String {
        let d = &self.session.display;
        let by = self.i18n.tr(&format!("color-by-{}", d.color_by.key()));
        if d.color_by == murmel_engine::ColorBy::Quality {
            format!(
                "{by}: {}",
                self.i18n.tr(&format!("metric-{}", d.metric.key()))
            )
        } else {
            by
        }
    }

    /// The Clip group of the toolbar: the axis buttons (X, Y, Z), the
    /// cut mode, flip and the plane's position.
    fn clip_group(&mut self, ui: &mut Ui, secondary: Color32) {
        ui.label(
            RichText::new(self.i18n.tr("toolbar-group-clip").to_uppercase())
                .small()
                .color(secondary),
        );
        let clip = self.session.display.clip;
        let has_mesh = self.session.mesh.is_some();
        ui.add_enabled_ui(has_mesh, |ui| {
            let on = clip.is_some_and(|c| c.enabled);
            for (axis, n) in [("x", 0), ("y", 1), ("z", 2)] {
                let active = on && clip.is_some_and(|c| c.normal[n].abs() > 0.999);
                let name = self.i18n.tr(&format!("cmd-clip-{axis}"));
                let b = ui
                    .add(
                        egui::Button::new(RichText::new(axis.to_uppercase()).size(13.0))
                            .selected(active)
                            .min_size(egui::vec2(28.0, 28.0)),
                    )
                    .on_hover_text(format!("{name} ({})", axis.to_uppercase()));
                b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
                if b.clicked() {
                    if active {
                        self.call("clip.set", json!({"enabled": false}));
                    } else {
                        self.call("clip.set", json!({ "axis": axis }));
                    }
                }
            }
            let Some(c) = clip else {
                return;
            };
            let mode = c.mode;
            let mut pick = None;
            let r = egui::ComboBox::from_id_salt("clip-mode")
                .selected_text(self.i18n.tr(&format!("clip-mode-{}", mode.key())))
                .show_ui(ui, |ui| {
                    for m in murmel_engine::ClipMode::ALL {
                        let name = self.i18n.tr(&format!("clip-mode-{}", m.key()));
                        if ui.selectable_label(m == mode, name).clicked() {
                            pick = Some(m);
                        }
                    }
                });
            r.response
                .on_hover_text(self.i18n.tr("toolbar-clip-mode-tip"));
            if let Some(m) = pick.filter(|&m| m != mode) {
                self.call("clip.set", json!({ "mode": m }));
            }
            let flip = self.i18n.tr("toolbar-clip-flip");
            if ui
                .button(flip)
                .on_hover_text(self.i18n.tr("toolbar-clip-flip-tip"))
                .clicked()
            {
                self.call("clip.set", json!({"flip": true}));
            }
            if let Some(m) = &self.session.mesh {
                let (off, [lo, hi]) = c.offset(&m.mesh.bounds());
                let mut v = off;
                let name = self.i18n.tr("toolbar-clip-position");
                let r = ui
                    .add_sized(
                        [120.0, 20.0],
                        egui::Slider::new(&mut v, lo..=hi).show_value(false),
                    )
                    .on_hover_text(self.i18n.tr("toolbar-clip-position-tip"));
                r.widget_info(|| WidgetInfo::slider(true, v, name.clone()));
                if r.changed() && v != off {
                    self.call("clip.set", json!({ "offset": v }));
                }
            }
        });
    }

    /// The Select group: what a click picks (cells or faces).
    fn select_group(&mut self, ui: &mut Ui, secondary: Color32) {
        ui.label(
            RichText::new(self.i18n.tr("toolbar-group-select").to_uppercase())
                .small()
                .color(secondary),
        );
        let now = self.session.display.pick;
        for t in murmel_engine::PickTarget::ALL {
            let label = self.i18n.tr(&format!("toolbar-pick-{}", t.key()));
            let tip = self.i18n.tr(&format!("cmd-pick-{}", t.key()));
            let b = ui
                .add(egui::Button::new(RichText::new(&label).size(13.0)).selected(now == t))
                .on_hover_text(&tip);
            b.widget_info(|| WidgetInfo::selected(Role::Button, true, now == t, tip.clone()));
            if b.clicked() && now != t {
                self.call("select.filter", json!({ "target": t }));
            }
        }
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
                if let Some(p) = self
                    .hover
                    .as_ref()
                    .and_then(|h| h.hit.as_ref())
                    .and_then(|h| h["point"].as_array())
                {
                    let c = |k: usize| {
                        format!("{:.4}", p.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0))
                    };
                    ui.label(
                        RichText::new(self.i18n.tr_args(
                            "status-cursor",
                            &[("x", c(0).into()), ("y", c(1).into()), ("z", c(2).into())],
                        ))
                        .monospace()
                        .color(secondary),
                    );
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
                    let s = (m.surface.cells, m.surface.faces);
                    let sel = &self.session.display.selection;
                    if sel.len() > 1 {
                        ui.label(
                            RichText::new(
                                self.i18n.tr_args(
                                    "status-selected-many",
                                    &[("count", sel.len().into())],
                                ),
                            )
                            .small(),
                        );
                    } else if let Some(&sel) = sel.first() {
                        let id = sel.id(&m.mesh);
                        let (area, name) = id.split_once(':').unwrap_or(("mesh", ""));
                        let name = if area == "kind" {
                            self.i18n.tr(&format!("kind-{name}"))
                        } else {
                            name.to_string()
                        };
                        let area = self.i18n.tr(&format!("props-area-{area}"));
                        ui.label(
                            RichText::new(self.i18n.tr_args(
                                "status-selected",
                                &[("area", area.into()), ("item", name.into())],
                            ))
                            .small(),
                        );
                    }
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
                ui.add_space(8.0);
                ui.heading(self.i18n.tr("settings-colors"));
                ui.horizontal(|ui| {
                    let current = self.session.tokens.mesh.solid;
                    let mut rgb = current.0;
                    let name = self.i18n.tr("settings-mesh-color");
                    let r = egui::color_picker::color_edit_button_srgb(ui, &mut rgb);
                    r.widget_info(|| WidgetInfo::labeled(Role::ColorWell, true, name.clone()));
                    ui.label(&name);
                    if rgb != current.0 {
                        let hex = murmel_engine::Rgb(rgb).hex();
                        self.call("settings.set_colors", json!({ "solid": hex }));
                    }
                });
                if self.session.colors != murmel_engine::MeshColors::default()
                    && ui.button(self.i18n.tr("settings-reset-colors")).clicked()
                {
                    self.call("settings.set_colors", json!({"reset": true}));
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
        self.now = ctx.input(|i| i.time);
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
        if self.show_tree {
            egui::Panel::left("tree")
                .default_size(264.0)
                .size_range(200.0..=480.0)
                .resizable(true)
                .frame(
                    egui::Frame::NONE
                        .fill(color(p.bg_panel))
                        .inner_margin(egui::Margin::symmetric(10, 0)),
                )
                .show(ui, |ui| self.model_tree(ui));
        }
        if self.show_props {
            egui::Panel::right("properties")
                .default_size(304.0)
                .size_range(220.0..=520.0)
                .resizable(true)
                .frame(
                    egui::Frame::NONE
                        .fill(color(p.bg_panel))
                        .inner_margin(egui::Margin::symmetric(12, 0)),
                )
                .show(ui, |ui| self.properties(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(color(p.viewport_bottom)))
            .show(ui, |ui| self.viewport(ui, frame));
        self.settings_window(&ctx);
        self.shortcuts_window(&ctx);
        self.palette_window(&ctx);
    }
}
