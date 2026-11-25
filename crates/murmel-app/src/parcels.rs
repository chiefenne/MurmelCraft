//! The spray parcels in the window (D268, first milestone): a row in the
//! model tree's "Slices and iso-surfaces" (the eye shows and hides them,
//! a click selects them, the context menu holds their settings), their
//! size factor under it, an "Open spray parcels…" button, and the
//! settings menu their colour bar's title opens. Every change calls a
//! tool (`parcels.open`, `parcels.set`, `parcels.clear`).

use egui::{Rect, RichText, Role, Sense, Ui, WidgetInfo};
use murmel_engine::Item;
use murmel_engine::field::RangeMode;
use murmel_engine::legend::{Body, Legend, Owner};
use murmel_engine::parcel::{DIAMETER, MAX_SCALE, Size};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::style::color;

/// A droplet diameter in the window: micrometres.
const MICRO: f64 = 1e-6;

impl MurmelApp {
    /// The label of the parcels: their file and its time.
    pub(crate) fn parcels_label(&self) -> String {
        let d = &self.session.display;
        let (Some(v), Some(p)) = (&d.parcels, d.parcel_data.as_deref()) else {
            return String::new();
        };
        let file = v
            .path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned());
        match p.time {
            Some(t) => self.i18n.tr_args(
                "tree-parcels-label-time",
                &[
                    ("file", file.into()),
                    ("time", crate::panels::number(t).into()),
                ],
            ),
            None => self
                .i18n
                .tr_args("tree-parcels-label", &[("file", file.into())]),
        }
    }

    /// Ask for a parcel file and show its parcels.
    pub(crate) fn parcels_open_dialog(&mut self) {
        let mut d = rfd::FileDialog::new()
            .set_title(self.i18n.tr("cmd-parcels-open"))
            .add_filter(self.i18n.tr("open-filter-vtkhdf"), &["vtkhdf", "hdf", "h5"]);
        // FoehnCraft writes them to the spray folder beside the result.
        if let Some(dir) = self
            .session
            .mesh
            .as_ref()
            .and_then(|m| m.path.parent())
            .map(|p| p.join("spray"))
            .filter(|p| p.is_dir())
        {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_file() {
            self.call("parcels.open", json!({ "path": p }));
        }
    }

    /// The parcels' settings: colour, size, filters, colour map, range.
    /// Shared by the tree row's context menu and the colour bar's title.
    pub(crate) fn parcel_menu(&self, ui: &mut Ui, calls: &mut Vec<(&'static str, Value)>) {
        let d = &self.session.display;
        let (Some(v), Some(p)) = (&d.parcels, d.parcel_data.as_deref()) else {
            return;
        };
        let set = |x: Value| ("parcels.set", x);
        ui.set_min_width(240.0);
        // Colour: solid, or a parcel field (a vector by its magnitude or
        // a component).
        ui.label(RichText::new(self.i18n.tr("parcels-color")).small());
        if ui
            .selectable_label(v.color.is_none(), self.i18n.tr("tree-stream-solid"))
            .clicked()
            && v.color.is_some()
        {
            calls.push(set(json!({"color": "solid"})));
        }
        if v.color.is_none() && ui.button(self.i18n.tr("tree-color-edit")).clicked() {
            calls.push((
                "window.color",
                json!({"item": "parcels", "label": self.parcels_label()}),
            ));
        }
        for f in &p.fields {
            let on = v.color.as_ref().is_some_and(|c| c.name == f.name);
            if f.is_vector() {
                ui.menu_button(&f.name, |ui| {
                    for c in murmel_mesh::Component::ALL {
                        let on = v
                            .color
                            .as_ref()
                            .is_some_and(|x| x.name == f.name && x.component == c);
                        let text = self.i18n.tr(&format!("field-component-{}", c.name()));
                        if ui.selectable_label(on, text).clicked() && !on {
                            calls.push(set(json!({"color": f.name, "component": c.name()})));
                        }
                    }
                });
            } else if ui.selectable_label(on, &f.name).clicked() && !on {
                calls.push(set(json!({"color": f.name})));
            }
        }
        ui.separator();
        // Size.
        ui.label(RichText::new(self.i18n.tr("parcels-size")).small());
        ui.horizontal(|ui| {
            for s in Size::ALL {
                let possible = s == Size::Fixed || p.field(DIAMETER).is_some();
                let text = self.i18n.tr(&format!("parcels-size-{}", s.key()));
                if ui
                    .add_enabled(possible, egui::Button::selectable(v.size == s, text))
                    .clicked()
                    && v.size != s
                {
                    calls.push(set(json!({"size": s.key()})));
                }
            }
        });
        // The diameter filter, in micrometres.
        if p.field(DIAMETER).is_some() {
            ui.separator();
            let mut on = v.diameter.is_some();
            let label = self.i18n.tr("parcels-diameter-filter");
            if ui.checkbox(&mut on, &label).changed() {
                calls.push(set(if on {
                    let r = p
                        .field(DIAMETER)
                        .and_then(|f| f.range(murmel_mesh::Component::Magnitude))
                        .unwrap_or((0.0, 1.0));
                    json!({"diameter": [r.0, r.1]})
                } else {
                    json!({"diameter": null})
                }));
            }
            if let Some([lo, hi]) = v.diameter {
                let (mut a, mut b) = (lo / MICRO, hi / MICRO);
                ui.horizontal(|ui| {
                    for (key, x) in [
                        ("parcels-diameter-min", &mut a),
                        ("parcels-diameter-max", &mut b),
                    ] {
                        let name = self.i18n.tr(key);
                        ui.label(&name);
                        let r = ui.add(
                            egui::DragValue::new(x)
                                .range(0.0..=1e6)
                                .speed(1.0)
                                .suffix(" µm"),
                        );
                        r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name.clone()));
                    }
                });
                if (a * MICRO, b * MICRO) != (lo, hi) && a <= b {
                    calls.push(set(json!({"diameter": [a * MICRO, b * MICRO]})));
                }
            }
        }
        if v.color.is_none() {
            return;
        }
        ui.separator();
        // The colour map, its direction and the range, kept with the
        // parcels.
        let label = self.i18n.tr("field-map");
        ui.label(RichText::new(&label).small());
        let current = match &v.map {
            Some(id) => self.i18n.tr(&format!("colormap-{id}")),
            None => self
                .i18n
                .tr_args("field-map-auto", &[("map", String::new().into())]),
        };
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("parcels-map")
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(v.map.is_none(), self.i18n.tr("parcels-map-auto"))
                        .clicked()
                    {
                        calls.push(set(json!({"map": "auto"})));
                    }
                    for id in murmel_engine::colormap::ids() {
                        let on = v.map.as_deref() == Some(id);
                        let text = self.i18n.tr(&format!("colormap-{id}"));
                        if ui.selectable_label(on, text).clicked() && !on {
                            calls.push(set(json!({"map": id})));
                        }
                    }
                })
                .response
                .widget_info(|| {
                    WidgetInfo::labeled(Role::ComboBox, true, format!("{label}: {current}"))
                });
            let mut rev = v.reverse;
            if ui
                .checkbox(&mut rev, self.i18n.tr("field-reverse"))
                .changed()
            {
                calls.push(set(json!({"reverse": rev})));
            }
        });
        let label = self.i18n.tr("field-range");
        ui.label(RichText::new(&label).small());
        let current = self.i18n.tr(&format!("field-range-{}", v.range.key()));
        egui::ComboBox::from_id_salt("parcels-range")
            .selected_text(&current)
            .show_ui(ui, |ui| {
                for k in ["field", "shown", "symmetric", "fixed"] {
                    let on = v.range.key() == k;
                    let text = if k == "field" {
                        self.i18n.tr("parcels-range-file")
                    } else {
                        self.i18n.tr(&format!("field-range-{k}"))
                    };
                    if ui.selectable_label(on, text).clicked() && !on {
                        calls.push(set(json!({"range": k})));
                    }
                }
            })
            .response
            .widget_info(|| {
                WidgetInfo::labeled(Role::ComboBox, true, format!("{label}: {current}"))
            });
        if let RangeMode::Fixed { min, max } = v.range {
            let (mut lo, mut hi) = (min, max);
            let speed = ((max - min).abs() / 200.0).max(f64::MIN_POSITIVE);
            ui.horizontal(|ui| {
                for (key, x) in [("field-range-min", &mut lo), ("field-range-max", &mut hi)] {
                    let name = self.i18n.tr(key);
                    ui.label(&name);
                    let r = ui.add(egui::DragValue::new(x).speed(speed));
                    r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name.clone()));
                }
            });
            if (lo, hi) != (min, max) && lo < hi {
                calls.push(set(json!({"min": lo, "max": hi})));
            }
        }
    }

    /// The parcels' row in the tree, and the button that opens them.
    pub(crate) fn parcel_rows(
        &self,
        ui: &mut Ui,
        p: &murmel_engine::theme::Palette,
    ) -> Option<(&'static str, Value)> {
        let m = self.session.mesh.as_ref()?;
        let d = &self.session.display;
        let tokens = &self.session.tokens;
        let secondary = color(p.text_secondary);
        let mut action = None;
        if let Some(v) = &d.parcels {
            let item = Item::Parcels;
            let id = item.id(&m.mesh);
            let label = self.parcels_label();
            ui.horizontal(|ui| {
                if self.eye(ui, v.enabled, &label, p).clicked() {
                    action = Some(("parcels.set", json!({"enabled": !v.enabled})));
                }
                // The swatch: the solid colour, or the ends of their bar.
                let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                let ends = Legend::own_bars(&self.session)
                    .into_iter()
                    .find(|(o, _)| *o == Owner::Parcels)
                    .and_then(|(_, l)| match l.body {
                        Body::Ramp(r) => Some((r.sample(0.15), r.sample(0.85))),
                        Body::Categories(_) => None,
                    });
                match (v.color.is_some(), ends) {
                    (true, Some((lo, hi))) => {
                        let half = Rect::from_min_max(rect.min, rect.center_bottom());
                        ui.painter().rect_filled(half, 0.0, color(lo));
                        let rest = Rect::from_min_max(rect.center_top(), rect.max);
                        ui.painter().rect_filled(rest, 0.0, color(hi));
                    }
                    _ => {
                        let rgb = murmel_engine::parcel::solid_color(d, tokens);
                        ui.painter().rect_filled(rect, 2.0, color(rgb));
                    }
                }
                let selected = d.is_selected(item);
                let text = if v.enabled {
                    RichText::new(&label)
                } else {
                    RichText::new(&label).color(secondary)
                };
                let r = ui.selectable_label(selected, text);
                r.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, label.clone()));
                let r = r.on_hover_text(self.i18n.tr("tree-parcels-tip"));
                if r.clicked() {
                    action = Some(("select.item", json!({ "item": id })));
                }
                r.context_menu(|ui| {
                    let mut calls = Vec::new();
                    self.parcel_menu(ui, &mut calls);
                    ui.separator();
                    if let Some(a) = self.default_entries(ui, item, &[]) {
                        calls.push(a);
                    }
                    if ui.button(self.i18n.tr("tree-parcels-other")).clicked() {
                        calls.push(("window.parcels_open", json!({})));
                    }
                    if ui.button(self.i18n.tr("cmd-parcels-clear")).clicked() {
                        calls.push(("parcels.clear", json!({})));
                    }
                    if let Some(c) = calls.pop() {
                        action = Some(c);
                        ui.close();
                    }
                });
            });
            // The size factor, and how many show.
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                let name = self.i18n.tr("tree-parcels-scale");
                ui.label(RichText::new(&name).small());
                let mut x = v.scale;
                let r = ui
                    .add(
                        egui::DragValue::new(&mut x)
                            .range(0.01..=MAX_SCALE)
                            .speed(0.02)
                            .max_decimals(2),
                    )
                    .on_hover_text(self.i18n.tr("tree-parcels-scale-tip"));
                r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name.clone()));
                r.context_menu(|ui| {
                    if let Some(a) = self.default_entries(ui, Item::Parcels, &["scale"]) {
                        action = Some(a);
                    }
                });
                if x != v.scale && x > 0.0 {
                    action = Some(("parcels.set", json!({ "scale": x })));
                }
                let count = self.i18n.tr_args(
                    if m.surface.parcels_capped {
                        "tree-parcels-count-capped"
                    } else {
                        "tree-parcels-count"
                    },
                    &[
                        ("drawn", m.surface.parcels.into()),
                        (
                            "total",
                            d.parcel_data.as_ref().map_or(0, |p| p.len()).into(),
                        ),
                    ],
                );
                ui.label(RichText::new(count).small().color(secondary));
            });
        }
        if d.parcels.is_none() {
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                let b = ui
                    .add(egui::Button::new(
                        RichText::new(self.i18n.tr("cmd-parcels-open")).small(),
                    ))
                    .on_hover_text(self.i18n.tr("tree-parcels-open-tip"));
                if b.clicked() {
                    action = Some(("window.parcels_open", json!({})));
                }
            });
        }
        action
    }
}
