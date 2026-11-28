//! Kraxn's window: a wizard over the same tools as the text mode.
//! Welcome → for whom → where from → which programs → options → the
//! steps → running and the result; and a page to change the default or
//! remove installed versions. Network and install work runs on a thread;
//! the window only shows what it reports.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use egui::{Align, Layout, RichText};
use kraxn_core::install::Event;
use kraxn_core::tools::Envelope;
use kraxn_core::{Origin, Plan, Scope, Want};
use serde_json::{Value, json};

use crate::i18n::I18n;
use crate::kit;
use crate::logos::Logos;
use crate::runner::Runner;
use crate::style::{self, Tokens, icon};
use crate::text::{done_source, file_types_list, tool_line};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Welcome,
    Scope,
    Source,
    Products,
    Options,
    Review,
    Running,
    Manage,
}

/// A message from a background job.
enum Msg {
    Event(Event),
    Done(Envelope),
}

/// A tool call on a thread.
struct Job {
    rx: Receiver<Msg>,
    events: Vec<Event>,
    done: Option<Envelope>,
}

impl Job {
    fn start(ctx: &egui::Context, runner: &Arc<Mutex<Runner>>, id: &str, input: Value) -> Self {
        let (tx, rx) = channel();
        let (ctx, runner, id) = (ctx.clone(), runner.clone(), id.to_string());
        std::thread::spawn(move || {
            let e = {
                let mut r = match runner.lock() {
                    Ok(r) => r,
                    Err(p) => p.into_inner(),
                };
                let tx2 = tx.clone();
                let ctx2 = ctx.clone();
                r.call(&id, &input, &mut |ev| {
                    let _ = tx2.send(Msg::Event(ev));
                    ctx2.request_repaint();
                })
            };
            let _ = tx.send(Msg::Done(e));
            ctx.request_repaint();
        });
        Self {
            rx,
            events: Vec::new(),
            done: None,
        }
    }

    fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Event(e) => self.events.push(e),
                Msg::Done(e) => self.done = Some(e),
            }
        }
    }
}

/// One product on the Programs page.
struct Row {
    id: String,
    title: String,
    chosen: bool,
    /// Versions with a file for this system, newest first.
    versions: Vec<String>,
    /// Every version that is not a pre-release (for a source build).
    all_versions: Vec<String>,
    /// Can be built from the source (online, with build paths).
    buildable: bool,
    from_source: bool,
    /// The catalogue's features and whether each is chosen.
    features: Vec<(String, bool)>,
    version: String,
    make_default: bool,
    installed: Vec<String>,
    job: Option<Job>,
    error: Option<String>,
}

pub struct KraxnApp {
    pub t: I18n,
    pub runner: Arc<Mutex<Runner>>,
    pub page: Page,
    pub scope: Scope,
    pub folder: String,
    pub offline: bool,
    pub offline_dir: String,
    rows: Vec<Row>,
    pub path: bool,
    /// Source builds for this computer's processor.
    pub native: bool,
    pub menu: bool,
    pub file_types: bool,
    preview: Option<Job>,
    run: Option<Job>,
    installed: Option<Value>,
    manage_job: Option<Job>,
    note: Option<String>,
    pub plan_file: String,
    logos: Option<Logos>,
}

impl KraxnApp {
    pub fn new(cc: &eframe::CreationContext<'_>, runner: Runner) -> Result<Self, String> {
        style::install(&cc.egui_ctx, &Tokens::builtin()?);
        Self::headless(runner)
    }

    /// Without a window (tests set the style themselves).
    pub fn headless(runner: Runner) -> Result<Self, String> {
        let rows = runner
            .catalogue
            .products
            .iter()
            .map(|p| Row {
                id: p.id.clone(),
                title: p.title.clone(),
                chosen: true,
                versions: Vec::new(),
                all_versions: Vec::new(),
                buildable: false,
                from_source: false,
                features: p.features.iter().map(|f| (f.clone(), false)).collect(),
                version: "latest".into(),
                make_default: true,
                installed: Vec::new(),
                job: None,
                error: None,
            })
            .collect();
        Ok(Self {
            t: I18n::english()?,
            runner: Arc::new(Mutex::new(runner)),
            page: Page::Welcome,
            scope: Scope::User,
            folder: String::new(),
            offline: false,
            offline_dir: String::new(),
            rows,
            path: true,
            native: false,
            menu: true,
            file_types: true,
            preview: None,
            run: None,
            installed: None,
            manage_job: None,
            note: None,
            plan_file: String::new(),
            logos: None,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Runner> {
        match self.runner.lock() {
            Ok(r) => r,
            Err(p) => p.into_inner(),
        }
    }

    fn prefix(&self) -> Option<PathBuf> {
        let f = self.folder.trim();
        (!f.is_empty()).then(|| PathBuf::from(f))
    }

    fn default_root(&self, scope: Scope) -> String {
        let r = self.lock();
        kraxn_core::Layout::new(scope, r.platform, &r.env, None)
            .map(|l| l.root.display().to_string())
            .unwrap_or_default()
    }

    /// The plan the pages describe.
    pub fn plan(&self) -> Plan {
        Plan {
            scope: self.scope,
            prefix: self.prefix(),
            offline_dir: self.offline.then(|| PathBuf::from(self.offline_dir.trim())),
            path: self.path,
            menu: self.menu,
            file_types: self.file_types,
            installs: self
                .rows
                .iter()
                .filter(|r| r.chosen)
                .map(|r| {
                    let source = r.from_source && r.buildable;
                    Want {
                        version: r.version.clone(),
                        make_default: r.make_default,
                        from: if source {
                            Origin::Source
                        } else {
                            Origin::Release
                        },
                        features: if source {
                            r.features
                                .iter()
                                .filter(|f| f.1)
                                .map(|f| f.0.clone())
                                .collect()
                        } else {
                            Vec::new()
                        },
                        native: source && self.native,
                        ..Want::new(&r.id)
                    }
                })
                .collect(),
            ..Plan::default()
        }
    }

    fn installed_list(&self) -> Value {
        let mut input = json!({"scope": self.scope});
        if let Some(p) = self.prefix() {
            input["prefix"] = json!(p);
        }
        self.lock().quiet("installed.list", &input).result
    }

    /// Fetches the release lists (on threads) and what is installed.
    fn load_products(&mut self, ctx: &egui::Context) {
        let have = self.installed_list();
        let offline = self.offline.then(|| self.offline_dir.trim().to_string());
        let catalogue = self.lock().catalogue.clone();
        for row in &mut self.rows {
            row.buildable = offline.is_none()
                && catalogue
                    .get(&row.id)
                    .is_ok_and(|p| !p.build_paths.is_empty());
            row.installed = have["products"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|i| i["product"] == row.id.as_str())
                .flat_map(|i| i["versions"].as_array().cloned().unwrap_or_default())
                .filter_map(|v| v["version"].as_str().map(str::to_string))
                .collect();
            row.versions.clear();
            row.error = None;
            let mut input = json!({"product": row.id});
            if let Some(d) = &offline {
                input["offline_dir"] = json!(d);
            }
            row.job = Some(Job::start(ctx, &self.runner, "release.list", input));
        }
    }

    fn poll(&mut self) {
        for row in &mut self.rows {
            let Some(job) = &mut row.job else { continue };
            job.poll();
            let Some(e) = job.done.take() else { continue };
            row.job = None;
            if e.ok() {
                let rels = e.result["releases"].as_array().cloned().unwrap_or_default();
                let version = |r: &Value| r["version"].as_str().map(str::to_string);
                row.versions = rels
                    .iter()
                    .filter(|r| r["file"].is_string())
                    .filter_map(version)
                    .collect();
                row.all_versions = rels
                    .iter()
                    .filter(|r| !r["prerelease"].as_bool().unwrap_or(false))
                    .filter_map(version)
                    .collect();
                if row.versions.is_empty() {
                    // nothing to download: only a build, and only if asked
                    row.chosen = false;
                    row.from_source = true;
                }
            } else {
                row.chosen = false;
                // a folder without this product is not an error, just empty
                row.error = e
                    .diagnostics
                    .first()
                    .filter(|d| d.code != "KRX012")
                    .map(|d| d.to_string());
            }
        }
        for j in [&mut self.preview, &mut self.run, &mut self.manage_job]
            .into_iter()
            .flatten()
        {
            j.poll();
        }
    }

    /// The steps' preview is still being made.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn review_busy(&self) -> bool {
        self.preview.as_ref().is_none_or(|j| j.done.is_none())
    }

    /// The install has ended (well or not).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn run_finished(&self) -> bool {
        self.run.as_ref().is_some_and(|j| j.done.is_some())
    }

    /// A change on the manage page is running.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn manage_busy(&self) -> bool {
        self.manage_job.is_some()
    }

    fn loading(&self) -> bool {
        self.rows.iter().any(|r| r.job.is_some())
    }

    /// The release lists are still being fetched.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn products_loading(&self) -> bool {
        self.loading()
    }

    /// The steps' check is done, fine, and no tool blocks the build.
    fn review_ok(&self) -> bool {
        self.preview
            .as_ref()
            .and_then(|j| j.done.as_ref())
            .is_some_and(|e| e.ok() && !e.result["blocked"].as_bool().unwrap_or(false))
    }

    fn any_source(&self) -> bool {
        self.rows
            .iter()
            .any(|r| r.chosen && r.from_source && r.buildable)
    }

    fn can_next(&self) -> bool {
        match self.page {
            Page::Source => !self.offline || PathBuf::from(self.offline_dir.trim()).is_dir(),
            Page::Products => !self.loading() && self.rows.iter().any(|r| r.chosen),
            Page::Scope => self.prefix().is_none_or(|p| p.is_absolute()),
            _ => true,
        }
    }

    fn go(&mut self, ctx: &egui::Context, page: Page) {
        self.note = None;
        match page {
            Page::Products => self.load_products(ctx),
            Page::Review => {
                self.preview = Some(Job::start(
                    ctx,
                    &self.runner,
                    "plan.check",
                    json!({"plan": self.plan()}),
                ));
            }
            Page::Running => {
                self.run = Some(Job::start(
                    ctx,
                    &self.runner,
                    "plan.apply",
                    json!({"plan": self.plan()}),
                ));
            }
            Page::Manage => self.installed = Some(self.installed_list()),
            _ => {}
        }
        self.page = page;
    }

    fn next_page(&self) -> Option<Page> {
        Some(match self.page {
            Page::Scope => Page::Source,
            Page::Source => Page::Products,
            Page::Products => Page::Options,
            Page::Options => Page::Review,
            Page::Review => Page::Running,
            _ => return None,
        })
    }

    fn back_page(&self) -> Option<Page> {
        Some(match self.page {
            Page::Scope | Page::Manage => Page::Welcome,
            Page::Source => Page::Scope,
            Page::Products => Page::Source,
            Page::Options => Page::Products,
            Page::Review => Page::Options,
            _ => return None,
        })
    }

    /// The side panel: Kraxn's logo and name, the wizard's steps, the
    /// theme switch.
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            kit::logo(ui, self.logos.as_ref().map(|l| &l.kraxn), 44.0, "Kraxn");
            ui.vertical(|ui| {
                ui.label(kit::bold(self.t.tr("app-title")).size(22.0));
                ui.label(RichText::new(self.t.tr("win-installer")).color(kit::secondary_text(ui)));
            });
        });
        ui.add_space(28.0);
        let steps = [
            (Page::Scope, "win-step-scope"),
            (Page::Source, "win-step-source"),
            (Page::Products, "win-step-products"),
            (Page::Options, "win-step-options"),
            (Page::Review, "win-step-review"),
            (Page::Running, "win-step-running"),
        ];
        let now = steps.iter().position(|(p, _)| *p == self.page);
        if let Some(now) = now {
            for (i, (_, key)) in steps.iter().enumerate() {
                let finished = self.page == Page::Running && self.run_finished();
                let mark = if i < now || finished {
                    kit::Mark::Done
                } else if i == now {
                    kit::Mark::Now
                } else {
                    kit::Mark::Later
                };
                kit::step_row(ui, i + 1, &self.t.tr(key), mark);
            }
        } else {
            for (id, title) in self.product_titles() {
                ui.horizontal(|ui| {
                    let tex = self.logos.as_ref().and_then(|l| l.products.get(&id));
                    kit::logo(ui, tex, 28.0, &title);
                    ui.label(&title);
                });
                ui.add_space(4.0);
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(16.0);
            ui.label(
                RichText::new(format!("Kraxn {}", env!("CARGO_PKG_VERSION")))
                    .small()
                    .color(kit::secondary_text(ui)),
            );
            let dark = ui.visuals().dark_mode;
            let (glyph, key) = if dark {
                (icon::SUN, "win-theme-light")
            } else {
                (icon::MOON, "win-theme-dark")
            };
            let name = self.t.tr(key);
            let r = ui
                .button(format!("{glyph}  {name}"))
                .on_hover_text(name.clone());
            r.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, &name));
            if r.clicked() {
                ui.ctx().set_theme(if dark {
                    egui::Theme::Light
                } else {
                    egui::Theme::Dark
                });
            }
        });
    }

    fn product_titles(&self) -> Vec<(String, String)> {
        self.rows
            .iter()
            .map(|r| (r.id.clone(), r.title.clone()))
            .collect()
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let (label, files) = {
            let r = self.lock();
            (r.platform.label(), r.platform.asset_key().to_string())
        };
        ui.add_space(8.0);
        kit::title(
            ui,
            &self.t.tr("win-welcome-title"),
            Some(&self.t.tr("win-welcome-text")),
        );
        let products = self.product_titles();
        let logos = self.logos.as_ref();
        ui.columns(products.len().max(1), |cols| {
            for ((id, title), ui) in products.iter().zip(cols.iter_mut()) {
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    kit::card(ui, false, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_min_height(212.0);
                        let tex = logos.and_then(|l| l.products.get(id));
                        kit::logo(ui, tex, 72.0, title);
                        ui.add_space(10.0);
                        ui.label(kit::bold(title).size(17.0));
                        ui.label(
                            RichText::new(self.t.tr(&format!("win-role-{id}")))
                                .color(kit::accent(ui)),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(self.t.tr(&format!("win-about-{id}")))
                                .color(kit::secondary_text(ui)),
                        );
                    });
                });
            }
        });
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            if kit::primary(ui, true, &self.t.tr("menu-install")).clicked() {
                self.go(ui.ctx(), Page::Scope);
            }
            ui.add_space(8.0);
            if kit::secondary(ui, true, &self.t.tr("win-manage")).clicked() {
                self.go(ui.ctx(), Page::Manage);
            }
        });
        ui.add_space(20.0);
        ui.label(
            RichText::new(
                self.t
                    .tr_args("app-system", &[("system", label), ("files", files)]),
            )
            .small()
            .color(kit::secondary_text(ui)),
        );
    }

    fn scope_page(&mut self, ui: &mut egui::Ui) {
        kit::title(
            ui,
            &self.t.tr("scope-title"),
            Some(&self.t.tr("win-sub-scope")),
        );
        let (ru, rs) = (
            self.default_root(Scope::User),
            self.default_root(Scope::System),
        );
        kit::choice(
            ui,
            &mut self.scope,
            Scope::User,
            &self.t.tr("win-scope-user-title"),
            &self
                .t
                .tr_args("win-scope-user-text", &[("root", ru.clone())]),
        );
        kit::choice(
            ui,
            &mut self.scope,
            Scope::System,
            &self.t.tr("win-scope-system-title"),
            &self
                .t
                .tr_args("win-scope-system-text", &[("root", rs.clone())]),
        );
        ui.add_space(8.0);
        let root = if self.scope == Scope::User { ru } else { rs };
        let label = ui.label(kit::bold(self.t.tr("win-folder")));
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.folder)
                    .hint_text(root)
                    .desired_width(420.0)
                    .margin(egui::vec2(8.0, 6.0)),
            )
            .labelled_by(label.id);
            if kit::secondary(ui, true, &self.t.tr("win-choose")).clicked()
                && let Some(d) = rfd::FileDialog::new().pick_folder()
            {
                self.folder = d.display().to_string();
            }
        });
    }

    fn source_page(&mut self, ui: &mut egui::Ui) {
        kit::title(
            ui,
            &self.t.tr("source-title"),
            Some(&self.t.tr("win-sub-source")),
        );
        kit::choice(
            ui,
            &mut self.offline,
            false,
            &self.t.tr("source-online"),
            &self.t.tr("win-source-online-text"),
        );
        kit::choice(
            ui,
            &mut self.offline,
            true,
            &self.t.tr("source-folder"),
            &self.t.tr("win-source-folder-text"),
        );
        if self.offline {
            ui.add_space(8.0);
            let label = ui.label(kit::bold(self.t.tr("source-folder-ask")));
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.offline_dir)
                        .desired_width(420.0)
                        .margin(egui::vec2(8.0, 6.0)),
                )
                .labelled_by(label.id);
                if kit::secondary(ui, true, &self.t.tr("win-choose")).clicked()
                    && let Some(d) = rfd::FileDialog::new().pick_folder()
                {
                    self.offline_dir = d.display().to_string();
                }
            });
            if !self.offline_dir.trim().is_empty()
                && !PathBuf::from(self.offline_dir.trim()).is_dir()
            {
                ui.label(
                    RichText::new(self.t.tr("source-folder-missing"))
                        .color(ui.visuals().error_fg_color),
                );
            }
        }
    }

    fn products_page(&mut self, ui: &mut egui::Ui) {
        kit::title(
            ui,
            &self.t.tr("products-title"),
            Some(&self.t.tr("win-sub-products")),
        );
        if self.loading() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(self.t.tr("win-loading"));
            });
        }
        let latest_word = self.t.tr("win-latest");
        let default_word = self.t.tr("win-make-default");
        let source_word = self.t.tr("win-from-source");
        let logos = self.logos.as_ref();
        for row in &mut self.rows {
            let selected = row.chosen;
            kit::card(ui, selected, |ui| {
                ui.set_width(ui.available_width());
                let has_file = !row.versions.is_empty();
                let can_build = row.buildable && !row.all_versions.is_empty();
                let enabled = has_file || can_build;
                ui.horizontal_top(|ui| {
                    kit::logo(
                        ui,
                        logos.and_then(|l| l.products.get(&row.id)),
                        56.0,
                        &row.title,
                    );
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            ui.checkbox(&mut row.chosen, kit::bold(&row.title).size(16.0));
                        });
                        ui.label(
                            RichText::new(self.t.tr(&format!("win-about-{}", row.id)))
                                .color(kit::secondary_text(ui)),
                        );
                        if let Some(e) = &row.error {
                            ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
                        } else if row.job.is_none() && !has_file {
                            let text = if can_build {
                                self.t.tr_args(
                                    "products-buildable",
                                    &[(
                                        "version",
                                        row.all_versions.first().cloned().unwrap_or_default(),
                                    )],
                                )
                            } else {
                                self.t.tr("products-no-file")
                            };
                            ui.label(RichText::new(text).color(ui.visuals().warn_fg_color));
                        }
                        if row.chosen && can_build && has_file {
                            ui.checkbox(&mut row.from_source, source_word.clone());
                        }
                        let source = row.from_source && can_build;
                        if row.chosen && source {
                            for (f, on) in &mut row.features {
                                let about = self.t.tr(&format!("feature-{f}"));
                                ui.checkbox(on, format!("{f}: {about}"));
                            }
                        }
                        if !row.installed.is_empty() {
                            ui.label(self.t.tr_args(
                                "win-installed",
                                &[("versions", row.installed.join(", "))],
                            ));
                        }
                        if row.chosen && enabled {
                            let list = if source {
                                row.all_versions.clone()
                            } else {
                                row.versions.clone()
                            };
                            if row.version != "latest" && !list.contains(&row.version) {
                                row.version = "latest".into();
                            }
                            ui.horizontal(|ui| {
                                let shown = if row.version == "latest" {
                                    format!(
                                        "{latest_word} ({})",
                                        list.first().cloned().unwrap_or_default()
                                    )
                                } else {
                                    row.version.clone()
                                };
                                let label = ui.label(self.t.tr("win-version"));
                                egui::ComboBox::from_id_salt(format!("version-{}", row.id))
                                    .selected_text(shown)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut row.version,
                                            "latest".to_string(),
                                            latest_word.clone(),
                                        );
                                        for v in &list {
                                            ui.selectable_value(
                                                &mut row.version,
                                                v.clone(),
                                                v.clone(),
                                            );
                                        }
                                    })
                                    .response
                                    .labelled_by(label.id);
                                if !row.installed.is_empty() {
                                    ui.checkbox(&mut row.make_default, default_word.clone());
                                }
                            });
                        }
                    });
                });
            });
            ui.add_space(10.0);
        }
    }

    fn options_page(&mut self, ui: &mut egui::Ui) {
        kit::title(
            ui,
            &self.t.tr("win-options"),
            Some(&self.t.tr("win-sub-options")),
        );
        ui.checkbox(&mut self.path, self.t.tr("path-ask"));
        if self.any_source() {
            ui.checkbox(&mut self.native, self.t.tr("win-native"));
        }
        let windows: Vec<kraxn_core::Product> = {
            let r = self.lock();
            self.rows
                .iter()
                .filter(|row| row.chosen)
                .filter_map(|row| r.catalogue.get(&row.id).ok())
                .filter(|p| p.window.is_some())
                .cloned()
                .collect()
        };
        for p in windows {
            ui.checkbox(
                &mut self.menu,
                self.t.tr_args("win-menu", &[("title", p.title.clone())]),
            );
            if !p.file_types.is_empty() {
                ui.add_enabled(
                    self.menu,
                    egui::Checkbox::new(
                        &mut self.file_types,
                        self.t.tr_args(
                            "win-file-types",
                            &[("title", p.title.clone()), ("types", file_types_list(&p))],
                        ),
                    ),
                );
            }
        }
        ui.label(
            RichText::new(self.t.tr("win-more-later"))
                .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
        );
    }

    fn steps_ui(&self, ui: &mut egui::Ui, steps: &Value) {
        let admin = self.t.tr("steps-admin");
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 120.0)
            .show(ui, |ui| {
                for s in steps.as_array().into_iter().flatten() {
                    let title = s["title"].as_str().unwrap_or("");
                    let mut head = kit::bold(title);
                    if s["admin"].as_bool().unwrap_or(false) {
                        head = kit::bold(format!("{title}  {admin}"))
                            .color(ui.visuals().warn_fg_color);
                    }
                    egui::CollapsingHeader::new(head)
                        .id_salt(title)
                        .default_open(false)
                        .show(ui, |ui| {
                            for a in s["actions"].as_array().into_iter().flatten() {
                                ui.monospace(a.as_str().unwrap_or(""));
                            }
                        });
                }
            });
    }

    fn review_page(&mut self, ui: &mut egui::Ui) {
        kit::title(
            ui,
            &self.t.tr("steps-title"),
            Some(&self.t.tr("win-sub-review")),
        );
        let Some(job) = &self.preview else { return };
        let Some(e) = job.done.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(self.t.tr("win-loading"));
            });
            return;
        };
        if !e.ok() {
            let m = e
                .diagnostics
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            ui.label(
                RichText::new(self.t.tr_args("done-failed", &[("message", m)]))
                    .color(ui.visuals().error_fg_color),
            );
            return;
        }
        if let Some(tools) = e.result["tools"].as_array().filter(|t| !t.is_empty()) {
            ui.label(kit::bold(self.t.tr("tools-title")));
            for t in tools {
                let mut rt = RichText::new(tool_line(&self.t, t));
                if !t["ok"].as_bool().unwrap_or(false) && !t["fix"].is_object() {
                    rt = rt.color(ui.visuals().error_fg_color);
                }
                ui.label(rt);
            }
            if e.result["blocked"].as_bool().unwrap_or(false) {
                ui.label(
                    RichText::new(self.t.tr("tools-blocked")).color(ui.visuals().warn_fg_color),
                );
                if ui.button(self.t.tr("win-check-again")).clicked() {
                    self.go(ui.ctx(), Page::Review);
                    return;
                }
            }
            ui.add_space(8.0);
        }
        self.steps_ui(ui, &e.result["steps"]);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let label = ui.label(self.t.tr("win-plan-file"));
            ui.add(egui::TextEdit::singleline(&mut self.plan_file).desired_width(320.0))
                .labelled_by(label.id);
            if ui.button(self.t.tr("win-save-plan")).clicked() {
                if self.plan_file.trim().is_empty()
                    && let Some(f) = rfd::FileDialog::new()
                        .set_file_name("kraxn-plan.toml")
                        .save_file()
                {
                    self.plan_file = f.display().to_string();
                }
                let f = self.plan_file.trim().to_string();
                if !f.is_empty() {
                    self.note = Some(match std::fs::write(&f, self.plan().to_toml()) {
                        Ok(()) => self.t.tr_args("saved", &[("file", f)]),
                        Err(err) => self
                            .t
                            .tr_args("done-failed", &[("message", format!("{f}: {err}"))]),
                    });
                }
            }
        });
    }

    fn running_page(&mut self, ui: &mut egui::Ui) {
        let Some(job) = &self.run else { return };
        let mut downloads: Vec<(String, u64, u64)> = Vec::new();
        let mut infos: Vec<String> = Vec::new();
        let mut log: Vec<&str> = Vec::new();
        for e in &job.events {
            match e {
                Event::Download { file, done, total } => {
                    match downloads.iter_mut().find(|d| &d.0 == file) {
                        Some(d) => {
                            d.1 = *done;
                            d.2 = *total;
                        }
                        None => downloads.push((file.clone(), *done, *total)),
                    }
                }
                Event::Info { text } => infos.push(text.clone()),
                Event::Log { line } => log.push(line),
                Event::Step { title, ok } => infos.push(self.t.tr_args(
                    if *ok {
                        "run-step-ok"
                    } else {
                        "run-step-failed"
                    },
                    &[("title", title.clone())],
                )),
            }
        }
        match &job.done {
            None => kit::title(
                ui,
                &self.t.tr("win-running"),
                Some(&self.t.tr("win-sub-running")),
            ),
            Some(e) if e.ok() => {
                kit::title(ui, &self.t.tr("win-done"), Some(&self.t.tr("win-sub-done")))
            }
            Some(_) => {
                ui.label(
                    kit::bold(self.t.tr("win-failed"))
                        .size(26.0)
                        .color(ui.visuals().error_fg_color),
                );
                ui.label(RichText::new(self.t.tr("win-sub-failed")).color(kit::secondary_text(ui)));
                ui.add_space(16.0);
            }
        }
        for (file, done, total) in &downloads {
            let f = if *total > 0 {
                *done as f32 / *total as f32
            } else {
                0.0
            };
            ui.add(
                egui::ProgressBar::new(f)
                    .text(file.clone())
                    .show_percentage(),
            );
        }
        kit::card(ui, false, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for i in &infos {
                        ui.label(RichText::new(i).color(kit::secondary_text(ui)));
                    }
                });
        });
        ui.add_space(8.0);
        if !log.is_empty() {
            let building = job.done.is_none();
            egui::CollapsingHeader::new(self.t.tr("win-build-log"))
                .default_open(building)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("build-log")
                        .max_height(180.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for l in &log[log.len().saturating_sub(500)..] {
                                ui.monospace(*l);
                            }
                        });
                });
        }
        let Some(e) = &job.done else {
            ui.spinner();
            return;
        };
        ui.separator();
        let report = &e.result;
        for p in report["products"].as_array().into_iter().flatten() {
            let title = self
                .lock()
                .catalogue
                .get(p["product"].as_str().unwrap_or(""))
                .map(|x| x.title.clone())
                .unwrap_or_default();
            let v = p["version"].as_str().unwrap_or("").to_string();
            if p["already"].as_bool().unwrap_or(false) {
                ui.label(
                    self.t
                        .tr_args("done-already", &[("title", title), ("version", v)]),
                );
                continue;
            }
            ui.label(kit::bold(self.t.tr_args(
                "done-version",
                &[
                    ("title", title),
                    ("version", v),
                    ("dir", p["dir"].as_str().unwrap_or("").into()),
                ],
            )));
            ui.label(format!("    {}", done_source(&self.t, p)));
            if let Some(c) = p["check"].as_object() {
                let ok = c["ok"].as_bool().unwrap_or(false);
                let text = self.t.tr_args(
                    "done-started",
                    &[
                        ("command", c["command"].as_str().unwrap_or("").into()),
                        ("message", c["message"].as_str().unwrap_or("").into()),
                    ],
                );
                let mut rt = RichText::new(format!("    {text}"));
                if !ok {
                    rt = rt.color(ui.visuals().error_fg_color);
                }
                ui.label(rt);
            }
        }
        for n in report["notes"].as_array().into_iter().flatten() {
            ui.label(n.as_str().unwrap_or(""));
        }
        if !e.ok() {
            let m = e
                .diagnostics
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            ui.label(
                RichText::new(self.t.tr_args("done-failed", &[("message", m)]))
                    .color(ui.visuals().error_fg_color),
            );
        }
    }

    fn manage_page(&mut self, ui: &mut egui::Ui) {
        kit::title(
            ui,
            &self.t.tr("win-manage"),
            Some(&self.t.tr("win-sub-manage")),
        );
        let before = self.scope;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (sc, key) in [
                (Scope::User, "win-manage-user"),
                (Scope::System, "win-manage-system"),
            ] {
                kit::segment(ui, &mut self.scope, sc, &self.t.tr(key));
            }
        });
        ui.add_space(12.0);
        if before != self.scope {
            self.installed = Some(self.installed_list());
        }
        if let Some(job) = &self.manage_job
            && let Some(e) = &job.done
        {
            let text = if e.ok() {
                self.t.tr_args("done-ok", &[("what", e.tool.clone())])
            } else {
                let m = e
                    .diagnostics
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("; ");
                self.t.tr_args("done-failed", &[("message", m)])
            };
            self.note = Some(text);
            self.manage_job = None;
            self.installed = Some(self.installed_list());
        }
        let busy = self.manage_job.is_some();
        let list = self
            .installed
            .as_ref()
            .and_then(|v| v["products"].as_array().cloned())
            .unwrap_or_default();
        if list.is_empty() {
            ui.label(self.t.tr("list-none"));
        }
        let mut action: Option<(&str, Value)> = None;
        let logos = self.logos.as_ref();
        for p in &list {
            kit::card(ui, false, |ui| {
                ui.set_width(ui.available_width());
                let title = p["title"].as_str().unwrap_or("").to_string();
                ui.horizontal(|ui| {
                    kit::logo(
                        ui,
                        logos.and_then(|l| l.products.get(p["product"].as_str().unwrap_or(""))),
                        40.0,
                        &title,
                    );
                    ui.add_space(6.0);
                    ui.label(kit::bold(&title).size(17.0));
                });
                ui.add_space(6.0);
                for v in p["versions"].as_array().into_iter().flatten() {
                    let ver = v["version"].as_str().unwrap_or("").to_string();
                    let is_default = v["default"].as_bool().unwrap_or(false);
                    ui.horizontal(|ui| {
                        let d = if is_default {
                            format!(" {}", self.t.tr("list-default"))
                        } else {
                            String::new()
                        };
                        ui.label(self.t.tr_args(
                            "list-row",
                            &[
                                ("title", title.clone()),
                                ("version", ver.clone()),
                                ("default", d),
                            ],
                        ));
                        ui.add_enabled_ui(!busy, |ui| {
                            if !is_default
                                && ui
                                    .button(self.t.tr_args(
                                        "win-make-default-of",
                                        &[("version", ver.clone())],
                                    ))
                                    .clicked()
                            {
                                action = Some((
                                    "version.activate",
                                    json!({"product": p["product"], "version": ver}),
                                ));
                            }
                            if ui
                                .button(
                                    self.t.tr_args("win-remove-of", &[("version", ver.clone())]),
                                )
                                .clicked()
                            {
                                action = Some((
                                    "version.remove",
                                    json!({"product": p["product"], "version": ver}),
                                ));
                            }
                        });
                    });
                }
                ui.add_space(4.0);
                ui.add_enabled_ui(!busy, |ui| {
                    if kit::secondary(
                        ui,
                        true,
                        &self.t.tr_args("remove-all", &[("title", title.clone())]),
                    )
                    .clicked()
                    {
                        action = Some(("version.remove", json!({"product": p["product"]})));
                    }
                });
            });
        }
        if let Some((tool, mut input)) = action {
            input["scope"] = json!(self.scope);
            if let Some(pr) = self.prefix() {
                input["prefix"] = json!(pr);
            }
            self.manage_job = Some(Job::start(ui.ctx(), &self.runner, tool, input));
        }
        if busy {
            ui.spinner();
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let running = self.run.as_ref().is_some_and(|j| j.done.is_none());
            if let Some(b) = self.back_page()
                && kit::secondary(ui, !running, &self.t.tr("win-back")).clicked()
            {
                self.go(ui.ctx(), b);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(n) = self.next_page() {
                    let key = if n == Page::Running {
                        "win-install"
                    } else {
                        "win-next"
                    };
                    let ok = self.can_next() && (n != Page::Running || self.review_ok());
                    if kit::primary(ui, ok, &self.t.tr(key)).clicked() {
                        self.go(ui.ctx(), n);
                    }
                }
                if self.page == Page::Running
                    && self.run.as_ref().is_some_and(|j| j.done.is_some())
                    && kit::primary(ui, true, &self.t.tr("win-close")).clicked()
                {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if let Some(n) = &self.note {
                    ui.label(n);
                }
            });
        });
    }

    /// The window's content, also for tests.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll();
        if self.logos.is_none() {
            self.logos = Logos::load(ui.ctx()).ok();
        }
        let side = ui.visuals().extreme_bg_color;
        egui::Panel::left("steps")
            .exact_size(220.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(side)
                    .inner_margin(egui::Margin::symmetric(20, 0)),
            )
            .show(ui, |ui| self.sidebar(ui));
        if self.page != Page::Welcome {
            egui::Panel::bottom("footer")
                .frame(
                    egui::Frame::new()
                        .fill(ui.visuals().panel_fill)
                        .inner_margin(egui::Margin::symmetric(28, 14)),
                )
                .show(ui, |ui| self.footer(ui));
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(egui::Margin::symmetric(28, 20)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                    Page::Welcome => self.welcome(ui),
                    Page::Scope => self.scope_page(ui),
                    Page::Source => self.source_page(ui),
                    Page::Products => self.products_page(ui),
                    Page::Options => self.options_page(ui),
                    Page::Review => self.review_page(ui),
                    Page::Running => self.running_page(ui),
                    Page::Manage => self.manage_page(ui),
                });
            });
    }
}

impl eframe::App for KraxnApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        KraxnApp::ui(self, ui);
    }
}

/// Opens the window.
pub fn run(runner: Runner) -> Result<(), String> {
    let mut viewport = egui::ViewportBuilder::default();
    if let Some(i) = crate::logos::icon() {
        viewport = viewport.with_icon(i);
    }
    let options = eframe::NativeOptions {
        viewport: viewport
            .with_title("Kraxn")
            .with_inner_size([980.0, 700.0])
            .with_min_inner_size([820.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Kraxn",
        options,
        Box::new(move |cc| Ok(Box::new(KraxnApp::new(cc, runner)?))),
    )
    .map_err(|e| e.to_string())
}
