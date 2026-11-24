//! The keyboard map, the command palette (Ctrl + K) and the shortcut
//! sheet (F1). All three read the command table of `murmel-tools`, so a
//! key, a palette entry and the sheet always agree, and each ends in a
//! call of a registered tool.

use egui::{Key, KeyboardShortcut, Modifiers, RichText, Role, WidgetInfo};
use murmel_tools::commands::{Command, Run};

use crate::app::MurmelApp;
use crate::style::color;

/// A key of the table (`Ctrl+Alt+1`) as an egui shortcut; `Ctrl` is the
/// Command key on macOS.
pub fn shortcut(key: &str) -> Option<KeyboardShortcut> {
    let mut parts: Vec<&str> = key.split('+').collect();
    let k = Key::from_name(parts.pop()?)?;
    let mut m = Modifiers::NONE;
    for p in parts {
        m |= match p {
            "Ctrl" => Modifiers::COMMAND,
            "Alt" => Modifiers::ALT,
            "Shift" => Modifiers::SHIFT,
            _ => return None,
        };
    }
    Some(KeyboardShortcut::new(m, k))
}

/// Take a press of `s` from this frame's input. A shortcut with Alt
/// (Option on macOS) also matches by the key's place on the keyboard:
/// Option + a digit types a character on a Mac (Option + 7 is `|` on a
/// German layout), and egui reports that character as the key, so a
/// logical match alone missed the views (docs/pitfalls.md).
pub fn consume(i: &mut egui::InputState, s: &KeyboardShortcut) -> bool {
    if !s.modifiers.alt {
        return i.consume_shortcut(s);
    }
    let mut hit = false;
    i.events.retain(|e| {
        let m = matches!(
            e,
            egui::Event::Key { key, physical_key, pressed: true, modifiers, .. }
                if (*key == s.logical_key || *physical_key == Some(s.logical_key))
                    && modifiers.matches_logically(s.modifiers)
        );
        hit |= m;
        !m
    });
    hit
}

/// A key as shown to the user: `Ctrl + K`, `Cmd + K` on macOS.
pub fn key_text(key: &str) -> String {
    let (ctrl, alt) = crate::i18n::modifier_names();
    key.split('+')
        .map(|p| match p {
            "Ctrl" => ctrl,
            "Alt" => alt,
            "Plus" => "+",
            "Minus" => "−",
            "Equals" => "=",
            "Escape" => "Esc",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// The keys a focused widget uses itself: the arrows (with or without
/// Shift) and Esc.
fn widget_key(s: &KeyboardShortcut) -> bool {
    let plain = !s.modifiers.command && !s.modifiers.alt;
    plain
        && matches!(
            s.logical_key,
            Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::Escape
        )
}

/// The palette's state while it is open.
#[derive(Default)]
pub struct Palette {
    pub query: String,
    pub selected: usize,
    pub focus: bool,
}

/// Whether every word of `query` occurs in `text` (case ignored).
fn matches(query: &str, text: &str) -> bool {
    let t = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|w| t.contains(w))
}

impl MurmelApp {
    /// Run a command: call its tool, or do its window action.
    pub(crate) fn run_command(&mut self, ctx: &egui::Context, c: &Command) {
        if let Run::Window(_) = c.run {
            match c.id.as_str() {
                "window.open" => self.open_dialog(),
                "window.open_folder" => self.open_folder_dialog(),
                "window.screenshot" => self.screenshot_dialog(),
                "window.export_image" => self.export.open = true,
                "window.export_movie" => self.movie.open = true,
                "window.session_save" => self.save_session(false),
                "window.session_save_as" => self.save_session(true),
                "window.session_load" => self.load_session_dialog(),
                "window.palette" => self.open_palette(String::new()),
                "window.shortcuts" => self.shortcuts_open = !self.shortcuts_open,
                "window.tree" => self.show_tree = !self.show_tree,
                "window.properties" => self.show_props = !self.show_props,
                "window.filter" => {
                    self.show_tree = true;
                    self.focus_filter = true;
                }
                "window.settings" => self.settings_open = true,
                "window.save_view" => self.save_view(),
                "window.measure_distance" => self.toggle_measuring(crate::measure::Mode::Distance),
                "window.measure_angle" => self.toggle_measuring(crate::measure::Mode::Angle),
                "window.probe" => self.toggle_measuring(crate::measure::Mode::Probe),
                "window.next_region" => self.cycle_region(ctx, true),
                "window.previous_region" => self.cycle_region(ctx, false),
                "window.color_by" => {
                    let q = self.i18n.tr("toolbar-color-by");
                    self.open_palette(q);
                }
                _ => {}
            }
            ctx.request_repaint();
            return;
        }
        if let Some((tool, input)) = c.input(&self.session) {
            self.call(tool, input);
        }
    }

    pub(crate) fn open_palette(&mut self, query: String) {
        self.palette = Some(Palette {
            query,
            selected: 0,
            focus: true,
        });
    }

    /// The keys of the table, more modifiers first: egui matches
    /// modifiers logically, so a plain H would also take Alt + H
    /// (docs/pitfalls.md).
    pub(crate) fn keys(&mut self, ctx: &egui::Context) {
        let mut keyed: Vec<(KeyboardShortcut, Command)> = Vec::new();
        for c in &self.commands {
            for k in &c.keys {
                if let Some(s) = shortcut(k) {
                    keyed.push((s, c.clone()));
                }
            }
        }
        keyed.sort_by_key(|(s, _)| {
            std::cmp::Reverse(
                [s.modifiers.command, s.modifiers.alt, s.modifiers.shift]
                    .iter()
                    .filter(|b| **b)
                    .count(),
            )
        });
        let typing = ctx.text_edit_focused();
        // Esc stops a mesh being read before anything else.
        if self.opening.is_some()
            && !typing
            && self.palette.is_none()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.cancel_opening();
        }
        // Esc stops measuring before it clears the selection.
        if self.measuring.is_some()
            && !typing
            && self.palette.is_none()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.measuring = None;
            self.snap_hover = None;
        }
        // A widget the keyboard reached keeps the arrows (tree rows,
        // sliders, the next button) and Esc (leave it); the viewport and
        // a clicked button leave them to the view (docs/pitfalls.md).
        let widget_keys = self.focus.widget_keys();
        for (s, c) in keyed {
            // Only a text field keeps the keys; a clicked button must not
            // swallow F, H or Esc (docs/pitfalls.md). The palette and the
            // filter work from a text field too.
            let always = matches!(
                c.id.as_str(),
                "window.palette"
                    | "window.filter"
                    | "window.next_region"
                    | "window.previous_region"
            );
            if typing && !always {
                continue;
            }
            if widget_keys && widget_key(&s) {
                continue;
            }
            if self.palette.is_some() && c.id != "window.palette" {
                continue;
            }
            if ctx.input_mut(|i| consume(i, &s)) {
                if c.id == "window.palette" && self.palette.is_some() {
                    self.palette = None;
                } else {
                    self.run_command(ctx, &c);
                }
            }
        }
    }

    /// The command palette: type to filter, arrows to choose, Enter to
    /// run, Esc to close. Commands that do not apply now (nothing
    /// selected) are left out.
    pub(crate) fn palette_window(&mut self, ctx: &egui::Context) {
        let Some(mut pal) = self.palette.take() else {
            return;
        };
        let p = self.palette(ctx).clone();
        let list: Vec<(Command, String)> = self
            .commands
            .iter()
            .filter(|c| !matches!(c.run, Run::Dynamic(..)) || c.input(&self.session).is_some())
            .map(|c| (c.clone(), self.i18n.tr(&c.label_key)))
            .filter(|(c, label)| {
                let group = self.i18n.tr(&format!("group-{}", c.group));
                matches(&pal.query, &format!("{label} {group} {}", c.id))
            })
            .collect();
        let (up, down, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if down {
            pal.selected = (pal.selected + 1).min(list.len().saturating_sub(1));
        }
        if up {
            pal.selected = pal.selected.saturating_sub(1);
        }
        pal.selected = pal.selected.min(list.len().saturating_sub(1));
        let mut run: Option<Command> = None;
        if enter {
            run = list.get(pal.selected).map(|(c, _)| c.clone());
        }
        let mut open = !esc;
        let screen = ctx.content_rect();
        let width = 560.0f32.min(screen.width() - 32.0);
        egui::Area::new(egui::Id::unique("command-palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(
                screen.center().x - width / 2.0,
                screen.top() + 72.0,
            ))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(width);
                    let hint = self.i18n.tr("palette-placeholder");
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut pal.query)
                            .hint_text(hint)
                            .desired_width(f32::INFINITY),
                    );
                    let title = self.i18n.tr("palette-title");
                    r.widget_info(|| WidgetInfo::labeled(Role::SearchInput, true, title.clone()));
                    if pal.focus {
                        r.request_focus();
                        pal.focus = false;
                    }
                    if r.changed() {
                        pal.selected = 0;
                    }
                    ui.add_space(4.0);
                    if list.is_empty() {
                        ui.label(
                            RichText::new(self.i18n.tr("palette-empty"))
                                .color(color(p.text_secondary)),
                        );
                    }
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            for (i, (c, label)) in list.iter().enumerate() {
                                let on = i == pal.selected;
                                let keys = c
                                    .keys
                                    .iter()
                                    .map(|k| key_text(k))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                let row = ui.add(
                                    egui::Button::selectable(on, label.as_str())
                                        .shortcut_text(RichText::new(keys).monospace())
                                        .min_size(egui::vec2(width, 24.0)),
                                );
                                if on && (up || down) {
                                    row.scroll_to_me(None);
                                }
                                if row.clicked() {
                                    run = Some(c.clone());
                                }
                            }
                        });
                });
            });
        if let Some(c) = run {
            open = false;
            self.run_command(ctx, &c);
        }
        if open && self.palette.is_none() {
            self.palette = Some(pal);
        }
    }

    /// The shortcut sheet (F1): every keyed command by group.
    pub(crate) fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.shortcuts_open {
            return;
        }
        let p = self.palette(ctx).clone();
        let mut open = true;
        egui::Window::new(self.i18n.tr("shortcuts-title"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(640.0)
            .show(ctx, |ui| {
                let groups: Vec<&str> = murmel_tools::commands::GROUPS
                    .iter()
                    .copied()
                    .filter(|g| {
                        self.commands
                            .iter()
                            .any(|c| c.group == *g && !c.keys.is_empty())
                    })
                    .collect();
                ui.columns(2, |cols| {
                    for (n, g) in groups.iter().enumerate() {
                        let ui = &mut cols[if n < 1 { 0 } else { 1 }];
                        ui.label(
                            RichText::new(self.i18n.tr(&format!("group-{g}")).to_uppercase())
                                .small()
                                .color(color(p.text_secondary)),
                        );
                        egui::Grid::new(format!("keys-{g}"))
                            .num_columns(2)
                            .spacing([12.0, 4.0])
                            .show(ui, |ui| {
                                for c in self.commands.iter().filter(|c| c.group == *g) {
                                    if c.keys.is_empty() {
                                        continue;
                                    }
                                    let keys = c
                                        .keys
                                        .iter()
                                        .map(|k| key_text(k))
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    ui.label(RichText::new(keys).monospace());
                                    ui.label(self.i18n.tr(&c.label_key));
                                    ui.end_row();
                                }
                            });
                        ui.add_space(8.0);
                    }
                });
                ui.label(
                    RichText::new(self.i18n.tr("shortcuts-palette-only"))
                        .small()
                        .color(color(p.text_secondary)),
                );
            });
        self.shortcuts_open &= open;
    }

    pub(crate) fn screenshot_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title(self.i18n.tr("screenshot-title"))
            .add_filter("PNG", &["png"])
            .set_file_name("murmelcraft.png")
            .save_file();
        if let Some(path) = picked {
            let [w, h] = self.session.viewport;
            self.call(
                "view.screenshot",
                serde_json::json!({"path": path, "width": w.round() as u32, "height": h.round() as u32}),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use murmel_tools::commands::commands;

    use super::*;

    /// Invariant: every key of the command table parses into an egui
    /// shortcut, so no key in the sheet is dead in the window.
    #[test]
    fn every_key_parses() {
        for c in commands() {
            for k in &c.keys {
                assert!(shortcut(k).is_some(), "{}: {k}", c.id);
            }
        }
        let s = shortcut("Ctrl+Alt+1").unwrap();
        assert!(s.modifiers.command && s.modifiers.alt && s.logical_key == Key::Num1);
    }

    /// Invariant: Option + 7 on a Mac keyboard whose layout types a
    /// character there (`|` on a German layout: egui's key is Pipe, the
    /// physical key Num7) runs Alt + 7, while a plain 7 shortcut does not
    /// take it. Breaks if the views match only the logical key again
    /// (the maintainer's Mac test, 2026-11-24); no other test sends a key
    /// whose logical and physical keys differ.
    #[test]
    fn option_digit_matches_by_place() {
        let press = |key, physical| egui::Event::Key {
            key,
            physical_key: Some(physical),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::ALT,
        };
        let run = |event: egui::Event, s: KeyboardShortcut| {
            let ctx = egui::Context::default();
            let mut hit = false;
            let raw = egui::RawInput {
                events: vec![event],
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                hit = ui.ctx().input_mut(|i| consume(i, &s));
            });
            out.textures_delta.clear();
            hit
        };
        let alt7 = shortcut("Alt+7").unwrap();
        assert!(run(press(Key::Pipe, Key::Num7), alt7));
        assert!(run(press(Key::Num7, Key::Num7), alt7), "US layout");
        assert!(!run(press(Key::Pipe, Key::Num7), shortcut("7").unwrap()));
        assert!(!run(press(Key::Pipe, Key::Num3), alt7));
    }

    #[test]
    fn palette_matching() {
        assert!(matches("fro top", "Front top view"));
        assert!(!matches("back", "Front top"));
        assert!(matches("", "anything"));
    }
}
