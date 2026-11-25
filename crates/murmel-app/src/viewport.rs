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
        self.viewport_rect = rect;
        let resp = ui.interact(
            rect,
            ui.scope_id().with("viewport"),
            Sense::click_and_drag(),
        );
        // The canvas takes the focus (F6), and keeps the arrow keys for
        // orbiting instead of letting egui move the focus with them.
        self.focus.viewport = Some(resp.id);
        self.focus.offer(crate::focus::Region::Viewport, &resp);
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                resp.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        let size = [rect.width() as f64, rect.height() as f64];
        if size[0] >= 1.0 && size[1] >= 1.0 && self.session.viewport != size {
            self.session.viewport = size;
        }
        self.navigate(ui, &resp, rect);
        let dark = self.dark(ui.ctx());
        let cam = self.shown_camera(ui.ctx());
        self.draw_mesh(ui, frame, rect, dark, &cam);
        self.overlays(ui, rect, dark, &cam);
        let (hover_pos, alt) = ui.input(|i| (i.pointer.hover_pos(), i.modifiers.alt));
        let pointer = hover_pos
            .filter(|_| resp.hovered() && self.drag.is_none())
            .map(|p| (p, alt));
        self.measure_overlay(ui, rect, &cam, dark, pointer);
        let free = match self.session.display.color_by {
            murmel_engine::ColorBy::Quality => {
                self.quality_legend(ui, rect, dark);
                rect.right() - 16.0
            }
            murmel_engine::ColorBy::Field => self.field_legend(ui, rect, dark),
            _ => {
                self.legend(ui, rect, dark);
                rect.right() - 16.0
            }
        };
        self.own_legends(ui, rect, dark, free);
        self.clip_handles(ui, rect, &cam, dark);
        self.stream_handles(ui, rect, &cam, dark);
        self.view_cube(ui, rect, &cam, dark);
        self.draw_band(ui);
        if (self.zoom_armed || self.band.is_some()) && resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }

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
            ui.menu_button(self.i18n.tr("menu-view-cube"), |ui| {
                self.cube_menu(ui, true)
            });
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
        let (cells, faces) = self.session.mesh.as_ref().map_or((0, 0), |m| {
            // An overview draws no cells: it tells those of the file.
            let cells = match (&m.overview, &m.blocks) {
                (Some(_), None) => m.cells(),
                _ => m.surface.cells,
            };
            (cells, m.surface.faces)
        });
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
            // An armed zoom to box takes the primary drag, whatever the
            // bindings say.
            self.drag = match button {
                Some(Button::Primary) if self.zoom_armed => Some(Action::ZoomBox),
                Some(b) => nav.drag(b, m),
                None => None,
            };
            self.orbit_pivot = None;
            if self.drag == Some(Action::ZoomBox) {
                let start = ui.input(|i| i.pointer.press_origin());
                self.band = start.map(|p| (p, p));
            }
            if self.drag == Some(Action::Orbit) {
                let start = ui.input(|i| i.pointer.press_origin());
                self.orbit_pivot = self.pivot_at(start.map(|p| ndc(p, rect)));
            }
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
                Some(Action::ZoomBox) => {
                    if let (Some((a, _)), Some(p)) = (self.band, resp.interact_pointer_pos()) {
                        self.band = Some((a, rect.clamp(p)));
                    }
                }
                _ => {}
            }
        }
        if resp.drag_stopped() {
            if self.drag == Some(Action::ZoomBox) {
                self.finish_band(rect, nav.click_slop_px);
            }
            self.drag = None;
            self.orbit_pivot = None;
        }
        let plain = Mods { shift: false, ..m };
        let click = resp
            .clicked_by(PointerButton::Primary)
            .then(|| resp.interact_pointer_pos())
            .flatten();
        if let Some(p) = click {
            // While measuring a click places a point (Alt: unsnapped);
            // else a click on a measurement's label selects it, and
            // elsewhere picks what is under it.
            if self.measuring.is_some() {
                self.measure_click(p, rect, m.alt);
            } else if let Some(item) = self.measure_label_at(p) {
                let id = self.session.mesh.as_ref().map(|o| item.id(&o.mesh));
                self.call("select.item", json!({"item": id, "add": m.shift}));
            } else if nav.click(Button::Primary, plain) == Some(Action::Pick) {
                self.call("select.pick", json!({"ndc": ndc(p, rect), "add": m.shift}));
            }
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

    /// The end of a rubber-band drag: zoom into the box, unless it is
    /// thinner than a click (then nothing happens); one box per press of
    /// W or the toolbar button (the maintainer's card of 2026-11-25).
    fn finish_band(&mut self, rect: Rect, slop: f64) {
        let band = self.band.take();
        self.zoom_armed = false;
        let Some((a, b)) = band else { return };
        let r = Rect::from_two_pos(a, b);
        if (r.width() as f64) < slop || (r.height() as f64) < slop {
            return;
        }
        self.call(
            "view.zoom_box",
            json!({"from_ndc": ndc(a, rect), "to_ndc": ndc(b, rect)}),
        );
    }

    /// The rubber band: a thin accent outline over a faint accent fill.
    fn draw_band(&self, ui: &Ui) {
        let Some((a, b)) = self.band else { return };
        let accent = color(self.palette(ui.ctx()).accent);
        let r = Rect::from_two_pos(a, b);
        let p = ui.painter();
        p.rect_filled(r, 0.0, accent.gamma_multiply(0.12));
        p.rect_stroke(
            r,
            0.0,
            egui::Stroke::new(1.5, accent),
            egui::StrokeKind::Middle,
        );
    }

    /// The point an orbit turns about (UI spec, Viewport): the point of
    /// the mesh under `at`, or the centre of the faces shown when the
    /// pointer is over empty space.
    fn pivot_at(&mut self, at: Option<[f64; 2]>) -> Option<[f64; 3]> {
        if let Some(at) = at {
            let e = murmel_tools::call("inspect.at", json!({ "ndc": at }), &mut self.session);
            let p = &e.result["hit"]["point"];
            if let Some(v) = p.as_array().filter(|v| v.len() == 3) {
                let c = |i: usize| v[i].as_f64();
                if let (Some(x), Some(y), Some(z)) = (c(0), c(1), c(2)) {
                    return Some([x, y, z]);
                }
            }
        }
        let m = self.session.mesh.as_ref()?;
        m.surface
            .visible_bounds
            .or(m.surface.bounds)
            .map(|b| b.center())
    }

    /// Ask `inspect.at` once the pointer rests, and show the answer as a
    /// tooltip; the status bar shows the point.
    fn hover(&mut self, ui: &Ui, resp: &egui::Response, rect: Rect, pos: Option<Pos2>) {
        let key = format!(
            "{:?}|{}",
            self.session.camera, self.session.surface_revision
        );
        // While measuring the snap marker says what is under the pointer;
        // while a clip preview waits for its exact cut, nothing is asked
        // (a tool would wait for the cut).
        let Some(pos) = pos.filter(|_| {
            resp.hovered()
                && self.drag.is_none()
                && self.measuring.is_none()
                && self.session.preview.is_none()
        }) else {
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
        // The field's range, map and style are uniforms: a change of them
        // draws again without a new surface.
        let field = self.session.field_paint();
        // During a clip drag: the surface without the moving planes, cut
        // by them on the GPU, until the exact cut is in place.
        let drawn = self.session.drawn();
        let (surface, revision, planes) = match &drawn {
            Some((s, r, p)) => (Some(*s), *r, p.as_slice()),
            None => (None, self.session.surface_revision, &[][..]),
        };
        // During a slice drag: the moving slices drawn from the field's
        // grid until their exact faces are in place.
        let slice = self.session.slice_drawn();
        let slice_key = slice.as_ref().map(|d| {
            (
                &d.polygons,
                d.grid.as_ref().map(|g| g.id),
                &d.hide_faces,
                &d.hide_edges,
                &d.clips,
            )
        });
        // A new iso value: ray cast through its field's grid until its
        // exact surface is in place.
        let iso = self.session.iso_drawn();
        let iso_key = iso.as_ref().map(|d| {
            (
                d.texels.id,
                d.color,
                d.field.as_ref().map(|g| g.id),
                &d.hide_faces,
                &d.clips,
            )
        });
        // While the view moves the edges are left out if frames with
        // them are slow (the maintainer's card: over 33 ms).
        let display = &self.session.display;
        let gap = ui.input(|i| i.unstable_dt) as f64;
        let hideable = display.draw_edges() && display.mesh_faces();
        let moving = self
            .moving_edges
            .frame(self.now, gap, cam, (revision, px), hideable);
        match moving.repaint {
            Some(s) if s <= 0.0 => ui.ctx().request_repaint(),
            Some(s) => ui
                .ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(s)),
            None => {}
        }
        let edges = display.draw_edges() && !moving.hide;
        let key = format!(
            "{cam:?}|{revision}|{planes:?}|{slice_key:?}|{iso_key:?}|{px:?}|{dark}|{:?}|{edges}|{field:?}",
            display.render_mode
        );
        if g.last.as_deref() != Some(key.as_str()) {
            g.renderer.render(
                &rs.device,
                &rs.queue,
                &View {
                    surface,
                    revision,
                    camera: cam,
                    size: px,
                    bg_top: p.viewport_top.linear(),
                    bg_bottom: p.viewport_bottom.linear(),
                    faces: self.session.display.draw_faces(),
                    edges,
                    line_width: 1.0,
                    transparent: false,
                    field: field.as_ref(),
                    scale: ppp,
                    clip_planes: planes,
                    slice: slice.as_ref(),
                    iso: iso.as_ref(),
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
                self.opening_text()
                    .unwrap_or_else(|| self.i18n.tr("viewport-empty")),
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
        let at = rect.min + egui::vec2(16.0, 16.0 + self.cube_room().0);
        let bg = Rect::from_min_size(at, galley.size() + egui::vec2(20.0, 10.0));
        painter.rect_filled(bg, 6.0, color(p.bg_panel).gamma_multiply(0.92));
        painter.galley(at + egui::vec2(10.0, 5.0), galley, Color32::WHITE);

        // Axis triad, bottom left: X red, Y green, Z blue.
        let t = &self.session.tokens.mesh;
        let origin = Pos2::new(
            rect.left() + 48.0 + self.cube_room().1,
            rect.bottom() - 48.0,
        );
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

fn glam_dir(k: usize) -> [f64; 3] {
    let mut d = [0.0; 3];
    d[k] = 1.0;
    d
}

/// A screen point as normalised device coordinates (-1..1, y up).
pub(crate) fn ndc(p: Pos2, rect: Rect) -> [f64; 2] {
    let x = (p.x - rect.left()) / rect.width().max(1.0) * 2.0 - 1.0;
    let y = 1.0 - (p.y - rect.top()) / rect.height().max(1.0) * 2.0;
    [x as f64, y as f64]
}
