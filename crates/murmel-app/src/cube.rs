//! The view cube, top right of the viewport (UI spec, "Viewport,
//! navigation and view cube"): 96 px, drawn with egui's painter over the
//! mesh (decided 2026-11-23), from the patches of `murmel_engine::cube`,
//! so the drawing and the hit test are the geometry the engine tests
//! check. A click on a face, edge or corner flies to one of 26 views in
//! 250 ms; a drag orbits. Above it: Home and the projection menu; beside
//! it: two arrows that roll the view by 90 degrees; below it: rotation by
//! any angle about a screen axis.

use egui::{Color32, FontId, Pos2, Rect, Role, Sense, Shape, Stroke, Ui, Vec2, WidgetInfo};
use murmel_engine::cube::{self, BORDER};
use murmel_engine::{Camera, Projection, ScreenAxis};
use serde_json::json;

use crate::app::{MurmelApp, icon_button};
use crate::style::{SEMIBOLD, color, icon};

/// Length of a flight to a view.
pub const FLIGHT_SECONDS: f64 = 0.25;
/// Side of the cube's square in the corner.
const SIDE: f32 = 96.0;
/// Width of the block: the cube with its roll arrows, and the rotate row.
const BLOCK: f32 = 196.0;

impl MurmelApp {
    /// The camera the viewport shows: the session's, or a point of the
    /// flight towards it.
    pub(crate) fn shown_camera(&mut self, ctx: &egui::Context) -> Camera {
        if let Some((from, t0)) = &self.flight {
            let t = (self.now - t0) / FLIGHT_SECONDS;
            if t < 1.0 {
                ctx.request_repaint();
                return Camera::lerp(from, &self.session.camera, t);
            }
            self.flight = None;
        }
        self.session.camera.clone()
    }

    pub(crate) fn view_cube(&mut self, ui: &mut Ui, viewport: Rect, cam: &Camera, dark: bool) {
        let p = self.session.tokens.palette(dark).clone();
        let top = viewport.right_top() + egui::vec2(-16.0 - BLOCK, 12.0);
        let mid = top.x + BLOCK / 2.0;

        // Home and projection, above the cube.
        let bar = Rect::from_min_size(egui::pos2(mid - 30.0, top.y), egui::vec2(64.0, 28.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let home = self.i18n.tr("cmd-home");
                if icon_button(ui, icon::HOUSE, &home, &self.i18n.tr("cube-home"), false).clicked()
                {
                    self.call("view.standard", json!({"view": "home"}));
                }
                let proj = self.session.camera.projection;
                let glyph = if cam.is_orthographic() {
                    icon::SQUARE
                } else {
                    icon::BOX
                };
                let name = self.i18n.tr("cube-projection");
                let r = ui.menu_button(egui::RichText::new(glyph).size(16.0), |ui| {
                    for m in [
                        Projection::PerspectiveOrthoFaces,
                        Projection::Perspective,
                        Projection::Orthographic,
                    ] {
                        let label = self.i18n.tr(&format!("projection-{}", m.key()));
                        if ui.radio(m == proj, label).clicked() {
                            self.call("view.set_camera", json!({ "projection": m }));
                            ui.close();
                        }
                    }
                });
                let long = self.i18n.tr(&format!("projection-{}", proj.key()));
                let tip = format!("{long}\n{name}");
                r.response
                    .on_hover_text(tip)
                    .widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
            });
        });

        // The cube.
        let area = Rect::from_min_size(
            egui::pos2(mid - SIDE / 2.0, top.y + 32.0),
            egui::vec2(SIDE, SIDE),
        );
        let resp = ui.interact(
            area,
            ui.scope_id().with("view-cube"),
            Sense::click_and_drag(),
        );
        let centre = area.center();
        // The iso silhouette (half width √2, half height about 1.6)
        // fills the square.
        let scale = SIDE / 2.0 / 1.62;
        let to_screen = |q: [f64; 2]| centre + egui::vec2(q[0] as f32, -q[1] as f32) * scale;
        let from_screen = |s: Pos2| {
            let d = (s - centre) / scale;
            [d.x as f64, -d.y as f64]
        };
        let hover = resp
            .hover_pos()
            .filter(|_| !resp.dragged())
            .and_then(|s| cube::hit(cam, from_screen(s)));
        let current = cam.standard();
        let painter = ui.painter_at(area.expand(2.0));
        // Faces in a light neutral between the raised background and the
        // text colour, darker as they turn away.
        let light = lerp(
            color(p.bg_raised),
            color(p.text_primary),
            if dark { 0.55 } else { 0.1 },
        );
        let dim = color(p.bg_raised);
        let accent = color(p.accent);
        for patch in cube::patches(cam) {
            let pts: Vec<Pos2> = patch.corners.iter().map(|&q| to_screen(q)).collect();
            let mut fill = lerp(dim, light, 0.35 + 0.65 * patch.facing as f32);
            if Some(patch.view) == current {
                fill = lerp(fill, accent, 0.55);
            } else if Some(patch.view) == hover {
                fill = lerp(fill, accent, 0.3);
            }
            painter.add(Shape::convex_polygon(
                pts,
                fill,
                Stroke::new(0.75, color(p.line)),
            ));
        }
        // Face outlines over the grid lines.
        for f in cube::labels(cam) {
            let c = |a: f64, b: f64| {
                to_screen([
                    f.center[0] + f.right[0] * a + f.up[0] * b,
                    f.center[1] + f.right[1] * a + f.up[1] * b,
                ])
            };
            let outline = vec![c(-1.0, -1.0), c(1.0, -1.0), c(1.0, 1.0), c(-1.0, 1.0)];
            painter.add(Shape::closed_line(outline, Stroke::new(1.0, color(p.line))));
            if f.facing < 0.3 {
                continue;
            }
            let text = self.i18n.tr(&format!("view-{}", f.face.name()));
            let size = (10.0 + 3.0 * f.facing as f32).round();
            let font = FontId::new(size, egui::FontFamily::Name(SEMIBOLD.into()));
            let ink = if dark {
                color(p.bg_app)
            } else {
                color(p.text_primary)
            };
            let galley = painter.layout_no_wrap(text, font, ink);
            // Fit the label inside the face centre patch.
            let room = (1.0 - BORDER) as f32 * 2.0 * scale * f.facing.sqrt() as f32;
            if galley.size().x > room * 1.15 {
                continue;
            }
            let angle = (-f.right[1] as f32).atan2(f.right[0] as f32);
            let half = galley.size() / 2.0;
            let rot = egui::emath::Rot2::from_angle(angle);
            let pos = to_screen(f.center) - rot * Vec2::new(half.x, half.y);
            painter.add(egui::epaint::TextShape::new(pos, galley, ink).with_angle(angle));
        }

        let name = self.i18n.tr_args(
            "cube-name",
            &[(
                "view",
                current
                    .map(|v| self.i18n.tr(&format!("view-{}", v.name())))
                    .unwrap_or_else(|| self.i18n.tr("viewport-view-custom"))
                    .into(),
            )],
        );
        resp.widget_info(|| WidgetInfo::labeled(Role::Canvas, true, name.clone()));
        if resp.clicked()
            && let Some(view) = resp
                .interact_pointer_pos()
                .and_then(|s| cube::hit(cam, from_screen(s)))
        {
            self.call("view.standard", json!({ "view": view }));
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            self.orbit_by_drag(d.x as f64, d.y as f64);
        }

        // Roll arrows beside the cube.
        for (dir, deg, key) in [
            (-1.0f32, 90.0, "cube-roll-left"),
            (1.0, -90.0, "cube-roll-right"),
        ] {
            let at = area.center() + egui::vec2(dir * (SIDE / 2.0 + 14.0), -SIDE / 2.0 + 10.0);
            let r = Rect::from_center_size(at, egui::vec2(22.0, 22.0));
            let b = ui.interact(r, ui.scope_id().with(key), Sense::click());
            let ink = if b.hovered() {
                accent
            } else {
                color(p.text_secondary)
            };
            roll_arrow(ui.painter(), at, dir, ink);
            let name = self.i18n.tr(key);
            let b = b.on_hover_text(&name);
            b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
            if b.clicked() {
                self.call("view.rotate", json!({"axis": "z", "deg": deg}));
            }
        }

        // Rotate by any angle about a screen axis, below the cube.
        let row = Rect::from_min_size(
            egui::pos2(top.x, area.bottom() + 10.0),
            egui::vec2(BLOCK, 26.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(row), |ui| {
            egui::Frame::NONE
                .fill(color(p.bg_panel).gamma_multiply(0.92))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(4, 2))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (axis, label) in [
                            (ScreenAxis::X, "X"),
                            (ScreenAxis::Y, "Y"),
                            (ScreenAxis::Z, "Z"),
                        ] {
                            let name = self.i18n.tr_args("rotate-axis", &[("axis", label.into())]);
                            let b = ui
                                .add(egui::Button::selectable(self.rotate_axis == axis, label))
                                .on_hover_text(self.i18n.tr("rotate-title"));
                            b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
                            if b.clicked() {
                                self.rotate_axis = axis;
                            }
                        }
                        let degrees = self.i18n.tr("rotate-degrees");
                        let d = ui.add(
                            egui::DragValue::new(&mut self.rotate_deg)
                                .range(0.0..=360.0)
                                .speed(1.0)
                                .suffix("°")
                                .max_decimals(1),
                        );
                        d.widget_info(|| {
                            WidgetInfo::labeled(Role::SpinButton, true, degrees.clone())
                        });
                        let axis = match self.rotate_axis {
                            ScreenAxis::X => "x",
                            ScreenAxis::Y => "y",
                            ScreenAxis::Z => "z",
                        };
                        for (sign, key) in [(-1.0, "rotate-minus"), (1.0, "rotate-plus")] {
                            let name = self.i18n.tr_args(
                                key,
                                &[
                                    ("deg", format!("{}", self.rotate_deg).into()),
                                    ("axis", axis.to_uppercase().into()),
                                ],
                            );
                            let glyph = if sign < 0.0 { "−" } else { "+" };
                            let b = ui.button(glyph).on_hover_text(&name);
                            b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
                            if b.clicked() {
                                self.call(
                                    "view.rotate",
                                    json!({"axis": axis, "deg": sign * self.rotate_deg}),
                                );
                            }
                        }
                    });
                });
        });
    }

    /// Orbit by a drag of `dx`, `dy` pixels (y down). The drag is turned
    /// back by the roll, so the scene follows the pointer on a rolled
    /// screen too.
    pub(crate) fn orbit_by_drag(&mut self, dx: f64, dy: f64) {
        let (s, c) = self.session.camera.roll_deg.to_radians().sin_cos();
        let up = -dy;
        let (lx, lup) = (dx * c + up * s, -dx * s + up * c);
        let speed = self.session.navigation.orbit_speed;
        self.call(
            "view.orbit",
            json!({"yaw_deg": -lx * speed, "pitch_deg": -lup * speed}),
        );
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// A curved arrow of our own (UI spec: custom icons drawn by us): a
/// quarter arc about `at`, its head counter-clockwise for `dir` -1 and
/// clockwise for +1.
fn roll_arrow(painter: &egui::Painter, at: Pos2, dir: f32, ink: Color32) {
    let r = 7.0;
    let n = 12;
    let (a0, a1) = (-150f32.to_radians(), -30f32.to_radians());
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            at + egui::vec2(a.cos() * r, a.sin() * r + 3.0)
        })
        .collect();
    let (tip, prev) = if dir < 0.0 {
        (pts[0], pts[1])
    } else {
        (pts[n], pts[n - 1])
    };
    painter.add(Shape::line(pts, Stroke::new(1.5, ink)));
    let d = (tip - prev).normalized();
    let side = egui::vec2(-d.y, d.x);
    painter.add(Shape::convex_polygon(
        vec![
            tip + d * 4.0,
            tip - d * 1.0 + side * 3.5,
            tip - d * 1.0 - side * 3.5,
        ],
        ink,
        Stroke::NONE,
    ));
}
