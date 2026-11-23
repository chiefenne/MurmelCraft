//! The commands of the window: the keyboard map and the command palette
//! (UI spec, "Keyboard map and accessibility"). Each command is a call of
//! a registered tool with a fixed input, or with an input read from the
//! session (the selection, the next projection); the few that only change
//! the window (panels, the palette itself, file dialogs) are marked
//! `Window` and name the tool they end in, if any.
//!
//! The table is data, so the window, the shortcut sheet, `docs/keys.md`
//! and the tests all read the same keys.

use murmel_engine::{ColorBy, Item, Projection, RenderMode, Session, StandardView};
use serde_json::{Value, json};

/// What a command does.
#[derive(Clone, Debug)]
pub enum Run {
    /// Call `tool` with a fixed input.
    Tool(&'static str, Value),
    /// Call `tool` with an input read from the session; `None` when the
    /// command does not apply now (nothing selected, no mesh).
    Dynamic(&'static str, fn(&Session) -> Option<Value>),
    /// A front-end action, named by the id after `window.`; it calls the
    /// tool given, if any, once the window has what it needs (a path from
    /// a file dialog).
    Window(Option<&'static str>),
}

/// One command of the keyboard map and the palette.
#[derive(Clone, Debug)]
pub struct Command {
    /// `area.verb` or `area.verb:argument`; window actions are `window.*`.
    pub id: String,
    /// Translation key of the name shown in the palette and the sheet.
    pub label_key: String,
    /// Palette group (translation key `group-<group>`).
    pub group: &'static str,
    /// Keys, like `Ctrl+K`, `Alt+1`, `Shift+Left` or `F1`. `Ctrl` is the
    /// Command key on macOS. Empty for palette-only commands.
    pub keys: Vec<&'static str>,
    pub run: Run,
}

impl Command {
    /// The tool this command calls in the end, if any.
    pub fn tool(&self) -> Option<&'static str> {
        match &self.run {
            Run::Tool(t, _) | Run::Dynamic(t, _) => Some(t),
            Run::Window(t) => *t,
        }
    }

    /// The input to call the tool with now; `None` for a window command
    /// or one that does not apply.
    pub fn input(&self, s: &Session) -> Option<(&'static str, Value)> {
        match &self.run {
            Run::Tool(t, v) => Some((t, v.clone())),
            Run::Dynamic(t, f) => f(s).map(|v| (*t, v)),
            Run::Window(_) => None,
        }
    }
}

/// The palette groups in the order of the shortcut sheet; every command's
/// group is one of them (a test checks).
pub const GROUPS: &[&str] = &[
    "view",
    "file",
    "window",
    "display",
    "clip",
    "selection",
    "settings",
];

/// The views with keys (the numeric keypad's layout: 1 front, 3 right,
/// 7 top; Ctrl for the opposite view). egui cannot tell the keypad's
/// digits from the top row's, so the views take Alt (decided 2026-11-23)
/// and the plain digits stay the render modes.
const VIEW_KEYS: [(&str, &str); 6] = [
    ("front", "Alt+1"),
    ("back", "Ctrl+Alt+1"),
    ("right", "Alt+3"),
    ("left", "Ctrl+Alt+3"),
    ("top", "Alt+7"),
    ("bottom", "Ctrl+Alt+7"),
];

/// The ids of the selected items (the mesh left out); `None` when there
/// are none.
fn selected(s: &Session) -> Option<Vec<String>> {
    let m = s.mesh.as_ref()?;
    let ids: Vec<String> = s
        .display
        .selection
        .iter()
        .filter(|&&i| i != Item::Mesh)
        .map(|i| i.id(&m.mesh))
        .collect();
    (!ids.is_empty()).then_some(ids)
}

/// Move the clip plane by `share` of its range (towards the side cut
/// away for a positive share, so more is cut).
fn clip_step(s: &Session, share: f64) -> Option<Value> {
    let c = s.display.clip?;
    let m = s.mesh.as_ref()?;
    let (off, [lo, hi]) = c.offset(&m.mesh.bounds());
    Some(json!({"offset": (off - share * (hi - lo)).clamp(lo, hi)}))
}

fn cmd(
    id: impl Into<String>,
    label_key: impl Into<String>,
    group: &'static str,
    keys: &[&'static str],
    run: Run,
) -> Command {
    Command {
        id: id.into(),
        label_key: label_key.into(),
        group,
        keys: keys.to_vec(),
        run,
    }
}

/// Every command, in the order of the shortcut sheet.
pub fn commands() -> Vec<Command> {
    use Run::{Dynamic, Tool, Window};
    let mut v = vec![
        cmd(
            "window.open",
            "cmd-open",
            "file",
            &["Ctrl+O"],
            Window(Some("mesh.open")),
        ),
        cmd(
            "mesh.close",
            "cmd-close",
            "file",
            &[],
            Tool("mesh.close", json!({})),
        ),
        cmd(
            "window.screenshot",
            "cmd-screenshot",
            "file",
            &[],
            Window(Some("view.screenshot")),
        ),
        cmd(
            "window.palette",
            "cmd-palette",
            "window",
            &["Ctrl+K"],
            Window(None),
        ),
        cmd(
            "window.shortcuts",
            "cmd-shortcuts",
            "window",
            &["F1"],
            Window(None),
        ),
        cmd("window.tree", "cmd-tree", "window", &["T"], Window(None)),
        cmd(
            "window.properties",
            "cmd-properties",
            "window",
            &["P"],
            Window(None),
        ),
        cmd(
            "window.filter",
            "cmd-filter",
            "window",
            &["Ctrl+F"],
            Window(None),
        ),
        cmd(
            "window.settings",
            "cmd-settings",
            "window",
            &[],
            Window(None),
        ),
        cmd(
            "view.fit",
            "cmd-fit",
            "view",
            &["F"],
            Tool("view.fit", json!({})),
        ),
        cmd(
            "view.standard:home",
            "cmd-home",
            "view",
            &["Home"],
            Tool("view.standard", json!({"view": "home"})),
        ),
    ];
    for view in StandardView::cube() {
        let name = view.name();
        let keys: Vec<&'static str> = VIEW_KEYS
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, k)| *k)
            .collect();
        v.push(cmd(
            format!("view.standard:{name}"),
            format!("view-{name}"),
            "view",
            &keys,
            Tool("view.standard", json!({ "view": name })),
        ));
    }
    v.push(cmd(
        "view.projection:next",
        "cmd-projection-next",
        "view",
        &["5"],
        Dynamic("view.set_camera", |s| {
            Some(json!({"projection": s.camera.projection.next()}))
        }),
    ));
    for p in [
        Projection::PerspectiveOrthoFaces,
        Projection::Perspective,
        Projection::Orthographic,
    ] {
        v.push(cmd(
            format!("view.projection:{}", p.key()),
            format!("projection-{}", p.key()),
            "view",
            &[],
            Tool("view.set_camera", json!({ "projection": p })),
        ));
    }
    for (dir, key, yaw, pitch) in [
        ("left", "Left", 15.0, 0.0),
        ("right", "Right", -15.0, 0.0),
        ("up", "Up", 0.0, -15.0),
        ("down", "Down", 0.0, 15.0),
    ] {
        v.push(cmd(
            format!("view.orbit:{dir}"),
            format!("cmd-orbit-{dir}"),
            "view",
            &[key],
            Tool("view.orbit", json!({"yaw_deg": yaw, "pitch_deg": pitch})),
        ));
    }
    for (dir, key, f) in [
        (
            "left",
            "Shift+Left",
            (|s| Some(json!({"dx_px": -0.05 * s.viewport[1]}))) as fn(&Session) -> Option<Value>,
        ),
        ("right", "Shift+Right", |s| {
            Some(json!({"dx_px": 0.05 * s.viewport[1]}))
        }),
        ("up", "Shift+Up", |s| {
            Some(json!({"dy_px": -0.05 * s.viewport[1]}))
        }),
        ("down", "Shift+Down", |s| {
            Some(json!({"dy_px": 0.05 * s.viewport[1]}))
        }),
    ] {
        v.push(cmd(
            format!("view.pan:{dir}"),
            format!("cmd-pan-{dir}"),
            "view",
            &[key],
            Dynamic("view.pan", f),
        ));
    }
    v.push(cmd(
        "view.zoom:in",
        "cmd-zoom-in",
        "view",
        &["Plus", "Equals"],
        Tool("view.zoom", json!({"factor": 0.8})),
    ));
    v.push(cmd(
        "view.zoom:out",
        "cmd-zoom-out",
        "view",
        &["Minus"],
        Tool("view.zoom", json!({"factor": 1.25})),
    ));
    for (axis, sign, deg) in [
        ("z", "plus", 90.0),
        ("z", "minus", -90.0),
        ("x", "plus", 90.0),
        ("x", "minus", -90.0),
        ("y", "plus", 90.0),
        ("y", "minus", -90.0),
    ] {
        v.push(cmd(
            format!("view.rotate:{axis}-{sign}"),
            format!("cmd-rotate-{axis}-{sign}"),
            "view",
            &[],
            Tool("view.rotate", json!({"axis": axis, "deg": deg})),
        ));
    }
    for (i, m) in RenderMode::ALL.into_iter().enumerate() {
        const DIGITS: [&str; 4] = ["1", "2", "3", "4"];
        v.push(cmd(
            format!("display.render_mode:{}", m.key()),
            format!("render-mode-{}", m.key()),
            "display",
            &[DIGITS[i]],
            Tool("display.render_mode", json!({"mode": m})),
        ));
    }
    v.push(cmd(
        "window.color_by",
        "cmd-color-by-menu",
        "display",
        &["C"],
        Window(None),
    ));
    for c in ColorBy::ALL {
        v.push(cmd(
            format!("display.color_by:{}", c.key()),
            format!("cmd-color-by-{}", c.key()),
            "display",
            &[],
            Tool("display.color_by", json!({"by": c})),
        ));
    }
    for (axis, key) in [("x", "X"), ("y", "Y"), ("z", "Z")] {
        v.push(cmd(
            format!("clip.set:{axis}"),
            format!("cmd-clip-{axis}"),
            "clip",
            &[key],
            Tool("clip.set", json!({ "axis": axis })),
        ));
    }
    v.push(cmd(
        "clip.set:view",
        "cmd-clip-view",
        "clip",
        &[],
        Tool("clip.set", json!({"axis": "view"})),
    ));
    v.push(cmd(
        "clip.set:flip",
        "cmd-clip-flip",
        "clip",
        &["Shift+C"],
        Dynamic("clip.set", |s| {
            s.display.clip.map(|_| json!({"flip": true}))
        }),
    ));
    for m in murmel_engine::ClipMode::ALL {
        v.push(cmd(
            format!("clip.set:{}", m.key()),
            format!("cmd-clip-{}", m.key()),
            "clip",
            &[],
            Tool("clip.set", json!({ "mode": m })),
        ));
    }
    for (dir, step) in [("more", 0.05), ("less", -0.05)] {
        v.push(cmd(
            format!("clip.set:{dir}"),
            format!("cmd-clip-{dir}"),
            "clip",
            &[],
            Dynamic(
                "clip.set",
                if step > 0.0 {
                    |s| clip_step(s, 0.05)
                } else {
                    |s| clip_step(s, -0.05)
                },
            ),
        ));
    }
    v.push(cmd(
        "clip.set:toggle",
        "cmd-clip-toggle",
        "clip",
        &[],
        Dynamic("clip.set", |s| {
            s.display.clip.map(|c| json!({"enabled": !c.enabled}))
        }),
    ));
    v.push(cmd(
        "clip.clear",
        "cmd-clip-clear",
        "clip",
        &[],
        Dynamic("clip.clear", |s| s.display.clip.map(|_| json!({}))),
    ));
    v.push(cmd(
        "display.visibility:hide",
        "cmd-hide",
        "selection",
        &["H"],
        Dynamic("display.visibility", |s| {
            selected(s).map(|ids| json!({"action": "hide", "items": ids}))
        }),
    ));
    v.push(cmd(
        "display.visibility:isolate",
        "cmd-isolate",
        "selection",
        &["I"],
        Dynamic("display.visibility", |s| {
            selected(s).map(|ids| json!({"action": "isolate", "items": ids}))
        }),
    ));
    v.push(cmd(
        "display.visibility:show_all",
        "cmd-show-all",
        "selection",
        &["Alt+H"],
        Dynamic("display.visibility", |s| {
            s.mesh.as_ref().map(|_| json!({"action": "show_all"}))
        }),
    ));
    v.push(cmd(
        "view.fit:selection",
        "cmd-fit-selection",
        "selection",
        &[],
        Dynamic("view.fit", |s| {
            let m = s.mesh.as_ref()?;
            let it = s.display.current().filter(|&i| i != Item::Mesh)?;
            Some(json!({"item": it.id(&m.mesh)}))
        }),
    ));
    v.push(cmd(
        "select.clear",
        "cmd-clear-selection",
        "selection",
        &["Escape"],
        Dynamic("select.clear", |s| {
            (!s.display.selection.is_empty()).then(|| json!({}))
        }),
    ));
    for t in murmel_engine::PickTarget::ALL {
        v.push(cmd(
            format!("select.filter:{}", t.key()),
            format!("cmd-pick-{}", t.key()),
            "selection",
            &[],
            Tool("select.filter", json!({ "target": t })),
        ));
    }
    for t in ["dark", "light", "system"] {
        v.push(cmd(
            format!("settings.set_theme:{t}"),
            format!("cmd-theme-{t}"),
            "settings",
            &[],
            Tool("settings.set_theme", json!({ "theme": t })),
        ));
    }
    for p in murmel_engine::navigation::PRESETS {
        v.push(cmd(
            format!("settings.set_bindings:{p}"),
            format!("preset-{p}"),
            "settings",
            &[],
            Tool("settings.set_bindings", json!({ "preset": p })),
        ));
    }
    v.push(cmd(
        "settings.set_colors:reset",
        "settings-reset-colors",
        "settings",
        &[],
        Tool("settings.set_colors", json!({"reset": true})),
    ));
    v
}

/// Tools no command calls, and why: they answer questions whose answers
/// the window already shows, or need an input only a panel can give.
pub const NOT_IN_PALETTE: &[(&str, &str)] = &[
    (
        "system.info",
        "the status bar shows the GPU, the CLI the version",
    ),
    (
        "system.commands",
        "the catalogue is for scripts; the palette is the window's",
    ),
    (
        "diag.codes",
        "the status bar shows a diagnostic with its fix",
    ),
    ("mesh.info", "the Properties panel shows it"),
    ("view.get_camera", "the view note and the cube show it"),
    ("inspect.item", "the Properties panel shows it"),
    ("settings.get", "the Settings window shows it"),
    ("select.item", "needs an item: the model tree selects"),
    (
        "select.pick",
        "needs a screen point: a click in the viewport picks",
    ),
    (
        "inspect.at",
        "needs a screen point: the hover tooltip and the status bar ask",
    ),
    (
        "display.set_color",
        "needs a colour: the tree's context menu picks it",
    ),
];

/// Normalise a key for comparison: modifiers in a fixed order.
pub fn normalise(key: &str) -> String {
    let mut parts: Vec<&str> = key.split('+').collect();
    let k = parts.pop().unwrap_or_default();
    let mut mods: Vec<&str> = parts;
    mods.sort_unstable();
    mods.push(k);
    mods.join("+")
}

/// The English text of a translation key without arguments, read from
/// the Fluent file (for `docs/keys.md` and the CLI; the window formats
/// with Fluent itself).
pub fn english(key: &str) -> String {
    const FTL: &str = include_str!("../../../assets/i18n/en/murmel.ftl");
    FTL.lines()
        .find_map(|l| {
            let (k, v) = l.split_once(" = ")?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
        .unwrap_or_else(|| key.to_string())
}

/// `docs/keys.md`: the keyboard map, generated from the table.
pub fn keys_markdown() -> String {
    let label = english;
    let mut out = String::from(
        "# Keyboard map\n\nGenerated by `cargo run -p murmel-cli -- keys --markdown > docs/keys.md`\n\
         from `crates/murmel-tools/src/commands.rs`; a test fails when it is out of date.\n\
         `Ctrl` is the Command key on macOS. Every command is also in the command palette\n\
         (Ctrl + K), with the palette-only commands below the keyed ones.\n\n\
         | Keys | Command | Calls |\n|---|---|---|\n",
    );
    let all = commands();
    let (keyed, rest): (Vec<_>, Vec<_>) = all.iter().partition(|c| !c.keys.is_empty());
    for c in keyed.into_iter().chain(rest) {
        let keys = if c.keys.is_empty() {
            "palette".to_string()
        } else {
            c.keys
                .iter()
                .map(|k| format!("`{}`", k.replace('+', " + ")))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let calls = match &c.run {
            Run::Tool(t, v) => format!("`{t}` `{v}`"),
            Run::Dynamic(t, _) => format!("`{t}` (input from the session)"),
            Run::Window(Some(t)) => format!("window, then `{t}`"),
            Run::Window(None) => "window".to_string(),
        };
        out.push_str(&format!("| {keys} | {} | {calls} |\n", label(&c.label_key)));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use murmel_engine::Tokens;

    use super::*;

    fn root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Invariant (AGENTS.md: every command has a key or a palette entry):
    /// every tool is called by some command, or is listed in
    /// NOT_IN_PALETTE with the reason; command ids are unique; every
    /// command's label is translated.
    #[test]
    fn every_tool_is_reachable() {
        let all = commands();
        for t in crate::tools() {
            let called = all.iter().any(|c| c.tool() == Some(t.id));
            let exempt = NOT_IN_PALETTE.iter().any(|(id, _)| *id == t.id);
            assert!(
                called ^ exempt,
                "{}: called {called}, exempt {exempt}",
                t.id
            );
        }
        for (id, _) in NOT_IN_PALETTE {
            assert!(crate::find(id).is_some(), "{id} is no tool");
        }
        let mut ids: Vec<&str> = all.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate command id");
        for c in &all {
            assert!(GROUPS.contains(&c.group), "{}: group {}", c.id, c.group);
            assert_ne!(
                english(&c.label_key),
                c.label_key,
                "{} untranslated",
                c.label_key
            );
        }
    }

    /// Invariant: no key runs two commands (modifiers in any order), and
    /// the spec's keys are where it puts them.
    #[test]
    fn keys_are_unique() {
        let all = commands();
        let mut keys: Vec<String> = all
            .iter()
            .flat_map(|c| c.keys.iter().map(|k| normalise(k)))
            .collect();
        keys.sort();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n, "a key is bound twice");
        let key_of = |id: &str| all.iter().find(|c| c.id == id).map(|c| c.keys.clone());
        assert_eq!(key_of("window.palette"), Some(vec!["Ctrl+K"]));
        assert_eq!(key_of("view.fit"), Some(vec!["F"]));
        assert_eq!(key_of("view.projection:next"), Some(vec!["5"]));
        assert_eq!(key_of("view.standard:front"), Some(vec!["Alt+1"]));
        assert_eq!(key_of("view.standard:bottom"), Some(vec!["Ctrl+Alt+7"]));
        assert_eq!(key_of("display.render_mode:feature_edges"), Some(vec!["4"]));
    }

    /// Invariant: every command with a fixed input runs on a session with
    /// the channel open, and the selection commands run once something is
    /// selected; so no key or palette entry fails with a bad input.
    #[test]
    fn every_command_runs() {
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir =
            Some(std::env::temp_dir().join(format!("murmel-commands-{}", std::process::id())));
        let channel = root().join("test-fixtures/channel/foehncraft.vtu");
        let open = |s: &mut Session| {
            assert!(crate::call("mesh.open", json!({"path": channel}), s).ok());
            assert!(crate::call("select.item", json!({"item": "patch:inlet"}), s).ok());
        };
        open(&mut s);
        for c in commands() {
            if c.id == "mesh.close" || c.id.starts_with("display.visibility") {
                continue;
            }
            match c.input(&s) {
                Some((tool, input)) => {
                    let e = crate::call(tool, input, &mut s);
                    assert!(e.ok(), "{}: {:?}", c.id, e.diagnostics);
                }
                None => assert!(
                    matches!(c.run, Run::Window(_)) || c.id == "select.clear",
                    "{} has no input with a selection",
                    c.id
                ),
            }
            if s.display.selection.is_empty() {
                open(&mut s);
            }
        }
        for id in [
            "display.visibility:hide",
            "display.visibility:isolate",
            "display.visibility:show_all",
            "mesh.close",
        ] {
            open(&mut s);
            let c = commands().into_iter().find(|c| c.id == id).unwrap();
            let (tool, input) = c.input(&s).unwrap();
            assert!(crate::call(tool, input, &mut s).ok(), "{id}");
        }
        let clear = commands()
            .into_iter()
            .find(|c| c.id == "select.clear")
            .unwrap();
        assert!(clear.input(&s).is_none(), "nothing selected after close");
        let _ = std::fs::remove_dir_all(s.config_dir.unwrap());
    }

    /// Invariant: docs/keys.md is what `murmel keys --markdown` prints.
    #[test]
    fn keys_md_is_up_to_date() {
        let doc = std::fs::read_to_string(root().join("docs/keys.md")).unwrap_or_default();
        assert!(
            doc == keys_markdown(),
            "docs/keys.md is out of date; run `cargo run -p murmel-cli -- keys --markdown > docs/keys.md`"
        );
    }
}
