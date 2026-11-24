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

/// Invariant (the maintainer's words of 2026-11-24, "each configurable
/// value can be set interactively as default"): values saved by
/// settings.set_default, from an item (its values now; one at its
/// built-in value forgets the default) or given, are where new items of
/// their kind start, while items already open keep theirs; they are
/// saved in settings.toml beside the theme and load back; a field shown
/// for the first time takes the default field style, the view starts
/// from the default view; values tied to one mesh or field, items of the
/// mesh, unknown kinds and keys are refused; nothing here is an undo
/// step; reset forgets one value, a kind or all. Breaks when a tool that
/// adds items skips the defaults or a settings write drops them.
#[test]
fn defaults_start_new_items() {
    use murmel_engine::defaults::Kind;
    let dir = std::env::temp_dir().join(format!("murmel-defaults-{}", std::process::id()));
    let mut s = session();
    s.config_dir = Some(dir.clone());
    let path = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    assert!(call("mesh.open", json!({ "path": path }), &mut s).ok());
    assert!(call("slice.set", json!({"axis": "z"}), &mut s).ok());
    let e = call(
        "arrows.set",
        json!({"density": 60, "length": "equal"}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    let steps = s.journal.undo.len();
    let e = call("settings.set_default", json!({"item": "arrows"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["saved"]["density"], 60);
    assert_eq!(e.result["saved"]["length"], "equal");
    assert_eq!(e.result["saved"]["color"], "magnitude");
    assert!(
        e.result["forgot"]
            .as_array()
            .unwrap()
            .contains(&json!("scale"))
    );
    assert_eq!(s.journal.undo.len(), steps, "not an undo step");
    // New arrows start from them; the open ones keep theirs.
    assert!(call("arrows.clear", json!({}), &mut s).ok());
    let e = call("arrows.set", json!({}), &mut s);
    assert_eq!(e.result["arrows"]["density"], 60);
    assert_eq!(e.result["arrows"]["length"], "equal");
    let given = json!({"kind": "slice", "values": {"count": 3}});
    assert!(call("settings.set_default", given, &mut s).ok());
    assert_eq!(
        s.display.slices[0].count, 1,
        "an open slice keeps its count"
    );
    assert!(call("slice.set", json!({"new": true, "axis": "x"}), &mut s).ok());
    let new = s.display.slices[1];
    assert!(new.count == 3 && new.spacing > 0.0, "{new:?}");
    let given = json!({"kind": "clip", "values": {"mode": "flat"}});
    assert!(call("settings.set_default", given, &mut s).ok());
    assert!(call("clip.set", json!({"new": true, "axis": "y"}), &mut s).ok());
    assert_eq!(s.display.clips[0].mode, murmel_engine::ClipMode::Flat);
    // A field shown for the first time takes the default style; a field
    // with a style keeps it.
    let style = json!({"field": "pressure", "bands": 5, "mode": "banded"});
    assert!(call("field.style", style, &mut s).ok());
    let given = json!({"kind": "field_style", "values": {"mode": "banded", "bands": 9}});
    assert!(call("settings.set_default", given, &mut s).ok());
    let by = json!({"by": "field", "field": "k"});
    assert!(call("display.color_by", by, &mut s).ok());
    assert_eq!(s.display.style_of("k").bands, 9);
    let by = json!({"by": "field", "field": "pressure"});
    assert!(call("display.color_by", by, &mut s).ok());
    assert_eq!(s.display.style_of("pressure").bands, 5);
    // The view: taken from the session now.
    assert!(call("display.render_mode", json!({"mode": "wireframe"}), &mut s).ok());
    let e = call("settings.set_default", json!({"kind": "view"}), &mut s);
    assert_eq!(
        e.result["saved"]["render_mode"], "wireframe",
        "{:?}",
        e.result
    );
    // Saved beside the theme, and back.
    assert!(call("settings.set_theme", json!({"theme": "light"}), &mut s).ok());
    let (st, _, problems) = murmel_engine::settings::load(&dir, false);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(st.defaults, s.defaults);
    assert_eq!(st.defaults.get(Kind::Arrows, "density"), Some(&json!(60)));
    let mut t = session();
    t.defaults = st.defaults.clone();
    murmel_engine::defaults::start_view(&mut t);
    assert_eq!(t.display.render_mode, murmel_engine::RenderMode::Wireframe);
    let e = call("settings.get", json!({}), &mut s);
    assert_eq!(e.result["defaults"]["slice"]["count"], 3);
    assert!(
        e.result["defaultable"]["arrows"]
            .as_array()
            .unwrap()
            .contains(&json!("density"))
    );
    // Refused, with nothing changed.
    for (bad, code) in [
        (
            json!({"kind": "arrows", "values": {"field": "pressure"}}),
            "SET003",
        ),
        (
            json!({"kind": "arrows", "values": {"density": 500}}),
            "SET003",
        ),
        (json!({"kind": "lava", "values": {}}), "SET003"),
        (json!({"item": "patch:inlet"}), "SET003"),
        (json!({"item": "arrows", "keys": ["field"]}), "SET003"),
        (json!({"item": "arrows", "kind": "arrows"}), "TOOL002"),
        (json!({"item": "stream:4"}), "TOOL002"),
        (json!({"kind": "slice"}), "TOOL002"),
        (json!({}), "TOOL002"),
    ] {
        let e = call("settings.set_default", bad.clone(), &mut s);
        assert_eq!(e.diagnostics[0].code, code, "{bad}");
        assert_eq!(s.defaults, st.defaults, "{bad}");
    }
    let e = call("settings.reset_default", json!({"key": "count"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "TOOL002");
    let e = call(
        "settings.reset_default",
        json!({"kind": "arrows", "key": "density"}),
        &mut s,
    );
    assert_eq!(e.result["forgot"], 1);
    assert!(s.defaults.get(Kind::Arrows, "density").is_none());
    let e = call("settings.reset_default", json!({}), &mut s);
    assert!(e.ok() && s.defaults.is_empty());
    let (st, _, _) = murmel_engine::settings::load(&dir, false);
    assert!(st.defaults.is_empty());
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

/// Invariant (export.image, the maintainer's cards of 2026-11-24): the
/// picture is the viewport times the scale; the legend (top left) and the
/// triad (bottom left) are drawn only when asked; a transparent picture
/// has alpha 0 where no mesh is and 255 on the mesh; the dpi is stored in
/// the PNG; the view is unchanged afterwards; bad settings and a missing
/// mesh are refused with their codes. Breaks when an option is ignored or
/// the overlays move; the render test checks how the overlays look.
#[test]
fn export_image_options() {
    let mut s = session();
    let dir = root().join("target/render-test");
    std::fs::create_dir_all(&dir).unwrap();
    let out = |n: &str| dir.join(format!("tool-export-{n}.png"));
    let e = call("export.image", json!({"path": out("none")}), &mut s);
    assert_eq!(e.diagnostics[0].code, "VIEW001");
    let channel = root().join("test-fixtures/channel/foehncraft.vtu");
    assert!(call("mesh.open", json!({"path": channel}), &mut s).ok());
    assert!(call("display.color_by", json!({"by": "patch"}), &mut s).ok());
    s.viewport = [320.0, 200.0];
    let camera = s.camera.clone();

    let e = call(
        "export.image",
        json!({"path": out("plain"), "scale": 2, "legend": false, "axes": false, "dpi": 300}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(
        (e.result["width"].clone(), e.result["height"].clone()),
        (json!(640), json!(400))
    );
    assert_eq!(e.result["line_width"], 2.0);
    let (_, _, plain) = murmel_render::read_png(&out("plain")).unwrap();
    let file = std::fs::File::open(out("plain")).unwrap();
    let info = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .unwrap();
    let dims = info.info().pixel_dims.unwrap();
    assert_eq!((dims.xppu, dims.unit), (11811, png::Unit::Meter));

    let e = call(
        "export.image",
        json!({"path": out("overlays"), "scale": 2}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    let (w, _, over) = murmel_render::read_png(&out("overlays")).unwrap();
    let px = |img: &[u8], x: u32, y: u32| {
        let i = 4 * (y * w + x) as usize;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    };
    let differs = |x0: u32, y0: u32, x1: u32, y1: u32| {
        (y0..y1).any(|y| (x0..x1).any(|x| px(&plain, x, y) != px(&over, x, y)))
    };
    // The legend panel at (16, 16) points, the triad 48 points in from
    // the bottom left; the top right corner stays as it was.
    assert!(differs(40, 40, 80, 60), "no legend");
    assert!(differs(60, 400 - 140, 140, 400 - 60), "no triad");
    assert!(!differs(560, 0, 640, 40), "the top right changed");

    let e = call(
        "export.image",
        json!({"path": out("transparent"), "width": 400, "transparent": true, "legend": false}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["height"], 250);
    let (tw, th, t) = murmel_render::read_png(&out("transparent")).unwrap();
    assert_eq!((tw, th), (400, 250));
    assert_eq!(t[3], 0, "corner not transparent");
    let mid = 4 * ((th / 2) * tw + tw / 2) as usize;
    assert_eq!(t[mid + 3], 255, "mesh not opaque");

    assert_eq!(s.camera, camera, "the view moved");
    let bad = call(
        "export.image",
        json!({"path": out("x"), "scale": 20}),
        &mut s,
    );
    assert_eq!(bad.diagnostics[0].code, "EXPORT001");
    let bad = call(
        "export.image",
        json!({"path": out("x"), "line_width": 0.5}),
        &mut s,
    );
    assert_eq!(bad.diagnostics[0].code, "EXPORT001");
    let bad = call(
        "export.image",
        json!({"path": out("x"), "width": 9000}),
        &mut s,
    );
    assert_eq!(bad.diagnostics[0].code, "RENDER003");
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
    // A run's tables, for the chart tools that read them.
    let run = root().join("test-fixtures/spray-run");
    assert!(call("charts.open", json!({"path": run}), &mut s).ok());
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
        // This mesh has no results, so no probe history: refused, and
        // still nothing changed.
        let no_results = t.id == "probe.history"
            && s.fields()
                .is_none_or(murmel_engine::field::FieldSet::is_empty)
            && e.diagnostics.first().is_some_and(|d| d.code == "PROBE001");
        // No series is open, so no range over its steps.
        let no_series = t.id == "series.range"
            && s.series.is_none()
            && e.diagnostics.first().is_some_and(|d| d.code == "SERIES002");
        assert!(
            e.ok() || no_results || no_series,
            "{}: {:?}",
            t.id,
            e.diagnostics
        );
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

/// Invariant: a session names the mesh digest by its hash: `blake3` for a
/// mesh without evidence (or with KoflCraft's evidence 1.1), `sha256` for
/// one whose evidence 1.0 recorded SHA-256; a session saved before 1.15
/// (`sha256` of a mesh now checked with BLAKE3) still recognises its mesh,
/// and warns SESS003 only when that SHA-256 differs. Breaks when loading
/// compares digests of two hashes (every older session would warn its
/// mesh changed) or skips the check for older sessions.
#[test]
fn sessions_name_the_hash_of_their_mesh() {
    use murmel_mesh::FileHash;
    let dir = scratch("session-hash");
    let mesh = dir.join("box.vtu");
    std::fs::copy(root().join("test-fixtures/box/foehncraft.vtu"), &mesh).unwrap();
    let file = dir.join("s.murmel");
    let mut s = session();
    let open = call("mesh.open", json!({"path": mesh}), &mut s);
    assert_eq!(open.result["hash"], "blake3");
    assert_eq!(
        open.result["files"][0]["blake3"],
        FileHash::Blake3
            .of_bytes(&std::fs::read(&mesh).unwrap())
            .as_str()
    );
    assert!(call("session.save", json!({"path": file}), &mut s).ok());
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(v["mesh"]["blake3"], open.result["digest"]);
    assert!(v["mesh"].get("sha256").is_none());

    // As saved before 1.15.
    let sha = FileHash::Sha256.of_bytes(&std::fs::read(&mesh).unwrap());
    v["format_version"] = "1.14".into();
    v["mesh"].as_object_mut().unwrap().remove("blake3");
    v["mesh"]["sha256"] = sha.into();
    std::fs::write(&file, v.to_string()).unwrap();
    for fresh in [true, false] {
        let mut t = if fresh { session() } else { s.clone() };
        let e = call("session.load", json!({"path": file}), &mut t);
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(e.result["warnings"], json!([]), "fresh: {fresh}");
    }
    v["mesh"]["sha256"] = FileHash::Sha256.of_bytes(b"other").into();
    std::fs::write(&file, v.to_string()).unwrap();
    let e = call("session.load", json!({"path": file}), &mut session());
    assert_eq!(e.result["warnings"][0]["code"], "SESS003");

    // A mesh whose evidence (1.0) recorded SHA-256 is saved with it.
    let channel = channel_case(&dir.join("channel"));
    assert!(call("mesh.open", json!({"path": channel}), &mut s).ok());
    assert!(call("session.save", json!({"path": file}), &mut s).ok());
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(
        v["mesh"]["sha256"],
        FileHash::Sha256
            .of_bytes(&std::fs::read(&channel).unwrap())
            .as_str()
    );
    assert!(v["mesh"].get("blake3").is_none());
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
    assert_eq!(
        v["format_version"],
        murmel_engine::session_file::SESSION_VERSION
    );
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
    assert_eq!(e.result["colormaps"][0]["id"], "rainbow_desaturated");
    assert!(e.result["shown"].is_null());

    let e = call("display.color_by", json!({"by": "field"}), &mut s);
    assert_eq!(e.result["color_by"], "field");
    assert_eq!(e.result["field"]["field"], "velocity");
    assert_eq!(e.result["field"]["title"], "|velocity|");
    assert_eq!(e.result["field"]["style"]["mode"], "smooth");
    let e = call(
        "display.color_by",
        json!({"by": "field", "component": "x"}),
        &mut s,
    );
    assert_eq!(e.result["field"]["title"], "velocity_x");
    let e = call("display.color_by", json!({"by": "solid"}), &mut s);
    assert!(e.result["field"].is_null());
    // Back to the field: the component chosen before stays.
    let e = call("display.color_by", json!({"by": "field"}), &mut s);
    assert_eq!(e.result["field"]["component"], "x");
    let e = call(
        "display.color_by",
        json!({"by": "field", "field": "pressure", "component": "y"}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "FIELD003");
    let e = call(
        "display.color_by",
        json!({"by": "field", "field": "nope"}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "FIELD002");
    let e = call(
        "display.color_by",
        json!({"by": "patch", "field": "pressure"}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "TOOL002");
    let e = call(
        "display.color_by",
        json!({"by": "field", "field": "pressure"}),
        &mut s,
    );
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
    let e = call(
        "field.style",
        json!({"field": "nope", "mode": "flat"}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "FIELD002");
    let e = call("journal.undo", json!({}), &mut s);
    assert_eq!(e.result["undone"], "field.style");
    let e = call("field.style", json!({"reset": true}), &mut s);
    assert_eq!(e.result["style"]["mode"], "smooth");
    assert_eq!(e.result["range"], whole);
    // Properties and probes read the fields: a cell its own values, a
    // probe the values interpolated at its point (same names, in file
    // order; the boundary-only y+ only on a boundary face).
    let e = call("inspect.item", json!({"item": "cell:3"}), &mut s);
    let names: Vec<&str> = e.result["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["name"].as_str())
        .collect();
    assert_eq!(
        names,
        ["velocity", "speed", "pressure", "k", "omega", "nu_t"]
    );
    let e = call("probe.at", json!({"at": {"cell": 3}}), &mut s);
    assert_eq!(e.result["fields"].as_array().unwrap().len(), 6);
    assert_eq!(e.result["fields"][0]["value"].as_array().unwrap().len(), 3);
    let m = s.mesh.as_ref().unwrap();
    let wall = m.mesh.boundary_face_id[0];
    let e = call(
        "inspect.item",
        json!({"item": format!("face:{wall}")}),
        &mut s,
    );
    assert!(
        e.result["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "y_plus" && f["location"] == "boundary"),
        "{}",
        e.result["fields"]
    );
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
    let style =
        json!({"mode": "banded", "bands": 6, "lines": true, "map": "magma", "range": "symmetric"});
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
}

/// Invariant (stage 2 of field results, the maintainer's cards of
/// 2026-11-24): slice.set adds slices along an axis (normal towards the
/// eye), moves them by offset, spreads a series through the mesh from a
/// count alone or takes count and spacing, and refuses an unknown slice
/// (SLICE001), a ninth slice (SLICE002) and a bad series (SLICE003);
/// slice.rotate tilts the current slice about its pivot; iso.set takes
/// the field shown and the middle of its range by default, refuses a
/// mesh without fields (FIELD001), an unknown field (FIELD002), a ninth
/// iso-surface (ISO002) and an unknown one (ISO001); display.mesh_look
/// switches the faint outline; each is one undo step, and the clear
/// tools remove one or all. A front end that called these with bad
/// input would otherwise draw nothing and report success.
#[test]
fn slice_and_iso_tools() {
    let mut s = session();
    let e = call(
        "mesh.open",
        json!({"path": root().join("test-fixtures/channel/foehncraft.vtu")}),
        &mut s,
    );
    assert!(e.ok());
    // Slices work without fields (in the cell colours); iso-surfaces
    // need them.
    let e = call("slice.set", json!({"axis": "x"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["slices"].as_array().unwrap().len(), 1);
    let e = call("iso.set", json!({"value": 1.0}), &mut s);
    assert_eq!(e.diagnostics[0].code, "FIELD001");

    let path = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    assert!(call("mesh.open", json!({ "path": path }), &mut s).ok());
    assert!(s.display.slices.is_empty(), "a new mesh starts without");
    let b = s.mesh.as_ref().unwrap().mesh.bounds();
    let before = s.journal.undo.len();
    let e = call("slice.set", json!({"axis": "y"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let sl = &e.result["slices"][0];
    assert_eq!(sl["id"], "slice:1");
    assert_eq!(sl["normal"][0], 0.0);
    assert_eq!(sl["normal"][1].as_f64().unwrap().abs(), 1.0);
    assert_eq!(s.journal.undo.len(), before + 1, "one undo step");
    let e = call("slice.set", json!({"offset": 0.01}), &mut s);
    assert!((e.result["slices"][0]["offset"].as_f64().unwrap() - 0.01).abs() < 1e-9);
    // A count alone spreads the series through the mesh.
    let e = call("slice.set", json!({"count": 4}), &mut s);
    let sl = &e.result["slices"][0];
    assert_eq!(sl["count"], 4);
    let height = (b.max[1] - b.min[1]) / 4.0;
    assert!((sl["spacing"].as_f64().unwrap() - height).abs() < 1e-9);
    let e = call("slice.set", json!({"count": 3, "spacing": 0.002}), &mut s);
    assert_eq!(e.result["slices"][0]["spacing"], 0.002);
    for (bad, code) in [
        (json!({"slice": 5}), "SLICE001"),
        (json!({"count": 0}), "SLICE003"),
        (json!({"count": 65}), "SLICE003"),
        (json!({"spacing": -1.0}), "SLICE003"),
        (json!({"new": true, "slice": 1}), "TOOL002"),
        (json!({"axis": "x", "normal": [1.0, 0.0, 0.0]}), "TOOL002"),
        (json!({"normal": [0.0, 0.0, 0.0]}), "TOOL002"),
    ] {
        let e = call("slice.set", bad.clone(), &mut s);
        assert_eq!(e.diagnostics[0].code, code, "{bad}");
    }
    for _ in 1..murmel_engine::extract::MAX_SLICES {
        assert!(call("slice.set", json!({"new": true, "axis": "z"}), &mut s).ok());
    }
    let e = call("slice.set", json!({"new": true, "axis": "z"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "SLICE002");
    let e = call("slice.clear", json!({"slice": 2}), &mut s);
    assert_eq!(
        e.result["slices"].as_array().unwrap().len(),
        murmel_engine::extract::MAX_SLICES - 1
    );
    // slice.rotate tilts the slice selected.
    assert!(call("select.item", json!({"item": "slice:1"}), &mut s).ok());
    let n0 = s.display.slices[0].normal;
    let e = call(
        "slice.rotate",
        json!({"axis": [0.0, 0.0, 1.0], "deg": 90.0}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    let n1 = s.display.slices[0].normal;
    assert!((n0[1] - n1[0]).abs() < 1e-9 || (n0[1] + n1[0]).abs() < 1e-9);
    assert!(n1[1].abs() < 1e-9, "{n1:?}");
    let e = call("slice.clear", json!({"all": true}), &mut s);
    assert!(e.result["slices"].as_array().unwrap().is_empty());

    // Iso-surfaces: the field shown, the middle of its range.
    let by = json!({"by": "field", "field": "pressure"});
    let range = call("display.color_by", by, &mut s).result["field"]["range"].clone();
    let e = call("iso.set", json!({}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let iso = &e.result["isos"][0];
    assert_eq!(iso["id"], "iso:1");
    assert_eq!(iso["field"], "pressure");
    assert_eq!(iso["unit"], "Pa");
    assert_eq!(iso["color"], "solid");
    let mid = 0.5 * (range[0].as_f64().unwrap() + range[1].as_f64().unwrap());
    assert!((iso["value"].as_f64().unwrap() - mid).abs() < 1e-9);
    let e = call(
        "iso.set",
        json!({"new": true, "field": "velocity", "component": "x", "value": 1.5, "color": "field"}),
        &mut s,
    );
    let iso = &e.result["isos"][1];
    assert_eq!(iso["title"], "velocity_x");
    assert_eq!(iso["color"], "field");
    for (bad, code) in [
        (json!({"iso": 9}), "ISO001"),
        (json!({"field": "nope"}), "FIELD002"),
        (json!({"field": "pressure", "component": "y"}), "FIELD003"),
    ] {
        let e = call("iso.set", bad.clone(), &mut s);
        assert_eq!(e.diagnostics[0].code, code, "{bad}");
    }
    for _ in 2..murmel_engine::extract::MAX_ISOS {
        assert!(call("iso.set", json!({"new": true}), &mut s).ok());
    }
    let e = call("iso.set", json!({"new": true}), &mut s);
    assert_eq!(e.diagnostics[0].code, "ISO002");
    // The surface draws them, the mesh as its faint outline.
    assert!(s.display.outline_mesh());
    let e = call("display.mesh_look", json!({"look": "faces"}), &mut s);
    assert_eq!(e.result["mesh_look"], "faces");
    let e = call("journal.undo", json!({}), &mut s);
    assert_eq!(e.result["undone"], "display.mesh_look");
    assert!(s.display.outline_mesh());
    // Properties of each.
    let e = call("inspect.item", json!({"item": "iso:1"}), &mut s);
    let keys: Vec<&str> = e.result["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["key"].as_str())
        .collect();
    assert!(
        keys.contains(&"field") && keys.contains(&"value"),
        "{keys:?}"
    );
    let e = call("iso.clear", json!({"iso": 1}), &mut s);
    assert_eq!(e.result["isos"][0]["field"], "velocity");
    let e = call("iso.clear", json!({"all": true}), &mut s);
    assert!(e.result["isos"].as_array().unwrap().is_empty());
    assert!(!s.display.outline_mesh());
}

/// Invariant: arrows.set takes the field shown when it is a vector (else
/// the file's first vector), draws arrows on the slices shown and, with
/// surface, on the mesh; it refuses a mesh without fields (FIELD001), a
/// scalar (ARROW001), an unknown field (FIELD002) and a density or scale
/// out of range (ARROW002); a change is one undo step; Alt + V shows and
/// hides them; arrows.clear removes them with their colour. A front end
/// that called it with bad input would otherwise draw nothing and report
/// success.
#[test]
fn arrow_tools() {
    let mut s = session();
    let vtu = root().join("test-fixtures/channel/foehncraft.vtu");
    assert!(call("mesh.open", json!({ "path": vtu }), &mut s).ok());
    let e = call("arrows.set", json!({}), &mut s);
    assert_eq!(e.diagnostics[0].code, "FIELD001");

    let path = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    assert!(call("mesh.open", json!({ "path": path }), &mut s).ok());
    let by = json!({"by": "field", "field": "pressure"});
    assert!(call("display.color_by", by, &mut s).ok());
    // The field shown is a scalar: the file's first vector.
    let before = s.journal.undo.len();
    let e = call("arrows.set", json!({}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(s.journal.undo.len(), before + 1);
    let a = &e.result["arrows"];
    assert_eq!(a["field"], "velocity");
    assert_eq!(a["density"], 40);
    assert_eq!(a["length"], "floor");
    assert_eq!(a["color"], "magnitude");
    assert_eq!(a["id"], "arrows");
    // No slice and not on the surface: none drawn yet, and no colour
    // range of the arrows drawn.
    assert_eq!(a["drawn"], 0);
    assert!(a["range"].is_null());
    assert!(call("slice.set", json!({"axis": "z"}), &mut s).ok());
    // Coloured by their magnitude: over the field's whole range; the
    // slice's own spacing (the duct is 4 long: 40 along it).
    let e = call("arrows.set", json!({}), &mut s);
    let a = &e.result["arrows"];
    let spacing = a["slice_spacing"][0].as_f64().unwrap();
    assert!((spacing - 0.1).abs() < 1e-9, "{spacing}");
    let (lo, hi) = (
        a["range"][0].as_f64().unwrap(),
        a["range"][1].as_f64().unwrap(),
    );
    // Each component is kept within its cells' values, so a vector
    // rebuilt between cells may peak a hair above the largest cell's
    // magnitude, never by more.
    let top = a["color_range"][1].as_f64().unwrap();
    assert!(lo < hi && hi <= top * (1.0 + 1e-4), "{a}");
    let e = call(
        "arrows.set",
        json!({"length": "equal", "color": "field"}),
        &mut s,
    );
    assert_eq!(e.result["arrows"]["length"], "equal");
    assert!(e.result["arrows"]["color_range"].is_null());
    let drawn = e.result["arrows"]["drawn"].as_u64().unwrap();
    assert!(drawn > 100, "{drawn}");
    let e = call("arrows.set", json!({"surface": true}), &mut s);
    assert!(e.result["arrows"]["drawn"].as_u64().unwrap() > drawn);
    let e = call("arrows.set", json!({"in_plane": true}), &mut s);
    assert!(e.result["arrows"]["longest"].as_f64().unwrap() > 0.0);
    for (bad, code) in [
        (json!({"field": "pressure"}), "ARROW001"),
        (json!({"field": "lava"}), "FIELD002"),
        (json!({"density": 3}), "ARROW002"),
        (json!({"density": 201}), "ARROW002"),
        (json!({"scale": 0.0}), "ARROW002"),
        (json!({"scale": 101.0}), "ARROW002"),
        (json!({"length": "long"}), "TOOL002"),
        (json!({"shape": "cone"}), "TOOL002"),
    ] {
        let e = call("arrows.set", bad.clone(), &mut s);
        assert_eq!(e.diagnostics[0].code, code, "{bad}");
    }
    // Alt + V hides and shows them.
    let toggle = crate::commands::commands()
        .into_iter()
        .find(|c| c.keys.contains(&"Alt+V"))
        .unwrap();
    assert_eq!(toggle.id, "arrows.set:toggle");
    let crate::commands::Run::Dynamic(tool, input) = toggle.run else {
        panic!("Alt+V is not a tool call");
    };
    let e = call(tool, input(&s).unwrap(), &mut s);
    assert_eq!(e.result["arrows"]["enabled"], false);
    assert_eq!(s.mesh.as_ref().unwrap().surface.arrows, 0);
    let colour = json!({"item": "arrows", "color": "#ABCDEF"});
    assert!(call("display.set_color", colour, &mut s).ok());
    let e = call("arrows.clear", json!({}), &mut s);
    assert!(e.result["arrows"].is_null());
    assert!(s.display.colors.is_empty());
    // Without arrows, Alt + V adds them.
    let e = call(tool, input(&s).unwrap(), &mut s);
    assert_eq!(e.result["arrows"]["enabled"], true);
}

/// Invariant (session format 1.6): slices (with their series),
/// iso-surfaces (field, value, colour mode, own colour), the mesh's
/// look and the arrows (with their colour) come back from a session
/// file and draw the same surface; an iso-surface or arrows of a field
/// the result does not have (or arrows of a scalar) are skipped and
/// reported. Without them in the file a saved result view would come
/// back without its slices.
#[test]
fn extract_sessions_round_trip() {
    use murmel_engine::Item;
    use serde_json::Value;
    let dir = std::env::temp_dir().join(format!("murmel-extract-session-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("extracts.murmel");
    let path = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let mut s = session();
    assert!(call("mesh.open", json!({ "path": path }), &mut s).ok());
    let by = json!({"by": "field", "field": "velocity"});
    assert!(call("display.color_by", by, &mut s).ok());
    assert!(call("slice.set", json!({"axis": "x", "count": 3}), &mut s).ok());
    let iso = json!({"field": "speed", "value": 1.2});
    assert!(call("iso.set", iso, &mut s).ok());
    let iso = json!({"new": true, "field": "k", "value": 0.05, "color": "field"});
    assert!(call("iso.set", iso, &mut s).ok());
    let colour = json!({"item": "iso:1", "color": "#123456"});
    assert!(call("display.set_color", colour, &mut s).ok());
    assert!(call("display.mesh_look", json!({"look": "faces"}), &mut s).ok());
    let arrows =
        json!({"surface": true, "density": 30, "scale": 1.5, "length": "equal", "color": "field"});
    assert!(call("arrows.set", arrows, &mut s).ok());
    let colour = json!({"item": "arrows", "color": "#ABCDEF"});
    assert!(call("display.set_color", colour, &mut s).ok());
    let e = call("session.save", json!({"path": file}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let text = std::fs::read_to_string(&file).unwrap();
    let version = murmel_engine::session_file::SESSION_VERSION;
    assert!(
        text.contains(&format!("\"format_version\": \"{version}\"")),
        "{text}"
    );

    let mut t = session();
    let e = call("session.load", json!({"path": file}), &mut t);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(t.display.slices, s.display.slices);
    assert_eq!(t.display.isos, s.display.isos);
    assert_eq!(t.display.mesh_look, s.display.mesh_look);
    assert_eq!(t.display.arrows, s.display.arrows);
    assert!(t.display.arrows.as_ref().is_some_and(|a| a.surface));
    assert_eq!(t.display.colors, s.display.colors);
    let (a, b) = (
        &s.mesh.as_ref().unwrap().surface,
        &t.mesh.as_ref().unwrap().surface,
    );
    assert_eq!(a.indices.len(), b.indices.len());
    assert_eq!(a.field_range, b.field_range);

    let text = text.replace("\"k\"", "\"tke\"");
    std::fs::write(&file, text).unwrap();
    let mut u = session();
    let e = call("session.load", json!({"path": file}), &mut u);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["skipped"], json!(["field:tke"]));
    assert_eq!(u.display.isos.len(), 1);
    // Arrows of a scalar field are left out, with their colour.
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    v["display"]["arrows"]["field"] = json!("pressure");
    std::fs::write(&file, v.to_string()).unwrap();
    let mut w = session();
    let e = call("session.load", json!({"path": file}), &mut w);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert!(
        e.result["skipped"]
            .as_array()
            .unwrap()
            .contains(&json!("field:pressure"))
    );
    assert!(w.display.arrows.is_none());
    assert!(!w.display.colors.contains_key(&Item::Arrows));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant: export.movie writes the frames its motion makes (a part
/// turn, a clip sweep, a flight with holds) at the size asked, as PNG
/// frames or an animated GIF, counts them in the progress, takes a
/// Cancel, and leaves the camera and the clip planes as they were; the
/// fields of another motion, missing planes or views, and bad rates are
/// refused with their codes. Breaks when a motion makes the wrong number
/// of frames, the movie stands still, or the view is left moved; the
/// engine tests check the motions without rendering.
#[test]
fn export_movie_formats_and_motions() {
    use std::sync::Arc;

    use crate::movie::{Progress, with_progress};
    let mut s = session();
    let dir = scratch("movie");
    let e = call(
        "export.movie",
        json!({"path": dir.join("none.gif"), "motion": "orbit"}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "VIEW001");
    let channel = root().join("test-fixtures/channel/foehncraft.vtu");
    assert!(call("mesh.open", json!({"path": channel}), &mut s).ok());
    s.viewport = [320.0, 200.0];
    let camera = s.camera.clone();
    let small = |mut v: serde_json::Value| {
        v["width"] = json!(64);
        v["height"] = json!(40);
        v["fps"] = json!(10);
        v
    };

    // A part turn as PNG frames, counted in the progress.
    let p = Arc::new(Progress::default());
    let e = with_progress(p.clone(), || {
        call(
            "export.movie",
            small(
                json!({"path": dir.join("orbit"), "motion": "orbit", "degrees": 90, "seconds": 0.3}),
            ),
            &mut s,
        )
    });
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["frames"], 3);
    assert_eq!(e.result["format"], "png");
    assert_eq!((p.done(), p.total()), (3, 3));
    let (w, h, first) = murmel_render::read_png(&dir.join("orbit/frame-00001.png")).unwrap();
    let (_, _, last) = murmel_render::read_png(&dir.join("orbit/frame-00003.png")).unwrap();
    assert_eq!((w, h), (64, 40));
    assert_ne!(first, last, "the camera did not turn");
    assert!(!dir.join("orbit/frame-00004.png").exists());
    assert_eq!(s.camera, camera, "the camera stays");

    // A clip sweep as a GIF that loops; the plane goes back.
    assert!(call("clip.set", json!({"axis": "x"}), &mut s).ok());
    let clips = s.display.clips.clone();
    let gif_path = dir.join("sweep.gif");
    let e = call(
        "export.movie",
        small(json!({"path": gif_path, "motion": "clip", "seconds": 0.4, "legend": false})),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(
        (e.result["frames"].clone(), e.result["fps"].clone()),
        (json!(4), json!(10.0))
    );
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut dec = opts
        .read_info(std::fs::File::open(&gif_path).unwrap())
        .unwrap();
    assert_eq!((dec.width(), dec.height()), (64, 40));
    let mut frames = Vec::new();
    while let Some(f) = dec.read_next_frame().unwrap() {
        assert_eq!(f.delay, 10);
        frames.push(f.buffer.to_vec());
    }
    assert_eq!(frames.len(), 4);
    assert_ne!(frames[0], frames[3], "the plane did not move");
    assert_eq!(s.display.clips, clips, "the plane stays");

    // A flight through two saved views: two frames at each, two between.
    assert!(call("views.save", json!({"name": "a"}), &mut s).ok());
    assert!(call("view.standard", json!({"view": "top"}), &mut s).ok());
    assert!(call("views.save", json!({"name": "b"}), &mut s).ok());
    let e = call(
        "export.movie",
        small(json!({"path": dir.join("flight"), "motion": "views", "hold": 0.2, "travel": 0.3})),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["frames"], 2 * 2 + 2);
    assert_eq!(e.result["motion"]["views"], json!(["a", "b"]));

    // A cancelled GIF leaves no file behind.
    let p = Arc::new(Progress::default());
    p.cancel();
    let cancelled = dir.join("cancelled.gif");
    let e = with_progress(p, || {
        call(
            "export.movie",
            small(json!({"path": cancelled, "motion": "orbit"})),
            &mut s,
        )
    });
    assert_eq!(e.diagnostics[0].code, "EXPORT006");
    assert!(!cancelled.exists());

    let refused = |v: serde_json::Value, s: &mut Session| {
        let mut v = v;
        v["path"] = json!(dir.join("refused.gif"));
        call("export.movie", v, s).diagnostics[0].code
    };
    assert_eq!(
        refused(json!({"motion": "clip", "degrees": 90}), &mut s),
        "TOOL002"
    );
    assert_eq!(
        refused(json!({"motion": "views", "views": ["a", "nope"]}), &mut s),
        "EXPORT005"
    );
    assert_eq!(
        refused(json!({"motion": "views", "views": ["a"]}), &mut s),
        "EXPORT005"
    );
    assert_eq!(
        refused(json!({"motion": "orbit", "fps": 0}), &mut s),
        "EXPORT001"
    );
    assert_eq!(
        refused(
            json!({"motion": "orbit", "seconds": 1200, "fps": 60}),
            &mut s
        ),
        "EXPORT001"
    );
    assert_eq!(
        refused(
            json!({"motion": "orbit", "format": "mp4", "transparent": true}),
            &mut s
        ),
        "EXPORT001"
    );
    assert!(call("clip.clear", json!({}), &mut s).ok());
    assert_eq!(refused(json!({"motion": "clip"}), &mut s), "EXPORT005");
    assert!(!dir.join("refused.gif").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant: with ffmpeg installed an MP4 is written (an ISO media file,
/// odd sides made even); without one the call fails with EXPORT002 and
/// writes nothing. Each machine runs the half its ffmpeg allows and
/// prints which; the fake ffmpeg test in `movie.rs` checks the pipe
/// everywhere.
#[test]
fn export_movie_mp4() {
    let mut s = session();
    let dir = scratch("movie-mp4");
    let channel = root().join("test-fixtures/channel/foehncraft.vtu");
    assert!(call("mesh.open", json!({"path": channel}), &mut s).ok());
    let out = dir.join("turn.mp4");
    let e = call(
        "export.movie",
        json!({"path": out, "motion": "orbit", "seconds": 0.3, "fps": 10, "width": 65, "height": 41}),
        &mut s,
    );
    match crate::movie::find_ffmpeg(std::env::var_os("MURMEL_FFMPEG"), std::env::var_os("PATH")) {
        Ok(ffmpeg) => {
            assert!(e.ok(), "{:?}", e.diagnostics);
            assert_eq!(
                (e.result["width"].clone(), e.result["height"].clone()),
                (json!(64), json!(40))
            );
            let bytes = std::fs::read(&out).unwrap();
            assert_eq!(&bytes[4..8], b"ftyp", "not an MP4");
            eprintln!(
                "MP4 made with {} ({})",
                ffmpeg.display(),
                e.result["encoder"]
            );
        }
        Err(_) => {
            assert_eq!(e.diagnostics[0].code, "EXPORT002");
            assert!(!out.exists());
            eprintln!("no ffmpeg here: checked EXPORT002 only");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant: field.catalogue lists every quantity with its inputs and
/// where the reference values start from; field.derive adds the field
/// (no journal step), takes reference values in part and says where
/// the rest came from, removes it again, and refuses what cannot be
/// derived with its code. Breaks when a tool reads the wrong input or
/// the reference values given are not the ones used.
#[test]
fn derived_field_tools() {
    let mut s = session();
    let path = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    assert!(call("mesh.open", json!({ "path": path }), &mut s).ok());
    let e = call("field.catalogue", json!({}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let qs = e.result["quantities"].as_array().unwrap();
    assert_eq!(qs.len(), murmel_engine::derive::Quantity::ALL.len());
    // The velocity and pressure ones can be made from the channel's
    // fields; the turbulence ones need k, ω or ε and ν_t.
    for q in qs {
        let group = q["group"].as_str().unwrap();
        if matches!(group, "vortices" | "pressure" | "turbomachinery") {
            assert_eq!(q["available"], true, "{q}");
        }
    }
    assert_eq!(qs[0]["inputs"][0]["field"], "velocity");
    assert_eq!(e.result["rotation"]["from"], "built_in");
    assert_eq!(
        e.result["rotation"]["zones"][0]["axis"],
        json!([0.0, 0.0, 1.0])
    );
    assert_eq!(e.result["reference"]["from"]["density"], "built_in");
    assert_eq!(e.result["reference"]["density"], 1.225);

    let before = s.journal.undo.len();
    let e = call("field.derive", json!({"quantity": "vorticity"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(s.journal.undo.len(), before);
    assert_eq!(e.result["field"], "vorticity");
    assert_eq!(e.result["places"][0]["components"], 3);
    assert_eq!(e.result["places"][0]["unit"], "1/s");
    assert!(
        call(
            "display.color_by",
            json!({"by": "field", "field": "vorticity"}),
            &mut s
        )
        .ok()
    );

    let e = call(
        "field.derive",
        json!({"quantity": "pressure-coefficient", "reference": {"velocity": 2.0}}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["recipe"]["reference"]["velocity"], 2.0);
    assert_eq!(e.result["recipe"]["reference"]["density"], 1.225);
    assert_eq!(e.result["reference_from"]["velocity"], "given");
    assert_eq!(e.result["reference_from"]["density"], "built_in");
    // Made again: the values not given are its own.
    let e = call(
        "field.derive",
        json!({"quantity": "pressure-coefficient", "reference": {"density": 1.0}}),
        &mut s,
    );
    assert_eq!(e.result["recipe"]["reference"]["velocity"], 2.0);
    assert_eq!(e.result["reference_from"]["velocity"], "kept");
    let e = call("field.catalogue", json!({}), &mut s);
    let cp = e.result["quantities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["quantity"] == "pressure-coefficient")
        .unwrap();
    assert_eq!(cp["derived"]["reference"]["density"], 1.0);

    for (input, code) in [
        (
            json!({"quantity": "lambda2", "velocity": "pressure"}),
            "FIELD006",
        ),
        (
            json!({"quantity": "lambda2", "velocity": "nothing"}),
            "FIELD005",
        ),
        (
            json!({"quantity": "total-pressure", "reference": {"density": -1.0}}),
            "FIELD007",
        ),
        (
            json!({"quantity": "vorticity", "reference": {"density": 1.0}}),
            "TOOL002",
        ),
        (json!({"quantity": "helicity"}), "TOOL002"),
        (
            json!({"quantity": "q-criterion", "remove": true}),
            "FIELD009",
        ),
    ] {
        let e = call("field.derive", input.clone(), &mut s);
        assert_eq!(e.diagnostics[0].code, code, "{input}");
    }
    let e = call(
        "field.derive",
        json!({"quantity": "vorticity", "remove": true}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert!(s.fields().unwrap().get("vorticity").is_none());
}

/// Invariant: beside FoehnCraft's case the turbomachinery quantities
/// take its turning zones and the wall distance its walls, the
/// catalogue says so, and values given win; the turbulence quantities
/// need their fields. Breaks when the case beside the result is not
/// read or a given rotation is replaced by the case's.
#[test]
fn derived_fields_from_the_case() {
    let dir = std::env::temp_dir().join(format!("murmel-derived-case-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fields.vtkhdf");
    std::fs::copy(
        root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf"),
        &path,
    )
    .unwrap();
    let mut s = session();
    assert!(call("mesh.open", json!({ "path": path }), &mut s).ok());
    let e = call("field.catalogue", json!({}), &mut s);
    let patches: Vec<String> = e.result["walls"]["mesh_patches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_string())
        .collect();
    assert!(patches.len() >= 2, "{patches:?}");
    std::fs::write(
        dir.join("case.resolved.toml"),
        format!(
            "[[mrf]]\norigin = [0.0, 0.0, 0.0]\naxis = [1, 0, 0]\nrpm = 1200.0\n[boundary.{}]\ntype = \"wall\"\n[boundary.{}]\ntype = \"velocity-inlet\"\n",
            patches[0], patches[1]
        ),
    )
    .unwrap();
    let e = call("field.catalogue", json!({}), &mut s);
    assert_eq!(e.result["rotation"]["from"], "case");
    assert_eq!(e.result["rotation"]["zones"][0]["rpm"], 1200.0);
    assert_eq!(e.result["walls"]["patches"], json!([patches[0]]));

    let e = call("field.derive", json!({"quantity": "flow-angle"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["rotation_from"], "case");
    assert_eq!(e.result["recipe"]["rotation"]["zones"][0]["rpm"], 1200.0);
    assert_eq!(e.result["places"][0]["unit"], "°");
    let mine = json!({"zones": [{"origin": [0, 0, 0], "axis": [0, 0, 1], "rpm": 10}]});
    let e = call(
        "field.derive",
        json!({"quantity": "relative-velocity", "rotation": mine}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["rotation_from"], "given");
    assert_eq!(e.result["recipe"]["rotation"]["zones"][0]["rpm"], 10.0);

    let e = call("field.derive", json!({"quantity": "wall-distance"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["recipe"]["walls"], json!([patches[0]]));
    assert_eq!(e.result["places"][0]["unit"], "m");
    let r = e.result["places"][0]["range"].as_array().unwrap();
    assert!(r[0].as_f64().unwrap() > 0.0, "{r:?}");

    for (input, code) in [
        (
            json!({"quantity": "viscosity-ratio", "nu_t": "nothing"}),
            "FIELD005",
        ),
        (
            json!({"quantity": "flow-angle", "rotation": {"zones": [{"zone": "impeller", "origin": [0, 0, 0], "axis": [0, 0, 1], "rpm": 1}]}}),
            "FIELD016",
        ),
        (
            json!({"quantity": "wall-distance", "walls": ["nowhere"]}),
            "FIELD017",
        ),
        (
            json!({"quantity": "vorticity", "rotation": mine}),
            "TOOL002",
        ),
        (json!({"quantity": "flow-angle", "walls": []}), "TOOL002"),
    ] {
        let e = call("field.derive", input.clone(), &mut s);
        assert!(!e.ok(), "{input} was made: {}", e.result);
        assert_eq!(e.diagnostics[0].code, code, "{input}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant: field.calculate adds a formula's field (no journal step),
/// says what it reads and where it has no value, computes your kept
/// formula by its name alone, makes the fields read from it again, and
/// refuses bad formulas with the codes of diag.codes; your formulas are
/// written to settings.toml and load back, and field.catalogue lists
/// them with the calculated fields and the functions. Breaks when a
/// kept formula is lost between settings and the session.
#[test]
fn calculator_tools() {
    let dir = std::env::temp_dir().join(format!("murmel-calc-{}", std::process::id()));
    let mut s = session();
    s.config_dir = Some(dir.clone());
    let path = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    assert!(call("mesh.open", json!({ "path": path }), &mut s).ok());

    let before = s.journal.undo.len();
    let e = call(
        "field.calculate",
        json!({"name": "speed2", "formula": "mag(velocity) ^ 2", "unit": "m2/s2"}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(s.journal.undo.len(), before);
    assert_eq!(e.result["field"], "speed2");
    assert_eq!(e.result["reads"], json!(["velocity"]));
    assert_eq!(e.result["places"][0]["unit"], "m2/s2");
    assert_eq!(e.result["places"][0]["no_value"], 0);
    let e = call(
        "field.calculate",
        json!({"name": "root", "formula": "sqrt(pressure - 1e9)"}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert!(e.result["places"][0]["no_value"].as_u64().unwrap() > 0);

    // Kept, then computed by its name; settings.toml has it.
    let e = call(
        "settings.keep_formula",
        json!({"name": "half", "formula": "speed2 / 2"}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    let (st, _, problems) = murmel_engine::settings::load(&dir, false);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(st.formulas.len(), 1);
    assert_eq!(st.formulas[0].formula, "speed2 / 2");
    let e = call("field.calculate", json!({"name": "half"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["formula"]["formula"], "speed2 / 2");
    // speed2 made again makes half again.
    let e = call(
        "field.calculate",
        json!({"name": "speed2", "formula": "dot(velocity, velocity)"}),
        &mut s,
    );
    assert_eq!(e.result["made_again"], json!(["half"]));

    let e = call("field.catalogue", json!({}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["formulas"][0]["computed"], true);
    let calc = e.result["calculated"].as_array().unwrap();
    assert_eq!(calc.len(), 3);
    assert_eq!(calc[2]["kept"], true);
    assert!(
        e.result["functions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "curl")
    );

    for (input, code) in [
        (json!({"name": "x1", "formula": "velocity +"}), "FIELD011"),
        (json!({"name": "x1", "formula": "nothing * 2"}), "FIELD012"),
        (
            json!({"name": "x1", "formula": "velocity * velocity"}),
            "FIELD013",
        ),
        (json!({"name": "vorticity", "formula": "1"}), "FIELD014"),
        (json!({"name": "speed2", "formula": "half"}), "FIELD015"),
        (json!({"name": "pressure", "formula": "1"}), "FIELD008"),
        (json!({"name": "nope"}), "TOOL002"),
        (json!({"name": "speed2", "remove": true}), "FIELD010"),
        (json!({"name": "velocity", "remove": true}), "FIELD009"),
        (
            json!({"name": "half", "formula": "1", "remove": true}),
            "TOOL002",
        ),
    ] {
        let e = call("field.calculate", input.clone(), &mut s);
        assert_eq!(e.diagnostics[0].code, code, "{input}: {:?}", e.diagnostics);
    }
    let e = call(
        "settings.keep_formula",
        json!({"name": "bad", "formula": "(1"}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "FIELD011");
    for n in ["half", "speed2", "root"] {
        let e = call(
            "field.calculate",
            json!({"name": n, "remove": true}),
            &mut s,
        );
        assert!(e.ok(), "{n}: {:?}", e.diagnostics);
    }
    assert!(call("settings.forget_formula", json!({"name": "half"}), &mut s).ok());
    assert!(s.formulas.is_empty());
    let e = call("settings.forget_formula", json!({"name": "half"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "TOOL002");
    let _ = std::fs::remove_dir_all(dir);
}

/// Parcels at the centre of the channel's box and spread through it, in
/// FoehnCraft's layout, written to a fresh file.
fn parcel_file(s: &Session, name: &str) -> PathBuf {
    use murmel_mesh::{Field, Location};
    let b = s.mesh.as_ref().unwrap().mesh.bounds();
    let at = |t: f64| [0, 1, 2].map(|k| b.min[k] + t * (b.max[k] - b.min[k]));
    let points: Vec<[f64; 3]> = (1..=9).map(|i| at(i as f64 / 10.0)).collect();
    let n = points.len();
    let scalar = |name: &str, values: Vec<f64>| Field {
        name: name.into(),
        unit: None,
        location: Location::Point,
        components: 1,
        values,
    };
    let fields = vec![
        Field {
            name: "velocity".into(),
            unit: None,
            location: Location::Point,
            components: 3,
            values: (0..3 * n).map(|i| (i % 3) as f64).collect(),
        },
        scalar("diameter", (1..=n).map(|i| i as f64 * 1e-5).collect()),
        scalar("parcel_id", (0..n).map(|i| i as f64).collect()),
    ];
    let dir = std::env::temp_dir().join(format!("murmel-parcel-tools-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    murmel_mesh::parcels::write_parcels(&path, &points, &fields, 0.5, 100, "channel").unwrap();
    path
}

/// Invariant: parcels.open needs a mesh (VIEW001), reads a parcel file
/// and draws every parcel coloured by diameter with a colour bar of its
/// own; parcels.set colours by a vector component, sizes by diameter,
/// filters by diameter (and refuses an injector filter on a file without
/// injectors, listing its fields: PARCEL002), refuses a scale out of
/// range (PARCEL003); a clip plane drops the parcels it cuts away; each
/// call is one undo step; a session file brings them back with their
/// settings; parcels.clear removes them with their colour. A front end
/// that called these with bad input would otherwise draw nothing and
/// report success.
#[test]
fn parcel_tools() {
    let mut s = session();
    let e = call("parcels.open", json!({"path": "nowhere.vtkhdf"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "VIEW001");
    let mesh = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    assert!(call("mesh.open", json!({ "path": mesh }), &mut s).ok());
    let e = call("parcels.set", json!({}), &mut s);
    assert_eq!(e.diagnostics[0].code, "PARCEL001");
    let path = parcel_file(&s, "parcels_000100.vtkhdf");
    let before = s.journal.undo.len();
    let e = call("parcels.open", json!({ "path": path }), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(s.journal.undo.len(), before + 1);
    let p = &e.result["parcels"];
    assert_eq!(p["parcels"], 9);
    assert_eq!(p["drawn"], 9);
    assert_eq!(p["time"], 0.5);
    assert_eq!(p["step"], 100);
    assert_eq!(p["color"]["name"], "diameter");
    assert_eq!(p["color_range"][0], 1e-5);
    assert!((p["color_range"][1].as_f64().unwrap() - 9e-5).abs() < 1e-18);
    assert!(s.display.outline_mesh(), "the mesh is a faint outline");
    let bars = murmel_engine::Legend::own_bars(&s);
    assert!(
        bars.iter()
            .any(|(o, _)| *o == murmel_engine::legend::Owner::Parcels),
        "the parcels have a colour bar of their own"
    );
    // 20 triangles a parcel, and none of the mesh (its faint outline is
    // lines).
    assert_eq!(s.mesh.as_ref().unwrap().surface.tri_cell.len(), 9 * 20);
    let e = call("parcels.set", json!({"enabled": false}), &mut s);
    assert_eq!(e.result["parcels"]["drawn"], 0);
    assert!(
        !s.display.outline_mesh(),
        "hidden parcels show the mesh's faces"
    );
    let e = call(
        "parcels.set",
        json!({"enabled": true, "color": "velocity", "component": "y", "size": "diameter", "scale": 2.0}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    let p = &e.result["parcels"];
    assert_eq!(p["color"]["component"], "y");
    // Every y component is 1: the range widens about it.
    let r = [0, 1].map(|k| p["color_range"][k].as_f64().unwrap());
    assert!(
        (r[0] - 0.999).abs() < 1e-12 && (r[1] - 1.001).abs() < 1e-12,
        "{r:?}"
    );
    assert_eq!(p["size"], "diameter");
    let e = call("parcels.set", json!({"diameter": [2.5e-5, 6.5e-5]}), &mut s);
    assert_eq!(e.result["parcels"]["drawn"], 4);
    let e = call("parcels.set", json!({"diameter": null}), &mut s);
    assert_eq!(e.result["parcels"]["drawn"], 9);
    for (bad, code) in [
        (json!({"injector": 1}), "PARCEL002"),
        (json!({"color": "temperature"}), "PARCEL002"),
        (json!({"scale": 0.0}), "PARCEL003"),
        (json!({"scale": 101.0}), "PARCEL003"),
        (json!({"diameter": [2.0, 1.0]}), "PARCEL003"),
        (json!({"map": "plaid"}), "PARCEL003"),
        (json!({"size": "huge"}), "TOOL002"),
    ] {
        let e = call("parcels.set", bad.clone(), &mut s);
        assert_eq!(e.diagnostics[0].code, code, "{bad}");
    }
    let e = call("parcels.set", json!({"color": "temperature"}), &mut s);
    assert!(
        e.diagnostics[0]
            .message
            .contains("velocity, diameter, parcel_id"),
        "{}",
        e.diagnostics[0].message
    );
    // A clip plane through the middle leaves about half.
    assert!(call("clip.set", json!({"axis": "x"}), &mut s).ok());
    let drawn = s.mesh.as_ref().unwrap().surface.parcels;
    assert!((3..=6).contains(&drawn), "{drawn}");
    assert!(call("clip.clear", json!({}), &mut s).ok());
    // A session brings them back.
    let colour = json!({"item": "parcels", "color": "#ABCDEF"});
    assert!(call("display.set_color", colour, &mut s).ok());
    let file = path.with_extension("murmel");
    assert!(call("session.save", json!({ "path": file }), &mut s).ok());
    assert!(call("parcels.clear", json!({}), &mut s).ok());
    assert!(s.display.parcels.is_none() && s.display.colors.is_empty());
    assert_eq!(s.mesh.as_ref().unwrap().surface.parcels, 0);
    let e = call("session.load", json!({ "path": file }), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let v = s.display.parcels.as_ref().unwrap();
    assert_eq!(v.scale, 2.0);
    assert_eq!(v.color.as_ref().unwrap().name, "velocity");
    assert_eq!(s.mesh.as_ref().unwrap().surface.parcels, 9);
    assert!(s.display.colors.contains_key(&murmel_engine::Item::Parcels));
    // The palette's toggle hides them.
    let toggle = crate::commands::commands()
        .into_iter()
        .find(|c| c.id == "parcels.set:toggle")
        .unwrap();
    let (tool, input) = toggle.input(&s).unwrap();
    let e = call(tool, input, &mut s);
    assert_eq!(e.result["parcels"]["enabled"], false);
}

/// A run of two time steps of the channel (the same mesh and fields) with
/// parcels: fields_000010/20.vtkhdf, spray/parcels_000010/20.vtkhdf and,
/// with `manifest`, a series.json giving times 0.1 and 0.2.
fn series_run(s: &mut Session, name: &str, manifest: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("murmel-series-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("spray")).unwrap();
    let mesh = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    assert!(call("mesh.open", json!({ "path": mesh }), s).ok());
    for step in [10u64, 20] {
        std::fs::copy(&mesh, dir.join(format!("fields_{step:06}.vtkhdf"))).unwrap();
        let p = parcel_file(s, &format!("parcels_{step:06}.vtkhdf"));
        std::fs::copy(&p, dir.join(format!("spray/parcels_{step:06}.vtkhdf"))).unwrap();
    }
    if manifest {
        let entry = |step: u64, t: f64| {
            json!({"step": step, "time": t, "extra": true, "files": {
                "fields": format!("fields_{step:06}.vtkhdf"),
                "parcels": format!("spray/parcels_{step:06}.vtkhdf")}})
        };
        let m = json!({"format": "foehncraft.series", "format_version": 1, "case": "channel",
            "entries": [entry(10, 0.1), entry(20, 0.2)]});
        std::fs::write(dir.join("series.json"), m.to_string()).unwrap();
    }
    call("mesh.close", json!({}), s);
    dir
}

/// Invariant: series.open reads FoehnCraft's manifest (or a folder) and
/// shows the first step with its parcels; series.go to another step of
/// the same mesh keeps the slice, the colouring, the parcel settings,
/// the camera and the journal and only changes the files; steps, times
/// and offsets out of the series are refused with what it has
/// (SERIES003); series.play keeps the rate and refuses one out of range;
/// a session file brings the series back at the step shown; export.movie
/// with motion series writes one frame set per step and shows the step
/// it started from again; opening another mesh ends the series. Breaks
/// if stepping through a run reset the view (the player would be
/// useless) or a movie left the session at its last step.
#[test]
fn series_tools() {
    let mut s = session();
    let e = call("series.go", json!({"by": 1}), &mut s);
    assert_eq!(e.diagnostics[0].code, "SERIES002");
    let dir = series_run(&mut s, "tools", true);
    let e = call("series.open", json!({"path": dir.join("nope")}), &mut s);
    assert_eq!(e.diagnostics[0].code, "SERIES001");
    let e = call("series.open", json!({ "path": dir }), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let r = &e.result["series"];
    assert_eq!(
        (r["count"].clone(), r["step"].clone(), r["time"].clone()),
        (json!(2), json!(10), json!(0.1))
    );
    assert_eq!(r["source"], "manifest");
    assert_eq!(e.result["parcels"]["parcels"], 9);
    assert!(call("slice.set", json!({"axis": "z"}), &mut s).ok());
    assert!(
        call(
            "parcels.set",
            json!({"color": "velocity", "scale": 3.0}),
            &mut s
        )
        .ok()
    );
    assert!(call("view.orbit", json!({"yaw_deg": 20.0}), &mut s).ok());
    let camera = s.camera.clone();
    let steps = s.journal.undo.len();
    let e = call("series.go", json!({"step": 20}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["series"]["time"], 0.2);
    assert_eq!(
        s.mesh.as_ref().unwrap().path,
        dir.join("fields_000020.vtkhdf")
    );
    assert_eq!(s.display.slices.len(), 1, "the slice stays");
    let v = s.display.parcels.as_ref().unwrap();
    assert_eq!(
        (v.color.as_ref().unwrap().name.as_str(), v.scale),
        ("velocity", 3.0)
    );
    assert_eq!(v.path, dir.join("spray/parcels_000020.vtkhdf"));
    assert_eq!(s.camera, camera, "the camera stays");
    assert_eq!(
        s.journal.undo.len(),
        steps,
        "the journal stays, with no step"
    );
    // Undo keeps the step shown: the parcels stay those of step 20.
    assert!(call("journal.undo", json!({}), &mut s).ok());
    assert_eq!(
        s.display.parcels.as_ref().unwrap().path,
        dir.join("spray/parcels_000020.vtkhdf")
    );
    assert!(call("journal.redo", json!({}), &mut s).ok());
    let e = call("series.go", json!({"step": 15}), &mut s);
    assert_eq!(e.diagnostics[0].code, "SERIES003");
    assert!(
        e.diagnostics[0].message.contains("10, 20"),
        "{}",
        e.diagnostics[0].message
    );
    assert_eq!(
        call("series.go", json!({"index": 2}), &mut s).diagnostics[0].code,
        "SERIES003"
    );
    let e = call("series.go", json!({"by": 1, "step": 10}), &mut s);
    assert_eq!(e.diagnostics[0].code, "TOOL002");
    let e = call("series.go", json!({"by": 1}), &mut s);
    assert_eq!(
        e.result["series"]["step"], 10,
        "by wraps round while looping"
    );
    let e = call("series.go", json!({"time": 0.18}), &mut s);
    assert_eq!(e.result["series"]["step"], 20);
    let e = call("series.play", json!({"rate": 100.0}), &mut s);
    assert_eq!(e.diagnostics[0].code, "SERIES003");
    let e = call(
        "series.play",
        json!({"rate": 8.0, "loop": false, "playing": true}),
        &mut s,
    );
    assert_eq!(
        e.result["series"]["playback"],
        json!({"playing": true, "rate": 8.0, "loop": false})
    );
    assert_eq!(
        call("series.go", json!({"by": 1}), &mut s).result["series"]["step"],
        20,
        "held at the end"
    );
    // A movie of the steps: 2 steps of 3 frames, then step 20 again.
    let frames = dir.join("movie");
    let e = call(
        "export.movie",
        json!({"path": frames, "motion": "series", "hold": 0.25, "fps": 12, "width": 64, "height": 48}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["frames"], 6);
    assert_eq!(e.result["motion"]["steps"], 2);
    assert_eq!(e.result["time"], true, "series movies show the time");
    assert_eq!(s.series.as_ref().unwrap().entry().step, 20);
    let e = call(
        "export.movie",
        json!({"path": frames, "motion": "series", "to": 15}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "SERIES003");
    // A session file keeps the series and the step.
    let file = dir.join("run.murmel");
    assert!(call("session.save", json!({ "path": file }), &mut s).ok());
    let mut t = session();
    let e = call("session.load", json!({ "path": file }), &mut t);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let r = t.series.as_ref().unwrap();
    assert_eq!(
        (r.entry().step, r.playback.rate, r.playback.looping),
        (20, 8.0, false)
    );
    assert!(!r.playback.playing, "a loaded session does not play");
    // Another mesh ends the series.
    let mesh = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    assert!(call("mesh.open", json!({ "path": mesh }), &mut s).ok());
    assert!(s.series.is_none());
    // A folder without a manifest: the numbered files, times from them.
    let dir2 = series_run(&mut s, "folder", false);
    let e = call("series.open", json!({"path": dir2, "step": 20}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["series"]["source"], "folder");
    assert_eq!(e.result["series"]["time"], 0.5, "the parcel file's time");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

/// Invariant (the maintainer's cards of 2026-11-24): a probe shows the
/// value of the field colouring the mesh (`shown`), and probe.history
/// reads every probe over every step of the open series: the step shown
/// from memory and the others from their files, derived and calculated
/// fields made again of each step's values, a step without a fields
/// file noted; the history is kept, so probe.export after it reads no
/// file again, until the probes change; probes and fields narrow it,
/// and the CSV has a line per step. Breaks if a history read the wrong
/// file, dropped a made field, or re-read the run on every call.
#[test]
fn probe_history_tools() {
    let mut s = session();
    let dir = series_run(&mut s, "probe-history", false);
    // Step 30 has parcels only.
    let m = json!({"format": "foehncraft.series", "format_version": 1, "entries": [
        {"step": 10, "time": 0.1, "files": {"fields": "fields_000010.vtkhdf"}},
        {"step": 20, "time": 0.2, "files": {"fields": "fields_000020.vtkhdf"}},
        {"step": 30, "time": 0.3, "files": {"parcels": "spray/parcels_000020.vtkhdf"}},
    ]});
    std::fs::write(dir.join("series.json"), m.to_string()).unwrap();
    let e = call("probe.history", json!({}), &mut s);
    assert_eq!(e.diagnostics[0].code, "VIEW001");
    assert!(call("series.open", json!({ "path": dir }), &mut s).ok());
    let e = call("probe.history", json!({}), &mut s);
    assert_eq!(e.diagnostics[0].code, "PROBE001", "no probe yet");
    for cell in [3, 40] {
        assert!(call("probe.add", json!({"at": {"cell": cell}}), &mut s).ok());
    }
    assert!(
        call(
            "probe.add",
            json!({"at": {"point": [99.0, 0.0, 0.0]}}),
            &mut s
        )
        .ok()
    );
    // Shown: nothing while the mesh is solid, then the field's value.
    let e = call("measure.list", json!({}), &mut s);
    assert!(e.result["probes"][0]["shown"].is_null());
    assert!(
        call(
            "display.color_by",
            json!({"by": "field", "field": "velocity", "component": "x"}),
            &mut s
        )
        .ok()
    );
    let e = call("measure.list", json!({}), &mut s);
    let shown = &e.result["probes"][0]["shown"];
    assert_eq!(shown["title"], "velocity_x");
    assert_eq!(shown["unit"], "m/s");
    let vx = e.result["probes"][0]["fields"][0]["value"][0]
        .as_f64()
        .unwrap();
    assert!((shown["value"].as_f64().unwrap() - vx).abs() < 1e-12);
    assert!(e.result["probes"][2]["shown"].is_null(), "outside the mesh");

    assert!(
        call(
            "field.calculate",
            json!({"name": "speed2", "formula": "mag(velocity) ^ 2"}),
            &mut s
        )
        .ok()
    );
    let e = call("probe.history", json!({}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let r = &e.result;
    assert_eq!(r["steps"], 3);
    assert_eq!(
        r["probes"],
        json!([{"id": "probe:1", "name": "Probe 1"}, {"id": "probe:2", "name": "Probe 2"}]),
        "the probe outside is left out"
    );
    let cols: Vec<&str> = r["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(
        cols.contains(&"pressure") && cols.contains(&"speed2"),
        "{cols:?}"
    );
    assert!(!cols.contains(&"y_plus"), "boundary fields are left out");
    let k = cols.iter().position(|c| *c == "speed2").unwrap();
    // Steps 10 and 20 hold the same file: the same values, read from
    // memory and from the file; step 30 has none and a note.
    let rows = r["rows"].as_array().unwrap();
    assert_eq!(rows[0]["values"], rows[1]["values"]);
    assert_eq!(rows[0]["time"], 0.1);
    assert!(rows[1]["values"][1][k][0].as_f64().unwrap() > 0.0);
    assert!(rows[2]["values"][0][0].is_null());
    assert!(rows[2]["note"].as_str().unwrap().contains("no fields file"));

    // Kept: with the step 20 file gone, the export still has its values.
    std::fs::rename(dir.join("fields_000020.vtkhdf"), dir.join("gone.vtkhdf")).unwrap();
    let csv = dir.join("probes.csv");
    let e = call(
        "probe.export",
        json!({"path": csv, "probes": ["probe:2"], "fields": ["pressure", "velocity"]}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["steps"], 3);
    let text = std::fs::read_to_string(&csv).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4);
    assert!(
        lines[0].starts_with("step,time,Probe 2: pressure"),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].ends_with("Probe 2: velocity_z [m/s]"),
        "{}",
        lines[0]
    );
    assert_eq!(lines[2].split(',').count(), 6);
    assert!(
        !lines[2].split(',').nth(2).unwrap().is_empty(),
        "step 20 kept"
    );
    // A new probe reads the run again, and step 20's file is missing now.
    assert!(call("probe.add", json!({"at": {"cell": 5}}), &mut s).ok());
    let e = call("probe.history", json!({"fields": ["pressure"]}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert!(e.result["rows"][1]["note"].is_string());
    assert_eq!(e.result["columns"].as_array().unwrap().len(), 1);
    // Narrowing to what the history lacks is refused.
    let e = call("probe.history", json!({"fields": ["nope"]}), &mut s);
    assert_eq!(e.diagnostics[0].code, "FIELD001");
    let e = call("probe.history", json!({"probes": ["probe:3"]}), &mut s);
    assert_eq!(e.diagnostics[0].code, "PROBE001");
    let e = call(
        "probe.export",
        json!({"path": dir.join("no/such/dir.csv")}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "PROBE003");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant (charts, D268 second milestone): charts.open reads a
/// FoehnCraft run's tables, charts.show changes the panel's chart and
/// refuses what the run lacks (CHART003) without changing it, charts.data
/// and stats.at follow the series' step, charts.export writes a PNG of
/// the size asked and an SVG, and every chart tool without a run is
/// CHART002. Breaks when a tool stops reaching the engine, the step of
/// the player is not used or an export writes another size; the engine's
/// tests check the numbers.
#[test]
fn chart_tools() {
    let mut s = session();
    for (id, input) in [
        ("charts.show", json!({})),
        ("charts.data", json!({})),
        ("stats.at", json!({})),
    ] {
        let e = call(id, input, &mut s);
        assert_eq!(e.diagnostics[0].code, "CHART002", "{id}");
    }
    let e = call("charts.open", json!({"path": "/nonexistent/run"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "CHART001");
    let run = root().join("test-fixtures/spray-run");
    let e = call("charts.open", json!({"path": run}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["charts"]["run"]["injectors"], json!(["nozzle"]));
    assert_eq!(e.result["charts"]["spec"]["chart"], "mass_flow");
    let e = call(
        "charts.show",
        json!({"chart": "histogram", "datums": ["nope"]}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "CHART003");
    assert!(e.diagnostics[0].message.contains("outlet"));
    assert_eq!(s.charts.as_ref().unwrap().spec.chart.key(), "mass_flow");
    let e = call(
        "charts.show",
        json!({"chart": "histogram", "datums": ["outlet"], "weight": "droplets"}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["chart"]["bars"].as_array().unwrap().len(), 40);
    // The panel's chart is the default of charts.data; a step picks the
    // histogram's interval.
    let e = call(
        "charts.data",
        json!({"step": 100, "cumulative": false}),
        &mut s,
    );
    assert!(
        e.result["chart"]["title"]
            .as_str()
            .unwrap()
            .contains("step 100")
    );
    let e = call("stats.at", json!({"step": 100}), &mut s);
    assert_eq!(e.result["statistics"]["step"], 100);
    let dir = root().join("target/render-test");
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("chart.png");
    let e = call(
        "charts.export",
        json!({"path": png, "chart": "penetration", "width": 400, "height": 240, "scale": 1.5}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    let (w, h, _) = murmel_render::read_png(&png).unwrap();
    assert_eq!((w, h), (600, 360));
    let svg = dir.join("chart.svg");
    let e = call(
        "charts.export",
        json!({"path": svg, "theme": "light"}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert!(std::fs::read_to_string(&svg).unwrap().starts_with("<svg"));
    let e = call(
        "charts.export",
        json!({"path": dir.join("chart.jpg")}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "CHART003");
    let e = call(
        "charts.export",
        json!({"path": png, "chart": "columns", "columns": ["spray:in_flow_kg", "spray:d32_m"]}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "CHART003");
}

/// Invariant (the maintainer's card of 2026-11-24: a mesh opens whole up
/// to a memory budget, half the computer's memory unless set; a blocked
/// VTKHDF file that needs more opens as an overview): settings.set_memory
/// sets the budget, or gives it back to half the memory, saves it in
/// settings.toml and refuses budgets out of range or half given, leaving
/// the budget as it was; settings.get shows it; mesh.open opens a blocked
/// file beyond the budget as an overview (its cells and blocks in the
/// summary), whole with mode "whole", and refuses an overview of a file
/// without a block index. Breaks when the setting is not saved or loaded,
/// a bad budget is taken, or the open ignores the mode; the session tests
/// open overviews but never through the tool.
#[test]
fn the_memory_budget_decides_how_a_mesh_opens() {
    let dir = scratch("memory");
    let mut s = session();
    s.config_dir = Some(dir.clone());
    let e = call("settings.get", json!({}), &mut s);
    assert_eq!(e.result["memory"]["automatic"], true);
    assert!(e.result["memory"]["budget_gib"].as_f64().unwrap() > 0.0);

    let e = call("settings.set_memory", json!({"budget_gib": 2.0}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["memory"]["budget_gib"], 2.0);
    assert_eq!(s.memory_budget, Some(2.0));
    let (st, _, problems) = murmel_engine::settings::load(&dir, false);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(st.memory.budget_gib, Some(2.0));
    for bad in [
        json!({"budget_gib": 0.1}),
        json!({"budget_gib": 5000.0}),
        json!({"budget_gib": 2.0, "automatic": true}),
        json!({"automatic": false}),
        json!({}),
        json!({"budget": 2.0}),
    ] {
        let e = call("settings.set_memory", bad.clone(), &mut s);
        assert_eq!(e.diagnostics[0].code, "TOOL002", "{bad}");
        assert_eq!(s.memory_budget, Some(2.0), "{bad}");
    }
    let e = call("settings.set_memory", json!({"automatic": true}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(s.memory_budget, None);
    let (st, _, _) = murmel_engine::settings::load(&dir, false);
    assert!(st.memory.is_default());

    let plain = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let (mesh, results) = murmel_mesh::read_results(&plain).unwrap();
    let blocked = dir.join("blocked.vtkhdf");
    murmel_mesh::blocks::write_blocked(&blocked, &mesh, &results, 100).unwrap();
    // A budget far below what the file needs (the tool takes 0.25 GiB
    // and up; the file needs a megabyte).
    s.memory_budget = Some(1e-6);
    let e = call("mesh.open", json!({"path": blocked}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let o = &e.result["overview"];
    assert_eq!(o["cells"], mesh.cell_count());
    assert_eq!(o["blocks"], mesh.cell_count().div_ceil(100));
    assert!(!o["fields_on_boundary"].as_array().unwrap().is_empty());
    let e = call(
        "mesh.open",
        json!({"path": blocked, "mode": "whole"}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert!(e.result["overview"].is_null());
    assert_eq!(e.result["summary"]["cells"], mesh.cell_count());
    let e = call("mesh.open", json!({"path": plain}), &mut s);
    assert!(e.result["overview"].is_null());
    let e = call(
        "mesh.open",
        json!({"path": plain, "mode": "overview"}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "BLOCK004");
    let e = call("mesh.open", json!({"path": plain, "mode": "half"}), &mut s);
    assert_eq!(e.diagnostics[0].code, "TOOL002");
    let _ = std::fs::remove_dir_all(dir);
}

/// Invariant (the maintainer's card 7 of 2026-11-24: the overview is the
/// surface, the block ranges and a preview grid): through the tools, a
/// slice and an iso-surface on a mesh open as an overview draw on its
/// preview grid, read before the tool returns, and mesh.info's overview
/// tells the grid (`preview`). Breaks when the tool returns before the
/// grid is read (nothing drawn for a script or an agent), or when the
/// result does not tell it. The engine tests do not go through the
/// tools.
#[test]
fn slices_on_an_overview_draw_through_the_tools() {
    let dir = std::env::temp_dir().join(format!("murmel-preview-tool-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let (mesh, results) = murmel_mesh::read_results(&plain).unwrap();
    let path = dir.join("blocked.vtkhdf");
    murmel_mesh::blocks::write_blocked(&path, &mesh, &results, 100).unwrap();
    let mut s = Session::new(Tokens::builtin().unwrap());
    s.config_dir = None;
    s.preview_cells = 16;
    let e = call(
        "mesh.open",
        json!({"path": path, "mode": "overview"}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    let preview = &e.result["overview"]["preview"];
    assert!(preview["grid"].is_null() && preview["reading"] == json!(false));

    let e = call("slice.set", json!({"axis": "x"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let sliced = s.mesh.as_ref().unwrap().surface.triangles();
    assert!(sliced > 0);
    let e = call("iso.set", json!({"field": "pressure"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert!(s.mesh.as_ref().unwrap().surface.triangles() > sliced);
    let e = call("mesh.info", json!({}), &mut s);
    let preview = &e.result["overview"]["preview"];
    assert_eq!(preview["grid"], json!([16, 8, 4]));
    assert_eq!(preview["cells"], json!(512));
    assert!(preview["not_read"].is_null());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant: through the tools, the boxes of an overview's blocks show
/// and hide with display.visibility on `blocks`, take the colour set on
/// it, are told by mesh.info's overview (`block_boxes`), come back from
/// a session file, are kept as the view's default (`show_blocks`), and
/// the item is refused on a mesh read whole. Breaks when `blocks` is not
/// an item the tools take, when the session file or the view default
/// loses the setting, or when a mesh without blocks takes it. The engine
/// test draws them but never through the tools, the session file or the
/// defaults.
#[test]
fn block_boxes_through_the_tools() {
    let dir = std::env::temp_dir().join(format!("murmel-boxes-tool-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain = root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let (mesh, results) = murmel_mesh::read_results(&plain).unwrap();
    let path = dir.join("blocked.vtkhdf");
    murmel_mesh::blocks::write_blocked(&path, &mesh, &results, 100).unwrap();
    let mut s = Session::new(Tokens::builtin().unwrap());
    s.config_dir = Some(dir.join("config"));
    let open = json!({"path": path, "mode": "overview"});
    let e = call("mesh.open", open.clone(), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let boxes = &e.result["overview"]["block_boxes"];
    assert_eq!(boxes["shown"], json!(false));
    let n = boxes["boxes"].as_u64().unwrap();
    assert!(n > 2, "{boxes}");
    assert_eq!(boxes["drawn"], json!(0));

    let show = json!({"action": "show", "items": ["blocks"]});
    let e = call("display.visibility", show, &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let e = call("mesh.info", json!({}), &mut s);
    let boxes = &e.result["overview"]["block_boxes"];
    assert_eq!(boxes["shown"], json!(true));
    assert_eq!(boxes["drawn"], json!(n));
    let color = json!({"item": "blocks", "color": "#7040C0"});
    let e = call("display.set_color", color, &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    let violet = murmel_engine::theme::Rgb([0x70, 0x40, 0xC0]).linear();
    let sf = &s.mesh.as_ref().unwrap().surface;
    assert!(!sf.stream_colors.is_empty() && sf.stream_colors.iter().all(|&c| c == violet));

    // a session file brings them back
    let file = dir.join("boxes.murmel");
    assert!(call("session.save", json!({"path": file}), &mut s).ok());
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(saved["display"]["show_blocks"], json!(true));
    let mut t = Session::new(Tokens::builtin().unwrap());
    t.config_dir = None;
    let e = call("session.load", json!({"path": file}), &mut t);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert!(t.display.show_blocks);
    // (within the budget the session opens it whole: no blocks; as an
    // overview, they show at once)
    assert!(call("mesh.open", open, &mut t).ok());
    assert_eq!(t.mesh.as_ref().unwrap().surface.block_boxes as u64, n);

    // the view's default keeps them
    let e = call("settings.set_default", json!({"kind": "view"}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(
        e.result["saved"]["show_blocks"],
        json!(true),
        "{}",
        e.result
    );
    let hide = json!({"action": "hide", "items": ["blocks"]});
    assert!(call("display.visibility", hide, &mut s).ok());
    assert!(!s.display.show_blocks);
    murmel_engine::defaults::start_view(&mut s);
    assert!(s.display.show_blocks);

    // a mesh read whole has no blocks
    let whole = json!({"path": path, "mode": "whole"});
    assert!(call("mesh.open", whole, &mut s).ok());
    let show = json!({"action": "show", "items": ["blocks"]});
    let e = call("display.visibility", show, &mut s);
    assert!(!e.ok());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A run of three steps of the box (`test-fixtures/box`) as VTU files with
/// a cell field `temperature` of `factor × (k + 1)` on entry `k`: the
/// factors 1, 2 and −1 give the ranges 1 to 4, 2 to 8 and −4 to −1.
fn box_steps(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("murmel-steps-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let vtu = std::fs::read_to_string(root().join("test-fixtures/box/foehncraft.vtu")).unwrap();
    // 4 cells and 16 boundary polygons; the polygons get no value.
    let mut entries = Vec::new();
    for (step, factor) in [(10u64, 1.0f64), (20, 2.0), (30, -1.0)] {
        let values: Vec<String> = (0..20)
            .map(|k| {
                if k < 4 {
                    (factor * (k + 1) as f64).to_string()
                } else {
                    "nan".into()
                }
            })
            .collect();
        let array = format!(
            "        <DataArray type=\"Float64\" Name=\"temperature\" format=\"ascii\">\n{}\n        </DataArray>\n      </CellData>",
            values.join("\n")
        );
        let file = format!("step_{step}.vtu");
        std::fs::write(
            dir.join(&file),
            vtu.replacen("      </CellData>", &array, 1),
        )
        .unwrap();
        entries.push(json!({"step": step, "time": step as f64 / 100.0, "files": {"fields": file}}));
    }
    let m = json!({"format": "foehncraft.series", "format_version": 1, "entries": entries});
    std::fs::write(dir.join("series.json"), m.to_string()).unwrap();
    dir
}

/// Invariant: while a series is open, a field first shown is coloured
/// over every step (range steps, the maintainer's card of 2026-11-24):
/// until read, its range is the step's own; series.range reads every
/// step once and the range then spans them all, at every step, without
/// reading again; range field gives the step's own back; parcels refuse
/// range steps; a session file keeps it. Breaks if colours rescaled at
/// each step of a playing series, or a step outside the shown one was
/// missed.
#[test]
fn range_over_the_steps() {
    let mut s = session();
    let e = call("series.range", json!({}), &mut s);
    assert_eq!(e.diagnostics[0].code, "VIEW001");
    let dir = box_steps("range");
    let e = call(
        "series.open",
        json!({ "path": dir.join("series.json") }),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    let e = call("series.range", json!({}), &mut s);
    assert_eq!(e.diagnostics[0].code, "SERIES004", "no field shown yet");
    let e = call(
        "display.color_by",
        json!({"by": "field", "field": "temperature"}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(s.field_style().range, murmel_engine::RangeMode::Steps);
    let (lo, hi) = s.field_range().unwrap();
    assert!(
        (lo, hi) == (1.0, 4.0),
        "the step's own until read: {lo} {hi}"
    );
    let e = call("field.style", json!({}), &mut s);
    assert_eq!(e.result["steps_range"]["complete"], false);
    let e = call("series.range", json!({}), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(e.result["range"], json!([-4.0, 8.0]));
    assert_eq!(
        (e.result["read"].clone(), e.result["total"].clone()),
        (json!(3), json!(3))
    );
    assert_eq!(s.field_range(), Some((-4.0, 8.0)));
    // Another step keeps the range; nothing is read again (the files
    // of the other steps are gone).
    assert!(call("series.go", json!({"step": 20}), &mut s).ok());
    let _ = std::fs::remove_file(dir.join("step_30.vtu"));
    assert_eq!(s.field_range(), Some((-4.0, 8.0)));
    let e = call("series.range", json!({}), &mut s);
    assert_eq!(
        e.result["range"],
        json!([-4.0, 8.0]),
        "kept, not read again"
    );
    // The step's own range again.
    assert!(call("field.style", json!({"range": "field"}), &mut s).ok());
    assert_eq!(s.field_range(), Some((2.0, 8.0)));
    assert!(call("field.style", json!({"range": "steps"}), &mut s).ok());
    assert_eq!(s.field_range(), Some((-4.0, 8.0)));
    // A session file keeps range steps.
    let file = dir.join("steps.murmel");
    assert!(call("session.save", json!({ "path": file }), &mut s).ok());
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"steps\""), "{text}");
    assert!(call("field.style", json!({"range": "field"}), &mut s).ok());
    let e = call("session.load", json!({ "path": file }), &mut s);
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(s.field_style().range, murmel_engine::RangeMode::Steps);
    let _ = std::fs::remove_dir_all(dir);
}

/// Invariant: the player's speed and loop can be your defaults
/// (settings.set_default kind player, from the values or from the open
/// series): a series opened later starts with them, paused; a rate out
/// of 0.1 to 60 is refused. Breaks if series.open went back to 4 steps
/// a second and looping whatever was saved.
#[test]
fn player_defaults() {
    let mut s = session();
    let dir = box_steps("player-defaults");
    s.config_dir = Some(dir.join("config"));
    let e = call(
        "settings.set_default",
        json!({"kind": "player", "values": {"rate": 100.0}}),
        &mut s,
    );
    assert_eq!(e.diagnostics[0].code, "SET003");
    let e = call(
        "settings.set_default",
        json!({"kind": "player", "values": {"rate": 12.0, "loop": false}}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert!(
        call(
            "series.open",
            json!({ "path": dir.join("series.json") }),
            &mut s
        )
        .ok()
    );
    let p = s.series.as_ref().unwrap().playback;
    assert_eq!((p.rate, p.looping, p.playing), (12.0, false, false));
    // Kept from the open series: the rate now.
    assert!(call("series.play", json!({"rate": 2.5}), &mut s).ok());
    let e = call(
        "settings.set_default",
        json!({"kind": "player", "keys": ["rate"]}),
        &mut s,
    );
    assert!(e.ok(), "{:?}", e.diagnostics);
    assert_eq!(
        s.defaults
            .get(murmel_engine::defaults::Kind::Player, "rate"),
        Some(&json!(2.5))
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Invariant (the maintainer's card of 2026-11-24, "Time label,
/// optional"): export.image with time draws "t = … s · step …" in a
/// panel at the bottom right while a series is open, and nothing else
/// changes; without a series time draws nothing. Times read short.
/// Breaks if the label went missing from exports, or covered the legend
/// or the triad.
#[test]
fn time_label_in_exports() {
    assert_eq!(crate::series::time_text(0.25), "0.25");
    assert_eq!(crate::series::time_text(2.0), "2");
    assert_eq!(crate::series::time_text(0.000125), "0.000125");
    assert_eq!(crate::series::time_text(2.5e-7), "2.5000e-7");
    assert_eq!(crate::series::time_text(0.0), "0");
    let mut s = session();
    let dir = box_steps("time-label");
    assert!(
        call(
            "series.open",
            json!({ "path": dir.join("series.json") }),
            &mut s
        )
        .ok()
    );
    let i18n = crate::export::i18n().unwrap();
    assert_eq!(
        crate::series::caption(&s, &i18n).as_deref(),
        Some("t = 0.1 s · step 10")
    );
    let shot = |s: &mut Session, time: bool, name: &str| {
        let path = dir.join(name);
        let e = call(
            "export.image",
            json!({"path": path, "width": 320, "height": 200, "scale": 1.0, "time": time, "legend": false, "axes": false}),
            s,
        );
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(e.result["time"], time);
        let (w, _, rgba) = murmel_render::read_png(&path).unwrap();
        (w, rgba)
    };
    let plain = shot(&mut s, false, "plain.png");
    let timed = shot(&mut s, true, "timed.png");
    let px = |(w, rgba): &(u32, Vec<u8>), x: u32, y: u32| {
        let k = 4 * (y * w + x) as usize;
        rgba[k..k + 4].to_vec()
    };
    let differs = |x0: u32, x1: u32, y0: u32, y1: u32| {
        (x0..x1).any(|x| (y0..y1).any(|y| px(&plain, x, y) != px(&timed, x, y)))
    };
    assert!(differs(200, 304, 150, 184), "the label, bottom right");
    assert!(!differs(0, 160, 0, 200), "the left half stays");
    assert!(!differs(0, 320, 0, 120), "the top stays");
    let _ = std::fs::remove_dir_all(dir);
}
