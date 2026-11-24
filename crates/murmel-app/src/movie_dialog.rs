//! The movie window (File, "Export movie…", Ctrl + Shift + M): what moves
//! (the camera turning about its target, a clip plane sweeping through the
//! mesh, a flight through the saved views, or the steps of the open time
//! series), how long, the frame rate,
//! the format (MP4, GIF or PNG frames), the size and the overlays.
//! "Export…" asks for the file and calls `export.movie` on a copy of the
//! session on a thread of its own, so the window stays live: a progress
//! bar counts the frames and Cancel stops the movie. An agent's
//! `export.movie` over the live bridge runs the same way and shows here.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use egui::{Key, Modifiers, RichText};
use murmel_engine::animation;
use murmel_tools::movie::{Format, Motion, Progress};
use murmel_tools::{Diagnostic, Envelope};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::export_dialog::SizeChoice;
use crate::style::color;

/// Frame rates the window offers.
const RATES: [u32; 3] = [24, 30, 60];

/// The movie window's settings, kept while the window is closed.
#[derive(Clone, Debug)]
pub struct MovieDialog {
    pub open: bool,
    pub motion: Motion,
    /// Orbit: the angle of the turn.
    pub degrees: f64,
    /// Orbit and clip: the length in seconds.
    pub seconds: f64,
    /// Clip: the plane, counted from 1.
    pub plane: u32,
    /// Views: seconds at each view and of each flight.
    pub hold: f64,
    pub travel: f64,
    /// Series: seconds each step shows.
    pub step_hold: f64,
    pub fps: u32,
    pub format: Format,
    pub size: SizeChoice,
    pub custom: [u32; 2],
    pub legend: bool,
    pub axes: bool,
    /// The time and step of the open series, bottom right.
    pub time: bool,
    pub transparent: bool,
    /// What the last movie did, shown in the window.
    pub last: Option<String>,
    /// Write here instead of asking with the file dialog (window tests).
    pub path: Option<PathBuf>,
}

impl Default for MovieDialog {
    fn default() -> Self {
        Self {
            open: false,
            motion: Motion::Orbit,
            degrees: 360.0,
            seconds: murmel_tools::movie::SECONDS,
            plane: 1,
            hold: murmel_tools::movie::HOLD,
            travel: murmel_tools::movie::TRAVEL,
            step_hold: 1.0 / murmel_engine::series::RATE,
            fps: 30,
            format: Format::Mp4,
            size: SizeChoice::FullHd,
            custom: [1920, 1080],
            legend: true,
            axes: true,
            time: true,
            transparent: false,
            last: None,
            path: None,
        }
    }
}

impl MovieDialog {
    /// The input of `export.movie`, without the path.
    pub fn input(&self) -> Value {
        let mut v = json!({
            "motion": self.motion.name(),
            "format": self.format.name(),
            "fps": self.fps,
            "legend": self.legend,
            "axes": self.axes,
            "time": self.time,
        });
        match self.motion {
            Motion::Orbit => {
                v["degrees"] = json!(self.degrees);
                v["seconds"] = json!(self.seconds);
            }
            Motion::Clip => {
                v["plane"] = json!(self.plane);
                v["seconds"] = json!(self.seconds);
            }
            Motion::Views => {
                v["hold"] = json!(self.hold);
                v["travel"] = json!(self.travel);
            }
            Motion::Series => {
                v["hold"] = json!(self.step_hold);
            }
        }
        if self.transparent && self.format != Format::Mp4 {
            v["transparent"] = json!(true);
        }
        self.size.put(&mut v, self.custom);
        v
    }

    /// Frames of the movie with `views` saved views and `steps` steps of
    /// the series.
    pub fn frames(&self, views: usize, steps: usize) -> usize {
        let fps = self.fps as f64;
        match self.motion {
            Motion::Orbit | Motion::Clip => animation::frame_count(self.seconds, fps),
            Motion::Views => animation::flight_count(views, self.hold, self.travel, fps),
            Motion::Series => steps * animation::frame_count(self.step_hold, fps),
        }
    }
}

/// A movie being made on its own thread.
pub struct MovieJob {
    pub progress: Arc<Progress>,
    handle: JoinHandle<Envelope>,
    /// The agent's call waiting for the movie, when an agent asked.
    reply: Option<murmel_tools::live::Job>,
    file: String,
}

impl MurmelApp {
    fn motion_name(&self, m: Motion) -> String {
        self.i18n.tr(match m {
            Motion::Orbit => "movie-motion-orbit",
            Motion::Clip => "movie-motion-clip",
            Motion::Views => "movie-motion-views",
            Motion::Series => "movie-motion-series",
        })
    }

    fn format_name(&self, f: Format) -> String {
        self.i18n.tr(match f {
            Format::Mp4 => "movie-format-mp4",
            Format::Gif => "movie-format-gif",
            Format::Png => "movie-format-png",
        })
    }

    /// Make the movie `input` asks for on a copy of the session, on a
    /// thread of its own; `reply` is the agent's call waiting for it.
    pub(crate) fn start_movie(&mut self, input: Value, reply: Option<murmel_tools::live::Job>) {
        if self.movie_job.is_some() {
            let e = Envelope::failed(
                "export.movie",
                1,
                Diagnostic::new(
                    "EXPORT007",
                    "a movie is being made; wait for it or cancel it",
                ),
            );
            match reply {
                Some(r) => r.answer(e.to_json()),
                None => self.movie.last = e.diagnostics.first().map(|d| d.message.clone()),
            }
            return;
        }
        let file = input["path"]
            .as_str()
            .map(PathBuf::from)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let mut session = self.session.clone();
        let progress = Arc::new(Progress::default());
        let p = progress.clone();
        let handle = std::thread::spawn(move || {
            murmel_tools::movie::with_progress(p, || {
                murmel_tools::call("export.movie", input, &mut session)
            })
        });
        self.movie.last = None;
        self.movie.open = true;
        self.movie_job = Some(MovieJob {
            progress,
            handle,
            reply,
            file,
        });
    }

    /// Take the result of a finished movie; keep the window drawing while
    /// one is made.
    pub(crate) fn poll_movie(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.movie_job else { return };
        if !job.handle.is_finished() {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        let Some(job) = self.movie_job.take() else {
            return;
        };
        let e = job.handle.join().unwrap_or_else(|_| {
            Envelope::failed(
                "export.movie",
                1,
                Diagnostic::new("EXPORT004", "the movie stopped unexpectedly"),
            )
        });
        self.movie.last = Some(if e.ok() {
            self.i18n.tr_args(
                "movie-done",
                &[
                    ("file", job.file.into()),
                    ("n", e.result["frames"].as_i64().unwrap_or(0).into()),
                ],
            )
        } else {
            e.diagnostics
                .first()
                .map(|d| d.message.clone())
                .unwrap_or_default()
        });
        if let Some(r) = job.reply {
            r.answer(e.to_json());
        }
    }

    pub(crate) fn movie_window(&mut self, ctx: &egui::Context) {
        if !self.movie.open {
            return;
        }
        let p = self.palette(ctx).clone();
        let mut md = self.movie.clone();
        let mut open = true;
        let mut go = false;
        let mut cancel = false;
        let clips = self.session.display.clips.len() as u32;
        let views = self.session.display.views.len();
        let steps = self.session.series.as_ref().map_or(0, |r| r.len());
        md.plane = md.plane.clamp(1, clips.max(1));
        let input = md.input();
        let pixels = SizeChoice::pixels(&input, self.session.viewport);
        let frames = md.frames(views, steps);
        let running = self
            .movie_job
            .as_ref()
            .map(|j| (j.progress.done(), j.progress.total()));
        egui::Window::new(self.i18n.tr("movie-title"))
            .id(egui::Id::unique("export-movie"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(340.0)
            .show(ctx, |ui| {
                egui::Grid::new("movie-grid")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        let label = ui.label(self.i18n.tr("movie-motion"));
                        egui::ComboBox::from_id_salt("movie-motion")
                            .selected_text(self.motion_name(md.motion))
                            .width(200.0)
                            .show_ui(ui, |ui| {
                                for m in Motion::ALL {
                                    ui.selectable_value(&mut md.motion, m, self.motion_name(m));
                                }
                            })
                            .response
                            .labelled_by(label.id);
                        ui.end_row();
                        let seconds = |ui: &mut egui::Ui, v: &mut f64, key: &str, lo: f64| {
                            let label = ui.label(self.i18n.tr(key));
                            ui.add(
                                egui::DragValue::new(v)
                                    .range(lo..=1200.0)
                                    .speed(0.1)
                                    .max_decimals(1)
                                    .suffix(" s"),
                            )
                            .labelled_by(label.id);
                            ui.end_row();
                        };
                        match md.motion {
                            Motion::Orbit => {
                                let label = ui.label(self.i18n.tr("movie-degrees"));
                                ui.add(
                                    egui::DragValue::new(&mut md.degrees)
                                        .range(-3600.0..=3600.0)
                                        .speed(1.0)
                                        .max_decimals(0)
                                        .suffix("°"),
                                )
                                .labelled_by(label.id);
                                ui.end_row();
                                seconds(ui, &mut md.seconds, "movie-seconds", 0.5);
                            }
                            Motion::Clip => {
                                let label = ui.label(self.i18n.tr("movie-plane"));
                                if clips == 0 {
                                    ui.label(
                                        RichText::new(self.i18n.tr("movie-no-plane"))
                                            .color(color(p.text_secondary)),
                                    );
                                } else {
                                    let name = |n: u32| {
                                        self.i18n
                                            .tr_args("movie-plane-n", &[("n", (n as i64).into())])
                                    };
                                    egui::ComboBox::from_id_salt("movie-plane")
                                        .selected_text(name(md.plane))
                                        .width(200.0)
                                        .show_ui(ui, |ui| {
                                            for n in 1..=clips {
                                                ui.selectable_value(&mut md.plane, n, name(n));
                                            }
                                        })
                                        .response
                                        .labelled_by(label.id);
                                }
                                ui.end_row();
                                seconds(ui, &mut md.seconds, "movie-seconds", 0.5);
                            }
                            Motion::Views => {
                                ui.label(self.i18n.tr("movie-views"));
                                let note = if views < 2 {
                                    self.i18n.tr("movie-no-views")
                                } else {
                                    self.i18n
                                        .tr_args("movie-views-n", &[("n", (views as i64).into())])
                                };
                                ui.label(RichText::new(note).color(color(p.text_secondary)));
                                ui.end_row();
                                let label = ui.label(self.i18n.tr("movie-hold"));
                                ui.add(
                                    egui::DragValue::new(&mut md.hold)
                                        .range(0.0..=60.0)
                                        .speed(0.1)
                                        .max_decimals(1)
                                        .suffix(" s"),
                                )
                                .labelled_by(label.id);
                                ui.end_row();
                                let label = ui.label(self.i18n.tr("movie-travel"));
                                ui.add(
                                    egui::DragValue::new(&mut md.travel)
                                        .range(0.1..=60.0)
                                        .speed(0.1)
                                        .max_decimals(1)
                                        .suffix(" s"),
                                )
                                .labelled_by(label.id);
                                ui.end_row();
                            }
                            Motion::Series => {
                                ui.label(self.i18n.tr("movie-steps"));
                                let note = if steps == 0 {
                                    self.i18n.tr("movie-no-series")
                                } else {
                                    self.i18n
                                        .tr_args("movie-steps-n", &[("n", (steps as i64).into())])
                                };
                                ui.label(RichText::new(note).color(color(p.text_secondary)));
                                ui.end_row();
                                let label = ui.label(self.i18n.tr("movie-step-hold"));
                                ui.add(
                                    egui::DragValue::new(&mut md.step_hold)
                                        .range(0.02..=60.0)
                                        .speed(0.01)
                                        .max_decimals(2)
                                        .suffix(" s"),
                                )
                                .labelled_by(label.id);
                                ui.end_row();
                            }
                        }
                        let label = ui.label(self.i18n.tr("movie-fps"));
                        let rate = |n: u32| {
                            self.i18n
                                .tr_args("movie-fps-n", &[("n", (n as i64).into())])
                        };
                        egui::ComboBox::from_id_salt("movie-fps")
                            .selected_text(rate(md.fps))
                            .width(200.0)
                            .show_ui(ui, |ui| {
                                for n in RATES {
                                    ui.selectable_value(&mut md.fps, n, rate(n));
                                }
                            })
                            .response
                            .labelled_by(label.id);
                        ui.end_row();
                        let label = ui.label(self.i18n.tr("movie-format"));
                        egui::ComboBox::from_id_salt("movie-format")
                            .selected_text(self.format_name(md.format))
                            .width(200.0)
                            .show_ui(ui, |ui| {
                                for f in Format::ALL {
                                    ui.selectable_value(&mut md.format, f, self.format_name(f));
                                }
                            })
                            .response
                            .labelled_by(label.id);
                        ui.end_row();
                        let label = ui.label(self.i18n.tr("export-size"));
                        egui::ComboBox::from_id_salt("movie-size")
                            .selected_text(self.size_name(md.size))
                            .width(200.0)
                            .show_ui(ui, |ui| {
                                for c in SizeChoice::ALL {
                                    ui.selectable_value(&mut md.size, c, self.size_name(c));
                                }
                            })
                            .response
                            .labelled_by(label.id);
                        ui.end_row();
                        if md.size == SizeChoice::Custom {
                            for (k, key) in [(0, "export-width"), (1, "export-height")] {
                                let label = ui.label(self.i18n.tr(key));
                                ui.add(
                                    egui::DragValue::new(&mut md.custom[k])
                                        .range(16..=murmel_render::MAX_SIDE)
                                        .suffix(" px"),
                                )
                                .labelled_by(label.id);
                                ui.end_row();
                            }
                        }
                        ui.label("");
                        let note = match &pixels {
                            Ok((w, h, _)) => self.i18n.tr_args(
                                "movie-frames",
                                &[
                                    ("n", (frames as i64).into()),
                                    ("w", (*w as i64).into()),
                                    ("h", (*h as i64).into()),
                                ],
                            ),
                            Err(d) => d.message.clone(),
                        };
                        ui.label(RichText::new(note).small().color(color(p.text_secondary)));
                        ui.end_row();
                    });
                ui.add_space(4.0);
                ui.checkbox(&mut md.legend, self.i18n.tr("export-legend"));
                ui.checkbox(&mut md.axes, self.i18n.tr("export-axes"));
                if self.session.series.is_some() {
                    ui.checkbox(&mut md.time, self.i18n.tr("export-time"));
                }
                ui.add_enabled(
                    md.format != Format::Mp4,
                    egui::Checkbox::new(&mut md.transparent, self.i18n.tr("export-transparent")),
                );
                ui.add_space(8.0);
                match running {
                    Some((done, total)) => {
                        let text = self.i18n.tr_args(
                            "movie-progress",
                            &[("k", (done as i64).into()), ("n", (total as i64).into())],
                        );
                        ui.add(
                            egui::ProgressBar::new(done as f32 / total.max(1) as f32).text(text),
                        );
                        if ui.button(self.i18n.tr("movie-cancel")).clicked() {
                            cancel = true;
                        }
                    }
                    None => {
                        let ready = pixels.is_ok()
                            && self.session.mesh.is_some()
                            && match md.motion {
                                Motion::Orbit => true,
                                Motion::Clip => clips > 0,
                                Motion::Views => views >= 2,
                                Motion::Series => steps > 0,
                            };
                        if ui
                            .add_enabled(ready, egui::Button::new(self.i18n.tr("export-save")))
                            .clicked()
                        {
                            go = true;
                        }
                    }
                }
                if let Some(last) = &md.last {
                    ui.label(RichText::new(last).small().color(color(p.text_secondary)));
                }
            });
        let esc = !ctx.text_edit_focused()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        md.open = open && !esc;
        // Closing the window leaves a movie running; its result shows when
        // the window opens again.
        self.movie = md;
        if cancel && let Some(j) = &self.movie_job {
            j.progress.cancel();
        }
        if go {
            self.export_movie();
        }
    }

    /// Ask for the file or folder (unless the window test set one) and
    /// start the movie.
    fn export_movie(&mut self) {
        let path = match self.movie.path.clone() {
            Some(p) => Some(p),
            None => {
                let d = rfd::FileDialog::new().set_title(self.i18n.tr("movie-file-title"));
                match self.movie.format {
                    Format::Mp4 => d
                        .add_filter("MP4", &["mp4"])
                        .set_file_name("murmelcraft.mp4")
                        .save_file(),
                    Format::Gif => d
                        .add_filter("GIF", &["gif"])
                        .set_file_name("murmelcraft.gif")
                        .save_file(),
                    Format::Png => d.pick_folder(),
                }
            }
        };
        let Some(path) = path else { return };
        let mut input = self.movie.input();
        input["path"] = json!(path);
        self.start_movie(input, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the window sends only the fields of its motion (the
    /// tool refuses the others), no transparency for an MP4, and counts
    /// the frames the tool will make. Breaks when a motion sends another
    /// motion's field; the window test makes one movie only.
    #[test]
    fn choices_to_input() {
        let mut d = MovieDialog::default();
        let v = d.input();
        assert_eq!(v["motion"], "orbit");
        assert_eq!(
            (v["width"].clone(), v["fps"].clone()),
            (json!(1920), json!(30))
        );
        assert!(v.get("hold").is_none() && v.get("plane").is_none());
        assert_eq!(d.frames(0, 0), 180);
        d.motion = Motion::Views;
        d.transparent = true;
        let v = d.input();
        assert!(v.get("seconds").is_none() && v.get("degrees").is_none());
        assert!(v.get("transparent").is_none(), "no transparent MP4");
        d.format = Format::Gif;
        assert_eq!(d.input()["transparent"], true);
        assert_eq!(d.frames(3, 0), 3 * 15 + 2 * 59);
        d.motion = Motion::Clip;
        assert_eq!(d.input()["plane"], 1);
        d.motion = Motion::Series;
        let v = d.input();
        assert_eq!(v["hold"], 0.25);
        assert!(v.get("travel").is_none() && v.get("seconds").is_none());
        assert_eq!(d.frames(0, 60), 60 * 8);
    }
}
