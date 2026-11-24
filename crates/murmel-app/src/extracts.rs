//! The model tree's "Slices and iso-surfaces" (field results, stage 2):
//! a row per slice and per iso-surface, like the clip planes' rows (the
//! eye turns one on and off, a click selects it, so the handles and the
//! tools act on it, a double-click fits it, the context menu changes or
//! removes it), the buttons that add them, and while they show, whether
//! the mesh draws as its faint outline or with its faces (decided
//! 2026-11-24). The colour bar's part (a click places an iso-surface,
//! marks at their values) is here too. The vector arrows (stage 3) have a
//! row of their own, with their density and length below it; the
//! streamlines (stage 4) a row per set (`streams.rs`). Every change calls
//! a tool.

use egui::{Align, Layout, Pos2, Rect, RichText, Role, Sense, Stroke, Ui, WidgetInfo};
use murmel_engine::Item;
use murmel_engine::extract::{IsoColor, MAX_ISOS, MAX_SLICES, MeshLook};
use murmel_engine::glyph::{MAX_DENSITY, MAX_SCALE, MIN_DENSITY};
use murmel_engine::legend::Ramp;
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::panels::number;
use crate::style::{color, icon};

/// The planes of a series that the tree's "Series" makes.
const SERIES: u32 = 5;

impl MurmelApp {
    /// "X", "−Y", or "oblique": the axis a plane's normal lies along.
    fn normal_name(&self, n: [f64; 3]) -> String {
        ["x", "y", "z"]
            .into_iter()
            .zip(n)
            .find(|(_, v)| v.abs() > 0.999)
            .map(|(a, v)| format!("{}{}", if v < 0.0 { "−" } else { "" }, a.to_uppercase()))
            .unwrap_or_else(|| self.i18n.tr("tree-clip-oblique"))
    }

    /// The label of slice `k` in the tree and the Properties heading.
    pub(crate) fn slice_label(&self, k: usize) -> String {
        let Some(c) = self.session.display.slices.get(k) else {
            return String::new();
        };
        let normal = self.normal_name(c.normal);
        if c.count > 1 {
            self.i18n.tr_args(
                "tree-slice-series",
                &[
                    ("n", (k + 1).into()),
                    ("normal", normal.into()),
                    ("count", c.count.into()),
                ],
            )
        } else {
            self.i18n.tr_args(
                "tree-slice-label",
                &[("n", (k + 1).into()), ("normal", normal.into())],
            )
        }
    }

    /// The label of iso-surface `k`: its field and value.
    pub(crate) fn iso_label(&self, k: usize) -> String {
        let (Some(c), Some(m)) = (self.session.display.isos.get(k), &self.session.mesh) else {
            return String::new();
        };
        let f = m.fields.get(&c.field.name);
        let title = c.field.title(f.is_some_and(|f| f.is_vector()));
        let mut value = number(c.value);
        if let Some(u) = f.and_then(|f| f.unit.as_deref()) {
            value = format!("{value} {u}");
        }
        self.i18n.tr_args(
            "tree-iso-label",
            &[
                ("n", (k + 1).into()),
                ("field", title.into()),
                ("value", value.into()),
            ],
        )
    }

    /// The label of the arrows: their field.
    pub(crate) fn arrows_label(&self) -> String {
        let Some(a) = &self.session.display.arrows else {
            return String::new();
        };
        self.i18n
            .tr_args("tree-arrows-label", &[("field", a.field.clone().into())])
    }

    /// An eye button that turns a row's item on or off.
    pub(crate) fn eye(
        &self,
        ui: &mut Ui,
        on: bool,
        label: &str,
        p: &murmel_engine::theme::Palette,
    ) -> egui::Response {
        let name = self.i18n.tr_args(
            if on { "tree-hide" } else { "tree-show" },
            &[("item", label.to_string().into())],
        );
        let b = ui
            .add(
                egui::Button::new(
                    RichText::new(if on { icon::EYE } else { icon::EYE_OFF })
                        .size(14.0)
                        .color(if on {
                            color(p.text_primary)
                        } else {
                            color(p.text_secondary)
                        }),
                )
                .frame(false)
                .min_size(egui::vec2(20.0, 20.0)),
            )
            .on_hover_text(&name);
        b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
        b
    }

    /// The section: slices, then iso-surfaces, the add buttons and the
    /// mesh's look beside them.
    pub(crate) fn extract_section(
        &self,
        ui: &mut Ui,
        p: &murmel_engine::theme::Palette,
    ) -> Option<(&'static str, Value)> {
        let mesh = &self.session.mesh.as_ref()?.mesh;
        let has_fields = self
            .session
            .mesh
            .as_ref()
            .is_some_and(|m| !m.fields.is_empty());
        let has_vector = self.session.mesh.as_ref().is_some_and(|m| {
            m.fields
                .names()
                .iter()
                .any(|n| m.fields.get(n).is_some_and(|f| f.is_vector()))
        });
        let d = &self.session.display;
        let secondary = color(p.text_secondary);
        let mut action = None;
        egui::CollapsingHeader::new(
            RichText::new(self.i18n.tr("tree-extracts").to_uppercase())
                .small()
                .color(secondary),
        )
        .id_salt("tree-extracts")
        .default_open(true)
        .show(ui, |ui| {
            let handle = self.handle_target().map(|(t, _)| t);
            // The colour bar's ends, for the swatch of an iso-surface
            // coloured by the field.
            let ramp_ends = match murmel_engine::Legend::of(&self.session).map(|l| l.body) {
                Some(murmel_engine::legend::Body::Ramp(r)) => {
                    Some((r.sample(0.15), r.sample(0.85)))
                }
                _ => None,
            };
            for (k, c) in d.slices.iter().enumerate() {
                let item = Item::Slice(k as u32);
                let id = item.id(mesh);
                let n = k + 1;
                let label = self.slice_label(k);
                ui.horizontal(|ui| {
                    if self.eye(ui, c.enabled, &label, p).clicked() {
                        action = Some(("slice.set", json!({"slice": n, "enabled": !c.enabled})));
                    }
                    let selected = d.is_selected(item);
                    let text = if c.enabled {
                        RichText::new(&label)
                    } else {
                        RichText::new(&label).color(secondary)
                    };
                    let r = ui.selectable_label(selected, text);
                    // Said, and marked, when another plane could carry
                    // the handles.
                    let moved = handle == Some(crate::clip_handle::Target::Slice(k))
                        && (d.slices.len() > 1 || d.active_clips().next().is_some());
                    let row = if moved {
                        self.i18n
                            .tr_args("tree-slice-current", &[("row", label.clone().into())])
                    } else {
                        label.clone()
                    };
                    r.widget_info(|| {
                        WidgetInfo::selected(Role::Button, true, selected, row.clone())
                    });
                    let r = r.on_hover_text(self.i18n.tr("tree-slice-tip"));
                    if r.clicked() {
                        action = Some(("select.item", json!({ "item": id })));
                    }
                    if r.double_clicked() {
                        action = Some(("view.fit", json!({ "item": id })));
                    }
                    r.context_menu(|ui| {
                        let count = if c.count > 1 {
                            (
                                "cmd-slice-single",
                                ("slice.set", json!({"slice": n, "count": 1})),
                            )
                        } else {
                            (
                                "cmd-slice-series",
                                ("slice.set", json!({"slice": n, "count": SERIES})),
                            )
                        };
                        for (key, call) in [
                            count,
                            (
                                "cmd-clip-flip",
                                ("slice.set", json!({"slice": n, "flip": true})),
                            ),
                            (
                                "cmd-slice-x",
                                ("slice.set", json!({"slice": n, "axis": "x"})),
                            ),
                            (
                                "cmd-slice-y",
                                ("slice.set", json!({"slice": n, "axis": "y"})),
                            ),
                            (
                                "cmd-slice-z",
                                ("slice.set", json!({"slice": n, "axis": "z"})),
                            ),
                            (
                                "cmd-slice-view",
                                ("slice.set", json!({"slice": n, "axis": "view"})),
                            ),
                            ("tree-fit", ("view.fit", json!({ "item": id }))),
                            ("tree-slice-remove", ("slice.clear", json!({ "slice": n }))),
                        ] {
                            if ui.button(self.i18n.tr(key)).clicked() {
                                action = Some(call);
                                ui.close();
                            }
                        }
                    });
                    if moved {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(self.i18n.tr("tree-clip-current-mark"))
                                    .small()
                                    .color(secondary),
                            );
                        });
                    }
                });
            }
            for (k, c) in d.isos.iter().enumerate() {
                let item = Item::Iso(k as u32);
                let id = item.id(mesh);
                let n = k + 1;
                let label = self.iso_label(k);
                ui.horizontal(|ui| {
                    if self.eye(ui, c.enabled, &label, p).clicked() {
                        action = Some(("iso.set", json!({"iso": n, "enabled": !c.enabled})));
                    }
                    // The swatch: its solid colour, or a ramp when it
                    // takes the field's colours.
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                    match c.color {
                        IsoColor::Solid => {
                            let rgb = murmel_engine::extract::iso_color(d, &self.session.tokens, k);
                            ui.painter().rect_filled(rect, 2.0, color(rgb));
                        }
                        IsoColor::Field => {
                            let (a, b) = ramp_ends.unwrap_or((
                                self.session.tokens.mesh.field.none,
                                self.session.tokens.mesh.field.none,
                            ));
                            let half = Rect::from_min_max(rect.min, rect.center_bottom());
                            ui.painter().rect_filled(half, 0.0, color(a));
                            let rest = Rect::from_min_max(rect.center_top(), rect.max);
                            ui.painter().rect_filled(rest, 0.0, color(b));
                        }
                    }
                    let selected = d.is_selected(item);
                    let text = if c.enabled {
                        RichText::new(&label)
                    } else {
                        RichText::new(&label).color(secondary)
                    };
                    let r = ui.selectable_label(selected, text);
                    r.widget_info(|| {
                        WidgetInfo::selected(Role::Button, true, selected, label.clone())
                    });
                    let r = r.on_hover_text(self.i18n.tr("tree-iso-tip"));
                    if r.clicked() {
                        action = Some(("select.item", json!({ "item": id })));
                    }
                    if r.double_clicked() {
                        action = Some(("view.fit", json!({ "item": id })));
                    }
                    r.context_menu(|ui| {
                        for (key, call) in [
                            (
                                "cmd-iso-solid",
                                ("iso.set", json!({"iso": n, "color": "solid"})),
                            ),
                            (
                                "cmd-iso-field",
                                ("iso.set", json!({"iso": n, "color": "field"})),
                            ),
                        ] {
                            if ui.button(self.i18n.tr(key)).clicked() {
                                action = Some(call);
                                ui.close();
                            }
                        }
                        if c.color == IsoColor::Solid
                            && ui.button(self.i18n.tr("tree-color-edit")).clicked()
                        {
                            action = Some(("window.color", json!({"item": id, "label": label})));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(self.i18n.tr("tree-iso-remove")).clicked() {
                            action = Some(("iso.clear", json!({ "iso": n })));
                            ui.close();
                        }
                    });
                });
            }
            if let Some(a) = &d.arrows {
                let item = Item::Arrows;
                let id = item.id(mesh);
                let label = self.arrows_label();
                let tokens = &self.session.tokens;
                ui.horizontal(|ui| {
                    if self.eye(ui, a.enabled, &label, p).clicked() {
                        action = Some(("arrows.set", json!({"enabled": !a.enabled})));
                    }
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                    match a.color {
                        IsoColor::Solid => {
                            let rgb = d
                                .item_color(mesh, tokens, item)
                                .unwrap_or(tokens.mesh.field.arrow);
                            ui.painter().rect_filled(rect, 2.0, color(rgb));
                        }
                        IsoColor::Field => {
                            let none = tokens.mesh.field.none;
                            let (lo, hi) = ramp_ends.unwrap_or((none, none));
                            let half = Rect::from_min_max(rect.min, rect.center_bottom());
                            ui.painter().rect_filled(half, 0.0, color(lo));
                            let rest = Rect::from_min_max(rect.center_top(), rect.max);
                            ui.painter().rect_filled(rest, 0.0, color(hi));
                        }
                    }
                    let selected = d.is_selected(item);
                    let text = if a.enabled {
                        RichText::new(&label)
                    } else {
                        RichText::new(&label).color(secondary)
                    };
                    let r = ui.selectable_label(selected, text);
                    r.widget_info(|| {
                        WidgetInfo::selected(Role::Button, true, selected, label.clone())
                    });
                    let r = r.on_hover_text(self.i18n.tr("tree-arrows-tip"));
                    if r.clicked() {
                        action = Some(("select.item", json!({ "item": id })));
                    }
                    r.context_menu(|ui| {
                        let mut surface = a.surface;
                        if ui
                            .checkbox(&mut surface, self.i18n.tr("prop-on_surface"))
                            .changed()
                        {
                            action = Some(("arrows.set", json!({ "surface": surface })));
                            ui.close();
                        }
                        let mut in_plane = a.in_plane;
                        if ui
                            .checkbox(&mut in_plane, self.i18n.tr("prop-in_plane"))
                            .changed()
                        {
                            action = Some(("arrows.set", json!({ "in_plane": in_plane })));
                            ui.close();
                        }
                        ui.separator();
                        for (key, call) in [
                            (
                                "cmd-arrows-solid",
                                ("arrows.set", json!({"color": "solid"})),
                            ),
                            (
                                "cmd-arrows-field",
                                ("arrows.set", json!({"color": "field"})),
                            ),
                        ] {
                            if ui.button(self.i18n.tr(key)).clicked() {
                                action = Some(call);
                                ui.close();
                            }
                        }
                        if a.color == IsoColor::Solid
                            && ui.button(self.i18n.tr("tree-color-edit")).clicked()
                        {
                            action = Some(("window.color", json!({"item": id, "label": label})));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(self.i18n.tr("tree-arrows-remove")).clicked() {
                            action = Some(("arrows.clear", json!({})));
                            ui.close();
                        }
                    });
                });
                // How many and how long, under the row.
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    let name = self.i18n.tr("tree-arrows-density");
                    ui.label(RichText::new(&name).small());
                    let mut n = a.density;
                    let r = ui
                        .add(
                            egui::DragValue::new(&mut n)
                                .range(MIN_DENSITY..=MAX_DENSITY)
                                .speed(0.25),
                        )
                        .on_hover_text(self.i18n.tr("tree-arrows-density-tip"));
                    r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name.clone()));
                    if n != a.density {
                        action = Some(("arrows.set", json!({ "density": n })));
                    }
                    let name = self.i18n.tr("tree-arrows-scale");
                    ui.label(RichText::new(&name).small());
                    let mut x = a.scale;
                    let r = ui
                        .add(
                            egui::DragValue::new(&mut x)
                                .range(0.01..=MAX_SCALE)
                                .speed(0.02)
                                .max_decimals(2),
                        )
                        .on_hover_text(self.i18n.tr("tree-arrows-scale-tip"));
                    r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name.clone()));
                    if x != a.scale && x > 0.0 {
                        action = Some(("arrows.set", json!({ "scale": x })));
                    }
                });
                // Nothing to draw them on yet.
                if a.enabled && !a.surface && !d.slices.iter().any(|x| x.enabled) {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.label(
                            RichText::new(self.i18n.tr("tree-arrows-nowhere"))
                                .small()
                                .color(secondary),
                        );
                    });
                }
            }
            if let Some(a) = self.stream_rows(ui, p, ramp_ends) {
                action = Some(a);
            }
            // One button per line, indented like the rows' labels.
            let adds = [
                (
                    d.slices.len() < MAX_SLICES,
                    "tree-slice-new",
                    "cmd-slice-new",
                    ("slice.set", json!({"new": true, "axis": "x"})),
                ),
                (
                    has_fields && d.isos.len() < MAX_ISOS,
                    "tree-iso-new",
                    "tree-iso-new-tip",
                    ("iso.set", json!({"new": true})),
                ),
                (
                    has_vector && d.arrows.is_none(),
                    "tree-arrows-new",
                    "tree-arrows-new-tip",
                    ("arrows.set", json!({})),
                ),
            ];
            for (show, key, tip, call) in adds {
                if !show {
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    let b = ui
                        .add(egui::Button::new(RichText::new(self.i18n.tr(key)).small()))
                        .on_hover_text(self.i18n.tr(tip));
                    if b.clicked() {
                        action = Some(call);
                    }
                });
            }
            if let Some(a) = self.stream_add(ui) {
                action = Some(a);
            }
            if d.extracts_shown() {
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    let mut faces = d.mesh_look == MeshLook::Faces;
                    let r = ui
                        .checkbox(
                            &mut faces,
                            RichText::new(self.i18n.tr("tree-mesh-faces")).small(),
                        )
                        .on_hover_text(self.i18n.tr("tree-mesh-faces-tip"));
                    if r.changed() {
                        let look = if faces { "faces" } else { "outline" };
                        action = Some(("display.mesh_look", json!({ "look": look })));
                    }
                });
            }
        });
        action
    }

    /// The iso-surfaces of the field the bar shows: their marks beside
    /// the bar (the current one larger), and what a click on the bar at
    /// height `y` does: move the selected iso-surface of this field to
    /// the value there, else add one.
    pub(crate) fn iso_marks(&self, painter: &egui::Painter, bar: Rect, ramp: &Ramp) {
        let Some(shown) = self.session.field_values() else {
            return;
        };
        let d = &self.session.display;
        let current = self.selected_iso();
        for (k, c) in d.isos.iter().enumerate() {
            if !c.enabled || c.field != shown.choice || ramp.hi <= ramp.lo {
                continue;
            }
            let t = ramp.position(c.value);
            if !(0.0..=1.0).contains(&t) {
                continue;
            }
            let y = bar.bottom() - t as f32 * bar.height();
            let big = current == Some(k);
            let s = if big { 6.0 } else { 4.5 };
            let x = bar.right() + 2.0;
            let rgb = murmel_engine::extract::iso_color(d, &self.session.tokens, k);
            let fill = color(rgb);
            let edge = color(self.session.tokens.mesh.field.line);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(x, y),
                    Pos2::new(x + s * 1.6, y - s),
                    Pos2::new(x + s * 1.6, y + s),
                ],
                fill,
                Stroke::new(1.0, edge),
            ));
        }
    }

    /// The iso-surface selected last, if any.
    fn selected_iso(&self) -> Option<usize> {
        let d = &self.session.display;
        d.selection.iter().rev().find_map(|&i| match i {
            Item::Iso(k) if (k as usize) < d.isos.len() => Some(k as usize),
            _ => None,
        })
    }

    /// The call a click on the colour bar makes at value `v`: move the
    /// selected iso-surface when it is of the field shown, else add one.
    pub(crate) fn iso_click(&self, v: f64) -> Option<(&'static str, Value)> {
        let shown = self.session.field_values()?;
        let d = &self.session.display;
        match self.selected_iso() {
            Some(k) if d.isos[k].field == shown.choice => Some((
                "iso.set",
                json!({"iso": k + 1, "value": v, "enabled": true}),
            )),
            _ if d.isos.len() < MAX_ISOS => Some((
                "iso.set",
                json!({
                    "new": true,
                    "field": shown.choice.name,
                    "component": shown.choice.component,
                    "value": v,
                }),
            )),
            _ => None,
        }
    }
}

/// The value at height `y` of the bar, rounded to a tenth of the ticks'
/// step, so a click gives a short number.
pub(crate) fn bar_value(bar: Rect, ramp: &Ramp, y: f32) -> f64 {
    let t = ((bar.bottom() - y) / bar.height()).clamp(0.0, 1.0) as f64;
    let v = ramp.lo + t * (ramp.hi - ramp.lo);
    let step = match ramp.ticks.as_slice() {
        [a, b, ..] if b > a => (b - a) / 10.0,
        _ => (ramp.hi - ramp.lo) / 100.0,
    };
    if step > 0.0 && step.is_finite() {
        ((v / step).round() * step).clamp(ramp.lo, ramp.hi)
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A click on the bar gives the value at that height, rounded to a
    /// tenth of the ticks' step and kept in the range.
    #[test]
    fn bar_values_are_short() {
        let ramp = Ramp {
            stops: Vec::new(),
            lo: 0.0,
            hi: 10.0,
            ticks: vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0],
            unit: None,
            bands: None,
            contours: Vec::new(),
            threshold: None,
        };
        let bar = Rect::from_min_size(Pos2::new(0.0, 0.0), egui::vec2(16.0, 100.0));
        assert_eq!(bar_value(bar, &ramp, 100.0), 0.0);
        assert_eq!(bar_value(bar, &ramp, 0.0), 10.0);
        assert!((bar_value(bar, &ramp, 66.0) - 3.4).abs() < 1e-9);
        assert_eq!(bar_value(bar, &ramp, -20.0), 10.0);
    }
}
