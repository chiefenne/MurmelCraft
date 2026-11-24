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
        include_str!("../../murmel-mesh/src/vtkhdf.rs"),
        include_str!("../../murmel-mesh/src/hdf5.rs"),
        include_str!("../../murmel-mesh/src/evidence.rs"),
        include_str!("../../murmel-render/src/lib.rs"),
    ] {
        for part in src.split('"').skip(1).step_by(2) {
            // Codes are an upper-case area and three digits: SET001,
            // TOOL001, FIELD001.
            let is_code = (6..=8).contains(&part.len())
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

/// Invariant: settings.set_view_cube changes only the fields it is given,
/// is saved in settings.toml beside the theme and loads back, and refuses
/// an unknown placement or a distance out of range without a change.
/// Breaks when the cube's settings drop the theme (or the other way) or a
/// bad distance is stored; the engine tests check placement geometry only.
#[test]
fn view_cube_placement_is_saved() {
    let dir = std::env::temp_dir().join(format!("murmel-cube-{}", std::process::id()));
    let mut s = session();
    s.config_dir = Some(dir.clone());
    assert!(call("settings.set_theme", json!({"theme": "light"}), &mut s).ok());
    let e = call(
        "settings.set_view_cube",
        json!({"placement": "floating", "position": [40.0, 50.0]}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["view_cube"]["placement"], "floating");
    let e = call(
        "settings.set_view_cube",
        json!({"auto_hide": true, "distance_px": 80.0}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(s.view_cube.placement, murmel_engine::Placement::Floating);
    assert_eq!(s.view_cube.position, Some([40.0, 50.0]));
    let (st, _, problems) = murmel_engine::settings::load(&dir, false);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(st.theme, murmel_engine::Theme::Light);
    assert_eq!(st.view_cube, s.view_cube);
    for bad in [
        json!({"placement": "middle"}),
        json!({"distance_px": -1.0}),
        json!({"distance_px": 5000.0}),
        json!({"position": [1.0]}),
        json!({"size": 3}),
    ] {
        let e = call("settings.set_view_cube", bad.clone(), &mut s);
        assert_eq!(e.diagnostics[0].code, "TOOL002", "{bad}");
        assert_eq!(s.view_cube, st.view_cube, "{bad}");
    }
    let e = call("settings.get", json!({}), &mut s);
    assert_eq!(e.result["view_cube"]["distance_px"], 80.0);
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
/// wgpu and only murmel-app touches egui. The MCP server, like the CLI,
/// reaches the renderer only through the tools.
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
        (
            "murmel-mcp",
            &["egui", "eframe", "wgpu", "murmel-render"][..],
        ),
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
    let e = call("display.color_by", json!({"by": "speed"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "TOOL002");

    let e = call("select.item", json!({"item": "patch:inlet"}), &mut s);
    assert_eq!(e.result["selection"], json!(["patch:inlet"]));
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

/// Invariant (V0.4 clip plane): `clip.set` by axis cuts the channel
/// through its centre and keeps whole cells (more than half, fewer than
/// all); flip keeps the other side, so the two sides cover every cell
/// with the crossed cells in both; an offset at the end of the range keeps
/// everything or almost nothing; a zero normal or axis plus normal is
/// refused; `clip.clear` and opening a mesh bring back every cell.
#[test]
fn clip_plane_tools() {
    let mut s = session();
    let ch = root().join("test-fixtures/channel/foehncraft.vtu");
    assert!(!call("clip.set", json!({"axis": "x"}), &mut s).ok());
    assert!(call("mesh.open", json!({"path": ch}), &mut s).ok());
    let e = call("clip.set", json!({"axis": "x"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let clip = &e.result["clip"];
    // The home view looks from +x (right, front, top), so the +x half
    // (towards the eye) goes; from the left the -x half goes.
    assert_eq!(clip["normal"], json!([1.0, 0.0, 0.0]));
    assert!(call("view.standard", json!({"view": "left"}), &mut s).ok());
    let e = call("clip.set", json!({"axis": "x"}), &mut s);
    assert_eq!(e.result["clip"]["normal"], json!([-1.0, 0.0, 0.0]));
    let e = call("clip.set", json!({"axis": "x", "flip": true}), &mut s);
    assert_eq!(clip["mode"], "crinkle");
    let a = e.result["visible_cells"].as_u64().unwrap();
    assert!(a > 388 && a < 776, "{a}");
    let e = call("clip.set", json!({"flip": true}), &mut s);
    let b = e.result["visible_cells"].as_u64().unwrap();
    assert!(a + b > 776, "{a} + {b}");
    let range = e.result["clip"]["range"].clone();
    let (lo, hi) = (range[0].as_f64().unwrap(), range[1].as_f64().unwrap());
    // The plane moves along its normal (-x now) by the offset: at the high
    // end it sits at the low x end and cuts nothing away; at the low end
    // only the cells it crosses stay.
    let e = call("clip.set", json!({"offset": hi, "mode": "flat"}), &mut s);
    assert_eq!(e.result["clip"]["mode"], "flat");
    assert_eq!(e.result["visible_cells"], 776);
    let e = call("clip.set", json!({"offset": lo}), &mut s);
    let few = e.result["visible_cells"].as_u64().unwrap();
    assert!(few > 0 && few < 200, "{few}");
    for bad in [
        json!({"normal": [0, 0, 0]}),
        json!({"axis": "y", "normal": [0, 1, 0]}),
        json!({"origin": [0, 0, 0], "offset": 1}),
    ] {
        let e = call("clip.set", bad.clone(), &mut s);
        assert_eq!(e.diagnostics[0].code, "TOOL002", "{bad}");
    }
    let e = call("clip.set", json!({"enabled": false}), &mut s);
    assert_eq!(e.result["visible_cells"], 776);
    assert_eq!(e.result["clip"]["enabled"], false);
    let e = call("clip.clear", json!({}), &mut s);
    assert_eq!(e.result["clip"], serde_json::Value::Null);
    assert!(call("clip.set", json!({"axis": "view"}), &mut s).ok());
    assert!(call("mesh.open", json!({"path": ch}), &mut s).ok());
    let e = call("inspect.item", json!({}), &mut s);
    assert!(e.ok());
    assert!(s.display.clips.is_empty());
}

/// Invariant (V1.0 clip handle): `clip.rotate` tilts the plane about a
/// world vector or a screen axis through the pivot (by default inside
/// the mesh, so cells stay on both sides), a drag of many small steps in
/// one undo group is one undo step back to the plane before it, and the
/// Alt + arrow commands send screen-axis tilts that turn the normal the
/// way they say (left: towards the screen's left). Without a plane, a
/// zero axis or a non-finite angle it is refused. Breaks if the rotation
/// sense, the pivot or the journal entry change; the engine test checks
/// the geometry only, not the tool, the keys or the undo.
#[test]
fn clip_rotate_tool() {
    let mut s = session();
    let ch = root().join("test-fixtures/channel/foehncraft.vtu");
    assert!(call("mesh.open", json!({"path": ch}), &mut s).ok());
    let e = call("clip.rotate", json!({"axis": "y", "deg": 5}), &mut s);
    assert_eq!(e.diagnostics[0].code, "TOOL002", "no plane yet");
    assert!(call("view.standard", json!({"view": "front"}), &mut s).ok());
    assert!(call("clip.set", json!({"normal": [1, 0, 0]}), &mut s).ok());
    let e = call("clip.rotate", json!({"axis": [0, 0, 1], "deg": 90}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let n = s.display.clip().unwrap().normal;
    assert!((n[1] - 1.0).abs() < 1e-9, "x turns to y about z: {n:?}");
    let shown = e.result["visible_cells"].as_u64().unwrap();
    assert!(shown > 100 && shown < 776, "{shown}");
    for bad in [
        json!({"axis": [0, 0, 0], "deg": 5}),
        json!({"axis": "y"}),
        json!({"axis": "w", "deg": 5}),
        json!({"axis": "x", "deg": 5, "pivot": [0, 0]}),
    ] {
        let e = call("clip.rotate", bad.clone(), &mut s);
        assert!(!e.ok(), "{bad}");
    }
    // A drag: many small tilts in one group, undone in one step.
    assert!(call("clip.set", json!({"normal": [1, 0, 0]}), &mut s).ok());
    let start = *s.display.clip().unwrap();
    assert!(call("journal.group", json!({"open": true}), &mut s).ok());
    let pivot = start.pivot(&s.mesh.as_ref().unwrap().mesh.bounds());
    for _ in 0..12 {
        let e = call(
            "clip.rotate",
            json!({"axis": [0, 1, 0], "deg": 2.5, "pivot": pivot}),
            &mut s,
        );
        assert!(e.ok());
    }
    assert!(call("journal.group", json!({"open": false}), &mut s).ok());
    let tilted = *s.display.clip().unwrap();
    let cos: f64 = (0..3).map(|k| tilted.normal[k] * start.normal[k]).sum();
    assert!((cos - 30f64.to_radians().cos()).abs() < 1e-9, "{cos}");
    assert!(
        tilted.distance(pivot).abs() < 1e-9,
        "the pivot stays on the plane"
    );
    assert!(call("journal.undo", json!({}), &mut s).ok());
    assert_eq!(s.display.clip().copied(), Some(start));
    // Keys: Alt + Left turns the normal towards the screen's left,
    // Alt + Up towards its top.
    let cmds = crate::commands::commands();
    let run = |id: &str, s: &mut Session| {
        let c = cmds.iter().find(|c| c.id == id).unwrap();
        let (tool, input) = c.input(s).unwrap();
        assert!(call(tool, input, s).ok());
    };
    let (right, up) = s.camera.basis();
    let (right, up) = (right.to_array(), up.to_array());
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let n0 = s.display.clip().unwrap().normal;
    run("clip.rotate:left", &mut s);
    let n1 = s.display.clip().unwrap().normal;
    assert!(dot(n1, right) < dot(n0, right), "left: {n0:?} -> {n1:?}");
    run("clip.rotate:up", &mut s);
    let n2 = s.display.clip().unwrap().normal;
    assert!(dot(n2, up) > dot(n1, up), "up: {n1:?} -> {n2:?}");
}

/// Invariant: quality colouring and its threshold go through tools only.
/// A threshold hides the cells below it (fewer cells shown, and exactly as
/// many as inspect.quality counts at or above it), null shows them all
/// again, and changing the metric drops the threshold. Breaks when the
/// threshold is applied to another metric or not to the visible counts;
/// the engine tests check the values, not the tools.
#[test]
fn quality_colouring_and_threshold() {
    let mut s = session();
    let channel = root().join("test-fixtures/channel/foehncraft.vtu");
    assert!(call("mesh.open", json!({"path": channel}), &mut s).ok());
    let e = call(
        "display.color_by",
        json!({"by": "quality", "metric": "volume_ratio"}),
        &mut s,
    );
    assert_eq!(e.result["color_by"], "quality");
    assert_eq!(e.result["quality"]["metric"], "volume_ratio");
    let range = e.result["quality"]["range"].as_array().unwrap().clone();
    assert!(range[0].as_f64().unwrap() >= 1.0);
    let all = e.result["visible_cells"].as_u64().unwrap();
    let q = call("inspect.quality", json!({"bins": 8}), &mut s);
    assert_eq!(q.result["cells"].as_u64(), Some(all));
    assert_eq!(q.result["histogram"].as_array().unwrap().len(), 8);
    let mid = (q.result["min"].as_f64().unwrap() + q.result["max"].as_f64().unwrap()) / 2.0;
    let q = call("inspect.quality", json!({"above": mid}), &mut s);
    let above = q.result["above"]["cells"].as_u64().unwrap();
    let e = call("display.threshold", json!({"value": mid}), &mut s);
    let shown = e.result["visible_cells"].as_u64().unwrap();
    assert_eq!(shown, above);
    assert!(shown < all, "{shown} of {all}");
    assert_eq!(e.result["quality"]["threshold"], json!(mid));
    let e = call("display.threshold", json!({"value": null}), &mut s);
    assert_eq!(e.result["visible_cells"].as_u64(), Some(all));
    call("display.threshold", json!({"value": mid}), &mut s);
    let e = call(
        "display.color_by",
        json!({"by": "quality", "metric": "skewness"}),
        &mut s,
    );
    assert_eq!(e.result["quality"]["threshold"], serde_json::Value::Null);
    assert_eq!(e.result["visible_cells"].as_u64(), Some(all));
    let e = call("inspect.quality", json!({"bins": 0}), &mut s);
    assert_eq!(e.diagnostics[0].code, "TOOL002");
}

/// Invariant: view.orbit about a pivot moves the target (the eye turns
/// about the pivot), about the target it does not, and a pivot that is
/// not finite is refused. Breaks when the pivot is ignored by the tool;
/// the camera test checks only the engine.
#[test]
fn orbit_about_a_pivot() {
    let mut s = session();
    let e = call("view.orbit", json!({"yaw_deg": 30.0}), &mut s);
    let target = e.result["target"].clone();
    let e = call(
        "view.orbit",
        json!({"yaw_deg": 30.0, "pivot": target}),
        &mut s,
    );
    assert_eq!(e.result["target"], target);
    let e = call(
        "view.orbit",
        json!({"yaw_deg": 30.0, "pivot": [5.0, 0.0, 0.0]}),
        &mut s,
    );
    assert_ne!(e.result["target"], target);
    let e = call(
        "view.orbit",
        json!({"yaw_deg": 1.0, "pivot": [1.0, 2.0]}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "TOOL002");
}

/// Invariant: every tool is either a journal step (JOURNALED) or listed
/// in NOT_JOURNALED with a reason, never both. Breaks when a new tool
/// that changes the view is added without deciding whether undo covers
/// it.
#[test]
fn every_tool_decides_on_the_journal() {
    for t in crate::tools() {
        let j = crate::JOURNALED.contains(&t.id);
        let n = crate::NOT_JOURNALED.iter().any(|(id, _)| *id == t.id);
        assert!(j != n, "{}: journalled {j}, not journalled {n}", t.id);
    }
    for id in crate::JOURNALED
        .iter()
        .chain(crate::NOT_JOURNALED.iter().map(|(id, _)| id))
    {
        assert!(crate::find(id).is_some(), "{id} is no tool");
    }
}

/// Invariant: through the registry, undo and redo walk the camera and
/// display steps, keep the selection, make one step of a group (a mouse
/// drag), and a new mesh starts an empty journal. Breaks when a tool call
/// stops recording its step, when a drag needs one undo per frame, or
/// when undo drops the selection; the engine journal tests call the
/// journal directly, not through the tools.
#[test]
fn undo_and_redo_through_the_tools() {
    let mut s = session();
    let path = root().join("test-fixtures/channel/foehncraft.vtu");
    assert!(call("mesh.open", json!({"path": path}), &mut s).ok());
    let cam0 = s.camera.clone();
    assert!(call("journal.group", json!({"open": true}), &mut s).ok());
    for _ in 0..20 {
        assert!(
            call(
                "view.orbit",
                json!({"yaw_deg": 2.0, "pitch_deg": 1.0}),
                &mut s
            )
            .ok()
        );
    }
    assert!(call("journal.group", json!({"open": false}), &mut s).ok());
    let cam1 = s.camera.clone();
    assert!(call("display.color_by", json!({"by": "patch"}), &mut s).ok());
    assert!(call("select.item", json!({"item": "patch:inlet"}), &mut s).ok());
    let selection = s.display.selection.clone();
    let e = call("journal.list", json!({}), &mut s);
    assert_eq!(e.result["undo"], json!(["view.orbit", "display.color_by"]));
    let e = call("journal.undo", json!({}), &mut s);
    assert_eq!(e.result["undone"], "display.color_by");
    assert_eq!(s.display.color_by, murmel_engine::ColorBy::Solid);
    assert_eq!(s.display.selection, selection);
    let e = call("journal.undo", json!({}), &mut s);
    assert_eq!(e.result["undone"], "view.orbit");
    assert_eq!(s.camera, cam0);
    let e = call("journal.undo", json!({}), &mut s);
    assert!(e.ok());
    assert_eq!(e.result["undone"], serde_json::Value::Null);
    let e = call("journal.redo", json!({}), &mut s);
    assert_eq!(e.result["redone"], "view.orbit");
    assert_eq!(s.camera, cam1);
    assert_eq!(e.result["redo"], 1);
    assert!(call("mesh.open", json!({"path": path}), &mut s).ok());
    let e = call("journal.list", json!({}), &mut s);
    assert_eq!(e.result["undo"], json!([]));
    assert_eq!(e.result["redo"], json!([]));
}

/// Invariant: `mesh.open` opens an OpenFOAM case folder through the same
/// tool as a .vtu, and the viewer's tools work on it: patch and zone ids,
/// quality colouring, the clip plane and the properties (format and
/// OpenFOAM patch type). Breaks when a front end needs another tool for
/// polyMesh, or when the polyMesh topology is not what the clip and the
/// quality walk; the reader tests check the mesh, not the tools.
#[test]
fn opens_an_openfoam_case() {
    let mut s = session();
    let case = root().join("test-fixtures/openfoam/channel_binary");
    let e = call("mesh.open", json!({"path": case}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["format"], "openfoam");
    assert_eq!(e.result["summary"]["cells"], 16);
    assert_eq!(e.result["files"].as_array().map(Vec::len), Some(6));
    let items = e.result["items"].as_array().unwrap();
    for id in ["patch:walls", "patch:sides", "zone:left", "zone:right"] {
        assert!(items.contains(&json!(id)), "{id}");
    }
    let e = call("display.color_by", json!({"by": "quality"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let q = call("inspect.quality", json!({}), &mut s);
    assert_eq!(q.result["cells"], 16);
    let e = call("clip.set", json!({"axis": "x"}), &mut s);
    let shown = e.result["visible_cells"].as_u64().unwrap();
    assert!(shown > 0 && shown < 16, "{shown}");
    call("clip.clear", json!({}), &mut s);
    let e = call("inspect.item", json!({"item": "patch:sides"}), &mut s);
    let rows = e.result["rows"].as_array().unwrap();
    let get = |k: &str| {
        rows.iter()
            .find(|r| r["key"] == k)
            .map(|r| r["value"].clone())
    };
    assert_eq!(get("openfoam_type"), Some(json!("empty")));
    assert_eq!(get("faces"), Some(json!(16)));
    let e = call("inspect.item", json!({"item": "mesh"}), &mut s);
    let rows = e.result["rows"].as_array().unwrap();
    let get = |k: &str| {
        rows.iter()
            .find(|r| r["key"] == k)
            .map(|r| r["value"].clone())
    };
    assert_eq!(get("format"), Some(json!("OpenFOAM polyMesh (binary)")));
    assert_eq!(get("validity"), Some(json!("no_evidence")));
    let e = call(
        "mesh.open",
        json!({"path": root().join("test-fixtures")}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "MESH013");
}

/// Invariant: every input field that names a file or folder is declared,
/// in `reads` (read) or `outputs` (written), and every declared field is in
/// the tool's schema. The MCP server confines exactly these fields to its
/// roots; a new path field left undeclared would let an agent read or
/// write anywhere, and no other test looks at the schemas this way.
#[test]
fn every_path_field_is_declared() {
    const PATH_NAMES: &[&str] = &["path", "mesh", "file", "folder", "dir", "output"];
    for t in tools() {
        let schema = t.input_schema();
        let props = schema["properties"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        for name in props.keys() {
            let pathlike = PATH_NAMES
                .iter()
                .any(|p| name == p || name.ends_with(&format!("_{p}")));
            if pathlike {
                assert!(
                    t.reads.contains(&name.as_str()) || t.outputs.contains(&name.as_str()),
                    "{}: field {name} names a path but is in neither reads nor outputs",
                    t.id
                );
            }
        }
        for f in t.reads.iter().chain(t.outputs) {
            assert!(
                props.contains_key(*f),
                "{}: declared field {f} is not in the schema",
                t.id
            );
            assert_eq!(
                props[*f]["type"], "string",
                "{}: {f} must be a string",
                t.id
            );
        }
        assert!(
            t.reads.iter().all(|f| !t.outputs.contains(f)),
            "{}: a field is both read and written",
            t.id
        );
    }
}

/// Invariant: a tool marked reads only (the MCP server tells clients it is
/// read-only and idempotent) changes neither the camera, the display, the
/// selection nor the journal, and writes no file. Breaks when such a tool
/// starts changing the session while its reason still says reads only.
#[test]
fn reads_only_tools_change_nothing() {
    let mut s = session();
    let channel = root().join("test-fixtures/channel/foehncraft.vtu");
    assert!(call("mesh.open", json!({"path": channel}), &mut s).ok());
    assert!(call("select.item", json!({"item": "patch:walls"}), &mut s).ok());
    assert!(
        call(
            "view.orbit",
            json!({"yaw_deg": 10.0, "pitch_deg": 5.0}),
            &mut s
        )
        .ok()
    );
    let mut ran = 0;
    for t in tools().iter().filter(|t| crate::reads_only(t.id)) {
        assert_eq!(t.action, crate::Action::Read, "{}", t.id);
        assert!(t.outputs.is_empty(), "{}", t.id);
        let before = (
            murmel_engine::Snapshot::of(&s),
            s.display.selection.clone(),
            s.journal.clone(),
        );
        // Inputs that need a point take the centre of the view.
        let input = match t.id {
            "inspect.at" | "measure.snap" => json!({"ndc": [0.0, 0.0]}),
            "probe.at" => json!({"at": {"ndc": [0.0, 0.0]}}),
            _ => json!({}),
        };
        let e = call(t.id, input, &mut s);
        assert!(e.ok(), "{}: {:?}", t.id, e.diagnostics);
        let after = (
            murmel_engine::Snapshot::of(&s),
            s.display.selection.clone(),
            s.journal.clone(),
        );
        assert!(before == after, "{} changed the session", t.id);
        ran += 1;
    }
    assert!(ran >= 10, "only {ran} reads-only tools");
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("murmel-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A copy of the channel and its evidence in `dir`.
fn channel_case(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    for f in ["foehncraft.vtu", "mesh-evidence.json"] {
        std::fs::copy(root().join("test-fixtures/channel").join(f), dir.join(f)).unwrap();
    }
    dir.join("foehncraft.vtu")
}

/// Invariant: session.load in a fresh session gives back what
/// session.save wrote: camera, display (colour mode, quality metric and
/// threshold, render mode, hidden, isolated and coloured items, clip,
/// measurements and probes), the selection, the mesh colours and the view cube placement, with an
/// empty journal; saving again writes the same file. Breaks when a field
/// of the display is added to the session but not to the file, or when
/// loading leaves a part of the state at its default; no other test
/// reads a session file back.
#[test]
fn sessions_round_trip() {
    let dir = scratch("session-trip");
    let mesh = channel_case(&dir.join("case"));
    let file = dir.join("case/view.murmel");
    let mut s = session();
    for (tool, input) in [
        ("mesh.open", json!({"path": mesh})),
        ("view.orbit", json!({"yaw_deg": 33.0, "pitch_deg": -12.5})),
        ("view.zoom", json!({"factor": 0.7})),
        ("views.save", json!({"name": "Close up"})),
        ("view.standard", json!({"view": "top"})),
        ("views.save", json!({})),
        ("view.orbit", json!({"yaw_deg": 5.0})),
        ("display.render_mode", json!({"mode": "shaded"})),
        (
            "display.color_by",
            json!({"by": "quality", "metric": "skewness"}),
        ),
        ("display.threshold", json!({"value": 0.2})),
        (
            "display.visibility",
            json!({"action": "hide", "items": ["patch:walls"]}),
        ),
        (
            "display.set_color",
            json!({"item": "patch:outlet", "color": "#123456"}),
        ),
        (
            "clip.set",
            json!({"axis": "x", "mode": "flat", "offset": 0.01}),
        ),
        (
            "measure.add",
            json!({"kind": "angle", "points": [{"node": 0}, {"face": 3}, {"point": [0.1, 0.2, 0.3]}], "name": "Corner"}),
        ),
        (
            "measure.add",
            json!({"kind": "distance", "points": [{"node": 0}, {"cell": 5}]}),
        ),
        (
            "display.visibility",
            json!({"action": "hide", "items": ["measure:2"]}),
        ),
        ("probe.add", json!({"at": {"point": [0.5, 0.25, 0.25]}})),
        ("select.item", json!({"item": "patch:inlet"})),
        ("select.item", json!({"item": "cell:7", "add": true})),
        ("select.item", json!({"item": "probe:1", "add": true})),
        (
            "settings.set_view_cube",
            json!({"placement": "bottom_left"}),
        ),
        ("settings.set_colors", json!({"solid": "#336699"})),
    ] {
        let e = call(tool, input, &mut s);
        // The settings tools have no config folder here (SET001) but
        // apply to the session.
        assert!(
            e.ok() || e.diagnostics[0].code == "SET001",
            "{tool}: {:?}",
            e.diagnostics
        );
    }
    let e = call("session.save", json!({"path": file}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["session"]["mesh"]["path"], "foehncraft.vtu");
    assert_eq!(s.display.views.len(), 2);

    let mut t = session();
    let e = call("session.load", json!({"path": file}), &mut t);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["warnings"], json!([]));
    assert_eq!(
        murmel_engine::Snapshot::of(&t),
        murmel_engine::Snapshot::of(&s)
    );
    assert_eq!(t.display.selection, s.display.selection);
    assert_eq!(t.display.pick, s.display.pick);
    assert_eq!(t.view_cube, s.view_cube);
    assert_eq!(t.colors, s.colors);
    assert_eq!(t.tokens.mesh.solid, s.tokens.mesh.solid);
    assert_eq!(t.display.views, s.display.views, "saved views come back");
    assert_eq!(t.display.measurements.len(), 2, "measurements come back");
    assert_eq!(t.display.probes, s.display.probes, "probes come back");
    assert!(t.display.quality.is_some(), "quality colouring computes it");
    assert!(t.journal.undo.is_empty());
    let again = dir.join("again.murmel");
    assert!(call("session.save", json!({"path": again}), &mut t).ok());
    let a: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let mut b: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&again).unwrap()).unwrap();
    // Saved one folder up, so only the relative path differs.
    assert_eq!(b["mesh"]["path"], "case/foehncraft.vtu");
    b["mesh"]["path"] = a["mesh"]["path"].clone();
    assert_eq!(a, b);

    // A session loaded over the same mesh keeps it open (no re-read)
    // and still puts the view back.
    assert!(call("view.orbit", json!({"yaw_deg": 90.0}), &mut t).ok());
    let rev = t.surface_revision;
    assert!(call("session.load", json!({"path": file}), &mut t).ok());
    assert_eq!(t.camera, s.camera);
    assert_eq!(t.surface_revision, rev + 1, "one redraw, no new mesh");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant: a case folder moved as a whole opens through the relative
/// path, a session file moved on its own through the absolute one.
/// Breaks when only one of the two paths is kept or tried; the round
/// trip test never moves a file.
#[test]
fn moved_sessions_find_their_mesh() {
    let dir = scratch("session-moved");
    let mesh = channel_case(&dir.join("a"));
    let mut s = session();
    assert!(call("mesh.open", json!({"path": mesh}), &mut s).ok());
    assert!(
        call(
            "session.save",
            json!({"path": dir.join("a/s.murmel")}),
            &mut s
        )
        .ok()
    );
    std::fs::copy(dir.join("a/s.murmel"), dir.join("alone.murmel")).unwrap();
    std::fs::rename(dir.join("a"), dir.join("b")).unwrap();

    let mut t = session();
    let e = call(
        "session.load",
        json!({"path": dir.join("b/s.murmel")}),
        &mut t,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(t.mesh.as_ref().unwrap().path, dir.join("b/foehncraft.vtu"));

    // The absolute path (now gone) is the only one left: MESH001 names
    // the session's mesh, and the session is unchanged.
    let e = call(
        "session.load",
        json!({"path": dir.join("alone.murmel")}),
        &mut t,
    );
    assert_eq!(e.diagnostics[0].code, "MESH001", "{:?}", e.diagnostics);
    assert!(e.diagnostics[0].message.starts_with("the session's mesh"));
    assert_eq!(t.mesh.as_ref().unwrap().path, dir.join("b/foehncraft.vtu"));
    std::fs::rename(dir.join("b"), dir.join("a")).unwrap();
    let e = call(
        "session.load",
        json!({"path": dir.join("alone.murmel")}),
        &mut t,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(t.mesh.as_ref().unwrap().path, dir.join("a/foehncraft.vtu"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant: a mesh changed since the save warns SESS003, items it no
/// longer has are skipped with SESS004 and listed, and the rest of the
/// session still loads; a broken or foreign file fails SESS001 and an
/// unwritable path SESS002. Breaks when a stale session loads silently,
/// or when one unknown id fails the whole load.
#[test]
fn stale_and_broken_sessions() {
    let dir = scratch("session-stale");
    let mesh = channel_case(&dir);
    let file = dir.join("s.murmel");
    let mut s = session();
    assert!(call("mesh.open", json!({"path": mesh}), &mut s).ok());
    assert!(
        call(
            "display.visibility",
            json!({"action": "hide", "items": ["patch:walls"]}),
            &mut s
        )
        .ok()
    );
    assert!(call("session.save", json!({"path": file}), &mut s).ok());
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    v["mesh"]["sha256"] = "00".into();
    v["display"]["hidden"] = json!(["patch:walls", "patch:gone", "cell:999999"]);
    std::fs::write(&file, v.to_string()).unwrap();

    let mut t = session();
    let e = call("session.load", json!({"path": file}), &mut t);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let codes: Vec<_> = e.result["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].clone())
        .collect();
    assert_eq!(codes, [json!("SESS003"), json!("SESS004")]);
    assert_eq!(e.result["skipped"], json!(["patch:gone", "cell:999999"]));
    assert_eq!(e.result["display"]["hidden"], json!(["patch:walls"]));

    let e = call("session.load", json!({"path": mesh}), &mut t);
    assert_eq!(e.diagnostics[0].code, "SESS001");
    let e = call(
        "session.load",
        json!({"path": dir.join("none.murmel")}),
        &mut t,
    );
    assert_eq!(e.diagnostics[0].code, "SESS001");
    let e = call(
        "session.save",
        json!({"path": dir.join("no/such/dir.murmel")}),
        &mut t,
    );
    assert_eq!(e.diagnostics[0].code, "SESS002");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant: saved views are named cameras of the open mesh: views.save
/// names a new one "View N" or updates the one with the name given,
/// views.go puts its camera back, rename refuses a taken or empty name
/// (VIEW004) and an unknown view is VIEW003; save, rename and delete are
/// undo steps that do not rebuild the surface, and opening a mesh drops
/// them. The V and Shift + V commands cycle through them. Breaks when a
/// view keeps a copy of the camera that later moves with it, when the
/// journal leaves views out, or when undoing a bookmark redraws the
/// mesh; the session round trip only checks they are saved.
#[test]
fn saved_views() {
    let mut s = session();
    let path = root().join("test-fixtures/channel/foehncraft.vtu");
    assert_eq!(
        call("views.save", json!({}), &mut s).diagnostics[0].code,
        "VIEW001"
    );
    assert!(call("mesh.open", json!({"path": path}), &mut s).ok());
    let home = s.camera.clone();
    let e = call("views.save", json!({}), &mut s);
    assert_eq!(e.result["name"], "View 1");
    assert_eq!(e.result["updated"], false);
    assert!(call("view.standard", json!({"view": "top"}), &mut s).ok());
    let top = s.camera.clone();
    let e = call("views.save", json!({"name": "  Top  "}), &mut s);
    assert_eq!(e.result["name"], "Top");
    assert_eq!(e.result["views"], json!(["View 1", "Top"]));
    assert_eq!(
        call("views.save", json!({}), &mut s).result["name"],
        "View 2"
    );
    assert!(call("view.orbit", json!({"yaw_deg": 40.0}), &mut s).ok());
    let e = call("views.save", json!({"name": "View 2"}), &mut s);
    assert_eq!(e.result["updated"], true);
    assert_eq!(s.display.view("View 2").unwrap().camera, s.camera);

    let e = call("views.go", json!({"name": "View 1"}), &mut s);
    assert!(e.ok());
    assert_eq!(s.camera, home);
    assert_eq!(
        call("views.go", json!({"name": "nope"}), &mut s).diagnostics[0].code,
        "VIEW003"
    );
    let clash = call(
        "views.rename",
        json!({"name": "Top", "to": "View 1"}),
        &mut s,
    );
    assert_eq!(clash.diagnostics[0].code, "VIEW004");
    let empty = call("views.save", json!({"name": " "}), &mut s);
    assert_eq!(empty.diagnostics[0].code, "VIEW004");
    let long = call("views.save", json!({"name": "x".repeat(65)}), &mut s);
    assert_eq!(long.diagnostics[0].code, "VIEW004");
    assert!(call("views.rename", json!({"name": "Top", "to": "Plan"}), &mut s).ok());
    assert!(call("views.go", json!({"name": "Plan"}), &mut s).ok());
    assert_eq!(s.camera, top);

    // The commands step from the view the camera is at.
    let next = crate::commands::commands()
        .into_iter()
        .find(|c| c.id == "views.go:next")
        .unwrap();
    assert_eq!(next.input(&s).unwrap().1, json!({"name": "View 2"}));
    let prev = crate::commands::commands()
        .into_iter()
        .find(|c| c.id == "views.go:previous")
        .unwrap();
    assert_eq!(prev.input(&s).unwrap().1, json!({"name": "View 1"}));

    let rev = s.surface_revision;
    assert!(call("views.delete", json!({"name": "Plan"}), &mut s).ok());
    assert!(s.display.view("Plan").is_none());
    let e = call("journal.undo", json!({}), &mut s);
    assert_eq!(e.result["undone"], "views.delete");
    assert!(s.display.view("Plan").is_some());
    assert_eq!(s.surface_revision, rev, "a bookmark is not a redraw");
    let e = call("views.list", json!({}), &mut s);
    assert_eq!(e.result["views"].as_array().unwrap().len(), 3);
    assert_eq!(e.result["views"][1]["view"], "top");

    assert!(call("mesh.open", json!({"path": path}), &mut s).ok());
    assert!(
        s.display.views.is_empty(),
        "views are cameras of the old mesh"
    );
    assert!(next.input(&s).is_none());
}

/// Invariant: colour mode zone gives each cell zone of an OpenFOAM case
/// (cellZones) its own colour on the boundary and in the tree, and a
/// colour set on a zone wins. Breaks when the zone mode falls back to the
/// solid colour or ignores display.set_color; no other test colours by
/// zone.
#[test]
fn colour_by_cell_zone() {
    let mut s = session();
    let path = root().join("test-fixtures/openfoam/channel_ascii");
    assert!(call("mesh.open", json!({"path": path}), &mut s).ok());
    assert!(call("display.color_by", json!({"by": "zone"}), &mut s).ok());
    let m = &s.mesh.as_ref().unwrap().mesh;
    let left = murmel_engine::Item::parse("zone:left", m).unwrap();
    let right = murmel_engine::Item::parse("zone:right", m).unwrap();
    let d = &s.display;
    let (cl, cr) = (
        d.item_color(m, &s.tokens, left).unwrap(),
        d.item_color(m, &s.tokens, right).unwrap(),
    );
    assert_ne!(cl, cr);
    let colours: std::collections::BTreeSet<[u8; 3]> = (0..m.boundary.len())
        .map(|f| d.face_color(m, &s.tokens, f).0)
        .collect();
    assert!(
        colours.contains(&cl.0) && colours.contains(&cr.0),
        "{colours:?}"
    );
    assert!(
        call(
            "display.set_color",
            json!({"item": "zone:left", "color": "#102030"}),
            &mut s
        )
        .ok()
    );
    let m = &s.mesh.as_ref().unwrap().mesh;
    assert_eq!(
        s.display.item_color(m, &s.tokens, left).unwrap().hex(),
        "#102030"
    );
}

/// Invariant: a display holds several clip planes. clip.set with new adds
/// one (it becomes the current plane), plane picks one by its number,
/// and without plane the selected clip plane, else the last added,
/// changes; every enabled plane cuts, so two planes show fewer cells
/// than one; the eye (display.visibility) turns one plane off; clip.clear
/// removes one plane and renumbers the rest; undo brings it back; at
/// most MAX_CLIPS planes (CLIP002), unknown numbers are CLIP001 or
/// VIEW002. Breaks when tools act on the wrong plane, when a plane stops
/// cutting, or when removing one leaves the selection pointing at the
/// wrong plane; the surface tests check the geometry, not the tools.
#[test]
fn several_clip_planes() {
    let mut s = session();
    let path = root().join("test-fixtures/channel/foehncraft.vtu");
    assert!(call("mesh.open", json!({"path": path}), &mut s).ok());
    let all = s.mesh.as_ref().unwrap().mesh.cell_count();
    let e = call("clip.set", json!({"axis": "x"}), &mut s);
    let one = e.result["visible_cells"].as_u64().unwrap() as usize;
    assert_eq!(e.result["clip"]["id"], "clip:1");
    let e = call(
        "clip.set",
        json!({"new": true, "axis": "y", "mode": "flat"}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["clips"].as_array().unwrap().len(), 2);
    assert_eq!(e.result["clip"]["id"], "clip:2", "the new plane is current");
    let two = e.result["visible_cells"].as_u64().unwrap() as usize;
    assert!(two < one && one < all, "{two} < {one} < {all}");

    // A selected plane is the current one.
    assert!(call("select.item", json!({"item": "clip:1"}), &mut s).ok());
    let n1 = s.display.clips[0].normal;
    assert!(call("clip.set", json!({"flip": true}), &mut s).ok());
    assert_eq!(s.display.clips[0].normal, n1.map(|v| -v));
    assert_eq!(s.display.clips[1].mode, murmel_engine::ClipMode::Flat);
    let e = call("clip.set", json!({"plane": 2, "mode": "crinkle"}), &mut s);
    assert!(e.ok());
    assert_eq!(s.display.clips[1].mode, murmel_engine::ClipMode::Crinkle);
    assert_eq!(
        call("clip.set", json!({"plane": 3}), &mut s).diagnostics[0].code,
        "CLIP001"
    );
    assert_eq!(
        call("select.item", json!({"item": "clip:5"}), &mut s).diagnostics[0].code,
        "VIEW002"
    );
    let e = call("inspect.item", json!({"item": "clip:2"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert!(
        e.result["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["key"] == "offset")
    );
    assert!(call("view.fit", json!({"item": "clip:2"}), &mut s).ok());

    // The eye turns one plane off; the other still cuts.
    let e = call(
        "display.visibility",
        json!({"action": "hide", "items": ["clip:2"]}),
        &mut s,
    );
    assert!(!s.display.clips[1].enabled);
    assert!(e.result["hidden"].as_array().unwrap().is_empty());
    assert_eq!(e.result["visible_cells"].as_u64().unwrap() as usize, one);

    // Removing plane 1 renumbers plane 2; the selection forgets plane 1.
    assert!(call("clip.clear", json!({"plane": 1}), &mut s).ok());
    assert_eq!(s.display.clips.len(), 1);
    assert!(s.display.selection.is_empty());
    assert!(
        !s.display.clips[0].enabled,
        "the old plane 2 is now plane 1"
    );
    let e = call("journal.undo", json!({}), &mut s);
    assert_eq!(e.result["undone"], "clip.clear");
    assert_eq!(s.display.clips.len(), 2);

    // clip.rotate (the viewport's tilt arcs) tilts the plane named.
    let before = s.display.clips.clone();
    let e = call(
        "clip.rotate",
        json!({"plane": 2, "axis": "x", "deg": 30.0}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(s.display.clips[0], before[0]);
    assert_ne!(s.display.clips[1].normal, before[1].normal);
    let e = call(
        "clip.rotate",
        json!({"plane": 9, "axis": "x", "deg": 3.0}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "CLIP001");

    for _ in 2..crate::tools::MAX_CLIPS {
        assert!(call("clip.set", json!({"new": true}), &mut s).ok());
    }
    let e = call("clip.set", json!({"new": true}), &mut s);
    assert_eq!(e.diagnostics[0].code, "CLIP002");
    assert!(call("clip.clear", json!({"all": true}), &mut s).ok());
    assert!(s.display.clips.is_empty());
}

/// Invariant: session files keep every clip plane in order (format
/// 1.2, `clips`), and a 1.1 file's single `clip` loads as plane 1.
/// Breaks when only the current plane is saved, or when older files
/// lose their plane; the round trip test saves one plane.
#[test]
fn sessions_keep_every_clip_plane() {
    let dir = scratch("session-clips");
    let mesh = channel_case(&dir.join("case"));
    let file = dir.join("case/clips.murmel");
    let mut s = session();
    assert!(call("mesh.open", json!({"path": mesh}), &mut s).ok());
    assert!(call("clip.set", json!({"axis": "x"}), &mut s).ok());
    assert!(
        call(
            "clip.set",
            json!({"new": true, "axis": "z", "mode": "flat"}),
            &mut s
        )
        .ok()
    );
    assert!(call("select.item", json!({"item": "clip:1"}), &mut s).ok());
    assert!(call("session.save", json!({"path": file}), &mut s).ok());
    let mut t = session();
    assert!(call("session.load", json!({"path": file}), &mut t).ok());
    assert_eq!(t.display.clips, s.display.clips);
    assert_eq!(t.display.current_clip(), Some(0));

    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(v["format_version"], "1.3");
    let first = v["display"]["clips"][0].clone();
    let d = v["display"].as_object_mut().unwrap();
    d.remove("clips");
    d.insert("clip".into(), first);
    d.insert("selection".into(), json!([]));
    v["format_version"] = json!("1.1");
    let old = dir.join("case/old.murmel");
    std::fs::write(&old, v.to_string()).unwrap();
    let mut u = session();
    let e = call("session.load", json!({"path": old}), &mut u);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(u.display.clips, s.display.clips[..1]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant (measuring, 2026-11-24): measure.add keeps a distance
/// between two points (the node-to-node length, with dx, dy, dz) and an
/// angle at its middle point as tree items measure:1, measure:2; a probe
/// at typed coordinates finds the cell holding them, one at a screen
/// point the cell under it; probe.at reads the same rows without keeping
/// anything. Rename, hide, select, fit and remove work on the ids, every
/// change is one undo step, and bad input fails with its code. Breaks
/// when a measuring tool skips the journal, a point is resolved against
/// the wrong mesh entity, or the ids stop matching the tree's; the engine
/// tests check the geometry, not the tools.
#[test]
fn measuring_tools() {
    let mut s = session();
    let channel = root().join("test-fixtures/channel/foehncraft.vtu");
    assert!(call("mesh.open", json!({"path": channel}), &mut s).ok());
    let m = s.mesh.as_ref().unwrap().mesh.clone();
    let (p0, p1) = (m.points[0], m.points[1]);
    let len: f64 = (0..3).map(|k| (p1[k] - p0[k]).powi(2)).sum::<f64>().sqrt();
    let e = call(
        "measure.add",
        json!({"kind": "distance", "points": [{"node": 0}, {"node": 1}]}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["id"], "measure:1");
    assert!((e.result["value"].as_f64().unwrap() - len).abs() < 1e-12);
    assert_eq!(e.result["points"][0]["snap"], "node");
    let e = call(
        "measure.add",
        json!({"kind": "angle", "points": [
            {"point": [1.0, 0.0, 0.0]}, {"point": [0.0, 0.0, 0.0]}, {"point": [0.0, 1.0, 0.0]}
        ]}),
        &mut s,
    );
    assert_eq!(e.result["id"], "measure:2");
    assert!((e.result["value"].as_f64().unwrap() - 90.0).abs() < 1e-9);
    assert_eq!(e.result["unit"], "°");
    assert_eq!(s.journal.undo.len(), 2, "each measurement is a step");

    // A probe at a cell's centroid finds that cell.
    let c = 123usize;
    let centre = murmel_engine::inspect::cell_centroid(&m, c);
    let e = call("probe.add", json!({"at": {"point": centre}}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["id"], "probe:1");
    assert_eq!(e.result["at"]["cell"], c);
    let rows = e.result["rows"].as_array().unwrap();
    let row = |k: &str| {
        rows.iter()
            .find(|r| r["key"] == k)
            .map(|r| r["value"].clone())
    };
    assert_eq!(row("cell"), Some(json!(format!("cell:{c}"))));
    assert_eq!(row("distance_to_centroid"), Some(json!(0.0)));
    // At the centre of the view: the cell under it, as inspect.at says.
    let under = call("inspect.at", json!({"ndc": [0.0, 0.0]}), &mut s);
    let e = call("probe.at", json!({"at": {"ndc": [0.0, 0.0]}}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(
        format!("cell:{}", e.result["at"]["cell"]),
        under.result["hit"]["cell"].as_str().unwrap()
    );
    assert_eq!(s.display.probes.len(), 1, "probe.at keeps nothing");
    let outside = call(
        "probe.at",
        json!({"at": {"point": [99.0, 0.0, 0.0]}}),
        &mut s,
    );
    assert_eq!(outside.result["rows"][2]["key"], "inside");
    assert_eq!(outside.result["rows"][2]["value"], false);

    // Rename, hide, select, properties, fit.
    assert!(
        call(
            "measure.rename",
            json!({"item": "measure:1", "name": "Inlet gap"}),
            &mut s
        )
        .ok()
    );
    assert_eq!(
        s.display.measurement(1).unwrap().name.as_deref(),
        Some("Inlet gap")
    );
    let bad = call(
        "measure.rename",
        json!({"item": "measure:1", "name": " "}),
        &mut s,
    );
    assert_eq!(bad.diagnostics[0].code, "MEAS003");
    let not = call(
        "measure.rename",
        json!({"item": "patch:inlet", "name": "x"}),
        &mut s,
    );
    assert_eq!(not.diagnostics[0].code, "VIEW002");
    assert!(
        call(
            "display.visibility",
            json!({"action": "hide", "items": ["measure:2", "probe:1"]}),
            &mut s
        )
        .ok()
    );
    assert!(!s.display.measurement(2).unwrap().visible);
    assert!(!s.display.probe(1).unwrap().visible);
    assert!(s.display.hidden.is_empty(), "not hidden as mesh items");
    assert!(call("display.visibility", json!({"action": "show_all"}), &mut s).ok());
    assert!(s.display.measurement(2).unwrap().visible);
    assert!(call("select.item", json!({"item": "measure:1"}), &mut s).ok());
    let e = call("inspect.item", json!({}), &mut s);
    assert_eq!(e.result["item"], "measure:1");
    let rows = e.result["rows"].as_array().unwrap();
    assert!(
        rows.iter()
            .any(|r| r["key"] == "distance" && r["unit"] == "m")
    );
    let cam = s.camera.clone();
    assert!(call("view.fit", json!({"item": "probe:1"}), &mut s).ok());
    assert_ne!(s.camera, cam, "fit frames the probe's cell");

    // Remove, undo, redo; the ids are not reused.
    let steps = s.journal.undo.len();
    let e = call("measure.remove", json!({"items": ["measure:1"]}), &mut s);
    assert_eq!(e.result["removed"], 1);
    assert!(s.display.selection.is_empty(), "the selection forgets it");
    assert!(call("journal.undo", json!({}), &mut s).ok());
    assert!(s.display.measurement(1).is_some(), "undo brings it back");
    assert!(call("journal.redo", json!({}), &mut s).ok());
    assert!(s.display.measurement(1).is_none());
    let e = call(
        "measure.add",
        json!({"kind": "distance", "points": [{"edge": [0, 1]}, {"cell": 0}]}),
        &mut s,
    );
    assert_eq!(e.result["id"], "measure:3", "the next after the highest");
    assert_eq!(e.result["points"][0]["snap"], "edge_midpoint");
    assert_eq!(s.journal.undo.len(), steps + 2);
    let e = call("measure.remove", json!({"all": true}), &mut s);
    assert_eq!(e.result["removed"], 3);

    // Bad input; the whole mesh in view again, so a corner is empty.
    assert!(call("view.standard", json!({"view": "front"}), &mut s).ok());
    let code = |s: &mut Session, tool: &str, input: serde_json::Value| {
        call(tool, input, s).diagnostics[0].code
    };
    let two = json!({"kind": "distance", "points": [{"node": 0, "cell": 1}, {"node": 1}]});
    assert_eq!(code(&mut s, "measure.add", two), "TOOL002");
    let short = json!({"kind": "angle", "points": [{"node": 0}, {"node": 1}]});
    assert_eq!(code(&mut s, "measure.add", short), "TOOL002");
    let far = json!({"kind": "distance", "points": [{"node": 0}, {"node": 999999}]});
    assert_eq!(code(&mut s, "measure.add", far), "MEAS002");
    let empty = json!({"at": {"ndc": [0.99, 0.99]}});
    assert_eq!(code(&mut s, "probe.add", empty), "MEAS001");
    let snap = call("measure.snap", json!({"ndc": [0.99, 0.99]}), &mut s);
    assert!(snap.ok() && snap.result["anchor"].is_null(), "empty space");
    assert_eq!(code(&mut s, "measure.remove", json!({})), "TOOL002");
    let e = call("measure.list", json!({}), &mut s);
    assert_eq!(e.result["measurements"], json!([]));
    assert_eq!(e.result["probes"], json!([]));
}

/// Invariant (stage 1 of field results): a FoehnCraft result opens with
/// its fields; colour by field takes the file's first field unless one
/// is named, keeps a vector's component, refuses a component of a
/// scalar (FIELD003), an unknown field (FIELD002) and a mesh without
/// fields (FIELD001); field.style validates every value (FIELD004),
/// keeps one style per field, fixes the range shown now when no bounds
/// are given, and is an undo step. A front end calling these tools with
/// a field the file does not have would otherwise colour nothing and
/// report success.
#[test]
fn field_tools() {
    let mut s = session();
    let e = call(
        "mesh.open",
        json!({"path": root().join("test-fixtures/channel/foehncraft.vtu")}),
        &mut s,
    );
    assert!(e.ok());
    let e = call("display.color_by", json!({"by": "field"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "FIELD001");
    let path = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let e = call("mesh.open", json!({ "path": path }), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["format"], "vtkhdf");
    assert_eq!(e.result["fields"][0], "velocity");

    let e = call("field.list", json!({}), &mut s);
    assert_eq!(e.result["iteration"], 250);
    assert_eq!(e.result["fields"][0]["components"], 3);
    assert_eq!(e.result["fields"][0]["unit"], "m/s");
    assert!(e.result["fields"][0]["ranges"]["x"].is_array());
    assert_eq!(e.result["colormaps"][0]["id"], "viridis");
    assert!(e.result["shown"].is_null());

    let e = call("display.color_by", json!({"by": "field"}), &mut s);
    assert_eq!(e.result["color_by"], "field");
    assert_eq!(e.result["field"]["field"], "velocity");
    assert_eq!(e.result["field"]["title"], "|velocity|");
    assert_eq!(e.result["field"]["style"]["mode"], "smooth");
    let e = call("display.color_by", json!({"by": "field", "component": "x"}), &mut s);
    assert_eq!(e.result["field"]["title"], "velocity_x");
    let e = call("display.color_by", json!({"by": "solid"}), &mut s);
    assert!(e.result["field"].is_null());
    // Back to the field: the component chosen before stays.
    let e = call("display.color_by", json!({"by": "field"}), &mut s);
    assert_eq!(e.result["field"]["component"], "x");
    let e = call("display.color_by", json!({"by": "field", "field": "pressure", "component": "y"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "FIELD003");
    let e = call("display.color_by", json!({"by": "field", "field": "nope"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "FIELD002");
    let e = call("display.color_by", json!({"by": "patch", "field": "pressure"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "TOOL002");
    let e = call("display.color_by", json!({"by": "field", "field": "pressure"}), &mut s);
    assert_eq!(e.result["field"]["title"], "pressure");
    let whole = e.result["field"]["range"].clone();

    for bad in [
        json!({"bands": 1}),
        json!({"bands": 65}),
        json!({"line_count": 0}),
        json!({"map": "jet2"}),
        json!({"range": "fixed", "min": 2.0, "max": 1.0}),
    ] {
        let e = call("field.style", bad.clone(), &mut s);
        assert_eq!(e.diagnostics[0].code, "FIELD004", "{bad}");
    }
    let before = s.journal.undo.len();
    let e = call(
        "field.style",
        json!({"mode": "banded", "bands": 8, "lines": true, "line_count": 7, "map": "turbo", "reverse": true}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["style"]["bands"], 8);
    assert_eq!(e.result["map"], "turbo");
    assert_eq!(s.journal.undo.len(), before + 1);
    // A fixed range without bounds keeps the range shown now.
    let e = call("field.style", json!({"range": "fixed"}), &mut s);
    assert_eq!(e.result["range"], whole);
    assert_eq!(e.result["style"]["range"]["fixed"]["min"], whole[0]);
    let e = call("field.style", json!({"min": -1.0, "max": 1.0}), &mut s);
    assert_eq!(e.result["range"], json!([-1.0, 1.0]));
    // Each field keeps its own style.
    let e = call("field.style", json!({"field": "k", "mode": "flat"}), &mut s);
    assert!(e.ok());
    assert_eq!(e.result["style"]["mode"], "banded");
    let e = call("field.list", json!({}), &mut s);
    let k = e.result["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "k")
        .unwrap()
        .clone();
    assert_eq!(k["style"]["mode"], "flat");
    let e = call("field.style", json!({"field": "nope", "mode": "flat"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "FIELD002");
    let e = call("journal.undo", json!({}), &mut s);
    assert_eq!(e.result["undone"], "field.style");
    let e = call("field.style", json!({"reset": true}), &mut s);
    assert_eq!(e.result["style"]["mode"], "smooth");
    assert_eq!(e.result["range"], whole);
}

/// Invariant (session format 1.4): the field shown, its component and
/// every field's style come back from a session file, and loading draws
/// the same field paint (range, map, bands, lines) as before; a file
/// naming a field the result does not have falls back to solid colour
/// and reports it as skipped. Without the field in the file a saved
/// result view would come back in the mesh colour.
#[test]
fn field_sessions_round_trip() {
    let dir = std::env::temp_dir().join(format!("murmel-field-session-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("fields.murmel");
    let path = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let mut s = session();
    assert!(call("mesh.open", json!({ "path": path }), &mut s).ok());
    let by = json!({"by": "field", "field": "velocity", "component": "z"});
    assert!(call("display.color_by", by, &mut s).ok());
    let style = json!({"mode": "banded", "bands": 6, "lines": true, "map": "magma", "range": "symmetric"});
    assert!(call("field.style", style, &mut s).ok());
    let k = json!({"field": "k", "range": "fixed", "min": 0.0, "max": 0.5});
    assert!(call("field.style", k, &mut s).ok());
    let e = call("session.save", json!({"path": file}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let mut t = session();
    let e = call("session.load", json!({"path": file}), &mut t);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(t.display.field, s.display.field);
    assert_eq!(t.display.field_styles, s.display.field_styles);
    assert_eq!(t.field_paint(), s.field_paint());
    assert!(t.field_paint().is_some());

    let text = std::fs::read_to_string(&file)
        .unwrap()
        .replace("\"velocity\"", "\"vorticity\"");
    std::fs::write(&file, text).unwrap();
    let mut u = session();
    let e = call("session.load", json!({"path": file}), &mut u);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["skipped"], json!(["field:vorticity"]));
    assert_eq!(u.display.color_by, murmel_engine::ColorBy::Solid);
    let _ = std::fs::remove_dir_all(&dir);
}
