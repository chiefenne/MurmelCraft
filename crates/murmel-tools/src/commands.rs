//! The commands of the window: the keyboard map and the command palette
//! (UI spec, "Keyboard map and accessibility"). Each command is a call of
//! a registered tool with a fixed input, or with an input read from the
//! session (the selection, the next projection); the few that only change
//! the window (panels, the palette itself, file dialogs) are marked
//! `Window` and name the tool they end in, if any.
//!
//! The table is data, so the window, the shortcut sheet, `docs/keys.md`
//! and the tests all read the same keys.

use murmel_engine::keys::KeyMap;
use murmel_engine::{
    ColorBy, FieldMode, Item, Metric, Projection, Quality, RenderMode, Session, StandardView,
};
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
    /// Keys, like `Ctrl+K`, `Alt+1`, `Shift+Left` or `F1`, in the form
    /// of [`murmel_engine::keys::canonical`]. `Ctrl` is the Command key
    /// on macOS. Empty for palette-only commands. [`with_keys`] puts
    /// your keys here.
    pub keys: Vec<String>,
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
    "edit",
    "file",
    "window",
    "display",
    "field",
    "clip",
    "slice",
    "region",
    "measure",
    "series",
    "selection",
    "settings",
    "help",
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

/// The value of the current quality metric at or above which `share` of
/// the cells lie (the worst ones); `None` without a mesh.
fn worst_share(s: &Session, share: f64) -> Option<f64> {
    let m = s.mesh.as_ref()?;
    let owned;
    let q = match &s.display.quality {
        Some(q) => q.as_ref(),
        None => {
            owned = Quality::compute(&m.mesh);
            &owned
        }
    };
    let mut v: Vec<f32> = q.values(s.display.metric).to_vec();
    if v.is_empty() {
        return None;
    }
    v.sort_unstable_by(|a, b| a.total_cmp(b));
    let k = ((v.len() as f64 * (1.0 - share)).floor() as usize).min(v.len() - 1);
    Some(v[k] as f64)
}

/// Move the clip plane by `share` of its range (towards the side cut
/// away for a positive share, so more is cut).
fn clip_step(s: &Session, share: f64) -> Option<Value> {
    let c = *s.display.clip()?;
    let m = s.mesh.as_ref()?;
    let (off, [lo, hi]) = c.offset(&m.mesh.bounds());
    Some(json!({"offset": (off - share * (hi - lo)).clamp(lo, hi)}))
}

/// The input of a `Dynamic` command, read from the session.
type Input = fn(&Session) -> Option<Value>;

/// A tilt of the clip plane, when there is one.
fn tilt(s: &Session, axis: &str, deg: f64) -> Option<Value> {
    s.display
        .clip()
        .map(|_| json!({ "axis": axis, "deg": deg }))
}

/// The saved view `by` places after the one the camera is at (the first
/// or the last when it is at none), wrapping round.
fn step_view(s: &Session, by: isize) -> Option<Value> {
    let views = &s.display.views;
    if views.is_empty() || s.mesh.is_none() {
        return None;
    }
    let n = views.len() as isize;
    let k = match views.iter().position(|v| v.camera == s.camera) {
        Some(at) => (at as isize + by).rem_euclid(n),
        None if by > 0 => 0,
        None => n - 1,
    };
    Some(json!({"name": views[k as usize].name}))
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
        keys: keys.iter().map(|k| k.to_string()).collect(),
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
            "window.open_folder",
            "cmd-open-folder",
            "file",
            &["Ctrl+Shift+O"],
            Window(Some("mesh.open")),
        ),
        cmd(
            "mesh.close",
            "cmd-close",
            "file",
            &["Ctrl+W"],
            Tool("mesh.close", json!({})),
        ),
        cmd(
            "recent.clear",
            "cmd-recent-clear",
            "file",
            &[],
            Tool("recent.clear", json!({})),
        ),
        cmd(
            "window.session_save",
            "cmd-session-save",
            "file",
            &["Ctrl+S"],
            Window(Some("session.save")),
        ),
        cmd(
            "window.session_save_as",
            "cmd-session-save-as",
            "file",
            &["Ctrl+Shift+S"],
            Window(Some("session.save")),
        ),
        cmd(
            "window.session_load",
            "cmd-session-load",
            "file",
            &[],
            Window(Some("session.load")),
        ),
        cmd(
            "window.screenshot",
            "cmd-screenshot",
            "file",
            &[],
            Window(Some("view.screenshot")),
        ),
        cmd(
            "window.export_image",
            "cmd-export-image",
            "file",
            &["Ctrl+Shift+E"],
            Window(Some("export.image")),
        ),
        cmd(
            "window.export_movie",
            "cmd-export-movie",
            "file",
            &["Ctrl+Shift+M"],
            Window(Some("export.movie")),
        ),
        // Closes the window (macOS takes Cmd + Q itself first, to the same
        // end).
        cmd("window.quit", "cmd-quit", "file", &["Ctrl+Q"], Window(None)),
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
            "window.next_region",
            "cmd-next-region",
            "window",
            &["F6"],
            Window(None),
        ),
        cmd(
            "window.previous_region",
            "cmd-previous-region",
            "window",
            &["Shift+F6"],
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
            "window.minimize",
            "cmd-minimize",
            "window",
            &["Ctrl+M"],
            Window(None),
        ),
        cmd(
            "window.fullscreen",
            "cmd-fullscreen",
            "window",
            &["F11"],
            Window(None),
        ),
        // The shortcut editor: keys (settings.set_keys) and mouse bindings
        // (settings.set_bindings) in one window.
        cmd(
            "window.edit_shortcuts",
            "cmd-edit-shortcuts",
            "settings",
            &[],
            Window(Some("settings.set_keys")),
        ),
        cmd(
            "view.fit",
            "cmd-fit",
            "view",
            &["F"],
            Tool("view.fit", json!({})),
        ),
        // Arms the rubber band: the next drag in the viewport draws a box
        // and zooms into it, then the mouse navigates again (one box per
        // press, the maintainer's card of 2026-11-25). Z is the Z clip.
        cmd(
            "window.zoom_box",
            "cmd-zoom-box",
            "view",
            &["W"],
            Window(Some("view.zoom_box")),
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
        "window.save_view",
        "cmd-views-save",
        "view",
        &["Ctrl+B"],
        // The window names the view in the person's language.
        Window(Some("views.save")),
    ));
    v.push(cmd(
        "views.go:next",
        "cmd-views-next",
        "view",
        &["V"],
        Dynamic("views.go", |s| step_view(s, 1)),
    ));
    v.push(cmd(
        "views.go:previous",
        "cmd-views-previous",
        "view",
        &["Shift+V"],
        Dynamic("views.go", |s| step_view(s, -1)),
    ));
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
    for c in ColorBy::ALL.into_iter().filter(|&c| c != ColorBy::Quality) {
        v.push(cmd(
            format!("display.color_by:{}", c.key()),
            format!("cmd-color-by-{}", c.key()),
            "display",
            &[],
            Tool("display.color_by", json!({"by": c})),
        ));
    }
    // Fields: the colouring one key away (B cycles flat, smooth and
    // banded; L turns the contour lines on and off), the rest in the
    // palette. They act on the field shown now.
    v.push(cmd(
        "field.style:next_mode",
        "cmd-field-next-mode",
        "field",
        &["B"],
        Dynamic("field.style", |s| {
            let v = s.field_values()?;
            let next = match s.field_style().mode {
                FieldMode::Flat => FieldMode::Smooth,
                FieldMode::Smooth => FieldMode::Banded,
                FieldMode::Banded => FieldMode::Flat,
            };
            Some(json!({"field": v.choice.name, "mode": next}))
        }),
    ));
    v.push(cmd(
        "field.style:lines",
        "cmd-field-lines",
        "field",
        &["L"],
        Dynamic("field.style", |s| {
            let v = s.field_values()?;
            Some(json!({"field": v.choice.name, "lines": !s.field_style().lines}))
        }),
    ));
    for m in FieldMode::ALL {
        v.push(cmd(
            format!("field.style:{}", m.key()),
            format!("cmd-field-mode-{}", m.key()),
            "field",
            &[],
            Tool("field.style", json!({ "mode": m })),
        ));
    }
    v.push(cmd(
        "field.style:reverse",
        "cmd-field-reverse",
        "field",
        &[],
        Dynamic("field.style", |s| {
            let v = s.field_values()?;
            Some(json!({"field": v.choice.name, "reverse": !s.field_style().reverse}))
        }),
    ));
    for r in ["field", "shown", "symmetric", "fixed", "steps"] {
        v.push(cmd(
            format!("field.style:range-{r}"),
            format!("cmd-field-range-{r}"),
            "field",
            &[],
            Tool("field.style", json!({ "range": r })),
        ));
    }
    v.push(cmd(
        "field.style:map-auto",
        "cmd-field-map-auto",
        "field",
        &[],
        Tool("field.style", json!({"map": "auto"})),
    ));
    for id in murmel_engine::colormap::ids() {
        v.push(cmd(
            format!("field.style:map-{id}"),
            format!("colormap-{id}"),
            "field",
            &[],
            Tool("field.style", json!({ "map": id })),
        ));
    }
    v.push(cmd(
        "field.style:reset",
        "cmd-field-reset",
        "field",
        &[],
        Tool("field.style", json!({"reset": true})),
    ));
    // Derived fields: the catalogue window, and each quantity at once
    // with its inputs found by name.
    v.push(cmd(
        "window.derived",
        "cmd-derived",
        "field",
        &[],
        Window(Some("field.derive")),
    ));
    // The calculator: the same window, its formula field focused.
    v.push(cmd(
        "window.calculator",
        "cmd-calculator",
        "field",
        &[],
        Window(Some("field.calculate")),
    ));
    for q in murmel_engine::derive::Quantity::ALL {
        v.push(cmd(
            format!("field.derive:{}", q.key()),
            format!("cmd-derive-{}", q.key()),
            "field",
            &[],
            Tool("field.derive", json!({"quantity": q.key()})),
        ));
    }
    for m in Metric::ALL {
        v.push(cmd(
            format!("display.color_by:quality-{}", m.key()),
            format!("cmd-color-by-quality-{}", m.key()),
            "display",
            &[],
            Tool("display.color_by", json!({"by": "quality", "metric": m})),
        ));
    }
    v.push(cmd(
        "display.threshold:worst",
        "cmd-threshold-worst",
        "display",
        &[],
        Dynamic("display.threshold", |s| {
            worst_share(s, 0.1).map(|t| json!({ "value": t }))
        }),
    ));
    v.push(cmd(
        "display.threshold:clear",
        "cmd-threshold-clear",
        "display",
        &[],
        Dynamic("display.threshold", |s| {
            s.display.threshold.map(|_| json!({ "value": null }))
        }),
    ));
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
        "clip.set:new",
        "cmd-clip-new",
        "clip",
        &["Alt+C"],
        Dynamic("clip.set", |s| {
            (s.mesh.is_some() && s.display.clips.len() < crate::tools::MAX_CLIPS)
                .then(|| json!({"new": true, "axis": "x"}))
        }),
    ));
    v.push(cmd(
        "select.item:next_clip",
        "cmd-clip-next",
        "clip",
        &[],
        Dynamic("select.item", |s| {
            let m = s.mesh.as_ref()?;
            let n = s.display.clips.len();
            let k = (s.display.current_clip()? + 1) % n;
            Some(json!({"item": murmel_engine::Item::Clip(k as u32).id(&m.mesh)}))
        }),
    ));
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
            s.display.clip().map(|_| json!({"flip": true}))
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
    // The clip plane by keys: Alt + Shift + Up and Down slide it (as
    // Shift + arrows pan the camera), Alt + arrows tilt it (as the
    // arrows orbit the camera).
    for (dir, step, key) in [
        ("more", 0.05, "Alt+Shift+Up"),
        ("less", -0.05, "Alt+Shift+Down"),
    ] {
        v.push(cmd(
            format!("clip.set:{dir}"),
            format!("cmd-clip-{dir}"),
            "clip",
            &[key],
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
    // The normal turns towards the screen side named: about the screen's
    // up axis for left and right, about its right axis for up and down.
    let tilts: [(&str, &'static str, Input); 4] = [
        ("left", "Alt+Left", |s| tilt(s, "y", -5.0)),
        ("right", "Alt+Right", |s| tilt(s, "y", 5.0)),
        ("up", "Alt+Up", |s| tilt(s, "x", -5.0)),
        ("down", "Alt+Down", |s| tilt(s, "x", 5.0)),
    ];
    for (dir, key, input) in tilts {
        v.push(cmd(
            format!("clip.rotate:{dir}"),
            format!("cmd-clip-tilt-{dir}"),
            "clip",
            &[key],
            Dynamic("clip.rotate", input),
        ));
    }
    v.push(cmd(
        "clip.set:toggle",
        "cmd-clip-toggle",
        "clip",
        &[],
        Dynamic("clip.set", |s| {
            s.display.clip().map(|c| json!({"enabled": !c.enabled}))
        }),
    ));
    v.push(cmd(
        "clip.clear",
        "cmd-clip-clear",
        "clip",
        &[],
        Dynamic("clip.clear", |s| s.display.clip().map(|_| json!({}))),
    ));
    v.push(cmd(
        "clip.clear:all",
        "cmd-clip-clear-all",
        "clip",
        &[],
        Dynamic("clip.clear", |s| {
            (!s.display.clips.is_empty()).then(|| json!({"all": true}))
        }),
    ));
    // Slices are placed like clip planes (decided 2026-11-24): Shift +
    // X, Y, Z set the current one's normal (adding one when there is
    // none), Alt + S adds one.
    for (axis, key) in [("x", "Shift+X"), ("y", "Shift+Y"), ("z", "Shift+Z")] {
        v.push(cmd(
            format!("slice.set:{axis}"),
            format!("cmd-slice-{axis}"),
            "slice",
            &[key],
            Tool("slice.set", json!({ "axis": axis })),
        ));
    }
    v.push(cmd(
        "slice.set:new",
        "cmd-slice-new",
        "slice",
        &["Alt+S"],
        Dynamic("slice.set", |s| {
            (s.mesh.is_some() && s.display.slices.len() < murmel_engine::extract::MAX_SLICES)
                .then(|| json!({"new": true, "axis": "x"}))
        }),
    ));
    v.push(cmd(
        "slice.set:view",
        "cmd-slice-view",
        "slice",
        &[],
        Tool("slice.set", json!({"axis": "view"})),
    ));
    v.push(cmd(
        "slice.set:series",
        "cmd-slice-series",
        "slice",
        &[],
        Dynamic("slice.set", |s| {
            s.display.current_slice().map(|_| json!({"count": 5}))
        }),
    ));
    v.push(cmd(
        "slice.set:single",
        "cmd-slice-single",
        "slice",
        &[],
        Dynamic("slice.set", |s| {
            let k = s.display.current_slice()?;
            (s.display.slices[k].count > 1).then(|| json!({"count": 1}))
        }),
    ));
    v.push(cmd(
        "slice.set:toggle",
        "cmd-slice-toggle",
        "slice",
        &[],
        Dynamic("slice.set", |s| {
            let k = s.display.current_slice()?;
            Some(json!({"enabled": !s.display.slices[k].enabled}))
        }),
    ));
    v.push(cmd(
        "slice.clear",
        "cmd-slice-clear",
        "slice",
        &[],
        Dynamic("slice.clear", |s| {
            s.display.current_slice().map(|_| json!({}))
        }),
    ));
    v.push(cmd(
        "slice.clear:all",
        "cmd-slice-clear-all",
        "slice",
        &[],
        Dynamic("slice.clear", |s| {
            (!s.display.slices.is_empty()).then(|| json!({"all": true}))
        }),
    ));
    v.push(cmd(
        "iso.set:new",
        "cmd-iso-new",
        "slice",
        &["Alt+I"],
        Dynamic("iso.set", |s| {
            let fields = s.mesh.as_ref().map(|m| !m.fields.is_empty())?;
            (fields && s.display.isos.len() < murmel_engine::extract::MAX_ISOS)
                .then(|| json!({"new": true}))
        }),
    ));
    for c in murmel_engine::IsoColor::ALL {
        v.push(cmd(
            format!("iso.set:{}", c.key()),
            format!("cmd-iso-{}", c.key()),
            "slice",
            &[],
            match c {
                murmel_engine::IsoColor::Solid => Dynamic("iso.set", |s| {
                    s.display.current_iso().map(|_| json!({"color": "solid"}))
                }),
                murmel_engine::IsoColor::Field => Dynamic("iso.set", |s| {
                    s.display.current_iso().map(|_| json!({"color": "field"}))
                }),
            },
        ));
    }
    v.push(cmd(
        "iso.set:toggle",
        "cmd-iso-toggle",
        "slice",
        &[],
        Dynamic("iso.set", |s| {
            let k = s.display.current_iso()?;
            Some(json!({"enabled": !s.display.isos[k].enabled}))
        }),
    ));
    v.push(cmd(
        "iso.clear",
        "cmd-iso-clear",
        "slice",
        &[],
        Dynamic("iso.clear", |s| s.display.current_iso().map(|_| json!({}))),
    ));
    v.push(cmd(
        "iso.clear:all",
        "cmd-iso-clear-all",
        "slice",
        &[],
        Dynamic("iso.clear", |s| {
            (!s.display.isos.is_empty()).then(|| json!({"all": true}))
        }),
    ));
    // Vector arrows: Alt + V shows them (on the slices; the field shown
    // if it is a vector, else the file's first) or hides them again.
    v.push(cmd(
        "arrows.set:toggle",
        "cmd-arrows-toggle",
        "slice",
        &["Alt+V"],
        Dynamic("arrows.set", |s| {
            let m = s.mesh.as_ref()?;
            match &s.display.arrows {
                Some(a) => Some(json!({"enabled": !a.enabled})),
                None => {
                    let vector = m
                        .fields
                        .names()
                        .iter()
                        .any(|n| m.fields.get(n).is_some_and(|f| f.is_vector()));
                    vector.then(|| json!({"enabled": true}))
                }
            }
        }),
    ));
    v.push(cmd(
        "arrows.set:surface",
        "cmd-arrows-surface",
        "slice",
        &[],
        Dynamic("arrows.set", |s| {
            s.display
                .arrows
                .as_ref()
                .map(|a| json!({"surface": !a.surface}))
        }),
    ));
    v.push(cmd(
        "arrows.set:in_plane",
        "cmd-arrows-in-plane",
        "slice",
        &[],
        Dynamic("arrows.set", |s| {
            s.display
                .arrows
                .as_ref()
                .map(|a| json!({"in_plane": !a.in_plane}))
        }),
    ));
    v.push(cmd(
        "arrows.set:denser",
        "cmd-arrows-denser",
        "slice",
        &[],
        Dynamic("arrows.set", |s| {
            let a = s.display.arrows.as_ref()?;
            let d = (a.density * 3 / 2).min(murmel_engine::glyph::MAX_DENSITY);
            (d != a.density).then(|| json!({"density": d}))
        }),
    ));
    v.push(cmd(
        "arrows.set:sparser",
        "cmd-arrows-sparser",
        "slice",
        &[],
        Dynamic("arrows.set", |s| {
            let a = s.display.arrows.as_ref()?;
            let d = (a.density * 2 / 3).max(murmel_engine::glyph::MIN_DENSITY);
            (d != a.density).then(|| json!({"density": d}))
        }),
    ));
    v.push(cmd(
        "arrows.set:longer",
        "cmd-arrows-longer",
        "slice",
        &[],
        Dynamic("arrows.set", |s| {
            let a = s.display.arrows.as_ref()?;
            let x = (a.scale * 1.5).min(murmel_engine::glyph::MAX_SCALE);
            (x != a.scale).then(|| json!({"scale": x}))
        }),
    ));
    v.push(cmd(
        "arrows.set:shorter",
        "cmd-arrows-shorter",
        "slice",
        &[],
        Dynamic("arrows.set", |s| {
            let a = s.display.arrows.as_ref()?;
            Some(json!({"scale": a.scale / 1.5}))
        }),
    ));
    for c in murmel_engine::glyph::ArrowColor::ALL {
        use murmel_engine::glyph::ArrowColor as C;
        v.push(cmd(
            format!("arrows.set:{}", c.key()),
            format!("cmd-arrows-{}", c.key()),
            "slice",
            &[],
            match c {
                C::Solid => Dynamic("arrows.set", |s| {
                    s.display.arrows.as_ref().map(|_| json!({"color": "solid"}))
                }),
                C::Field => Dynamic("arrows.set", |s| {
                    s.display.arrows.as_ref().map(|_| json!({"color": "field"}))
                }),
                C::Magnitude => Dynamic("arrows.set", |s| {
                    s.display
                        .arrows
                        .as_ref()
                        .map(|_| json!({"color": "magnitude"}))
                }),
            },
        ));
    }
    for l in murmel_engine::glyph::Length::ALL {
        use murmel_engine::glyph::Length as L;
        v.push(cmd(
            format!("arrows.set:length-{}", l.key()),
            format!("cmd-arrows-length-{}", l.key()),
            "slice",
            &[],
            match l {
                L::Magnitude => Dynamic("arrows.set", |s| {
                    s.display
                        .arrows
                        .as_ref()
                        .map(|_| json!({"length": "magnitude"}))
                }),
                L::Floor => Dynamic("arrows.set", |s| {
                    s.display
                        .arrows
                        .as_ref()
                        .map(|_| json!({"length": "floor"}))
                }),
                L::Equal => Dynamic("arrows.set", |s| {
                    s.display
                        .arrows
                        .as_ref()
                        .map(|_| json!({"length": "equal"}))
                }),
            },
        ));
    }
    v.push(cmd(
        "arrows.clear",
        "cmd-arrows-clear",
        "slice",
        &[],
        Dynamic("arrows.clear", |s| {
            s.display.arrows.as_ref().map(|_| json!({}))
        }),
    ));
    // Spray parcels (D268): open a parcel file, show or hide them, size
    // them by diameter, remove them.
    v.push(cmd(
        "window.parcels_open",
        "cmd-parcels-open",
        "file",
        &[],
        Window(Some("parcels.open")),
    ));
    v.push(cmd(
        "parcels.set:toggle",
        "cmd-parcels-toggle",
        "slice",
        &[],
        Dynamic("parcels.set", |s| {
            s.display
                .parcels
                .as_ref()
                .map(|p| json!({"enabled": !p.enabled}))
        }),
    ));
    v.push(cmd(
        "parcels.set:size",
        "cmd-parcels-size",
        "slice",
        &[],
        Dynamic("parcels.set", |s| {
            let p = s.display.parcels.as_ref()?;
            let data = s.display.parcel_data.as_ref()?;
            data.field(murmel_engine::parcel::DIAMETER)?;
            Some(json!({"size": match p.size {
                murmel_engine::parcel::Size::Fixed => "diameter",
                murmel_engine::parcel::Size::Diameter => "fixed",
            }}))
        }),
    ));
    v.push(cmd(
        "parcels.clear",
        "cmd-parcels-clear",
        "slice",
        &[],
        Dynamic("parcels.clear", |s| {
            s.display.parcels.as_ref().map(|_| json!({}))
        }),
    ));
    // Time series (D268): open one, play or pause it (Space), step on
    // and back (. and ,), go to the first or the last step.
    v.push(cmd(
        "window.series_open",
        "cmd-series-open",
        "series",
        &[],
        Window(Some("series.open")),
    ));
    v.push(cmd(
        "series.play:toggle",
        "cmd-series-play",
        "series",
        &["Space"],
        Dynamic("series.play", |s| {
            let r = s.series.as_ref()?;
            (r.len() > 1).then(|| json!({"playing": !r.playback.playing}))
        }),
    ));
    let steps: [(&str, &str, &[&'static str], Input); 4] = [
        ("series.go:next", "cmd-series-next", &["Period"], |s| {
            s.series.as_ref().map(|_| json!({"by": 1}))
        }),
        (
            "series.go:previous",
            "cmd-series-previous",
            &["Comma"],
            |s| s.series.as_ref().map(|_| json!({"by": -1})),
        ),
        ("series.go:first", "cmd-series-first", &[], |s| {
            s.series.as_ref().map(|_| json!({"to": "first"}))
        }),
        ("series.go:last", "cmd-series-last", &[], |s| {
            s.series.as_ref().map(|_| json!({"to": "last"}))
        }),
    ];
    for (id, key, keys, input) in steps {
        v.push(cmd(id, key, "series", keys, Dynamic("series.go", input)));
    }
    // Charts and statistics of a run (D268, second milestone): open the
    // tables of an output folder, show or hide the panel, pick a chart,
    // save it as a picture.
    v.push(cmd(
        "window.charts_open",
        "cmd-charts-open",
        "series",
        &[],
        Window(Some("charts.open")),
    ));
    v.push(cmd(
        "charts.show:toggle",
        "cmd-charts-toggle",
        "series",
        &[],
        Dynamic("charts.show", |s| {
            s.charts.as_ref().map(|c| json!({"visible": !c.visible}))
        }),
    ));
    for kind in murmel_engine::chart::ChartKind::ALL {
        if kind == murmel_engine::chart::ChartKind::Columns {
            continue;
        }
        v.push(cmd(
            format!("charts.show:{}", kind.key()),
            format!("cmd-charts-{}", kind.key().replace('_', "-")),
            "series",
            &[],
            Dynamic(
                "charts.show",
                match kind {
                    murmel_engine::chart::ChartKind::MassFlow => |s: &Session| {
                        s.charts
                            .as_ref()
                            .map(|_| json!({"chart": "mass_flow", "visible": true}))
                    },
                    murmel_engine::chart::ChartKind::Cumulative => |s: &Session| {
                        s.charts
                            .as_ref()
                            .map(|_| json!({"chart": "cumulative", "visible": true}))
                    },
                    murmel_engine::chart::ChartKind::Penetration => |s: &Session| {
                        s.charts
                            .as_ref()
                            .map(|_| json!({"chart": "penetration", "visible": true}))
                    },
                    murmel_engine::chart::ChartKind::Diameters => |s: &Session| {
                        s.charts
                            .as_ref()
                            .map(|_| json!({"chart": "diameters", "visible": true}))
                    },
                    _ => |s: &Session| {
                        s.charts
                            .as_ref()
                            .map(|_| json!({"chart": "histogram", "visible": true}))
                    },
                },
            ),
        ));
    }
    v.push(cmd(
        "window.charts_export",
        "cmd-charts-export",
        "series",
        &[],
        Window(Some("charts.export")),
    ));
    // Streamlines: Alt + L adds a seed line across the mesh near the low
    // end of its longest side (where a duct's inlet usually is), or turns
    // the current set on and off.
    v.push(cmd(
        "stream.set:toggle",
        "cmd-stream-toggle",
        "slice",
        &["Alt+L"],
        Dynamic("stream.set", |s| {
            let m = s.mesh.as_ref()?;
            match s.display.current_stream() {
                Some(k) => Some(json!({
                    "stream": k + 1,
                    "enabled": !s.display.streams[k].enabled,
                })),
                None => {
                    let vector = m
                        .fields
                        .names()
                        .iter()
                        .any(|n| m.fields.get(n).is_some_and(|f| f.is_vector()));
                    vector.then(|| json!({"new": true, "seed": "line"}))
                }
            }
        }),
    ));
    for kind in murmel_engine::Seed::KINDS {
        v.push(cmd(
            format!("stream.set:new_{kind}"),
            format!("cmd-stream-new-{kind}"),
            "slice",
            &[],
            match kind {
                "line" => Dynamic("stream.set", |s| new_stream(s, "line")),
                "sphere" => Dynamic("stream.set", |s| new_stream(s, "sphere")),
                "patch" => Dynamic("stream.set", |s| new_stream(s, "patch")),
                _ => Dynamic("stream.set", |s| new_stream(s, "plane")),
            },
        ));
    }
    for d in murmel_engine::Direction::ALL {
        v.push(cmd(
            format!("stream.set:{}", d.key()),
            format!("cmd-stream-{}", d.key()),
            "slice",
            &[],
            match d {
                murmel_engine::Direction::Forward => Dynamic("stream.set", |s| {
                    s.display
                        .current_stream()
                        .map(|_| json!({"direction": "forward"}))
                }),
                murmel_engine::Direction::Backward => Dynamic("stream.set", |s| {
                    s.display
                        .current_stream()
                        .map(|_| json!({"direction": "backward"}))
                }),
                murmel_engine::Direction::Both => Dynamic("stream.set", |s| {
                    s.display
                        .current_stream()
                        .map(|_| json!({"direction": "both"}))
                }),
            },
        ));
    }
    v.push(cmd(
        "stream.set:tubes",
        "cmd-stream-tubes",
        "slice",
        &[],
        Dynamic("stream.set", |s| {
            let k = s.display.current_stream()?;
            Some(json!({"look": match s.display.streams[k].look {
                murmel_engine::Look::Lines => "tubes",
                murmel_engine::Look::Tubes => "lines",
            }}))
        }),
    ));
    v.push(cmd(
        "stream.set:arrowheads",
        "cmd-stream-arrowheads",
        "slice",
        &[],
        Dynamic("stream.set", |s| {
            let k = s.display.current_stream()?;
            Some(json!({"arrowheads": !s.display.streams[k].arrowheads}))
        }),
    ));
    v.push(cmd(
        "stream.set:more",
        "cmd-stream-more",
        "slice",
        &[],
        Dynamic("stream.set", |s| {
            let k = s.display.current_stream()?;
            let n = s.display.streams[k].count;
            let more = (n * 3 / 2).max(n + 1).min(murmel_engine::stream::MAX_COUNT);
            (more != n).then(|| json!({"count": more}))
        }),
    ));
    v.push(cmd(
        "stream.set:fewer",
        "cmd-stream-fewer",
        "slice",
        &[],
        Dynamic("stream.set", |s| {
            let k = s.display.current_stream()?;
            let n = s.display.streams[k].count;
            let fewer = (n * 2 / 3).max(murmel_engine::stream::MIN_COUNT);
            (fewer != n).then(|| json!({"count": fewer}))
        }),
    ));
    v.push(cmd(
        "stream.set:solid",
        "cmd-stream-solid",
        "slice",
        &[],
        Dynamic("stream.set", |s| {
            s.display
                .current_stream()
                .map(|_| json!({"color": "solid"}))
        }),
    ));
    v.push(cmd(
        "stream.set:field",
        "cmd-stream-field",
        "slice",
        &[],
        Dynamic("stream.set", |s| {
            s.display
                .current_stream()
                .map(|_| json!({"color": "field"}))
        }),
    ));
    v.push(cmd(
        "stream.clear",
        "cmd-stream-clear",
        "slice",
        &[],
        Dynamic("stream.clear", |s| {
            s.display.current_stream().map(|_| json!({}))
        }),
    ));
    v.push(cmd(
        "stream.clear:all",
        "cmd-stream-clear-all",
        "slice",
        &[],
        Dynamic("stream.clear", |s| {
            (!s.display.streams.is_empty()).then(|| json!({"all": true}))
        }),
    ));
    for l in murmel_engine::MeshLook::ALL {
        v.push(cmd(
            format!("display.mesh_look:{}", l.key()),
            format!("cmd-mesh-look-{}", l.key()),
            "slice",
            &[],
            Tool("display.mesh_look", json!({ "look": l })),
        ));
    }
    // Regions of interest: Alt + R adds a box in the middle of the mesh,
    // or turns the current region on and off.
    v.push(cmd(
        "region.set:toggle",
        "cmd-region-toggle",
        "region",
        &["Alt+R"],
        Dynamic("region.set", |s| {
            s.mesh.as_ref()?;
            Some(match s.display.current_region() {
                Some(k) => json!({
                    "region": k + 1,
                    "enabled": !s.display.regions[k].enabled,
                }),
                None => json!({"new": true, "shape": "box"}),
            })
        }),
    ));
    // Alt + Shift + R: the current box, sphere or slab region cuts flat,
    // or whole cells again.
    v.push(cmd(
        "region.set:cut",
        "cmd-region-cut",
        "region",
        &["Alt+Shift+R"],
        Dynamic("region.set", |s| {
            let k = s.display.current_region()?;
            let r = &s.display.regions[k];
            r.shape.cuts_flat().then(|| {
                let cut = match r.cut {
                    murmel_engine::ClipMode::Flat => "crinkle",
                    murmel_engine::ClipMode::Crinkle => "flat",
                };
                json!({"region": k + 1, "cut": cut})
            })
        }),
    ));
    for shape in ["box", "sphere", "slab"] {
        v.push(cmd(
            format!("region.set:{shape}"),
            format!("cmd-region-{shape}"),
            "region",
            &[],
            Dynamic(
                "region.set",
                match shape {
                    "box" => |s: &Session| {
                        s.mesh
                            .as_ref()
                            .map(|_| json!({"new": true, "shape": "box"}))
                    },
                    "sphere" => |s: &Session| {
                        s.mesh
                            .as_ref()
                            .map(|_| json!({"new": true, "shape": "sphere"}))
                    },
                    _ => |s: &Session| {
                        s.mesh
                            .as_ref()
                            .map(|_| json!({"new": true, "shape": "slab"}))
                    },
                },
            ),
        ));
    }
    v.push(cmd(
        "region.set:picked",
        "cmd-region-picked",
        "region",
        &[],
        Dynamic("region.set", |s| {
            let cells: Vec<u32> = s
                .display
                .selection
                .iter()
                .filter_map(|i| match i {
                    Item::Cell(c) => Some(*c),
                    _ => None,
                })
                .collect();
            (!cells.is_empty())
                .then(|| json!({"new": true, "shape": "cells", "cells": cells, "rings": 1}))
        }),
    ));
    v.push(cmd(
        "region.set:zone",
        "cmd-region-zone",
        "region",
        &[],
        Dynamic("region.set", |s| {
            let m = s.mesh.as_ref()?;
            let z = s.display.selection.iter().rev().find_map(|i| match i {
                Item::Zone(z) => Some(*z),
                _ => None,
            })?;
            m.mesh
                .zones
                .get(z as usize)
                .map(|name| json!({"new": true, "shape": "zone", "zone": name}))
        }),
    ));
    v.push(cmd(
        "region.set:patch",
        "cmd-region-patch",
        "region",
        &[],
        Dynamic("region.set", |s| {
            let m = s.mesh.as_ref()?;
            let p = s.display.selection.iter().rev().find_map(|i| match i {
                Item::Patch(p) => Some(*p),
                _ => None,
            })?;
            m.mesh
                .patches
                .get(p as usize)
                .map(|pa| json!({"new": true, "shape": "patch", "patch": pa.name, "layers": 3}))
        }),
    ));
    v.push(cmd(
        "region.clear",
        "cmd-region-clear",
        "region",
        &[],
        Dynamic("region.clear", |s| {
            s.display.current_region().map(|k| json!({"region": k + 1}))
        }),
    ));
    v.push(cmd(
        "region.clear:all",
        "cmd-region-clear-all",
        "region",
        &[],
        Dynamic("region.clear", |s| {
            (!s.display.regions.is_empty()).then(|| json!({"all": true}))
        }),
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
    // Measuring (UI spec addition, 2026-11-24): M and Shift + M start
    // the window's measure mode, whose clicks end in measure.add; P is
    // the Properties panel, so probes take Shift + P.
    v.push(cmd(
        "window.measure_distance",
        "cmd-measure-distance",
        "measure",
        &["M"],
        Window(Some("measure.add")),
    ));
    v.push(cmd(
        "window.measure_angle",
        "cmd-measure-angle",
        "measure",
        &["Shift+M"],
        Window(Some("measure.add")),
    ));
    v.push(cmd(
        "window.probe",
        "cmd-probe",
        "measure",
        &["Shift+P"],
        Window(Some("probe.add")),
    ));
    v.push(cmd(
        "window.probe_history",
        "cmd-probe-history",
        "measure",
        &[],
        Window(Some("probe.history")),
    ));
    v.push(cmd(
        "measure.remove:selected",
        "cmd-measure-remove-selected",
        "measure",
        &["Delete", "Backspace"],
        Dynamic("measure.remove", |s| {
            let m = s.mesh.as_ref()?;
            let ids: Vec<String> = s
                .display
                .selection
                .iter()
                .filter(|i| matches!(i, Item::Measure(_) | Item::Probe(_)))
                .map(|i| i.id(&m.mesh))
                .collect();
            (!ids.is_empty()).then(|| json!({ "items": ids }))
        }),
    ));
    v.push(cmd(
        "measure.remove:all",
        "cmd-measure-remove-all",
        "measure",
        &[],
        Dynamic("measure.remove", |s| {
            let d = &s.display;
            (s.mesh.is_some() && !(d.measurements.is_empty() && d.probes.is_empty()))
                .then(|| json!({"all": true}))
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
    v.push(cmd(
        "journal.undo",
        "cmd-undo",
        "edit",
        &["Ctrl+Z"],
        Dynamic("journal.undo", |s| {
            (!s.journal.undo.is_empty()).then(|| json!({}))
        }),
    ));
    v.push(cmd(
        "journal.redo",
        "cmd-redo",
        "edit",
        &["Ctrl+Shift+Z"],
        Dynamic("journal.redo", |s| {
            (!s.journal.redo.is_empty()).then(|| json!({}))
        }),
    ));
    // The steps to undo and redo; a click on one walks there.
    v.push(cmd(
        "window.undo_history",
        "cmd-undo-history",
        "edit",
        &[],
        Window(Some("journal.list")),
    ));
    // The view as a picture on the clipboard, as the viewport shows it.
    v.push(cmd(
        "window.copy_image",
        "cmd-copy-image",
        "edit",
        &["Ctrl+Shift+C"],
        Window(None),
    ));
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
    for p in murmel_engine::Placement::ALL {
        v.push(cmd(
            format!("settings.set_view_cube:{}", p.key()),
            format!("cube-placement-{}", p.key()),
            "settings",
            &[],
            Tool("settings.set_view_cube", json!({ "placement": p })),
        ));
    }
    v.push(cmd(
        "settings.set_view_cube:auto_hide",
        "cmd-cube-auto-hide",
        "settings",
        &[],
        Dynamic("settings.set_view_cube", |s| {
            Some(json!({"auto_hide": !s.view_cube.auto_hide}))
        }),
    ));
    v.push(cmd(
        "settings.set_agents",
        "cmd-agents",
        "settings",
        &[],
        Dynamic("settings.set_agents", |s| {
            Some(json!({"enabled": !s.agents}))
        }),
    ));
    v.push(cmd(
        "settings.set_colors:reset",
        "settings-reset-colors",
        "settings",
        &[],
        Tool("settings.set_colors", json!({"reset": true})),
    ));
    // Your defaults: the selected item's settings, the view's look, the
    // colouring of the field shown; or forget them all.
    v.push(cmd(
        "settings.set_default:selected",
        "cmd-defaults-selected",
        "settings",
        &[],
        Dynamic("settings.set_default", |s| {
            let m = s.mesh.as_ref()?;
            let item = s
                .display
                .selection
                .iter()
                .rev()
                .copied()
                .find(|&i| murmel_engine::defaults::Kind::of_item(i).is_some())?;
            Some(json!({"item": item.id(&m.mesh)}))
        }),
    ));
    v.push(cmd(
        "settings.set_default:view",
        "cmd-defaults-view",
        "settings",
        &[],
        Tool("settings.set_default", json!({"kind": "view"})),
    ));
    v.push(cmd(
        "settings.set_default:field_style",
        "cmd-defaults-field-style",
        "settings",
        &[],
        Dynamic("settings.set_default", |s| {
            let f = s.display.field.as_ref()?;
            s.mesh.as_ref()?.fields.get(&f.name)?;
            Some(json!({"kind": "field_style", "field": f.name}))
        }),
    ));
    v.push(cmd(
        "settings.reset_default",
        "cmd-defaults-reset",
        "settings",
        &[],
        Tool("settings.reset_default", json!({})),
    ));
    // Help: pages of the repository opened in the browser, and About.
    for id in [
        "window.help_tools",
        "window.help_agents",
        "window.report_issue",
        "window.about",
    ] {
        let label = format!("cmd-{}", id.trim_start_matches("window.").replace('_', "-"));
        v.push(cmd(id, label, "help", &[], Window(None)));
    }
    v
}

/// A new streamline set seeded on `kind`, when the open mesh has a
/// vector field and room for another set.
fn new_stream(s: &Session, kind: &str) -> Option<Value> {
    let m = s.mesh.as_ref()?;
    let vector = m
        .fields
        .names()
        .iter()
        .any(|n| m.fields.get(n).is_some_and(|f| f.is_vector()));
    (vector
        && s.display.streams.len() < murmel_engine::stream::MAX_STREAMS
        && murmel_engine::Seed::default_for(kind, &m.mesh).is_some())
    .then(|| json!({"new": true, "seed": kind}))
}

/// Tools no command calls, and why: they answer questions whose answers
/// the window already shows, or need an input only a panel can give.
pub const NOT_IN_PALETTE: &[(&str, &str)] = &[
    ("recent.list", "File ▸ Open recent shows it"),
    (
        "series.range",
        "the window reads it by itself while range mode steps colours a series",
    ),
    ("charts.data", "the chart panel draws it"),
    ("stats.at", "the statistics beside the chart show it"),
    (
        "slice.rotate",
        "the slice handle's tilt arcs call it, as for clip planes",
    ),
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
    (
        "region.list",
        "the model tree and the Properties panel show the regions",
    ),
    (
        "blocks.make",
        "a conversion for the command line and agents; the window offers it when a mesh too large for memory opens",
    ),
    (
        "field.list",
        "the colour-by menu and the legend show the fields",
    ),
    ("field.catalogue", "the Derived fields window shows it"),
    (
        "settings.keep_formula",
        "the Derived fields window keeps a formula among yours",
    ),
    (
        "settings.forget_formula",
        "the Derived fields window forgets one of your formulas",
    ),
    ("views.list", "the model tree shows the saved views"),
    (
        "views.rename",
        "needs a new name: the model tree renames (double-click or the context menu)",
    ),
    (
        "views.delete",
        "needs a view: the model tree's context menu deletes one",
    ),
    ("view.get_camera", "the view note and the cube show it"),
    ("inspect.item", "the Properties panel shows it"),
    ("settings.get", "the Settings window shows it"),
    (
        "settings.set_memory",
        "needs a size: the Settings window sets the budget, or gives it back to half the memory",
    ),
    (
        "journal.group",
        "the window groups each mouse gesture and run of wheel steps itself",
    ),
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
    (
        "inspect.quality",
        "the quality legend shows the range and the histogram",
    ),
    (
        "measure.snap",
        "needs a screen point: the measure mode asks while the pointer moves",
    ),
    (
        "measure.list",
        "the model tree lists measurements and probes",
    ),
    (
        "measure.rename",
        "needs a new name: the model tree renames (double-click or the context menu)",
    ),
    (
        "probe.at",
        "needs a point: a probe placed with Shift + P shows the same rows",
    ),
    (
        "probe.export",
        "needs a file name: the Probe history window exports CSV",
    ),
];

/// The command table with your keys (`[keys]` in settings.toml) laid
/// over the built-in ones; ids that are no command are left out (they
/// are reported by [`key_problems`]).
pub fn with_keys(map: &KeyMap) -> Vec<Command> {
    let mut all = commands();
    for c in &mut all {
        if let Some(k) = map.get(&c.id) {
            c.keys = k.clone();
        }
    }
    all
}

/// The commands other than `except` that `key` (canonical) runs now.
pub fn holders<'a>(all: &'a [Command], key: &str, except: &str) -> Vec<&'a Command> {
    all.iter()
        .filter(|c| c.id != except && c.keys.iter().any(|k| k == key))
        .collect()
}

/// Every key that runs more than one command, with their ids (a clash
/// only a hand-edited settings.toml can make; the window and
/// `settings.set_keys` refuse one).
pub fn clashes(all: &[Command]) -> Vec<(String, Vec<String>)> {
    let mut by: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
    for c in all {
        for k in &c.keys {
            by.entry(k.as_str()).or_default().push(c.id.clone());
        }
    }
    by.into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(k, ids)| (k.to_string(), ids))
        .collect()
}

/// What is wrong with a `[keys]` table, one line each, for the warning
/// at start: ids that are no command, keys that are not keys (or not in
/// their one form, or reserved), and keys that run two commands.
pub fn key_problems(map: &KeyMap) -> Vec<String> {
    let all = with_keys(map);
    let mut out = Vec::new();
    for (id, keys) in map {
        if !all.iter().any(|c| &c.id == id) {
            out.push(format!("settings.toml [keys]: {id} is no command"));
            continue;
        }
        for k in keys {
            match murmel_engine::keys::canonical(k) {
                Err(e) => out.push(format!("settings.toml [keys] {id}: {e}")),
                Ok(c) if c != *k => out.push(format!(
                    "settings.toml [keys] {id}: write {k:?} as {c:?}"
                )),
                Ok(c) if murmel_engine::keys::reserved(&c) => out.push(format!(
                    "settings.toml [keys] {id}: {c} moves the keyboard focus and cannot run a command"
                )),
                Ok(_) => {}
            }
        }
    }
    for (k, ids) in clashes(&all) {
        out.push(format!(
            "settings.toml [keys]: {k} runs {}; only the first acts",
            ids.join(" and ")
        ));
    }
    out
}

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
            (k.trim() == key).then(|| {
                v.trim()
                    .replace("{ $ctrl }", "Ctrl")
                    .replace("{ $alt }", "Alt")
            })
        })
        .unwrap_or_else(|| key.to_string())
}

/// `docs/keys.md`: the keyboard map, generated from the table.
pub fn keys_markdown() -> String {
    let label = english;
    let mut out = String::from(
        "# Keyboard map\n\nGenerated by `cargo run -p murmel-cli -- keys --markdown > docs/keys.md`\n\
         from `crates/murmel-tools/src/commands.rs`; a test fails when it is out of date.\n\
         On macOS `Ctrl` is the Command key (⌘ Cmd) and `Alt` the Option key (⌥ Option); the\n\
         window names them so. The views match the digit's place on the keyboard, so\n\
         Option + 7 works on layouts where it types a character. Every command is also in the command palette\n\
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
        assert_eq!(key_of("window.palette"), Some(vec!["Ctrl+K".to_string()]));
        assert_eq!(key_of("view.fit"), Some(vec!["F".to_string()]));
        assert_eq!(key_of("view.projection:next"), Some(vec!["5".to_string()]));
        assert_eq!(
            key_of("view.standard:front"),
            Some(vec!["Alt+1".to_string()])
        );
        assert_eq!(
            key_of("view.standard:bottom"),
            Some(vec!["Ctrl+Alt+7".to_string()])
        );
        assert_eq!(
            key_of("display.render_mode:feature_edges"),
            Some(vec!["4".to_string()])
        );
    }

    /// Invariant: every built-in key is in the one form of
    /// `murmel_engine::keys::canonical` and is not reserved, so a key you
    /// type in the editor or settings.toml compares equal to the table's
    /// and the clash check sees it. Breaks if a key is added as
    /// `Shift+Ctrl+X` or with a name the editor cannot record.
    #[test]
    fn builtin_keys_are_canonical() {
        for c in commands() {
            for k in &c.keys {
                assert_eq!(
                    murmel_engine::keys::canonical(k).as_deref(),
                    Ok(k.as_str()),
                    "{}",
                    c.id
                );
                assert!(!murmel_engine::keys::reserved(k), "{}", c.id);
            }
        }
    }

    /// Invariant: every command with a fixed input runs on a session with
    /// the channel open (its FoehnCraft result, so the field commands
    /// apply), and the selection commands run once something is
    /// selected; so no key or palette entry fails with a bad input.
    #[test]
    fn every_command_runs() {
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir =
            Some(std::env::temp_dir().join(format!("murmel-commands-{}", std::process::id())));
        let channel = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
        let open = |s: &mut Session| {
            assert!(crate::call("mesh.open", json!({"path": channel}), s).ok());
            assert!(crate::call("select.item", json!({"item": "patch:inlet"}), s).ok());
            // A cell and a zone too, for the region commands that take
            // the picked cells and the selected zone.
            let zone = format!("zone:{}", s.mesh.as_ref().unwrap().mesh.zones[0]);
            for item in ["cell:0".to_string(), zone] {
                let e = crate::call("select.item", json!({"item": item, "add": true}), s);
                assert!(e.ok(), "{item}: {:?}", e.diagnostics);
            }
            // A saved view, so the commands that step through them apply.
            assert!(crate::call("views.save", json!({}), s).ok());
            // Two clip planes, so removing the current one leaves one for
            // "remove all".
            for _ in 0..2 {
                assert!(crate::call("clip.set", json!({"new": true}), s).ok());
            }
            // Two slices and two iso-surfaces, for the same reason.
            for _ in 0..2 {
                assert!(crate::call("slice.set", json!({"new": true}), s).ok());
                assert!(crate::call("iso.set", json!({"new": true}), s).ok());
            }
            // A measurement (selected too) and a probe, so deleting the
            // selected ones and then all of them applies.
            let at = |n: u32| json!({ "node": n });
            let d = json!({"kind": "distance", "points": [at(0), at(1)]});
            assert!(crate::call("measure.add", d, s).ok());
            assert!(crate::call("probe.add", json!({"at": {"cell": 0}}), s).ok());
            // Spray parcels, for their commands.
            let parcels = std::env::temp_dir().join(format!(
                "murmel-commands-{}/parcels_000001.vtkhdf",
                std::process::id()
            ));
            std::fs::create_dir_all(parcels.parent().unwrap()).unwrap();
            let d = murmel_mesh::Field {
                name: "diameter".into(),
                unit: None,
                location: murmel_mesh::Location::Point,
                components: 1,
                values: vec![1e-4],
            };
            let at = s.mesh.as_ref().unwrap().mesh.bounds().center();
            murmel_mesh::parcels::write_parcels(
                &parcels,
                &[at],
                std::slice::from_ref(&d),
                0.1,
                1,
                "c",
            )
            .unwrap();
            let second = parcels.with_file_name("parcels_000002.vtkhdf");
            murmel_mesh::parcels::write_parcels(&second, &[at], &[d], 0.2, 2, "c").unwrap();
            // A time series of the two (the channel stays the mesh), for
            // the player's commands; it opens the first file's parcels.
            let dir = parcels.parent().unwrap();
            let e = crate::call("series.open", json!({"path": dir}), s);
            assert!(e.ok(), "{:?}", e.diagnostics);
            assert!(s.display.parcels.is_some());
            // A run's tables, for the chart commands.
            let run = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../test-fixtures/spray-run");
            assert!(crate::call("charts.open", json!({"path": run}), s).ok());
            // A slice selected, so its settings can become the defaults.
            assert!(crate::call("select.item", json!({"item": "slice:1", "add": true}), s).ok());
            assert!(crate::call("select.item", json!({"item": "measure:1", "add": true}), s).ok());
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
                // Undo and redo apply only with a step to walk; the
                // journal tests run them.
                None => assert!(
                    matches!(c.run, Run::Window(_))
                        || c.id == "select.clear"
                        || c.id.starts_with("journal."),
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
