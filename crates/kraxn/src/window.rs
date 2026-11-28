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
use kraxn_core::{Plan, Scope, Want};
use serde_json::{Value, json};

use crate::i18n::I18n;
use crate::runner::Runner;
use crate::style::{self, Tokens, icon};

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
    preview: Option<Job>,
    run: Option<Job>,
    installed: Option<Value>,
    manage_job: Option<Job>,
    note: Option<String>,
    pub plan_file: String,
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
            preview: None,
            run: None,
            installed: None,
            manage_job: None,
            note: None,
            plan_file: String::new(),
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
            installs: self
                .rows
                .iter()
                .filter(|r| r.chosen)
                .map(|r| Want {
                    version: r.version.clone(),
                    make_default: r.make_default,
                    ..Want::new(&r.id)
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
        for row in &mut self.rows {
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
                row.versions = e.result["releases"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|r| r["file"].is_string())
                    .filter_map(|r| r["version"].as_str().map(str::to_string))
                    .collect();
                if row.versions.is_empty() {
                    row.chosen = false;
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

    fn header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(icon::BOX)
                    .size(22.0)
                    .color(ui.visuals().hyperlink_color),
            );
            ui.heading(self.t.tr("app-title"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let dark = ui.visuals().dark_mode;
                let (glyph, key) = if dark {
                    (icon::SUN, "win-theme-light")
                } else {
                    (icon::MOON, "win-theme-dark")
                };
                let r = ui.button(glyph).on_hover_text(self.t.tr(key));
                let name = self.t.tr(key);
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, &name)
                });
                if r.clicked() {
                    ui.ctx().set_theme(if dark {
                        egui::Theme::Light
                    } else {
                        egui::Theme::Dark
                    });
                }
            });
        });
        ui.separator();
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let (label, files) = {
            let r = self.lock();
            (r.platform.label(), r.platform.asset_key().to_string())
        };
        ui.label(
            self.t
                .tr_args("app-line", &[("version", env!("CARGO_PKG_VERSION").into())]),
        );
        ui.label(
            RichText::new(
                self.t
                    .tr_args("app-system", &[("system", label), ("files", files)]),
            )
            .color(ui.visuals().widgets.noninteractive.fg_stroke.color),
        );
        ui.add_space(16.0);
        for k in [
            "product-murmelcraft",
            "product-koflcraft",
            "product-foehncraft",
        ] {
            ui.label(format!("•  {}", self.t.tr(k)));
        }
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            if ui.button(self.t.tr("menu-install")).clicked() {
                self.go(ui.ctx(), Page::Scope);
            }
            if ui.button(self.t.tr("win-manage")).clicked() {
                self.go(ui.ctx(), Page::Manage);
            }
        });
    }

    fn scope_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.t.tr("scope-title"));
        let (ru, rs) = (
            self.default_root(Scope::User),
            self.default_root(Scope::System),
        );
        ui.radio_value(
            &mut self.scope,
            Scope::User,
            self.t.tr_args("scope-user", &[("root", ru.clone())]),
        );
        ui.radio_value(
            &mut self.scope,
            Scope::System,
            self.t.tr_args("scope-system", &[("root", rs.clone())]),
        );
        ui.add_space(12.0);
        let root = if self.scope == Scope::User { ru } else { rs };
        ui.label(self.t.tr_args("folder-ask", &[("root", root.clone())]));
        ui.horizontal(|ui| {
            let name = self.t.tr("win-folder");
            ui.add(
                egui::TextEdit::singleline(&mut self.folder)
                    .hint_text(root)
                    .desired_width(420.0),
            )
            .labelled_by(ui.label(name).id);
            if ui.button(self.t.tr("win-choose")).clicked()
                && let Some(d) = rfd::FileDialog::new().pick_folder()
            {
                self.folder = d.display().to_string();
            }
        });
        if self.scope == Scope::System {
            ui.label(RichText::new(self.t.tr("win-admin-note")).color(ui.visuals().warn_fg_color));
        }
    }

    fn source_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.t.tr("source-title"));
        ui.radio_value(&mut self.offline, false, self.t.tr("source-online"));
        ui.radio_value(&mut self.offline, true, self.t.tr("source-folder"));
        if self.offline {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let name = self.t.tr("source-folder-ask");
                ui.add(egui::TextEdit::singleline(&mut self.offline_dir).desired_width(420.0))
                    .labelled_by(ui.label(name).id);
                if ui.button(self.t.tr("win-choose")).clicked()
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
        ui.heading(self.t.tr("products-title"));
        if self.loading() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(self.t.tr("win-loading"));
            });
        }
        let latest_word = self.t.tr("win-latest");
        let default_word = self.t.tr("win-make-default");
        for row in &mut self.rows {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let enabled = !row.versions.is_empty();
                ui.add_enabled_ui(enabled, |ui| {
                    ui.checkbox(&mut row.chosen, RichText::new(&row.title).strong());
                });
                ui.label(self.t.tr(&format!("product-{}", row.id)));
                if let Some(e) = &row.error {
                    ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
                } else if row.job.is_none() && row.versions.is_empty() {
                    ui.label(
                        RichText::new(self.t.tr("products-no-file"))
                            .color(ui.visuals().warn_fg_color),
                    );
                }
                if !row.installed.is_empty() {
                    ui.label(
                        self.t
                            .tr_args("win-installed", &[("versions", row.installed.join(", "))]),
                    );
                }
                if row.chosen && enabled {
                    ui.horizontal(|ui| {
                        let shown = if row.version == "latest" {
                            format!(
                                "{latest_word} ({})",
                                row.versions.first().cloned().unwrap_or_default()
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
                                for v in &row.versions {
                                    ui.selectable_value(&mut row.version, v.clone(), v.clone());
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
        }
    }

    fn options_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.t.tr("win-options"));
        ui.checkbox(&mut self.path, self.t.tr("path-ask"));
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
                    let mut head = RichText::new(title).strong();
                    if s["admin"].as_bool().unwrap_or(false) {
                        head = RichText::new(format!("{title}  {admin}"))
                            .strong()
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
        ui.heading(self.t.tr("steps-title"));
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
            None => {
                ui.heading(self.t.tr("win-running"));
            }
            Some(e) if e.ok() => {
                ui.heading(
                    RichText::new(self.t.tr("win-done")).color(ui.visuals().hyperlink_color),
                );
            }
            Some(_) => {
                ui.heading(
                    RichText::new(self.t.tr("win-failed")).color(ui.visuals().error_fg_color),
                );
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
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for i in &infos {
                    ui.label(i);
                }
            });
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
            ui.label(
                RichText::new(self.t.tr_args(
                    "done-version",
                    &[
                        ("title", title),
                        ("version", v),
                        ("dir", p["dir"].as_str().unwrap_or("").into()),
                    ],
                ))
                .strong(),
            );
            let k = if p["verified"].as_bool().unwrap_or(false) {
                "done-checked"
            } else {
                "done-unchecked"
            };
            ui.label(format!("    {}", self.t.tr(k)));
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
        ui.heading(self.t.tr("win-manage"));
        let before = self.scope;
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.scope, Scope::User, self.t.tr("list-scope-user"));
            ui.radio_value(
                &mut self.scope,
                Scope::System,
                self.t.tr("list-scope-system"),
            );
        });
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
        for p in &list {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let title = p["title"].as_str().unwrap_or("").to_string();
                ui.label(RichText::new(&title).strong());
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
                ui.add_enabled_ui(!busy, |ui| {
                    if ui
                        .button(self.t.tr_args("remove-all", &[("title", title.clone())]))
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
                && ui
                    .add_enabled(!running, egui::Button::new(self.t.tr("win-back")))
                    .clicked()
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
                    let ok = self.can_next()
                        && (n != Page::Running
                            || self
                                .preview
                                .as_ref()
                                .and_then(|j| j.done.as_ref())
                                .is_some_and(|e| e.ok()));
                    if ui
                        .add_enabled(ok, egui::Button::new(self.t.tr(key)))
                        .clicked()
                    {
                        self.go(ui.ctx(), n);
                    }
                }
                if self.page == Page::Running
                    && self.run.as_ref().is_some_and(|j| j.done.is_some())
                    && ui.button(self.t.tr("win-close")).clicked()
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
        egui::Panel::bottom("footer").show(ui, |ui| {
            ui.add_space(8.0);
            self.footer(ui);
            ui.add_space(8.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            self.header(ui);
            ui.add_space(8.0);
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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kraxn")
            .with_inner_size([820.0, 640.0])
            .with_min_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Kraxn",
        options,
        Box::new(move |cc| Ok(Box::new(KraxnApp::new(cc, runner)?))),
    )
    .map_err(|e| e.to_string())
}
