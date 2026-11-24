//! The live bridge in the window (docs/mcp.md, "Drive the open window"):
//! while agents are allowed (`settings.set_agents`, on by default) the
//! window listens on its private socket, runs each call an agent sends on
//! its own session, the same way a click does, and shows in the status bar
//! that an agent is connected, with a button that stops it.

use egui::{RichText, Role, Ui, WidgetInfo};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::style::color;

impl MurmelApp {
    /// Open or close the socket to match the setting, and run the calls
    /// that arrived since the last frame.
    pub(crate) fn serve_agents(&mut self, ctx: &egui::Context) {
        if !self.session.agents {
            // closing drops every connection: the agents' waiting calls
            // end with LIVE002
            self.bridge = None;
            self.bridge_problem = None;
            self.bridge_tried = false;
            return;
        }
        if self.bridge.is_none() && !self.bridge_tried {
            self.bridge_tried = true;
            if let Some(path) = self.bridge_path.clone() {
                let c = ctx.clone();
                match murmel_tools::live::Listener::start(
                    &path,
                    std::sync::Arc::new(move || c.request_repaint()),
                ) {
                    Ok(l) => self.bridge = Some(l),
                    Err(d) => self.bridge_problem = Some(d),
                }
            }
        }
        let jobs = self.bridge.as_ref().map(|b| b.jobs()).unwrap_or_default();
        for job in jobs {
            // A movie is made on a thread of its own, as from the movie
            // window, so the window stays live and shows its progress; the
            // agent gets the answer when it is done.
            if job.tool == "export.movie" {
                let input = job.input.clone();
                self.start_movie(input, Some(job));
                continue;
            }
            let input = self.agent_input(&job.tool, job.input.clone());
            // As over the headless server, the settings tools change this
            // window only and never write the person's files (SET001).
            let dir = self.session.config_dir.take();
            let e = self.call_envelope(&job.tool, input);
            if self.session.config_dir.is_none() {
                self.session.config_dir = dir;
            }
            job.answer(e.to_json());
        }
    }

    /// A screenshot without a size is taken at the size of the viewport,
    /// so the agent sees what the person sees.
    fn agent_input(&self, tool: &str, input: Value) -> Value {
        let mut input = match input {
            Value::Null => json!({}),
            v => v,
        };
        if tool == "view.screenshot"
            && let Some(o) = input.as_object_mut()
        {
            let [w, h] = self.session.viewport;
            o.entry("width").or_insert(json!(w.round().max(1.0) as u32));
            o.entry("height")
                .or_insert(json!(h.round().max(1.0) as u32));
        }
        input
    }

    /// How many agents drive the window now.
    pub(crate) fn agents_connected(&self) -> usize {
        self.bridge.as_ref().map_or(0, |b| b.connected())
    }

    /// The badge in the status bar while an agent is connected: its text
    /// (for the status node) and a Stop button.
    pub(crate) fn agents_badge(&mut self, ui: &mut Ui, said: &mut Vec<String>) {
        let n = self.agents_connected();
        if n == 0 {
            return;
        }
        let p = self.palette(ui.ctx()).clone();
        let stop = self.i18n.tr("status-agents-stop");
        let tip = self.i18n.tr("status-agents-stop-tip");
        let b = ui.small_button(&stop).on_hover_text(&tip).on_hover_text(
            self.i18n.tr_args(
                "settings-agents-listening",
                &[(
                    "path",
                    self.bridge_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                        .into(),
                )],
            ),
        );
        let name = format!("{stop}. {tip}");
        b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
        if b.clicked() {
            self.call("settings.set_agents", json!({"enabled": false}));
        }
        let text = self.i18n.tr_args("status-agents", &[("count", n.into())]);
        ui.label(RichText::new(&text).small().strong().color(color(p.accent)));
        said.push(text);
    }

    /// The Agents section of the Settings window.
    pub(crate) fn agents_settings(&mut self, ui: &mut Ui) {
        ui.heading(self.i18n.tr("settings-agents"));
        let mut on = self.session.agents;
        if ui
            .checkbox(&mut on, self.i18n.tr("settings-agents-allow"))
            .changed()
        {
            self.call("settings.set_agents", json!({ "enabled": on }));
        }
        let line = if let Some(d) = &self.bridge_problem {
            self.i18n.tr_args(
                "status-error",
                &[
                    ("code", d.code.into()),
                    ("message", d.message.clone().into()),
                ],
            )
        } else if let Some(b) = &self.bridge {
            self.i18n.tr_args(
                "settings-agents-listening",
                &[("path", b.path().display().to_string().into())],
            )
        } else {
            self.i18n.tr("settings-agents-off")
        };
        ui.label(RichText::new(line).small());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;
    use murmel_tools::live::Client;
    use serde_json::json;

    use crate::app::MurmelApp;

    /// Invariant: an agent's calls run in the window (the person's undo
    /// journal gets the step), a screenshot without a size comes at the
    /// viewport's size, a settings call over the bridge changes the
    /// window but writes no file, the status bar says an agent is
    /// connected, and its Stop button closes the socket so the next call
    /// says LIVE001. Breaks when the window stops serving the bridge, the
    /// settings guard goes, or Stop leaves the socket open; the wire tests
    /// in murmel-tools run without a window.
    #[test]
    fn an_agent_drives_the_window_until_stopped() {
        let dir = std::env::temp_dir().join(format!("murmel-app-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let socket = dir.join("sock/live.sock");
        let config = dir.join("config");
        let shot = dir.join("shot.png");
        let channel = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        let s = socket.clone();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .build_eframe(move |cc| {
                let mut app = MurmelApp::new(cc, None).unwrap();
                app.session.config_dir = Some(config.clone());
                app.session.agents = true;
                app.bridge_path = Some(s.clone());
                app.open_path(channel.clone());
                app
            });
        h.run_steps(3);
        assert!(h.state().bridge.is_some(), "{:?}", h.state().bridge_problem);

        let (p, out) = (socket.clone(), shot.clone());
        let agent = std::thread::spawn(move || {
            let mut c = Client::new(p);
            let top = c.call("view.standard", json!({"view": "top"})).unwrap();
            let theme = c
                .call("settings.set_theme", json!({"theme": "light"}))
                .unwrap();
            let shot = c.call("view.screenshot", json!({"path": out})).unwrap();
            (c, top, theme, shot)
        });
        while !agent.is_finished() {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (mut c, top, theme, shot) = agent.join().unwrap();
        assert_eq!(top["status"], "finished", "{top}");
        assert_eq!(h.state().session.journal.undo.len(), 1, "an undo step");
        // applied to the window, not saved
        assert_eq!(theme["diagnostics"][0]["code"], "SET001", "{theme}");
        assert_eq!(h.state().session.theme, murmel_engine::Theme::Light);
        assert!(
            !h.state()
                .session
                .config_dir
                .as_ref()
                .unwrap()
                .join("settings.toml")
                .exists()
        );
        assert_eq!(shot["status"], "finished", "{shot}");
        let [w, vh] = h.state().session.viewport;
        assert_eq!(shot["result"]["width"], w.round() as u64);
        assert_eq!(shot["result"]["height"], vh.round() as u64);

        // the badge, and Stop
        h.run_steps(2);
        h.get_by_label("Agent connected");
        h.get_by_label("Stop. Stop agents driving this window (switch them on again in Settings)")
            .click();
        h.run_steps(2);
        assert!(!h.state().session.agents);
        assert!(h.state().bridge.is_none());
        assert!(!socket.exists());
        let e = c.call("view.get_camera", json!({})).unwrap_err();
        assert_eq!(e.code, "LIVE001", "{e:?}");
    }
}
