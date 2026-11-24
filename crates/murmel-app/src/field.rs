//! The field legend (stage 1 of field results; decided 2026-11-24: a
//! vertical bar at the right edge, below the view cube): the field's
//! name and unit, its colour bar with ticks, the bands of banded
//! colouring and marks at the contour lines' values, and the range mode.
//! The name opens the field's settings: field and component, colouring
//! (flat, smooth, banded) and bands, contour lines and their count,
//! colour map and its direction, and the range. Every change calls a
//! tool (`display.color_by`, `field.style`); the legend only reads
//! `murmel_engine::Legend`, as exported pictures do.

use egui::containers::menu::{MenuButton, MenuConfig};
use egui::{Color32, Pos2, Rect, RichText, Role, Sense, Stroke, Ui, WidgetInfo};
use murmel_engine::field::{self, Component};
use murmel_engine::legend::{Body, Ramp};
use murmel_engine::{FieldMode, Legend, RangeMode};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::style::color;

/// Width of the colour bar.
const BAR_W: f32 = 16.0;
/// The bar's height when the viewport has room, and the least it gets.
const BAR_H: f32 = 220.0;
const BAR_H_MIN: f32 = 72.0;
/// Room right of the bar for the iso-surfaces' marks.
const MARK_W: f32 = 12.0;
/// Room for the header (name, unit) and the footer (range mode).
const HEADER: f32 = 46.0;
const FOOTER: f32 = 22.0;

impl MurmelApp {
    /// The legend of colour mode field, at the right edge of `rect`.
    /// Returns the right edge left for the next legend (the
    /// streamlines' bars).
    pub(crate) fn field_legend(&mut self, ui: &mut Ui, rect: Rect, dark: bool) -> f32 {
        let free = rect.right() - 16.0;
        let Some(legend) = Legend::of(&self.session) else {
            return free;
        };
        let Body::Ramp(ramp) = &legend.body else {
            return free;
        };
        let p = self.session.tokens.palette(dark).clone();
        let line = self.session.tokens.mesh.field.line;
        let title = legend.title.text(&self.i18n);
        let font = egui::FontId::proportional(12.0);
        let labels: Vec<(f64, String)> = ramp
            .ticks
            .iter()
            .map(|&t| (t, ramp.tick_label(t)))
            .collect();
        let label_w = {
            let painter = ui.painter();
            labels
                .iter()
                .map(|(_, l)| {
                    painter
                        .layout_no_wrap(l.clone(), font.clone(), Color32::WHITE)
                        .size()
                        .x
                })
                .fold(0.0, f32::max)
        };
        let width = (label_w + 10.0 + BAR_W + MARK_W).max(132.0);
        let (top, bottom) = self.cube_right_room();
        let room = rect.height() - top - bottom - 32.0 - HEADER - FOOTER - 20.0;
        let bar_h = room.clamp(BAR_H_MIN, BAR_H);
        let size = egui::vec2(width + 20.0, HEADER + bar_h + 12.0 + FOOTER + 20.0);
        let at = Pos2::new(rect.right() - 16.0 - size.x, rect.top() + 16.0 + top);
        let area = Rect::from_min_size(at, size);
        let range_key = self.session.field_style().range.key();
        let mut click: Option<(&'static str, Value)> = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            egui::Frame::NONE
                .fill(color(p.bg_panel).gamma_multiply(0.92))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    // The name opens the settings; they stay open while
                    // the user changes several of them.
                    let name = self
                        .i18n
                        .tr_args("field-settings-name", &[("field", title.clone().into())]);
                    let (r, _) = MenuButton::new(RichText::new(&title).strong())
                        .config(
                            MenuConfig::new()
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                        )
                        .ui(ui, |ui| self.field_menu(ui));
                    r.on_hover_text(self.i18n.tr("field-settings-tip"))
                        .widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
                    ui.label(
                        RichText::new(ramp.unit.clone().unwrap_or_default())
                            .small()
                            .color(color(p.text_secondary)),
                    );
                    // The marks of the iso-surfaces sit right of the bar.
                    let (resp, painter) =
                        ui.allocate_painter(egui::vec2(width, bar_h + 12.0), Sense::click());
                    let bar = Rect::from_min_size(
                        Pos2::new(resp.rect.right() - BAR_W - MARK_W, resp.rect.top() + 6.0),
                        egui::vec2(BAR_W, bar_h),
                    );
                    paint_bar(&painter, bar, ramp, color(line));
                    self.iso_marks(&painter, bar, ramp);
                    // A click on the bar places an iso-surface there
                    // (decided 2026-11-24: "typed or clicked on the colour
                    // bar").
                    let near = |p: Pos2| bar.expand2(egui::vec2(4.0, 2.0)).contains(p);
                    if resp.hover_pos().is_some_and(near) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.clicked()
                        && let Some(at) = resp.interact_pointer_pos().filter(|&p| near(p))
                    {
                        click = self.iso_click(crate::extracts::bar_value(bar, ramp, at.y));
                    }
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
                        "field-legend-name",
                        &[
                            ("field", title.clone().into()),
                            ("lo", ramp.tick_label(ramp.lo).into()),
                            ("hi", ramp.tick_label(ramp.hi).into()),
                        ],
                    );
                    let resp = resp.on_hover_text(self.i18n.tr("field-bar-tip"));
                    resp.widget_info(|| WidgetInfo::labeled(Role::Image, true, desc.clone()));
                    ui.label(
                        RichText::new(self.i18n.tr(&format!("field-range-{range_key}")))
                            .small()
                            .color(color(p.text_secondary)),
                    );
                });
        });
        if let Some((tool, input)) = click {
            self.call(tool, input);
        }
        area.left() - 8.0
    }

    /// The field's settings, in the legend's menu.
    fn field_menu(&mut self, ui: &mut Ui) {
        let Some(v) = self.session.field_values() else {
            return;
        };
        let (name, vector, comp) = (v.choice.name.clone(), v.vector, v.choice.component);
        let st = self.session.field_style();
        let mut calls: Vec<(&'static str, Value)> = Vec::new();
        ui.set_min_width(220.0);
        // The field and, for a vector, its component.
        let names: Vec<String> = self
            .session
            .fields()
            .map(|f| f.names().iter().map(|n| n.to_string()).collect())
            .unwrap_or_default();
        let label = self.i18n.tr("color-by-field");
        egui::ComboBox::from_id_salt("field-name")
            .selected_text(&name)
            .show_ui(ui, |ui| {
                for n in &names {
                    if ui.selectable_label(*n == name, n).clicked() && *n != name {
                        calls.push(("display.color_by", json!({"by": "field", "field": n})));
                    }
                }
            })
            .response
            .widget_info(|| WidgetInfo::labeled(Role::ComboBox, true, format!("{label}: {name}")));
        if vector {
            ui.horizontal(|ui| {
                for c in Component::ALL {
                    let text = self.i18n.tr(&format!("field-component-{}", c.name()));
                    if ui.selectable_label(c == comp, &text).clicked() && c != comp {
                        calls.push((
                            "display.color_by",
                            json!({"by": "field", "field": name, "component": c}),
                        ));
                    }
                }
            });
        }
        ui.separator();
        let range = self.session.field_range();
        self.style_menu(ui, &st, range, true, &mut calls);
        for (tool, mut input) in calls {
            if tool == "field.style" {
                input["field"] = json!(name);
            }
            self.call(tool, input);
        }
    }

    /// The settings of a field's style, shared by the field legend and the
    /// streamlines' colour bars: colouring and bands, contour lines (when
    /// `lines`), the colour map and its direction, and the range; `range`
    /// is the range shown, for the automatic map's name. Pushes the
    /// `field.style` calls (the caller names the field).
    pub(crate) fn style_menu(
        &self,
        ui: &mut Ui,
        st: &murmel_engine::FieldStyle,
        range: Option<(f64, f64)>,
        lines: bool,
        calls: &mut Vec<(&'static str, Value)>,
    ) {
        // Colouring, bands and contour lines.
        ui.label(RichText::new(self.i18n.tr("field-mode")).small());
        ui.horizontal(|ui| {
            for m in FieldMode::ALL {
                let r = ui
                    .selectable_label(
                        m == st.mode,
                        self.i18n.tr(&format!("field-mode-{}", m.key())),
                    )
                    .on_hover_text(self.i18n.tr(&format!("field-mode-{}-tip", m.key())));
                if r.clicked() && m != st.mode {
                    calls.push(("field.style", json!({ "mode": m })));
                }
            }
        });
        if st.mode == FieldMode::Banded {
            let mut b = st.bands;
            ui.horizontal(|ui| {
                let label = self.i18n.tr("field-bands");
                ui.label(&label);
                let r =
                    ui.add(egui::DragValue::new(&mut b).range(field::MIN_BANDS..=field::MAX_BANDS));
                r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, label.clone()));
            });
            if b != st.bands {
                calls.push(("field.style", json!({ "bands": b })));
            }
        }
        if lines {
            ui.horizontal(|ui| {
                let mut on = st.lines;
                let label = self.i18n.tr("field-lines");
                if ui
                    .checkbox(&mut on, &label)
                    .on_hover_text(self.i18n.tr("field-lines-tip"))
                    .changed()
                {
                    calls.push(("field.style", json!({ "lines": on })));
                }
                if st.lines {
                    let mut n = st.line_count;
                    let label = self.i18n.tr("field-line-count");
                    let r = ui.add(egui::DragValue::new(&mut n).range(1..=field::MAX_LINES));
                    r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, label.clone()));
                    if n != st.line_count {
                        calls.push(("field.style", json!({ "line_count": n })));
                    }
                }
            });
        }
        ui.separator();
        // The colour map and its direction.
        let auto_map = range
            .and_then(|r| {
                let auto = murmel_engine::FieldStyle {
                    map: None,
                    ..st.clone()
                };
                field::map_for(&auto, r)
            })
            .map(|m| self.i18n.tr(&format!("colormap-{}", m.id)))
            .unwrap_or_default();
        let auto_text = self
            .i18n
            .tr_args("field-map-auto", &[("map", auto_map.into())]);
        let current = match &st.map {
            Some(id) => self.i18n.tr(&format!("colormap-{id}")),
            None => auto_text.clone(),
        };
        let label = self.i18n.tr("field-map");
        ui.label(RichText::new(&label).small());
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("field-map")
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(st.map.is_none(), &auto_text).clicked() {
                        calls.push(("field.style", json!({"map": "auto"})));
                    }
                    for id in murmel_engine::colormap::ids() {
                        let on = st.map.as_deref() == Some(id);
                        let text = self.i18n.tr(&format!("colormap-{id}"));
                        if ui.selectable_label(on, text).clicked() && !on {
                            calls.push(("field.style", json!({ "map": id })));
                        }
                    }
                })
                .response
                .widget_info(|| {
                    WidgetInfo::labeled(Role::ComboBox, true, format!("{label}: {current}"))
                });
            let mut rev = st.reverse;
            if ui
                .checkbox(&mut rev, self.i18n.tr("field-reverse"))
                .changed()
            {
                calls.push(("field.style", json!({ "reverse": rev })));
            }
        });
        ui.separator();
        // The range.
        let label = self.i18n.tr("field-range");
        ui.label(RichText::new(&label).small());
        let current = self.i18n.tr(&format!("field-range-{}", st.range.key()));
        egui::ComboBox::from_id_salt("field-range")
            .selected_text(&current)
            .show_ui(ui, |ui| {
                for k in ["field", "shown", "symmetric", "fixed"] {
                    let on = st.range.key() == k;
                    if ui
                        .selectable_label(on, self.i18n.tr(&format!("field-range-{k}")))
                        .clicked()
                        && !on
                    {
                        calls.push(("field.style", json!({ "range": k })));
                    }
                }
            })
            .response
            .widget_info(|| {
                WidgetInfo::labeled(Role::ComboBox, true, format!("{label}: {current}"))
            });
        if let RangeMode::Fixed { min, max } = st.range {
            let (mut lo, mut hi) = (min, max);
            let speed = ((max - min).abs() / 200.0).max(f64::MIN_POSITIVE);
            ui.horizontal(|ui| {
                for (key, v) in [("field-range-min", &mut lo), ("field-range-max", &mut hi)] {
                    let label = self.i18n.tr(key);
                    ui.label(&label);
                    let r = ui.add(egui::DragValue::new(v).speed(speed));
                    r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, label.clone()));
                }
            });
            if (lo, hi) != (min, max) && lo < hi {
                calls.push(("field.style", json!({"min": lo, "max": hi})));
            }
        }
        ui.separator();
        if ui.button(self.i18n.tr("field-reset")).clicked() {
            calls.push(("field.style", json!({"reset": true})));
        }
    }
}

/// The colour bar: the map from the low end (bottom) to the high end, in
/// flat bands for banded colouring, with a mark at each contour line.
pub(crate) fn paint_bar(painter: &egui::Painter, bar: Rect, ramp: &Ramp, line: Color32) {
    let y_of = |t: f64| bar.bottom() - t as f32 * bar.height();
    match ramp.bands {
        Some(n) if n > 0 => {
            for i in 0..n {
                let (a, b) = (i as f64 / n as f64, (i + 1) as f64 / n as f64);
                let c = color(ramp.sample(0.5 * (a + b)));
                painter.rect_filled(
                    Rect::from_x_y_ranges(bar.x_range(), y_of(b)..=y_of(a)),
                    0.0,
                    c,
                );
            }
        }
        _ => {
            let steps = bar.height().round().max(1.0) as usize;
            for i in 0..steps {
                let (a, b) = (i as f64 / steps as f64, (i + 1) as f64 / steps as f64);
                let c = color(ramp.sample(0.5 * (a + b)));
                painter.rect_filled(
                    Rect::from_x_y_ranges(bar.x_range(), y_of(b)..=y_of(a)),
                    0.0,
                    c,
                );
            }
        }
    }
    for &v in &ramp.contours {
        let y = y_of(ramp.position(v));
        painter.line_segment(
            [Pos2::new(bar.left(), y), Pos2::new(bar.right(), y)],
            Stroke::new(1.0, line),
        );
    }
}
