//! The Probe history window (the maintainer's card of 2026-11-24,
//! "Follow steps, plus history plot"): every probe's value of one field
//! over the time steps of the open series, read on a thread of its own
//! by `probe.history` on a copy of the session, plotted with the egui
//! painter (one line per probe, in the patch colours), the step shown
//! marked, the values of the step under the pointer listed, and
//! "Export CSV…" (`probe.export`, which takes the history just read).
//! The history is read again by itself when the probes, the series or
//! the derived fields change; playing the series reads nothing again.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use egui::{Color32, Pos2, Rect, RichText, Role, Sense, Shape, Stroke, WidgetInfo};
use murmel_engine::field::Component;
use murmel_engine::history::{History, Progress};
use murmel_tools::{Diagnostic, Envelope};
use serde_json::json;

use crate::app::MurmelApp;
use crate::panels::number;
use crate::style::color;

/// A history being read on its own thread.
pub(crate) struct HistoryJob {
    progress: Arc<Progress>,
    handle: JoinHandle<Envelope>,
    /// What it reads ([`murmel_tools::probe::key_of`]).
    key: String,
}

/// The window's state.
#[derive(Default)]
pub(crate) struct HistoryWindow {
    pub open: bool,
    /// The field plotted (the field colouring the mesh at first).
    pub field: Option<String>,
    pub component: Component,
    pub job: Option<HistoryJob>,
    /// Why the history of `failed_key` could not be read.
    pub error: Option<String>,
    failed_key: Option<String>,
    /// The file written last.
    pub exported: Option<String>,
}

/// The plot's height, in points.
const PLOT_HEIGHT: f32 = 260.0;

impl MurmelApp {
    pub(crate) fn open_probe_history(&mut self) {
        self.history.open = true;
        if self.history.field.is_none()
            && let Some(c) = &self.session.display.field
        {
            self.history.field = Some(c.name.clone());
            self.history.component = c.component;
        }
    }

    /// Take a finished reading; keep the window drawing while one runs.
    pub(crate) fn poll_probe_history(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.history.job else { return };
        if !job.handle.is_finished() {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        let Some(job) = self.history.job.take() else {
            return;
        };
        let e = job.handle.join().unwrap_or_else(|_| {
            Envelope::failed(
                "probe.history",
                1,
                Diagnostic::new("PROBE002", "the history stopped unexpectedly"),
            )
        });
        if let Some(d) = e.diagnostics.first().filter(|_| !e.ok())
            && d.code != "PROBE002"
        {
            self.history.error = Some(d.message.clone());
            self.history.failed_key = Some(job.key);
        }
        ctx.request_repaint();
    }

    /// Read the history of `key` on a copy of the session.
    fn start_history(&mut self, key: String) {
        let mut session = self.session.clone();
        let progress = Arc::new(Progress::default());
        let p = progress.clone();
        let handle = std::thread::spawn(move || {
            murmel_tools::probe::with_progress(p, || {
                murmel_tools::call("probe.history", json!({}), &mut session)
            })
        });
        self.history.error = None;
        self.history.failed_key = None;
        self.history.job = Some(HistoryJob {
            progress,
            handle,
            key,
        });
    }

    pub(crate) fn probe_history_window(&mut self, ctx: &egui::Context) {
        if !self.history.open {
            return;
        }
        let key = murmel_tools::probe::key_of(&self.session);
        let kept = murmel_tools::probe::kept(&self.session);
        let inside = !murmel_tools::probe::points(&self.session).is_empty();
        let has_fields = self.session.fields().is_some_and(|f| !f.is_empty());
        // Read what is not kept, once per key; a reading of an older key
        // is stopped.
        if let Some(k) = &key {
            match &self.history.job {
                Some(j) if &j.key != k => j.progress.cancel(),
                None if kept.is_none()
                    && inside
                    && has_fields
                    && self.history.failed_key.as_ref() != Some(k) =>
                {
                    self.start_history(k.clone());
                }
                _ => {}
            }
        }
        let p = self.palette(ctx).clone();
        let mut open = true;
        let mut export = false;
        let mut cancel = false;
        egui::Window::new(self.i18n.tr("history-title"))
            .id(egui::Id::unique("probe-history"))
            .open(&mut open)
            .collapsible(false)
            .default_width(620.0)
            .show(ctx, |ui| {
                let secondary = color(p.text_secondary);
                if !has_fields {
                    ui.label(self.i18n.tr("history-no-fields"));
                    return;
                }
                if !inside {
                    ui.label(self.i18n.tr("history-no-probes"));
                    return;
                }
                if let Some(j) = &self.history.job {
                    ui.horizontal(|ui| {
                        let (done, total) = (j.progress.done(), j.progress.total());
                        ui.add(
                            egui::ProgressBar::new(if total > 0 {
                                done as f32 / total as f32
                            } else {
                                0.0
                            })
                            .desired_width(240.0)
                            .text(self.i18n.tr_args(
                                "history-reading",
                                &[("done", done.into()), ("total", total.into())],
                            )),
                        );
                        if ui.button(self.i18n.tr("history-cancel")).clicked() {
                            cancel = true;
                        }
                    });
                }
                if let Some(err) = &self.history.error {
                    ui.colored_label(color(p.status_error), err);
                }
                let Some(h) = kept.as_deref() else {
                    return;
                };
                self.history_controls(ui, h, &mut export);
                if self.session.series.is_none() {
                    ui.label(
                        RichText::new(self.i18n.tr("history-one-step"))
                            .small()
                            .color(secondary),
                    );
                }
                self.history_plot(ui, h, &p);
                let skipped = h.rows.iter().filter(|r| r.note.is_some()).count();
                if skipped > 0 {
                    let why = h
                        .rows
                        .iter()
                        .find_map(|r| r.note.clone())
                        .unwrap_or_default();
                    ui.label(
                        RichText::new(
                            self.i18n
                                .tr_args("history-skipped", &[("count", skipped.into())]),
                        )
                        .small()
                        .color(secondary),
                    )
                    .on_hover_text(why);
                }
                if let Some(f) = &self.history.exported {
                    ui.label(RichText::new(f).small().color(secondary));
                }
            });
        if cancel && let Some(j) = &self.history.job {
            j.progress.cancel();
            // Not read again by itself until something changes.
            self.history.failed_key = Some(j.key.clone());
            self.history.error = Some(self.i18n.tr("history-cancelled"));
        }
        if export {
            self.export_history();
        }
        if !open {
            self.history.open = false;
            if let Some(j) = &self.history.job {
                j.progress.cancel();
            }
        }
    }

    /// The field and component picked, and the export button.
    fn history_controls(&mut self, ui: &mut egui::Ui, h: &History, export: &mut bool) {
        if self
            .history
            .field
            .as_ref()
            .and_then(|f| h.column(f))
            .is_none()
        {
            self.history.field = h.columns.first().map(|c| c.name.clone());
            self.history.component = Component::Magnitude;
        }
        let Some(name) = self.history.field.clone() else {
            return;
        };
        let vector = h.column(&name).is_some_and(|k| h.columns[k].components > 1);
        ui.horizontal(|ui| {
            let label = self.i18n.tr("history-field");
            ui.label(&label);
            egui::ComboBox::from_id_salt("history-field")
                .selected_text(&name)
                .show_ui(ui, |ui| {
                    for c in &h.columns {
                        if ui.selectable_label(c.name == name, &c.name).clicked() {
                            self.history.field = Some(c.name.clone());
                            self.history.component = Component::Magnitude;
                        }
                    }
                })
                .response
                .widget_info(|| {
                    WidgetInfo::labeled(Role::ComboBox, true, format!("{label}: {name}"))
                });
            if vector {
                for c in Component::ALL {
                    let text = self.i18n.tr(&format!("field-component-{}", c.name()));
                    if ui
                        .selectable_label(c == self.history.component, &text)
                        .clicked()
                    {
                        self.history.component = c;
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(self.i18n.tr("history-export")).clicked() {
                    *export = true;
                }
            });
        });
    }

    /// The plot: a line per probe over time (or step), the step shown
    /// marked, the values of the step under the pointer listed.
    fn history_plot(&mut self, ui: &mut egui::Ui, h: &History, p: &murmel_engine::theme::Palette) {
        let Some(name) = self.history.field.clone() else {
            return;
        };
        let Some(k) = h.column(&name) else { return };
        let comp = self.history.component;
        let unit = h.columns[k].unit.clone();
        // x: the time when every step has one, else the step.
        let by_time = h.rows.iter().all(|r| r.time.is_some());
        let xs: Vec<f64> = h
            .rows
            .iter()
            .map(|r| {
                if by_time {
                    r.time.unwrap_or_default()
                } else {
                    r.step.map_or(r.index as f64, |s| s as f64)
                }
            })
            .collect();
        let value = |row: usize, probe: usize| -> Option<f64> {
            let v = h.rows.get(row)?.values.get(probe)?.get(k)?.as_ref()?;
            let x = match (v.as_slice(), comp) {
                ([s], _) => *s,
                (v, Component::Magnitude) => v.iter().map(|x| x * x).sum::<f64>().sqrt(),
                (v, Component::X) => *v.first()?,
                (v, Component::Y) => *v.get(1)?,
                (v, Component::Z) => *v.get(2)?,
            };
            x.is_finite().then_some(x)
        };
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for r in 0..h.rows.len() {
            for pr in 0..h.probes.len() {
                if let Some(v) = value(r, pr) {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
        }
        if !lo.is_finite() {
            ui.label(self.i18n.tr("history-no-values"));
            return;
        }
        let (lo, hi) = murmel_engine::field::widen((lo, hi));
        let (x0, x1) = xs
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &x| {
                (a.min(x), b.max(x))
            });
        let (x0, x1) = if x1 > x0 {
            (x0, x1)
        } else {
            (x0 - 0.5, x0 + 0.5)
        };

        let title = if h.columns[k].components > 1 {
            murmel_engine::field::FieldChoice {
                name: name.clone(),
                component: comp,
            }
            .title(true)
        } else {
            name.clone()
        };
        let plot_name = self
            .i18n
            .tr_args("history-plot", &[("field", title.clone().into())]);
        let width = ui.available_width().max(320.0);
        let (resp, painter) = ui.allocate_painter(egui::vec2(width, PLOT_HEIGHT), Sense::hover());
        resp.widget_info(|| WidgetInfo::labeled(Role::Image, true, &plot_name));
        let outer = resp.rect;
        let font = egui::FontId::proportional(11.0);
        let text = color(p.text_secondary);
        let line = color(p.line);
        // Room for the value labels on the left and the x labels below.
        let plot = Rect::from_min_max(
            outer.min + egui::vec2(64.0, 8.0),
            outer.max - egui::vec2(12.0, 34.0),
        );
        painter.rect_filled(plot, 2.0, color(p.bg_raised));
        let to_x = |x: f64| plot.left() + ((x - x0) / (x1 - x0)) as f32 * plot.width();
        let to_y = |v: f64| plot.bottom() - ((v - lo) / (hi - lo)) as f32 * plot.height();
        for (a, b, horizontal) in [(lo, hi, true), (x0, x1, false)] {
            for t in ticks(a, b, if horizontal { 5 } else { 6 }) {
                if horizontal {
                    let y = to_y(t);
                    painter.line_segment(
                        [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                        Stroke::new(0.6, line),
                    );
                    painter.text(
                        Pos2::new(plot.left() - 6.0, y),
                        egui::Align2::RIGHT_CENTER,
                        number(t),
                        font.clone(),
                        text,
                    );
                } else {
                    let x = to_x(t);
                    painter.line_segment(
                        [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                        Stroke::new(0.6, line),
                    );
                    painter.text(
                        Pos2::new(x, plot.bottom() + 4.0),
                        egui::Align2::CENTER_TOP,
                        number(t),
                        font.clone(),
                        text,
                    );
                }
            }
        }
        let x_title = self.i18n.tr(if by_time {
            "history-axis-time"
        } else {
            "history-axis-step"
        });
        painter.text(
            Pos2::new(plot.center().x, outer.bottom() - 2.0),
            egui::Align2::CENTER_BOTTOM,
            &x_title,
            font.clone(),
            text,
        );
        let y_title = match &unit {
            Some(u) => format!("{title} ({u})"),
            None => title.clone(),
        };
        painter.text(
            Pos2::new(plot.left() + 6.0, plot.top() + 4.0),
            egui::Align2::LEFT_TOP,
            &y_title,
            font.clone(),
            color(p.text_primary),
        );
        // The step shown.
        if let Some(s) = &self.session.series
            && let Some(&x) = xs.get(s.current)
        {
            let x = to_x(x);
            painter.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(1.5, color(p.accent)),
            );
        }
        let colours = self.probe_colours(h.probes.len());
        for (pr, c) in colours.iter().enumerate() {
            let mut run: Vec<Pos2> = Vec::new();
            for (r, &x) in xs.iter().enumerate() {
                match value(r, pr) {
                    Some(v) => run.push(Pos2::new(to_x(x), to_y(v))),
                    None => flush(&painter, &mut run, *c),
                }
            }
            flush(&painter, &mut run, *c);
        }
        // The step under the pointer.
        if let Some(pos) = resp.hover_pos().filter(|p| plot.contains(*p)) {
            let near = xs
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    (to_x(*a.1) - pos.x)
                        .abs()
                        .total_cmp(&(to_x(*b.1) - pos.x).abs())
                })
                .map(|(i, _)| i);
            if let Some(r) = near {
                let x = to_x(xs[r]);
                painter.line_segment(
                    [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                    Stroke::new(1.0, text),
                );
                let row = &h.rows[r];
                let mut lines = vec![
                    self.i18n.tr_args(
                        "history-hover-step",
                        &[
                            (
                                "step",
                                row.step
                                    .map_or_else(|| "–".into(), |s| s.to_string())
                                    .into(),
                            ),
                            ("time", row.time.map_or_else(|| "–".into(), number).into()),
                        ],
                    ),
                ];
                for (pr, &n) in h.probes.iter().enumerate() {
                    let v = value(r, pr).map_or_else(|| "–".to_string(), number);
                    lines.push(format!(
                        "{}: {v}",
                        self.measure_name(murmel_engine::Item::Probe(n))
                    ));
                }
                if let Some(note) = &row.note {
                    lines.push(note.clone());
                }
                resp.clone().on_hover_text_at_pointer(lines.join("\n"));
            }
        }
        // The legend: a swatch and the name of each probe.
        ui.horizontal_wrapped(|ui| {
            for (pr, &n) in h.probes.iter().enumerate() {
                let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 3.0), Sense::hover());
                ui.painter().rect_filled(r, 0.0, colours[pr]);
                ui.label(RichText::new(self.measure_name(murmel_engine::Item::Probe(n))).small());
                ui.add_space(8.0);
            }
        });
    }

    /// A colour per probe: the patch colours, cycled.
    fn probe_colours(&self, n: usize) -> Vec<Color32> {
        let pal = &self.session.tokens.mesh.patch;
        (0..n)
            .map(|i| {
                pal.get(i % pal.len().max(1))
                    .map_or(Color32::GRAY, |c| color(*c))
            })
            .collect()
    }

    /// Ask for a file and write the history kept there.
    fn export_history(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title(self.i18n.tr("history-file-title"))
            .add_filter("CSV", &["csv"])
            .set_file_name("probes.csv")
            .save_file();
        let Some(path) = picked else { return };
        let e = self.call_envelope("probe.export", json!({"path": path}));
        self.history.exported = Some(if e.ok() {
            self.i18n.tr_args(
                "history-exported",
                &[("file", path.display().to_string().into())],
            )
        } else {
            e.diagnostics
                .first()
                .map(|d| d.message.clone())
                .unwrap_or_default()
        });
    }
}

/// Draw a run of points as a line, and start a new run.
fn flush(painter: &egui::Painter, run: &mut Vec<Pos2>, c: Color32) {
    match run.len() {
        0 => {}
        1 => {
            painter.circle_filled(run[0], 2.5, c);
        }
        _ => {
            painter.add(Shape::line(std::mem::take(run), Stroke::new(1.8, c)));
        }
    }
    run.clear();
}

/// About `n` round values between `a` and `b` (1, 2 or 5 times a power
/// of ten apart).
fn ticks(a: f64, b: f64, n: usize) -> Vec<f64> {
    let span = b - a;
    if !(span.is_finite() && span > 0.0) {
        return vec![a];
    }
    let raw = span / n.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * mag)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * mag);
    let first = (a / step).ceil() as i64;
    let last = (b / step).floor() as i64;
    (first..=last)
        .map(|i| {
            let t = i as f64 * step;
            if t.abs() < step * 1e-9 { 0.0 } else { t }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    /// Invariant: ticks are round, inside the range and evenly spaced.
    #[test]
    fn ticks_are_round() {
        let t = super::ticks(0.0, 1.0, 5);
        assert_eq!(t.len(), 6);
        assert!(
            t.iter()
                .enumerate()
                .all(|(i, x)| (x - 0.2 * i as f64).abs() < 1e-12)
        );
        assert_eq!(super::ticks(-3.2, 7.9, 5), vec![0.0, 5.0]);
        assert_eq!(super::ticks(-3.2, 7.9, 10), vec![-2.0, 0.0, 2.0, 4.0, 6.0]);
        assert_eq!(super::ticks(1.0, 1.0, 5), vec![1.0]);
    }
}
