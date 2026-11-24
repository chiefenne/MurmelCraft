//! The side panels (UI spec, "Model tree, properties panel and status
//! bar"): the model tree on the left with counts, colour swatches and eye
//! toggles, and the read-only properties of the selection on the right.
//! Both call tools for every change and read the session to draw.

use egui::{Align, Color32, Layout, RichText, Role, Sense, Ui, WidgetInfo};
use murmel_engine::display::all_items;
use murmel_engine::{Item, Validity};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::i18n::count;
use crate::style::{color, icon};

/// The property rows of one item, as `inspect.item` returned them, kept
/// until the surface or the selection changes.
pub struct PropsCache {
    /// The surface revision, the item shown and, for a measurement or a
    /// probe, its state (they change without a new surface).
    pub key: (u64, Option<Item>, String),
    pub item: String,
    pub rows: Vec<Value>,
    /// The field values of the item (a cell, a face, a probe).
    pub fields: Vec<Value>,
}

/// A tree row: what it names, its label, count and swatch.
struct Row {
    item: Item,
    label: String,
    count: usize,
    swatch: Option<murmel_engine::Rgb>,
    tip: Option<String>,
}

impl MurmelApp {
    fn rows(&self) -> Vec<(&'static str, Vec<Row>)> {
        let Some(m) = &self.session.mesh else {
            return Vec::new();
        };
        let mesh = &m.mesh;
        let summary = mesh.summary();
        let t = &self.session.tokens;
        let d = &self.session.display;
        let mut zones = Vec::new();
        let mut kinds = Vec::new();
        let mut patches = Vec::new();
        for item in all_items(mesh) {
            match item {
                Item::Zone(z) => zones.push(Row {
                    item,
                    label: mesh.zones[z as usize].clone(),
                    count: summary.zones.get(z as usize).map_or(0, |n| n.cells),
                    swatch: d.item_color(mesh, t, item),
                    tip: None,
                }),
                Item::Kind(k) => kinds.push(Row {
                    item,
                    label: self.i18n.tr(&format!("kind-{}", k.name())),
                    count: summary.cell_kinds.get(k.name()).copied().unwrap_or(0),
                    swatch: d.item_color(mesh, t, item),
                    tip: None,
                }),
                Item::Patch(p) => {
                    let patch = &mesh.patches[p as usize];
                    patches.push(Row {
                        item,
                        label: patch.name.clone(),
                        count: summary.patches.get(p as usize).map_or(0, |n| n.faces),
                        swatch: d.item_color(mesh, t, item),
                        tip: Some(self.i18n.tr_args(
                            "tree-patch-tip",
                            &[(
                                "role",
                                self.i18n.tr(&format!("role-{}", patch.role.name())).into(),
                            )],
                        )),
                    })
                }
                Item::Mesh
                | Item::Cell(_)
                | Item::Face(_)
                | Item::Clip(_)
                | Item::Measure(_)
                | Item::Probe(_)
                | Item::Slice(_)
                | Item::Iso(_)
                | Item::Arrows
                | Item::Stream(_)
                | Item::Region(_) => {}
            }
        }
        patches.sort_by(|a, b| a.label.cmp(&b.label));
        vec![
            ("tree-zones", zones),
            ("tree-cell-types", kinds),
            ("tree-patches", patches),
        ]
    }

    pub(crate) fn model_tree(&mut self, ui: &mut Ui) {
        let p = self.palette(ui.ctx()).clone();
        let secondary = color(p.text_secondary);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            // The search glyph is painted, not a label: it would be read
            // as a private-use character.
            let (g, _) = ui.allocate_exact_size(egui::vec2(14.0, 20.0), Sense::hover());
            ui.painter().text(
                g.center(),
                egui::Align2::CENTER_CENTER,
                icon::SEARCH,
                egui::FontId::proportional(13.0),
                secondary,
            );
            let hint = self.i18n.tr("tree-filter");
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.tree_filter)
                    .hint_text(hint)
                    .desired_width(f32::INFINITY),
            );
            crate::focus::set_name(ui, r.id, &self.i18n.tr("cmd-filter"));
            self.focus.offer(crate::focus::Region::Tree, &r);
            if std::mem::take(&mut self.focus_filter) {
                r.request_focus();
            }
        });
        ui.add_space(4.0);
        let Some(m) = &self.session.mesh else {
            ui.label(
                RichText::new(self.i18n.tr("tree-empty"))
                    .small()
                    .color(secondary),
            );
            return;
        };
        let file = m
            .path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned());
        let (badge, badge_key) = match &m.validity {
            Validity::Valid => (p.status_ok, "validity-tip-valid"),
            Validity::NoEvidence => (p.status_warn, "validity-tip-no_evidence"),
            Validity::Failed(_) => (p.status_error, "validity-tip-failed"),
            Validity::Checking => (p.text_secondary, "validity-tip-checking"),
        };
        let mut badge_tip = self.i18n.tr(badge_key);
        if let Validity::Failed(why) = &m.validity {
            badge_tip = format!("{badge_tip}\n{}", why.join("\n"));
        }
        let selection = self.session.display.selection.clone();
        let mut action: Option<(&'static str, Value)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.5, color(badge));
                    let tip = badge_tip.clone();
                    resp.widget_info(|| WidgetInfo::labeled(Role::Image, true, tip.clone()));
                    resp.on_hover_text(badge_tip.clone());
                    let root = ui.selectable_label(
                        selection.is_empty() || selection == [Item::Mesh],
                        RichText::new(&file)
                            .family(egui::FontFamily::Name(crate::style::SEMIBOLD.into())),
                    );
                    if root.clicked() {
                        action = Some(("select.clear", json!({})));
                    }
                    if root.double_clicked() {
                        action = Some(("view.fit", json!({})));
                    }
                });
                let filter = self.tree_filter.to_lowercase();
                for (section, rows) in self.rows() {
                    let rows: Vec<&Row> = rows
                        .iter()
                        .filter(|r| filter.is_empty() || r.label.to_lowercase().contains(&filter))
                        .collect();
                    if rows.is_empty() {
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
                        for r in rows {
                            if let Some(a) = self.tree_row(ui, r, &p) {
                                action = Some(a);
                            }
                        }
                    });
                }
                if let Some(a) = self.views_section(ui, &p, &filter) {
                    action = Some(a);
                }
                if let Some(a) = self.clip_row(ui, &p) {
                    action = Some(a);
                }
                if let Some(a) = self.extract_section(ui, &p) {
                    action = Some(a);
                }
                if let Some(a) = self.region_section(ui, &p) {
                    action = Some(a);
                }
                if let Some(a) = self.measure_section(ui, &p, &filter) {
                    action = Some(a);
                }
            });
        match action {
            Some(("window.color", input)) => {
                let field = |k: &str| input[k].as_str().unwrap_or_default().to_string();
                self.open_color_edit(&field("item"), &field("label"));
            }
            Some((id, input)) => {
                self.call(id, input);
            }
            None => {}
        }
    }

    fn tree_row(
        &self,
        ui: &mut Ui,
        r: &Row,
        p: &murmel_engine::theme::Palette,
    ) -> Option<(&'static str, Value)> {
        let mesh = &self.session.mesh.as_ref()?.mesh;
        let id = r.item.id(mesh);
        let hidden = self.session.display.is_hidden(r.item);
        let selected = self.session.display.is_selected(r.item);
        let mut action = None;
        ui.horizontal(|ui| {
            let eye = if hidden { icon::EYE_OFF } else { icon::EYE };
            let name = self.i18n.tr_args(
                if hidden { "tree-show" } else { "tree-hide" },
                &[("item", r.label.clone().into())],
            );
            let b = ui
                .add(
                    egui::Button::new(RichText::new(eye).size(14.0).color(if hidden {
                        color(p.text_secondary)
                    } else {
                        color(p.text_primary)
                    }))
                    .frame(false)
                    .min_size(egui::vec2(20.0, 20.0)),
                )
                .on_hover_text(&name);
            b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
            if b.clicked() {
                let verb = if hidden { "show" } else { "hide" };
                action = Some(("display.visibility", json!({"action": verb, "items": [id]})));
            }
            if let Some(c) = r.swatch {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                ui.painter().rect_filled(rect, 2.0, color(c));
            }
            let text = if hidden {
                RichText::new(&r.label).color(color(p.text_secondary))
            } else {
                RichText::new(&r.label)
            };
            let label = ui.selectable_label(selected, text);
            let row = self.i18n.tr_args(
                if matches!(r.item, Item::Patch(_)) {
                    "tree-row-faces"
                } else {
                    "tree-row-cells"
                },
                &[
                    ("item", r.label.clone().into()),
                    ("count", count(r.count).into()),
                ],
            );
            let row = if hidden {
                self.i18n.tr_args("tree-row-hidden", &[("row", row.into())])
            } else {
                row
            };
            label.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, row.clone()));
            let label = match &r.tip {
                Some(t) => label.on_hover_text(t),
                None => label,
            };
            if label.clicked() {
                action = Some(("select.item", json!({"item": id})));
            }
            if label.double_clicked() {
                action = Some(("view.fit", json!({"item": id})));
            }
            label.context_menu(|ui| {
                for (key, call) in [
                    (
                        "tree-isolate",
                        (
                            "display.visibility",
                            json!({"action": "isolate", "items": [id]}),
                        ),
                    ),
                    (
                        "tree-show-all",
                        ("display.visibility", json!({"action": "show_all"})),
                    ),
                    ("tree-fit", ("view.fit", json!({"item": id}))),
                ]
                .into_iter()
                .chain(matches!(r.item, Item::Zone(_)).then(|| {
                    (
                        "tree-color-by-zone",
                        ("display.color_by", json!({"by": "zone"})),
                    )
                })) {
                    if ui.button(self.i18n.tr(key)).clicked() {
                        action = Some(call);
                        ui.close();
                    }
                }
                if ui.button(self.i18n.tr("tree-copy-name")).clicked() {
                    ui.ctx().copy_text(r.label.clone());
                    ui.close();
                }
                ui.separator();
                // The colour opens a window of its own: a picker inside
                // the menu closed with it (docs/pitfalls.md).
                if ui.button(self.i18n.tr("tree-color-edit")).clicked() {
                    action = Some(("window.color", json!({"item": id, "label": r.label})));
                    ui.close();
                }
                let own = self.session.mesh.as_ref().is_some_and(|m| {
                    Item::parse(&id, &m.mesh)
                        .is_some_and(|i| self.session.display.colors.contains_key(&i))
                });
                if own && ui.button(self.i18n.tr("tree-reset-color")).clicked() {
                    action = Some(("display.set_color", json!({"item": id, "color": null})));
                    ui.close();
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(count(r.count))
                        .monospace()
                        .color(color(p.text_secondary)),
                );
            });
        });
        action
    }

    /// Save the camera as a new view, named "View N" in the person's
    /// language (Ctrl + B, the tree's button).
    pub(crate) fn save_view(&mut self) {
        if self.session.mesh.is_none() {
            return;
        }
        let name = self
            .session
            .display
            .next_view_name(&self.i18n.tr("view-name-base"));
        self.call("views.save", json!({ "name": name }));
    }

    /// "Saved views": a row per named camera (click goes there,
    /// double-click renames, the context menu updates, renames and
    /// deletes) and a button that saves the current one.
    fn views_section(
        &mut self,
        ui: &mut Ui,
        p: &murmel_engine::theme::Palette,
        filter: &str,
    ) -> Option<(&'static str, Value)> {
        let secondary = color(p.text_secondary);
        let mut action = None;
        let mut save = false;
        let views: Vec<(String, bool)> = self
            .session
            .display
            .views
            .iter()
            .filter(|v| filter.is_empty() || v.name.to_lowercase().contains(filter))
            .map(|v| (v.name.clone(), v.camera == self.session.camera))
            .collect();
        egui::CollapsingHeader::new(
            RichText::new(self.i18n.tr("tree-saved-views").to_uppercase())
                .small()
                .color(secondary),
        )
        .id_salt("tree-saved-views")
        .default_open(true)
        .show(ui, |ui| {
            for (name, here) in &views {
                let editing = self.view_edit.as_ref().is_some_and(|(n, _)| n == name);
                if editing {
                    if let Some(a) = self.view_name_edit(ui) {
                        action = Some(a);
                    }
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    let r = ui.selectable_label(*here, name);
                    let row = self
                        .i18n
                        .tr_args("tree-view-row", &[("item", name.clone().into())]);
                    r.widget_info(|| WidgetInfo::selected(Role::Button, true, *here, row.clone()));
                    let r = r.on_hover_text(self.i18n.tr("tree-view-tip"));
                    if r.clicked() {
                        action = Some(("views.go", json!({ "name": name })));
                    }
                    if r.double_clicked() {
                        self.view_edit = Some((name.clone(), name.clone()));
                    }
                    r.context_menu(|ui| {
                        for (key, call) in [
                            ("tree-view-go", ("views.go", json!({ "name": name }))),
                            ("tree-view-update", ("views.save", json!({ "name": name }))),
                            (
                                "tree-view-delete",
                                ("views.delete", json!({ "name": name })),
                            ),
                        ] {
                            if ui.button(self.i18n.tr(key)).clicked() {
                                action = Some(call);
                                ui.close();
                            }
                        }
                        if ui.button(self.i18n.tr("tree-view-rename")).clicked() {
                            self.view_edit = Some((name.clone(), name.clone()));
                            ui.close();
                        }
                    });
                });
            }
            if views.is_empty() && !filter.is_empty() {
                return;
            }
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                let b = ui
                    .add(egui::Button::new(
                        RichText::new(self.i18n.tr("tree-view-new")).small(),
                    ))
                    .on_hover_text(self.i18n.tr("tree-view-new-tip"));
                if b.clicked() {
                    save = true;
                }
            });
        });
        if save {
            self.save_view();
        }
        action
    }

    /// The text field of a saved view being renamed: Enter or leaving it
    /// renames, Escape keeps the old name.
    fn view_name_edit(&mut self, ui: &mut Ui) -> Option<(&'static str, Value)> {
        let (old, mut text) = self.view_edit.clone()?;
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
                .tr_args("tree-view-rename-field", &[("item", old.clone().into())]);
            crate::focus::set_name(ui, r.id, &name);
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if escape {
                self.view_edit = None;
            } else if r.lost_focus() {
                self.view_edit = None;
                if text.trim() != old {
                    action = Some(("views.rename", json!({ "name": old, "to": text })));
                }
            } else {
                self.view_edit = Some((old.clone(), text.clone()));
            }
        });
        action
    }

    /// "Clip planes": a row per plane (the eye turns it on and off, a
    /// click selects it, so the toolbar and the handle act on it; the
    /// context menu flips it, changes the cut, fits it or removes it) and
    /// a button that adds one.
    fn clip_row(
        &self,
        ui: &mut Ui,
        p: &murmel_engine::theme::Palette,
    ) -> Option<(&'static str, Value)> {
        let mesh = &self.session.mesh.as_ref()?.mesh;
        let d = &self.session.display;
        let secondary = color(p.text_secondary);
        let current = d.current_clip();
        let mut action = None;
        egui::CollapsingHeader::new(
            RichText::new(self.i18n.tr("tree-clip-planes").to_uppercase())
                .small()
                .color(secondary),
        )
        .id_salt("tree-clip-planes")
        .default_open(true)
        .show(ui, |ui| {
            for (k, c) in d.clips.iter().enumerate() {
                let item = Item::Clip(k as u32);
                let id = item.id(mesh);
                let plane = k + 1;
                ui.horizontal(|ui| {
                    let axis = ["x", "y", "z"]
                        .into_iter()
                        .zip(c.normal)
                        .find(|(_, n)| n.abs() > 0.999)
                        .map(|(a, n)| {
                            format!("{}{}", if n < 0.0 { "−" } else { "" }, a.to_uppercase())
                        });
                    let label = self.i18n.tr_args(
                        "tree-clip-label",
                        &[
                            ("n", plane.into()),
                            (
                                "normal",
                                axis.unwrap_or_else(|| self.i18n.tr("tree-clip-oblique"))
                                    .into(),
                            ),
                            (
                                "mode",
                                self.i18n.tr(&format!("clip-mode-{}", c.mode.key())).into(),
                            ),
                        ],
                    );
                    let eye = if c.enabled { icon::EYE } else { icon::EYE_OFF };
                    let name = self.i18n.tr_args(
                        if c.enabled { "tree-hide" } else { "tree-show" },
                        &[("item", label.clone().into())],
                    );
                    let b = ui
                        .add(
                            egui::Button::new(RichText::new(eye).size(14.0).color(if c.enabled {
                                color(p.text_primary)
                            } else {
                                secondary
                            }))
                            .frame(false)
                            .min_size(egui::vec2(20.0, 20.0)),
                        )
                        .on_hover_text(&name);
                    b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
                    if b.clicked() {
                        action = Some(("clip.set", json!({"plane": plane, "enabled": !c.enabled})));
                    }
                    let selected = d.is_selected(item);
                    let text = if c.enabled {
                        RichText::new(&label)
                    } else {
                        RichText::new(&label).color(secondary)
                    };
                    let r = ui.selectable_label(selected, text);
                    let row = if current == Some(k) {
                        self.i18n
                            .tr_args("tree-clip-current", &[("row", label.clone().into())])
                    } else {
                        label.clone()
                    };
                    r.widget_info(|| {
                        WidgetInfo::selected(Role::Button, true, selected, row.clone())
                    });
                    let r = r.on_hover_text(self.i18n.tr("tree-clip-tip"));
                    if r.clicked() {
                        action = Some(("select.item", json!({ "item": id })));
                    }
                    if r.double_clicked() {
                        action = Some(("view.fit", json!({ "item": id })));
                    }
                    r.context_menu(|ui| {
                        for (key, call) in [
                            (
                                "cmd-clip-flip",
                                ("clip.set", json!({"plane": plane, "flip": true})),
                            ),
                            (
                                "cmd-clip-crinkle",
                                ("clip.set", json!({"plane": plane, "mode": "crinkle"})),
                            ),
                            (
                                "cmd-clip-flat",
                                ("clip.set", json!({"plane": plane, "mode": "flat"})),
                            ),
                            (
                                "cmd-clip-view",
                                ("clip.set", json!({"plane": plane, "axis": "view"})),
                            ),
                            ("tree-fit", ("view.fit", json!({ "item": id }))),
                            (
                                "tree-clip-remove",
                                ("clip.clear", json!({ "plane": plane })),
                            ),
                        ] {
                            if ui.button(self.i18n.tr(key)).clicked() {
                                action = Some(call);
                                ui.close();
                            }
                        }
                    });
                    if current == Some(k) && d.clips.len() > 1 {
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
            if d.clips.len() < murmel_tools::MAX_CLIPS {
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    let b = ui
                        .add(egui::Button::new(
                            RichText::new(self.i18n.tr("tree-clip-new")).small(),
                        ))
                        .on_hover_text(self.i18n.tr("cmd-clip-new"));
                    if b.clicked() {
                        action = Some(("clip.set", json!({"new": true, "axis": "x"})));
                    }
                });
            }
        });
        action
    }

    /// Fetch the rows of the selection through `inspect.item` when the
    /// surface or the selection changed.
    fn refresh_props(&mut self) {
        let d = &self.session.display;
        let current = d.current();
        let state = match current {
            Some(Item::Measure(n)) => format!("{:?}", d.measurement(n)),
            Some(Item::Probe(n)) => format!("{:?}", d.probe(n)),
            _ => String::new(),
        };
        let key = (self.session.surface_revision, current, state);
        if self.session.mesh.is_none() {
            self.props = None;
            return;
        }
        if self.props.as_ref().is_some_and(|c| c.key == key) {
            return;
        }
        let e = murmel_tools::call("inspect.item", json!({}), &mut self.session);
        self.props = e.ok().then(|| PropsCache {
            key,
            item: e.result["item"].as_str().unwrap_or("mesh").to_string(),
            rows: e.result["rows"].as_array().cloned().unwrap_or_default(),
            fields: e.result["fields"].as_array().cloned().unwrap_or_default(),
        });
    }

    pub(crate) fn properties(&mut self, ui: &mut Ui) {
        self.refresh_props();
        let p = self.palette(ui.ctx()).clone();
        let secondary = color(p.text_secondary);
        ui.add_space(8.0);
        let Some(c) = &self.props else {
            ui.heading(self.i18n.tr("props-title"));
            ui.label(
                RichText::new(self.i18n.tr("props-empty"))
                    .small()
                    .color(secondary),
            );
            return;
        };
        let heading = match c.item.split_once(':') {
            Some((area, name)) => {
                let name = if area == "kind" {
                    self.i18n.tr(&format!("kind-{name}"))
                } else if matches!(area, "clip" | "slice" | "iso" | "region" | "stream") {
                    self.i18n
                        .tr_args("props-clip-name", &[("n", name.to_string().into())])
                } else if area == "measure" || area == "probe" {
                    self.session
                        .mesh
                        .as_ref()
                        .and_then(|m| Item::parse(&c.item, &m.mesh))
                        .map_or_else(|| name.to_string(), |i| self.measure_name(i))
                } else {
                    name.to_string()
                };
                format!("{} · {name}", self.i18n.tr(&format!("props-area-{area}")))
            }
            None if c.item == "arrows" => self.i18n.tr("props-area-arrows"),
            None => self.i18n.tr("props-area-mesh"),
        };
        ui.heading(heading);
        let n = self.session.display.selection.len();
        if n > 1 {
            ui.label(
                RichText::new(
                    self.i18n
                        .tr_args("props-more-selected", &[("count", (n - 1).into())]),
                )
                .small()
                .color(secondary),
            );
        }
        ui.add_space(4.0);
        let out =
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    // Label and value columns of fixed shares of the panel,
                    // both cut with an ellipsis (full text on hover), so long
                    // values never widen the panel.
                    let width = ui.available_width();
                    let label_w = (width * 0.48).round();
                    let mut first: Option<egui::Response> = None;
                    for r in &c.rows {
                        let key = r["key"].as_str().unwrap_or("");
                        let mut label = self.i18n.tr(&format!("prop-{key}"));
                        if let Some(u) = r["unit"].as_str() {
                            label = format!("{label} ({u})");
                        }
                        let value = self.format_value(key, &r["value"]);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.0;
                            ui.allocate_ui_with_layout(
                                egui::vec2(label_w - 12.0, 20.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.set_width(label_w - 12.0);
                                    ui.add(
                                        egui::Label::new(RichText::new(&label).color(secondary))
                                            .truncate(),
                                    )
                                    .on_hover_text(&label);
                                },
                            );
                            let resp = ui
                                .add(
                                    egui::Label::new(RichText::new(&value).monospace())
                                        .truncate()
                                        .sense(Sense::click()),
                                )
                                .on_hover_text(
                                    self.i18n
                                        .tr_args("props-copy", &[("value", value.clone().into())]),
                                );
                            // Read as "Cells: 776", and the first row is where
                            // F6 lands.
                            let name = self.i18n.tr_args(
                                "prop-row-name",
                                &[
                                    ("label", label.clone().into()),
                                    ("value", value.clone().into()),
                                ],
                            );
                            crate::focus::set_name(ui, resp.id, &name);
                            first.get_or_insert(resp.clone());
                            if resp.clicked() {
                                ui.ctx().copy_text(value.clone());
                            }
                        });
                        ui.add_space(2.0);
                    }
                    // The field values: a scalar's value, a vector's
                    // components and its length.
                    if !c.fields.is_empty() {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(self.i18n.tr("props-fields"))
                                .small()
                                .strong()
                                .color(secondary),
                        );
                        for f in &c.fields {
                            let name = f["name"].as_str().unwrap_or("");
                            let unit = f["unit"].as_str();
                            let vals: Vec<f64> = f["value"]
                                .as_array()
                                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                                .unwrap_or_default();
                            let mut lines = Vec::new();
                            let with_unit = |l: String| match unit {
                                Some(u) => format!("{l} ({u})"),
                                None => l,
                            };
                            if vals.len() == 1 {
                                lines.push((with_unit(name.to_string()), number(vals[0])));
                            } else {
                                let parts: Vec<String> = vals.iter().map(|&x| number(x)).collect();
                                lines.push((with_unit(name.to_string()), parts.join(", ")));
                                let mag = vals.iter().map(|x| x * x).sum::<f64>().sqrt();
                                lines.push((with_unit(format!("|{name}|")), number(mag)));
                            }
                            for (label, value) in lines {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 12.0;
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(label_w - 12.0, 20.0),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.set_width(label_w - 12.0);
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&label).color(secondary),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(&label);
                                        },
                                    );
                                    let resp = ui
                                        .add(
                                            egui::Label::new(RichText::new(&value).monospace())
                                                .truncate()
                                                .sense(Sense::click()),
                                        )
                                        .on_hover_text(self.i18n.tr_args(
                                            "props-copy",
                                            &[("value", value.clone().into())],
                                        ));
                                    let name = self.i18n.tr_args(
                                        "prop-row-name",
                                        &[
                                            ("label", label.clone().into()),
                                            ("value", value.clone().into()),
                                        ],
                                    );
                                    crate::focus::set_name(ui, resp.id, &name);
                                    if resp.clicked() {
                                        ui.ctx().copy_text(value.clone());
                                    }
                                });
                                ui.add_space(2.0);
                            }
                        }
                    }
                    first
                });
        if let Some(r) = out.inner {
            self.focus.offer(crate::focus::Region::Properties, &r);
        }
    }

    fn format_value(&self, key: &str, v: &Value) -> String {
        match v {
            Value::String(s) => match key {
                "validity" => self.i18n.tr(&format!("validity-{s}")),
                "role" => self.i18n.tr(&format!("role-{s}")),
                "mode" | "shown_as" => self.i18n.tr(&format!("clip-mode-{s}")),
                "shape" => self.i18n.tr(&format!("region-shape-{s}")),
                "type" => self.i18n.tr(&format!("kind-{s}")),
                "kind" => self.i18n.tr(&format!("measure-kind-{s}")),
                "color" => self.i18n.tr(&format!("iso-color-{s}")),
                "seed" => self.i18n.tr(&format!("stream-seed-kind-{s}")),
                "direction" => self.i18n.tr(&format!("stream-direction-{s}")),
                "look" => self.i18n.tr(&format!("stream-look-{s}")),
                "snap" | "snap_a" | "snap_b" | "snap_vertex" => self.i18n.tr(&format!("snap-{s}")),
                "name"
                    if self
                        .props
                        .as_ref()
                        .is_some_and(|c| c.item.starts_with("kind:")) =>
                {
                    self.i18n.tr(&format!("kind-{s}"))
                }
                _ => s.clone(),
            },
            Value::Bool(b) => self.i18n.tr(if *b { "value-yes" } else { "value-no" }),
            Value::Number(n) => match n.as_u64() {
                Some(u) => count(u as usize),
                None => n.as_f64().map_or(n.to_string(), number),
            },
            Value::Array(a) => {
                let parts: Vec<String> = a
                    .iter()
                    .map(|x| x.as_f64().map_or(x.to_string(), number))
                    .collect();
                format!("({})", parts.join(", "))
            }
            Value::Null => "–".into(),
            Value::Object(_) => v.to_string(),
        }
    }

    /// Legend of the colouring, top left under the view note: the swatches
    /// of `murmel_engine::Legend` (exported pictures draw the same model).
    pub(crate) fn legend(&self, ui: &Ui, rect: egui::Rect, dark: bool) {
        let Some(legend) = murmel_engine::Legend::of(&self.session) else {
            return;
        };
        // A ramp (quality) has its own legend with the histogram
        // (quality.rs).
        let murmel_engine::legend::Body::Categories(swatches) = &legend.body else {
            return;
        };
        let p = self.session.tokens.palette(dark);
        let entries: Vec<(murmel_engine::Rgb, String)> = swatches
            .iter()
            .map(|e| (e.color, e.label.text(&self.i18n)))
            .collect();
        if entries.is_empty() {
            return;
        }
        let painter = ui.painter_at(rect);
        let font = egui::FontId::proportional(12.0);
        let title = legend.title.text(&self.i18n);
        let mut galleys =
            vec![painter.layout_no_wrap(title, font.clone(), color(p.text_secondary))];
        for (_, name) in &entries {
            galleys.push(painter.layout_no_wrap(name.clone(), font.clone(), color(p.text_primary)));
        }
        let w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 18.0;
        let line = 18.0;
        let at = rect.min + egui::vec2(16.0, 52.0 + self.cube_room().0);
        let bg =
            egui::Rect::from_min_size(at, egui::vec2(w + 20.0, line * galleys.len() as f32 + 10.0));
        painter.rect_filled(bg, 6.0, color(p.bg_panel).gamma_multiply(0.92));
        let mut y = at.y + 5.0;
        for (i, g) in galleys.into_iter().enumerate() {
            let x = at.x + 10.0;
            if i == 0 {
                painter.galley(egui::pos2(x, y), g, Color32::WHITE);
            } else {
                let sw = egui::Rect::from_min_size(egui::pos2(x, y + 3.0), egui::vec2(10.0, 10.0));
                painter.rect_filled(sw, 2.0, color(entries[i - 1].0));
                painter.galley(egui::pos2(x + 18.0, y), g, Color32::WHITE);
            }
            y += line;
        }
    }
}

/// A number with up to six significant digits, without trailing zeros.
pub(crate) fn number(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return format!("{x}");
    }
    let a = x.abs();
    if !(1e-4..1e7).contains(&a) {
        return format!("{x:.4e}");
    }
    let digits = (5 - a.log10().floor() as i32).clamp(0, 9) as usize;
    let s = format!("{x:.digits$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    s.replace('-', "\u{2212}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn numbers_are_short() {
        assert_eq!(super::number(8.000000000000023), "8");
        assert_eq!(super::number(1.23456789), "1.23457");
        assert_eq!(super::number(-1.0), "\u{2212}1");
        assert_eq!(super::number(0.000012), "1.2000e-5");
    }
}
