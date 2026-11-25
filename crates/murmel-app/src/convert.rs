//! A large mesh that is not blocked (the maintainer's card 4 of
//! 2026-11-24): opening it asks whether to convert it once to a blocked
//! copy beside it, which then opens as an overview, or to open it whole;
//! "Convert large meshes without asking" (here or in Settings) skips the
//! question. The conversion runs on a thread of its own; the status bar
//! follows it, with Cancel or Esc ([`murmel_engine::convert`]).

use std::time::Duration;

use egui::{RichText, Role, Ui, WidgetInfo};
use murmel_engine::OpenMode;
use murmel_engine::memory::gib;
use murmel_tools::Diagnostic;
use serde_json::json;

use crate::app::MurmelApp;
use crate::style::color;

/// The file name of `p`, else the whole path.
fn name_of(p: &std::path::Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl MurmelApp {
    /// When the mesh at `path`, opened as it fits the budget, is not
    /// blocked and needs more: open its up-to-date blocked copy, convert
    /// it without asking (the setting), or ask. True when the open is
    /// taken over.
    pub(crate) fn offer_convert(&mut self, path: &std::path::Path, mode: OpenMode) -> bool {
        if mode != OpenMode::Fit {
            return false;
        }
        let Some(o) = self.session.convert_offer(path) else {
            return false;
        };
        if o.fresh {
            self.open_path_as(o.out, OpenMode::Fit);
        } else if self.session.convert_large {
            self.session.start_converting(&o);
        } else {
            self.convert_remember = false;
            self.convert_offer = Some(o);
        }
        true
    }

    /// The question for a large mesh that is not blocked.
    pub(crate) fn convert_window(&mut self, ctx: &egui::Context) {
        let Some(o) = self.convert_offer.clone() else {
            return;
        };
        let p = self.palette(ctx).clone();
        let mut open = true;
        let mut chosen: Option<Choice> = None;
        let mut remember = self.convert_remember;
        egui::Window::new(self.i18n.tr("convert-title"))
            .id(egui::Id::unique("convert-large"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(480.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(self.i18n.tr_args(
                    "convert-needs",
                    &[
                        ("file", name_of(&o.path).into()),
                        ("cells", (o.cells as f64).into()),
                        ("needs", gib(o.needs).into()),
                        ("budget", gib(o.budget).into()),
                    ],
                ));
                ui.add_space(4.0);
                ui.label(self.i18n.tr_args(
                    "convert-offer",
                    &[
                        ("out", name_of(&o.out).into()),
                        ("disk", gib(o.disk).into()),
                    ],
                ));
                if o.beyond_memory {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(self.i18n.tr("convert-beyond-memory"))
                            .color(color(p.status_warn)),
                    );
                }
                ui.add_space(6.0);
                ui.checkbox(&mut remember, self.i18n.tr("convert-always"));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(self.i18n.tr("convert-go")).clicked() {
                        chosen = Some(Choice::Convert);
                    }
                    if ui.button(self.i18n.tr("convert-whole")).clicked() {
                        chosen = Some(Choice::Whole);
                    }
                    if ui.button(self.i18n.tr("convert-cancel")).clicked() {
                        chosen = Some(Choice::Cancel);
                    }
                });
            });
        self.convert_remember = remember;
        let esc = !ctx.text_edit_focused()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if !open || esc {
            chosen = Some(Choice::Cancel);
        }
        let Some(c) = chosen else {
            return;
        };
        self.convert_offer = None;
        match c {
            Choice::Convert => {
                if remember {
                    self.call("settings.set_memory", json!({ "convert_large": true }));
                }
                self.session.start_converting(&o);
            }
            Choice::Whole => self.open_path_as(o.path, OpenMode::Whole),
            Choice::Cancel => {}
        }
    }

    /// Open the blocked copy once converted, or say why there is none;
    /// keep looking while it is converted.
    pub(crate) fn poll_convert(&mut self, ctx: &egui::Context) {
        let Some(j) = &self.session.convert_job else {
            return;
        };
        if !j.is_done() {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        match self.session.poll_convert(false) {
            Some(Ok(out)) => self.open_path_as(out, OpenMode::Fit),
            // Stopped: nothing was written.
            Some(Err(e)) if e.code == "MESH019" => {}
            Some(Err(e)) => self.set_error(Some(Diagnostic::new(e.code, e.message))),
            None => {}
        }
        ctx.request_repaint();
    }

    /// What the window says while a mesh is converted: the file, the
    /// phase and the seconds so far.
    pub(crate) fn convert_text(&self) -> Option<String> {
        let j = self.session.convert_job.as_ref()?;
        let phase = self
            .i18n
            .tr(crate::opening::phase_message(j.progress.phase()));
        Some(self.i18n.tr_args(
            "status-convert",
            &[
                ("file", name_of(&j.path).into()),
                ("phase", phase.into()),
                (
                    "seconds",
                    format!("{:.1}", j.started.elapsed().as_secs_f64()).into(),
                ),
            ],
        ))
    }

    /// The status bar while a mesh is converted: what it says and Cancel.
    pub(crate) fn convert_status(&mut self, ui: &mut Ui, said: &mut Vec<String>) -> bool {
        let Some(text) = self.convert_text() else {
            return false;
        };
        let cancel = self.i18n.tr("status-loading-cancel");
        let tip = self.i18n.tr("status-convert-cancel-tip");
        let b = ui.small_button(&cancel).on_hover_text(&tip);
        let name = format!("{cancel}. {tip}");
        b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
        if b.clicked() {
            self.session.cancel_convert();
        }
        ui.label(RichText::new(&text).small());
        said.push(text);
        true
    }
}

/// What the question was answered with.
#[derive(Clone, Copy)]
enum Choice {
    Convert,
    Whole,
    Cancel,
}
