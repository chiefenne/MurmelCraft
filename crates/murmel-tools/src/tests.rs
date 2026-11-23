use std::path::{Path, PathBuf};

use murmel_engine::{Session, Tokens};
use serde_json::json;

use crate::{call, codes, markdown, tools};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn session() -> Session {
    let mut s = Session::new(Tokens::builtin().unwrap());
    s.config_dir = None;
    s
}

/// Invariant: docs/tools.md is what `murmel commands --markdown` prints.
/// Regenerate with `cargo run -p murmel-cli -- commands --markdown > docs/tools.md`.
#[test]
fn tools_md_is_up_to_date() {
    let doc = std::fs::read_to_string(root().join("docs/tools.md")).unwrap_or_default();
    assert!(
        doc == markdown(),
        "docs/tools.md is out of date; run `cargo run -p murmel-cli -- commands --markdown > docs/tools.md`"
    );
}

/// Invariant: ids are unique `area.verb`, and every code a crate can report
/// is registered once (the mesh and render codes join the tools' own).
#[test]
fn ids_and_codes_are_unique() {
    let mut ids: Vec<_> = tools().iter().map(|t| t.id).collect();
    assert!(ids.iter().all(|i| i.split('.').count() == 2));
    ids.sort_unstable();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n);
    let mut c: Vec<_> = codes().iter().map(|c| c.code).collect();
    c.sort_unstable();
    let n = c.len();
    c.dedup();
    assert_eq!(c.len(), n);
    for src in [
        include_str!("tools.rs"),
        include_str!("../../murmel-mesh/src/vtu.rs"),
        include_str!("../../murmel-mesh/src/evidence.rs"),
        include_str!("../../murmel-render/src/lib.rs"),
    ] {
        for part in src.split('"').skip(1).step_by(2) {
            let is_code = part.len() == 7
                && part[..part.len() - 3]
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase())
                && part[part.len() - 3..].chars().all(|ch| ch.is_ascii_digit());
            if is_code {
                assert!(crate::code(part).is_some(), "{part} is not registered");
            }
        }
    }
}

/// Invariant: an unknown tool and a bad input fail with TOOL001 and
/// TOOL002 and exit code 2; a tool that needs a mesh says VIEW001.
#[test]
fn failures_have_codes() {
    let mut s = session();
    let e = call("view.spin", json!({}), &mut s);
    assert_eq!((e.exit_code, e.diagnostics[0].code), (2, "TOOL001"));
    let e = call("view.zoom", json!({"factor": "big"}), &mut s);
    assert_eq!((e.exit_code, e.diagnostics[0].code), (2, "TOOL002"));
    let e = call("view.orbit", json!({"yaw": 3}), &mut s);
    assert_eq!(e.diagnostics[0].code, "TOOL002");
    let e = call("view.fit", json!({}), &mut s);
    assert_eq!((e.exit_code, e.diagnostics[0].code), (1, "VIEW001"));
    let e = call("mesh.open", json!({"path": "/no/such.vtu"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "MESH001");
    let e = call("settings.set_theme", json!({"theme": "light"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "SET001");
}

/// Invariant: the camera tools compose as the mouse uses them: open fits,
/// orbit turns, zoom scales the distance, a face view is orthographic.
#[test]
fn open_and_navigate() {
    let mut s = session();
    let path = root().join("test-fixtures/channel/foehncraft.vtu");
    let e = call("mesh.open", json!({"path": path}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["summary"]["cells"], 776);
    assert_eq!(e.result["evidence"]["cells"], 776);
    let d0 = s.camera.distance;
    let e = call(
        "view.zoom",
        json!({"factor": 0.5, "cursor_ndc": [0.2, 0.1]}),
        &mut s,
    );
    assert!(e.ok());
    assert!((s.camera.distance - 0.5 * d0).abs() < 1e-9);
    let y0 = s.camera.yaw_deg;
    assert!(call("view.orbit", json!({"yaw_deg": 10}), &mut s).ok());
    assert!((s.camera.yaw_deg - y0 - 10.0).abs() < 1e-9);
    let e = call("view.standard", json!({"view": "top"}), &mut s);
    assert_eq!(e.result["orthographic"], true);
    assert!(call("view.orbit", json!({"pitch_deg": -5}), &mut s).ok());
    assert!(!s.camera.is_orthographic());
    let e = call(
        "view.set_camera",
        json!({"projection": "orthographic"}),
        &mut s,
    );
    assert_eq!(e.result["orthographic"], true);
}

/// Invariant: settings tools write their files in the config folder and
/// refuse a binding table that binds one input twice.
#[test]
fn settings_write_files() {
    let dir = std::env::temp_dir().join(format!("murmel-settings-{}", std::process::id()));
    let mut s = session();
    s.config_dir = Some(dir.clone());
    let e = call("settings.set_bindings", json!({"preset": "apple"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(s.navigation.preset, "apple");
    let text = std::fs::read_to_string(dir.join("navigation.toml")).unwrap();
    assert!(text.contains("apple"));
    let twice = json!({"bindings": [
        {"action": "orbit", "gesture": "drag", "button": "primary"},
        {"action": "pan", "gesture": "drag", "button": "primary"}
    ]});
    let e = call("settings.set_bindings", twice, &mut s);
    assert_eq!(e.diagnostics[0].code, "SET002");
    assert_eq!(s.navigation.preset, "apple");
    assert!(call("settings.set_theme", json!({"theme": "system"}), &mut s).ok());
    assert!(
        std::fs::read_to_string(dir.join("settings.toml"))
            .unwrap()
            .contains("system")
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Invariant: user mesh colours reach the tokens, are saved in
/// settings.toml beside the theme (neither setting drops the other) and
/// load back; a reset returns to the built-in colours; bad input is
/// refused without a change.
#[test]
fn colours_are_configurable_and_saved() {
    let dir = std::env::temp_dir().join(format!("murmel-colours-{}", std::process::id()));
    let mut s = session();
    s.config_dir = Some(dir.clone());
    let builtin = Tokens::builtin().unwrap();
    assert!(call("settings.set_theme", json!({"theme": "light"}), &mut s).ok());
    let e = call(
        "settings.set_colors",
        json!({"solid": "#112233", "role": {"inlet": "#445566"}}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(s.tokens.mesh.solid.hex(), "#112233");
    assert!(call("settings.set_theme", json!({"theme": "dark"}), &mut s).ok());
    let (st, _, problems) = murmel_engine::settings::load(&dir, false);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(st.theme, murmel_engine::Theme::Dark);
    assert_eq!(st.colors.solid.map(|c| c.hex()).as_deref(), Some("#112233"));
    assert_eq!(st.colors.role["inlet"].hex(), "#445566");
    for bad in [
        json!({"solid": "green"}),
        json!({"role": {"door": "#000000"}}),
        json!({"edge_darken": 2.0}),
        json!({"solids": "#000000"}),
        json!({"reset": "yes"}),
    ] {
        let e = call("settings.set_colors", bad.clone(), &mut s);
        assert_eq!(e.diagnostics[0].code, "TOOL002", "{bad}");
        assert_eq!(s.tokens.mesh.solid.hex(), "#112233", "{bad}");
    }
    assert!(call("settings.set_colors", json!({"reset": true}), &mut s).ok());
    assert_eq!(s.tokens, builtin);
    let _ = std::fs::remove_dir_all(dir);
}

/// Invariant: display.set_color colours one item of the open mesh, shows
/// in the surface and in the display state, and null takes it back.
#[test]
fn item_colours_reach_the_surface() {
    let mut s = session();
    let channel = root().join("test-fixtures/channel/foehncraft.vtu");
    assert!(call("mesh.open", json!({"path": channel}), &mut s).ok());
    let solid = murmel_engine::Rgb::linear(s.tokens.mesh.solid);
    let red = murmel_engine::Rgb([255, 0, 0]).linear();
    let colours = |s: &murmel_engine::Session| s.mesh.as_ref().unwrap().surface.colors.clone();
    assert!(colours(&s).iter().all(|c| *c == solid));
    let e = call(
        "display.set_color",
        json!({"item": "patch:outlet", "color": "#FF0000"}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["colors"]["patch:outlet"], "#FF0000");
    let n = colours(&s).iter().filter(|c| **c == red).count();
    assert!(n > 0 && n < colours(&s).len());
    let e = call(
        "display.set_color",
        json!({"item": "patch:outlet", "color": null}),
        &mut s,
    );
    assert!(e.ok());
    assert!(colours(&s).iter().all(|c| *c == solid));
    let e = call(
        "display.set_color",
        json!({"item": "patch:nope", "color": "#FF0000"}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "VIEW002");
}

/// Headless screenshot through the registry, as `murmel call` runs it.
#[test]
fn screenshot_writes_a_png() {
    let mut s = session();
    let out = root().join("target/render-test/tool-screenshot.png");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let e = call(
        "view.screenshot",
        json!({"path": out, "width": 64, "height": 48,
               "mesh": root().join("test-fixtures/box/foehncraft.vtu"), "view": "iso"}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["files"][0], json!(out));
    let (w, h, _) = murmel_render::read_png(&out).unwrap();
    assert_eq!((w, h), (64, 48));
    let e = call("view.screenshot", json!({"path": out, "width": 0}), &mut s);
    assert_eq!(e.diagnostics[0].code, "RENDER003");
}

/// Invariant (layering): the mesh and engine crates have no window and no
/// GPU, so everything they do runs headless; only murmel-render touches
/// wgpu and only murmel-app touches egui.
#[test]
fn engine_has_no_window_or_gpu() {
    for (krate, banned) in [
        (
            "murmel-mesh",
            &["wgpu", "egui", "eframe", "murmel-engine", "murmel-render"][..],
        ),
        (
            "murmel-engine",
            &["wgpu", "egui", "eframe", "murmel-render"][..],
        ),
        ("murmel-render", &["egui", "eframe"][..]),
        ("murmel-tools", &["egui", "eframe"][..]),
        ("murmel-cli", &["egui", "eframe", "wgpu"][..]),
    ] {
        let toml =
            std::fs::read_to_string(root().join(format!("crates/{krate}/Cargo.toml"))).unwrap();
        let deps = toml.split("[dependencies]").nth(1).unwrap_or("");
        let deps = deps.split("\n[").next().unwrap_or("");
        for b in banned {
            let found = deps
                .lines()
                .any(|l| l.split(['=', ' ', '.']).next() == Some(b));
            assert!(!found, "{krate} must not depend on {b}");
        }
    }
}

/// Invariant: the display tools drive what the viewport draws. Isolating
/// the walls draws only their 256 faces, the selection is reported by id,
/// colour by cell type is accepted, an unknown item says VIEW002, and the
/// properties of the mesh and of a patch come back as rows.
#[test]
fn display_selection_and_properties() {
    let mut s = session();
    let channel = root().join("test-fixtures/channel/foehncraft.vtu");
    let e = call("mesh.open", json!({"path": channel}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["validity"], "valid");
    assert!(
        e.result["items"]
            .as_array()
            .unwrap()
            .contains(&json!("kind:prism"))
    );

    let e = call(
        "display.visibility",
        json!({"action": "isolate", "items": ["patch:walls"]}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["visible_faces"], 256);
    assert_eq!(s.mesh.as_ref().unwrap().surface.faces, 256);
    let e = call(
        "display.visibility",
        json!({"action": "hide", "items": ["patch:nope"]}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "VIEW002");
    let e = call("display.visibility", json!({"action": "hide"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "TOOL002");
    let e = call("display.visibility", json!({"action": "show_all"}), &mut s);
    assert_eq!(e.result["visible_faces"], 426);

    let e = call("display.color_by", json!({"by": "cell_type"}), &mut s);
    assert_eq!(e.result["color_by"], "cell_type");
    let e = call("display.color_by", json!({"by": "quality"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "TOOL002");

    let e = call("select.item", json!({"item": "patch:inlet"}), &mut s);
    assert_eq!(e.result["selection"], "patch:inlet");
    let e = call("inspect.item", json!({}), &mut s);
    assert_eq!(e.result["item"], "patch:inlet");
    let rows = e.result["rows"].as_array().unwrap();
    let get = |k: &str| {
        rows.iter()
            .find(|r| r["key"] == k)
            .map(|r| r["value"].clone())
    };
    assert_eq!(get("faces"), Some(json!(85)));
    assert_eq!(get("foehncraft_type"), Some(json!("velocity-inlet")));
    let e = call("view.fit", json!({"item": "patch:inlet"}), &mut s);
    assert!(e.ok());

    call("select.clear", json!({}), &mut s);
    let e = call("inspect.item", json!({}), &mut s);
    assert_eq!(e.result["item"], "mesh");
    let rows = e.result["rows"].as_array().unwrap();
    let get = |k: &str| {
        rows.iter()
            .find(|r| r["key"] == k)
            .map(|r| r["value"].clone())
    };
    assert_eq!(get("faces_interior"), Some(json!(1747)));
    assert_eq!(get("validity"), Some(json!("valid")));
}
