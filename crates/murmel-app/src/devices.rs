//! The injector devices in the window (FoehnCraft D345, plan stage
//! ID2m): a row in the model tree's "Slices and iso-surfaces" under the
//! parcels (the eye shows and hides them, a click selects them, the
//! context menu shows and hides their parts and holds their colour), and
//! an "Open injector devices…" button. Every change calls a tool
//! (`devices.open`, `devices.set`, `devices.clear`).

use egui::{RichText, Role, Sense, Ui, WidgetInfo};
use murmel_engine::Item;
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::style::color;

impl MurmelApp {
    /// The label of the devices: their file.
    pub(crate) fn devices_label(&self) -> String {
        let Some(v) = &self.session.display.devices else {
            return String::new();
        };
        let file = v
            .path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned());
        self.i18n
            .tr_args("tree-devices-label", &[("file", file.into())])
    }

    /// Ask for a device file and show its devices.
    pub(crate) fn devices_open_dialog(&mut self) {
        let mut d = rfd::FileDialog::new()
            .set_title(self.i18n.tr("cmd-devices-open"))
            .add_filter(self.i18n.tr("open-filter-devices"), &["vtu"]);
        // FoehnCraft writes it to the spray folder beside the result.
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
            self.call("devices.open", json!({ "path": p }));
        }
    }

    /// The devices' row in the tree, and the button that opens them
    /// while parcels show without them.
    pub(crate) fn device_rows(
        &self,
        ui: &mut Ui,
        p: &murmel_engine::theme::Palette,
    ) -> Option<(&'static str, Value)> {
        let m = self.session.mesh.as_ref()?;
        let d = &self.session.display;
        let tokens = &self.session.tokens;
        let secondary = color(p.text_secondary);
        let mut action = None;
        if let (Some(v), Some(data)) = (&d.devices, d.device_data.as_deref()) {
            let item = Item::Devices;
            let id = item.id(&m.mesh);
            let label = self.devices_label();
            ui.horizontal(|ui| {
                if self.eye(ui, v.enabled, &label, p).clicked() {
                    action = Some(("devices.set", json!({"enabled": !v.enabled})));
                }
                let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                let rgb = murmel_engine::device::surface_color(d, tokens);
                ui.painter().rect_filled(rect, 2.0, color(rgb));
                let selected = d.is_selected(item);
                let text = if v.enabled {
                    RichText::new(&label)
                } else {
                    RichText::new(&label).color(secondary)
                };
                let r = ui.selectable_label(selected, text);
                r.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, label.clone()));
                let r = r.on_hover_text(self.i18n.tr("tree-devices-tip"));
                if r.clicked() {
                    action = Some(("select.item", json!({ "item": id })));
                }
                r.context_menu(|ui| {
                    let mut calls = Vec::new();
                    for (key, on, name) in [
                        ("surface", v.surface, "devices-surface"),
                        ("axes", v.axes, "devices-axes"),
                        ("cones", v.cones, "devices-cones"),
                    ] {
                        let mut x = on;
                        if ui.checkbox(&mut x, self.i18n.tr(name)).changed() {
                            calls.push(("devices.set", json!({ key: x })));
                        }
                    }
                    if ui.button(self.i18n.tr("tree-color-edit")).clicked() {
                        calls.push((
                            "window.color",
                            json!({"item": "devices", "label": self.devices_label()}),
                        ));
                    }
                    ui.separator();
                    if ui.button(self.i18n.tr("cmd-devices-open")).clicked() {
                        calls.push(("window.devices_open", json!({})));
                    }
                    if ui.button(self.i18n.tr("cmd-devices-clear")).clicked() {
                        calls.push(("devices.clear", json!({})));
                    }
                    if let Some(c) = calls.pop() {
                        action = Some(c);
                        ui.close();
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                let count = self.i18n.tr_args(
                    "tree-devices-count",
                    &[
                        ("devices", data.count().into()),
                        ("holes", data.holes().into()),
                    ],
                );
                ui.label(RichText::new(count).small().color(secondary));
            });
        } else if d.parcels.is_some() {
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                let b = ui.add(egui::Button::new(
                    RichText::new(self.i18n.tr("cmd-devices-open")).small(),
                ));
                let b = self.tip(b, "window.devices_open");
                if b.clicked() {
                    action = Some(("window.devices_open", json!({})));
                }
            });
        }
        action
    }
}
