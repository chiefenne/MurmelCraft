//! The colour bars of their own (UI spec, field results): those of the
//! streamlines coloured by a field (decided 2026-11-24) and of the
//! arrows coloured by their own magnitude (the maintainer's card of
//! 2026-11-24), side by side left of the field legend
//! ([`murmel_engine::legend::Legend::own_bars`]). Each title opens the
//! bar's settings; every change calls a tool.

use egui::containers::menu::{MenuButton, MenuConfig};
use egui::{Color32, Pos2, Rect, RichText, Role, Sense, Stroke, Ui, WidgetInfo};
use murmel_engine::glyph::ArrowColor;
use murmel_engine::legend::{Body, Legend, Owner};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::style::color;

/// Width and height of a colour bar of its own.
const BAR_W: f32 = 16.0;
const BAR_H: f32 = 160.0;
const BAR_H_MIN: f32 = 60.0;

impl MurmelApp {
    /// The colour bars of their own, side by side leftwards from `right`
    /// (the left edge of the field legend, or the viewport's right edge),
    /// below the view cube when it sits top right. The title opens the
    /// bar's settings: for streamlines the colour field (and a vector's
    /// component) of their sets, for arrows their colouring; and the
    /// field's style.
    pub(crate) fn own_legends(&mut self, ui: &mut Ui, rect: Rect, dark: bool, right: f32) {
        let bars = Legend::own_bars(&self.session);
        if bars.is_empty() {
            return;
        }
        let p = self.session.tokens.palette(dark).clone();
        let line = color(self.session.tokens.mesh.field.line);
        let font = egui::FontId::proportional(12.0);
        let (top, bottom) = self.cube_right_room();
        let room = rect.height() - top - bottom - 32.0 - 120.0;
        let bar_h = room.clamp(BAR_H_MIN, BAR_H);
        let mut right = right;
        let mut calls: Vec<(&'static str, Value)> = Vec::new();
        for (b, (owner, legend)) in bars.iter().enumerate() {
            let Body::Ramp(ramp) = &legend.body else {
                continue;
            };
            let field = legend.title.field().to_string();
            let title = legend.title.text(&self.i18n);
            // The field whose style the bar shows, and the keys of its
            // names.
            let (name, keys) = match owner {
                Owner::Streams(sets) => {
                    let choice = sets
                        .first()
                        .and_then(|&k| self.session.display.streams.get(k))
                        .and_then(|s| s.color.as_ref());
                    let Some(choice) = choice else { continue };
                    (choice.name.clone(), STREAM_KEYS)
                }
                Owner::Arrows => {
                    let Some(a) = &self.session.display.arrows else {
                        continue;
                    };
                    (a.field.clone(), ARROW_KEYS)
                }
            };
            let labels: Vec<(f64, String)> = ramp
                .ticks
                .iter()
                .map(|&t| (t, ramp.tick_label(t)))
                .collect();
            let widest = |texts: &mut dyn Iterator<Item = &str>, font: &egui::FontId| {
                let painter = ui.painter();
                texts
                    .map(|l| {
                        painter
                            .layout_no_wrap(l.to_string(), font.clone(), Color32::WHITE)
                            .size()
                            .x
                    })
                    .fold(0.0, f32::max)
            };
            let label_w = widest(&mut labels.iter().map(|(_, l)| l.as_str()), &font);
            // The title is a menu button, in the buttons' text style.
            let button = egui::TextStyle::Button.resolve(ui.style());
            let title_w = widest(&mut std::iter::once(title.as_str()), &button);
            let width = (label_w + 10.0 + BAR_W)
                .max(title_w + 2.0 * ui.spacing().button_padding.x + 8.0)
                .max(110.0);
            let size = egui::vec2(width + 20.0, 46.0 + bar_h + 12.0 + 22.0 + 20.0);
            let at = Pos2::new(right - size.x, rect.top() + 16.0 + top);
            let area = Rect::from_min_size(at, size);
            right = area.left() - 8.0;
            let style = self.session.display.style_of(&name);
            let range_key = style.range.key();
            ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                egui::Frame::NONE
                    .fill(color(p.bg_panel).gamma_multiply(0.92))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let menu_name = self
                            .i18n
                            .tr_args(keys.settings, &[("field", field.clone().into())]);
                        let (r, _) = MenuButton::new(RichText::new(&title).strong())
                            .config(
                                MenuConfig::new()
                                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                            )
                            .ui(ui, |ui| {
                                ui.push_id(("own-bar", b), |ui| match owner {
                                    Owner::Streams(sets) => {
                                        let choice = sets
                                            .first()
                                            .and_then(|&k| self.session.display.streams.get(k))
                                            .and_then(|s| s.color.clone());
                                        if let Some(choice) = choice {
                                            self.stream_bar_menu(ui, sets, &choice, &mut calls);
                                        }
                                    }
                                    Owner::Arrows => self.arrow_bar_menu(ui, &mut calls),
                                });
                            });
                        r.on_hover_text(self.i18n.tr(keys.tip)).widget_info(|| {
                            WidgetInfo::labeled(Role::Button, true, menu_name.clone())
                        });
                        ui.label(
                            RichText::new(ramp.unit.clone().unwrap_or_default())
                                .small()
                                .color(color(p.text_secondary)),
                        );
                        let (resp, painter) =
                            ui.allocate_painter(egui::vec2(width, bar_h + 12.0), Sense::hover());
                        let bar = Rect::from_min_size(
                            Pos2::new(resp.rect.right() - BAR_W, resp.rect.top() + 6.0),
                            egui::vec2(BAR_W, bar_h),
                        );
                        crate::field::paint_bar(&painter, bar, ramp, line);
                        painter.rect_stroke(
                            bar,
                            0.0,
                            Stroke::new(1.0, color(p.line)),
                            egui::StrokeKind::Outside,
                        );
                        for (v, text) in &labels {
                            let y = bar.bottom() - ramp.position(*v) as f32 * bar.height();
                            painter.line_segment(
                                [Pos2::new(bar.left() - 4.0, y), Pos2::new(bar.left(), y)],
                                Stroke::new(1.0, color(p.text_secondary)),
                            );
                            painter.text(
                                Pos2::new(bar.left() - 7.0, y),
                                egui::Align2::RIGHT_CENTER,
                                text,
                                font.clone(),
                                color(p.text_primary),
                            );
                        }
                        let desc = self.i18n.tr_args(
                            keys.legend,
                            &[
                                ("field", field.clone().into()),
                                ("lo", ramp.tick_label(ramp.lo).into()),
                                ("hi", ramp.tick_label(ramp.hi).into()),
                            ],
                        );
                        resp.widget_info(|| WidgetInfo::labeled(Role::Image, true, desc.clone()));
                        ui.label(
                            RichText::new(self.i18n.tr(&format!("field-range-{range_key}")))
                                .small()
                                .color(color(p.text_secondary)),
                        );
                    });
            });
        }
        for (tool, input) in calls {
            self.call(tool, input);
        }
    }

    /// The settings of the arrows' colour bar: how they are coloured
    /// (`arrows.set`), and their field's style (`field.style`, shared
    /// with the mesh's legend).
    fn arrow_bar_menu(&self, ui: &mut Ui, calls: &mut Vec<(&'static str, Value)>) {
        let (Some(m), Some(a)) = (self.session.mesh.as_ref(), &self.session.display.arrows) else {
            return;
        };
        ui.set_min_width(220.0);
        for c in ArrowColor::ALL {
            let text = self.i18n.tr(&format!("iso-color-{}", c.key()));
            if ui.radio(a.color == c, text).clicked() && a.color != c {
                calls.push(("arrows.set", json!({ "color": c.key() })));
            }
        }
        ui.separator();
        let style = self.session.display.style_of(&a.field);
        let range = murmel_engine::glyph::color_range(
            &self.session.display,
            &m.mesh,
            Some(&m.fields),
            m.surface.arrow_range,
        );
        let mut own: Vec<(&'static str, Value)> = Vec::new();
        self.style_menu(ui, &style, range, false, &mut own);
        for (tool, mut input) in own {
            if tool == "field.style" || tool == "settings.set_default" {
                input["field"] = json!(a.field);
            }
            calls.push((tool, input));
        }
    }
}

/// The translation keys of a bar's names: its settings button, that
/// button's tip, and the bar itself.
#[derive(Clone, Copy)]
struct Keys {
    settings: &'static str,
    tip: &'static str,
    legend: &'static str,
}

const STREAM_KEYS: Keys = Keys {
    settings: "stream-settings-name",
    tip: "stream-settings-tip",
    legend: "stream-legend-name",
};

const ARROW_KEYS: Keys = Keys {
    settings: "arrow-settings-name",
    tip: "arrow-settings-tip",
    legend: "arrow-legend-name",
};
