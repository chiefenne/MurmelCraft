//! The model tree's "Regions of interest" (large-mesh post-processing,
//! stage 1): a row per region, like the slices' rows (the eye turns one
//! on and off, a click selects it so the Properties panel shows what it
//! takes, a double-click fits it, the context menu removes it), and the
//! buttons that add a box, a sphere, a slab or a region of the picked
//! cells. Every change calls a tool.

use egui::{RichText, Role, Ui, WidgetInfo};
use murmel_engine::Item;
use murmel_engine::region::{MAX_REGIONS, Shape};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::style::color;

impl MurmelApp {
    /// The label of region `k` in the tree: its number, its shape and the
    /// cells it takes.
    pub(crate) fn region_label(&self, k: usize) -> String {
        let d = &self.session.display;
        let Some(r) = d.regions.get(k) else {
            return String::new();
        };
        let what = match &r.shape {
            Shape::Zone { zone } => format!("{} {zone}", self.i18n.tr("region-shape-zone")),
            Shape::Patch { patch, layers } => self.i18n.tr_args(
                "region-patch-layers",
                &[
                    ("patch", patch.clone().into()),
                    ("layers", (*layers).into()),
                ],
            ),
            s => self.i18n.tr(&format!("region-shape-{}", s.kind())),
        };
        let what = if r.cut == murmel_engine::ClipMode::Flat {
            self.i18n
                .tr_args("region-cut-flat", &[("shape", what.into())])
        } else {
            what
        };
        let cells = d
            .region_cells
            .as_ref()
            .filter(|rc| rc.regions == d.regions)
            .and_then(|rc| rc.estimates.get(k))
            .map(|e| e.cells);
        match cells {
            Some(n) => self.i18n.tr_args(
                "tree-region-label",
                &[
                    ("n", (k + 1).into()),
                    ("shape", what.into()),
                    ("cells", n.into()),
                ],
            ),
            None => format!("{} {}: {what}", self.i18n.tr("props-area-region"), k + 1),
        }
    }

    /// The section: a row per region and the buttons that add one.
    pub(crate) fn region_section(
        &self,
        ui: &mut Ui,
        p: &murmel_engine::theme::Palette,
    ) -> Option<(&'static str, Value)> {
        let mesh = &self.session.mesh.as_ref()?.mesh;
        let d = &self.session.display;
        let secondary = color(p.text_secondary);
        let mut action = None;
        egui::CollapsingHeader::new(
            RichText::new(self.i18n.tr("tree-regions").to_uppercase())
                .small()
                .color(secondary),
        )
        .id_salt("tree-regions")
        .default_open(true)
        .show(ui, |ui| {
            for (k, r) in d.regions.iter().enumerate() {
                let item = Item::Region(k as u32);
                let id = item.id(mesh);
                let n = k + 1;
                let label = self.region_label(k);
                ui.horizontal(|ui| {
                    if self.eye(ui, r.enabled, &label, p).clicked() {
                        action = Some(("region.set", json!({"region": n, "enabled": !r.enabled})));
                    }
                    let selected = d.is_selected(item);
                    let text = if r.enabled {
                        RichText::new(&label)
                    } else {
                        RichText::new(&label).color(secondary)
                    };
                    let row = ui.selectable_label(selected, text);
                    row.widget_info(|| {
                        WidgetInfo::selected(Role::Button, true, selected, label.clone())
                    });
                    let row = row.on_hover_text(self.i18n.tr("tree-region-tip"));
                    if row.clicked() {
                        action = Some(("select.item", json!({ "item": id })));
                    }
                    if row.double_clicked() {
                        action = Some(("view.fit", json!({ "item": id })));
                    }
                    row.context_menu(|ui| {
                        let mut calls = vec![("tree-fit", ("view.fit", json!({ "item": id })))];
                        if r.shape.cuts_flat() {
                            calls.push(match r.cut {
                                murmel_engine::ClipMode::Crinkle => (
                                    "tree-region-cut-flat",
                                    ("region.set", json!({"region": n, "cut": "flat"})),
                                ),
                                murmel_engine::ClipMode::Flat => (
                                    "tree-region-cut-whole",
                                    ("region.set", json!({"region": n, "cut": "crinkle"})),
                                ),
                            });
                        }
                        calls.push((
                            "tree-region-remove",
                            ("region.clear", json!({ "region": n })),
                        ));
                        for (key, call) in calls {
                            if ui.button(self.i18n.tr(key)).clicked() {
                                action = Some(call);
                                ui.close();
                            }
                        }
                        ui.separator();
                        if let Some(a) = self.default_entries(ui, item, &[]) {
                            action = Some(a);
                        }
                    });
                });
            }
            let picked: Vec<u32> = d
                .selection
                .iter()
                .filter_map(|i| match i {
                    Item::Cell(c) => Some(*c),
                    _ => None,
                })
                .collect();
            let room = d.regions.len() < MAX_REGIONS;
            let adds = [
                (
                    room,
                    "tree-region-new-box",
                    ("region.set", json!({"new": true, "shape": "box"})),
                ),
                (
                    room,
                    "tree-region-new-sphere",
                    ("region.set", json!({"new": true, "shape": "sphere"})),
                ),
                (
                    room,
                    "tree-region-new-slab",
                    ("region.set", json!({"new": true, "shape": "slab"})),
                ),
                (
                    room && !picked.is_empty(),
                    "tree-region-new-picked",
                    (
                        "region.set",
                        json!({"new": true, "shape": "cells", "cells": picked, "rings": 1}),
                    ),
                ),
            ];
            for (show, key, call) in adds {
                if !show {
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    let b = ui
                        .add(egui::Button::new(RichText::new(self.i18n.tr(key)).small()))
                        .on_hover_text(self.i18n.tr("tree-region-new-tip"));
                    if b.clicked() {
                        action = Some(call);
                    }
                });
            }
        });
        action
    }
}
