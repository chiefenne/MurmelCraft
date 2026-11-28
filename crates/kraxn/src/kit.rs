//! The pieces Kraxn's window is built of: buttons, cards and the step
//! list, in MurmelCraft's tokens (taken from the style's visuals).

use egui::{Color32, CornerRadius, Margin, Response, RichText, Stroke, Ui, Vec2};

/// The accent colour (the logo's magenta).
pub fn accent(ui: &Ui) -> Color32 {
    ui.visuals().hyperlink_color
}

/// Text on the accent: the panel colour in light, the app's in dark.
fn on_accent(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        ui.visuals().extreme_bg_color
    } else {
        ui.visuals().panel_fill
    }
}

/// Semibold text in the primary text colour (`strong()` would take the
/// accent).
pub fn bold(text: impl Into<String>) -> RichText {
    RichText::new(text).family(egui::FontFamily::Name(crate::style::SEMIBOLD.into()))
}

pub fn secondary_text(ui: &Ui) -> Color32 {
    ui.visuals().widgets.noninteractive.fg_stroke.color
}

/// The main action of a page: filled with the accent.
pub fn primary(ui: &mut Ui, enabled: bool, text: &str) -> Response {
    let fill = if enabled {
        accent(ui)
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    let color = if enabled {
        on_accent(ui)
    } else {
        secondary_text(ui)
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(bold(text).color(color))
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(8))
            .min_size(Vec2::new(132.0, 38.0)),
    )
}

/// Other actions: outlined.
pub fn secondary(ui: &mut Ui, enabled: bool, text: &str) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(bold(text))
            .corner_radius(CornerRadius::same(8))
            .min_size(Vec2::new(110.0, 38.0)),
    )
}

/// A raised card; `selected` draws its edge in the accent.
pub fn card<R>(ui: &mut Ui, selected: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    let stroke = if selected {
        Stroke::new(2.0, accent(ui))
    } else {
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
    };
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, add)
        .inner
}

/// A page's title and the line under it.
pub fn title(ui: &mut Ui, title: &str, sub: Option<&str>) {
    ui.label(bold(title).size(26.0));
    if let Some(s) = sub {
        ui.label(RichText::new(s).color(secondary_text(ui)));
    }
    ui.add_space(16.0);
}

/// One of two or three choices: a card with a radio button (its name is
/// the title) and a line below.
pub fn choice<V: PartialEq + Copy>(
    ui: &mut Ui,
    value: &mut V,
    this: V,
    title: &str,
    text: &str,
) -> bool {
    let selected = *value == this;
    let mut changed = false;
    card(ui, selected, |ui| {
        ui.set_width(ui.available_width());
        if radio(ui, selected, title).clicked() {
            *value = this;
            changed = true;
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            ui.label(RichText::new(text).color(secondary_text(ui)));
        });
    });
    ui.add_space(8.0);
    changed
}

/// A radio button with a ring that reads on both themes: grey when off,
/// the accent with a dot when on; the whole row takes the click.
fn radio(ui: &mut Ui, selected: bool, title: &str) -> Response {
    let galley = egui::WidgetText::from(bold(title).size(16.0)).into_galley(
        ui,
        Some(egui::TextWrapMode::Wrap),
        ui.available_width() - 28.0,
        egui::TextStyle::Body,
    );
    let h = galley.size().y.max(20.0);
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), h), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, title));
    let hovered = resp.hovered() || resp.has_focus();
    let centre = egui::pos2(rect.left() + 9.0, rect.center().y);
    let ring = if selected || hovered {
        accent(ui)
    } else {
        secondary_text(ui)
    };
    let p = ui.painter();
    p.circle_stroke(centre, 8.0, Stroke::new(2.0, ring));
    if selected {
        p.circle_filled(centre, 4.5, accent(ui));
    }
    let text = ui.visuals().text_color();
    p.galley(
        egui::pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0),
        galley,
        text,
    );
    if resp.has_focus() {
        p.rect_stroke(
            rect.expand(4.0),
            CornerRadius::same(6),
            ui.visuals().selection.stroke,
            egui::StrokeKind::Outside,
        );
    }
    resp
}

/// One button of a row of segments: filled with the accent when chosen.
pub fn segment<V: PartialEq + Copy>(ui: &mut Ui, value: &mut V, this: V, text: &str) -> Response {
    let on = *value == this;
    let (fill, color) = if on {
        (accent(ui), on_accent(ui))
    } else {
        (ui.visuals().faint_bg_color, ui.visuals().text_color())
    };
    let r = ui.add(
        egui::Button::selectable(on, bold(text).color(color))
            .fill(fill)
            .corner_radius(CornerRadius::same(16))
            .min_size(Vec2::new(0.0, 30.0)),
    );
    if r.clicked() {
        *value = this;
    }
    r
}

/// A logo, `size` points square, with its name for screen readers.
pub fn logo(ui: &mut Ui, tex: Option<&egui::TextureHandle>, size: f32, name: &str) {
    match tex {
        Some(t) => {
            ui.add(
                egui::Image::new(t)
                    .fit_to_exact_size(Vec2::splat(size))
                    .corner_radius(CornerRadius::same((size * 0.22) as u8))
                    .alt_text(format!("{name} logo")),
            );
        }
        None => {
            ui.allocate_space(Vec2::splat(size));
        }
    }
}

/// The state of a step in the side list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Done,
    Now,
    Later,
}

/// One row of the side list: a dot and the step's name.
pub fn step_row(ui: &mut Ui, n: usize, name: &str, mark: Mark) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), egui::Sense::hover());
        let a = accent(ui);
        let line = ui.visuals().widgets.noninteractive.bg_stroke.color;
        let p = ui.painter();
        let c = rect.center();
        match mark {
            Mark::Done => {
                p.circle_filled(c, 11.0, a);
                // A check mark, painted (the fonts have no ✓).
                p.line(
                    vec![
                        c + Vec2::new(-5.0, 0.5),
                        c + Vec2::new(-1.5, 4.0),
                        c + Vec2::new(5.0, -3.5),
                    ],
                    Stroke::new(2.0, on_accent(ui)),
                );
            }
            Mark::Now => {
                p.circle_stroke(c, 10.5, Stroke::new(2.0, a));
                p.text(
                    c,
                    egui::Align2::CENTER_CENTER,
                    n.to_string(),
                    egui::FontId::proportional(12.0),
                    a,
                );
            }
            Mark::Later => {
                p.circle_stroke(c, 10.5, Stroke::new(1.5, line));
                p.text(
                    c,
                    egui::Align2::CENTER_CENTER,
                    n.to_string(),
                    egui::FontId::proportional(12.0),
                    secondary_text(ui),
                );
            }
        }
        let text = match mark {
            Mark::Now => bold(name),
            Mark::Done => RichText::new(name),
            Mark::Later => RichText::new(name).color(secondary_text(ui)),
        };
        ui.label(text);
    });
    ui.add_space(6.0);
}
