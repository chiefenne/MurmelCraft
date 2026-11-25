//! The menu bar: File, Edit, View, Display, Results, Tools, Window and
//! Help. The menus hold a curated choice of the commands of
//! [`crate::commands`] (the maintainer's card of 2026-11-25: the common
//! ones; every command stays in the palette and keeps its keys), so a
//! menu item, its key and its palette entry always call the same tool.
//!
//! The table is data: the window draws it, `docs/menus.md` is generated
//! from it, and the tests check that every entry names a command.

use std::fmt::Write;

use crate::commands::{Command, commands, english};

/// One entry of a menu.
#[derive(Clone, Debug)]
pub enum Entry {
    /// A command, by id.
    Command(&'static str),
    /// Every command whose id starts with this prefix, in the order of
    /// the command table (the render modes, the colour maps).
    Each(&'static str),
    /// A submenu: translation key of its name and its entries.
    Sub(&'static str, Vec<Entry>),
    /// A line between groups of entries.
    Separator,
    /// The files opened lately (`recent.list`), filled in by the window.
    Recent,
}

/// One menu of the bar.
#[derive(Clone, Debug)]
pub struct Menu {
    /// `file`, `edit`, …; the translation key of its name is `menu-<id>`.
    pub id: &'static str,
    pub entries: Vec<Entry>,
}

impl Menu {
    pub fn label_key(&self) -> String {
        format!("menu-{}", self.id)
    }
}

use Entry::{Command as C, Each, Separator as S, Sub};

/// The menus, left to right.
pub fn menus() -> Vec<Menu> {
    vec![
        Menu {
            id: "file",
            entries: vec![
                C("window.open"),
                C("window.open_folder"),
                Sub("menu-recent", vec![Entry::Recent, S, C("recent.clear")]),
                C("window.session_load"),
                S,
                C("window.series_open"),
                C("window.parcels_open"),
                C("window.charts_open"),
                S,
                C("mesh.close"),
                S,
                C("window.session_save"),
                C("window.session_save_as"),
                S,
                Sub(
                    "menu-export",
                    vec![
                        C("window.export_image"),
                        C("window.export_movie"),
                        C("window.screenshot"),
                        S,
                        C("window.charts_export"),
                    ],
                ),
                S,
                C("window.quit"),
            ],
        },
        Menu {
            id: "edit",
            entries: vec![
                C("journal.undo"),
                C("journal.redo"),
                C("window.undo_history"),
                S,
                C("window.copy_image"),
                S,
                C("window.filter"),
                C("select.clear"),
                S,
                C("window.palette"),
            ],
        },
        Menu {
            id: "view",
            entries: vec![
                C("view.fit"),
                C("view.fit:selection"),
                C("window.zoom_box"),
                C("view.zoom:in"),
                C("view.zoom:out"),
                S,
                Sub(
                    "menu-standard-views",
                    vec![
                        C("view.standard:home"),
                        S,
                        C("view.standard:front"),
                        C("view.standard:back"),
                        C("view.standard:right"),
                        C("view.standard:left"),
                        C("view.standard:top"),
                        C("view.standard:bottom"),
                        S,
                        C("view.standard:front_right_top"),
                        C("view.standard:front_left_top"),
                        C("view.standard:back_right_top"),
                        C("view.standard:back_left_top"),
                    ],
                ),
                Sub("menu-projection", vec![Each("view.projection:")]),
                Sub("menu-render-mode", vec![Each("display.render_mode:")]),
                Sub(
                    "menu-saved-views",
                    vec![
                        C("window.save_view"),
                        C("views.go:next"),
                        C("views.go:previous"),
                    ],
                ),
                S,
                Sub("menu-theme", vec![Each("settings.set_theme:")]),
                Sub("menu-view-cube", vec![Each("settings.set_view_cube:")]),
            ],
        },
        Menu {
            id: "display",
            entries: vec![
                Sub("menu-color-by", vec![Each("display.color_by:")]),
                Sub(
                    "menu-quality",
                    vec![C("display.threshold:worst"), C("display.threshold:clear")],
                ),
                S,
                C("display.visibility:hide"),
                C("display.visibility:isolate"),
                C("display.visibility:show_all"),
                S,
                Sub(
                    "menu-clip",
                    vec![
                        C("clip.set:new"),
                        C("clip.set:x"),
                        C("clip.set:y"),
                        C("clip.set:z"),
                        C("clip.set:view"),
                        S,
                        C("clip.set:flip"),
                        C("clip.set:crinkle"),
                        C("clip.set:flat"),
                        C("clip.set:toggle"),
                        C("select.item:next_clip"),
                        S,
                        C("clip.clear"),
                        C("clip.clear:all"),
                    ],
                ),
                Sub(
                    "menu-region",
                    vec![
                        C("region.set:toggle"),
                        C("region.set:cut"),
                        C("region.set:picked"),
                        C("region.set:zone"),
                        C("region.set:patch"),
                        S,
                        C("region.clear"),
                        C("region.clear:all"),
                    ],
                ),
            ],
        },
        Menu {
            id: "results",
            entries: vec![
                Sub(
                    "menu-field-style",
                    vec![
                        C("field.style:next_mode"),
                        C("field.style:lines"),
                        C("field.style:reverse"),
                        S,
                        Each("field.style:range-"),
                        S,
                        C("field.style:reset"),
                    ],
                ),
                Sub("menu-colormap", vec![Each("field.style:map-")]),
                C("window.derived"),
                C("window.calculator"),
                S,
                Sub(
                    "menu-slices",
                    vec![
                        C("slice.set:new"),
                        C("slice.set:x"),
                        C("slice.set:y"),
                        C("slice.set:z"),
                        C("slice.set:view"),
                        C("slice.set:series"),
                        C("slice.set:single"),
                        S,
                        C("slice.set:toggle"),
                        C("slice.clear"),
                        C("slice.clear:all"),
                    ],
                ),
                Sub(
                    "menu-iso",
                    vec![
                        C("iso.set:new"),
                        C("iso.set:toggle"),
                        S,
                        C("iso.clear"),
                        C("iso.clear:all"),
                    ],
                ),
                Sub(
                    "menu-arrows",
                    vec![
                        C("arrows.set:toggle"),
                        C("arrows.set:surface"),
                        C("arrows.set:in_plane"),
                        S,
                        C("arrows.set:denser"),
                        C("arrows.set:sparser"),
                        C("arrows.set:longer"),
                        C("arrows.set:shorter"),
                        S,
                        C("arrows.clear"),
                    ],
                ),
                Sub(
                    "menu-streamlines",
                    vec![
                        C("stream.set:toggle"),
                        C("stream.set:tubes"),
                        C("stream.set:arrowheads"),
                        C("stream.set:more"),
                        C("stream.set:fewer"),
                        S,
                        C("stream.clear"),
                        C("stream.clear:all"),
                    ],
                ),
                Sub(
                    "menu-parcels",
                    vec![
                        C("parcels.set:toggle"),
                        C("parcels.set:size"),
                        C("parcels.clear"),
                    ],
                ),
                S,
                Sub(
                    "menu-series",
                    vec![
                        C("series.play:toggle"),
                        Each("series.go:"),
                        S,
                        C("charts.show:toggle"),
                    ],
                ),
            ],
        },
        Menu {
            id: "tools",
            entries: vec![
                C("window.measure_distance"),
                C("window.measure_angle"),
                C("window.probe"),
                C("window.probe_history"),
                C("measure.remove:selected"),
                C("measure.remove:all"),
                S,
                Sub(
                    "menu-defaults",
                    vec![
                        C("settings.set_default:selected"),
                        C("settings.set_default:view"),
                        C("settings.set_default:field_style"),
                        S,
                        C("settings.reset_default"),
                    ],
                ),
                C("settings.set_agents"),
                S,
                C("window.edit_shortcuts"),
                C("window.settings"),
            ],
        },
        Menu {
            id: "window",
            entries: vec![
                C("window.minimize"),
                C("window.fullscreen"),
                S,
                C("window.tree"),
                C("window.properties"),
                S,
                C("window.next_region"),
                C("window.previous_region"),
            ],
        },
        Menu {
            id: "help",
            entries: vec![
                C("window.shortcuts"),
                S,
                C("window.help_tools"),
                C("window.help_agents"),
                C("window.report_issue"),
                S,
                C("window.about"),
            ],
        },
    ]
}

/// The commands an entry stands for, from `all`, in table order; empty
/// when an id names no command (a test checks there is none).
pub fn resolve<'a>(entry: &Entry, all: &'a [Command]) -> Vec<&'a Command> {
    match entry {
        Entry::Command(id) => all.iter().filter(|c| c.id == *id).collect(),
        Entry::Each(prefix) => all.iter().filter(|c| c.id.starts_with(prefix)).collect(),
        Entry::Sub(..) | Entry::Separator | Entry::Recent => Vec::new(),
    }
}

/// `docs/menus.md`, printed by `murmel menus --markdown`.
pub fn menus_markdown() -> String {
    let mut out = String::from(
        "# Menus\n\nGenerated by `cargo run -p murmel-cli -- menus --markdown > docs/menus.md`\n\
         from `crates/murmel-tools/src/menus.rs`; a test fails when it is out of date.\n\
         The menus hold the common commands; every command, in a menu or not, is also\n\
         in the command palette (Ctrl + K) and keeps its keys ([keys.md](keys.md)). On macOS\n\
         `Ctrl` is the Command key.\n",
    );
    let all = commands();
    fn entries(out: &mut String, list: &[Entry], depth: usize, all: &[Command]) {
        let pad = "  ".repeat(depth);
        for e in list {
            match e {
                Entry::Separator => {}
                Entry::Recent => {
                    let _ = writeln!(out, "{pad}- *the files opened lately* → `recent.list`");
                }
                Entry::Sub(key, sub) => {
                    let _ = writeln!(out, "{pad}- **{}** ▸", english(key));
                    entries(out, sub, depth + 1, all);
                }
                _ => {
                    for c in resolve(e, all) {
                        let keys = c
                            .keys
                            .iter()
                            .map(|k| format!("`{}`", k.replace('+', " + ")))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let tool = c.tool().map(|t| format!(" → `{t}`")).unwrap_or_default();
                        let keys = if keys.is_empty() {
                            String::new()
                        } else {
                            format!(" {keys}")
                        };
                        let _ = writeln!(out, "{pad}- {}{keys}{tool}", english(&c.label_key));
                    }
                }
            }
        }
    }
    for m in menus() {
        let _ = writeln!(out, "\n## {}\n", english(&m.label_key()));
        entries(&mut out, &m.entries, 0, &all);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(list: &[Entry], f: &mut impl FnMut(&Entry)) {
        for e in list {
            f(e);
            if let Entry::Sub(_, sub) = e {
                walk(sub, f);
            }
        }
    }

    /// Invariant: every menu entry names at least one command, no command
    /// sits in two places, and every menu and submenu name is translated.
    /// Breaks when a command is renamed or removed and its menu entry
    /// stays (the item would vanish from the menu without a word), or a
    /// command is put in twice. The window test only opens the menus.
    #[test]
    fn every_entry_names_a_command_once() {
        let all = commands();
        let mut seen: Vec<String> = Vec::new();
        for m in menus() {
            assert_ne!(english(&m.label_key()), m.label_key(), "{}", m.id);
            walk(&m.entries, &mut |e| match e {
                Entry::Sub(key, sub) => {
                    assert_ne!(english(key), *key, "{key} untranslated");
                    assert!(!sub.is_empty(), "{key} is empty");
                }
                Entry::Separator | Entry::Recent => {}
                _ => {
                    let found = resolve(e, &all);
                    assert!(!found.is_empty(), "{e:?} names no command");
                    for c in found {
                        assert!(!seen.contains(&c.id), "{} is in two places", c.id);
                        seen.push(c.id.clone());
                    }
                }
            });
        }
        assert!(seen.len() > 100, "{}", seen.len());
    }

    /// Invariant: docs/menus.md is what `murmel menus --markdown` prints.
    #[test]
    fn menus_md_is_up_to_date() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let doc = std::fs::read_to_string(root.join("docs/menus.md")).unwrap_or_default();
        assert!(
            doc == menus_markdown(),
            "docs/menus.md is out of date; run `cargo run -p murmel-cli -- menus --markdown > docs/menus.md`"
        );
    }
}
