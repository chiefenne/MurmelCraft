//! The window: title bar, toolbar, viewport and status bar (V0.1 of the
//! UI spec). The window holds no state of its own beyond what it draws:
//! every action is a call of a registered tool on the session.

use std::path::PathBuf;

use egui::{Align, Color32, Layout, RichText, Role, Sense, Ui, WidgetInfo};
use murmel_engine::{Session, Theme, Tokens, settings};
use murmel_tools::Diagnostic;
use serde_json::{Value, json};

use crate::focus::Region;
use crate::i18n::{I18n, count};
use crate::style::{self, color, icon};
use crate::viewport::GpuView;

pub struct MurmelApp {
    pub(crate) session: Session,
    pub(crate) i18n: I18n,
    pub(crate) gpu: Option<GpuView>,
    pub(crate) error: Option<Diagnostic>,
    /// When the status bar first showed `error`; it goes away
    /// `ERROR_SECONDS` later, or sooner when a journalled call works.
    pub(crate) error_since: Option<f64>,
    /// The session file last saved or loaded, where Ctrl + S saves.
    pub(crate) session_path: Option<PathBuf>,
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
    /// The saved view being renamed in the tree: its name and the text
    /// typed so far.
    pub(crate) view_edit: Option<(String, String)>,
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
    /// When the player last showed a step (while a series plays).
    pub(crate) played_at: Option<f64>,
    /// The edges hidden while the view moves when frames with them are
    /// slow.
    pub(crate) moving_edges: crate::edges::MovingEdges,
    /// The rotate row under the view cube: axis and degrees.
    pub(crate) rotate_axis: murmel_engine::ScreenAxis,
    pub(crate) rotate_deg: f64,
    /// The quality threshold while its slider is dragged (the surface is
    /// rebuilt when the drag ends).
    pub(crate) threshold_edit: Option<f64>,
    pub(crate) histogram: Option<crate::quality::HistogramCache>,
    /// The world point the current viewport orbit turns about.
    pub(crate) orbit_pivot: Option<[f64; 3]>,
    /// The toolbar leaves out group names when the full one does not fit;
    /// the width the full one needs, as last measured.
    pub(crate) toolbar_compact: bool,
    pub(crate) toolbar_full: f32,
    /// The floating view cube's position while its grip is dragged
    /// (saved by `settings.set_view_cube` when the drag ends).
    pub(crate) cube_move: Option<[f64; 2]>,
    /// The cube was dragged last frame, so auto-hide keeps it shown.
    pub(crate) cube_busy: bool,
    /// The auto-hide distance while it is edited.
    pub(crate) cube_distance_edit: Option<f64>,
    /// The memory budget being typed or dragged (GiB), set when done.
    pub(crate) memory_edit: Option<f64>,
    /// Until when a run of wheel steps keeps its undo group open.
    wheel_until: f64,
    /// A drag of a clip plane handle under way.
    pub(crate) clip_drag: Option<crate::clip_handle::ClipDrag>,
    /// Where the clip plane's handles were drawn last frame.
    pub(crate) clip_spots: Vec<crate::clip_handle::Spot>,
    /// The tilt arc whose rotation plane is drawn this frame (hovered or
    /// dragged), for the window tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) rotation_plane: Option<crate::clip_handle::Handle>,
    /// A drag of a streamline seed handle under way.
    pub(crate) stream_drag: Option<crate::stream_handle::StreamDrag>,
    /// Where the seed handles were drawn last frame.
    pub(crate) stream_spots: Vec<crate::stream_handle::SeedSpot>,
    /// The colour window of a tree item, while open.
    pub(crate) color_edit: Option<crate::color_edit::ColorEdit>,
    /// The export window and its settings (File, "Export image…").
    pub(crate) export: crate::export_dialog::ExportDialog,
    /// The movie window and its settings (File, "Export movie…").
    pub(crate) movie: crate::movie_dialog::MovieDialog,
    /// The Derived fields window ("Derived fields…").
    pub(crate) derived: crate::derived::DerivedWindow,
    /// The Probe history window and its reading.
    pub(crate) history: crate::probe_history::HistoryWindow,
    /// The colour range over the steps of a series, read on its own
    /// thread (range mode steps).
    pub(crate) steps_range: crate::player::StepsRangeJob,
    /// The movie being made on its own thread, if one is.
    pub(crate) movie_job: Option<crate::movie_dialog::MovieJob>,
    /// The large mesh being read on its own thread, if one is.
    pub(crate) opening: Option<crate::opening::Opening>,
    /// Meshes of at least this many bytes are read on their own thread.
    pub(crate) background_bytes: u64,
    /// The next background read waits for a message here (window tests
    /// hold it to click Cancel in time).
    #[cfg(test)]
    pub(crate) opening_gate: Option<std::sync::mpsc::Receiver<()>>,
    /// Keyboard focus: F6 regions, the focus ring, the arrow keys.
    pub(crate) focus: crate::focus::Focus,
    /// The width of the panel toggles at the right of the toolbar, as
    /// last drawn (the groups take the rest).
    toggles_w: f32,
    /// The same for the buttons at the right of the title bar.
    title_buttons_w: f32,
    /// The live bridge: where the window listens for agents (`None`: it
    /// does not), the listener while agents are allowed, why it could not
    /// start, and whether it was tried since agents were last allowed.
    pub(crate) bridge_path: Option<PathBuf>,
    pub(crate) bridge: Option<murmel_tools::live::Listener>,
    pub(crate) bridge_problem: Option<Diagnostic>,
    pub(crate) bridge_tried: bool,
    /// The measure mode (M, Shift + M, Shift + P) while it is on.
    pub(crate) measuring: Option<crate::measure::Measuring>,
    /// What measure points snap to (the toolbar's Snap menu).
    pub(crate) snap_kinds: Vec<murmel_engine::SnapKind>,
    /// The snap under the pointer, while nothing moves.
    pub(crate) snap_hover: Option<crate::measure::SnapHover>,
    /// Where the labels of measurements and probes were drawn last frame.
    pub(crate) measure_labels: Vec<crate::measure::Label>,
    /// The measurement or probe being renamed in the tree: its id and
    /// the text typed so far.
    pub(crate) measure_edit: Option<(String, String)>,
    /// Where the viewport was drawn last frame, for the window tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) viewport_rect: egui::Rect,
}

/// A pause of the wheel this long (seconds) ends its undo step.
const WHEEL_PAUSE: f64 = 0.3;
/// How long a message stays in the status bar after it is first shown.
const ERROR_SECONDS: f64 = 10.0;

impl MurmelApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Result<Self, String> {
        let tokens = Tokens::builtin()?;
        style::install(&cc.egui_ctx, &tokens);
        let mut session = Session::new(tokens);
        // The window opens a mesh first and hashes its files afterwards
        // (remembering them, so an unchanged file is not hashed again),
        // and a clip plane being dragged is cut on the GPU while the exact
        // cut is built on a thread of its own.
        session.hash_after_open = true;
        session.remember_checked = true;
        session.cache_dir = settings::cache_dir();
        session.defer_clips = true;
        session.defer_blocks = true;
        let mut error = None;
        if let Some(dir) = settings::config_dir() {
            let (st, nav, problems) = settings::load(&dir, cfg!(target_os = "macos"));
            session.theme = st.theme;
            session.navigation = nav;
            session.view_cube = st.view_cube;
            session.agents = st.agents.enabled;
            session.defaults = st.defaults;
            session.formulas = st.formulas;
            session.memory_budget = st.memory.budget_gib;
            murmel_engine::defaults::start_view(&mut session);
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
            error_since: None,
            session_path: None,
            drag: None,
            hover: None,
            settings_open: false,
            installed_theme: None,
            show_tree: true,
            show_props: true,
            tree_filter: String::new(),
            view_edit: None,
            focus_filter: false,
            props: None,
            commands: murmel_tools::commands::commands(),
            palette: None,
            shortcuts_open: false,
            flight: None,
            played_at: None,
            now: 0.0,
            moving_edges: Default::default(),
            rotate_axis: murmel_engine::ScreenAxis::Z,
            rotate_deg: 90.0,
            threshold_edit: None,
            histogram: None,
            orbit_pivot: None,
            toolbar_compact: false,
            toolbar_full: 0.0,
            cube_move: None,
            cube_busy: false,
            cube_distance_edit: None,
            memory_edit: None,
            wheel_until: 0.0,
            clip_drag: None,
            clip_spots: Vec::new(),
            rotation_plane: None,
            stream_drag: None,
            stream_spots: Vec::new(),
            color_edit: None,
            export: Default::default(),
            movie: Default::default(),
            derived: Default::default(),
            movie_job: None,
            history: Default::default(),
            steps_range: Default::default(),
            opening: None,
            background_bytes: crate::opening::BACKGROUND_BYTES,
            #[cfg(test)]
            opening_gate: None,
            focus: crate::focus::Focus::default(),
            toggles_w: 90.0,
            title_buttons_w: 110.0,
            bridge_path: murmel_tools::live::socket_path(),
            bridge: None,
            bridge_problem: None,
            bridge_tried: false,
            measuring: None,
            snap_kinds: murmel_engine::SnapKind::DEFAULT.to_vec(),
            snap_hover: None,
            measure_labels: Vec::new(),
            measure_edit: None,
            viewport_rect: egui::Rect::NOTHING,
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
        let e = self.call_envelope(id, input);
        e.ok().then_some(e.result)
    }

    /// The same, with the whole envelope (for the agents' calls).
    pub(crate) fn call_envelope(&mut self, id: &str, input: Value) -> murmel_tools::Envelope {
        let before = self.session.camera.clone();
        let e = murmel_tools::call(id, input, &mut self.session);
        // Jumps of the view fly there in 250 ms; drags and wheel steps
        // follow the pointer directly.
        if e.ok()
            && matches!(
                id,
                "view.standard"
                    | "view.rotate"
                    | "view.fit"
                    | "views.go"
                    | "journal.undo"
                    | "journal.redo"
            )
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
            // The person (or an agent) moved on: an older message no
            // longer describes what is on screen.
            if id == "mesh.open" || murmel_tools::JOURNALED.contains(&id) {
                self.set_error(None);
            }
            // Points placed on another mesh mean nothing on this one.
            if matches!(id, "mesh.open" | "mesh.close" | "session.load") {
                self.measuring = None;
                self.snap_hover = None;
                self.measure_edit = None;
            }
            if id == "session.load" {
                // The first warning (a changed mesh, skipped items) shows
                // in the status bar.
                let first = e.result["warnings"]
                    .as_array()
                    .and_then(|w| w.first())
                    .and_then(|w| {
                        let c = murmel_tools::code(w["code"].as_str()?)?;
                        Some(Diagnostic::new(c.code, w["message"].as_str()?))
                    });
                self.set_error(first);
            }
        } else {
            self.set_error(e.diagnostics.first().cloned());
        }
        e
    }

    /// Show a message in the status bar (or none); its time starts when
    /// it is first drawn.
    pub(crate) fn set_error(&mut self, d: Option<Diagnostic>) {
        self.error = d;
        self.error_since = None;
    }

    /// Drop the status bar's message `ERROR_SECONDS` after it was first
    /// shown, and wake the window then if nothing else does.
    fn expire_error(&mut self, ctx: &egui::Context) {
        if self.error.is_none() {
            return;
        }
        let since = *self.error_since.get_or_insert(self.now);
        let left = since + ERROR_SECONDS - self.now;
        if left <= 0.0 {
            self.set_error(None);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
        }
    }

    /// Group the calls of one mouse gesture (while a button is held) and
    /// of one run of wheel or pinch steps (until a pause of
    /// `WHEEL_PAUSE`) into one undo step (UI spec: undo steps are
    /// gestures, not frames).
    fn group_steps(&mut self, ctx: &egui::Context) {
        let (held, wheel) = ctx.input(|i| {
            (
                i.pointer.any_down(),
                i.smooth_scroll_delta != egui::Vec2::ZERO || i.zoom_delta() != 1.0,
            )
        });
        if wheel {
            self.wheel_until = self.now + WHEEL_PAUSE;
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(WHEEL_PAUSE));
        }
        let want = held || self.now < self.wheel_until;
        if want != self.session.journal.group_open() {
            self.call("journal.group", json!({ "open": want }));
        }
    }

    /// Open a mesh, or load a session file (`*.murmel`). A large mesh (a
    /// session's too) is read on a thread of its own first.
    pub(crate) fn open_path(&mut self, path: PathBuf) {
        self.open_path_as(path, murmel_engine::OpenMode::Fit);
    }

    /// [`MurmelApp::open_path`], a mesh opening as `mode` asks.
    pub(crate) fn open_path_as(&mut self, path: PathBuf, mode: murmel_engine::OpenMode) {
        use crate::opening::Then;
        let session = path
            .extension()
            .is_some_and(|e| e == murmel_engine::session_file::EXTENSION);
        if session {
            let mesh = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| murmel_engine::SessionFile::parse(&t).ok())
                .and_then(|f| f.mesh_path(&path))
                .filter(|m| self.session.mesh.as_ref().is_none_or(|o| &o.path != m));
            let background =
                mesh.is_some_and(|m| self.open_in_background(m, Then::Session(path.clone())));
            if !background {
                self.open_now(Then::Session(path));
            }
        } else if !self.open_in_background(path.clone(), Then::Mesh(path.clone(), mode)) {
            self.open_now(Then::Mesh(path, mode));
        }
    }

    /// Save the session where it was saved or loaded last; a new name
    /// (`ask`), or the first save, asks for the file.
    pub(crate) fn save_session(&mut self, ask: bool) {
        let path = match (&self.session_path, ask) {
            (Some(p), false) => Some(p.clone()),
            _ => {
                let ext = murmel_engine::session_file::EXTENSION;
                let mesh = self.session.mesh.as_ref().map(|m| m.path.clone());
                let stem = mesh
                    .as_ref()
                    .and_then(|p| p.file_stem())
                    .map_or("session".into(), |s| s.to_string_lossy().into_owned());
                let mut d = rfd::FileDialog::new()
                    .set_title(self.i18n.tr("session-save-title"))
                    .add_filter(self.i18n.tr("open-filter-session"), &[ext])
                    .set_file_name(format!("{stem}.{ext}"));
                if let Some(dir) = mesh.as_ref().and_then(|p| p.parent()) {
                    d = d.set_directory(dir);
                }
                d.save_file()
            }
        };
        if let Some(p) = path
            && self.call("session.save", json!({ "path": p })).is_some()
        {
            self.session_path = Some(p);
        }
    }

    pub(crate) fn load_session_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title(self.i18n.tr("session-load-title"))
            .add_filter(
                self.i18n.tr("open-filter-session"),
                &[murmel_engine::session_file::EXTENSION],
            )
            .pick_file();
        if let Some(p) = picked {
            self.open_path(p);
        }
    }

    pub(crate) fn open_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title(self.i18n.tr("toolbar-open-tip"))
            .add_filter(
                self.i18n.tr("open-filter-meshes"),
                &[
                    "vtu",
                    "vtkhdf",
                    "foam",
                    "OpenFOAM",
                    murmel_engine::session_file::EXTENSION,
                ],
            )
            .add_filter(self.i18n.tr("open-filter-vtu"), &["vtu"])
            .add_filter(self.i18n.tr("open-filter-vtkhdf"), &["vtkhdf"])
            .add_filter(self.i18n.tr("open-filter-foam"), &["foam", "OpenFOAM"])
            .add_filter(self.i18n.tr("open-filter-all"), &["*"])
            .pick_file();
        if let Some(p) = picked {
            self.open_path(p);
        }
    }

    /// Pick an OpenFOAM case, `constant` or `polyMesh` folder.
    pub(crate) fn open_folder_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title(self.i18n.tr("cmd-open-folder"))
            .pick_folder();
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
            ui.add(egui::Label::new(RichText::new(file).color(color(p.text_secondary))).truncate());
            // The buttons at the right, drawn left to right so that Tab
            // visits them in the order they show.
            let w = self.title_buttons_w;
            ui.add_space((ui.available_width() - w).max(0.0));
            let start = ui.cursor().min.x;
            let (glyph, next) = match self.session.theme {
                Theme::Dark => (icon::MOON, Theme::Light),
                Theme::Light => (icon::SUN, Theme::System),
                Theme::System => (icon::MONITOR, Theme::Dark),
            };
            let theme = self
                .i18n
                .tr(&format!("theme-{}", theme_key(self.session.theme)));
            let name = self.i18n.tr_args("theme-name", &[("theme", theme.into())]);
            let b = icon_button(ui, glyph, &name, &self.i18n.tr("settings-theme"), false);
            self.focus.offer(Region::Title, &b);
            if b.clicked() {
                self.call("settings.set_theme", json!({ "theme": next }));
            }
            let palette = self.i18n.tr("cmd-palette");
            let tip = format!("{palette} ({})", crate::commands::key_text("Ctrl+K"));
            if icon_button(ui, icon::SEARCH, &palette, &tip, self.palette.is_some()).clicked() {
                self.open_palette(String::new());
            }
            let settings = self.i18n.tr("settings-title");
            let b = icon_button(
                ui,
                icon::SETTINGS,
                &settings,
                &self.i18n.tr("settings-tip"),
                self.settings_open,
            );
            if b.clicked() {
                self.settings_open = !self.settings_open;
            }
            self.title_buttons_w = b.rect.max.x - start;
        });
    }

    /// The toolbar: the panel toggles at the right, the groups from the
    /// left. When the window is too narrow for all groups they scroll
    /// sideways under the toggles instead of drawing over them.
    fn toolbar(&mut self, ui: &mut Ui) {
        // Groups first and the toggles after them, so that Tab visits the
        // toolbar from left to right; the groups take what the toggles
        // (as wide as last frame) leave.
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            let room = (ui.available_width() - self.toggles_w).max(0.0);
            let size = egui::vec2(room, ui.available_height());
            ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
                ui.set_width(room);
                let out = egui::ScrollArea::horizontal()
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
                // Compact (no group names, short Colour by) when the full
                // toolbar would not fit, so the last group (Select, Faces)
                // never scrolls out of sight; the full width is measured
                // whenever the full toolbar is shown.
                let room = out.inner_rect.width();
                if !self.toolbar_compact {
                    self.toolbar_full = out.content_size.x;
                }
                let compact = needs_compact(self.toolbar_full, room);
                if compact != self.toolbar_compact {
                    self.toolbar_compact = compact;
                    ui.ctx().request_repaint();
                }
            });
            let start = ui.cursor().min.x;
            ui.separator();
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
            let props = self.i18n.tr("toolbar-properties");
            let b = icon_button(
                ui,
                icon::INFO,
                &props,
                &self.i18n.tr("toolbar-properties-tip"),
                self.show_props,
            );
            if b.clicked() {
                self.show_props = !self.show_props;
            }
            let w = b.rect.max.x - start;
            if (w - self.toggles_w).abs() > 0.5 {
                self.toggles_w = w;
                ui.ctx().request_repaint();
            }
        });
    }

    /// The name of a toolbar group, left out when the toolbar is compact.
    fn group_label(&self, ui: &mut Ui, group: &str, secondary: Color32) {
        if !self.toolbar_compact {
            ui.label(
                RichText::new(
                    self.i18n
                        .tr(&format!("toolbar-group-{group}"))
                        .to_uppercase(),
                )
                .small()
                .color(secondary),
            );
        }
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
        b.widget_info(|| WidgetInfo::labeled(Role::Button, true, open.clone()));
        self.focus.offer(Region::Toolbar, &b);
        if b.clicked() {
            self.open_dialog();
        }
        ui.separator();
        self.group_label(ui, "view", secondary);
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
        let name = self
            .i18n
            .tr_args("projection-name", &[("mode", long.clone().into())]);
        b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
        if b.clicked() {
            self.call("view.set_camera", json!({ "projection": proj.next() }));
        }
        // Right-click: start the window with this projection.
        let mut keep = None;
        b.context_menu(|ui| keep = self.view_default_entries(ui, "projection"));
        if let Some((tool, input)) = keep {
            self.call(tool, input);
        }
        ui.separator();
        self.group_label(ui, "display", secondary);
        let mode = self.session.display.render_mode;
        let mut pick_mode = None;
        let current = self.i18n.tr(&format!("render-mode-{}", mode.key()));
        let name = self
            .i18n
            .tr_args("render-mode-name", &[("mode", current.clone().into())]);
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
        let r = r
            .response
            .on_hover_text(self.i18n.tr("toolbar-render-mode-tip"));
        r.widget_info(|| WidgetInfo::labeled(Role::ComboBox, true, name.clone()));
        if let Some(m) = pick_mode.filter(|&m| m != mode) {
            self.call("display.render_mode", json!({ "mode": m }));
        }
        // Right-click: start the window in this render mode (a menu id of
        // its own, as the combo box's list has the response's).
        let mut keep = None;
        egui::Popup::context_menu(&r)
            .id(r.id.with("keep-default"))
            .show(|ui| keep = self.view_default_entries(ui, "render_mode"));
        if let Some((tool, input)) = keep {
            self.call(tool, input);
        }
        let by = self.session.display.color_by;
        let metric = self.session.display.metric;
        let mut pick = None;
        let mut open_derived = false;
        let label = self.i18n.tr("toolbar-color-by");
        let current = self.color_by_name();
        let text = if self.toolbar_compact {
            current.clone()
        } else {
            format!("{label}: {current}")
        };
        let r = egui::ComboBox::from_id_salt("color-by")
            .selected_text(text)
            .show_ui(ui, |ui| {
                for c in murmel_engine::ColorBy::ALL {
                    if matches!(
                        c,
                        murmel_engine::ColorBy::Quality | murmel_engine::ColorBy::Field
                    ) {
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
                // The fields of a result, each by name (the legend picks
                // the component and the style).
                let fields: Vec<String> = self
                    .session
                    .fields()
                    .map(|f| f.names().iter().map(|n| n.to_string()).collect())
                    .unwrap_or_default();
                if !fields.is_empty() {
                    ui.separator();
                }
                let shown = (by == murmel_engine::ColorBy::Field)
                    .then(|| self.session.display.field.as_ref().map(|c| c.name.clone()))
                    .flatten();
                let any = !fields.is_empty();
                for n in fields {
                    let on = shown.as_deref() == Some(n.as_str());
                    let text = self
                        .i18n
                        .tr_args("field-menu-item", &[("name", n.clone().into())]);
                    if ui.selectable_label(on, text).clicked() && !on {
                        pick = Some(json!({"by": "field", "field": n}));
                    }
                }
                if any
                    && ui
                        .selectable_label(false, self.i18n.tr("cmd-derived"))
                        .clicked()
                {
                    open_derived = true;
                }
            });
        let tip = format!(
            "{label}: {current}\n{}",
            self.i18n.tr("toolbar-color-by-tip")
        );
        r.response.on_hover_text(tip).widget_info(|| {
            WidgetInfo::labeled(Role::ComboBox, true, format!("{label}: {current}"))
        });
        if let Some(input) = pick.filter(|i| {
            i["by"] != json!(by) || i.get("metric").is_some() || i.get("field").is_some()
        }) {
            self.call("display.color_by", input);
        }
        if open_derived {
            self.open_derived();
        }
        ui.separator();
        self.clip_group(ui, secondary);
        ui.separator();
        self.measure_group(ui);
        ui.separator();
        self.select_group(ui, secondary);
    }

    /// The name of the colouring, with the metric in colour mode quality.
    pub(crate) fn color_by_name(&self) -> String {
        let d = &self.session.display;
        let by = self.i18n.tr(&format!("color-by-{}", d.color_by.key()));
        match d.color_by {
            murmel_engine::ColorBy::Quality => format!(
                "{by}: {}",
                self.i18n.tr(&format!("metric-{}", d.metric.key()))
            ),
            murmel_engine::ColorBy::Field => match self.session.field_values() {
                Some(v) => format!("{by}: {}", v.choice.title(v.vector)),
                None => by,
            },
            _ => by,
        }
    }

    /// The Clip group of the toolbar: the axis buttons (X, Y, Z), the
    /// cut mode, flip and the plane's position.
    fn clip_group(&mut self, ui: &mut Ui, secondary: Color32) {
        self.group_label(ui, "clip", secondary);
        let clip = self.session.display.clip().copied();
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
            let current = self.i18n.tr(&format!("clip-mode-{}", mode.key()));
            let name = self
                .i18n
                .tr_args("clip-mode-name", &[("mode", current.clone().into())]);
            let r = egui::ComboBox::from_id_salt("clip-mode")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for m in murmel_engine::ClipMode::ALL {
                        let name = self.i18n.tr(&format!("clip-mode-{}", m.key()));
                        if ui.selectable_label(m == mode, name).clicked() {
                            pick = Some(m);
                        }
                    }
                });
            r.response
                .on_hover_text(self.i18n.tr("toolbar-clip-mode-tip"))
                .widget_info(|| WidgetInfo::labeled(Role::ComboBox, true, name.clone()));
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
        self.group_label(ui, "select", secondary);
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
        if let Some(h) = self.measure_hint() {
            return h;
        }
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
        // What the bar says, for its node in the accessibility tree: the
        // message first, then the hint.
        let mut said: Vec<String> = Vec::new();
        let hint = self.hint();
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new(&hint).small().color(secondary));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(self.i18n.tr("status-units"))
                        .small()
                        .color(secondary),
                );
                self.agents_badge(ui, &mut said);
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
                if self.opening_status(ui, &mut said)
                    || self.blocks_status(ui, &mut said)
                    || self.preview_status(ui, &mut said)
                {
                } else if let Some(e) = &self.error {
                    let text = self.i18n.tr_args(
                        "status-error",
                        &[
                            ("code", e.code.into()),
                            ("message", e.message.clone().into()),
                        ],
                    );
                    // A warning (a session's changed mesh) is amber.
                    let red = color(if e.severity == murmel_tools::Severity::Warning {
                        p.status_warn
                    } else {
                        p.status_error
                    });
                    ui.label(RichText::new(&text).small().color(red))
                        .on_hover_text(e.fix.unwrap_or(""));
                    // The alert glyph is painted left of the text, not
                    // put in it: a screen reader would read it.
                    let (g, _) = ui.allocate_exact_size(egui::vec2(14.0, 16.0), Sense::hover());
                    ui.painter().text(
                        g.center(),
                        egui::Align2::CENTER_CENTER,
                        icon::ALERT,
                        egui::FontId::proportional(12.0),
                        red,
                    );
                    said.push(text);
                } else if let Some(m) = &self.session.mesh {
                    let s = (m.surface.cells, m.surface.faces);
                    let sel = &self.session.display.selection;
                    let cells = self.i18n.tr_args(
                        "status-cells",
                        &[("cells", count(s.0).into()), ("faces", count(s.1).into())],
                    );
                    if sel.len() > 1 {
                        let text = self
                            .i18n
                            .tr_args("status-selected-many", &[("count", sel.len().into())]);
                        ui.label(RichText::new(&text).small());
                        said.push(text);
                    } else if let Some(&sel) = sel.first() {
                        let id = sel.id(&m.mesh);
                        let (area, name) = id.split_once(':').unwrap_or(("mesh", ""));
                        let name = if area == "kind" {
                            self.i18n.tr(&format!("kind-{name}"))
                        } else {
                            name.to_string()
                        };
                        let area = self.i18n.tr(&format!("props-area-{area}"));
                        let text = self.i18n.tr_args(
                            "status-selected",
                            &[("area", area.into()), ("item", name.into())],
                        );
                        ui.label(RichText::new(&text).small());
                        said.push(text);
                    }
                    ui.label(RichText::new(&cells).small());
                    said.push(cells);
                    if let Some(r) = &self.session.series {
                        let text = match murmel_tools::series::shown_time(&self.session) {
                            Some(t) => self.i18n.tr_args(
                                "status-series-time",
                                &[
                                    ("time", format!("{t:.4}").into()),
                                    ("step", (r.entry().step as i64).into()),
                                ],
                            ),
                            None => self.i18n.tr_args(
                                "status-series-step",
                                &[("step", (r.entry().step as i64).into())],
                            ),
                        };
                        ui.label(RichText::new(&text).small().monospace());
                        said.push(text);
                    }
                }
            });
        });
        // The bar as one status node F6 can reach, so a screen reader
        // reads its message.
        said.push(hint);
        let name = said.join(". ");
        let resp = ui.interact(
            ui.max_rect(),
            ui.scope_id().with("status-node"),
            Sense::focusable_noninteractive(),
        );
        resp.widget_info(|| WidgetInfo::labeled(Role::Status, true, name.clone()));
        self.focus.offer(Region::Status, &resp);
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
                ui.heading(self.i18n.tr("settings-view-cube"));
                self.cube_menu(ui, false);
                ui.add_space(8.0);
                self.agents_settings(ui);
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
                ui.add_space(8.0);
                self.defaults_settings(ui);
                ui.add_space(8.0);
                self.memory_settings(ui);
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

impl MurmelApp {
    /// The memory budget: half the computer's memory, or one you set. A
    /// blocked mesh that needs more opens as an overview.
    fn memory_settings(&mut self, ui: &mut Ui) {
        use murmel_engine::memory::{MAX_GIB, MIN_GIB, gib, system_bytes};
        ui.heading(self.i18n.tr("settings-memory"));
        let mut automatic = self.session.memory_budget.is_none();
        let label = match system_bytes() {
            Some(b) => self
                .i18n
                .tr_args("settings-memory-half", &[("total", gib(b).into())]),
            None => self.i18n.tr("settings-memory-half-unknown"),
        };
        if ui.checkbox(&mut automatic, label).changed() {
            let input = if automatic {
                json!({ "automatic": true })
            } else {
                json!({ "budget_gib": gib(self.session.budget_bytes()).max(MIN_GIB) })
            };
            self.call("settings.set_memory", input);
        }
        if self.session.memory_budget.is_some() {
            let name = self.i18n.tr("settings-memory-budget");
            let mut g = self
                .memory_edit
                .or(self.session.memory_budget)
                .unwrap_or(MIN_GIB);
            ui.horizontal(|ui| {
                ui.label(&name);
                let r = ui.add(
                    egui::DragValue::new(&mut g)
                        .range(MIN_GIB..=MAX_GIB)
                        .speed(0.25)
                        .suffix(" GB")
                        .max_decimals(2),
                );
                r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name.clone()));
                if r.changed() {
                    self.memory_edit = Some(g);
                }
                let done = r.drag_stopped()
                    || (r.changed() && !r.dragged() && !r.has_focus())
                    || r.lost_focus();
                if done && let Some(v) = self.memory_edit.take() {
                    self.call("settings.set_memory", json!({ "budget_gib": v }));
                }
            });
        }
        ui.label(
            RichText::new(self.i18n.tr_args(
                "settings-memory-note",
                &[("budget", gib(self.session.budget_bytes()).into())],
            ))
            .small(),
        );
    }

    /// Name the pane `ui` is drawn in and remember where it is (F6).
    fn region(&mut self, ui: &Ui, region: Region) {
        crate::focus::name_pane(ui, &self.i18n.tr(region.key()));
        self.focus.region(region, ui.max_rect());
    }

    /// F6 and Shift + F6: focus the first widget of the next region.
    pub(crate) fn cycle_region(&mut self, ctx: &egui::Context, forward: bool) {
        if let Some((_, id)) = self.focus.next(ctx, forward) {
            ctx.memory_mut(|m| m.request_focus(id));
            self.focus.visible = true;
        }
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
        self.expire_error(&ctx);
        self.focus.begin(&ctx);
        self.apply_theme(&ctx);
        self.group_steps(&ctx);
        self.serve_agents(&ctx);
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
            .show(ui, |ui| {
                self.region(ui, Region::Title);
                self.title_bar(ui)
            });
        egui::Panel::top("toolbar")
            .exact_size(40.0)
            .frame(bar(p.bg_panel))
            .show(ui, |ui| {
                self.region(ui, Region::Toolbar);
                self.toolbar(ui)
            });
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(bar(p.bg_app))
            .show(ui, |ui| {
                self.region(ui, Region::Status);
                self.status_bar(ui)
            });
        if self.session.series.is_some() {
            egui::Panel::bottom("player")
                .exact_size(34.0)
                .frame(bar(p.bg_panel))
                .show(ui, |ui| {
                    self.region(ui, Region::Player);
                    self.player_bar(ui)
                });
        }
        if self.session.charts.as_ref().is_some_and(|c| c.visible) {
            egui::Panel::bottom("charts")
                .default_size(300.0)
                .size_range(180.0..=640.0)
                .resizable(true)
                .frame(
                    egui::Frame::NONE
                        .fill(color(p.bg_panel))
                        .inner_margin(egui::Margin::symmetric(12, 4))
                        .stroke(egui::Stroke::new(1.0, color(p.line))),
                )
                .show(ui, |ui| {
                    self.region(ui, Region::Charts);
                    self.chart_panel(ui)
                });
        }
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
                .show(ui, |ui| {
                    self.region(ui, Region::Tree);
                    self.model_tree(ui)
                });
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
                .show(ui, |ui| {
                    self.region(ui, Region::Properties);
                    self.properties(ui)
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(color(p.viewport_bottom)))
            .show(ui, |ui| {
                self.region(ui, Region::Viewport);
                self.viewport(ui, frame)
            });
        self.settings_window(&ctx);
        self.shortcuts_window(&ctx);
        self.palette_window(&ctx);
        self.color_window(&ctx);
        self.export_window(&ctx);
        self.derived_window(&ctx);
        self.poll_movie(&ctx);
        self.poll_probe_history(&ctx);
        self.probe_history_window(&ctx);
        self.play_series(&ctx);
        self.read_steps_range(&ctx);
        self.poll_opening(&ctx);
        self.poll_blocks(&ctx);
        self.poll_preview(&ctx);
        self.poll_digest(&ctx);
        self.poll_surface(&ctx);
        self.movie_window(&ctx);
        crate::focus::ring(&ctx, &self.focus, color(p.accent));
        self.focus.end(&ctx);
    }
}

/// Whether the toolbar must leave out its group names: when the full
/// toolbar, `full` points wide, is wider than the `room` it has (with half
/// a point for rounding). The full width is measured only while the full
/// toolbar shows, so the decision does not flicker.
pub(crate) fn needs_compact(full: f32, room: f32) -> bool {
    full > room + 0.5
}

#[cfg(test)]
mod toolbar_tests {
    /// Invariant: the toolbar turns compact exactly when the full one
    /// would scroll its last group (Select, Faces) out of sight, and back
    /// when there is room again. Breaks when the comparison is turned
    /// around or the full width is compared with itself; the window is
    /// otherwise checked by hand (a 1280 px window with colour by
    /// quality, which hid Faces before).
    #[test]
    fn compact_only_when_the_full_toolbar_does_not_fit() {
        assert!(super::needs_compact(1260.0, 1100.0));
        assert!(!super::needs_compact(1100.0, 1100.0));
        assert!(!super::needs_compact(1000.0, 1300.0));
    }
}
