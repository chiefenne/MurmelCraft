//! The quality legend (UI spec, Colour by: Quality): top left of the
//! viewport in colour mode quality, in place of the colour legend. The
//! metric, a histogram of the cells over the metric's range on the
//! quality ramp, and the threshold that hides good cells. Every change
//! calls a tool (`display.color_by`, `display.threshold`).

use std::sync::Arc;

use egui::{Rect, Role, Sense, Stroke, Ui, WidgetInfo};
use murmel_engine::quality::Stats;
use murmel_engine::{Metric, Quality};
use serde_json::json;

use crate::app::MurmelApp;
use crate::i18n::count;
use crate::panels::number;
use crate::style::color;

/// Width of the legend's content.
const WIDTH: f32 = 216.0;
/// Bins of the histogram.
const BINS: usize = 24;

/// The histogram of the shown metric, kept until the metric or the
/// quality changes.
pub(crate) struct HistogramCache {
    quality: Arc<Quality>,
    metric: Metric,
    stats: Stats,
}

impl MurmelApp {
    pub(crate) fn quality_legend(&mut self, ui: &mut Ui, rect: Rect, dark: bool) {
        let d = &self.session.display;
        let (Some(q), Some((lo, hi))) = (d.quality.clone(), d.quality_range()) else {
            return;
        };
        let metric = d.metric;
        let threshold = d.threshold;
        let stale = self
            .histogram
            .as_ref()
            .is_none_or(|h| h.metric != metric || !Arc::ptr_eq(&h.quality, &q));
        if stale {
            self.histogram = Some(HistogramCache {
                stats: q.stats(metric, BINS),
                quality: q.clone(),
                metric,
            });
        }
        let Some(stats) = self.histogram.as_ref().map(|h| h.stats.clone()) else {
            return;
        };
        let p = self.session.tokens.palette(dark).clone();
        let tokens = self.session.tokens.clone();
        let secondary = color(p.text_secondary);
        let shown_cells = self.session.mesh.as_ref().map_or(0, |m| m.surface.cells);

        let at = rect.min + egui::vec2(16.0, 52.0);
        let area = Rect::from_min_size(at, egui::vec2(WIDTH + 20.0, 220.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            egui::Frame::NONE
                .fill(color(p.bg_panel).gamma_multiply(0.92))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    // The metric.
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(self.i18n.tr("color-by-quality"))
                                .small()
                                .color(secondary),
                        );
                        let name = |m: Metric| self.i18n.tr(&format!("metric-{}", m.key()));
                        let mut pick = None;
                        let r = egui::ComboBox::from_id_salt("quality-metric")
                            .selected_text(name(metric))
                            .show_ui(ui, |ui| {
                                for m in Metric::ALL {
                                    if ui.selectable_label(m == metric, name(m)).clicked() {
                                        pick = Some(m);
                                    }
                                }
                            });
                        r.response.on_hover_text(self.i18n.tr("quality-metric-tip"));
                        if let Some(m) = pick.filter(|&m| m != metric) {
                            self.call("display.color_by", json!({"by": "quality", "metric": m}));
                        }
                    });

                    // Histogram over the ramp, with the threshold marked.
                    let shown_t = self.threshold_edit.or(threshold);
                    let (r, painter) = ui.allocate_painter(egui::vec2(WIDTH, 52.0), Sense::hover());
                    let bars = Rect::from_min_size(r.rect.min, egui::vec2(WIDTH, 36.0));
                    let ramp = Rect::from_min_size(
                        egui::pos2(r.rect.min.x, bars.bottom() + 4.0),
                        egui::vec2(WIDTH, 10.0),
                    );
                    let span = (hi - lo).max(f64::MIN_POSITIVE);
                    let x_of = |v: f64| ramp.left() + ((v - lo) / span) as f32 * WIDTH;
                    let peak = stats.histogram.iter().copied().max().unwrap_or(1).max(1);
                    let n = stats.histogram.len().max(1);
                    let bw = WIDTH / n as f32;
                    for (i, &c) in stats.histogram.iter().enumerate() {
                        if c == 0 {
                            continue;
                        }
                        // Square root, so a few bad cells still show
                        // beside thousands of good ones.
                        let h = (36.0 * (c as f32 / peak as f32).sqrt()).max(1.5);
                        let x0 = bars.left() + i as f32 * bw;
                        let mid = lo + span * (i as f64 + 0.5) / n as f64;
                        let mut fill = color(tokens.quality_color((i as f64 + 0.5) / n as f64));
                        if shown_t.is_some_and(|t| mid < t) {
                            fill = fill.gamma_multiply(0.3);
                        }
                        painter.rect_filled(
                            Rect::from_min_max(
                                egui::pos2(x0 + 0.5, bars.bottom() - h),
                                egui::pos2(x0 + bw - 0.5, bars.bottom()),
                            ),
                            1.0,
                            fill,
                        );
                    }
                    let steps = 48;
                    for i in 0..steps {
                        let t0 = i as f32 / steps as f32;
                        let seg = Rect::from_min_max(
                            egui::pos2(ramp.left() + t0 * WIDTH, ramp.top()),
                            egui::pos2(
                                ramp.left() + (t0 + 1.0 / steps as f32) * WIDTH + 0.5,
                                ramp.bottom(),
                            ),
                        );
                        let c = tokens.quality_color((i as f64 + 0.5) / steps as f64);
                        painter.rect_filled(seg, 0.0, color(c));
                    }
                    painter.rect_stroke(
                        ramp,
                        1.0,
                        Stroke::new(1.0, color(p.line)),
                        egui::StrokeKind::Outside,
                    );
                    if let Some(t) = shown_t {
                        let x = x_of(t.clamp(lo, hi));
                        painter.line_segment(
                            [
                                egui::pos2(x, bars.top()),
                                egui::pos2(x, ramp.bottom() + 2.0),
                            ],
                            Stroke::new(2.0, color(p.accent)),
                        );
                    }
                    let unit = metric.unit().unwrap_or("");
                    let desc = self.i18n.tr_args(
                        "quality-histogram",
                        &[
                            (
                                "metric",
                                self.i18n.tr(&format!("metric-{}", metric.key())).into(),
                            ),
                            ("cells", count(stats.cells).into()),
                            ("min", format!("{}{unit}", number(lo)).into()),
                            ("max", format!("{}{unit}", number(hi)).into()),
                        ],
                    );
                    r.widget_info(|| WidgetInfo::labeled(Role::Image, true, desc.clone()));
                    ui.horizontal(|ui| {
                        let mono =
                            |s: String| egui::RichText::new(s).monospace().small().color(secondary);
                        ui.label(mono(format!("{}{unit}", number(lo))));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(mono(format!("{}{unit}", number(hi))));
                        });
                    });

                    // The threshold.
                    ui.horizontal(|ui| {
                        let mut on = threshold.is_some();
                        let name = self.i18n.tr("quality-threshold");
                        let c = ui
                            .checkbox(&mut on, &name)
                            .on_hover_text(self.i18n.tr("quality-threshold-tip"));
                        if c.changed() {
                            let v = if on { Some(lo + 0.5 * (hi - lo)) } else { None };
                            self.threshold_edit = None;
                            self.call("display.threshold", json!({ "value": v }));
                        }
                    });
                    if let Some(t) = threshold {
                        let mut v = self.threshold_edit.unwrap_or(t);
                        let name = self.i18n.tr("quality-threshold");
                        let r = ui.add_sized(
                            [WIDTH, 18.0],
                            egui::Slider::new(&mut v, lo..=hi)
                                .suffix(unit)
                                .max_decimals(if metric == Metric::FaceCount { 0 } else { 3 }),
                        );
                        r.widget_info(|| WidgetInfo::slider(true, v, name.clone()));
                        if r.dragged() {
                            // Rebuild the surface once, when the drag ends.
                            self.threshold_edit = Some(v);
                        } else if r.drag_stopped() || (r.changed() && v != t) {
                            self.threshold_edit = None;
                            self.call("display.threshold", json!({ "value": v }));
                        }
                    }
                    ui.horizontal(|ui| {
                        let b = ui
                            .button(self.i18n.tr("quality-worst"))
                            .on_hover_text(self.i18n.tr("quality-worst-tip"));
                        if b.clicked()
                            && let Some(c) = self
                                .commands
                                .iter()
                                .find(|c| c.id == "display.threshold:worst")
                                .cloned()
                        {
                            let ctx = ui.ctx().clone();
                            self.threshold_edit = None;
                            self.run_command(&ctx, &c);
                        }
                        let shown = self.i18n.tr_args(
                            "quality-cells-shown",
                            &[
                                ("shown", count(shown_cells).into()),
                                ("cells", count(stats.cells).into()),
                            ],
                        );
                        ui.label(egui::RichText::new(shown).small().color(secondary));
                    });
                });
        });
    }
}
