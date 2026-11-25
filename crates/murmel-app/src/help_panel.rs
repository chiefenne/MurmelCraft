//! The Help panel: the manual, docked at the right of the window (the
//! maintainer's card of 2026-11-25). It shows the contents by chapter, a
//! page of `assets/help/en`, or the pages a search finds; F1 on a tip or
//! the tip's "More in the manual" opens the tip's page. A page lists the
//! commands it explains with your keys, and each runs from there.

use egui::{RichText, Role, Ui, WidgetInfo};
use murmel_engine::help::{self, Page};

use crate::app::MurmelApp;
use crate::commands::key_text;
use crate::style::color;
use crate::tips::markdown;

/// What the panel shows.
#[derive(Default)]
pub(crate) struct HelpView {
    /// The page, by file name; `None` is the contents.
    pub page: Option<&'static str>,
    /// The pages before it, for Back.
    pub back: Vec<Option<&'static str>>,
    /// The search; not empty, it shows the pages found.
    pub query: String,
}

/// What a click in the panel asks for.
enum Go {
    Page(Option<&'static str>),
    Back,
    Url(String),
    Command(murmel_tools::commands::Command),
    Close,
}

impl MurmelApp {
    /// Open the panel at `page` (the contents for `None`); the page it
    /// showed goes on the Back list.
    pub(crate) fn open_help(&mut self, page: Option<&'static str>) {
        let v = self.help_view.get_or_insert_with(HelpView::default);
        if v.page != page || !v.query.is_empty() {
            if v.page != page {
                v.back.push(v.page);
            }
            v.page = page;
            v.query.clear();
        }
    }

    pub(crate) fn help_panel(&mut self, ui: &mut Ui) {
        let Some(view) = &self.help_view else {
            return;
        };
        let secondary = color(self.palette(ui.ctx()).text_secondary);
        let mut go = None;
        let mut query = view.query.clone();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !view.back.is_empty(),
                    egui::Button::new(self.i18n.tr("help-back")),
                )
                .clicked()
            {
                go = Some(Go::Back);
            }
            if ui
                .add_enabled(
                    view.page.is_some() || !view.query.is_empty(),
                    egui::Button::new(self.i18n.tr("help-contents")),
                )
                .clicked()
            {
                go = Some(Go::Page(None));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = self.i18n.tr("help-close");
                let b = ui.button("×");
                b.widget_info(|| WidgetInfo::labeled(Role::Button, true, close.clone()));
                if b.on_hover_text(&close).clicked() {
                    go = Some(Go::Close);
                }
            });
        });
        let name = self.i18n.tr("help-search");
        let r = ui.add(
            egui::TextEdit::singleline(&mut query)
                .hint_text(&name)
                .desired_width(f32::INFINITY),
        );
        r.widget_info(|| WidgetInfo::labeled(Role::SearchInput, true, name.clone()));
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let picked = if !query.trim().is_empty() {
                    self.help_found(ui, &query, secondary)
                } else {
                    match view.page.and_then(help::page) {
                        Some(p) => self.help_page(ui, p, secondary),
                        None => self.help_contents(ui, secondary),
                    }
                };
                if picked.is_some() {
                    go = picked;
                }
            });
        let Some(view) = &mut self.help_view else {
            return;
        };
        view.query = query;
        match go {
            Some(Go::Page(p)) => {
                if view.page != p {
                    view.back.push(view.page);
                }
                view.page = p;
                view.query.clear();
            }
            Some(Go::Back) => {
                if let Some(p) = view.back.pop() {
                    view.page = p;
                    view.query.clear();
                }
            }
            Some(Go::Url(u)) => ui.ctx().open_url(egui::OpenUrl::new_tab(u)),
            Some(Go::Command(c)) => {
                let ctx = ui.ctx().clone();
                self.run_command(&ctx, &c);
            }
            Some(Go::Close) => self.help_view = None,
            None => {}
        }
    }

    /// A page's name as a link, with its line below.
    fn page_link(&self, ui: &mut Ui, p: &'static Page, secondary: egui::Color32) -> Option<Go> {
        let clicked = ui.link(self.i18n.tr(&p.title_key())).clicked();
        ui.label(
            RichText::new(self.i18n.tr(&p.line_key()))
                .small()
                .color(secondary),
        );
        ui.add_space(4.0);
        clicked.then_some(Go::Page(Some(p.name)))
    }

    fn help_contents(&self, ui: &mut Ui, secondary: egui::Color32) -> Option<Go> {
        let mut go = None;
        for (chapter, pages) in help::manual() {
            ui.add_space(6.0);
            ui.label(
                RichText::new(self.i18n.tr(&format!("help-chapter-{chapter}")))
                    .family(egui::FontFamily::Name(crate::style::SEMIBOLD.into()))
                    .size(15.0),
            );
            for p in pages {
                go = self.page_link(ui, p, secondary).or(go);
            }
        }
        go
    }

    fn help_found(&self, ui: &mut Ui, query: &str, secondary: egui::Color32) -> Option<Go> {
        let mut go = None;
        let mut any = false;
        for p in help::pages() {
            let extra = format!(
                "{} {}",
                self.i18n.tr(&p.title_key()),
                self.i18n.tr(&p.line_key())
            );
            if help::matches(p, query, &extra) {
                any = true;
                go = self.page_link(ui, p, secondary).or(go);
            }
        }
        if !any {
            ui.label(RichText::new(self.i18n.tr("help-no-match")).color(secondary));
        }
        go
    }

    fn help_page(&self, ui: &mut Ui, p: &'static Page, secondary: egui::Color32) -> Option<Go> {
        let mut go = None;
        ui.label(
            RichText::new(self.i18n.tr(&p.title_key()))
                .family(egui::FontFamily::Name(crate::style::SEMIBOLD.into()))
                .size(18.0),
        );
        ui.label(RichText::new(self.i18n.tr(&p.line_key())).color(secondary));
        ui.add_space(4.0);
        if let Some(target) = markdown(ui, &p.text, true) {
            go = Some(match help::page(&target) {
                Some(t) => Go::Page(Some(t.name)),
                None => Go::Url(target),
            });
        }
        // The commands of this page, with your keys; a click runs one.
        let mine: Vec<_> = self
            .commands
            .iter()
            .filter(|c| help::page_for(&c.id).is_some_and(|q| q.name == p.name))
            .collect();
        if !mine.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new(self.i18n.tr("help-commands"))
                    .family(egui::FontFamily::Name(crate::style::SEMIBOLD.into())),
            );
            for c in mine {
                let keys = c.keys.first().map(|k| key_text(k)).unwrap_or_default();
                let why = self.applies(c).err();
                let b = ui.add_enabled(
                    why.is_none(),
                    egui::Button::new(self.i18n.tr(&c.label_key))
                        .shortcut_text(RichText::new(keys).color(secondary)),
                );
                let b = match why {
                    Some(key) => b.on_disabled_hover_text(self.i18n.tr(key)),
                    None => b,
                };
                if b.clicked() {
                    go = Some(Go::Command(c.clone()));
                }
            }
        }
        if !p.related.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new(self.i18n.tr("help-see-also"))
                    .family(egui::FontFamily::Name(crate::style::SEMIBOLD.into())),
            );
            for r in p.related.iter().filter_map(|r| help::page(r)) {
                go = self.page_link(ui, r, secondary).or(go);
            }
        }
        go
    }
}
