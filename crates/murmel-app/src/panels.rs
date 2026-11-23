//! The side panels (UI spec, "Model tree, properties panel and status
//! bar"): the model tree on the left with counts, colour swatches and eye
//! toggles, and the read-only properties of the selection on the right.
//! Both call tools for every change and read the session to draw.

use egui::{Align, Color32, Layout, RichText, Role, Sense, Ui, WidgetInfo};
use murmel_engine::display::all_items;
use murmel_engine::{ColorBy, Item, Validity};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::i18n::count;
use crate::style::{color, icon};

/// The property rows of one item, as `inspect.item` returned them, kept
/// until the surface or the selection changes.
pub struct PropsCache {
    pub key: (u64, Option<Item>),
    pub item: String,
    pub rows: Vec<Value>,
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
                Item::Mesh | Item::Cell(_) | Item::Face(_) => {}
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
            ui.label(RichText::new(icon::SEARCH).color(secondary));
            let hint = self.i18n.tr("tree-filter");
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.tree_filter)
                    .hint_text(hint)
                    .desired_width(f32::INFINITY),
            );
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
                if let Some(a) = self.clip_row(ui, &p) {
                    action = Some(a);
                }
            });
        if let Some((id, input)) = action {
            self.call(id, input);
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
                ] {
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
                if let Some(c) = self.color_action(ui, &id) {
                    action = Some(c);
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

    /// The clip plane's row under "Clip planes": the eye turns it on and
    /// off, the context menu flips it, changes the cut or removes it.
    fn clip_row(
        &self,
        ui: &mut Ui,
        p: &murmel_engine::theme::Palette,
    ) -> Option<(&'static str, Value)> {
        let c = self.session.display.clip?;
        let secondary = color(p.text_secondary);
        let mut action = None;
        egui::CollapsingHeader::new(
            RichText::new(self.i18n.tr("tree-clip-planes").to_uppercase())
                .small()
                .color(secondary),
        )
        .id_salt("tree-clip-planes")
        .default_open(true)
        .show(ui, |ui| {
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
                    action = Some(("clip.set", json!({"enabled": !c.enabled})));
                }
                let r = ui.selectable_label(false, &label);
                r.context_menu(|ui| {
                    for (key, call) in [
                        ("cmd-clip-flip", ("clip.set", json!({"flip": true}))),
                        ("cmd-clip-crinkle", ("clip.set", json!({"mode": "crinkle"}))),
                        ("cmd-clip-flat", ("clip.set", json!({"mode": "flat"}))),
                        ("cmd-clip-view", ("clip.set", json!({"axis": "view"}))),
                        ("cmd-clip-clear", ("clip.clear", json!({}))),
                    ] {
                        if ui.button(self.i18n.tr(key)).clicked() {
                            action = Some(call);
                            ui.close();
                        }
                    }
                });
            });
        });
        action
    }

    /// Fetch the rows of the selection through `inspect.item` when the
    /// surface or the selection changed.
    fn refresh_props(&mut self) {
        let key = (
            self.session.surface_revision,
            self.session.display.current(),
        );
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
                } else {
                    name.to_string()
                };
                format!("{} · {name}", self.i18n.tr(&format!("props-area-{area}")))
            }
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
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                // Label and value columns of fixed shares of the panel,
                // both cut with an ellipsis (full text on hover), so long
                // values never widen the panel.
                let width = ui.available_width();
                let label_w = (width * 0.48).round();
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
                        if resp.clicked() {
                            ui.ctx().copy_text(value.clone());
                        }
                    });
                    ui.add_space(2.0);
                }
            });
    }

    fn format_value(&self, key: &str, v: &Value) -> String {
        match v {
            Value::String(s) => match key {
                "validity" => self.i18n.tr(&format!("validity-{s}")),
                "role" => self.i18n.tr(&format!("role-{s}")),
                "type" => self.i18n.tr(&format!("kind-{s}")),
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

    /// The colour entry of an item's context menu: a picker that calls
    /// display.set_color, and a reset when the item has its own colour.
    pub(crate) fn color_action(&self, ui: &mut Ui, id: &str) -> Option<(&'static str, Value)> {
        let m = self.session.mesh.as_ref()?;
        let item = Item::parse(id, &m.mesh)?;
        let d = &self.session.display;
        let current = d
            .item_color(&m.mesh, &self.session.tokens, item)
            .or_else(|| d.colors.get(&item).copied())
            .unwrap_or(self.session.tokens.mesh.solid);
        let mut rgb = current.0;
        let mut action = None;
        ui.horizontal(|ui| {
            let r = egui::color_picker::color_edit_button_srgb(ui, &mut rgb);
            let name = self.i18n.tr("tree-color");
            r.widget_info(|| WidgetInfo::labeled(Role::ColorWell, true, name.clone()));
            ui.label(self.i18n.tr("tree-color"));
            if rgb != current.0 {
                let hex = murmel_engine::Rgb(rgb).hex();
                action = Some(("display.set_color", json!({"item": id, "color": hex})));
            }
        });
        if d.colors.contains_key(&item) && ui.button(self.i18n.tr("tree-reset-color")).clicked() {
            action = Some(("display.set_color", json!({"item": id, "color": null})));
            ui.close();
        }
        action
    }

    /// Legend of the colouring, top left under the view note.
    pub(crate) fn legend(&self, ui: &Ui, rect: egui::Rect, dark: bool) {
        let Some(m) = &self.session.mesh else {
            return;
        };
        let p = self.session.tokens.palette(dark);
        let t = &self.session.tokens;
        let mesh = &m.mesh;
        let d = &self.session.display;
        let name = |i: Item| -> String {
            match i {
                Item::Mesh => self.i18n.tr("props-area-mesh"),
                Item::Kind(k) => self.i18n.tr(&format!("kind-{}", k.name())),
                other => other
                    .id(mesh)
                    .split_once(':')
                    .map_or(String::new(), |(_, n)| n.to_string()),
            }
        };
        let entries: Vec<(murmel_engine::Rgb, String)> = match d.color_by {
            ColorBy::Solid => std::iter::once(Item::Mesh)
                .chain(murmel_engine::display::all_items(mesh))
                .filter(|&i| !d.is_hidden(i))
                .filter_map(|i| d.item_color(mesh, t, i).map(|c| (c, name(i))))
                .collect(),
            ColorBy::Role => murmel_mesh_roles(mesh)
                .into_iter()
                .map(|r| (t.role_color(r), self.i18n.tr(&format!("role-{}", r.name()))))
                .collect(),
            ColorBy::Patch | ColorBy::CellType => murmel_engine::display::all_items(mesh)
                .into_iter()
                .filter(|&i| !d.is_hidden(i))
                .filter_map(|i| d.item_color(mesh, t, i).map(|c| (c, name(i))))
                .collect(),
            // The quality legend (quality.rs) takes its place.
            ColorBy::Quality => Vec::new(),
        };
        if entries.is_empty() {
            return;
        }
        let painter = ui.painter_at(rect);
        let font = egui::FontId::proportional(12.0);
        let title = self
            .i18n
            .tr(&format!("color-by-{}", self.session.display.color_by.key()));
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

/// The roles the patches of `mesh` have, in the spec's order.
fn murmel_mesh_roles(mesh: &murmel_mesh::Mesh) -> Vec<murmel_mesh::PatchRole> {
    murmel_mesh::PatchRole::ALL
        .into_iter()
        .filter(|r| mesh.patches.iter().any(|p| p.role == *r))
        .collect()
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
