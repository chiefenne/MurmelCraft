//! FoehnCraft's own fields in the Derived fields window (the maintainer's
//! card of 2026-11-24, "Run foehn post derive for me"): under the Wall
//! and Turbulence groups, a row for each field only FoehnCraft's models
//! give (y⁺, wall shear stress, the wall treatment's values, the
//! turbulence features, the wall normal, the curvature correction). When
//! FoehnCraft's `foehn`, the run's case and its restart file are found,
//! Compute runs `foehn post derive` in the background (the window keeps
//! drawing; Cancel or Esc stops it) and `field.foehn_derive` reads the
//! file it wrote in; otherwise the rows are greyed with the reason.

use std::path::PathBuf;
use std::time::Duration;

use egui::{RichText, Role, Ui, WidgetInfo};
use murmel_engine::derive::Group;
use murmel_engine::field::FieldSet;
use murmel_engine::foehn::{Model, Plan};
use murmel_tools::foehn::{Running, foehn_here, kept_of, plan_of};
use serde_json::json;

use crate::app::MurmelApp;
use crate::derived::DerivedWindow;

/// Whether `foehn post derive` can run for the result `mesh`: the
/// program and the plan, or why not.
#[derive(Clone, Debug)]
pub struct FoehnReady {
    pub mesh: PathBuf,
    pub ready: Result<(PathBuf, Plan), String>,
}

/// What a frame of the window asks of FoehnCraft.
#[derive(Default)]
pub(crate) struct FoehnAsk {
    pub start: Option<Model>,
    pub cancel: bool,
    /// The field to colour by.
    pub show: Option<&'static str>,
}

impl MurmelApp {
    /// Whether foehn can run for the open result, looked up again when
    /// another file is open.
    pub(crate) fn foehn_ready(&self, w: &mut DerivedWindow) {
        let Some(mesh) = self.session.mesh.as_ref().map(|m| m.path.clone()) else {
            w.foehn = None;
            return;
        };
        if w.foehn.as_ref().is_some_and(|f| f.mesh == mesh) {
            return;
        }
        let ready = plan_of(&self.session)
            .and_then(|plan| Ok((foehn_here()?, plan)))
            .map_err(|d| d.message);
        w.foehn = Some(FoehnReady { mesh, ready });
    }

    /// The rows of FoehnCraft's fields of `group`, under the group's own.
    pub(crate) fn foehn_block(
        &self,
        ui: &mut Ui,
        set: &FieldSet,
        group: Group,
        w: &DerivedWindow,
        secondary: egui::Color32,
        ask: &mut FoehnAsk,
    ) {
        let models: Vec<Model> = Model::ALL
            .into_iter()
            .filter(|m| m.group() == group)
            .collect();
        if models.is_empty() {
            return;
        }
        ui.add_space(6.0);
        ui.label(RichText::new(self.i18n.tr("derived-foehn-heading")).strong());
        let reason = match w.foehn.as_ref().map(|f| &f.ready) {
            Some(Ok(_)) => None,
            Some(Err(why)) => Some(why.clone()),
            None => Some(self.i18n.tr("derived-no-fields")),
        };
        if let Some(why) = &reason {
            ui.label(RichText::new(why).small().color(secondary));
        }
        if let Some(job) = &self.foehn_job {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(self.i18n.tr_args(
                    "derived-foehn-running",
                    &[(
                        "seconds",
                        format!("{:.0}", job.started.elapsed().as_secs_f64()).into(),
                    )],
                ));
                let name = self.i18n.tr("derived-foehn-cancel-name");
                let r = ui.button(self.i18n.tr("derived-foehn-cancel"));
                r.widget_info(|| WidgetInfo::labeled(Role::Button, true, &name));
                if r.clicked() {
                    ask.cancel = true;
                }
            });
        }
        if let Some(note) = &w.foehn_note {
            ui.label(RichText::new(note).small().color(secondary));
        }
        for m in models {
            let title = self.i18n.tr(&format!("derived-foehn-{}", m.key()));
            let in_file = m.fields().iter().all(|f| set.get(f).is_some());
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(&title).strong()).extend());
                ui.label(
                    RichText::new(format!("[{}]", m.unit()))
                        .small()
                        .color(secondary),
                );
                let named = |key: &str| {
                    self.i18n
                        .tr_args(key, &[("quantity", title.clone().into())])
                };
                if in_file {
                    ui.label(
                        RichText::new(self.i18n.tr("derived-in-file"))
                            .small()
                            .color(secondary),
                    );
                    let name = named("derived-show-name");
                    let r = ui.button(self.i18n.tr("derived-show"));
                    r.widget_info(|| WidgetInfo::labeled(Role::Button, true, &name));
                    if r.clicked() {
                        ask.show = Some(m.fields()[0]);
                    }
                } else {
                    let enabled = reason.is_none() && self.foehn_job.is_none();
                    let name = named("derived-foehn-compute-name");
                    let mut r = ui.add_enabled(
                        enabled,
                        egui::Button::new(self.i18n.tr("derived-foehn-compute")),
                    );
                    if let Some(why) = &reason {
                        r = r.on_disabled_hover_text(why);
                    }
                    r.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, &name));
                    if r.clicked() {
                        ask.start = Some(m);
                    }
                }
            });
        }
    }

    /// Start, or stop, what the window asked for.
    pub(crate) fn foehn_act(&mut self, ctx: &egui::Context, ask: FoehnAsk) {
        if let Some(field) = ask.show {
            self.call("display.color_by", json!({"by": "field", "field": field}));
        }
        if ask.cancel
            && let Some(job) = self.foehn_job.take()
        {
            job.stop();
            self.derived.foehn_note = Some(self.i18n.tr("derived-foehn-cancelled"));
        }
        let Some(m) = ask.start else { return };
        if self.foehn_job.is_some() {
            return;
        }
        let Some(Ok((program, plan))) = self.derived.foehn.as_ref().map(|f| f.ready.clone()) else {
            return;
        };
        match Running::start(&program, &plan, &[m], &kept_of(&self.session)) {
            Ok(job) => {
                self.derived.foehn_note = Some(job.command.join(" "));
                self.foehn_job = Some(job);
                ctx.request_repaint();
            }
            Err(d) => self.derived.foehn_note = Some(d.message),
        }
    }

    /// Read in what a finished `foehn post derive` wrote; keep the
    /// window drawing while it runs.
    pub(crate) fn poll_foehn(&mut self, ctx: &egui::Context) {
        let Some(job) = self.foehn_job.as_mut() else {
            return;
        };
        let Some(done) = job.poll() else {
            ctx.request_repaint_after(Duration::from_millis(200));
            return;
        };
        let Some(job) = self.foehn_job.take() else {
            return;
        };
        if let Err(d) = done {
            self.derived.foehn_note = Some(d.message);
            return;
        }
        let fields: Vec<&str> = job.models.iter().map(|m| m.key()).collect();
        let e = self.call_envelope(
            "field.foehn_derive",
            json!({"fields": fields, "file": job.file}),
        );
        if !e.ok() {
            self.derived.foehn_note = e.diagnostics.first().map(|d| d.message.clone());
            return;
        }
        let names = |key: &str| -> Vec<String> {
            e.result[key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let (added, missing) = (names("added"), names("not_written"));
        let mut note = if added.is_empty() {
            String::new()
        } else {
            self.i18n.tr_args(
                "derived-foehn-done",
                &[
                    ("fields", added.join(", ").into()),
                    (
                        "seconds",
                        format!("{:.0}", job.started.elapsed().as_secs_f64()).into(),
                    ),
                ],
            )
        };
        if !missing.is_empty() {
            if !note.is_empty() {
                note.push(' ');
            }
            note.push_str(&self.i18n.tr_args(
                "derived-foehn-not-written",
                &[("fields", missing.join(", ").into())],
            ));
        }
        self.derived.foehn_note = Some(note);
        if let Some(first) = added.first() {
            self.call("display.color_by", json!({"by": "field", "field": first}));
        }
    }
}
