//! The viewport: the mesh drawn by `murmel-render` into a texture that
//! egui shows, the mouse mapped through the navigation bindings to view
//! tools, the view note and the axis triad.

use egui::{Color32, PointerButton, Pos2, Rect, RichText, Role, Sense, Ui, WidgetInfo};
use murmel_engine::navigation::{Action, Button, Mods};
use murmel_render::{MeshRenderer, View};
use serde_json::json;

use crate::app::MurmelApp;
use crate::i18n::count;
use crate::style::color;

/// The pointer resting over the viewport: where, since when, and what
/// `inspect.at` found there once it rested 400 ms (UI spec, Picking).
pub struct Hover {
    pub pos: Pos2,
    pub since: f64,
    /// The camera and surface the answer belongs to.
    pub key: String,
    pub asked: bool,
    pub hit: Option<serde_json::Value>,
}

/// How long the pointer rests before the tooltip asks (UI spec).
const HOVER_SECONDS: f64 = 0.4;

pub struct GpuView {
    renderer: MeshRenderer,
    texture: Option<(egui::TextureId, [u32; 2])>,
    last: Option<String>,
    pub adapter: String,
}

impl GpuView {
    pub fn new(rs: &egui_wgpu::RenderState) -> Self {
        let info = rs.adapter.get_info();
        Self {
            renderer: MeshRenderer::new(&rs.device),
            texture: None,
            last: None,
            adapter: format!("{:?} · {}", info.backend, info.name),
        }
    }
}

fn mods(m: egui::Modifiers) -> Mods {
    Mods {
        cmd: m.mac_cmd,
        shift: m.shift,
        ctrl: m.ctrl,
        alt: m.alt,
    }
}

impl MurmelApp {
    pub(crate) fn viewport(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let rect = ui.max_rect();
        let resp = ui.interact(
            rect,
            ui.scope_id().with("viewport"),
            Sense::click_and_drag(),
        );
        let size = [rect.width() as f64, rect.height() as f64];
        if size[0] >= 1.0 && size[1] >= 1.0 && self.session.viewport != size {
            self.session.viewport = size;
        }
        self.navigate(ui, &resp, rect);
        let dark = self.dark(ui.ctx());
        let cam = self.shown_camera(ui.ctx());
        self.draw_mesh(ui, frame, rect, dark, &cam);
        self.overlays(ui, rect, dark, &cam);
        if self.session.display.color_by == murmel_engine::ColorBy::Quality {
            self.quality_legend(ui, rect, dark);
        } else {
            self.legend(ui, rect, dark);
        }
        self.clip_gizmo(ui, rect, &cam, dark);
        self.view_cube(ui, rect, &cam, dark);

        let desc = self.describe();
        resp.widget_info(|| WidgetInfo::labeled(Role::Canvas, true, desc.clone()));
        let sel = !self.session.display.selection.is_empty();
        let has_mesh = self.session.mesh.is_some();
        resp.context_menu(|ui| {
            if sel {
                for (key, cmd) in [
                    ("menu-hide", "display.visibility:hide"),
                    ("menu-isolate", "display.visibility:isolate"),
                    ("menu-fit-selection", "view.fit:selection"),
                ] {
                    if ui.button(self.i18n.tr(key)).clicked() {
                        if let Some(c) = self.commands.iter().find(|c| c.id == cmd).cloned() {
                            let ctx = ui.ctx().clone();
                            self.run_command(&ctx, &c);
                        }
                        ui.close();
                    }
                }
            }
            if has_mesh && ui.button(self.i18n.tr("menu-show-all")).clicked() {
                self.call("display.visibility", json!({"action": "show_all"}));
                ui.close();
            }
            ui.separator();
            for (key, call) in [
                ("menu-fit", ("view.fit", json!({}))),
                ("menu-home", ("view.standard", json!({"view": "home"}))),
                ("view-front", ("view.standard", json!({"view": "front"}))),
                ("view-top", ("view.standard", json!({"view": "top"}))),
                ("view-right", ("view.standard", json!({"view": "right"}))),
            ] {
                if ui.button(self.i18n.tr(key)).clicked() {
                    self.call(call.0, call.1);
                    ui.close();
                }
            }
        });
    }

    fn describe(&self) -> String {
        let (cells, faces) = self
            .session
            .mesh
            .as_ref()
            .map_or((0, 0), |m| (m.surface.cells, m.surface.faces));
        let view = match self.session.camera.standard() {
            Some(v) => self.i18n.tr(&format!("view-{}", v.name())),
            None => self.i18n.tr("viewport-view-custom"),
        };
        self.i18n.tr_args(
            "viewport-description",
            &[
                ("view", view.into()),
                ("cells", count(cells).into()),
                ("faces", count(faces).into()),
                ("colour", self.color_by_name().into()),
            ],
        )
    }

    /// Mouse to view tools, through the bindings. A press that moves less
    /// than egui's drag threshold is a click, so a drag never picks.
    fn navigate(&mut self, ui: &Ui, resp: &egui::Response, rect: Rect) {
        let (m, scroll, pinch, hover) = ui.input(|i| {
            (
                mods(i.modifiers),
                i.smooth_scroll_delta.y,
                i.zoom_delta(),
                i.pointer.hover_pos(),
            )
        });
        let nav = self.session.navigation.clone();
        if resp.drag_started() {
            let button = [
                (PointerButton::Primary, Button::Primary),
                (PointerButton::Secondary, Button::Secondary),
                (PointerButton::Middle, Button::Middle),
            ]
            .into_iter()
            .find(|(b, _)| resp.dragged_by(*b))
            .map(|(_, b)| b);
            self.drag = button.and_then(|b| nav.drag(b, m));
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            let (dx, dy) = (d.x as f64, d.y as f64);
            match self.drag {
                Some(Action::Orbit) => self.orbit_by_drag(dx, dy),
                Some(Action::Roll) => {
                    // Turn about the view direction as the pointer turns
                    // about the viewport centre.
                    if let Some(p) = resp.interact_pointer_pos() {
                        let c = rect.center();
                        let (a, b) = (p - d - c, p - c);
                        if a.length() > 8.0 && b.length() > 8.0 {
                            let deg = (a.x * b.y - a.y * b.x).atan2(a.dot(b)).to_degrees();
                            self.call("view.rotate", json!({"axis": "z", "deg": -deg as f64}));
                        }
                    }
                }
                Some(Action::Pan) => {
                    self.call("view.pan", json!({"dx_px": dx, "dy_px": dy}));
                }
                Some(Action::Zoom) => {
                    let sign = if nav.invert_zoom { -1.0 } else { 1.0 };
                    let f = (sign * dy * nav.zoom_speed * 2.0).exp();
                    self.call("view.zoom", json!({"factor": f}));
                }
                _ => {}
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
        let plain = Mods { shift: false, ..m };
        if resp.clicked_by(PointerButton::Primary)
            && nav.click(Button::Primary, plain) == Some(Action::Pick)
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.call("select.pick", json!({"ndc": ndc(p, rect), "add": m.shift}));
        }
        self.hover(ui, resp, rect, hover);
        if resp.double_clicked_by(PointerButton::Middle)
            && nav.double_click(Button::Middle, m) == Some(Action::Fit)
        {
            self.call("view.fit", json!({}));
        }
        if !resp.hovered() {
            return;
        }
        let cursor = hover.filter(|_| nav.zoom_to_cursor).map(|p| ndc(p, rect));
        let sign = if nav.invert_zoom { -1.0 } else { 1.0 };
        if scroll != 0.0 && nav.scroll(m) == Some(Action::Zoom) {
            let f = (-sign * scroll as f64 * nav.zoom_speed).exp();
            self.call("view.zoom", json!({"factor": f, "cursor_ndc": cursor}));
        }
        if (pinch - 1.0).abs() > 1e-4 && nav.pinch(Mods::NONE) == Some(Action::Zoom) {
            let f = 1.0 / pinch as f64;
            self.call("view.zoom", json!({"factor": f, "cursor_ndc": cursor}));
        }
    }

    /// Ask `inspect.at` once the pointer rests, and show the answer as a
    /// tooltip; the status bar shows the point.
    fn hover(&mut self, ui: &Ui, resp: &egui::Response, rect: Rect, pos: Option<Pos2>) {
        let key = format!(
            "{:?}|{}",
            self.session.camera, self.session.surface_revision
        );
        let Some(pos) = pos.filter(|_| resp.hovered() && self.drag.is_none()) else {
            self.hover = None;
            return;
        };
        let now = self.now;
        let fresh = match &self.hover {
            Some(h) => (h.pos - pos).length() > 1.0 || h.key != key,
            None => true,
        };
        if fresh {
            self.hover = Some(Hover {
                pos,
                since: now,
                key: key.clone(),
                asked: false,
                hit: None,
            });
        }
        let Some(h) = &mut self.hover else { return };
        if !h.asked {
            if now - h.since >= HOVER_SECONDS {
                h.asked = true;
                let e = murmel_tools::call(
                    "inspect.at",
                    json!({"ndc": ndc(pos, rect)}),
                    &mut self.session,
                );
                h.hit = e
                    .ok()
                    .then(|| e.result["hit"].clone())
                    .filter(|v| !v.is_null());
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(HOVER_SECONDS));
            }
        }
        let Some(hit) = self.hover.as_ref().and_then(|h| h.hit.clone()) else {
            return;
        };
        let text = self.hover_text(&hit);
        resp.clone().on_hover_ui_at_pointer(|ui| {
            ui.label(RichText::new(text).monospace());
        });
    }

    fn hover_text(&self, hit: &serde_json::Value) -> String {
        let s = |k: &str| hit[k].as_str().unwrap_or("").to_string();
        let num = |k: &str| {
            s(k).split_once(':')
                .map_or(String::new(), |(_, n)| n.to_string())
        };
        let mut lines = Vec::new();
        if hit["cell"].is_string() {
            lines.push(self.i18n.tr_args(
                "hover-cell",
                &[
                    ("id", num("cell").into()),
                    ("kind", self.i18n.tr(&format!("kind-{}", s("kind"))).into()),
                    ("zone", s("zone").into()),
                ],
            ));
        }
        if hit["patch"].is_string() {
            lines.push(self.i18n.tr_args(
                "hover-face",
                &[
                    ("id", num("face").into()),
                    ("patch", s("patch").into()),
                    ("role", self.i18n.tr(&format!("role-{}", s("role"))).into()),
                ],
            ));
        } else if hit["face"].is_string() {
            lines.push(
                self.i18n
                    .tr_args("hover-open-face", &[("id", num("face").into())]),
            );
        } else {
            lines.push(self.i18n.tr("hover-section"));
        }
        lines.join("\n")
    }

    fn draw_mesh(
        &mut self,
        ui: &Ui,
        frame: &mut eframe::Frame,
        rect: Rect,
        dark: bool,
        cam: &murmel_engine::Camera,
    ) {
        let p = self.session.tokens.palette(dark).clone();
        let painter = ui.painter_at(rect);
        let (Some(rs), Some(g)) = (frame.wgpu_render_state(), self.gpu.as_mut()) else {
            painter.rect_filled(rect, 0.0, color(p.viewport_bottom));
            return;
        };
        let ppp = ui.ctx().pixels_per_point();
        let px = [
            ((rect.width() * ppp).round() as u32).clamp(1, murmel_render::MAX_SIDE),
            ((rect.height() * ppp).round() as u32).clamp(1, murmel_render::MAX_SIDE),
        ];
        let key = format!(
            "{cam:?}|{}|{px:?}|{dark}|{:?}",
            self.session.surface_revision, self.session.display.render_mode
        );
        if g.last.as_deref() != Some(key.as_str()) {
            g.renderer.render(
                &rs.device,
                &rs.queue,
                &View {
                    surface: self.session.mesh.as_ref().map(|m| &m.surface),
                    revision: self.session.surface_revision,
                    camera: cam,
                    size: px,
                    bg_top: p.viewport_top.linear(),
                    bg_bottom: p.viewport_bottom.linear(),
                    faces: self.session.display.render_mode.faces(),
                    edges: self.session.display.render_mode.edges(),
                },
            );
            if let Some(view) = g.renderer.egui_view() {
                let mut r = rs.renderer.write();
                match g.texture {
                    Some((id, s)) if s == px => {
                        r.update_egui_texture_from_wgpu_texture(
                            &rs.device,
                            view,
                            wgpu::FilterMode::Linear,
                            id,
                        );
                    }
                    Some((id, _)) => {
                        r.update_egui_texture_from_wgpu_texture(
                            &rs.device,
                            view,
                            wgpu::FilterMode::Linear,
                            id,
                        );
                        g.texture = Some((id, px));
                    }
                    None => {
                        let id =
                            r.register_native_texture(&rs.device, view, wgpu::FilterMode::Linear);
                        g.texture = Some((id, px));
                    }
                }
            }
            g.last = Some(key);
        }
        if let Some((id, _)) = g.texture {
            painter.image(
                id,
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }

    fn overlays(&self, ui: &Ui, rect: Rect, dark: bool, cam: &murmel_engine::Camera) {
        let p = self.session.tokens.palette(dark);
        let painter = ui.painter_at(rect);
        let body = egui::FontId::proportional(13.0);
        if self.session.mesh.is_none() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                self.i18n.tr("viewport-empty"),
                body.clone(),
                color(p.text_secondary),
            );
        }
        // View note, top left.
        let proj = cam.projection;
        let note = format!(
            "{} · {}",
            self.i18n.tr(&format!(
                "render-mode-{}",
                self.session.display.render_mode.key()
            )),
            if cam.is_orthographic() {
                self.i18n.tr("projection-orthographic")
            } else {
                self.i18n.tr(&format!("projection-{}", proj.key()))
            }
        );
        let galley = painter.layout_no_wrap(note, body, color(p.text_primary));
        let at = rect.min + egui::vec2(16.0, 16.0);
        let bg = Rect::from_min_size(at, galley.size() + egui::vec2(20.0, 10.0));
        painter.rect_filled(bg, 6.0, color(p.bg_panel).gamma_multiply(0.92));
        painter.galley(at + egui::vec2(10.0, 5.0), galley, Color32::WHITE);

        // Axis triad, bottom left: X red, Y green, Z blue.
        let t = &self.session.tokens.mesh;
        let origin = Pos2::new(rect.left() + 48.0, rect.bottom() - 48.0);
        let (right, up) = cam.basis();
        let len = 28.0;
        let mut axes = [
            (glam_dir(0), t.axis_x, "X"),
            (glam_dir(1), t.axis_y, "Y"),
            (glam_dir(2), t.axis_z, "Z"),
        ];
        // Draw the axis pointing away first.
        let to_eye = cam.to_eye();
        axes.sort_by(|a, b| {
            let da: f64 = (0..3).map(|k| a.0[k] * to_eye[k]).sum();
            let db: f64 = (0..3).map(|k| b.0[k] * to_eye[k]).sum();
            da.total_cmp(&db)
        });
        for (dir, c, label) in axes {
            let sx: f64 = (0..3).map(|k| dir[k] * right[k]).sum();
            let sy: f64 = (0..3).map(|k| dir[k] * up[k]).sum();
            let end = origin + egui::vec2(sx as f32 * len, -sy as f32 * len);
            painter.line_segment([origin, end], egui::Stroke::new(2.0, color(c)));
            painter.text(
                origin + (end - origin) * 1.3,
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(11.0),
                color(c),
            );
        }
    }
}

impl MurmelApp {
    /// A world point on screen, through the camera the window draws.
    fn to_screen(&self, cam: &murmel_engine::Camera, rect: Rect, p: [f64; 3]) -> Option<Pos2> {
        let surface = &self.session.mesh.as_ref()?.surface;
        let aspect = (rect.width() / rect.height().max(1.0)) as f64;
        let [x, y] = cam.project(surface.origin, surface.radius, aspect, p)?;
        Some(Pos2::new(
            rect.left() + ((x + 1.0) * 0.5) as f32 * rect.width(),
            rect.top() + ((1.0 - y) * 0.5) as f32 * rect.height(),
        ))
    }

    /// The clip plane's gizmo: its outline inside the mesh's box, tinted
    /// with the accent, and an arrow along the normal that slides the
    /// plane when dragged (`clip.set` with an offset). Drawn by the
    /// painter, like the view cube, so screenshots leave it out.
    fn clip_gizmo(&mut self, ui: &Ui, rect: Rect, cam: &murmel_engine::Camera, dark: bool) {
        let Some(c) = self.session.display.active_clip().copied() else {
            return;
        };
        let Some(m) = &self.session.mesh else { return };
        let bounds = m.mesh.bounds();
        let section = c.box_section(&bounds);
        if section.is_empty() {
            return;
        }
        let accent = color(self.session.tokens.palette(dark).accent);
        let pts: Vec<Pos2> = section
            .iter()
            .filter_map(|&p| self.to_screen(cam, rect, p))
            .collect();
        if pts.len() != section.len() {
            return;
        }
        let painter = ui.painter_at(rect);
        painter.add(egui::Shape::convex_polygon(
            pts.clone(),
            accent.gamma_multiply(0.10),
            egui::Stroke::NONE,
        ));
        let mut ring = pts.clone();
        ring.push(pts[0]);
        painter.add(egui::Shape::dashed_line(
            &ring,
            egui::Stroke::new(1.5, accent),
            6.0,
            4.0,
        ));
        let n = section.len() as f64;
        let centre = [0, 1, 2].map(|k| section.iter().map(|p| p[k]).sum::<f64>() / n);
        let len = 0.18 * bounds.radius();
        let tip = [0, 1, 2].map(|k| centre[k] + c.normal[k] * len);
        let (Some(s0), Some(s1)) = (
            self.to_screen(cam, rect, centre),
            self.to_screen(cam, rect, tip),
        ) else {
            return;
        };
        let axis = s1 - s0;
        let px = axis.length();
        let id = ui.scope_id().with("clip-gizmo");
        let hit = Rect::from_center_size(s1, egui::vec2(22.0, 22.0)).union(Rect::from_center_size(
            s0 + axis * 0.5,
            egui::vec2(14.0, 14.0),
        ));
        let resp = ui.interact(hit, id, Sense::drag());
        let name = self.i18n.tr("toolbar-clip-position");
        resp.widget_info(|| WidgetInfo::labeled(Role::Slider, true, name.clone()));
        let hot = resp.hovered() || resp.dragged();
        let stroke = egui::Stroke::new(if hot { 3.0 } else { 2.0 }, accent);
        if px > 4.0 {
            painter.arrow(s0, axis, stroke);
        }
        painter.circle_filled(s0, 3.5, accent);
        let resp = resp.on_hover_text(self.i18n.tr("toolbar-clip-position-tip"));
        if resp.dragged() && px > 4.0 {
            let d = resp.drag_delta();
            let along = (d.x * axis.x + d.y * axis.y) / px;
            let metres = along as f64 * len / px as f64;
            if metres != 0.0 {
                let (off, [lo, hi]) = c.offset(&bounds);
                let v = (off + metres).clamp(lo, hi);
                self.call("clip.set", json!({ "offset": v }));
            }
        }
    }
}

fn glam_dir(k: usize) -> [f64; 3] {
    let mut d = [0.0; 3];
    d[k] = 1.0;
    d
}

/// A screen point as normalised device coordinates (-1..1, y up).
fn ndc(p: Pos2, rect: Rect) -> [f64; 2] {
    let x = (p.x - rect.left()) / rect.width().max(1.0) * 2.0 - 1.0;
    let y = 1.0 - (p.y - rect.top()) / rect.height().max(1.0) * 2.0;
    [x as f64, y as f64]
}
