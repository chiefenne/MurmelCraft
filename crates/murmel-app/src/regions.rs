//! The model tree's "Regions of interest" (large-mesh post-processing,
//! stage 1): a row per region, like the slices' rows (the eye turns one
//! on and off, a click selects it so the Properties panel shows what it
//! takes, a double-click fits it, the context menu removes it), and the
//! buttons that add a box, a sphere, a slab or a region of the picked
//! cells. Every change calls a tool.

use egui::{RichText, Role, Sense, Ui, WidgetInfo};
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
        let open = self.session.mesh.as_ref()?;
        let (mesh, overview) = (&open.mesh, open.overview.is_some());
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
            if let Some(a) = self.blocks_row(ui, p) {
                action = Some(a);
            }
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
                // An overview's regions are shapes: they read the blocks
                // they touch (a region of cells needs them all).
                (
                    room && !picked.is_empty() && !overview,
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

    /// The row of an overview's block boxes: their eye, colour and how
    /// many the regions read.
    fn blocks_row(
        &self,
        ui: &mut Ui,
        p: &murmel_engine::theme::Palette,
    ) -> Option<(&'static str, Value)> {
        let m = self.session.mesh.as_ref()?;
        let d = &self.session.display;
        let boxes = d.block_boxes.as_deref()?;
        let item = Item::Blocks;
        let id = item.id(&m.mesh);
        let label = self.i18n.tr("tree-blocks");
        let on = d.show_blocks;
        let secondary = color(p.text_secondary);
        let mut action = None;
        ui.horizontal(|ui| {
            if self.eye(ui, on, &label, p).clicked() {
                let verb = if on { "hide" } else { "show" };
                action = Some(("display.visibility", json!({"action": verb, "items": [id]})));
            }
            if let Some(c) = d.item_color(&m.mesh, &self.session.tokens, item) {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                ui.painter().rect_filled(rect, 2.0, color(c));
            }
            let selected = d.is_selected(item);
            let text = if on {
                RichText::new(&label)
            } else {
                RichText::new(&label).color(secondary)
            };
            let count = self.i18n.tr_args(
                "tree-blocks-count",
                &[
                    ("read", (boxes.read.len() as f64).into()),
                    ("blocks", (boxes.count() as f64).into()),
                ],
            );
            let row = ui.selectable_label(selected, text);
            let name = format!("{label}, {count}");
            row.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, name.clone()));
            let row = row.on_hover_text(self.i18n.tr("tree-blocks-tip"));
            if row.clicked() {
                action = Some(("select.item", json!({ "item": id })));
            }
            row.context_menu(|ui| {
                // The colour opens a window of its own: a picker inside
                // the menu closed with it (docs/pitfalls.md).
                if ui.button(self.i18n.tr("tree-color-edit")).clicked() {
                    action = Some(("window.color", json!({"item": id, "label": label})));
                    ui.close();
                }
                if d.colors.contains_key(&item)
                    && ui.button(self.i18n.tr("tree-reset-color")).clicked()
                {
                    action = Some(("display.set_color", json!({"item": id, "color": null})));
                    ui.close();
                }
                ui.separator();
                if let Some(a) = self.view_default_entries(ui, "show_blocks") {
                    action = Some(a);
                    ui.close();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(count).monospace().color(secondary));
            });
        });
        action
    }
}
