//! The streamlines in the window (field results, stage 4): a row per set
//! in the model tree's "Slices, iso-surfaces, arrows and streamlines"
//! (the eye turns it on and off, a click selects it so its seed handles
//! show, a double-click fits it, the context menu sets the direction,
//! the look, the colour and the seeds, and removes it), the seeds and the
//! length under the row, the menu that adds a set, and the settings of
//! the sets' own colour bars (decided 2026-11-24: their own field and
//! their own colour bar; drawn by [`crate::own_bars`]). Every change
//! calls a tool.

use egui::containers::menu::MenuButton;
use egui::{Rect, RichText, Role, Sense, Ui, WidgetInfo};
use murmel_engine::field::Component;
use murmel_engine::legend::{Body, Legend, Owner};
use murmel_engine::stream::{MAX_COUNT, MAX_STREAMS, MIN_COUNT};
use murmel_engine::{Direction, Item, Look, Seed};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::style::color;

impl MurmelApp {
    /// The label of streamline set `k`: its number, its field and its
    /// seeds.
    pub(crate) fn stream_label(&self, k: usize) -> String {
        let Some(st) = self.session.display.streams.get(k) else {
            return String::new();
        };
        let seed = match &st.seed {
            Seed::Patch { patch } => self
                .i18n
                .tr_args("stream-seed-patch", &[("patch", patch.clone().into())]),
            s => self.i18n.tr(&format!("stream-seed-{}", s.key())),
        };
        self.i18n.tr_args(
            "tree-stream-label",
            &[
                ("n", (k + 1).into()),
                ("field", st.field.clone().into()),
                ("seed", seed.into()),
            ],
        )
    }

    /// The tree rows of the streamline sets.
    pub(crate) fn stream_rows(
        &self,
        ui: &mut Ui,
        p: &murmel_engine::theme::Palette,
        main_ramp: Option<(murmel_engine::Rgb, murmel_engine::Rgb)>,
    ) -> Option<(&'static str, Value)> {
        let m = self.session.mesh.as_ref()?;
        let mesh = &m.mesh;
        let d = &self.session.display;
        let tokens = &self.session.tokens;
        let secondary = color(p.text_secondary);
        let bars = Legend::own_bars(&self.session);
        let names: Vec<String> = m.fields.names().iter().map(|n| n.to_string()).collect();
        let vectors: Vec<&String> = names
            .iter()
            .filter(|n| m.fields.get(n).is_some_and(|f| f.is_vector()))
            .collect();
        let mut action = None;
        for (k, st) in d.streams.iter().enumerate() {
            let item = Item::Stream(k as u32);
            let id = item.id(mesh);
            let n = k + 1;
            let label = self.stream_label(k);
            let set = |extra: Value| {
                let mut v = json!({ "stream": n });
                if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
                    o.extend(e);
                }
                ("stream.set", v)
            };
            ui.horizontal(|ui| {
                if self.eye(ui, st.enabled, &label, p).clicked() {
                    action = Some(set(json!({ "enabled": !st.enabled })));
                }
                // The swatch: the solid colour, or the ends of its bar.
                let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                match &st.color {
                    None => {
                        let rgb = murmel_engine::stream::solid_color(d, tokens, k);
                        ui.painter().rect_filled(rect, 2.0, color(rgb));
                    }
                    Some(_) => {
                        let none = tokens.mesh.field.none;
                        let ends = bars
                            .iter()
                            .find(|(o, _)| matches!(o, Owner::Streams(sets) if sets.contains(&k)))
                            .and_then(|(_, l)| match &l.body {
                                Body::Ramp(r) => Some((r.sample(0.15), r.sample(0.85))),
                                Body::Categories(_) => None,
                            })
                            .or(main_ramp)
                            .unwrap_or((none, none));
                        let half = Rect::from_min_max(rect.min, rect.center_bottom());
                        ui.painter().rect_filled(half, 0.0, color(ends.0));
                        let rest = Rect::from_min_max(rect.center_top(), rect.max);
                        ui.painter().rect_filled(rest, 0.0, color(ends.1));
                    }
                }
                let selected = d.is_selected(item);
                let text = if st.enabled {
                    RichText::new(&label)
                } else {
                    RichText::new(&label).color(secondary)
                };
                let r = ui.selectable_label(selected, text);
                r.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, label.clone()));
                let r = r.on_hover_text(self.i18n.tr("tree-stream-tip"));
                if r.clicked() {
                    action = Some(("select.item", json!({ "item": id })));
                }
                if r.double_clicked() {
                    action = Some(("view.fit", json!({ "item": id })));
                }
                r.context_menu(|ui| {
                    let mut pick = |a: (&'static str, Value), ui: &mut Ui| {
                        action = Some(a);
                        ui.close();
                    };
                    // Direction.
                    ui.label(RichText::new(self.i18n.tr("tree-stream-direction")).small());
                    ui.horizontal(|ui| {
                        for dir in Direction::ALL {
                            let on = st.direction == dir;
                            let text = self.i18n.tr(&format!("stream-direction-{}", dir.key()));
                            if ui.selectable_label(on, text).clicked() && !on {
                                pick(set(json!({ "direction": dir.key() })), ui);
                            }
                        }
                    });
                    // Look.
                    let mut tubes = st.look == Look::Tubes;
                    if ui
                        .checkbox(&mut tubes, self.i18n.tr("tree-stream-tubes"))
                        .changed()
                    {
                        let look = if tubes { "tubes" } else { "lines" };
                        pick(set(json!({ "look": look })), ui);
                    }
                    let mut heads = st.arrowheads;
                    if ui
                        .checkbox(&mut heads, self.i18n.tr("tree-stream-arrowheads"))
                        .changed()
                    {
                        pick(set(json!({ "arrowheads": heads })), ui);
                    }
                    ui.separator();
                    // Colour: solid, or any field (a vector by its
                    // magnitude; the bar's menu picks a component).
                    if ui
                        .selectable_label(st.color.is_none(), self.i18n.tr("tree-stream-solid"))
                        .clicked()
                        && st.color.is_some()
                    {
                        pick(set(json!({ "color": "solid" })), ui);
                    }
                    if st.color.is_none() && ui.button(self.i18n.tr("tree-color-edit")).clicked() {
                        pick(("window.color", json!({ "item": id, "label": label })), ui);
                    }
                    ui.menu_button(self.i18n.tr("tree-stream-color-by"), |ui| {
                        for name in &names {
                            let on = st.color.as_ref().is_some_and(|c| c.name == *name);
                            if ui.selectable_label(on, name).clicked() && !on {
                                pick(set(json!({ "color_field": name })), ui);
                            }
                        }
                    });
                    // The vector field traced, when there is a choice.
                    if vectors.len() > 1 {
                        ui.menu_button(self.i18n.tr("tree-stream-vector"), |ui| {
                            for name in &vectors {
                                let on = st.field == **name;
                                if ui.selectable_label(on, name.as_str()).clicked() && !on {
                                    pick(set(json!({ "field": name })), ui);
                                }
                            }
                        });
                    }
                    ui.separator();
                    // The seeds: their shape (each starts where the
                    // tool's default puts it), a patch, a plane's normal.
                    ui.menu_button(self.i18n.tr("tree-stream-seed-shape"), |ui| {
                        for kind in Seed::KINDS {
                            if kind == "patch" {
                                ui.menu_button(self.i18n.tr("tree-stream-from-patch"), |ui| {
                                    for patch in &mesh.patches {
                                        let on = matches!(&st.seed,
                                            Seed::Patch { patch: q } if *q == patch.name);
                                        if ui.selectable_label(on, &patch.name).clicked() && !on {
                                            pick(
                                                set(json!({
                                                    "seed": "patch",
                                                    "patch": patch.name,
                                                })),
                                                ui,
                                            );
                                        }
                                    }
                                });
                                continue;
                            }
                            let on = st.seed.key() == kind;
                            let text = self.i18n.tr(&format!("tree-stream-from-{kind}"));
                            if ui.selectable_label(on, text).clicked() && !on {
                                pick(set(json!({ "seed": kind })), ui);
                            }
                        }
                    });
                    if let Seed::Plane { normal, .. } = &st.seed {
                        ui.menu_button(self.i18n.tr("tree-stream-plane-normal"), |ui| {
                            for (a, key) in [(0, "X"), (1, "Y"), (2, "Z")] {
                                let mut nn = [0.0; 3];
                                nn[a] = 1.0;
                                let on = normal[a].abs() > 0.999;
                                if ui.selectable_label(on, key).clicked() && !on {
                                    pick(set(json!({ "normal": nn })), ui);
                                }
                            }
                            let view = self.session.camera.to_eye().to_array();
                            if ui.button(self.i18n.tr("tree-stream-plane-view")).clicked() {
                                pick(set(json!({ "normal": view })), ui);
                            }
                        });
                    }
                    ui.separator();
                    if ui.button(self.i18n.tr("tree-fit")).clicked() {
                        pick(("view.fit", json!({ "item": id })), ui);
                    }
                    if ui.button(self.i18n.tr("tree-stream-remove")).clicked() {
                        pick(("stream.clear", json!({ "stream": n })), ui);
                    }
                });
            });
            // How many seeds, how long, how thick, under the row.
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                let name = self.i18n.tr("tree-stream-seeds");
                ui.label(RichText::new(&name).small());
                let mut c = st.count;
                let r = ui
                    .add(
                        egui::DragValue::new(&mut c)
                            .range(MIN_COUNT..=MAX_COUNT)
                            .speed(0.5),
                    )
                    .on_hover_text(self.i18n.tr("tree-stream-seeds-tip"));
                r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name.clone()));
                if c != st.count {
                    action = Some(set(json!({ "count": c })));
                }
                let name = self.i18n.tr("tree-stream-length");
                ui.label(RichText::new(&name).small());
                let len = st.length_limit(mesh);
                let mut l = len;
                let r = ui
                    .add(
                        egui::DragValue::new(&mut l)
                            .range(len * 1e-3..=f64::MAX)
                            .speed(len / 200.0)
                            .max_decimals(4),
                    )
                    .on_hover_text(self.i18n.tr("tree-stream-length-tip"));
                r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name.clone()));
                if l != len && l > 0.0 {
                    action = Some(set(json!({ "max_length": l })));
                }
            });
            if st.look == Look::Tubes {
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    let name = self.i18n.tr("tree-stream-radius");
                    ui.label(RichText::new(&name).small());
                    let rad = st.tube_radius(mesh);
                    let mut x = rad;
                    let r = ui
                        .add(
                            egui::DragValue::new(&mut x)
                                .range(rad * 1e-2..=f64::MAX)
                                .speed(rad / 50.0)
                                .max_decimals(5),
                        )
                        .on_hover_text(self.i18n.tr("tree-stream-radius-tip"));
                    r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name.clone()));
                    if x != rad && x > 0.0 {
                        action = Some(set(json!({ "tube_radius": x })));
                    }
                });
            }
            // Traced, but not one line: say why.
            let traced = d
                .stream_traces
                .as_ref()
                .and_then(|t| t.0.get(k).cloned().flatten())
                .filter(|t| t.fits(st));
            if st.enabled
                && let Some(t) = traced.filter(|t| t.lines.is_empty())
            {
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    let r = ui.label(
                        RichText::new(self.i18n.tr("tree-stream-none"))
                            .small()
                            .color(secondary),
                    );
                    if let Some(why) = &t.problem {
                        r.on_hover_text(why);
                    }
                });
            }
        }
        action
    }

    /// The button that adds a streamline set: a menu of the four seed
    /// shapes (a patch by name).
    pub(crate) fn stream_add(&self, ui: &mut Ui) -> Option<(&'static str, Value)> {
        let m = self.session.mesh.as_ref()?;
        let d = &self.session.display;
        let vector = m
            .fields
            .names()
            .iter()
            .any(|n| m.fields.get(n).is_some_and(|f| f.is_vector()));
        if !vector || d.streams.len() >= MAX_STREAMS {
            return None;
        }
        let mut action = None;
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            let (r, _) = MenuButton::new(RichText::new(self.i18n.tr("tree-stream-new")).small())
                .ui(ui, |ui| {
                    for kind in Seed::KINDS {
                        if kind == "patch" {
                            ui.menu_button(self.i18n.tr("tree-stream-from-patch"), |ui| {
                                for patch in &m.mesh.patches {
                                    if ui.button(&patch.name).clicked() {
                                        action = Some((
                                            "stream.set",
                                            json!({
                                                "new": true,
                                                "seed": "patch",
                                                "patch": patch.name,
                                            }),
                                        ));
                                        ui.close();
                                    }
                                }
                            });
                            continue;
                        }
                        if ui
                            .button(self.i18n.tr(&format!("tree-stream-from-{kind}")))
                            .clicked()
                        {
                            action = Some(("stream.set", json!({"new": true, "seed": kind})));
                            ui.close();
                        }
                    }
                });
            r.on_hover_text(self.i18n.tr("tree-stream-new-tip"));
        });
        action
    }

    /// The settings of one streamline colour bar: the colour field of its
    /// sets (`stream.set` on each), a vector's component, and the
    /// field's style (`field.style`, shared with the mesh's legend).
    pub(crate) fn stream_bar_menu(
        &self,
        ui: &mut Ui,
        sets: &[usize],
        choice: &murmel_engine::field::FieldChoice,
        calls: &mut Vec<(&'static str, Value)>,
    ) {
        let Some(m) = self.session.mesh.as_ref() else {
            return;
        };
        ui.set_min_width(220.0);
        let names: Vec<String> = m.fields.names().iter().map(|n| n.to_string()).collect();
        let vector = m.fields.get(&choice.name).is_some_and(|f| f.is_vector());
        let each = |calls: &mut Vec<(&'static str, Value)>, extra: Value| {
            for &k in sets {
                let mut v = json!({ "stream": k + 1 });
                if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
                    o.extend(e.clone());
                }
                calls.push(("stream.set", v));
            }
        };
        let label = self.i18n.tr("tree-stream-color-by");
        egui::ComboBox::from_id_salt("stream-field")
            .selected_text(&choice.name)
            .show_ui(ui, |ui| {
                for n in &names {
                    if ui.selectable_label(*n == choice.name, n).clicked() && *n != choice.name {
                        each(calls, json!({ "color_field": n }));
                    }
                }
                if ui
                    .selectable_label(false, self.i18n.tr("tree-stream-solid"))
                    .clicked()
                {
                    each(calls, json!({ "color": "solid" }));
                }
            })
            .response
            .widget_info(|| {
                WidgetInfo::labeled(Role::ComboBox, true, format!("{label}: {}", choice.name))
            });
        if vector {
            ui.horizontal(|ui| {
                for c in Component::ALL {
                    let text = self.i18n.tr(&format!("field-component-{}", c.name()));
                    if ui.selectable_label(c == choice.component, &text).clicked()
                        && c != choice.component
                    {
                        each(calls, json!({ "color_field": choice.name, "component": c }));
                    }
                }
            });
        }
        ui.separator();
        let style = self.session.display.style_of(&choice.name);
        let range = sets.first().and_then(|&k| {
            murmel_engine::stream::color_range(&self.session.display, &m.mesh, Some(&m.fields), k)
        });
        let mut own: Vec<(&'static str, Value)> = Vec::new();
        self.style_menu(ui, &style, range, false, &mut own);
        for (tool, mut input) in own {
            if tool == "field.style" {
                input["field"] = json!(choice.name);
            }
            calls.push((tool, input));
        }
    }
}
