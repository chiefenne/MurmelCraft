//! The player bar of a time series (D268, first milestone): below the
//! viewport while a series is open (`series.open`, the palette's "Open
//! time series…"). First, previous, play or pause, next and last; a slider
//! over the steps; the step, the time and the position; loop and the rate
//! in steps a second. Every button calls `series.go` or `series.play`, so
//! agents and scripts do the same; while playing, the window calls
//! `series.go` at the rate. Steps are read on the window's thread: a large
//! mesh plays as fast as it reads.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use egui::{Color32, Pos2, Rect, RichText, Role, Stroke, Ui, WidgetInfo, pos2, vec2};
use murmel_engine::history::Progress;
use murmel_engine::series::{MAX_RATE, MIN_RATE};
use murmel_tools::{Diagnostic, Envelope};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::style::color;

/// The colour range over the steps of the open series (range mode
/// steps, the maintainer's card of 2026-11-24), read by `series.range`
/// on a copy of the session on a thread of its own. The copies share the
/// ranges read, so the legend widens while it reads; a reading of a
/// field no longer shown is stopped.
#[derive(Default)]
pub(crate) struct StepsRangeJob {
    job: Option<(String, Arc<Progress>, JoinHandle<Envelope>)>,
    /// The key that could not be read, and why; not tried again.
    failed: Option<(String, String)>,
}

/// What a painted button of the bar shows.
#[derive(Clone, Copy)]
enum Glyph {
    First,
    Previous,
    Play,
    Pause,
    Next,
    Last,
}

/// Paint `g` into `r` in `c`: triangles and bars, so no icon font is
/// needed.
fn paint(ui: &Ui, r: Rect, g: Glyph, c: Color32) {
    let p = ui.painter();
    let m = r.center();
    let h = 5.5;
    let tri = |tip: f32, base: f32| {
        // A triangle pointing from `base` to `tip` along x.
        vec![pos2(base, m.y - h), pos2(tip, m.y), pos2(base, m.y + h)]
    };
    let bar = |x: f32| Rect::from_min_max(pos2(x - 1.0, m.y - h), pos2(x + 1.0, m.y + h));
    let fill = |pts: Vec<Pos2>| {
        p.add(egui::Shape::convex_polygon(pts, c, Stroke::NONE));
    };
    match g {
        Glyph::Play => fill(tri(m.x + 5.0, m.x - 4.0)),
        Glyph::Pause => {
            p.rect_filled(bar(m.x - 3.0), 0.0, c);
            p.rect_filled(bar(m.x + 3.0), 0.0, c);
        }
        Glyph::Next => {
            fill(tri(m.x + 3.0, m.x - 5.0));
            p.rect_filled(bar(m.x + 5.0), 0.0, c);
        }
        Glyph::Previous => {
            fill(tri(m.x - 3.0, m.x + 5.0));
            p.rect_filled(bar(m.x - 5.0), 0.0, c);
        }
        Glyph::Last => {
            fill(tri(m.x + 1.0, m.x - 7.0));
            fill(tri(m.x + 7.0, m.x - 1.0));
            p.rect_filled(bar(m.x + 8.0), 0.0, c);
        }
        Glyph::First => {
            fill(tri(m.x - 1.0, m.x + 7.0));
            fill(tri(m.x - 7.0, m.x + 1.0));
            p.rect_filled(bar(m.x - 8.0), 0.0, c);
        }
    }
}

impl MurmelApp {
    /// A painted button with an accessible name and a tooltip.
    fn glyph_button(&self, ui: &mut Ui, g: Glyph, key: &str, enabled: bool) -> egui::Response {
        let name = self.i18n.tr(key);
        let b = ui
            .add_enabled(enabled, egui::Button::new("").min_size(vec2(30.0, 24.0)))
            .on_hover_text(&name);
        let c = if enabled {
            ui.visuals().widgets.inactive.fg_stroke.color
        } else {
            ui.visuals().widgets.noninteractive.fg_stroke.color
        };
        paint(ui, b.rect, g, c);
        b.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, name.clone()));
        b
    }

    /// Ask for a series.json or an output folder and open the series.
    pub(crate) fn series_open_dialog(&mut self) {
        let mut d = rfd::FileDialog::new().set_title(self.i18n.tr("cmd-series-open"));
        if let Some(dir) = self.session.mesh.as_ref().and_then(|m| m.path.parent()) {
            d = d.set_directory(dir);
        }
        // A folder: its series.json, else its numbered files.
        let picked: Option<PathBuf> = d.pick_folder();
        if let Some(p) = picked {
            self.call("series.open", json!({ "path": p }));
        }
    }

    /// Read the range over the steps of the field shown when range mode
    /// steps needs it; take a finished reading.
    pub(crate) fn read_steps_range(&mut self, ctx: &egui::Context) {
        let want = murmel_tools::series::steps_range_pending(&self.session);
        let j = &mut self.steps_range;
        if let Some((key, progress, handle)) = &j.job {
            if want.as_ref() != Some(key) {
                progress.cancel();
            }
            if !handle.is_finished() {
                ctx.request_repaint_after(Duration::from_millis(100));
                return;
            }
            if let Some((key, _, handle)) = j.job.take() {
                let e = handle.join().unwrap_or_else(|_| {
                    Envelope::failed(
                        "series.range",
                        1,
                        Diagnostic::new(
                            "SERIES004",
                            "the range over the steps stopped unexpectedly",
                        ),
                    )
                });
                if let Some(d) = e.diagnostics.first().filter(|_| !e.ok())
                    && !d.message.contains("cancelled")
                {
                    j.failed = Some((key, d.message.clone()));
                }
            }
            ctx.request_repaint();
            return;
        }
        let Some(key) = want else { return };
        if j.failed.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let mut session = self.session.clone();
        let progress = Arc::new(Progress::default());
        let p = progress.clone();
        let handle = std::thread::spawn(move || {
            murmel_tools::probe::with_progress(p, || {
                murmel_tools::call("series.range", json!({}), &mut session)
            })
        });
        j.failed = None;
        j.job = Some((key, progress, handle));
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    /// The player bar's note on the range over the steps: how far its
    /// reading is, or why it failed; `None` when there is nothing to say.
    fn steps_range_note(&self) -> Option<String> {
        let key = murmel_tools::series::steps_range_pending(&self.session)?;
        if let Some((k, why)) = &self.steps_range.failed
            && *k == key
        {
            return Some(
                self.i18n
                    .tr_args("player-range-failed", &[("why", why.clone().into())]),
            );
        }
        let (_, p, _) = self.steps_range.job.as_ref()?;
        Some(self.i18n.tr_args(
            "player-range-reading",
            &[
                ("read", (p.done() as i64).into()),
                ("n", (p.total() as i64).into()),
            ],
        ))
    }

    /// While the series plays, show the next step when its time has come
    /// and wake the window for the one after.
    pub(crate) fn play_series(&mut self, ctx: &egui::Context) {
        let Some(r) = &self.session.series else {
            self.played_at = None;
            return;
        };
        if !r.playback.playing {
            self.played_at = None;
            return;
        }
        let every = 1.0 / r.playback.rate;
        let next = r.next_played();
        let at = *self.played_at.get_or_insert(self.now);
        let wait = at + every - self.now;
        if wait > 0.0 {
            ctx.request_repaint_after(Duration::from_secs_f64(wait));
            return;
        }
        match next {
            Some(k) => {
                if self.call("series.go", json!({ "index": k })).is_none() {
                    self.call("series.play", json!({ "playing": false }));
                }
                // Late frames do not pile up: the next step is due one
                // interval from now.
                self.played_at = Some(self.now);
                ctx.request_repaint_after(Duration::from_secs_f64(every));
            }
            None => {
                self.call("series.play", json!({ "playing": false }));
            }
        }
    }

    /// The player bar, while a series is open.
    pub(crate) fn player_bar(&mut self, ui: &mut Ui) {
        let Some(r) = self.session.series.clone() else {
            return;
        };
        let p = self.palette(ui.ctx()).clone();
        let secondary = color(p.text_secondary);
        let n = r.len();
        let k = r.current;
        let mut calls: Vec<(&'static str, Value)> = Vec::new();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let first = self.glyph_button(ui, Glyph::First, "player-first", k > 0);
            self.focus.offer(crate::focus::Region::Player, &first);
            if first.clicked() {
                calls.push(("series.go", json!({"to": "first"})));
            }
            let back = k > 0 || r.playback.looping;
            if self
                .glyph_button(ui, Glyph::Previous, "player-previous", back && n > 1)
                .clicked()
            {
                calls.push(("series.go", json!({"by": -1})));
            }
            let (g, key) = if r.playback.playing {
                (Glyph::Pause, "player-pause")
            } else {
                (Glyph::Play, "player-play")
            };
            if self.glyph_button(ui, g, key, n > 1).clicked() {
                // Play from the start when the last step of a series that
                // does not loop is shown.
                if !r.playback.playing && !r.playback.looping && k + 1 == n {
                    calls.push(("series.go", json!({"to": "first"})));
                }
                calls.push(("series.play", json!({"playing": !r.playback.playing})));
            }
            let on = k + 1 < n || r.playback.looping;
            if self
                .glyph_button(ui, Glyph::Next, "player-next", on && n > 1)
                .clicked()
            {
                calls.push(("series.go", json!({"by": 1})));
            }
            if self
                .glyph_button(ui, Glyph::Last, "player-last", k + 1 < n)
                .clicked()
            {
                calls.push(("series.go", json!({"to": "last"})));
            }
            ui.add_space(6.0);
            // The slider over the steps, by index; it shows the step.
            let mut at = k;
            let steps = r.entries.iter().map(|e| e.step).collect::<Vec<_>>();
            let name = self.i18n.tr("player-slider");
            let slider = ui
                .add_enabled(
                    n > 1,
                    egui::Slider::new(&mut at, 0..=n.saturating_sub(1))
                        .show_value(false)
                        .custom_formatter(|v, _| {
                            steps
                                .get(v as usize)
                                .map(|s| s.to_string())
                                .unwrap_or_default()
                        }),
                )
                .on_hover_text(&name);
            slider.widget_info(|| WidgetInfo::slider(n > 1, at as f64, name.clone()));
            if slider.changed() && at != k {
                calls.push(("series.go", json!({"index": at})));
            }
            let time = murmel_tools::series::shown_time(&self.session);
            let text = match time {
                Some(t) => self.i18n.tr_args(
                    "player-at-time",
                    &[
                        ("step", (r.entry().step as i64).into()),
                        ("time", format!("{t:.4}").into()),
                        ("k", ((k + 1) as i64).into()),
                        ("n", (n as i64).into()),
                    ],
                ),
                None => self.i18n.tr_args(
                    "player-at",
                    &[
                        ("step", (r.entry().step as i64).into()),
                        ("k", ((k + 1) as i64).into()),
                        ("n", (n as i64).into()),
                    ],
                ),
            };
            ui.label(RichText::new(text).small().monospace());
            if let Some(note) = self.steps_range_note() {
                ui.label(RichText::new(note).small().color(secondary));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut rate = r.playback.rate;
                let label_text = self.i18n.tr("player-rate");
                let drag = ui
                    .add(
                        egui::DragValue::new(&mut rate)
                            .range(MIN_RATE..=MAX_RATE)
                            .speed(0.1)
                            .max_decimals(1)
                            .suffix(self.i18n.tr("player-rate-unit")),
                    )
                    .on_hover_text(&label_text);
                drag.widget_info(|| {
                    WidgetInfo::labeled(Role::SpinButton, true, label_text.clone())
                });
                if drag.changed() {
                    calls.push(("series.play", json!({"rate": rate})));
                }
                ui.label(RichText::new(&label_text).small().color(secondary));
                let mut looping = r.playback.looping;
                if ui
                    .checkbox(&mut looping, self.i18n.tr("player-loop"))
                    .changed()
                {
                    calls.push(("series.play", json!({"loop": looping})));
                }
            });
        });
        for (id, input) in calls {
            self.call(id, input);
            if id == "series.play" {
                self.played_at = None;
            }
        }
    }
}
