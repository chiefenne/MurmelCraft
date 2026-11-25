//! Tips: a short one after a moment (250 ms by default), and more of the
//! control's help page after a longer rest, or at once while Shift is
//! held (the maintainer's card of 2026-11-25). A tip names the control,
//! says in a line what it does, shows your key, and is drawn from the
//! help pages of `murmel_engine::help`, so the tip, the Help panel and
//! the manual tell the same. A control the keyboard reached shows its
//! tip too.

use egui::{Id, Response, RichText, Ui};
use murmel_engine::help::{Page, Span, page_for, spans};

use crate::app::MurmelApp;
use crate::commands::key_text;
use crate::style::color;

/// Where the last tip drawn is noted (a [`Shown`]).
pub(crate) const SHOWN: &str = "murmel-tip-shown";
/// Where a tip's "More in the manual" leaves the page to open.
pub(crate) const OPEN: &str = "murmel-tip-open";

/// The last tip drawn: F1 opens its page, and the window tests read it
/// (a tooltip is not in the AccessKit tree).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Shown {
    pub title: String,
    pub rich: bool,
    pub page: Option<&'static str>,
    pub time: f64,
}

/// The page of the tip shown in the last moment, if any.
pub(crate) fn tip_page(ctx: &egui::Context) -> Option<&'static str> {
    let now = ctx.input(|i| i.time);
    ctx.data(|d| d.get_temp::<Shown>(Id::unique(SHOWN)))
        .filter(|s| now - s.time < 0.3)
        .and_then(|s| s.page)
}

/// Take the page a tip's link asked for.
pub(crate) fn take_open(ctx: &egui::Context) -> Option<&'static str> {
    ctx.data_mut(|d| d.remove_temp::<&'static str>(Id::unique(OPEN)))
}

/// The widget a tip was first wanted for, and since when.
#[derive(Clone, Copy, PartialEq)]
struct Since {
    widget: Id,
    time: f64,
}

/// What a tip shows.
struct Text {
    title: String,
    line: String,
    key: String,
    page: Option<&'static Page>,
}

impl MurmelApp {
    /// Set egui's tooltip delay to the quick tip's (settings.toml [help]),
    /// when it differs.
    pub(crate) fn apply_tip_delay(&self, ctx: &egui::Context) {
        let delay = self.session.help.tip_ms as f32 / 1000.0;
        if (ctx.global_style().interaction.tooltip_delay - delay).abs() > 1e-4 {
            ctx.all_styles_mut(|s| s.interaction.tooltip_delay = delay);
        }
    }

    /// The tip of `id` (a command, a tool, a page or a `ui.*` topic):
    /// the command's name, the page's line and the command's key.
    pub(crate) fn tip(&self, resp: Response, id: &str) -> Response {
        self.tip_as(resp, id, None, None)
    }

    /// The tip of `id` with a name and a line of the control's own (a
    /// combo box names its current choice; the toolbar has its own
    /// wording). A key already named in `line` is not shown twice.
    pub(crate) fn tip_as(
        &self,
        resp: Response,
        id: &str,
        title: Option<String>,
        line: Option<String>,
    ) -> Response {
        let page = page_for(id).or_else(|| murmel_engine::help::page(id));
        let command = self.commands.iter().find(|c| c.id == id);
        let title = title
            .or_else(|| command.map(|c| self.i18n.tr(&c.label_key)))
            .or_else(|| page.map(|p| self.i18n.tr(&p.title_key())))
            .unwrap_or_default();
        let line = line
            .or_else(|| page.map(|p| self.i18n.tr(&p.line_key())))
            .unwrap_or_default();
        let key = command
            .and_then(|c| c.keys.first())
            .map(|k| key_text(k))
            .filter(|k| !line.contains(k.as_str()))
            .unwrap_or_default();
        let text = Text {
            title,
            line,
            key,
            page,
        };
        self.show_tip(resp, &text)
    }

    /// Settings ▸ Tips: how long before a tip, and before it shows more.
    pub(crate) fn tip_settings(&mut self, ui: &mut Ui) {
        use murmel_engine::settings::MAX_TIP_MS;
        let help = self.session.help;
        // One line (the window does not scroll): Tips  show after [250 ms]
        // more after [1000 ms].
        ui.horizontal(|ui| {
            ui.heading(self.i18n.tr("settings-help"));
            for (field, label, tip, now) in [
                (
                    "tip_ms",
                    "settings-tip-delay",
                    "settings-tip-delay-tip",
                    help.tip_ms,
                ),
                (
                    "rich_ms",
                    "settings-rich-delay",
                    "settings-rich-delay-tip",
                    help.rich_ms,
                ),
            ] {
                let name = self.i18n.tr(label);
                // The value being dragged, kept until the drag ends so that
                // settings.toml is written once.
                let slot = Id::unique(("murmel-tip-edit", field));
                let mut v = ui.data(|d| d.get_temp::<u32>(slot)).unwrap_or(now);
                {
                    ui.label(&name);
                    let r = ui
                        .add(
                            egui::DragValue::new(&mut v)
                                .range(0..=MAX_TIP_MS)
                                .speed(10.0)
                                .suffix(" ms"),
                        )
                        .on_hover_text(self.i18n.tr(tip));
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::SpinButton, true, name.clone())
                    });
                    if r.changed() {
                        ui.data_mut(|d| d.insert_temp(slot, v));
                    }
                    let done = r.drag_stopped()
                        || (r.changed() && !r.dragged() && !r.has_focus())
                        || r.lost_focus();
                    if done {
                        ui.data_mut(|d| d.remove::<u32>(slot));
                        if v != now {
                            self.call("settings.set_help", serde_json::json!({ field: v }));
                        }
                    }
                }
            }
        });
    }

    fn show_tip(&self, resp: Response, t: &Text) -> Response {
        let ctx = resp.ctx.clone();
        let now = ctx.input(|i| i.time);
        let help = self.session.help;
        let slot = Id::unique("murmel-tip-since");
        let wanted = resp.hovered() || (self.focus.visible && resp.has_focus());
        if !wanted {
            // Left: the next visit starts with the quick tip again.
            ctx.data_mut(|d| {
                if d.get_temp::<Since>(slot)
                    .is_some_and(|s| s.widget == resp.id)
                {
                    d.remove::<Since>(slot);
                }
            });
            return resp;
        }
        let since = ctx.data_mut(|d| {
            let s = d.get_temp::<Since>(slot).filter(|s| s.widget == resp.id);
            let s = s.unwrap_or(Since {
                widget: resp.id,
                time: now,
            });
            d.insert_temp(slot, s);
            s.time
        });
        let rested = now - since;
        let quick = help.tip_ms as f64 / 1000.0;
        let more_at = quick + help.rich_ms as f64 / 1000.0;
        let shift = ctx.input(|i| i.modifiers.shift);
        let rich = t.page.is_some() && (shift || rested >= more_at);
        if t.page.is_some() && !rich {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                (more_at - rested).max(0.01),
            ));
        }
        let draw = |ui: &mut Ui| {
            self.tip_contents(ui, t, rich);
            let shown = Shown {
                title: t.title.clone(),
                rich,
                page: t.page.map(|p| p.name),
                time: now,
            };
            ui.ctx()
                .data_mut(|d| d.insert_temp(Id::unique(SHOWN), shown));
        };
        if resp.hovered() {
            resp.on_hover_ui(draw)
        } else {
            // The keyboard's control: the same tip after the same rest.
            if rested >= quick {
                resp.show_tooltip_ui(draw);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(quick - rested));
            }
            resp
        }
    }

    fn tip_contents(&self, ui: &mut Ui, t: &Text, rich: bool) {
        let p = self.palette(ui.ctx()).clone();
        let secondary = color(p.text_secondary);
        ui.set_max_width(if rich { 360.0 } else { 300.0 });
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&t.title)
                    .family(egui::FontFamily::Name(crate::style::SEMIBOLD.into()))
                    .color(color(p.text_primary)),
            );
            if !t.key.is_empty() {
                ui.label(RichText::new(&t.key).monospace().small().color(secondary));
            }
        });
        if !t.line.is_empty() && t.line != t.title {
            ui.label(&t.line);
        }
        let Some(page) = t.page else {
            return;
        };
        if !rich {
            ui.label(
                RichText::new(self.i18n.tr("help-hold-shift"))
                    .small()
                    .color(secondary),
            );
            return;
        }
        ui.separator();
        show_picture(ui, page, 320.0);
        markdown(ui, &page.tip, false);
        if !page.related.is_empty() {
            let names: Vec<String> = page
                .related
                .iter()
                .filter_map(|r| murmel_engine::help::page(r))
                .map(|r| self.i18n.tr(&r.title_key()))
                .collect();
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    self.i18n
                        .tr_args("help-related", &[("pages", names.join(", ").into())]),
                )
                .small()
                .color(secondary),
            );
        }
        if ui.link(self.i18n.tr("help-f1")).clicked() {
            ui.ctx()
                .data_mut(|d| d.insert_temp(Id::unique(OPEN), page.name));
        }
    }
}

/// A little Markdown: `## ` headings, paragraphs, `- ` bullets and the
/// spans of [`murmel_engine::help::spans`]. With `links`, a link is
/// clickable and the clicked one's target is returned; else it reads as
/// its text.
pub(crate) fn markdown(ui: &mut Ui, text: &str, links: bool) -> Option<String> {
    let mut hit = None;
    for block in text.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        if let Some(h) = block.strip_prefix("## ") {
            ui.add_space(4.0);
            ui.label(
                RichText::new(h)
                    .family(egui::FontFamily::Name(crate::style::SEMIBOLD.into()))
                    .size(15.0),
            );
            continue;
        }
        let bullets = block.lines().all(|l| l.starts_with("- "));
        if bullets {
            for l in block.lines() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label("•  ");
                    hit = inline(ui, &l[2..], links).or(hit.take());
                });
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                hit = inline(ui, &block.replace('\n', " "), links).or(hit.take());
            });
        }
        ui.add_space(2.0);
    }
    hit
}

/// One line of text with its bold, code and link spans.
fn inline(ui: &mut Ui, text: &str, links: bool) -> Option<String> {
    let mut hit = None;
    for span in spans(text) {
        let r = match span {
            Span::Plain(s) => RichText::new(s),
            Span::Bold(s) => {
                RichText::new(s).family(egui::FontFamily::Name(crate::style::SEMIBOLD.into()))
            }
            Span::Code(s) => RichText::new(s).monospace(),
            Span::Link { text, target } => {
                if links && ui.link(text).clicked() {
                    hit = Some(target.to_string());
                }
                if links {
                    continue;
                }
                RichText::new(text)
            }
        };
        ui.label(r);
    }
    hit
}

/// The help pages' pictures (`murmel help shots`), built in.
macro_rules! pictures {
    ($($name:literal),* $(,)?) => {
        const PICTURES: &[(&str, &[u8])] = &[
            $(($name, include_bytes!(concat!("../../../assets/help/pictures/", $name)))),*
        ];
    };
}

pictures!(
    "arrows.png",
    "clip.png",
    "color-by.png",
    "field-style.png",
    "iso.png",
    "quality.png",
    "regions.png",
    "render-mode.png",
    "slices.png",
    "streamlines.png",
);

/// A PNG as RGBA with its size.
fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let size = [info.width as usize, info.height as usize];
    match info.color_type {
        png::ColorType::Rgba => Some(egui::ColorImage::from_rgba_unmultiplied(size, &buf)),
        _ => None,
    }
}

/// A page's picture as a texture, decoded the first time it shows.
pub(crate) fn picture(ctx: &egui::Context, page: &Page) -> Option<egui::TextureHandle> {
    let name = page.picture.as_deref()?;
    let slot = Id::unique(("murmel-help-picture", name));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(slot)) {
        return Some(t);
    }
    let bytes = PICTURES.iter().find(|(n, _)| *n == name)?.1;
    let t = ctx.load_texture(
        format!("help-{name}"),
        decode(bytes)?,
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(slot, t.clone()));
    Some(t)
}

/// Draw a page's picture `width` points wide (at most what there is).
pub(crate) fn show_picture(ui: &mut Ui, page: &Page, width: f32) {
    if let Some(t) = picture(ui.ctx(), page) {
        let [w, h] = t.size();
        let width = width.min(ui.available_width());
        let size = egui::vec2(width, width * h as f32 / w as f32);
        let name = page.name.to_string();
        let r = ui.add(
            egui::Image::new(&t)
                .fit_to_exact_size(size)
                .corner_radius(4.0),
        );
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, name.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the built-in pictures are exactly the pages' pictures,
    /// and each decodes. Breaks when `murmel help shots` made a picture
    /// that is not built in (its page would show none) or a page names
    /// one that is not.
    #[test]
    fn pictures_are_built_in() {
        let mut built: Vec<&str> = PICTURES.iter().map(|(n, _)| *n).collect();
        built.sort();
        let mut named: Vec<&str> = murmel_engine::help::pages()
            .iter()
            .filter_map(|p| p.picture.as_deref())
            .collect();
        named.sort();
        assert_eq!(built, named);
        for (n, b) in PICTURES {
            let img = decode(b).unwrap_or_else(|| panic!("{n} does not decode"));
            assert_eq!(img.size, [640, 400], "{n}");
        }
    }
}
