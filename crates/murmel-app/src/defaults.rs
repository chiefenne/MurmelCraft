//! Your defaults in the window (the maintainer's card of 2026-11-24:
//! right-click any value or item to save it as the default; new items
//! start from it; Settings lists the defaults and forgets each; open
//! items keep theirs). The entries go into the context menus of the
//! model tree's item rows, of the values under them and of the
//! Properties rows; every change calls `settings.set_default` or
//! `settings.reset_default`.

use egui::{RichText, Role, Ui, WidgetInfo};
use murmel_engine::Item;
use murmel_engine::defaults::Kind;
use serde_json::{Value, json};

use crate::app::MurmelApp;

/// The name a Properties row of `kind` has for its value `key` of the
/// item (they differ where the panel says it in other words).
pub(crate) fn item_key(kind: Kind, prop: &str) -> Option<&'static str> {
    let key = match (kind, prop) {
        (Kind::Arrows, "on_surface") => "surface",
        (Kind::Stream, "seeds_wanted") => "count",
        (Kind::Region, "mode") => "cut",
        (_, k) => k,
    };
    kind.keys().iter().copied().find(|k| *k == key)
}

impl MurmelApp {
    /// The menu entries that keep the values `keys` of item `id` (every
    /// value its kind can keep when empty) as the defaults of new items,
    /// and forget them again when they are saved.
    pub(crate) fn default_entries(
        &self,
        ui: &mut Ui,
        item: Item,
        keys: &[&str],
    ) -> Option<(&'static str, Value)> {
        let m = self.session.mesh.as_ref()?;
        let kind = Kind::of_item(item)?;
        self.entries(
            ui,
            kind,
            keys,
            json!({"item": item.id(&m.mesh), "keys": keys}),
        )
    }

    /// The menu entries that keep the view's value `key` (render mode,
    /// mesh look or projection) as the one the window starts with.
    pub(crate) fn view_default_entries(
        &self,
        ui: &mut Ui,
        key: &'static str,
    ) -> Option<(&'static str, Value)> {
        self.entries(
            ui,
            Kind::View,
            &[key],
            json!({"kind": Kind::View.key(), "keys": [key]}),
        )
    }

    /// The menu entries that keep value `key` of `kind` with
    /// settings.set_default's input `save`.
    pub(crate) fn kind_default_entries(
        &self,
        ui: &mut Ui,
        kind: Kind,
        key: &'static str,
        save: Value,
    ) -> Option<(&'static str, Value)> {
        self.entries(ui, kind, &[key], save)
    }

    /// Save `keys` of `kind` (all of them when empty) with
    /// settings.set_default's input `save`, and forget them once saved.
    fn entries(
        &self,
        ui: &mut Ui,
        kind: Kind,
        keys: &[&str],
        save: Value,
    ) -> Option<(&'static str, Value)> {
        let label = match keys {
            [one] => self.i18n.tr_args(
                "defaults-save-value",
                &[("value", self.default_key_name(kind, one).into())],
            ),
            _ => self.i18n.tr_args(
                "defaults-save-item",
                &[(
                    "kind",
                    self.i18n
                        .tr(&format!("defaults-kind-{}", kind.key()))
                        .into(),
                )],
            ),
        };
        let mut action = None;
        if ui
            .button(label)
            .on_hover_text(self.i18n.tr("defaults-save-tip"))
            .clicked()
        {
            action = Some(("settings.set_default", save));
            ui.close();
        }
        let wanted: Vec<&str> = if keys.is_empty() {
            kind.keys().to_vec()
        } else {
            keys.to_vec()
        };
        let saved = wanted
            .iter()
            .any(|k| self.session.defaults.get(kind, k).is_some());
        if saved
            && ui
                .button(self.i18n.tr("defaults-forget-these"))
                .on_hover_text(self.i18n.tr("defaults-forget-tip"))
                .clicked()
        {
            action = Some(match wanted.as_slice() {
                [one] if !keys.is_empty() => (
                    "settings.reset_default",
                    json!({"kind": kind.key(), "key": one}),
                ),
                _ => ("settings.reset_default", json!({"kind": kind.key()})),
            });
            ui.close();
        }
        action
    }

    /// The translated name of value `key` of `kind`.
    pub(crate) fn default_key_name(&self, kind: Kind, key: &str) -> String {
        self.i18n.tr(&format!("default-{}-{key}", kind.key()))
    }

    /// A saved default's value as the window says it elsewhere: a mode's
    /// translated name, yes or no, or the number.
    fn default_value_text(&self, kind: Kind, key: &str, value: &Value) -> String {
        match value {
            Value::Bool(b) => self.i18n.tr(if *b { "value-yes" } else { "value-no" }),
            Value::String(s) => {
                let family = match (kind, key) {
                    (Kind::Arrows, "length") => Some("arrow-length"),
                    (Kind::Arrows | Kind::Iso, "color") => Some("iso-color"),
                    (Kind::Stream, "direction") => Some("stream-direction"),
                    (Kind::Stream, "look") => Some("stream-look"),
                    (Kind::Clip, "mode") | (Kind::Region, "cut") => Some("clip-mode"),
                    (Kind::FieldStyle, "mode") => Some("field-mode"),
                    (Kind::FieldStyle, "range") => Some("field-range"),
                    (Kind::FieldStyle, "map") => Some("colormap"),
                    (Kind::View, "render_mode") => Some("render-mode"),
                    (Kind::View, "projection") => Some("projection"),
                    (Kind::View, "mesh_look") => Some("mesh-look"),
                    _ => None,
                };
                family
                    .map(|f| format!("{f}-{s}"))
                    .filter(|k| self.i18n.has(k))
                    .map_or_else(|| s.clone(), |k| self.i18n.tr(&k))
            }
            Value::Number(n) => n
                .as_f64()
                .map_or_else(|| n.to_string(), crate::panels::number),
            other => other.to_string(),
        }
    }

    /// The Settings window's list of your defaults, each with a button
    /// that forgets it, and one that forgets them all.
    pub(crate) fn defaults_settings(&mut self, ui: &mut Ui) {
        ui.heading(self.i18n.tr("settings-defaults"));
        let rows = self.session.defaults.rows();
        if rows.is_empty() {
            ui.label(RichText::new(self.i18n.tr("settings-defaults-none")).small());
            return;
        }
        let mut call: Option<(&'static str, Value)> = None;
        egui::ScrollArea::vertical()
            .id_salt("defaults")
            .max_height(180.0)
            .show(ui, |ui| {
                for (kind, key, value) in &rows {
                    let text = self.i18n.tr_args(
                        "settings-default-row",
                        &[
                            (
                                "kind",
                                self.i18n
                                    .tr(&format!("defaults-kind-{}", kind.key()))
                                    .into(),
                            ),
                            ("value", self.default_key_name(*kind, key).into()),
                            ("is", self.default_value_text(*kind, key, value).into()),
                        ],
                    );
                    ui.horizontal(|ui| {
                        let name = self
                            .i18n
                            .tr_args("settings-default-forget", &[("row", text.clone().into())]);
                        let b = ui
                            .small_button(self.i18n.tr("defaults-forget"))
                            .on_hover_text(&name);
                        b.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
                        if b.clicked() {
                            call = Some((
                                "settings.reset_default",
                                json!({"kind": kind.key(), "key": key}),
                            ));
                        }
                        ui.label(RichText::new(&text).small());
                    });
                }
            });
        if ui
            .button(self.i18n.tr("settings-defaults-forget-all"))
            .clicked()
        {
            call = Some(("settings.reset_default", json!({})));
        }
        if let Some((tool, input)) = call {
            self.call(tool, input);
        }
    }
}
