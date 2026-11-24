//! The chart panel and the statistics panel (D268, second milestone):
//! below the viewport while a run's tables are open (`charts.open`, or
//! `series.open` of a folder that has them). Left, the chart the session
//! names (`charts.show`) with a cursor at the player's step; right, the
//! statistics at that step (`stats.at`). Every control calls a tool, so
//! agents and scripts see and do the same; the chart is drawn from the
//! same layout as `charts.export` ([`murmel_engine::chart::Chart::layout`]).

use std::path::PathBuf;

use egui::{FontId, Pos2, RichText, Role, Shape, Stroke, Ui, WidgetInfo, pos2};
use murmel_engine::chart::{Anchor, ChartColorsResolved, ChartKind, Prim, Weight};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::style::color;

/// Width of the statistics column, points.
const STATS_W: f32 = 300.0;

/// A number with its unit, SI scaled for reading (`8.28 g`).
fn amount(v: Option<f64>, unit: &str) -> String {
    let Some(v) = v else {
        return "–".to_string();
    };
    let (u, f): (&str, f64) = match unit {
        "kg" if v.abs() < 1e-3 => ("mg", 1e6),
        "kg" if v.abs() < 1.0 => ("g", 1e3),
        "kg/s" if v.abs() < 1e-3 => ("mg/s", 1e6),
        "kg/s" if v.abs() < 1.0 => ("g/s", 1e3),
        "m" if v != 0.0 && v.abs() < 1e-3 => ("µm", 1e6),
        "m" if v.abs() < 1.0 => ("mm", 1e3),
        _ => (unit, 1.0),
    };
    let s = v * f;
    let t = if s.abs() >= 100.0 || s == 0.0 {
        format!("{s:.0}")
    } else if s.abs() >= 10.0 {
        format!("{s:.1}")
    } else {
        format!("{s:.2}")
    };
    if u.is_empty() || u == "-" {
        t
    } else {
        format!("{t} {u}")
    }
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

impl MurmelApp {
    /// Ask for an output folder and open its tables.
    pub(crate) fn charts_open_dialog(&mut self) {
        let mut d = rfd::FileDialog::new().set_title(self.i18n.tr("cmd-charts-open"));
        if let Some(dir) = self
            .session
            .series
            .as_ref()
            .and_then(|r| r.path.parent().map(PathBuf::from))
            .or_else(|| {
                self.session
                    .mesh
                    .as_ref()
                    .and_then(|m| m.path.parent().map(PathBuf::from))
            })
        {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_folder() {
            self.call("charts.open", json!({ "path": p }));
        }
    }

    /// Ask for a .png or .svg file and write the panel's chart.
    pub(crate) fn charts_export_dialog(&mut self) {
        if self.session.charts.is_none() {
            return;
        }
        let d = rfd::FileDialog::new()
            .set_title(self.i18n.tr("cmd-charts-export"))
            .add_filter("PNG", &["png"])
            .add_filter("SVG", &["svg"])
            .set_file_name("chart.png");
        if let Some(p) = d.save_file() {
            self.call("charts.export", json!({ "path": p }));
        }
    }

    /// The chart panel with the statistics beside it.
    pub(crate) fn chart_panel(&mut self, ui: &mut Ui) {
        let Some(c) = self.session.charts.clone() else {
            return;
        };
        let dark = self.dark(ui.ctx());
        let p = self.palette(ui.ctx()).clone();
        let secondary = color(p.text_secondary);
        let mut calls: Vec<(&'static str, Value)> = Vec::new();
        let step = murmel_tools::charts::player_step(&self.session);
        {
            // The chart column left (its choices on one row, the chart
            // below), the statistics right.
            let full = ui.available_rect_before_wrap();
            let split = (full.max.x - STATS_W - 12.0).max(full.min.x + 200.0);
            let chart_rect = egui::Rect::from_min_max(full.min, pos2(split, full.max.y));
            let stats_rect = egui::Rect::from_min_max(pos2(split + 12.0, full.min.y), full.max);
            ui.painter()
                .vline(split + 6.0, full.y_range(), Stroke::new(1.0, color(p.line)));
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(chart_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let name = self.i18n.tr("charts-chart");
                        let mut kind = c.spec.chart;
                        let combo = egui::ComboBox::from_id_salt("chart-kind")
                            .selected_text(self.i18n.tr(kind.label_key()))
                            .show_ui(ui, |ui| {
                                for k in ChartKind::ALL {
                                    ui.selectable_value(&mut kind, k, self.i18n.tr(k.label_key()));
                                }
                            });
                        combo.response.widget_info(|| {
                            WidgetInfo::labeled(Role::ComboBox, true, name.clone())
                        });
                        self.focus
                            .offer(crate::focus::Region::Charts, &combo.response);
                        if kind != c.spec.chart {
                            calls.push(("charts.show", json!({"chart": kind.key()})));
                        }
                        if c.spec.chart == ChartKind::Histogram {
                            let have: Vec<String> = c.run.histograms.keys().cloned().collect();
                            let shown = c
                                .spec
                                .datums
                                .first()
                                .cloned()
                                .or_else(|| have.first().cloned())
                                .unwrap_or_default();
                            let mut pick = shown.clone();
                            let dname = self.i18n.tr("charts-datum");
                            let r = egui::ComboBox::from_id_salt("chart-datum")
                                .selected_text(&shown)
                                .show_ui(ui, |ui| {
                                    for d in &have {
                                        ui.selectable_value(&mut pick, d.clone(), d);
                                    }
                                });
                            r.response.widget_info(|| {
                                WidgetInfo::labeled(Role::ComboBox, true, dname.clone())
                            });
                            if pick != shown {
                                calls.push(("charts.show", json!({"datums": [pick]})));
                            }
                            let mut mass = c.spec.weight == Weight::Mass;
                            if ui
                                .checkbox(&mut mass, self.i18n.tr("charts-by-mass"))
                                .changed()
                            {
                                calls.push((
                                    "charts.show",
                                    json!({"weight": if mass { "mass" } else { "droplets" }}),
                                ));
                            }
                            let mut cum = c.spec.cumulative;
                            if ui
                                .checkbox(&mut cum, self.i18n.tr("charts-cumulative"))
                                .changed()
                            {
                                calls.push(("charts.show", json!({"cumulative": cum})));
                            }
                        }
                        let save = ui
                            .button(self.i18n.tr("charts-save"))
                            .on_hover_text(self.i18n.tr("cmd-charts-export"));
                        if save.clicked() {
                            calls.push(("window.charts_export", Value::Null));
                        }
                    });
                    match c.run.chart(&c.spec, step, &self.i18n) {
                        Ok(chart) => {
                            let size = ui.available_size();
                            let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
                            let title = chart.title.clone();
                            resp.widget_info(|| {
                                WidgetInfo::labeled(Role::Image, true, title.clone())
                            });
                            let painter = ui.painter_at(rect);
                            let colors = ChartColorsResolved::new(&self.session.tokens, dark);
                            let measure = |t: &str, pt: f64| {
                                painter
                                    .layout_no_wrap(
                                        t.to_string(),
                                        FontId::proportional(pt as f32),
                                        secondary,
                                    )
                                    .size()
                                    .x as f64
                            };
                            let prims = chart.layout(
                                rect.width() as f64,
                                rect.height() as f64,
                                false,
                                &measure,
                            );
                            let at = |q: [f64; 2]| -> Pos2 {
                                pos2(rect.min.x + q[0] as f32, rect.min.y + q[1] as f32)
                            };
                            for q in prims {
                                match q {
                                    Prim::Rect { rect: r, ink } => {
                                        painter.rect_filled(
                                            egui::Rect::from_min_max(
                                                at([r[0], r[1]]),
                                                at([r[2], r[3]]),
                                            ),
                                            0.0,
                                            color(colors.of(ink)),
                                        );
                                    }
                                    Prim::Polyline { points, width, ink } => {
                                        painter.add(Shape::line(
                                            points.into_iter().map(at).collect(),
                                            Stroke::new(width as f32, color(colors.of(ink))),
                                        ));
                                    }
                                    Prim::Text {
                                        text,
                                        at: a,
                                        size,
                                        anchor,
                                        ink,
                                    } => {
                                        let g = painter.layout_no_wrap(
                                            text,
                                            FontId::proportional(size as f32),
                                            color(colors.of(ink)),
                                        );
                                        let base = at(a);
                                        let x = match anchor {
                                            Anchor::Start => base.x,
                                            Anchor::Middle => base.x - g.size().x / 2.0,
                                            Anchor::End => base.x - g.size().x,
                                        };
                                        // egui places a galley by its top;
                                        // the layout gives the baseline.
                                        let y = base.y - 0.8 * g.size().y;
                                        painter.galley(pos2(x, y), g, color(colors.of(ink)));
                                    }
                                }
                            }
                            if let Some(n) = chart.notes.first() {
                                ui.label(RichText::new(n).small().color(secondary));
                            }
                        }
                        Err(e) => {
                            ui.label(RichText::new(e.0).color(color(p.status_warn)));
                        }
                    }
                },
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(stats_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| self.stats_column(ui, step, secondary),
            );
            ui.allocate_rect(full, egui::Sense::hover());
        }
        for (id, input) in calls {
            if id == "window.charts_export" {
                self.charts_export_dialog();
            } else {
                self.call(id, input);
            }
        }
    }

    /// The statistics at the player's step, as `stats.at` gives them.
    fn stats_column(&mut self, ui: &mut Ui, step: Option<u64>, secondary: egui::Color32) {
        let Some(c) = &self.session.charts else {
            return;
        };
        let s = c.run.statistics(step);
        ui.add_space(4.0);
        let head = match (s["step"].as_u64(), num(&s["time"])) {
            (Some(st), Some(t)) => self.i18n.tr_args(
                "stats-at-time",
                &[
                    ("step", (st as f64).into()),
                    ("time", format!("{t:.4}").into()),
                ],
            ),
            (Some(st), None) => self
                .i18n
                .tr_args("stats-at", &[("step", (st as f64).into())]),
            _ => self.i18n.tr("stats-title"),
        };
        ui.label(RichText::new(head).strong());
        egui::ScrollArea::vertical()
            .id_salt("stats-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let row = |ui: &mut Ui, k: String, v: String| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(k).small().color(secondary));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(v).small().monospace());
                        });
                    });
                };
                let sp = &s["spray"];
                if sp.is_object() {
                    ui.label(RichText::new(self.i18n.tr("stats-spray")).small().strong());
                    for (key, field, unit) in [
                        ("stats-injected", "injected_kg", "kg"),
                        ("stats-in-flow", "in_flow_kg", "kg"),
                        ("stats-escaped", "escaped_kg", "kg"),
                        ("stats-deposited", "deposited_kg", "kg"),
                        ("stats-lost", "lost_kg", "kg"),
                        ("stats-parcels", "parcels", ""),
                        ("stats-d32-flow", "d32_m", "m"),
                    ] {
                        row(ui, self.i18n.tr(key), amount(num(&sp[field]), unit));
                    }
                    for inj in sp["injectors"].as_array().into_iter().flatten() {
                        let n = inj["name"].as_str().unwrap_or("");
                        row(
                            ui,
                            self.i18n.tr_args(
                                "stats-penetration",
                                &[("injector", n.to_string().into())],
                            ),
                            amount(num(&inj["penetration_m"]), "m"),
                        );
                    }
                }
                for d in s["datums"].as_array().into_iter().flatten() {
                    let liq = &d["liquid"];
                    if !liq.is_object() {
                        continue;
                    }
                    ui.add_space(4.0);
                    let n = d["name"].as_str().unwrap_or("").to_string();
                    ui.label(RichText::new(n).small().strong());
                    if d["kind"] == "volume" {
                        row(
                            ui,
                            self.i18n.tr("stats-liquid-inside"),
                            amount(num(&liq["kg"]), "kg"),
                        );
                        row(
                            ui,
                            self.i18n.tr("stats-parcels"),
                            amount(num(&liq["parcels"]), ""),
                        );
                    } else {
                        row(
                            ui,
                            self.i18n.tr("stats-through"),
                            amount(num(&liq["kg_net"]), "kg"),
                        );
                        row(
                            ui,
                            self.i18n.tr("stats-rate"),
                            amount(num(&liq["rate_kg_s"]), "kg/s"),
                        );
                        row(
                            ui,
                            self.i18n.tr("stats-d10"),
                            amount(num(&liq["d10_m"]), "m"),
                        );
                    }
                    row(
                        ui,
                        self.i18n.tr("stats-d32"),
                        amount(num(&liq["d32_m"]), "m"),
                    );
                    if d["kind"] != "volume" {
                        row(
                            ui,
                            self.i18n.tr("stats-dv50"),
                            amount(num(&liq["dv50_m"]), "m"),
                        );
                    }
                    let h = &d["histogram"];
                    if h.is_object() {
                        row(
                            ui,
                            self.i18n.tr("stats-dv10-dv90"),
                            format!(
                                "{} / {}",
                                amount(num(&h["dv10_m"]), "m"),
                                amount(num(&h["dv90_m"]), "m")
                            ),
                        );
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    /// Invariant: amounts read in the unit that suits them, a missing
    /// value shows a dash. Breaks if a factor or a threshold slips; the
    /// window test reads the panel only by name.
    #[test]
    fn amounts_are_scaled() {
        assert_eq!(super::amount(Some(0.00828), "kg"), "8.28 g");
        assert_eq!(super::amount(Some(9.1e-5), "m"), "91.0 µm");
        assert_eq!(super::amount(Some(0.0085), "kg/s"), "8.50 g/s");
        assert_eq!(super::amount(Some(1744.0), ""), "1744");
        assert_eq!(super::amount(None, "kg"), "–");
    }
}
