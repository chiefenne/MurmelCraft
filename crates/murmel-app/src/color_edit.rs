//! The colour window of a model tree item (its context menu's
//! "Colour…"): hue, saturation and brightness sliders, the hex code and
//! the patch palette as swatches. Every change calls `display.set_color`,
//! so the mesh shows it at once; the window's undo group (open while a
//! mouse button is held) makes one slider drag one undo step.
//!
//! The picker is a window of its own, not egui's colour button inside the
//! menu: that button's popup closed with the menu at the first click into
//! it (docs/pitfalls.md). It is built from named sliders and buttons, so a
//! screen reader and the keyboard reach every part of it.

use egui::ecolor::Hsva;
use egui::{Key, Modifiers, RichText, Role, Sense, WidgetInfo};
use murmel_engine::{Item, Rgb};
use serde_json::json;

use crate::app::MurmelApp;
use crate::style::color;

/// The colour window while it is open.
#[derive(Clone, Debug)]
pub struct ColorEdit {
    /// The item's id (`zone:fluid`).
    pub item: String,
    /// The item's name as the tree shows it.
    pub label: String,
    /// Hue, saturation and brightness, 0 to 1, kept here so the hue
    /// survives a grey (saturation 0).
    pub hsv: [f32; 3],
    /// The hex field's text.
    pub hex: String,
    /// The colour last set (HSV in floats does not give it back exactly).
    pub rgb: Rgb,
}

impl ColorEdit {
    pub fn new(item: String, label: String, rgb: Rgb) -> Self {
        let h = Hsva::from_srgb(rgb.0);
        Self {
            item,
            label,
            hsv: [h.h, h.s, h.v],
            hex: rgb.hex(),
            rgb,
        }
    }

    /// The colour of the sliders.
    fn sliders_rgb(&self) -> Rgb {
        let [h, s, v] = self.hsv;
        Rgb(Hsva::new(h, s, v, 1.0).to_srgb())
    }

    fn set(&mut self, rgb: Rgb) {
        let h = Hsva::from_srgb(rgb.0);
        // A grey has no hue: keep the one the sliders had.
        if h.s > 0.0 {
            self.hsv[0] = h.h;
        }
        self.hsv[1] = h.s;
        self.hsv[2] = h.v;
        self.hex = rgb.hex();
        self.rgb = rgb;
    }
}

impl MurmelApp {
    /// The colour the tree shows for `id` now (its own, else the colour
    /// mode's, else the solid colour).
    pub(crate) fn item_rgb(&self, id: &str) -> Option<Rgb> {
        let m = self.session.mesh.as_ref()?;
        let item = Item::parse(id, &m.mesh)?;
        let d = &self.session.display;
        Some(
            d.item_color(&m.mesh, &self.session.tokens, item)
                .or_else(|| d.colors.get(&item).copied())
                .unwrap_or(self.session.tokens.mesh.solid),
        )
    }

    /// Open the colour window for `id`.
    pub(crate) fn open_color_edit(&mut self, id: &str, label: &str) {
        if let Some(rgb) = self.item_rgb(id) {
            self.color_edit = Some(ColorEdit::new(id.to_string(), label.to_string(), rgb));
        }
    }

    pub(crate) fn color_window(&mut self, ctx: &egui::Context) {
        let Some(mut ed) = self.color_edit.take() else {
            return;
        };
        // The item went away (another mesh, a closed one).
        let Some(now) = self.item_rgb(&ed.item) else {
            return;
        };
        let own = self.session.mesh.as_ref().is_some_and(|m| {
            Item::parse(&ed.item, &m.mesh)
                .is_some_and(|i| self.session.display.colors.contains_key(&i))
        });
        // Changed elsewhere (undo, an agent): follow it.
        if now != ed.rgb {
            ed.set(now);
        }
        let p = self.palette(ctx).clone();
        let swatches = self.session.tokens.mesh.patch.clone();
        let title = self
            .i18n
            .tr_args("color-title", &[("item", ed.label.clone().into())]);
        let mut open = true;
        let mut done = false;
        let mut picked: Option<Rgb> = None;
        let mut picked_by_slider = false;
        let mut reset = false;
        egui::Window::new(title)
            .id(egui::Id::unique("color-edit"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            // Beside the model tree, not over it.
            .default_pos(ctx.content_rect().left_top() + egui::vec2(290.0, 120.0))
            .show(ctx, |ui| {
                ui.spacing_mut().slider_width = 160.0;
                let (r, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::hover());
                ui.painter().rect_filled(r, 4.0, color(ed.rgb));
                ui.painter().rect_stroke(
                    r,
                    4.0,
                    egui::Stroke::new(1.0, color(p.line)),
                    egui::StrokeKind::Inside,
                );
                ui.add_space(6.0);
                let mut hsv = ed.hsv;
                egui::Grid::new("color-sliders")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        for (k, key, scale, unit) in [
                            (0, "color-hue", 360.0, "°"),
                            (1, "color-saturation", 100.0, " %"),
                            (2, "color-brightness", 100.0, " %"),
                        ] {
                            let name = self.i18n.tr(key);
                            ui.label(&name);
                            let mut x = hsv[k] * scale;
                            let r = ui.add(
                                // Shown without decimals but not rounded:
                                // a rounding slider reports a change on
                                // the first frame and recoloured the item.
                                egui::Slider::new(&mut x, 0.0..=scale)
                                    .suffix(unit)
                                    .custom_formatter(|v, _| format!("{v:.0}")),
                            );
                            r.widget_info(|| WidgetInfo::slider(true, x as f64, name.clone()));
                            if r.changed() {
                                hsv[k] = x / scale;
                            }
                            ui.end_row();
                        }
                        let name = self.i18n.tr("color-hex");
                        ui.label(&name);
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut ed.hex)
                                .desired_width(90.0)
                                .font(egui::TextStyle::Monospace),
                        );
                        crate::focus::set_name(ui, r.id, &name);
                        if r.lost_focus() {
                            let text = ed.hex.trim();
                            let text = if text.starts_with('#') {
                                text.to_string()
                            } else {
                                format!("#{text}")
                            };
                            match Rgb::parse(&text) {
                                Some(c) => picked = Some(c),
                                None => ed.hex = ed.rgb.hex(),
                            }
                        }
                        ui.end_row();
                    });
                if hsv != ed.hsv {
                    ed.hsv = hsv;
                    picked = Some(ed.sliders_rgb());
                    picked_by_slider = true;
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new(self.i18n.tr("color-swatches"))
                        .small()
                        .color(color(p.text_secondary)),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    for (n, c) in swatches.iter().enumerate() {
                        let name = self.i18n.tr_args(
                            "color-swatch",
                            &[("n", (n + 1).into()), ("hex", c.hex().into())],
                        );
                        let r = ui.add(
                            egui::Button::new("")
                                .fill(color(*c))
                                .min_size(egui::vec2(18.0, 18.0)),
                        );
                        r.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
                        if r.on_hover_text(c.hex()).clicked() {
                            picked = Some(*c);
                        }
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if own && ui.button(self.i18n.tr("tree-reset-color")).clicked() {
                        reset = true;
                    }
                    if ui.button(self.i18n.tr("color-done")).clicked() {
                        done = true;
                    }
                });
            });
        if let Some(c) = picked
            && c != now
        {
            // Keep the sliders where the person put them (a grey keeps
            // its hue), the rest follows the colour.
            let hsv = ed.hsv;
            ed.set(c);
            if picked_by_slider {
                ed.hsv = hsv;
            }
            self.call(
                "display.set_color",
                json!({"item": ed.item, "color": c.hex()}),
            );
        }
        if reset {
            self.call("display.set_color", json!({"item": ed.item, "color": null}));
            if let Some(c) = self.item_rgb(&ed.item) {
                ed.set(c);
            }
        }
        let esc = !ctx.text_edit_focused()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        if open && !done && !esc {
            self.color_edit = Some(ed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the sliders' colour round-trips through the hex code
    /// and a grey keeps the hue the sliders had. Breaks if the HSV
    /// conversion drops a channel or a grey resets the hue slider; the
    /// window test sets the colour by the hex field only.
    #[test]
    fn hsv_round_trip() {
        let c = Rgb([0xD3, 0x81, 0x51]);
        let mut ed = ColorEdit::new("zone:fluid".into(), "fluid".into(), c);
        assert_eq!(ed.sliders_rgb(), c);
        let hue = ed.hsv[0];
        ed.set(Rgb([0x80, 0x80, 0x80]));
        assert_eq!(ed.hsv[0], hue);
        assert_eq!(ed.hsv[1], 0.0);
        assert_eq!(ed.hex, "#808080");
    }
}
