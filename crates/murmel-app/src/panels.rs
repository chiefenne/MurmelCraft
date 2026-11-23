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
        let by = self.session.display.color_by;
        let mut zones = Vec::new();
        let mut kinds = Vec::new();
        let mut patches = Vec::new();
        for item in all_items(mesh) {
            match item {
                Item::Zone(z) => zones.push(Row {
                    item,
                    label: mesh.zones[z as usize].clone(),
                    count: summary.zones.get(z as usize).map_or(0, |n| n.cells),
                    swatch: None,
                    tip: None,
                }),
                Item::Kind(k) => kinds.push(Row {
                    item,
                    label: self.i18n.tr(&format!("kind-{}", k.name())),
                    count: summary.cell_kinds.get(k.name()).copied().unwrap_or(0),
                    swatch: (by == ColorBy::CellType).then(|| t.kind_color(k)),
                    tip: None,
                }),
                Item::Patch(p) => {
                    let patch = &mesh.patches[p as usize];
                    patches.push(Row {
                        item,
                        label: patch.name.clone(),
                        count: summary.patches.get(p as usize).map_or(0, |n| n.faces),
                        swatch: match by {
                            ColorBy::Patch => Some(t.patch_color(p as usize)),
                            ColorBy::Role => Some(t.role_color(patch.role)),
                            ColorBy::CellType => None,
                        },
                        tip: Some(self.i18n.tr_args(
                            "tree-patch-tip",
                            &[(
                                "role",
                                self.i18n.tr(&format!("role-{}", patch.role.name())).into(),
                            )],
                        )),
                    })
                }
                Item::Mesh => {}
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
        let selection = self.session.display.selection;
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
                        selection.is_none() || selection == Some(Item::Mesh),
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
        let selected = self.session.display.selection == Some(r.item);
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

    /// Fetch the rows of the selection through `inspect.item` when the
    /// surface or the selection changed.
    fn refresh_props(&mut self) {
        let key = (
            self.session.surface_revision,
            self.session.display.selection,
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
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                egui::Grid::new("props")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .striped(false)
                    .show(ui, |ui| {
                        for r in &c.rows {
                            let key = r["key"].as_str().unwrap_or("");
                            let mut label = self.i18n.tr(&format!("prop-{key}"));
                            if let Some(u) = r["unit"].as_str() {
                                label = format!("{label} ({u})");
                            }
                            ui.label(RichText::new(label).color(secondary));
                            let value = self.format_value(key, &r["value"]);
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
                            ui.end_row();
                        }
                    });
            });
    }

    fn format_value(&self, key: &str, v: &Value) -> String {
        match v {
            Value::String(s) => match key {
                "validity" => self.i18n.tr(&format!("validity-{s}")),
                "role" => self.i18n.tr(&format!("role-{s}")),
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

    /// Legend of the colouring, top left under the view note.
    pub(crate) fn legend(&self, ui: &Ui, rect: egui::Rect, dark: bool) {
        let Some(m) = &self.session.mesh else {
            return;
        };
        let p = self.session.tokens.palette(dark);
        let t = &self.session.tokens;
        let mesh = &m.mesh;
        let entries: Vec<(murmel_engine::Rgb, String)> = match self.session.display.color_by {
            ColorBy::Patch => mesh
                .patches
                .iter()
                .enumerate()
                .filter(|(i, _)| !self.session.display.is_hidden(Item::Patch(*i as u32)))
                .map(|(i, pa)| (t.patch_color(i), pa.name.clone()))
                .collect(),
            ColorBy::Role => murmel_mesh_roles(mesh)
                .into_iter()
                .map(|r| (t.role_color(r), self.i18n.tr(&format!("role-{}", r.name()))))
                .collect(),
            ColorBy::CellType => murmel_engine::display::all_items(mesh)
                .into_iter()
                .filter_map(|i| match i {
                    Item::Kind(k) => {
                        Some((t.kind_color(k), self.i18n.tr(&format!("kind-{}", k.name()))))
                    }
                    _ => None,
                })
                .collect(),
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
        let at = rect.min + egui::vec2(16.0, 52.0);
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
fn number(x: f64) -> String {
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
