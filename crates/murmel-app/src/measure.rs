//! Measuring in the window (decided by the maintainer on 2026-11-24:
//! "Measure mode with snapping" and "Pinned probes in the tree"). M,
//! Shift + M and Shift + P start the measure mode; while it is on, the
//! pointer asks `measure.snap` for the point under it and shows a marker
//! of what it snapped to, a click places that point, and the last point
//! ends in `measure.add` or `probe.add`. Alt (Option) takes the point of
//! the surface without snapping; Esc stops.
//!
//! The measurements and probes kept in the session are drawn over the
//! viewport with the egui painter (like the clip handles, so screenshots
//! leave them out): lines with a halo, an arc at an angle's vertex, a
//! diamond for a probe, and a label with the value. A click on a label
//! selects its item.

use egui::{Color32, Pos2, Rect, RichText, Role, Shape, Stroke, Ui, WidgetInfo};
use murmel_engine::{Anchor, Camera, Item, MeasureKind, Measurement, SnapKind};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::panels::number;
use crate::style::{color, icon};

/// What the measure mode places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Distance,
    Angle,
    Probe,
}

impl Mode {
    /// How many points it takes.
    pub fn points(self) -> usize {
        match self {
            Mode::Distance => 2,
            Mode::Angle => 3,
            Mode::Probe => 1,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Mode::Distance => "distance",
            Mode::Angle => "angle",
            Mode::Probe => "probe",
        }
    }
}

/// The measure mode while it is on: what it places and the points placed
/// so far.
#[derive(Clone, Debug, PartialEq)]
pub struct Measuring {
    pub mode: Mode,
    pub points: Vec<Anchor>,
}

/// The snap under the resting pointer, kept while the pointer, the view
/// and the surface stay as they were.
pub struct SnapHover {
    key: String,
    pub anchor: Option<Anchor>,
}

/// Where a label was drawn last frame, so a click there selects its item.
#[derive(Clone, Copy, Debug)]
pub struct Label {
    pub item: Item,
    pub rect: Rect,
}

/// The radius of an angle's arc, in points.
const ARC_RADIUS: f32 = 22.0;

impl MurmelApp {
    /// Start the measure mode `mode`, or stop it when it is on already.
    pub(crate) fn toggle_measuring(&mut self, mode: Mode) {
        if self.measuring.as_ref().is_some_and(|m| m.mode == mode) || self.session.mesh.is_none() {
            self.measuring = None;
        } else {
            self.measuring = Some(Measuring {
                mode,
                points: Vec::new(),
            });
        }
        self.snap_hover = None;
    }

    /// The name a measurement or probe shows: the one given, else
    /// "Distance n", "Angle n" or "Probe n".
    pub(crate) fn measure_name(&self, item: Item) -> String {
        let d = &self.session.display;
        let (given, key, n) = match item {
            Item::Measure(n) => {
                let m = d.measurement(n);
                (
                    m.and_then(|m| m.name.clone()),
                    match m.map(|m| m.kind) {
                        Some(MeasureKind::Angle) => "measure-name-angle",
                        _ => "measure-name-distance",
                    },
                    n,
                )
            }
            Item::Probe(n) => (
                d.probe(n).and_then(|p| p.name.clone()),
                "measure-name-probe",
                n,
            ),
            _ => return String::new(),
        };
        given.unwrap_or_else(|| self.i18n.tr_args(key, &[("n", n.into())]))
    }

    /// A measurement's value as shown: `0.0125 m`, `47.5°`.
    pub(crate) fn measure_value(&self, m: &Measurement) -> String {
        match m.kind {
            MeasureKind::Distance => {
                format!("{} {}", number(m.value()), self.i18n.tr("status-units"))
            }
            MeasureKind::Angle => format!("{}°", number(round(m.value(), 2))),
        }
    }

    /// What probe `p` shows of the field colouring the mesh (the
    /// maintainer's card of 2026-11-24, "Shown field on label and
    /// tree"): `|velocity| 12.3 m/s`; `None` while no field colours the
    /// mesh or outside it.
    pub(crate) fn probe_shown(&self, p: &murmel_engine::measure::Probe) -> Option<String> {
        let m = self.session.mesh.as_ref()?;
        let s = murmel_engine::inspect::probe_shown(m, &self.session.display, p)?;
        Some(match &s.unit {
            Some(u) => format!("{} {} {u}", s.title, number(s.value)),
            None => format!("{} {}", s.title, number(s.value)),
        })
    }

    /// The status bar's hint while measuring.
    pub(crate) fn measure_hint(&self) -> Option<String> {
        let m = self.measuring.as_ref()?;
        let n = (m.points.len() + 1) as i64;
        Some(self.i18n.tr_args(
            &format!("measure-hint-{}", m.mode.key()),
            &[("n", n.into())],
        ))
    }

    /// Ask `measure.snap` for the point under `pos` (cached while nothing
    /// moves); Alt takes the surface point without snapping.
    pub(crate) fn snap_at(&mut self, pos: Pos2, rect: Rect, alt: bool) -> Option<Anchor> {
        let key = format!(
            "{pos:?}|{alt}|{:?}|{}|{rect:?}|{:?}",
            self.session.camera, self.session.surface_revision, self.snap_kinds
        );
        if let Some(h) = &self.snap_hover
            && h.key == key
        {
            return h.anchor.clone();
        }
        let kinds: Vec<SnapKind> = if alt {
            vec![SnapKind::Surface]
        } else {
            self.snap_kinds.clone()
        };
        let e = murmel_tools::call(
            "measure.snap",
            json!({"ndc": crate::viewport::ndc(pos, rect), "snap": kinds}),
            &mut self.session,
        );
        let anchor = e
            .ok()
            .then(|| serde_json::from_value::<Anchor>(e.result["anchor"].clone()).ok())
            .flatten();
        self.snap_hover = Some(SnapHover {
            key,
            anchor: anchor.clone(),
        });
        anchor
    }

    /// A click in the viewport while measuring: place the point under it;
    /// the last point of the mode adds the measurement or the probe.
    pub(crate) fn measure_click(&mut self, pos: Pos2, rect: Rect, alt: bool) {
        let Some(a) = self.snap_at(pos, rect, alt) else {
            return;
        };
        let Some(m) = self.measuring.as_mut() else {
            return;
        };
        m.points.push(a);
        if m.points.len() < m.mode.points() {
            return;
        }
        let mode = m.mode;
        let points: Vec<Value> = std::mem::take(&mut m.points)
            .into_iter()
            .map(|a| json!({ "anchor": a }))
            .collect();
        match mode {
            Mode::Probe => {
                self.call("probe.add", json!({"at": points[0]}));
            }
            Mode::Distance | Mode::Angle => {
                let kind = if mode == Mode::Angle {
                    "angle"
                } else {
                    "distance"
                };
                self.call("measure.add", json!({"kind": kind, "points": points}));
            }
        }
    }

    /// The measurement or probe whose label is at `pos`, as drawn last
    /// frame.
    pub(crate) fn measure_label_at(&self, pos: Pos2) -> Option<Item> {
        self.measure_labels
            .iter()
            .rev()
            .find(|l| l.rect.contains(pos))
            .map(|l| l.item)
    }

    /// Draw the measurements and probes, and while measuring the points
    /// placed, the snap marker under the pointer and the measurement it
    /// would make.
    pub(crate) fn measure_overlay(
        &mut self,
        ui: &Ui,
        rect: Rect,
        cam: &Camera,
        dark: bool,
        pointer: Option<(Pos2, bool)>,
    ) {
        self.measure_labels.clear();
        let Some(m) = &self.session.mesh else {
            return;
        };
        let (origin, radius) = (m.surface.origin, m.surface.radius);
        let aspect = (rect.width() / rect.height().max(1.0)) as f64;
        let cam = cam.clone();
        let to_screen = move |p: [f64; 3]| -> Option<Pos2> {
            let [x, y] = cam.project(origin, radius, aspect, p)?;
            Some(Pos2::new(
                rect.left() + ((x + 1.0) * 0.5) as f32 * rect.width(),
                rect.top() + ((1.0 - y) * 0.5) as f32 * rect.height(),
            ))
        };
        let palette = self.session.tokens.palette(dark).clone();
        let mc = *self.session.tokens.measure.colors(dark);
        let selection = color(self.session.tokens.mesh.selection);
        let painter = ui.painter_at(rect);
        let style = Style {
            line: color(mc.line),
            probe: color(mc.probe),
            halo: color(mc.halo),
            text: color(palette.text_primary),
            bg: color(palette.bg_panel).gamma_multiply(0.92),
        };
        let d = &self.session.display;
        let mut labels = Vec::new();
        for ms in d.measurements.iter().filter(|m| m.visible) {
            let item = Item::Measure(ms.n);
            let pts: Option<Vec<Pos2>> = ms.points.iter().map(|a| to_screen(a.point)).collect();
            let Some(pts) = pts else { continue };
            let c = if d.is_selected(item) {
                selection
            } else {
                style.line
            };
            let text = match &ms.name {
                Some(n) => format!("{n}  {}", self.measure_value(ms)),
                None => self.measure_value(ms),
            };
            if let Some(r) = draw_measure(&painter, &style, c, ms.kind, &pts, &text, false) {
                labels.push(Label { item, rect: r });
            }
        }
        for p in d.probes.iter().filter(|p| p.visible) {
            let item = Item::Probe(p.n);
            let Some(at) = to_screen(p.at.point) else {
                continue;
            };
            let c = if d.is_selected(item) {
                selection
            } else {
                style.probe
            };
            let name = self.measure_name(item);
            let text = match (p.at.cell, self.probe_shown(p)) {
                (Some(_), Some(value)) => self.i18n.tr_args(
                    "measure-label-probe-value",
                    &[("name", name.into()), ("value", value.into())],
                ),
                (Some(cell), None) => self.i18n.tr_args(
                    "measure-label-probe",
                    &[("name", name.into()), ("cell", cell.to_string().into())],
                ),
                (None, _) => self
                    .i18n
                    .tr_args("measure-label-probe-outside", &[("name", name.into())]),
            };
            diamond(&painter, at, 6.0, c, style.halo);
            labels.push(Label {
                item,
                rect: label(
                    &painter,
                    &style,
                    c,
                    at + egui::vec2(10.0, -10.0),
                    &text,
                    false,
                    false,
                ),
            });
        }
        self.measure_labels = labels;

        // The measure mode: the points placed, and what the pointer would
        // add.
        let Some(mode) = self.measuring.as_ref().map(|m| m.mode) else {
            return;
        };
        let hover = pointer.and_then(|(pos, alt)| self.snap_at(pos, rect, alt));
        let Some(measuring) = &self.measuring else {
            return;
        };
        let mut placed = measuring.points.clone();
        if let Some(h) = &hover {
            placed.push(h.clone());
        }
        let pts: Vec<Pos2> = placed.iter().filter_map(|a| to_screen(a.point)).collect();
        if pts.len() == placed.len() {
            match mode {
                Mode::Distance | Mode::Angle if pts.len() >= 2 => {
                    let kind = if mode == Mode::Angle && pts.len() == 3 {
                        MeasureKind::Angle
                    } else {
                        MeasureKind::Distance
                    };
                    // Two points of an angle show the first ray only.
                    let (kind, used) = if mode == Mode::Angle && pts.len() == 2 {
                        (MeasureKind::Distance, &placed[..2])
                    } else {
                        (kind, &placed[..])
                    };
                    let live = Measurement {
                        n: 0,
                        kind,
                        name: None,
                        points: used.to_vec(),
                        visible: true,
                    };
                    let text = if mode == Mode::Angle && kind == MeasureKind::Distance {
                        String::new()
                    } else {
                        self.measure_value(&live)
                    };
                    draw_measure(&painter, &style, style.line, kind, &pts, &text, true);
                }
                _ => {
                    for &p in &pts {
                        dot(&painter, p, style.line, style.halo);
                    }
                }
            }
        }
        if let Some(h) = &hover
            && let Some(at) = to_screen(h.point)
        {
            let c = if mode == Mode::Probe {
                style.probe
            } else {
                style.line
            };
            snap_marker(&painter, at, h.snap, c, style.halo);
            let tag = self.i18n.tr(&format!("snap-{}", h.snap.key()));
            label(
                &painter,
                &style,
                c,
                at + egui::vec2(12.0, 8.0),
                &tag,
                true,
                false,
            );
        }
    }

    /// The tree's "Measurements" and "Probes" groups: a row per item (the
    /// eye shows or hides it, a click selects, a double-click renames,
    /// the context menu fits, renames, copies the value or deletes) and a
    /// button that starts the measure mode.
    pub(crate) fn measure_section(
        &mut self,
        ui: &mut Ui,
        p: &murmel_engine::theme::Palette,
        filter: &str,
    ) -> Option<(&'static str, Value)> {
        let mesh = &self.session.mesh.as_ref()?.mesh;
        let secondary = color(p.text_secondary);
        let d = &self.session.display;
        // Each row: the item, its id, name and value, whether it shows
        // and whether it is selected.
        type RowData = (Item, String, String, String, bool, bool);
        let rows = |probes: bool| -> Vec<RowData> {
            let mut v = Vec::new();
            if probes {
                for pr in &d.probes {
                    let item = Item::Probe(pr.n);
                    // The field colouring the mesh, else the cell.
                    let value = match (pr.at.cell, self.probe_shown(pr)) {
                        (Some(_), Some(v)) => v,
                        (Some(c), None) => format!("cell:{c}"),
                        (None, _) => "–".to_string(),
                    };
                    v.push((
                        item,
                        item.id(mesh),
                        self.measure_name(item),
                        value,
                        pr.visible,
                        d.is_selected(item),
                    ));
                }
            } else {
                for ms in &d.measurements {
                    let item = Item::Measure(ms.n);
                    v.push((
                        item,
                        item.id(mesh),
                        self.measure_name(item),
                        self.measure_value(ms),
                        ms.visible,
                        d.is_selected(item),
                    ));
                }
            }
            v.retain(|r| filter.is_empty() || r.2.to_lowercase().contains(filter));
            v
        };
        let groups = [
            ("tree-measurements", rows(false), Mode::Distance),
            ("tree-probes", rows(true), Mode::Probe),
        ];
        let mut action = None;
        let mut start = None;
        let mut history = false;
        let any_probe = !d.probes.is_empty();
        for (section, rows, mode) in groups {
            if rows.is_empty() && !filter.is_empty() {
                continue;
            }
            egui::CollapsingHeader::new(
                RichText::new(self.i18n.tr(section).to_uppercase())
                    .small()
                    .color(secondary),
            )
            .id_salt(section)
            .default_open(true)
            .show(ui, |ui| {
                for (_, id, name, value, visible, selected) in &rows {
                    let (id, selected) = (id.clone(), *selected);
                    let editing = self.measure_edit.as_ref().is_some_and(|(i, _)| *i == id);
                    if editing {
                        if let Some(a) = self.measure_name_edit(ui) {
                            action = Some(a);
                        }
                        continue;
                    }
                    ui.horizontal(|ui| {
                        let eye = if *visible { icon::EYE } else { icon::EYE_OFF };
                        let tip = self.i18n.tr_args(
                            if *visible { "tree-hide" } else { "tree-show" },
                            &[("item", name.clone().into())],
                        );
                        let b = ui
                            .add(
                                egui::Button::new(RichText::new(eye).size(14.0).color(
                                    if *visible {
                                        color(p.text_primary)
                                    } else {
                                        secondary
                                    },
                                ))
                                .frame(false)
                                .min_size(egui::vec2(20.0, 20.0)),
                            )
                            .on_hover_text(&tip);
                        b.widget_info(|| WidgetInfo::labeled(Role::Button, true, tip.clone()));
                        if b.clicked() {
                            let verb = if *visible { "hide" } else { "show" };
                            action = Some((
                                "display.visibility",
                                json!({"action": verb, "items": [id]}),
                            ));
                        }
                        let text = if *visible {
                            RichText::new(name)
                        } else {
                            RichText::new(name).color(secondary)
                        };
                        let r = ui.selectable_label(selected, text);
                        let row = self.i18n.tr_args(
                            "tree-measure-row",
                            &[
                                ("item", name.clone().into()),
                                ("value", value.clone().into()),
                            ],
                        );
                        r.widget_info(|| {
                            WidgetInfo::selected(Role::Button, true, selected, row.clone())
                        });
                        let r = r.on_hover_text(self.i18n.tr("tree-measure-tip"));
                        if r.clicked() {
                            let add = ui.input(|i| i.modifiers.shift);
                            action = Some(("select.item", json!({"item": id, "add": add})));
                        }
                        if r.double_clicked() {
                            self.measure_edit = Some((id.clone(), name.clone()));
                        }
                        r.context_menu(|ui| {
                            if ui.button(self.i18n.tr("tree-fit")).clicked() {
                                action = Some(("view.fit", json!({"item": id})));
                                ui.close();
                            }
                            if ui.button(self.i18n.tr("tree-measure-rename")).clicked() {
                                self.measure_edit = Some((id.clone(), name.clone()));
                                ui.close();
                            }
                            if ui.button(self.i18n.tr("tree-measure-copy")).clicked() {
                                ui.ctx().copy_text(value.clone());
                                ui.close();
                            }
                            if ui.button(self.i18n.tr("tree-measure-delete")).clicked() {
                                action = Some(("measure.remove", json!({"items": [id]})));
                                ui.close();
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(value).monospace().color(secondary))
                                    .truncate(),
                            );
                        });
                    });
                }
                if !filter.is_empty() {
                    return;
                }
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    let (key, tip) = if mode == Mode::Probe {
                        ("tree-probe-new", "tree-probe-new-tip")
                    } else {
                        ("tree-measure-new", "tree-measure-new-tip")
                    };
                    let b = ui
                        .add(egui::Button::new(RichText::new(self.i18n.tr(key)).small()))
                        .on_hover_text(self.i18n.tr(tip));
                    if b.clicked() {
                        start = Some(mode);
                    }
                    if mode == Mode::Probe && any_probe {
                        let h = ui
                            .add(egui::Button::new(
                                RichText::new(self.i18n.tr("tree-probe-history")).small(),
                            ))
                            .on_hover_text(self.i18n.tr("tree-probe-history-tip"));
                        if h.clicked() {
                            history = true;
                        }
                    }
                });
            });
        }
        if let Some(m) = start {
            self.toggle_measuring(m);
        }
        if history {
            self.open_probe_history();
        }
        action
    }

    /// The text field of a measurement or probe being renamed: Enter or
    /// leaving it renames, Escape keeps the name, an empty name goes back
    /// to the default.
    fn measure_name_edit(&mut self, ui: &mut Ui) -> Option<(&'static str, Value)> {
        let (id, mut text) = self.measure_edit.clone()?;
        let old = self
            .session
            .mesh
            .as_ref()
            .and_then(|m| Item::parse(&id, &m.mesh))
            .map(|i| self.measure_name(i))
            .unwrap_or_default();
        let mut action = None;
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(f32::INFINITY)
                    .char_limit(murmel_engine::display::VIEW_NAME_MAX),
            );
            let name = self
                .i18n
                .tr_args("tree-measure-rename-field", &[("item", old.clone().into())]);
            crate::focus::set_name(ui, r.id, &name);
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.measure_edit = None;
            } else if r.lost_focus() {
                self.measure_edit = None;
                let t = text.trim();
                if t != old {
                    let name = (!t.is_empty()).then(|| t.to_string());
                    action = Some(("measure.rename", json!({"item": id, "name": name})));
                }
            } else {
                self.measure_edit = Some((id.clone(), text.clone()));
            }
        });
        action
    }

    /// The Measure button of the toolbar (one button, so the toolbar
    /// still fits at 1440 points): a menu of distance, angle and probe
    /// (checked while their mode is on) and what points snap to. While
    /// measuring it shows the mode and is drawn selected.
    pub(crate) fn measure_group(&mut self, ui: &mut Ui) {
        let has_mesh = self.session.mesh.is_some();
        let now = self.measuring.as_ref().map(|m| m.mode);
        let title = match now {
            Some(m) => self.i18n.tr(mode_key(m)),
            None => self.i18n.tr("toolbar-measure"),
        };
        let name = match now {
            Some(m) => self.i18n.tr_args(
                "toolbar-measure-on",
                &[("mode", self.i18n.tr(mode_key(m)).into())],
            ),
            None => self.i18n.tr("toolbar-measure"),
        };
        let mut toggle = None;
        let mut stop = false;
        ui.add_enabled_ui(has_mesh, |ui| {
            let button =
                egui::Button::new(RichText::new(&title).size(13.0)).selected(now.is_some());
            let (r, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                for mode in [Mode::Distance, Mode::Angle, Mode::Probe] {
                    let label = self.i18n.tr(&format!("{}-item", mode_key(mode)));
                    let on = now == Some(mode);
                    if ui.selectable_label(on, label).clicked() {
                        toggle = Some(mode);
                        ui.close();
                    }
                }
                if now.is_some() && ui.button(self.i18n.tr("toolbar-measure-stop")).clicked() {
                    stop = true;
                    ui.close();
                }
                ui.separator();
                ui.label(
                    RichText::new(self.i18n.tr("toolbar-snap"))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                for k in SnapKind::SNAP {
                    let mut on = self.snap_kinds.contains(&k);
                    let name = self.i18n.tr(&format!("snap-{}", k.key()));
                    if ui.checkbox(&mut on, name).changed() {
                        if on {
                            self.snap_kinds.push(k);
                        } else {
                            self.snap_kinds.retain(|&x| x != k);
                        }
                        self.snap_hover = None;
                    }
                }
            });
            let tip = self.i18n.tr("toolbar-measure-tip");
            r.on_hover_text(tip).widget_info(|| {
                WidgetInfo::selected(Role::Button, true, now.is_some(), name.clone())
            });
        });
        if let Some(m) = toggle {
            self.toggle_measuring(m);
        }
        if stop {
            self.measuring = None;
            self.snap_hover = None;
        }
    }
}

/// The translation key of a mode's toolbar name.
fn mode_key(m: Mode) -> &'static str {
    match m {
        Mode::Distance => "toolbar-measure-distance",
        Mode::Angle => "toolbar-measure-angle",
        Mode::Probe => "toolbar-probe",
    }
}

fn round(v: f64, digits: i32) -> f64 {
    let s = 10f64.powi(digits);
    (v * s).round() / s
}

/// Colours of the overlay.
struct Style {
    line: Color32,
    probe: Color32,
    halo: Color32,
    text: Color32,
    bg: Color32,
}

/// A line with a halo under it; dashed while the measurement is only
/// shown, not kept.
fn line(painter: &egui::Painter, pts: &[Pos2], c: Color32, halo: Color32, dashed: bool) {
    if dashed {
        painter.extend(Shape::dashed_line(pts, Stroke::new(4.0, halo), 7.0, 5.0));
        painter.extend(Shape::dashed_line(pts, Stroke::new(2.0, c), 7.0, 5.0));
    } else {
        painter.add(Shape::line(pts.to_vec(), Stroke::new(4.5, halo)));
        painter.add(Shape::line(pts.to_vec(), Stroke::new(2.0, c)));
    }
}

fn dot(painter: &egui::Painter, p: Pos2, c: Color32, halo: Color32) {
    painter.circle(p, 3.5, c, Stroke::new(1.5, halo));
}

fn diamond(painter: &egui::Painter, p: Pos2, r: f32, c: Color32, halo: Color32) {
    let pts = vec![
        p + egui::vec2(0.0, -r),
        p + egui::vec2(r, 0.0),
        p + egui::vec2(0.0, r),
        p + egui::vec2(-r, 0.0),
    ];
    painter.add(Shape::convex_polygon(pts, c, Stroke::new(1.5, halo)));
    painter.circle_filled(p, 1.5, halo);
}

/// The marker of what the pointer snaps to: a square on a node, a
/// triangle on an edge's midpoint, a circle on a face centre, a ring with
/// a dot on a cell centre, a diamond on an edge, a cross on the surface.
fn snap_marker(painter: &egui::Painter, p: Pos2, kind: SnapKind, c: Color32, halo: Color32) {
    let r = 6.0;
    let halo_s = Stroke::new(4.0, halo);
    let s = Stroke::new(2.0, c);
    match kind {
        SnapKind::Node => {
            let rect = Rect::from_center_size(p, egui::vec2(2.0 * r, 2.0 * r));
            painter.rect_stroke(rect, 0.0, halo_s, egui::StrokeKind::Middle);
            painter.rect_stroke(rect, 0.0, s, egui::StrokeKind::Middle);
        }
        SnapKind::EdgeMidpoint => {
            let pts = vec![
                p + egui::vec2(0.0, -r * 1.15),
                p + egui::vec2(r, r * 0.75),
                p + egui::vec2(-r, r * 0.75),
            ];
            painter.add(Shape::closed_line(pts.clone(), halo_s));
            painter.add(Shape::closed_line(pts, s));
        }
        SnapKind::FaceCentre => {
            painter.circle_stroke(p, r, halo_s);
            painter.circle_stroke(p, r, s);
        }
        SnapKind::CellCentre => {
            painter.circle_stroke(p, r, halo_s);
            painter.circle_stroke(p, r, s);
            painter.circle_filled(p, 2.0, c);
        }
        SnapKind::Edge => {
            let pts = vec![
                p + egui::vec2(0.0, -r),
                p + egui::vec2(r, 0.0),
                p + egui::vec2(0.0, r),
                p + egui::vec2(-r, 0.0),
            ];
            painter.add(Shape::closed_line(pts.clone(), halo_s));
            painter.add(Shape::closed_line(pts, s));
        }
        SnapKind::Surface | SnapKind::Typed => {
            for (a, b) in [
                (egui::vec2(-r, 0.0), egui::vec2(r, 0.0)),
                (egui::vec2(0.0, -r), egui::vec2(0.0, r)),
            ] {
                painter.line_segment([p + a, p + b], halo_s);
                painter.line_segment([p + a, p + b], s);
            }
        }
    }
}

/// A label with its top left at `at` (its centre when `centred`); its
/// rectangle.
fn label(
    painter: &egui::Painter,
    style: &Style,
    border: Color32,
    at: Pos2,
    text: &str,
    small: bool,
    centred: bool,
) -> Rect {
    let font = egui::FontId::proportional(if small { 11.0 } else { 12.5 });
    let galley = painter.layout_no_wrap(text.to_string(), font, style.text);
    let size = galley.size() + egui::vec2(10.0, 4.0);
    let min = if centred { at - size * 0.5 } else { at };
    let rect = Rect::from_min_size(min, size);
    painter.rect(
        rect,
        4.0,
        style.bg,
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    painter.galley(min + egui::vec2(5.0, 2.0), galley, style.text);
    rect
}

/// Draw a distance (two points) or an angle (three, the vertex second)
/// at screen points `pts`; the label's rectangle, when it has text.
fn draw_measure(
    painter: &egui::Painter,
    style: &Style,
    c: Color32,
    kind: MeasureKind,
    pts: &[Pos2],
    text: &str,
    dashed: bool,
) -> Option<Rect> {
    let at = match (kind, pts) {
        (MeasureKind::Distance, [a, b, ..]) => {
            line(painter, &[*a, *b], c, style.halo, dashed);
            let mid = a.lerp(*b, 0.5);
            // Beside the line, on the side away from its normal's down.
            let dir = (*b - *a).normalized();
            let side = egui::vec2(-dir.y, dir.x);
            let side = if side.y > 0.0 { -side } else { side };
            mid + side * 14.0
        }
        (MeasureKind::Angle, [a, v, b, ..]) => {
            line(painter, &[*a, *v, *b], c, style.halo, dashed);
            let (u, w) = ((*a - *v).normalized(), (*b - *v).normalized());
            let (t0, mut t1) = (u.y.atan2(u.x), w.y.atan2(w.x));
            // The short way round, as the angle is measured.
            if t1 - t0 > std::f32::consts::PI {
                t1 -= std::f32::consts::TAU;
            } else if t0 - t1 > std::f32::consts::PI {
                t1 += std::f32::consts::TAU;
            }
            let arc: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = t0 + (t1 - t0) * i as f32 / 24.0;
                    *v + egui::vec2(t.cos(), t.sin()) * ARC_RADIUS
                })
                .collect();
            if u.length() > 0.0 && w.length() > 0.0 {
                line(painter, &arc, c, style.halo, dashed);
            }
            let tm = 0.5 * (t0 + t1);
            *v + egui::vec2(tm.cos(), tm.sin()) * (ARC_RADIUS + 10.0)
        }
        _ => return None,
    };
    for &p in pts {
        dot(painter, p, c, style.halo);
    }
    if text.is_empty() {
        return None;
    }
    Some(label(painter, style, c, at, text, false, true))
}
