//! The viewport: the mesh drawn by `murmel-render` into a texture that
//! egui shows, the mouse mapped through the navigation bindings to view
//! tools, the view note and the axis triad.

use egui::{Color32, PointerButton, Pos2, Rect, Role, Sense, Ui, WidgetInfo};
use murmel_engine::navigation::{Action, Button, Mods};
use murmel_render::{MeshRenderer, View};
use serde_json::json;

use crate::app::MurmelApp;
use crate::i18n::count;
use crate::style::color;

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
        self.draw_mesh(ui, frame, rect, dark);
        self.overlays(ui, rect, dark);

        let desc = self.describe();
        resp.widget_info(|| WidgetInfo::labeled(Role::Canvas, true, desc.clone()));
        resp.context_menu(|ui| {
            for (key, call) in [
                ("menu-fit", ("view.fit", json!({}))),
                ("menu-home", ("view.standard", json!({"view": "home"}))),
                ("menu-front", ("view.standard", json!({"view": "front"}))),
                ("menu-top", ("view.standard", json!({"view": "top"}))),
                ("menu-right", ("view.standard", json!({"view": "right"}))),
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
            .map_or((0, 0), |m| (m.mesh.cell_count(), m.mesh.boundary.len()));
        let (y, p) = murmel_engine::StandardView::Iso.angles();
        let c = &self.session.camera;
        let view = if (c.yaw_deg - y).abs() < 1e-6 && (c.pitch_deg - p).abs() < 1e-6 {
            self.i18n.tr("viewport-view-iso")
        } else {
            self.i18n.tr("viewport-view-custom")
        };
        self.i18n.tr_args(
            "viewport-description",
            &[
                ("view", view.into()),
                ("cells", count(cells).into()),
                ("faces", count(faces).into()),
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
                Some(Action::Orbit) => {
                    let s = nav.orbit_speed;
                    self.call(
                        "view.orbit",
                        json!({"yaw_deg": -dx * s, "pitch_deg": dy * s}),
                    );
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

    fn draw_mesh(&mut self, ui: &Ui, frame: &mut eframe::Frame, rect: Rect, dark: bool) {
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
            "{:?}|{}|{px:?}|{dark}",
            self.session.camera, self.session.surface_revision
        );
        if g.last.as_deref() != Some(key.as_str()) {
            g.renderer.render(
                &rs.device,
                &rs.queue,
                &View {
                    surface: self.session.mesh.as_ref().map(|m| &m.surface),
                    revision: self.session.surface_revision,
                    camera: &self.session.camera,
                    size: px,
                    bg_top: p.viewport_top.linear(),
                    bg_bottom: p.viewport_bottom.linear(),
                    edges: true,
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

    fn overlays(&self, ui: &Ui, rect: Rect, dark: bool) {
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
        let proj = self.session.camera.projection;
        let note = format!(
            "{} · {}",
            self.i18n.tr("display-shaded-edges"),
            if self.session.camera.is_orthographic() {
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
        let (right, up) = self.session.camera.basis();
        let len = 28.0;
        let mut axes = [
            (glam_dir(0), t.axis_x, "X"),
            (glam_dir(1), t.axis_y, "Y"),
            (glam_dir(2), t.axis_z, "Z"),
        ];
        // Draw the axis pointing away first.
        let to_eye = self.session.camera.to_eye();
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
