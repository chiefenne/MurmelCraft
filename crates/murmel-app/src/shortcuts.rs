//! The shortcut editor (Settings, the F1 sheet and the palette open it):
//! every command's keys and the mouse bindings in one window. A key is
//! recorded by pressing it; a key or a mouse input another command or
//! action has shows a warning with Replace and Cancel (the maintainer's
//! card of 2026-11-25), and nothing changes until one is chosen. Keys go
//! through `settings.set_keys` (`[keys]` in settings.toml), the mouse
//! through `settings.set_bindings` (navigation.toml).

use egui::{Key, RichText, Role, Ui, WidgetInfo};
use murmel_engine::keys;
use murmel_engine::navigation::{Action, Binding, Button, Gesture, Mods, Navigation, PRESETS};
use murmel_tools::commands::{GROUPS, holders, key_problems};
use serde_json::json;

use crate::app::MurmelApp;
use crate::commands::key_text;
use crate::style::color;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Keys,
    Mouse,
}

/// A key another command has, waiting for Replace or Cancel.
#[derive(Clone, Debug)]
pub struct KeyClash {
    pub command: String,
    /// The command's keys with the new one.
    pub keys: Vec<String>,
    pub key: String,
    /// The names of the commands that have it.
    pub others: Vec<String>,
}

/// A mouse binding whose input another binding has, waiting for Replace
/// (drop the other) or Cancel.
#[derive(Clone, Debug)]
pub struct MouseClash {
    pub draft: Vec<Binding>,
    pub other: usize,
}

/// The editor's state while it is open.
#[derive(Clone, Debug, Default)]
pub struct Editor {
    pub tab: Tab,
    pub filter: String,
    /// The command a key is being recorded for.
    pub recording: Option<String>,
    pub clash: Option<KeyClash>,
    pub mouse_clash: Option<MouseClash>,
    /// A refused key or a key macOS takes first.
    pub note: Option<String>,
    /// The speeds while their fields are dragged.
    pub orbit_edit: Option<f64>,
    pub zoom_edit: Option<f64>,
}

/// What a click in the keys tab asks for, done after the list is drawn.
enum KeyDo {
    Set(String, Vec<String>),
    Record(String),
    Reset(String),
    ResetAll,
    Replace,
    Cancel,
}

/// The key text of a press: the modifiers and the key (the key's place
/// on the keyboard while Alt is held, as the window matches it), or why
/// it cannot be a shortcut.
pub(crate) fn pressed_text(
    key: Key,
    physical: Option<Key>,
    m: egui::Modifiers,
) -> Result<String, Refused> {
    // On macOS `command` is Cmd and a lone `ctrl` is the Control key,
    // which the table cannot name.
    if m.ctrl && !m.command {
        return Err(Refused::MacControl);
    }
    let k = if m.alt { physical.unwrap_or(key) } else { key };
    let mut t = String::new();
    for (on, p) in [(m.command, "Ctrl+"), (m.alt, "Alt+"), (m.shift, "Shift+")] {
        if on {
            t.push_str(p);
        }
    }
    t.push_str(k.name());
    let c = keys::canonical(&t).map_err(|_| Refused::Unknown(t))?;
    if keys::reserved(&c) {
        return Err(Refused::Reserved(c));
    }
    Ok(c)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Refused {
    MacControl,
    Reserved(String),
    Unknown(String),
}

impl MurmelApp {
    pub(crate) fn open_bindings(&mut self, tab: Tab) {
        let e = self.bindings.get_or_insert_with(Editor::default);
        e.tab = tab;
    }

    /// While a key is being recorded, take this frame's key presses (so no
    /// command runs and no widget sees them): Esc cancels, any other
    /// press becomes the key. Returns whether it took the keyboard.
    pub(crate) fn record_key(&mut self, ctx: &egui::Context) -> bool {
        let Some(id) = self.bindings.as_ref().and_then(|e| e.recording.clone()) else {
            return false;
        };
        let mut got: Option<Result<String, Refused>> = None;
        let mut cancel = false;
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed,
                    modifiers,
                    ..
                } => {
                    if *pressed && got.is_none() && !cancel {
                        if *key == Key::Escape && modifiers.is_none() {
                            cancel = true;
                        } else {
                            got = Some(pressed_text(*key, *physical_key, *modifiers));
                        }
                    }
                    false
                }
                egui::Event::Text(_) => false,
                _ => true,
            });
        });
        if !cancel && got.is_none() {
            return true;
        }
        if let Some(e) = self.bindings.as_mut() {
            e.recording = None;
        }
        match got {
            Some(Ok(k)) => {
                let mut list = self.keys_of(&id);
                if !list.contains(&k) {
                    list.push(k);
                }
                self.set_command_keys(&id, list, false);
            }
            Some(Err(r)) => {
                let note = match r {
                    Refused::MacControl => {
                        let (ctrl, _) = crate::i18n::modifier_names();
                        self.i18n
                            .tr_args("bindings-mac-control", &[("ctrl", ctrl.into())])
                    }
                    Refused::Reserved(k) | Refused::Unknown(k) => self
                        .i18n
                        .tr_args("bindings-reserved", &[("key", key_text(&k).into())]),
                };
                if let Some(e) = self.bindings.as_mut() {
                    e.note = Some(note);
                }
            }
            None => {}
        }
        true
    }

    /// The keys a command has now.
    pub(crate) fn keys_of(&self, id: &str) -> Vec<String> {
        self.commands
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.keys.clone())
            .unwrap_or_default()
    }

    /// Give a command its keys; a key another command has stops at the
    /// Replace or Cancel warning unless `replace`.
    pub(crate) fn set_command_keys(&mut self, id: &str, list: Vec<String>, replace: bool) {
        if !replace {
            for k in &list {
                let others: Vec<String> = holders(&self.commands, k, id)
                    .iter()
                    .map(|c| self.i18n.tr(&c.label_key))
                    .collect();
                if !others.is_empty() {
                    if let Some(e) = self.bindings.as_mut() {
                        e.clash = Some(KeyClash {
                            command: id.to_string(),
                            keys: list.clone(),
                            key: k.clone(),
                            others,
                        });
                    }
                    return;
                }
            }
        }
        let out = self.call(
            "settings.set_keys",
            json!({"command": id, "keys": list, "replace": replace}),
        );
        self.rebuild_commands();
        let note = out
            .as_ref()
            .and_then(|o| o["notes"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .find_map(|n| {
                let why = n["macos"].as_str()?;
                let k = n["key"].as_str()?;
                Some(
                    self.i18n
                        .tr_args(&format!("keys-os-{why}"), &[("key", key_text(k).into())]),
                )
            });
        if let Some(e) = self.bindings.as_mut() {
            e.clash = None;
            e.note = note;
        }
    }

    /// The command table with your keys, after they changed (here, from
    /// a tool call of an agent, or at start).
    pub(crate) fn rebuild_commands(&mut self) {
        if self.keys_built != self.session.keys {
            self.commands = murmel_tools::commands::with_keys(&self.session.keys);
            self.keys_built = self.session.keys.clone();
            self.share_keys();
        }
    }

    /// Give the strings that name keys (`{ $key-view-fit }`) the keys of
    /// now.
    pub(crate) fn share_keys(&mut self) {
        let shown: Vec<(String, String)> = self
            .commands
            .iter()
            .map(|c| (c.id.clone(), self.key_or_none(&c.id)))
            .collect();
        self.i18n.set_keys(shown);
    }

    pub(crate) fn bindings_window(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.bindings.as_ref().map(|e| e.tab) else {
            return;
        };
        let mut open = true;
        egui::Window::new(self.i18n.tr("bindings-title"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .default_height(560.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (t, key) in [
                        (Tab::Keys, "bindings-tab-keys"),
                        (Tab::Mouse, "bindings-tab-mouse"),
                    ] {
                        let name = self.i18n.tr(key);
                        let r = ui.selectable_label(tab == t, &name);
                        r.widget_info(|| {
                            WidgetInfo::selected(Role::Tab, true, tab == t, name.clone())
                        });
                        if r.clicked()
                            && let Some(e) = self.bindings.as_mut()
                        {
                            e.tab = t;
                        }
                    }
                });
                ui.separator();
                match tab {
                    Tab::Keys => self.keys_tab(ui),
                    Tab::Mouse => self.mouse_tab(ui),
                }
                if let Some(d) = &self.session.config_dir {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(self.i18n.tr_args(
                            "bindings-file",
                            &[("path", d.join("settings.toml").display().to_string().into())],
                        ))
                        .small(),
                    );
                }
            });
        if !open {
            self.bindings = None;
        }
    }

    /// A warning line in the warning colour.
    fn warn(&self, ui: &mut Ui, text: &str) {
        let c = color(self.palette(ui.ctx()).status_warn);
        ui.label(RichText::new(format!("{}  {text}", crate::style::icon::ALERT)).color(c));
    }

    fn keys_tab(&mut self, ui: &mut Ui) {
        let Some(ed) = self.bindings.clone() else {
            return;
        };
        let mut todo: Option<KeyDo> = None;
        let problems = key_problems(&self.session.keys);
        if !problems.is_empty() {
            self.warn(ui, &self.i18n.tr("bindings-problems"));
            for p in &problems {
                ui.label(RichText::new(p).small().monospace());
            }
            ui.add_space(4.0);
        }
        if let Some(c) = &ed.clash {
            ui.group(|ui| {
                self.warn(
                    ui,
                    &self.i18n.tr_args(
                        "bindings-clash",
                        &[
                            ("key", key_text(&c.key).into()),
                            ("others", c.others.join(", ").into()),
                        ],
                    ),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button(self.i18n.tr("bindings-replace"))
                        .on_hover_text(self.i18n.tr("bindings-replace-tip"))
                        .clicked()
                    {
                        todo = Some(KeyDo::Replace);
                    }
                    if ui.button(self.i18n.tr("bindings-cancel")).clicked() {
                        todo = Some(KeyDo::Cancel);
                    }
                });
            });
        }
        if let Some(n) = &ed.note {
            self.warn(ui, n);
        }
        let mut filter = ed.filter.clone();
        ui.horizontal(|ui| {
            let name = self.i18n.tr("bindings-filter-name");
            let r = ui.add(
                egui::TextEdit::singleline(&mut filter)
                    .hint_text(self.i18n.tr("bindings-filter"))
                    .desired_width(280.0),
            );
            r.widget_info(|| WidgetInfo::labeled(Role::SearchInput, true, name.clone()));
            if !self.session.keys.is_empty()
                && ui.button(self.i18n.tr("bindings-reset-all")).clicked()
            {
                todo = Some(KeyDo::ResetAll);
            }
        });
        if let Some(e) = self.bindings.as_mut() {
            e.filter = filter.clone();
        }
        let secondary = color(self.palette(ui.ctx()).text_secondary);
        let q = filter.to_lowercase();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height() - 24.0)
            .show(ui, |ui| {
                for g in GROUPS {
                    let rows: Vec<(String, String, Vec<String>)> = self
                        .commands
                        .iter()
                        .filter(|c| c.group == *g)
                        .map(|c| (c.id.clone(), self.i18n.tr(&c.label_key), c.keys.clone()))
                        .filter(|(id, label, keys)| {
                            let words = format!(
                                "{label} {id} {} {}",
                                keys.iter()
                                    .map(|k| key_text(k))
                                    .collect::<Vec<_>>()
                                    .join(" "),
                                keys.join(" ")
                            )
                            .to_lowercase();
                            q.split_whitespace().all(|w| words.contains(w))
                        })
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(self.i18n.tr(&format!("group-{g}")).to_uppercase())
                            .small()
                            .color(secondary),
                    );
                    egui::Grid::new(format!("bind-keys-{g}"))
                        .num_columns(3)
                        .min_col_width(220.0)
                        .striped(true)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            for (id, label, keys) in rows {
                                ui.add(egui::Label::new(&label).truncate())
                                    .on_hover_text(&id);
                                ui.horizontal(|ui| {
                                    for k in &keys {
                                        let shown = key_text(k);
                                        let name = self.i18n.tr_args(
                                            "bindings-remove-name",
                                            &[
                                                ("key", shown.clone().into()),
                                                ("command", label.clone().into()),
                                            ],
                                        );
                                        let r = ui
                                            .button(
                                                RichText::new(format!("{shown}  ×")).monospace(),
                                            )
                                            .on_hover_text(&name);
                                        r.widget_info(|| {
                                            WidgetInfo::labeled(Role::Button, true, name.clone())
                                        });
                                        if r.clicked() {
                                            let rest =
                                                keys.iter().filter(|x| *x != k).cloned().collect();
                                            todo = Some(KeyDo::Set(id.clone(), rest));
                                        }
                                    }
                                    if keys.is_empty() {
                                        ui.label(
                                            RichText::new(self.i18n.tr("bindings-no-key"))
                                                .color(secondary),
                                        );
                                    }
                                    let recording = ed.recording.as_deref() == Some(id.as_str());
                                    let text = if recording {
                                        self.i18n.tr("bindings-recording")
                                    } else {
                                        format!("+ {}", self.i18n.tr("bindings-add"))
                                    };
                                    let name = self.i18n.tr_args(
                                        "bindings-add-name",
                                        &[("command", label.clone().into())],
                                    );
                                    let r = ui.add(egui::Button::new(text).selected(recording));
                                    r.widget_info(|| {
                                        WidgetInfo::selected(
                                            Role::Button,
                                            true,
                                            recording,
                                            name.clone(),
                                        )
                                    });
                                    if r.clicked() && !recording {
                                        todo = Some(KeyDo::Record(id.clone()));
                                    }
                                });
                                if self.session.keys.contains_key(&id) {
                                    let name = self.i18n.tr_args(
                                        "bindings-reset-name",
                                        &[("command", label.clone().into())],
                                    );
                                    let r = ui
                                        .small_button(self.i18n.tr("bindings-reset"))
                                        .on_hover_text(&name);
                                    r.widget_info(|| {
                                        WidgetInfo::labeled(Role::Button, true, name.clone())
                                    });
                                    if r.clicked() {
                                        todo = Some(KeyDo::Reset(id.clone()));
                                    }
                                } else {
                                    ui.label("");
                                }
                                ui.end_row();
                            }
                        });
                }
            });
        let Some(todo) = todo else { return };
        let clash = self.bindings.as_ref().and_then(|e| e.clash.clone());
        if let Some(e) = self.bindings.as_mut() {
            e.note = None;
            e.clash = None;
        }
        match todo {
            KeyDo::Set(id, keys) => self.set_command_keys(&id, keys, false),
            KeyDo::Record(id) => {
                if let Some(e) = self.bindings.as_mut() {
                    e.recording = Some(id);
                }
            }
            KeyDo::Reset(id) => {
                self.call("settings.set_keys", json!({"command": id, "reset": true}));
                self.rebuild_commands();
            }
            KeyDo::ResetAll => {
                self.call("settings.set_keys", json!({"reset": true}));
                self.rebuild_commands();
            }
            KeyDo::Replace => {
                if let Some(c) = clash {
                    self.set_command_keys(&c.command, c.keys, true);
                }
            }
            KeyDo::Cancel => {}
        }
    }

    /// A binding as words: `Cmd + Left drag`, `Wheel`.
    pub(crate) fn binding_text(&self, b: &Binding) -> String {
        let mut parts = Vec::new();
        for (on, key) in [
            (b.mods.cmd, "mod-cmd"),
            (b.mods.ctrl, "mod-ctrl"),
            (b.mods.alt, "mod-alt"),
            (b.mods.shift, "mod-shift"),
        ] {
            if on {
                parts.push(self.i18n.tr(key));
            }
        }
        let input = match (b.gesture, b.button) {
            (Gesture::Scroll, _) => self.i18n.tr("input-scroll"),
            (Gesture::Pinch, _) => self.i18n.tr("input-pinch"),
            (g, button) => format!(
                "{} {}",
                self.i18n.tr(&format!(
                    "button-{}",
                    button.unwrap_or(Button::Primary).key()
                )),
                self.i18n.tr(&format!("gesture-{}", g.key()))
            ),
        };
        parts.push(input);
        parts.join(" + ")
    }

    /// Apply a changed binding table, or stop at the warning when the
    /// binding in `row` takes another one's input.
    fn apply_mouse(&mut self, draft: Vec<Binding>, row: usize) {
        let same = |a: &Binding, c: &Binding| {
            a.gesture == c.gesture && a.button == c.button && a.mods == c.mods
        };
        let other = draft
            .get(row)
            .and_then(|b| (0..draft.len()).find(|&j| j != row && same(b, &draft[j])));
        if let Some(other) = other {
            if let Some(e) = self.bindings.as_mut() {
                e.mouse_clash = Some(MouseClash { draft, other });
            }
            return;
        }
        if let Some(e) = self.bindings.as_mut() {
            e.mouse_clash = None;
        }
        self.call("settings.set_bindings", json!({ "bindings": draft }));
    }

    #[cfg(test)]
    pub(crate) fn apply_mouse_for_test(&mut self, draft: Vec<Binding>, row: usize) {
        self.apply_mouse(draft, row);
    }

    fn mouse_tab(&mut self, ui: &mut Ui) {
        let Some(ed) = self.bindings.clone() else {
            return;
        };
        let nav: Navigation = self.session.navigation.clone();
        if let Some(c) = &ed.mouse_clash {
            let other = c.draft[c.other];
            ui.group(|ui| {
                self.warn(
                    ui,
                    &self.i18n.tr_args(
                        "mouse-clash",
                        &[
                            ("input", self.binding_text(&other).into()),
                            (
                                "action",
                                self.i18n
                                    .tr(&format!("action-{}", other.action.key()))
                                    .into(),
                            ),
                        ],
                    ),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button(self.i18n.tr("bindings-replace"))
                        .on_hover_text(self.i18n.tr("bindings-replace-tip"))
                        .clicked()
                    {
                        let mut d = c.draft.clone();
                        d.remove(c.other);
                        if let Some(e) = self.bindings.as_mut() {
                            e.mouse_clash = None;
                        }
                        self.call("settings.set_bindings", json!({ "bindings": d }));
                    }
                    if ui.button(self.i18n.tr("bindings-cancel")).clicked()
                        && let Some(e) = self.bindings.as_mut()
                    {
                        e.mouse_clash = None;
                    }
                });
            });
        }
        ui.label(RichText::new(self.i18n.tr("mouse-preset")).strong());
        ui.horizontal_wrapped(|ui| {
            for p in PRESETS {
                let label = self.i18n.tr(&format!("preset-{p}"));
                if ui.radio(nav.preset == *p, label).clicked() && nav.preset != *p {
                    if let Some(e) = self.bindings.as_mut() {
                        e.mouse_clash = None;
                    }
                    self.call("settings.set_bindings", json!({ "preset": p }));
                }
            }
            if nav.preset == "custom" {
                let _ = ui.radio(true, self.i18n.tr("preset-custom"));
            }
        });
        ui.horizontal(|ui| {
            let mut z = nav.zoom_to_cursor;
            if ui
                .checkbox(&mut z, self.i18n.tr("settings-zoom-to-cursor"))
                .changed()
            {
                self.call("settings.set_bindings", json!({ "zoom_to_cursor": z }));
            }
            let mut inv = nav.invert_zoom;
            if ui
                .checkbox(&mut inv, self.i18n.tr("settings-invert-zoom"))
                .changed()
            {
                self.call("settings.set_bindings", json!({ "invert_zoom": inv }));
            }
        });
        for (key, field, now, edit, range, speed) in [
            (
                "mouse-orbit-speed",
                "orbit_speed",
                nav.orbit_speed,
                ed.orbit_edit,
                0.01..=5.0,
                0.01,
            ),
            (
                "mouse-zoom-speed",
                "zoom_speed",
                nav.zoom_speed,
                ed.zoom_edit,
                1e-4..=0.1,
                1e-4,
            ),
        ] {
            let name = self.i18n.tr(key);
            ui.horizontal(|ui| {
                ui.label(&name);
                let mut v = edit.unwrap_or(now);
                let r = ui.add(
                    egui::DragValue::new(&mut v)
                        .range(range)
                        .speed(speed)
                        .max_decimals(4),
                );
                r.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name.clone()));
                if r.changed()
                    && let Some(e) = self.bindings.as_mut()
                {
                    *if field == "orbit_speed" {
                        &mut e.orbit_edit
                    } else {
                        &mut e.zoom_edit
                    } = Some(v);
                }
                let done = r.drag_stopped()
                    || (r.changed() && !r.dragged() && !r.has_focus())
                    || r.lost_focus();
                if done && (v - now).abs() > 0.0 {
                    self.call("settings.set_bindings", json!({ field: v }));
                }
                if done && let Some(e) = self.bindings.as_mut() {
                    *if field == "orbit_speed" {
                        &mut e.orbit_edit
                    } else {
                        &mut e.zoom_edit
                    } = None;
                }
            });
        }
        ui.add_space(6.0);
        ui.label(RichText::new(self.i18n.tr("mouse-bindings")).strong());
        let mut change: Option<(Vec<Binding>, usize)> = None;
        let mac = cfg!(target_os = "macos");
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height(ui.available_height() - 60.0)
            .show(ui, |ui| {
                egui::Grid::new("bind-mouse")
                    .num_columns(4)
                    .striped(true)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        for h in ["mouse-action", "mouse-input", "mouse-mods"] {
                            ui.label(RichText::new(self.i18n.tr(h)).small());
                        }
                        ui.label("");
                        ui.end_row();
                        for (i, b) in nav.bindings.iter().enumerate() {
                            let mut nb = *b;
                            let input = self.binding_text(b);
                            let action = self.i18n.tr(&format!("action-{}", b.action.key()));
                            let named = |key: &str, value: &str| {
                                self.i18n.tr_args(
                                    key,
                                    &[
                                        ("binding", input.clone().into()),
                                        ("value", value.to_string().into()),
                                    ],
                                )
                            };
                            let action_name = named("mouse-action-name", &action);
                            egui::ComboBox::from_id_salt(("bind-action", i))
                                .selected_text(&action)
                                .show_ui(ui, |ui| {
                                    for a in Action::ALL {
                                        let t = self.i18n.tr(&format!("action-{}", a.key()));
                                        ui.selectable_value(&mut nb.action, a, t);
                                    }
                                })
                                .response
                                .widget_info(|| {
                                    WidgetInfo::labeled(Role::ComboBox, true, action_name.clone())
                                });
                            ui.horizontal(|ui| {
                                if b.gesture.has_button() {
                                    let bt = b.button.unwrap_or(Button::Primary);
                                    let shown = self.i18n.tr(&format!("button-{}", bt.key()));
                                    let name = named("mouse-button-name", &shown);
                                    egui::ComboBox::from_id_salt(("bind-button", i))
                                        .selected_text(&shown)
                                        .width(70.0)
                                        .show_ui(ui, |ui| {
                                            for x in Button::ALL {
                                                let t =
                                                    self.i18n.tr(&format!("button-{}", x.key()));
                                                ui.selectable_value(&mut nb.button, Some(x), t);
                                            }
                                        })
                                        .response
                                        .widget_info(|| {
                                            WidgetInfo::labeled(Role::ComboBox, true, name.clone())
                                        });
                                }
                                let shown = self.i18n.tr(&format!("gesture-{}", b.gesture.key()));
                                let name = named("mouse-gesture-name", &shown);
                                egui::ComboBox::from_id_salt(("bind-gesture", i))
                                    .selected_text(&shown)
                                    .width(90.0)
                                    .show_ui(ui, |ui| {
                                        for g in Gesture::ALL {
                                            let t = self.i18n.tr(&format!("gesture-{}", g.key()));
                                            ui.selectable_value(&mut nb.gesture, g, t);
                                        }
                                    })
                                    .response
                                    .widget_info(|| {
                                        WidgetInfo::labeled(Role::ComboBox, true, name.clone())
                                    });
                            });
                            ui.horizontal(|ui| {
                                let mut m: Mods = b.mods;
                                for (on, key, shown) in [
                                    (&mut m.cmd, "mod-cmd", mac || b.mods.cmd),
                                    (&mut m.ctrl, "mod-ctrl", true),
                                    (&mut m.alt, "mod-alt", true),
                                    (&mut m.shift, "mod-shift", true),
                                ] {
                                    if shown {
                                        let t = self.i18n.tr(key);
                                        let r = ui.selectable_label(*on, &t);
                                        let name = named("mouse-mod-name", &t);
                                        r.widget_info(|| {
                                            WidgetInfo::selected(
                                                Role::Button,
                                                true,
                                                *on,
                                                name.clone(),
                                            )
                                        });
                                        if r.clicked() {
                                            *on = !*on;
                                        }
                                    }
                                }
                                nb.mods = m;
                            });
                            let name = self.i18n.tr_args(
                                "mouse-remove-name",
                                &[("binding", self.binding_text(b).into())],
                            );
                            let r = ui.small_button("×").on_hover_text(&name);
                            r.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.clone()));
                            if r.clicked() {
                                let mut d = nav.bindings.clone();
                                d.remove(i);
                                change = Some((d, usize::MAX));
                            }
                            ui.end_row();
                            // A wheel or pinch has no button; a click
                            // or drag needs one.
                            nb.button = if nb.gesture.has_button() {
                                nb.button.or(Some(Button::Primary))
                            } else {
                                None
                            };
                            if nb != *b && change.is_none() {
                                let mut d = nav.bindings.clone();
                                d[i] = nb;
                                change = Some((d, i));
                            }
                        }
                    });
            });
        if ui
            .button(format!("+ {}", self.i18n.tr("mouse-add")))
            .clicked()
        {
            let mut d = nav.bindings.clone();
            if let Some(b) = free_binding(&nav) {
                d.push(b);
                change = Some((d, usize::MAX));
            }
        }
        if let Some((d, row)) = change {
            self.apply_mouse(d, row);
        }
    }
}

/// A binding with an input no other binding has (a left drag with the
/// fewest keys held), for Add binding.
fn free_binding(nav: &Navigation) -> Option<Binding> {
    let mut all: Vec<Mods> = (0..16u8)
        .map(|n| Mods {
            cmd: n & 8 != 0,
            ctrl: n & 1 != 0,
            alt: n & 2 != 0,
            shift: n & 4 != 0,
        })
        .collect();
    all.sort_by_key(|m| {
        [m.cmd, m.ctrl, m.alt, m.shift]
            .iter()
            .filter(|b| **b)
            .count()
    });
    for button in Button::ALL {
        for mods in &all {
            let b = Binding {
                action: Action::Pan,
                gesture: Gesture::Drag,
                button: Some(button),
                mods: *mods,
            };
            if !nav
                .bindings
                .iter()
                .any(|c| c.gesture == b.gesture && c.button == b.button && c.mods == b.mods)
            {
                return Some(b);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: a press becomes the table's key text: Cmd (command) is
    /// Ctrl, Alt takes the key's place (Option + 7 on a German Mac types
    /// `|`), Tab and the Mac Control key are refused. Breaks if a
    /// recorded key differs from the form the window matches (it would
    /// never run). The tool test checks typed texts only.
    #[test]
    fn presses_become_key_texts() {
        let m = |command, alt, shift| egui::Modifiers {
            alt,
            ctrl: command,
            shift,
            mac_cmd: false,
            command,
        };
        assert_eq!(
            pressed_text(Key::W, Some(Key::W), m(false, false, false)),
            Ok("W".into())
        );
        assert_eq!(
            pressed_text(Key::S, Some(Key::S), m(true, false, true)),
            Ok("Ctrl+Shift+S".into())
        );
        assert_eq!(
            pressed_text(Key::Pipe, Some(Key::Num7), m(false, true, false)),
            Ok("Alt+7".into())
        );
        assert_eq!(
            pressed_text(Key::Tab, None, m(false, false, true)),
            Err(Refused::Reserved("Shift+Tab".into()))
        );
        let control = egui::Modifiers {
            ctrl: true,
            command: false,
            ..Default::default()
        };
        assert_eq!(
            pressed_text(Key::K, None, control),
            Err(Refused::MacControl)
        );
        // Every key name of the engine's list is one egui knows.
        for k in murmel_engine::keys::KEY_NAMES {
            assert!(Key::from_name(k).is_some(), "{k}");
            assert_eq!(Key::from_name(k).map(|x| x.name()), Some(*k), "{k}");
        }
    }

    /// Invariant: Add binding finds an input no binding has, starting
    /// with the fewest keys held.
    #[test]
    fn a_free_binding_is_free() {
        for p in PRESETS {
            let nav = Navigation::preset(p).unwrap();
            let b = free_binding(&nav).unwrap();
            let mut more = nav.clone();
            more.bindings.push(b);
            assert!(more.conflicts().is_empty(), "{p}");
        }
    }
}
