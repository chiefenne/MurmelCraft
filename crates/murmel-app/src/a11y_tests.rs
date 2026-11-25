//! Accessibility of the real window, driven headless through egui_kittest
//! (MIT or Apache-2.0, a test-only dependency): the AccessKit tree a
//! screen reader sees, and the keyboard focus.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{AccessKitNode, NodeT as _, Queryable as _};

use crate::app::MurmelApp;

fn channel() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/channel/foehncraft.vtu")
}

/// The window at 1440 × 900 with the channel open; settings are never
/// written (no config folder).
fn harness() -> Harness<'static, MurmelApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|cc| {
            let mut app = MurmelApp::new(cc, None).unwrap();
            app.session.config_dir = None;
            app.bridge_path = None;
            app.open_path(channel());
            app
        });
    h.run_steps(3);
    h
}

/// Whether a text holds a Lucide glyph (Unicode private use area), which
/// a screen reader reads as an unknown character.
fn has_glyph(s: &str) -> bool {
    s.chars().any(|c| ('\u{E000}'..='\u{F8FF}').contains(&c))
}

/// The name of the pane a node is in.
fn pane_of(node: &AccessKitNode<'_>) -> Option<String> {
    std::iter::successors(Some(*node), |n| n.parent())
        .find(|n| n.role() == Role::Pane)
        .and_then(|n| n.label())
}

/// Whether some node of the tree has the name `name`.
fn named(h: &Harness<'_, MurmelApp>, name: &str) -> bool {
    h.root()
        .children_recursive()
        .any(|n| n.accesskit_node().label().as_deref() == Some(name))
}

fn focused_pane(h: &Harness<'_, MurmelApp>) -> Option<String> {
    h.root()
        .children_recursive()
        .map(|n| n.accesskit_node())
        .find(|n| n.is_focused())
        .and_then(|n| pane_of(&n))
}

/// Every problem of the tree: a control without a name (kittest's
/// check), a name or value with an icon glyph, a pane without a name.
fn problems(h: &Harness<'_, MurmelApp>) -> Vec<String> {
    let mut out = h.unnamed_widgets();
    for n in h.root().children_recursive() {
        let a = n.accesskit_node();
        for text in [a.label(), a.value()].into_iter().flatten() {
            if has_glyph(&text) {
                out.push(format!("{:?} {text:?} holds an icon glyph", a.role()));
            }
        }
        if a.role() == Role::Pane && a.label().is_none_or(|l| l.trim().is_empty()) {
            out.push(format!("pane at {:?} has no name", a.raw_bounds()));
        }
    }
    out
}

/// Invariant: every control of the window, the settings window and the
/// command palette has an accessible name without icon glyphs, and
/// every pane has a name (AGENTS.md, AccessKit). Breaks when a widget
/// is added without a name or with its glyph in the name; the key test
/// only checks that names are translated, not that widgets have them.
#[test]
fn every_control_has_a_clean_name() {
    let mut h = harness();
    assert_eq!(problems(&h), Vec::<String>::new(), "main window");
    // The names a screen reader needs to tell rows and values apart.
    h.get_by_label("Render mode: Shaded with edges");
    assert!(named(&h, "Cells: 776"), "property row");
    h.get_by_label("fluid, 776 cells");
    h.get_by_label("Theme: Dark");
    // A patch selected, the clip on and the quality colouring: more
    // rows, the clip group and the quality legend.
    h.state_mut()
        .call("select.item", serde_json::json!({"item": "patch:inlet"}));
    h.state_mut()
        .call("clip.set", serde_json::json!({"axis": "x"}));
    h.state_mut()
        .call("display.color_by", serde_json::json!({"by": "quality"}));
    h.run_steps(3);
    assert_eq!(problems(&h), Vec::<String>::new(), "patch, clip, quality");
    h.get_by_label("Clip mode: Crinkle");
    // A second clip plane: both rows are named, the current one says so.
    h.state_mut()
        .call("clip.set", serde_json::json!({"new": true, "axis": "y"}));
    h.run_steps(2);
    assert_eq!(problems(&h), Vec::<String>::new(), "two clip planes");
    assert!(named(&h, "Clip 1: X · Crinkle"), "first plane");
    let names: Vec<String> = h
        .root()
        .children_recursive()
        .filter_map(|n| n.accesskit_node().label())
        .filter(|l| l.starts_with("Clip 2"))
        .collect();
    assert!(
        names.iter().any(|l| l.ends_with("Y · Crinkle, current")),
        "current plane: {names:?}"
    );
    assert!(named(&h, "Add clip plane (Alt + C)"), "add button");
    // A saved view (Ctrl + B names it in the person's language), and its
    // name field while it is renamed.
    h.state_mut().save_view();
    h.run_steps(2);
    assert!(named(&h, "Saved view View 1"), "saved view row");
    assert!(named(&h, "Save view (Ctrl + B)"), "save view button");
    h.state_mut().view_edit = Some(("View 1".into(), "View 1".into()));
    h.run_steps(2);
    assert_eq!(problems(&h), Vec::<String>::new(), "saved views");
    assert!(named(&h, "New name for View 1"), "rename field");
    h.state_mut().view_edit = None;
    h.state_mut().settings_open = true;
    h.state_mut().shortcuts_open = true;
    h.run_steps(2);
    assert_eq!(problems(&h), Vec::<String>::new(), "settings and shortcuts");
    h.state_mut().open_palette(String::new());
    h.run_steps(2);
    assert_eq!(problems(&h), Vec::<String>::new(), "palette");
    // An error in the status bar: its alert glyph stays out of the name.
    h.state_mut().palette = None;
    h.state_mut().error = Some(murmel_tools::Diagnostic::new("MESH001", "test"));
    h.run_steps(2);
    assert_eq!(problems(&h), Vec::<String>::new(), "status error");
}

/// Invariant: F6 visits title bar, toolbar, model tree, viewport,
/// properties and status bar in this order and wraps round, Shift + F6
/// goes back, and a hidden panel is skipped. Breaks when a region's
/// first widget is not offered or the order changes; the unit test in
/// focus.rs checks the order without egui's focus.
#[test]
fn f6_visits_every_region() {
    let mut h = harness();
    let order = [
        "Title bar",
        "Toolbar",
        "Model tree",
        "Viewport",
        "Properties",
        "Status bar",
        "Title bar",
    ];
    for want in order {
        h.key_press(egui::Key::F6);
        h.run_steps(2);
        assert_eq!(focused_pane(&h).as_deref(), Some(want));
    }
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F6);
    h.run_steps(2);
    assert_eq!(focused_pane(&h).as_deref(), Some("Status bar"));
    assert!(h.state().focus.visible, "the ring shows after F6");
    h.state_mut().show_tree = false;
    h.run_steps(2);
    for want in ["Title bar", "Toolbar", "Viewport"] {
        h.key_press(egui::Key::F6);
        h.run_steps(2);
        assert_eq!(focused_pane(&h).as_deref(), Some(want));
    }
}

/// Invariant: the arrow keys orbit when the viewport has the focus and
/// after a mouse click on a button, but belong to a widget the keyboard
/// reached (the camera stays). Breaks when the window always or never
/// gives the arrows to the view (the second brings back the pitfall of
/// a clicked button swallowing the keys).
#[test]
fn arrows_orbit_unless_a_widget_has_the_keys() {
    let mut h = harness();
    let yaw = |h: &Harness<'_, MurmelApp>| h.state().session.camera.yaw_deg;
    // The keyboard reaches the toolbar: Left belongs to it.
    for _ in 0..2 {
        h.key_press(egui::Key::F6);
        h.run_steps(2);
    }
    assert_eq!(focused_pane(&h).as_deref(), Some("Toolbar"));
    let before = yaw(&h);
    h.key_press(egui::Key::ArrowLeft);
    h.run_steps(2);
    assert_eq!(yaw(&h), before, "Left went to the toolbar");
    // On to the viewport: Left orbits, and the focus stays.
    for _ in 0..2 {
        h.key_press(egui::Key::F6);
        h.run_steps(2);
    }
    assert_eq!(focused_pane(&h).as_deref(), Some("Viewport"));
    h.key_press(egui::Key::ArrowLeft);
    h.run_steps(2);
    assert_ne!(yaw(&h), before, "Left orbits in the viewport");
    assert_eq!(focused_pane(&h).as_deref(), Some("Viewport"));
    // A mouse click on a button hides the ring; Left orbits again.
    h.get_by_label("Click picks faces").click();
    h.run_steps(2);
    assert!(!h.state().focus.visible, "a click hides the ring");
    let before = yaw(&h);
    h.key_press(egui::Key::ArrowLeft);
    h.run_steps(2);
    assert_ne!(yaw(&h), before, "Left orbits after a click");
}

/// Drag from `at` by `by` in `steps` frames with the left button.
fn drag(h: &mut Harness<'_, MurmelApp>, at: egui::Pos2, by: egui::Vec2, steps: usize) {
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.run_steps(2);
    h.input_mut().events.push(button(at, true));
    h.run();
    for i in 1..=steps {
        let p = at + by * (i as f32 / steps as f32);
        h.input_mut().events.push(egui::Event::PointerMoved(p));
        h.run();
    }
    h.input_mut().events.push(button(at + by, false));
    h.run_steps(2);
}

/// Invariant (V1.0 clip handle): the arrow and the two arcs are drawn
/// with names, dragging the arrow slides the plane without turning it,
/// dragging an arc turns the normal and leaves the pivot on the plane,
/// and each drag is one undo step. Breaks if the handles stop taking the
/// pointer from the viewport's navigation, the drag maps to the wrong
/// tool, or the undo group is not held for the drag; the tool tests run
/// the tools, not the pointer.
#[test]
fn clip_handles_drag_and_undo() {
    use crate::clip_handle::Handle;
    let mut h = harness();
    h.state_mut()
        .call("clip.set", serde_json::json!({"axis": "x"}));
    h.run_steps(3);
    let spot = |h: &Harness<'_, MurmelApp>, k: Handle| {
        *h.state().clip_spots.iter().find(|s| s.handle == k).unwrap()
    };
    let start = *h.state().session.display.clip().unwrap();
    assert_eq!(h.state().clip_spots.len(), 3);

    let s = spot(&h, Handle::Slide);
    let cam = h.state().session.camera.clone();
    drag(&mut h, s.at, s.dir * 40.0, 6);
    let slid = *h.state().session.display.clip().unwrap();
    assert_eq!(slid.normal, start.normal, "a slide does not turn");
    assert!(slid.distance(start.origin).abs() > 1e-6, "the plane moved");
    assert_eq!(h.state().session.camera, cam, "the view did not orbit");
    h.state_mut().call("journal.undo", serde_json::json!({}));
    assert_eq!(
        h.state().session.display.clip().copied(),
        Some(start),
        "one undo step"
    );

    h.run_steps(2);
    for k in [Handle::TiltU, Handle::TiltV] {
        let s = spot(&h, k);
        drag(&mut h, s.at, s.dir * 30.0, 6);
        let tilted = *h.state().session.display.clip().unwrap();
        let cos: f64 = (0..3).map(|i| tilted.normal[i] * start.normal[i]).sum();
        assert!(cos < 0.999, "{k:?} turned the plane: {cos}");
        assert_eq!(h.state().session.camera, cam, "the view did not orbit");
        h.state_mut().call("journal.undo", serde_json::json!({}));
        assert_eq!(
            h.state().session.display.clip().copied(),
            Some(start),
            "{k:?}: one undo step"
        );
        h.run_steps(2);
    }
}

/// Invariant (the maintainer's Mac test, 2026-11-24): the tree's
/// right-click Colour… opens a colour window whose controls all have
/// names, and a code typed into its hex field colours the zone through
/// display.set_color. Breaks if the colour entry goes back into the menu
/// (its picker closed with the menu, so a click did nothing) or the
/// window stops calling the tool; the tool tests do not click.
#[test]
fn tree_colour_entry_opens_a_picker() {
    let mut h = harness();
    h.get_by_label("fluid, 776 cells").click_secondary();
    h.run_steps(3);
    h.get_by_label("Colour…").click();
    h.run_steps(3);
    assert!(h.state().color_edit.is_some(), "the window is open");
    assert!(
        h.state().session.display.colors.is_empty(),
        "opening the window changes no colour"
    );
    assert!(named(&h, "Colour of fluid"), "the window has a title");
    let issues = problems(&h);
    assert!(issues.is_empty(), "{issues:#?}");
    h.get_by_role_and_label(Role::TextInput, "Hex code").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_role_and_label(Role::TextInput, "Hex code")
        .type_text("12AB34");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let m = h.state().session.mesh.as_ref().unwrap();
    let zone = murmel_engine::Item::parse("zone:fluid", &m.mesh).unwrap();
    assert_eq!(
        h.state().session.display.colors.get(&zone).map(|c| c.hex()),
        Some("#12AB34".to_string())
    );
    h.get_by_label("Done").click();
    h.run_steps(3);
    assert!(h.state().color_edit.is_none(), "Done closes it");
}

/// Invariant (the maintainer's Mac test, 2026-11-24): the rotation plane
/// shows while a tilt arc is hovered, and not for the slide arrow or
/// with the pointer away. Breaks if the plane is drawn always, never, or
/// for the arrow; the drag test only checks what the drag does.
#[test]
fn rotation_plane_shows_on_hover() {
    use crate::clip_handle::Handle;
    let mut h = harness();
    h.state_mut()
        .call("clip.set", serde_json::json!({"axis": "x"}));
    h.run_steps(3);
    let spot = |h: &Harness<'_, MurmelApp>, k: Handle| {
        h.state()
            .clip_spots
            .iter()
            .find(|s| s.handle == k)
            .unwrap()
            .at
    };
    assert_eq!(h.state().rotation_plane, None);
    for (k, want) in [
        (Handle::TiltU, Some(Handle::TiltU)),
        (Handle::Slide, None),
        (Handle::TiltV, Some(Handle::TiltV)),
    ] {
        let at = spot(&h, k);
        h.input_mut().events.push(egui::Event::PointerMoved(at));
        h.run_steps(3);
        assert_eq!(h.state().rotation_plane, want, "{k:?}");
    }
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(5.0, 450.0)));
    h.run_steps(3);
    assert_eq!(h.state().rotation_plane, None, "pointer away");
}

/// Invariant (the maintainer's card of 2026-11-25, "neutral arrow,
/// orange + blue"): Settings has a named colour well for the slide arrow
/// and for each tilt arc, and a colour set for the second arc (the tool
/// the well calls) changes that arc in the one theme named. Breaks when
/// a well loses its name or the second arc falls back to the first's
/// colour; the theme test checks the built-in colours.
#[test]
fn handle_colours_in_settings() {
    let mut h = harness();
    h.state_mut().settings_open = true;
    h.run_steps(3);
    for name in [
        "Clip slide arrow",
        "Clip tilt arc, first",
        "Clip tilt arc, second",
    ] {
        assert!(named(&h, name), "{name}");
    }
    let before = h.state().session.tokens.gizmo.clone();
    // No config folder: applied for the session, not saved (SET001).
    let _ = h.state_mut().call_envelope(
        "settings.set_colors",
        serde_json::json!({"gizmo": {"dark": {"tilt2": "#123456"}}}),
    );
    let after = h.state().session.tokens.gizmo.clone();
    assert_eq!(after.dark.tilt2, murmel_engine::Rgb([0x12, 0x34, 0x56]));
    assert_eq!(
        (after.dark.slide, after.dark.tilt),
        (before.dark.slide, before.dark.tilt)
    );
    assert_eq!(after.light, before.light);
}

/// Invariant (export, the maintainer's cards of 2026-11-24): Cmd/Ctrl +
/// Shift + E opens the export window, its controls all have names, and
/// Export… writes the picture through export.image at the chosen size
/// (the viewport at 1x here) with the legend switched off, then says so
/// in the window; the view does not move. Breaks if the key or the window
/// stops calling the tool or a control loses its name; the tool tests do
/// not click.
#[test]
fn export_window_writes_a_picture() {
    let mut h = harness();
    let out = std::env::temp_dir().join(format!("murmel-export-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&out);
    h.state_mut().export.path = Some(out.clone());
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::E,
    );
    h.run_steps(3);
    assert!(h.state().export.open, "the window is open");
    assert!(named(&h, "Export image"), "the window has a title");
    let issues = problems(&h);
    assert!(issues.is_empty(), "{issues:#?}");
    h.state_mut().export.size = crate::export_dialog::SizeChoice::Scale(1);
    h.run_steps(2);
    h.get_by_label("Colour legend").click();
    h.run_steps(2);
    assert!(!h.state().export.legend);
    let camera = h.state().session.camera.clone();
    h.get_by_label("Export…").click();
    h.run_steps(3);
    let (w, hh, _) = murmel_render::read_png(&out).unwrap();
    let [vw, vh] = h.state().session.viewport;
    assert_eq!((w, hh), (vw.round() as u32, vh.round() as u32));
    let last = h.state().export.last.clone().unwrap_or_default();
    assert!(last.starts_with("Saved murmel-export-"), "{last}");
    assert_eq!(h.state().session.camera, camera);
    let _ = std::fs::remove_file(&out);
}

/// Invariant: Ctrl + Shift + M opens the movie window with every control
/// named; "Export…" makes the movie on its own thread while the window
/// stays live, shows the result when it is done, and leaves the view as
/// it was. Breaks when the window blocks, loses the result or moves the
/// camera; the tool tests check the frames.
#[test]
fn movie_window_makes_frames() {
    let mut h = harness();
    let dir = std::env::temp_dir().join(format!("murmel-movie-window-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    h.state_mut().movie.path = Some(dir.clone());
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::M,
    );
    h.run_steps(3);
    assert!(h.state().movie.open, "the window is open");
    assert!(named(&h, "Export movie"), "the window has a title");
    let issues = problems(&h);
    assert!(issues.is_empty(), "{issues:#?}");
    {
        let m = &mut h.state_mut().movie;
        m.format = murmel_tools::movie::Format::Png;
        m.size = crate::export_dialog::SizeChoice::Custom;
        m.custom = [64, 40];
        m.seconds = 0.5;
        m.fps = 24;
    }
    h.run_steps(2);
    let camera = h.state().session.camera.clone();
    h.get_by_label("Export…").click();
    h.run_steps(2);
    let start = std::time::Instant::now();
    while h.state().movie_job.is_some() {
        assert!(start.elapsed().as_secs() < 60, "the movie never ended");
        std::thread::sleep(std::time::Duration::from_millis(20));
        h.run_steps(1);
    }
    let last = h.state().movie.last.clone().unwrap_or_default();
    assert_eq!(
        last,
        format!(
            "Saved {}, 12 frames",
            dir.file_name().unwrap().to_string_lossy()
        )
    );
    assert!(dir.join("frame-00012.png").exists());
    assert_eq!(h.state().session.camera, camera);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A click at `at` with `mods` held (no movement, so not a drag).
fn click(h: &mut Harness<'_, MurmelApp>, at: egui::Pos2, mods: egui::Modifiers) {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: mods,
    };
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(mods));
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.run_steps(2);
    h.input_mut().events.push(button(true));
    h.run();
    h.input_mut().events.push(button(false));
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(2);
}

/// Invariant (measuring, decided 2026-11-24): M starts the measure mode,
/// two clicks on the mesh add one distance from snapped points (Alt
/// takes the plain surface point), the label drawn for it selects it on
/// a click, Esc stops the mode, and Shift + P with a click places a probe
/// in the cell under the pointer. Breaks if a click in the mode picks
/// instead of measuring, Esc clears the selection instead of stopping,
/// or labels stop selecting; the tool tests never click.
#[test]
fn measure_mode_clicks() {
    use murmel_engine::{Item, SnapKind};
    let mut h = harness();
    h.key_press(egui::Key::M);
    h.run_steps(2);
    assert_eq!(
        h.state().measuring.as_ref().map(|m| m.mode),
        Some(crate::measure::Mode::Distance)
    );
    let c = h.state().viewport_rect.center();
    click(&mut h, c, egui::Modifiers::NONE);
    assert_eq!(h.state().measuring.as_ref().unwrap().points.len(), 1);
    assert!(h.state().session.display.measurements.is_empty());
    click(&mut h, c + egui::vec2(60.0, 25.0), egui::Modifiers::ALT);
    let d = &h.state().session.display;
    assert_eq!(d.measurements.len(), 1, "two clicks, one distance");
    let pts = &d.measurements[0].points;
    assert_ne!(pts[0].snap, SnapKind::Typed);
    assert_eq!(pts[1].snap, SnapKind::Surface, "Alt: no snapping");
    assert!(d.selection.is_empty(), "measuring clicks do not pick");
    assert!(h.state().measuring.as_ref().unwrap().points.is_empty());

    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().measuring.is_none(), "Esc stops measuring");
    let label = h
        .state()
        .measure_labels
        .iter()
        .find(|l| l.item == Item::Measure(1))
        .map(|l| l.rect.center())
        .expect("the distance has a label");
    click(&mut h, label, egui::Modifiers::NONE);
    assert_eq!(h.state().session.display.selection, [Item::Measure(1)]);

    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::P);
    h.run_steps(2);
    click(&mut h, c, egui::Modifiers::NONE);
    let probes = &h.state().session.display.probes;
    assert_eq!(probes.len(), 1);
    assert!(probes[0].at.cell.is_some(), "the cell under the pointer");
    let row = h.root().children_recursive().find_map(|n| {
        n.accesskit_node()
            .label()
            .filter(|l| l.starts_with("Distance 1: ") && l.ends_with(" m"))
    });
    assert!(
        row.is_some(),
        "the tree row names the distance and its value"
    );
}

/// Invariant (stage 1 of field results): a FoehnCraft result coloured
/// by a field shows the legend at the right edge with a named settings
/// button and a described colour bar, every control of its settings
/// menu has a clean name, and the menu's controls call field.style
/// (banded colouring and contour lines here) and stay open for the next
/// change. Breaks when the legend or a menu control loses its name, or
/// the menu stops calling the tool; the tool tests do not click.
#[test]
fn field_legend_and_settings() {
    let mut h = harness();
    let result = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    h.state_mut().open_path(result);
    h.state_mut().call(
        "display.color_by",
        serde_json::json!({"by": "field", "field": "pressure"}),
    );
    h.run_steps(3);
    assert_eq!(problems(&h), Vec::<String>::new(), "field legend");
    let bar: Vec<String> = h
        .root()
        .children_recursive()
        .filter_map(|n| n.accesskit_node().label())
        .filter(|l| l.starts_with("Colour bar of pressure, from "))
        .collect();
    assert_eq!(bar.len(), 1, "{bar:?}");
    h.get_by_label("Field settings: pressure").click();
    h.run_steps(3);
    assert_eq!(problems(&h), Vec::<String>::new(), "field settings menu");
    h.get_by_label("Banded").click();
    h.run_steps(3);
    h.get_by_label("Contour lines").click();
    h.run_steps(3);
    let st = h.state().session.field_style();
    assert_eq!(st.mode, murmel_engine::FieldMode::Banded);
    assert!(st.lines);
    assert!(named(&h, "Bands"), "the menu stays open with the bands");
    assert_eq!(problems(&h), Vec::<String>::new(), "banded with lines");
}

/// Invariant (stage 2 of field results, the maintainer's cards of
/// 2026-11-24): the tree's "Add slice" adds a slice that takes the clip
/// planes' handles (a drag of the arrow slides it, one undo step), the
/// mesh turns into its faint outline with a named "Show the mesh faces"
/// box that brings the faces back, a click on the colour bar adds an
/// iso-surface of the field shown at that height, and with that
/// iso-surface selected a second click moves it instead of adding one;
/// every control and row keeps a clean name. Breaks if the handles stay
/// on clip planes only, the bar stops taking clicks, or a row loses its
/// name; the tool tests do not click.
#[test]
fn slices_and_iso_surfaces_in_the_window() {
    use crate::clip_handle::{Handle, Target};
    let mut h = harness();
    let result = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    h.state_mut().open_path(result);
    h.state_mut().call(
        "display.color_by",
        serde_json::json!({"by": "field", "field": "pressure"}),
    );
    h.run_steps(3);
    h.get_by_label("Add slice (Alt + S)").click();
    h.run_steps(3);
    assert_eq!(h.state().session.display.slices.len(), 1);
    assert!(h.state().session.display.outline_mesh(), "faint outline");
    assert_eq!(problems(&h), Vec::<String>::new(), "a slice");
    assert!(named(&h, "Slice 1: X"), "slice row");

    // The slice carries the handles; the arrow slides it.
    assert!(matches!(
        h.state().handle_target(),
        Some((Target::Slice(0), _))
    ));
    assert_eq!(h.state().clip_spots.len(), 3);
    let start = h.state().session.display.slices[0];
    let s = *h
        .state()
        .clip_spots
        .iter()
        .find(|s| s.handle == Handle::Slide)
        .unwrap();
    drag(&mut h, s.at, s.dir * 40.0, 6);
    let slid = h.state().session.display.slices[0];
    assert_eq!(slid.normal, start.normal, "a slide does not turn");
    assert!(slid.plane().distance(start.origin).abs() > 1e-6, "it moved");
    h.state_mut().call("journal.undo", serde_json::json!({}));
    assert_eq!(h.state().session.display.slices[0], start, "one undo step");
    h.run_steps(2);

    // A click on the colour bar adds an iso-surface of pressure there.
    let bar = h
        .root()
        .children_recursive()
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Colour bar of pressure, from "))
        })
        .map(|n| n.rect())
        .expect("the colour bar");
    let x = bar.right() - 12.0 - 8.0;
    click(&mut h, egui::pos2(x, bar.center().y), egui::Modifiers::NONE);
    let isos = h.state().session.display.isos.clone();
    assert_eq!(isos.len(), 1, "one iso-surface");
    assert_eq!(isos[0].field.name, "pressure");
    let (lo, hi) = h.state().session.field_range().unwrap();
    let mid = 0.5 * (lo + hi);
    assert!(
        (isos[0].value - mid).abs() < 0.1 * (hi - lo),
        "{} near the middle {mid}",
        isos[0].value
    );
    h.run_steps(2);
    assert_eq!(problems(&h), Vec::<String>::new(), "an iso-surface");
    let label = h.state().iso_label(0);
    assert!(label.starts_with("Iso 1: pressure = "), "{label}");

    // Selected, a lower click moves it.
    h.get_by_label(&label).click();
    h.run_steps(3);
    assert_eq!(
        h.state().session.display.selection,
        [murmel_engine::Item::Iso(0)]
    );
    click(
        &mut h,
        egui::pos2(x, bar.bottom() - 20.0),
        egui::Modifiers::NONE,
    );
    let isos = &h.state().session.display.isos;
    assert_eq!(isos.len(), 1, "moved, not added");
    assert!(isos[0].value < mid, "{} moved down", isos[0].value);

    // The mesh's faces come back with one click.
    h.get_by_label("Show the mesh faces").click();
    h.run_steps(3);
    assert_eq!(
        h.state().session.display.mesh_look,
        murmel_engine::MeshLook::Faces
    );
    assert!(!h.state().session.display.outline_mesh());
    assert_eq!(problems(&h), Vec::<String>::new(), "mesh faces");
}

/// Whether some text of the tree reads `text` (a label's text is its
/// value).
fn shows(h: &Harness<'_, MurmelApp>, text: &str) -> bool {
    h.root()
        .children_recursive()
        .any(|n| n.accesskit_node().value().as_deref() == Some(text))
}

/// Invariant (stage 3 of field results, the maintainer's cards of
/// 2026-11-24): the tree's "Add vector arrows" adds arrows of the
/// velocity, which say where they need to go until a slice shows; the
/// row is named after the field and selecting it shows the arrows'
/// Properties; the density and length boxes are named spin buttons that
/// change them through the tool; every control keeps a clean name.
/// Breaks if the add button calls nothing, the row or a box loses its
/// name, or the boxes change the display behind the tools' back; the
/// tool tests do not click.
#[test]
fn vector_arrows_in_the_window() {
    let mut h = harness();
    let result = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    h.state_mut().open_path(result);
    h.state_mut().call(
        "display.color_by",
        serde_json::json!({"by": "field", "field": "speed"}),
    );
    h.run_steps(3);
    h.get_by_label("Add vector arrows (Alt + V)").click();
    h.run_steps(3);
    let a = h.state().session.display.arrows.clone().expect("arrows");
    assert_eq!(a.field, "velocity");
    assert!(named(&h, "Arrows: velocity"), "arrows row");
    let nowhere = "Add a slice, or put them on the surface, to see them";
    assert!(shows(&h, nowhere), "the hint");
    assert_eq!(problems(&h), Vec::<String>::new(), "arrows without a slice");

    h.get_by_label("Add slice (Alt + S)").click();
    h.run_steps(3);
    let drawn = h.state().session.mesh.as_ref().unwrap().surface.arrows;
    assert!(drawn > 0, "arrows on the slice");
    assert!(!shows(&h, nowhere), "no hint with a slice");

    // Selected, the Properties show them.
    h.get_by_label("Arrows: velocity").click();
    h.run_steps(3);
    assert_eq!(
        h.state().session.display.selection,
        [murmel_engine::Item::Arrows]
    );
    assert!(shows(&h, "Vector arrows"), "Properties heading");

    // The density box changes them through arrows.set (one undo step).
    let before = h.state().session.journal.undo.len();
    h.get_by_role_and_label(Role::SpinButton, "Across").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_role_and_label(Role::SpinButton, "Across")
        .type_text("60");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(
        h.state().session.display.arrows.as_ref().unwrap().density,
        60
    );
    assert_eq!(h.state().session.journal.undo.len(), before + 1);
    assert!(named(&h, "Scale"), "length factor box");
    // Coloured by their magnitude (the default), with their own colour
    // bar beside the mesh's speed: named, its title a named button.
    assert!(
        named(&h, "Colours of the arrows: |velocity|"),
        "the arrows' colour bar"
    );
    assert!(
        h.root().children_recursive().any(|n| n
            .accesskit_node()
            .label()
            .is_some_and(|l| l.starts_with("Colour bar of the arrows: |velocity| from "))),
        "the bar itself"
    );
    assert_eq!(problems(&h), Vec::<String>::new(), "arrows on a slice");
}

/// Invariant (stage 4 of field results, the maintainer's cards of
/// 2026-11-24): the tree's "Add streamlines" menu adds a set from a seed
/// line, named after its field and seeds, with its own named colour bar
/// while the mesh is not coloured by the same field; the seed line's end
/// dot drags through `stream.set` (one undo step, no orbit) and the
/// whole line moves by its middle; the seeds box is a named spin button
/// that changes them through the tool; every control keeps a clean name.
/// Breaks if the menu calls nothing, the handles stop taking the pointer
/// from the navigation or change the seeds behind the tools' back, or a
/// row, box or bar loses its name; the tool tests do not click.
#[test]
fn streamlines_in_the_window() {
    use crate::stream_handle::SeedHandle;
    let mut h = harness();
    let result = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    h.state_mut().open_path(result);
    h.run_steps(3);
    h.get_by_label("Add streamlines (Alt + L)").click();
    h.run_steps(3);
    h.get_by_label("From a line of seeds").click();
    h.run_steps(3);
    let st = h.state().session.display.streams.clone();
    assert_eq!(st.len(), 1, "one set");
    let murmel_engine::Seed::Line { from, to } = st[0].seed else {
        panic!("a seed line: {:?}", st[0].seed)
    };
    assert!(
        named(&h, "Streamlines 1: velocity from a line"),
        "the tree row"
    );
    assert!(
        named(&h, "Colours of the streamlines: |velocity|"),
        "the own colour bar"
    );
    assert_eq!(problems(&h), Vec::<String>::new(), "streamlines");

    // The end dot drags the start of the line, one undo step.
    let spot = |h: &Harness<'_, MurmelApp>, k: SeedHandle| {
        h.state()
            .stream_spots
            .iter()
            .find(|s| s.handle == k)
            .map(|s| s.at)
            .unwrap()
    };
    assert_eq!(h.state().stream_spots.len(), 3, "two ends and the middle");
    let cam = h.state().session.camera.clone();
    let before = h.state().session.journal.undo.len();
    let start = spot(&h, SeedHandle::From);
    drag(&mut h, start, egui::vec2(-40.0, 25.0), 6);
    let murmel_engine::Seed::Line {
        from: moved,
        to: same,
    } = h.state().session.display.streams[0].seed
    else {
        panic!("still a line")
    };
    assert_ne!(moved, from, "the start moved");
    assert_eq!(same, to, "the end stayed");
    assert_eq!(h.state().session.camera, cam, "the view did not orbit");
    assert_eq!(h.state().session.journal.undo.len(), before + 1, "one step");
    h.state_mut().call("journal.undo", serde_json::json!({}));
    h.run_steps(2);
    assert_eq!(
        h.state().session.display.streams[0].seed,
        murmel_engine::Seed::Line { from, to }
    );

    // The middle moves both ends by the same step.
    let a = spot(&h, SeedHandle::From);
    let b = spot(&h, SeedHandle::To);
    drag(&mut h, a + (b - a) * 0.3, egui::vec2(0.0, 30.0), 6);
    let murmel_engine::Seed::Line { from: f2, to: t2 } = h.state().session.display.streams[0].seed
    else {
        panic!("still a line")
    };
    let d = |p: [f64; 3], q: [f64; 3]| [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    let (da, db) = (d(f2, from), d(t2, to));
    assert!(da.iter().any(|x| x.abs() > 1e-9), "the line moved");
    for k in 0..3 {
        assert!(
            (da[k] - db[k]).abs() < 1e-9,
            "both ends alike: {da:?} {db:?}"
        );
    }
    assert_eq!(h.state().session.camera, cam, "the view did not orbit");

    // The seeds box changes them through stream.set.
    h.get_by_role_and_label(Role::SpinButton, "Seeds").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_role_and_label(Role::SpinButton, "Seeds")
        .type_text("12");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().session.display.streams[0].count, 12);

    // Coloured like the mesh, the bar is the field legend's: no second.
    h.state_mut().call(
        "display.color_by",
        serde_json::json!({"by": "field", "field": "velocity"}),
    );
    h.run_steps(3);
    assert!(
        !named(&h, "Colours of the streamlines: |velocity|"),
        "no second bar"
    );
    assert_eq!(problems(&h), Vec::<String>::new(), "streamlines");
}

/// Wait until the mesh read on its own thread has opened (or failed).
fn opened(h: &mut Harness<'_, MurmelApp>) {
    let start = std::time::Instant::now();
    while h.state().opening.is_some() {
        assert!(start.elapsed().as_secs() < 60, "the opening never ended");
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    h.run_steps(1);
}

/// Hold the next background read until the returned sender sends.
fn hold(h: &mut Harness<'_, MurmelApp>) -> std::sync::mpsc::Sender<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    h.state_mut().opening_gate = Some(rx);
    tx
}

/// Invariant: a large mesh (here any, the threshold set to 0) is read on
/// its own thread while the window keeps drawing: the status bar names
/// the opening and offers Cancel with an accessible name, the mesh then
/// opens through `mesh.open` (a session file through `session.load`),
/// Cancel and Esc drop what was read and leave the open mesh as it was,
/// and a mesh that cannot be read reports its code in the status bar.
/// The mesh opens before its files are hashed (the badge is checking) and
/// the badge turns green once the window has taken the digest; a session
/// file's mesh is hashed with the read, so the session loads with its
/// digest. Breaks when the window opens on its own thread without taking
/// the result, when Cancel opens the mesh anyway, when an error of the
/// reader is lost, or when the window never takes the digest; the other
/// window tests open small meshes at once.
#[test]
fn large_meshes_open_in_the_background() {
    let cancel = "Cancel. Stop opening this mesh (Esc)";
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|cc| {
            let mut app = MurmelApp::new(cc, None).unwrap();
            app.session.config_dir = None;
            app.bridge_path = None;
            app.background_bytes = 0;
            app
        });
    h.run_steps(2);

    let go = hold(&mut h);
    h.state_mut().open_path(channel());
    h.run_steps(2);
    assert!(h.state().opening.is_some());
    assert!(h.state().session.mesh.is_none());
    assert!(named(&h, cancel), "the status bar offers Cancel");
    let text = h.state().opening_text().unwrap_or_default();
    assert!(text.starts_with("Opening foehncraft.vtu: "), "{text}");
    let issues = problems(&h);
    assert!(issues.is_empty(), "{issues:#?}");
    go.send(()).unwrap();
    opened(&mut h);
    let path = match &h.state().session.mesh {
        Some(m) => m.path.clone(),
        None => panic!("the channel did not open: {:?}", h.state().error),
    };
    let start = std::time::Instant::now();
    while h.state().session.hashing.is_some() {
        assert!(start.elapsed().as_secs() < 60, "the digest never came");
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    let m = h.state().session.mesh.as_ref().unwrap();
    assert_eq!(m.digest.len(), 64, "the window took the digest");
    assert_eq!(m.validity, murmel_engine::Validity::Valid);
    assert!(h.state().session.preloaded.is_none());
    assert!(h.state().error.is_none());
    assert!(!named(&h, cancel));

    let other = channel().with_file_name("../box/foehncraft.vtu");
    let go = hold(&mut h);
    h.state_mut().open_path(other.clone());
    h.run_steps(1);
    h.get_by_label(cancel).click();
    h.run_steps(1);
    go.send(()).unwrap();
    opened(&mut h);
    let open = h.state().session.mesh.as_ref().map(|m| m.path.clone());
    assert_eq!(open.as_ref(), Some(&path), "Cancel kept the channel");
    assert!(h.state().error.is_none());

    let go = hold(&mut h);
    h.state_mut().open_path(other);
    h.run_steps(1);
    h.key_press(egui::Key::Escape);
    h.run_steps(1);
    go.send(()).unwrap();
    opened(&mut h);
    let open = h.state().session.mesh.as_ref().map(|m| m.path.clone());
    assert_eq!(open.as_ref(), Some(&path), "Esc kept the channel");

    // A session file's mesh is read first, then the session loads.
    let dir = std::env::temp_dir().join(format!("murmel-bg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("channel.murmel");
    assert!(
        h.state_mut()
            .call("session.save", serde_json::json!({ "path": file }))
            .is_some()
    );
    h.state_mut().call("mesh.close", serde_json::json!({}));
    let go = hold(&mut h);
    h.state_mut().open_path(file.clone());
    h.run_steps(1);
    assert!(
        h.state().opening.is_some(),
        "the session's mesh is read first"
    );
    go.send(()).unwrap();
    opened(&mut h);
    let m = h.state().session.mesh.as_ref().unwrap();
    assert_eq!(
        m.digest.len(),
        64,
        "a session's mesh is hashed with the read"
    );
    assert!(h.state().session.hashing.is_none());
    assert_eq!(h.state().session_path.as_ref(), Some(&file));
    let _ = std::fs::remove_dir_all(&dir);

    h.state_mut()
        .open_path(channel().with_file_name("missing.vtu"));
    opened(&mut h);
    assert_eq!(h.state().error.as_ref().map(|e| e.code), Some("MESH001"));
}

/// Invariant (large-mesh post-processing, stage 1, the maintainer's cards
/// of 2026-11-24): the tree's "Add a box region" adds a box region that
/// shows only the cells it takes, named in the tree with its shape and
/// its cell count; a click on the row selects it so the Properties panel
/// shows the region; its context menu cuts it flat; its eye turns it off
/// and every cell shows again; every control keeps a clean name. Breaks if the region section loses
/// its buttons, the row its name, or the eye stops reaching region.set.
#[test]
fn regions_in_the_window() {
    let mut h = harness();
    let all = {
        let s = &h.state().session;
        s.display.visible_counts(&s.mesh.as_ref().unwrap().mesh).0
    };
    h.get_by_label("Add a box region (Alt + R)").click();
    h.run_steps(3);
    assert_eq!(h.state().session.display.regions.len(), 1);
    let shown = {
        let s = &h.state().session;
        s.display.visible_counts(&s.mesh.as_ref().unwrap().mesh).0
    };
    assert!(0 < shown && shown < all, "{shown} of {all} cells show");
    let label = h.state().region_label(0);
    assert!(label.starts_with("Region 1: box · "), "{label}");
    assert!(named(&h, &label), "region row");
    assert_eq!(problems(&h), Vec::<String>::new(), "a region");

    h.get_by_label(&label).click();
    h.run_steps(3);
    assert_eq!(
        h.state().session.display.selection,
        [murmel_engine::Item::Region(0)]
    );
    assert_eq!(problems(&h), Vec::<String>::new(), "region properties");

    // The context menu cuts it flat; the row says so.
    h.get_by_label(&label).click_secondary();
    h.run_steps(2);
    h.get_by_label("Cut flat along the border").click();
    h.run_steps(3);
    assert_eq!(
        h.state().session.display.regions[0].cut,
        murmel_engine::ClipMode::Flat
    );
    let label = h.state().region_label(0);
    assert!(label.starts_with("Region 1: box, cut flat · "), "{label}");
    assert!(named(&h, &label), "flat region row");
    assert_eq!(problems(&h), Vec::<String>::new(), "a flat region");

    h.get_by_label(&format!("Hide {label}")).click();
    h.run_steps(3);
    assert!(!h.state().session.display.regions[0].enabled);
    let back = {
        let s = &h.state().session;
        s.display.visible_counts(&s.mesh.as_ref().unwrap().mesh).0
    };
    assert_eq!(back, all, "no region on, every cell shows");
}

/// Invariant: a message in the status bar goes away ten seconds after it
/// is first shown, or as soon as a journalled call works, and a new one
/// starts its own time. Breaks when a stale message stays (an agent's
/// SET001 stayed until the next error).
#[test]
fn status_message_expires() {
    let mut h = harness();
    let test = || Some(murmel_tools::Diagnostic::new("SET001", "test"));
    h.state_mut().set_error(test());
    h.run_steps(2);
    let since = h.state().error_since.expect("stamped when first shown");
    assert!(h.state().error.is_some(), "shown at first");
    // As if it had been shown eleven seconds ago.
    h.state_mut().error_since = Some(since - 11.0);
    h.run_steps(1);
    assert!(h.state().error.is_none(), "gone after ten seconds");
    h.state_mut().set_error(test());
    assert!(
        h.state().error_since.is_none(),
        "a new message starts its own time"
    );
    h.state_mut()
        .call("view.zoom", serde_json::json!({"factor": 0.9}));
    assert!(
        h.state().error.is_none(),
        "gone when a journalled call works"
    );
}

/// Invariant (the maintainer's card of 2026-11-24: right-click any value
/// or item to save it as the default; Settings lists and forgets each):
/// a Properties row's context menu keeps its value as the default of new
/// items of its kind through settings.set_default, the item row's menu
/// keeps all its values, and the Settings window lists each default with
/// a named Forget button that forgets it through settings.reset_default;
/// every control keeps a clean name. Breaks when the window offers no
/// way to save a value, or the list cannot forget one.
#[test]
fn defaults_from_the_window() {
    use murmel_engine::defaults::Kind;
    let mut h = harness();
    let dir = std::env::temp_dir().join(format!("murmel-window-defaults-{}", std::process::id()));
    h.state_mut().session.config_dir = Some(dir.clone());
    let result = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    h.state_mut().open_path(result);
    h.run_steps(3);
    h.state_mut()
        .call("slice.set", serde_json::json!({"axis": "z"}));
    h.state_mut().call(
        "arrows.set",
        serde_json::json!({"density": 60, "length": "equal"}),
    );
    h.state_mut()
        .call("select.item", serde_json::json!({"item": "arrows"}));
    h.run_steps(3);
    // A Properties row is a label, which kittest's label query reads by
    // its text; find it by the name it is read as.
    h.get_by(|n| n.label().as_deref() == Some("Arrows across: 60"))
        .click_secondary();
    h.run_steps(2);
    h.get_by_label("Save Arrows across as your default").click();
    h.run_steps(3);
    let d = &h.state().session.defaults;
    assert_eq!(d.get(Kind::Arrows, "density"), Some(&serde_json::json!(60)));
    assert!(d.get(Kind::Arrows, "length").is_none(), "only that value");
    // The row's menu keeps them all.
    h.get_by_label("Arrows: velocity").click_secondary();
    h.run_steps(2);
    h.get_by_label("Save these settings as your default")
        .click();
    h.run_steps(3);
    let d = &h.state().session.defaults;
    assert_eq!(
        d.get(Kind::Arrows, "length"),
        Some(&serde_json::json!("equal"))
    );
    // The toolbar's render mode keeps the one the window starts in.
    h.state_mut().call(
        "display.render_mode",
        serde_json::json!({"mode": "wireframe"}),
    );
    h.run_steps(2);
    h.get_by_label("Render mode: Wireframe").click_secondary();
    h.run_steps(2);
    h.get_by_label("Save Render mode as your default").click();
    h.run_steps(3);
    let d = &h.state().session.defaults;
    assert_eq!(
        d.get(Kind::View, "render_mode"),
        Some(&serde_json::json!("wireframe"))
    );
    // Settings lists them, each with a named Forget button.
    h.state_mut().settings_open = true;
    h.run_steps(2);
    let forget = "Forget Arrows · Arrows across: 60";
    assert!(named(&h, forget), "the density's row");
    assert_eq!(problems(&h), Vec::<String>::new(), "settings with defaults");
    // The Settings window scrolls on a small screen: a click by AccessKit
    // reaches the button wherever it is.
    h.get_by_label(forget).click_accesskit();
    h.run_steps(3);
    let d = &h.state().session.defaults;
    assert!(d.get(Kind::Arrows, "density").is_none());
    assert!(d.get(Kind::Arrows, "length").is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// Invariant: every character of the English strings has a glyph in
/// the window's fonts (Inter, its Greek subset, Lucide), so no string
/// draws a box in place of a letter (λ₂, ρ_ref, Ω of the derived
/// fields). Breaks when a string uses a sign the bundled subsets leave
/// out.
#[test]
fn every_string_has_its_glyphs() {
    let h = harness();
    let ftl = include_str!("../../../assets/i18n/en/murmel.ftl");
    let font = egui::FontId::proportional(14.0);
    // The check can fail: Inter has no nabla.
    assert!(!h.ctx.fonts_mut(|f| f.has_glyph(&font, '∇')));
    let mut missing: Vec<(String, char)> = Vec::new();
    for line in ftl.lines().filter(|l| !l.trim_start().starts_with('#')) {
        for c in line.chars().filter(|c| !c.is_ascii()) {
            if !h.ctx.fonts_mut(|f| f.has_glyph(&font, c)) {
                missing.push((line.to_string(), c));
            }
        }
    }
    assert_eq!(missing, Vec::new());
}

/// Invariant: the calculator of the Derived fields window computes a
/// typed formula and shows its field, writes a clicked function into
/// the formula, says in the window why a formula is refused, keeps a
/// formula among yours (settings.toml) and lists it with Compute, Show,
/// Remove, Edit and Forget, each named with the formula's name. Breaks
/// when the editor's calls do not reach field.calculate or a refusal is
/// only in the status bar.
#[test]
fn formulas_from_the_window() {
    let mut h = harness();
    let dir = std::env::temp_dir().join(format!("murmel-window-calc-{}", std::process::id()));
    h.state_mut().session.config_dir = Some(dir.clone());
    let result = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    h.state_mut().open_path(result);
    h.run_steps(3);
    h.state_mut().open_calculator();
    h.run_steps(3);
    assert_eq!(problems(&h), Vec::<String>::new(), "the calculator");
    h.get_by_role_and_label(Role::TextInput, "Name").click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "Name")
        .type_text("speed2");
    h.run_steps(2);
    h.get_by_label("dot").click();
    h.run_steps(3);
    assert_eq!(h.state().derived.formula, "dot(");
    h.get_by_role_and_label(Role::TextInput, "Formula")
        .type_text("velocity, velocity)");
    h.run_steps(2);
    assert_eq!(h.state().derived.formula, "dot(velocity, velocity)");
    h.get_by_label("Compute the formula").click();
    h.run_steps(3);
    let s = &h.state().session;
    assert!(s.fields().unwrap().formula("speed2").is_some());
    assert_eq!(
        s.display.field.as_ref().map(|c| c.name.as_str()),
        Some("speed2"),
        "computed and shown"
    );
    h.get_by_label("Keep the formula among yours").click();
    h.run_steps(3);
    assert_eq!(h.state().session.formulas.len(), 1);
    let (st, _, _) = murmel_engine::settings::load(&dir, false);
    assert_eq!(st.formulas.len(), 1, "written to settings.toml");
    for n in [
        "Compute again speed2",
        "Show speed2",
        "Remove speed2",
        "Edit speed2",
        "Forget speed2",
    ] {
        let label = n.replacen("Compute again", "Compute", 1);
        assert!(named(&h, &label), "{label}");
    }
    assert_eq!(problems(&h), Vec::<String>::new(), "a formula kept");
    // A refused formula says why, in the window.
    h.state_mut().derived.formula = "velocity *".into();
    h.run_steps(2);
    h.get_by_label("Compute the formula").click();
    h.run_steps(3);
    let why = h.state().derived.error.clone().unwrap_or_default();
    assert!(why.contains("column"), "{why}");
    h.get_by_label(&why);
    // Forgotten: the field stays, offered to keep again; removed: gone.
    h.get_by_label("Forget speed2").click();
    h.run_steps(3);
    assert!(h.state().session.formulas.is_empty());
    assert!(named(&h, "Keep speed2 among your formulas"));
    h.get_by_label("Remove speed2").click();
    h.run_steps(3);
    assert!(h.state().session.fields().unwrap().get("speed2").is_none());
    assert!(!named(&h, "Remove speed2"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Invariant: the Derived fields window computes a quantity with one
/// click and shows it, offers Show and Remove for it then, greys a
/// quantity whose input is missing, and a right-click on a reference
/// value keeps it as your default; every control is named. Breaks when
/// the window's calls do not reach `field.derive` or a button loses
/// the quantity in its name.
#[test]
fn derived_fields_from_the_window() {
    use murmel_engine::defaults::Kind;
    let mut h = harness();
    let dir = std::env::temp_dir().join(format!("murmel-window-derived-{}", std::process::id()));
    h.state_mut().session.config_dir = Some(dir.clone());
    // A mesh without fields: the window says so.
    h.state_mut().open_derived();
    h.run_steps(3);
    h.get_by_label("Open a result with fields to derive more from them.");
    let result = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    h.state_mut().open_path(result);
    h.run_steps(3);
    assert_eq!(problems(&h), Vec::<String>::new(), "the catalogue");
    assert!(named(&h, "Vorticity, Velocity: velocity"), "input choice");
    h.get_by_label("Compute Vorticity").click();
    h.run_steps(3);
    let s = &h.state().session;
    assert!(s.fields().unwrap().recipe("vorticity").is_some());
    assert_eq!(
        s.display.field.as_ref().map(|c| c.name.as_str()),
        Some("vorticity"),
        "computed and shown"
    );
    assert!(named(&h, "Show Vorticity") && named(&h, "Remove Vorticity"));
    assert_eq!(problems(&h), Vec::<String>::new(), "a field computed");
    // A reference value kept as your default.
    h.get_by_label("Pressure").click();
    h.run_steps(3);
    h.get_by(|n| n.role() == Role::SpinButton && n.label().as_deref() == Some("Density ρ_ref"))
        .click_secondary();
    h.run_steps(2);
    h.get_by_label("Save Reference density as your default")
        .click();
    h.run_steps(3);
    assert_eq!(
        h.state().session.defaults.get(Kind::Reference, "density"),
        Some(&serde_json::json!(1.225))
    );
    // Turbomachinery: the turning zone typed in the window reaches the
    // recipe, and its speed can be kept as your default.
    h.get_by_label("Turbomachinery").click();
    h.run_steps(3);
    assert_eq!(problems(&h), Vec::<String>::new(), "turbomachinery");
    h.state_mut().derived.rotation.as_mut().unwrap().0.zones[0].rpm = 1500.0;
    h.run_steps(2);
    h.get_by_label("Compute Relative velocity").click();
    h.run_steps(3);
    let r = h
        .state()
        .session
        .fields()
        .unwrap()
        .recipe("relative_velocity")
        .cloned()
        .unwrap_or_else(|| panic!("not computed: {:?}", h.state().error));
    assert_eq!(r.rotation.unwrap().zones[0].rpm, 1500.0);
    h.get_by(|n| {
        n.role() == Role::SpinButton && n.label().as_deref() == Some("Speed of turning zone 1")
    })
    .click_secondary();
    h.run_steps(2);
    h.get_by_label("Save Speed of rotation as your default")
        .click();
    h.run_steps(3);
    assert_eq!(
        h.state().session.defaults.get(Kind::Rotation, "rpm"),
        Some(&serde_json::json!(1500.0))
    );
    // Turbulence and walls: shown with their inputs and walls.
    for g in ["Turbulence", "Wall"] {
        h.get_by_label(g).click();
        h.run_steps(3);
        assert_eq!(problems(&h), Vec::<String>::new(), "{g}");
    }
    h.get_by_label("Vortices").click();
    h.run_steps(3);
    h.get_by_label("Remove Vorticity").click();
    h.run_steps(3);
    assert!(
        h.state()
            .session
            .fields()
            .unwrap()
            .get("vorticity")
            .is_none()
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Invariant: with a time series open the player bar shows below the
/// viewport with named controls, F6 reaches it between the viewport and
/// the properties, Next shows the next step, Play turns into Pause and
/// back, and full stop steps on. Breaks if the bar lost its names, its
/// buttons stopped calling the series tools, or Space and full stop no
/// longer reached the player.
#[test]
fn player_bar_steps_and_plays() {
    let mut h = harness();
    let dir = std::env::temp_dir().join(format!("murmel-player-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fields = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    for step in [10, 20, 30] {
        std::fs::copy(&fields, dir.join(format!("fields_{step:06}.vtkhdf"))).unwrap();
    }
    assert!(
        h.state_mut()
            .call("series.open", serde_json::json!({"path": dir}))
            .is_some()
    );
    h.run_steps(3);
    assert_eq!(problems(&h), Vec::<String>::new(), "player bar");
    let step = |h: &Harness<'_, MurmelApp>| h.state().session.series.as_ref().unwrap().entry().step;
    assert_eq!(step(&h), 10);
    h.get_by_label("Next step").click();
    h.run_steps(2);
    assert_eq!(step(&h), 20);
    h.get_by_label("Play").click();
    h.run_steps(2);
    assert!(h.state().session.series.as_ref().unwrap().playback.playing);
    assert!(named(&h, "Pause"));
    h.get_by_label("Pause").click();
    h.run_steps(2);
    assert!(!h.state().session.series.as_ref().unwrap().playback.playing);
    // The keys: full stop steps on (wrapping round while looping).
    let before = step(&h);
    h.key_press(egui::Key::Period);
    h.run_steps(2);
    assert_ne!(step(&h), before);
    // F6 from the viewport reaches the player next, also on the first
    // step, where "First step" is disabled and cannot take the focus.
    assert!(
        h.state_mut()
            .call("series.go", serde_json::json!({"to": "first"}))
            .is_some()
    );
    h.run_steps(2);
    assert_eq!(step(&h), 10);
    let mut seen = Vec::new();
    for _ in 0..7 {
        h.key_press(egui::Key::F6);
        h.run_steps(2);
        seen.push(focused_pane(&h).unwrap_or_default());
    }
    let at = seen.iter().position(|p| p == "Viewport").unwrap();
    assert_eq!(seen[at + 1], "Player", "{seen:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant: a playing series reads its next step ahead on a thread of
/// its own and shows every step in turn. Breaks if the player waited
/// for a reading that never came, or skipped steps.
#[test]
fn playing_reads_ahead() {
    let mut h = harness();
    let dir = std::env::temp_dir().join(format!("murmel-ahead-play-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fields = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    for step in [10, 20, 30] {
        std::fs::copy(&fields, dir.join(format!("fields_{step:06}.vtkhdf"))).unwrap();
    }
    let app = h.state_mut();
    assert!(
        app.call("series.open", serde_json::json!({"path": dir}))
            .is_some()
    );
    assert!(
        app.call(
            "series.play",
            serde_json::json!({"playing": true, "rate": 60.0, "loop": false})
        )
        .is_some()
    );
    let mut seen = Vec::new();
    for _ in 0..400 {
        h.run_steps(1);
        let r = h.state().session.series.as_ref().unwrap();
        if seen.last() != Some(&r.entry().step) {
            seen.push(r.entry().step);
        }
        if !r.playback.playing {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(seen, vec![10, 20, 30]);
    assert!(h.state().ahead.is_none(), "nothing left reading");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant (the maintainer's card of 2026-11-24, "Range over all
/// steps"): with a series open, a field shown is coloured over every
/// step, read by the window on a thread of its own; the range then
/// spans the steps and stays while the player steps on. Breaks if the
/// window never started the reading, or the colours rescaled per step.
#[test]
fn range_over_the_steps_reads_in_the_background() {
    let mut h = harness();
    let dir = std::env::temp_dir().join(format!("murmel-steps-range-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let vtu = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/box/foehncraft.vtu"),
    )
    .unwrap();
    for (step, factor) in [(10, 1.0f64), (20, 3.0)] {
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
            "<DataArray type=\"Float64\" Name=\"t\" format=\"ascii\">\n{}\n</DataArray>\n</CellData>",
            values.join("\n")
        );
        std::fs::write(
            dir.join(format!("s_{step}.vtu")),
            vtu.replacen("      </CellData>", &array, 1),
        )
        .unwrap();
    }
    let m = serde_json::json!({"format": "foehncraft.series", "format_version": 1, "entries": [
        {"step": 10, "files": {"fields": "s_10.vtu"}},
        {"step": 20, "files": {"fields": "s_20.vtu"}}]});
    std::fs::write(dir.join("series.json"), m.to_string()).unwrap();
    let app = h.state_mut();
    assert!(
        app.call(
            "series.open",
            serde_json::json!({"path": dir.join("series.json")})
        )
        .is_some()
    );
    assert!(
        app.call(
            "display.color_by",
            serde_json::json!({"by": "field", "field": "t"})
        )
        .is_some()
    );
    for _ in 0..200 {
        h.run_steps(1);
        if h.state().session.field_range() == Some((1.0, 12.0)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(h.state().session.field_range(), Some((1.0, 12.0)));
    h.get_by_label("Next step").click();
    h.run_steps(2);
    assert_eq!(h.state().session.field_range(), Some((1.0, 12.0)));
    assert_eq!(problems(&h), Vec::<String>::new(), "player bar");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant (the maintainer's cards of 2026-11-24, "Shown field on
/// label and tree" and "Follow steps, plus history plot"): with a field
/// colouring the mesh, a probe's tree row gives that field's value at
/// the probe, and the Probe history window reads the history on a
/// thread of its own, then shows a named plot, the field picker and
/// Export CSV, every control named. Breaks if the row went back to the
/// cell alone, or the window never took the reading.
#[test]
fn probe_values_and_history_window() {
    let mut h = harness();
    let result = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    h.state_mut().open_path(result);
    h.state_mut()
        .call("probe.add", serde_json::json!({"at": {"cell": 3}}));
    h.state_mut().call(
        "display.color_by",
        serde_json::json!({"by": "field", "field": "pressure"}),
    );
    h.run_steps(3);
    let row = h.root().children_recursive().find_map(|n| {
        n.accesskit_node()
            .label()
            .filter(|l| l.starts_with("Probe 1: pressure "))
    });
    assert!(row.is_some(), "the probe's row gives the pressure there");
    h.state_mut().open_probe_history();
    for _ in 0..300 {
        h.run_steps(1);
        if h.state().history.job.is_none()
            && murmel_tools::probe::kept(&h.state().session).is_some()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run_steps(3);
    assert!(
        murmel_tools::probe::kept(&h.state().session).is_some(),
        "the history was read"
    );
    let plot = h.root().children_recursive().find_map(|n| {
        n.accesskit_node()
            .label()
            .filter(|l| l.ends_with("at each probe over the run"))
    });
    assert_eq!(plot.as_deref(), Some("pressure at each probe over the run"));
    assert!(h.query_by_label("Export CSV…").is_some());
    assert_eq!(problems(&h), Vec::<String>::new(), "probe history window");
}

/// Invariant: with a run's tables open the chart panel shows above the
/// viewport's bottom with named controls and the statistics, F6 reaches
/// it, choosing another chart in its combo box calls charts.show, and
/// the palette's command hides it. Breaks if the panel lost its names,
/// its controls stopped calling the chart tools, or the region left F6.
#[test]
fn chart_panel_shows_and_switches() {
    let mut h = harness();
    let run =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/spray-run");
    assert!(
        h.state_mut()
            .call("charts.open", serde_json::json!({"path": run}))
            .is_some()
    );
    h.run_steps(3);
    assert_eq!(problems(&h), Vec::<String>::new(), "chart panel");
    assert!(named(&h, "Liquid mass flow through the datums"));
    assert!(shows(&h, "Spray"), "the statistics show");
    assert!(shows(&h, "8.28 g"), "8.28 g left through the outlet");
    assert!(
        h.state_mut()
            .call(
                "charts.show",
                serde_json::json!({"chart": "histogram", "datums": ["floor"]})
            )
            .is_some()
    );
    h.run_steps(3);
    assert!(named(&h, "Datum"));
    assert!(named(&h, "Droplet sizes through floor up to step 300"));
    assert_eq!(problems(&h), Vec::<String>::new(), "histogram");
    let mut seen = Vec::new();
    for _ in 0..8 {
        h.key_press(egui::Key::F6);
        h.run_steps(2);
        seen.push(focused_pane(&h).unwrap_or_default());
    }
    assert!(seen.iter().any(|p| p == "Charts"), "{seen:?}");
    let hide = murmel_tools::commands::commands()
        .into_iter()
        .find(|c| c.id == "charts.show:toggle")
        .unwrap();
    let (tool, input) = hide.input(&h.state().session).unwrap();
    h.state_mut().call(tool, input);
    h.run_steps(3);
    assert!(!h.state().session.charts.as_ref().unwrap().visible);
    assert!(!named(&h, "Datum"));
}

/// The README picture of the chart panel (not a check): the window with
/// a FoehnCraft spray run, its parcels, the player and the charts, drawn
/// with wgpu. `MURMEL_README_RUN` names the run folder (FoehnCraft's
/// `spray_datums` gallery case, tools/readme-spray/README.md) and
/// `MURMEL_README_OUT` the PNG to write.
#[test]
#[ignore = "README picture: needs MURMEL_README_RUN and MURMEL_README_OUT and a GPU or lavapipe"]
fn readme_chart_panel() {
    let run = std::path::PathBuf::from(std::env::var("MURMEL_README_RUN").unwrap());
    let out = std::path::PathBuf::from(std::env::var("MURMEL_README_OUT").unwrap());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 1040.0))
        .wgpu()
        .build_eframe(|cc| {
            let mut app = MurmelApp::new(cc, None).unwrap();
            app.session.config_dir = None;
            app.bridge_path = None;
            app
        });
    h.run_steps(2);
    let calls = [
        (
            "mesh.open",
            serde_json::json!({"path": run.join("fields.vtkhdf")}),
        ),
        ("series.open", serde_json::json!({"path": run, "step": 150})),
        ("slice.set", serde_json::json!({"axis": "z"})),
        (
            "display.color_by",
            serde_json::json!({"by": "field", "field": "velocity"}),
        ),
        (
            "parcels.set",
            serde_json::json!({"color": "diameter", "map": "inferno", "scale": 2}),
        ),
        ("view.standard", serde_json::json!({"view": "top"})),
        ("select.clear", serde_json::json!({})),
        ("charts.show", serde_json::json!({"chart": "mass_flow"})),
    ];
    for (id, input) in calls {
        let r = h.state_mut().call_envelope(id, input.clone());
        assert!(r.ok(), "{id} {input}: {:?}", r.diagnostics);
    }
    h.state_mut().show_tree = false;
    h.state_mut().show_props = false;
    h.run_steps(6);
    let img = h.render().unwrap();
    img.save(&out).unwrap();
}

/// Whether a node's name starts with `text`.
fn named_from(h: &Harness<'_, MurmelApp>, text: &str) -> bool {
    h.root().children_recursive().any(|n| {
        let a = n.accesskit_node();
        [a.label(), a.value()]
            .into_iter()
            .flatten()
            .any(|t| t.starts_with(text))
    })
}

/// Invariant (the maintainer's card 1 of 2026-11-24, overview plus
/// regions): in the window a box drawn on an overview reads its blocks on
/// a thread of its own; the status bar says how many and of how many
/// cells, with Cancel, the tree's note tells the blocks shown once they
/// are in, and Esc stops a read, which leaves the blocks shown as they
/// were. Every control keeps a clean name. Breaks when the read holds the
/// window, the status or the note is lost, or Esc does not reach the
/// read; the engine tests do not draw the window.
#[test]
fn regions_on_an_overview_in_the_window() {
    let dir = std::env::temp_dir().join(format!("murmel-regions-win-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let (mesh, results) = murmel_mesh::read_results(&plain).unwrap();
    let path = dir.join("blocked.vtkhdf");
    murmel_mesh::blocks::write_blocked(&path, &mesh, &results, 100).unwrap();
    let (open, config) = (path.clone(), dir.clone());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(move |cc| {
            let mut app = MurmelApp::new(cc, None).unwrap();
            app.session.config_dir = Some(config);
            app.bridge_path = None;
            app.session.memory_budget = Some(1e-9);
            app.session.cache_dir = None;
            app.open_path(open);
            app
        });
    h.run_steps(3);
    opened(&mut h);
    assert!(h.state().session.mesh.as_ref().unwrap().overview.is_some());
    assert!(named_from(&h, "Overview: the 776 cells"));
    assert!(named_from(&h, "A box, sphere or slab region reads"));
    h.state_mut().session.memory_budget = None;

    // A box near a corner: blocks 0 and 3, 200 cells.
    let corner =
        serde_json::json!({"shape": "box", "center": [0.4, 0.175, 0.1], "size": [0.4, 0.25, 0.1]});
    assert!(h.state_mut().call("region.set", corner).is_some());
    assert!(
        h.state().session.block_job.is_some(),
        "read in the background"
    );
    h.run_steps(1);
    assert!(
        named_from(&h, "Reading 2 blocks (200 cells) for the regions"),
        "the status bar"
    );
    assert!(named(
        &h,
        "Cancel. Stop reading these blocks; the overview stays (Esc)"
    ));
    let start = std::time::Instant::now();
    while h.state().session.block_job.is_some() {
        assert!(start.elapsed().as_secs() < 60, "the read never ended");
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    h.run_steps(2);
    let m = h.state().session.mesh.as_ref().unwrap();
    assert_eq!(m.blocks.as_ref().unwrap().subset.blocks, vec![0, 3]);
    assert!(named_from(
        &h,
        "The regions show the cells of 2 of the 8 blocks"
    ));
    assert_eq!(problems(&h), Vec::<String>::new(), "blocks shown");

    // The box to the other end: another read, which Esc stops.
    let far = serde_json::json!({"center": [3.6, 1.825, 0.9]});
    assert!(h.state_mut().call("region.set", far).is_some());
    assert!(h.state().session.block_job.is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    let s = &h.state().session;
    assert!(s.block_job.is_none());
    assert!(
        matches!(&s.blocks_problem, Some((_, None))),
        "{:?}",
        s.blocks_problem
    );
    let m = s.mesh.as_ref().unwrap();
    assert_eq!(m.blocks.as_ref().unwrap().subset.blocks, vec![0, 3]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Invariant (the maintainer's card 7 of 2026-11-24: the overview is the
/// surface, the block ranges and a preview grid): in the window a slice
/// on an overview reads the preview grid on a thread of its own; the
/// status bar says so with Cancel, the slice draws once the grid is in,
/// and the tree's note tells the grid. Esc stops a read. Every control
/// keeps a clean name. Breaks when the read holds the window, the status
/// or the note is lost, or Esc does not reach the read; the engine tests
/// do not draw the window.
#[test]
fn slices_on_an_overview_in_the_window() {
    let dir = std::env::temp_dir().join(format!("murmel-preview-win-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let (mesh, results) = murmel_mesh::read_results(&plain).unwrap();
    let path = dir.join("blocked.vtkhdf");
    murmel_mesh::blocks::write_blocked(&path, &mesh, &results, 100).unwrap();
    let (open, config) = (path.clone(), dir.clone());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(move |cc| {
            let mut app = MurmelApp::new(cc, None).unwrap();
            app.session.config_dir = Some(config);
            app.bridge_path = None;
            app.session.memory_budget = Some(1e-9);
            app.session.cache_dir = None;
            app.session.preview_cells = 16;
            app.open_path(open);
            app
        });
    h.run_steps(3);
    opened(&mut h);
    assert!(h.state().session.mesh.as_ref().unwrap().overview.is_some());

    assert!(
        h.state_mut()
            .call("slice.set", serde_json::json!({"axis": "x"}))
            .is_some()
    );
    assert!(
        h.state().session.preview_job.is_some(),
        "read in the background"
    );
    h.run_steps(1);
    assert!(
        named_from(&h, "Sampling the volume on a preview grid"),
        "the status bar"
    );
    assert!(named(
        &h,
        "Cancel. Stop sampling; the slices show again when you add one (Esc)"
    ));
    let start = std::time::Instant::now();
    while h.state().session.preview_job.is_some() {
        assert!(start.elapsed().as_secs() < 60, "the read never ended");
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    h.run_steps(2);
    let m = h.state().session.mesh.as_ref().unwrap();
    assert!(m.fields.preview().is_some());
    assert!(m.surface.triangles() > 0, "the slice draws");
    // the tree's note, after what it says of the overview and the regions
    let note = "Slices, iso-surfaces and arrows show on a preview grid of 16 × 8 × 4 cells";
    assert!(h.root().children_recursive().any(|n| {
        let a = n.accesskit_node();
        [a.label(), a.value()]
            .into_iter()
            .flatten()
            .any(|t| t.starts_with("Overview:") && t.contains(note))
    }));
    assert_eq!(problems(&h), Vec::<String>::new(), "preview shown");

    // Opened again and sliced: Esc stops the read.
    let again = path.clone();
    h.state_mut().open_path(again);
    h.run_steps(3);
    opened(&mut h);
    assert!(
        h.state_mut()
            .call("slice.set", serde_json::json!({"axis": "y"}))
            .is_some()
    );
    assert!(h.state().session.preview_job.is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    let s = &h.state().session;
    assert!(s.preview_job.is_none());
    assert!(
        matches!(&s.preview_problem, Some((1, None))),
        "{:?}",
        s.preview_problem
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Invariant (the maintainer's card 8 of 2026-11-24, a block cache for
/// probes): on an overview a probe reads the block of the file holding
/// it on a thread of its own, the window polling it, and its tree row
/// then gives the field colouring the mesh at the probe. Breaks when the
/// window never polls the read (the row stays "…" or "–"), or when the
/// call waits for it. The engine and tool tests read at once.
#[test]
fn probes_on_an_overview_in_the_window() {
    let dir = std::env::temp_dir().join(format!("murmel-probe-win-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let (mesh, results) = murmel_mesh::read_results(&plain).unwrap();
    let path = dir.join("blocked.vtkhdf");
    murmel_mesh::blocks::write_blocked(&path, &mesh, &results, 100).unwrap();
    let (open, config) = (path.clone(), dir.clone());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(move |cc| {
            let mut app = MurmelApp::new(cc, None).unwrap();
            app.session.config_dir = Some(config);
            app.bridge_path = None;
            app.session.memory_budget = Some(1e-9);
            app.session.cache_dir = None;
            app.open_path(open);
            app
        });
    h.run_steps(3);
    opened(&mut h);
    assert!(h.state().session.mesh.as_ref().unwrap().overview.is_some());
    // (opened as an overview; the probe's block fits the budget)
    h.state_mut().session.memory_budget = None;
    h.state_mut().call(
        "display.color_by",
        serde_json::json!({"by": "field", "field": "pressure"}),
    );
    let b = mesh.bounds();
    let at: [f64; 3] = std::array::from_fn(|k| b.min[k] + 0.4 * (b.max[k] - b.min[k]));
    assert!(
        h.state_mut()
            .call("probe.add", serde_json::json!({"at": {"point": at}}))
            .is_some()
    );
    assert!(
        h.state().session.probe_blocks.job.is_some(),
        "read on a thread of its own"
    );
    let start = std::time::Instant::now();
    while h.state().session.probe_blocks.job.is_some() {
        assert!(start.elapsed().as_secs() < 60, "the read never ended");
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    h.run_steps(2);
    let row = h.root().children_recursive().find_map(|n| {
        n.accesskit_node()
            .label()
            .filter(|l| l.starts_with("Probe 1: "))
    });
    let row = row.expect("the probe's row");
    assert!(row.starts_with("Probe 1: pressure "), "{row}");
    assert_eq!(problems(&h), Vec::<String>::new(), "probe on an overview");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant (the maintainer's card 8 of 2026-11-24, a block cache for
/// streamlines): on an overview a streamline set is traced through the
/// blocks of the file on a thread of its own, the window polling it, and
/// its lines then draw; the controls keep clean names meanwhile and
/// after. Breaks when the window never polls the trace (no lines draw),
/// or when the call waits for it. The engine and tool tests trace at
/// once.
#[test]
fn streamlines_on_an_overview_in_the_window() {
    let dir = std::env::temp_dir().join(format!("murmel-stream-win-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let (mesh, results) = murmel_mesh::read_results(&plain).unwrap();
    let path = dir.join("blocked.vtkhdf");
    murmel_mesh::blocks::write_blocked(&path, &mesh, &results, 100).unwrap();
    let (open, config) = (path.clone(), dir.clone());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(move |cc| {
            let mut app = MurmelApp::new(cc, None).unwrap();
            app.session.config_dir = Some(config);
            app.bridge_path = None;
            app.session.memory_budget = Some(1e-9);
            app.session.cache_dir = None;
            app.open_path(open);
            app
        });
    h.run_steps(3);
    opened(&mut h);
    assert!(h.state().session.mesh.as_ref().unwrap().overview.is_some());
    // (opened as an overview; the blocks the lines reach fit the budget)
    h.state_mut().session.memory_budget = None;
    let set = serde_json::json!({"new": true, "field": "velocity", "seed": "line", "count": 8});
    assert!(h.state_mut().call("stream.set", set).is_some());
    assert!(
        h.state().session.probe_blocks.stream_job.is_some(),
        "traced on a thread of its own"
    );
    let start = std::time::Instant::now();
    while h.state().session.probe_blocks.stream_job.is_some() {
        assert!(start.elapsed().as_secs() < 60, "the trace never ended");
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    h.run_steps(2);
    let s = &h.state().session;
    let t = s.display.stream_traces.as_ref().unwrap().0[0]
        .clone()
        .unwrap();
    assert!(
        t.fits(&s.display.streams[0]) && !t.lines.is_empty(),
        "{t:?}"
    );
    let m = s.mesh.as_ref().unwrap();
    assert!(!m.surface.stream_positions.is_empty(), "the lines draw");
    assert_eq!(
        problems(&h),
        Vec::<String>::new(),
        "streamlines on an overview"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant (the maintainer's card 8 of 2026-11-24, stream slices and
/// iso-surfaces): on an overview a slice shows on the preview grid at
/// once, its Properties saying it is being cut, and once it has rested
/// the window cuts it through the blocks on a thread of its own and
/// draws its exact faces, Properties then telling the blocks; the
/// controls keep clean names meanwhile and after. Breaks when the window
/// never polls the cut (the grid's faces stay), or when the call waits
/// for it. The engine and tool tests cut at once or poll by hand.
#[test]
fn slices_on_an_overview_are_cut_in_the_window() {
    let dir = std::env::temp_dir().join(format!("murmel-cut-win-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let (mesh, results) = murmel_mesh::read_results(&plain).unwrap();
    let path = dir.join("blocked.vtkhdf");
    murmel_mesh::blocks::write_blocked(&path, &mesh, &results, 100).unwrap();
    let (open, config) = (path.clone(), dir.clone());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(move |cc| {
            let mut app = MurmelApp::new(cc, None).unwrap();
            app.session.config_dir = Some(config);
            app.bridge_path = None;
            app.session.memory_budget = Some(1e-9);
            app.session.cache_dir = None;
            app.open_path(open);
            app
        });
    h.run_steps(3);
    opened(&mut h);
    assert!(h.state().session.mesh.as_ref().unwrap().overview.is_some());
    // (opened as an overview; the blocks the slice cuts fit the budget)
    h.state_mut().session.memory_budget = None;
    assert!(
        h.state_mut()
            .call("slice.set", serde_json::json!({"axis": "x"}))
            .is_some()
    );
    let rows = |h: &Harness<'_, MurmelApp>| -> Vec<String> {
        let s = &h.state().session;
        let m = s.mesh.as_ref().unwrap();
        murmel_engine::inspect::rows(m, &s.display, murmel_engine::display::Item::Slice(0))
            .into_iter()
            .map(|r| r.key.to_string())
            .collect()
    };
    assert!(
        h.state().session.display.block_cuts.is_none(),
        "not cut yet"
    );
    assert!(rows(&h).contains(&"cutting".to_string()), "{:?}", rows(&h));
    let start = std::time::Instant::now();
    let cut = |h: &Harness<'_, MurmelApp>| {
        h.state()
            .session
            .display
            .block_cuts
            .as_ref()
            .is_some_and(|c| c.0.iter().any(|c| c.faces.is_some()))
    };
    while !cut(&h) {
        assert!(start.elapsed().as_secs() < 60, "the cut never ended");
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    h.run_steps(2);
    assert!(
        rows(&h).contains(&"cut_blocks".to_string()),
        "{:?}",
        rows(&h)
    );
    assert!(h.state().session.probe_blocks.cut_job.is_none());
    let m = h.state().session.mesh.as_ref().unwrap();
    assert!(m.surface.triangles() > 0, "the slice draws");
    assert_eq!(problems(&h), Vec::<String>::new(), "slices on an overview");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant (the maintainer's card 4 of 2026-11-24, a large legacy
/// file): opening a mesh that is not blocked and needs more than the
/// memory budget asks first: "Convert and open" converts it on a thread
/// of its own (the status bar following it) to a blocked copy beside it,
/// which opens as an overview; opening it again opens that copy at once;
/// "Open whole" opens the file itself whole; ticking "Convert large
/// meshes without asking" keeps that setting. Every control of the
/// question keeps a clean name. Breaks when the question is skipped
/// (the mesh opens whole beyond the budget), when a button does not do
/// what it says, when the window never polls the conversion, or when an
/// up-to-date copy is converted again. The engine test converts without
/// the window.
#[test]
fn a_large_mesh_that_is_not_blocked_is_offered_for_conversion() {
    let dir = std::env::temp_dir().join(format!("murmel-convert-win-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let vtu = dir.join("channel.vtu");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/channel/foehncraft.vtu");
    std::fs::copy(&fixture, &vtu).unwrap();
    let copy = dir.join("channel.blocked.vtkhdf");
    let (open, config) = (vtu.clone(), dir.clone());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(move |cc| {
            let mut app = MurmelApp::new(cc, None).unwrap();
            app.session.config_dir = Some(config);
            app.bridge_path = None;
            app.session.memory_budget = Some(1e-9);
            app.session.cache_dir = None;
            app.open_path(open);
            app
        });
    h.run_steps(3);
    assert!(h.state().session.mesh.is_none(), "asked before it opens");
    assert!(h.state().convert_offer.is_some());
    h.get_by_label("Open a large mesh");
    assert_eq!(problems(&h), Vec::<String>::new(), "the question");
    h.get_by_label("Convert large meshes without asking")
        .click();
    h.run_steps(1);
    h.get_by_label("Convert and open").click();
    h.run_steps(1);
    assert!(h.state().convert_offer.is_none());
    assert!(h.state().session.convert_large, "the setting kept");
    let start = std::time::Instant::now();
    while h.state().session.convert_job.is_some() {
        assert!(start.elapsed().as_secs() < 60, "the conversion never ended");
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    opened(&mut h);
    let m = h.state().session.mesh.as_ref().expect("the copy opens");
    assert_eq!(m.path, copy);
    assert!(m.overview.is_some(), "as an overview");

    // Again: the up-to-date copy opens at once.
    h.state_mut().open_path(vtu.clone());
    h.run_steps(2);
    opened(&mut h);
    assert!(h.state().session.convert_job.is_none() && h.state().convert_offer.is_none());
    assert_eq!(h.state().session.mesh.as_ref().unwrap().path, copy);

    // Without the copy and the setting, "Open whole" opens the file.
    std::fs::remove_file(&copy).unwrap();
    h.state_mut().session.convert_large = false;
    h.state_mut().open_path(vtu.clone());
    h.run_steps(2);
    assert!(h.state().convert_offer.is_some());
    h.get_by_label("Open whole").click();
    h.run_steps(2);
    opened(&mut h);
    let m = h.state().session.mesh.as_ref().unwrap();
    assert_eq!(m.path, vtu);
    assert_eq!(m.mesh.cell_count(), 776);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Invariant: on an overview the model tree has a "Block boxes" row
/// whose eye shows and hides the boxes of its blocks, named with how
/// many blocks there are and, once a region is drawn, how many it reads;
/// selecting it fills the Properties panel. Every control keeps a clean
/// name. Breaks when the row is missing on an overview or shown on a
/// mesh read whole, when its eye does not reach the boxes, or when its
/// name loses the counts; the engine and tool tests do not draw the
/// window.
#[test]
fn block_boxes_in_the_window() {
    let dir = std::env::temp_dir().join(format!("murmel-boxes-win-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let (mesh, results) = murmel_mesh::read_results(&plain).unwrap();
    let path = dir.join("blocked.vtkhdf");
    murmel_mesh::blocks::write_blocked(&path, &mesh, &results, 100).unwrap();
    let (open, config) = (path.clone(), dir.clone());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(move |cc| {
            let mut app = MurmelApp::new(cc, None).unwrap();
            app.session.config_dir = Some(config);
            app.bridge_path = None;
            app.session.memory_budget = Some(1e-9);
            app.session.cache_dir = None;
            app.open_path(open);
            app
        });
    h.run_steps(3);
    opened(&mut h);
    let n = {
        let s = &h.state().session;
        assert!(s.mesh.as_ref().unwrap().overview.is_some());
        s.display.block_boxes.as_ref().unwrap().count()
    };
    assert!(named(&h, &format!("Block boxes, {n}")));
    h.get_by_label("Show Block boxes").click();
    h.run_steps(2);
    {
        let s = &h.state().session;
        assert!(s.display.show_blocks);
        assert_eq!(s.mesh.as_ref().unwrap().surface.block_boxes, n);
    }
    assert!(named(&h, "Hide Block boxes"));
    // a region (within a budget its blocks fit): the row tells how many
    // blocks it reads
    h.state_mut().session.memory_budget = None;
    let b = mesh.bounds();
    let at: [f64; 3] = std::array::from_fn(|k| b.min[k] + 0.1 * (b.max[k] - b.min[k]));
    let size: [f64; 3] = std::array::from_fn(|k| 0.1 * (b.max[k] - b.min[k]));
    let region = serde_json::json!({"new": true, "shape": "box", "center": at, "size": size});
    assert!(h.state_mut().call("region.set", region).is_some());
    let start = std::time::Instant::now();
    while h.state().session.block_job.is_some() {
        assert!(start.elapsed().as_secs() < 60, "the read never ended");
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    h.run_steps(2);
    let read = h
        .state()
        .session
        .display
        .block_boxes
        .as_ref()
        .unwrap()
        .read
        .len();
    assert!(read > 0 && read < n);
    assert!(named(&h, &format!("Block boxes, {read} of {n} read")));
    h.get_by_label(&format!("Block boxes, {read} of {n} read"))
        .click();
    h.run_steps(2);
    assert!(named_from(&h, "Block boxes"), "the Properties heading");
    assert_eq!(problems(&h), Vec::<String>::new(), "block boxes shown");
    // read whole: no row
    h.state_mut()
        .open_path_as(path.clone(), murmel_engine::OpenMode::Whole);
    h.run_steps(3);
    opened(&mut h);
    assert!(h.state().session.mesh.as_ref().unwrap().overview.is_none());
    assert!(!named_from(&h, "Block boxes"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Invariant (the maintainer's card of 2026-11-24: beyond the memory
/// budget a blocked VTKHDF file opens as an overview): the model tree
/// says so, with the file's cells and blocks, and "Read the cells
/// anyway" opens the whole mesh; Settings shows the budget, as a named
/// box once one is set, and its checkbox gives it back to half the
/// memory; every control keeps a clean name. Breaks when the note or the
/// button is lost, the button opens the overview again, or the budget
/// box has no name; the engine and tool tests do not click.
#[test]
fn overview_in_the_window() {
    let dir = std::env::temp_dir().join(format!("murmel-overview-win-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    let (mesh, results) = murmel_mesh::read_results(&plain).unwrap();
    let path = dir.join("blocked.vtkhdf");
    murmel_mesh::blocks::write_blocked(&path, &mesh, &results, 100).unwrap();
    let (open, config) = (path.clone(), dir.clone());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(move |cc| {
            let mut app = MurmelApp::new(cc, None).unwrap();
            app.session.config_dir = Some(config);
            app.bridge_path = None;
            // A budget of a byte: the file needs more.
            app.session.memory_budget = Some(1e-9);
            app.session.cache_dir = None;
            app.open_path(open);
            app
        });
    h.run_steps(3);
    opened(&mut h);
    let m = h.state().session.mesh.as_ref().unwrap();
    assert!(m.overview.is_some(), "{:?}", h.state().error);
    assert_eq!(m.cells(), mesh.cell_count());
    let note = "Overview: the 776 cells in 8 blocks are not read.";
    assert!(
        h.root().children_recursive().any(|n| n
            .accesskit_node()
            .value()
            .is_some_and(|v| v.starts_with(note))),
        "the tree's note"
    );
    assert_eq!(problems(&h), Vec::<String>::new(), "an overview");

    h.get_by_label("Read the cells anyway").click();
    h.run_steps(3);
    opened(&mut h);
    let m = h.state().session.mesh.as_ref().unwrap();
    assert!(m.overview.is_none());
    assert_eq!(m.mesh.cell_count(), mesh.cell_count());
    assert!(!named(&h, "Read the cells anyway"));

    h.state_mut().settings_open = true;
    h.run_steps(2);
    assert!(named(&h, "Budget"), "the budget box");
    assert_eq!(problems(&h), Vec::<String>::new(), "settings with a budget");
    // The Settings window scrolls on a small screen: a click by AccessKit
    // reaches the box wherever it is.
    let half = |n: &AccessKitNode<'_>| {
        n.label()
            .is_some_and(|l| l.starts_with("Half of this computer's memory"))
    };
    h.get_by(half).click_accesskit();
    h.run_steps(3);
    assert_eq!(h.state().session.memory_budget, None);
    assert!(!named(&h, "Budget"));
    let _ = std::fs::remove_dir_all(dir);
}

/// The channel's result with nine spray parcels spread through its box,
/// in FoehnCraft's layout (`murmel_mesh::parcels::write_parcels`), in a
/// fresh folder of its own.
fn parcels_beside_the_channel(h: &mut Harness<'_, MurmelApp>) -> std::path::PathBuf {
    use murmel_mesh::{Field, Location};
    let result = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    h.state_mut().open_path(result);
    h.run_steps(3);
    let b = h.state().session.mesh.as_ref().unwrap().mesh.bounds();
    let at = |t: f64| [0, 1, 2].map(|k| b.min[k] + t * (b.max[k] - b.min[k]));
    let points: Vec<[f64; 3]> = (1..=9).map(|i| at(f64::from(i) / 10.0)).collect();
    let n = points.len();
    let fields = vec![
        Field {
            name: "velocity".into(),
            unit: None,
            location: Location::Point,
            components: 3,
            values: (0..3 * n).map(|i| (i % 3) as f64).collect(),
        },
        Field {
            name: "diameter".into(),
            unit: None,
            location: Location::Point,
            components: 1,
            values: (1..=n).map(|i| i as f64 * 1e-5).collect(),
        },
    ];
    let dir = std::env::temp_dir().join(format!("murmel-parcel-window-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("parcels_000100.vtkhdf");
    murmel_mesh::parcels::write_parcels(&path, &points, &fields, 0.5, 100, "channel").unwrap();
    path
}

/// Invariant (D268, first milestone: spray parcels in the window): with
/// no parcels the tree offers a named "Open spray parcels…" button; with
/// a parcel file open the tree has a named row with an eye, a named
/// size box and a context menu, and the parcels a named colour bar of
/// their own whose title opens their settings; the bar's filter box,
/// the menu's size choice, the size box and the eye each change the
/// parcels through `parcels.set` (one undo step each), and choosing a
/// solid colour drops their bar. Breaks if a control calls nothing,
/// changes the display behind the tools' back or loses its name; the
/// tool tests (`parcel_tools`) do not click, and the README chart
/// picture only draws the parcels.
#[test]
fn spray_parcels_in_the_window() {
    use murmel_engine::parcel::Size;
    let mut h = harness();
    let path = parcels_beside_the_channel(&mut h);
    assert!(named(&h, "Open spray parcels…"), "the open button");
    h.state_mut()
        .call("parcels.open", serde_json::json!({ "path": path }));
    h.run_steps(3);
    assert!(!named(&h, "Open spray parcels…"), "no open button now");
    let label = h.state().parcels_label();
    assert!(
        label.starts_with("Parcels · parcels_000100.vtkhdf"),
        "{label}"
    );
    assert!(named(&h, &label), "the tree row");
    assert!(
        named(&h, "Colours of the spray parcels: diameter"),
        "the parcels' colour bar"
    );
    assert!(
        named_from(&h, "Colour bar of the spray parcels: diameter from "),
        "the bar itself"
    );
    assert!(shows(&h, "9 of 9 shown"), "the count");
    assert_eq!(problems(&h), Vec::<String>::new(), "parcels");

    // The bar's title opens their settings: the diameter filter.
    let steps = |h: &Harness<'_, MurmelApp>| h.state().session.journal.undo.len();
    let before = steps(&h);
    h.get_by_label("Colours of the spray parcels: diameter")
        .click();
    h.run_steps(3);
    assert_eq!(problems(&h), Vec::<String>::new(), "the parcels' menu");
    h.get_by_role_and_label(Role::CheckBox, "Only some diameters")
        .click();
    h.run_steps(3);
    let v = h.state().session.display.parcels.clone().unwrap();
    let [lo, hi] = v.diameter.expect("a diameter filter");
    assert!(
        (lo - 1e-5).abs() < 1e-12 && (hi - 9e-5).abs() < 1e-12,
        "{lo} {hi}"
    );
    assert_eq!(steps(&h), before + 1, "one undo step");
    assert!(
        named(&h, "From"),
        "the filter's bounds, the menu still open"
    );
    h.key_press(egui::Key::Escape);
    h.run_steps(3);

    // The row's context menu sizes them by diameter.
    h.get_by_role_and_label(Role::Button, &label)
        .click_secondary();
    h.run_steps(3);
    h.get_by_label("By diameter").click();
    h.run_steps(3);
    assert_eq!(
        h.state().session.display.parcels.as_ref().unwrap().size,
        Size::Diameter
    );
    assert_eq!(steps(&h), before + 2);

    // The size box.
    h.get_by_role_and_label(Role::SpinButton, "Size").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_role_and_label(Role::SpinButton, "Size")
        .type_text("2.5");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(
        h.state().session.display.parcels.as_ref().unwrap().scale,
        2.5
    );
    assert_eq!(steps(&h), before + 3);

    // The eye hides them, and shows them again.
    h.get_by_label(&format!("Hide {label}")).click();
    h.run_steps(3);
    assert!(!h.state().session.display.parcels.as_ref().unwrap().enabled);
    assert_eq!(h.state().session.mesh.as_ref().unwrap().surface.parcels, 0);
    h.get_by_label(&format!("Show {label}")).click();
    h.run_steps(3);
    assert!(h.state().session.display.parcels.as_ref().unwrap().enabled);
    assert_eq!(steps(&h), before + 5);

    // A solid colour: no bar of their own.
    h.get_by_label("Colours of the spray parcels: diameter")
        .click();
    h.run_steps(3);
    h.get_by_label("Solid colour").click();
    h.run_steps(3);
    assert!(
        h.state()
            .session
            .display
            .parcels
            .as_ref()
            .unwrap()
            .color
            .is_none()
    );
    assert!(
        !named(&h, "Colours of the spray parcels: diameter"),
        "no bar for a solid colour"
    );
    assert_eq!(problems(&h), Vec::<String>::new(), "solid parcels");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// Invariant (the maintainer's card of 2026-11-24: arrows coloured by
/// their own magnitude have a colour bar of their own): the title of the
/// arrows' bar opens a named menu whose style entries change the arrow
/// field's style through `field.style` (the field the arrows show, not
/// the field on the mesh) and whose colouring radio buttons call
/// `arrows.set`; a solid colour drops the bar. Breaks if the menu styles
/// the mesh's field instead, a radio button calls nothing, or the menu
/// loses its names; `vector_arrows_in_the_window` only checks that the
/// bar is there.
#[test]
fn arrows_colour_bar_menu() {
    use murmel_engine::FieldMode;
    use murmel_engine::glyph::ArrowColor;
    let mut h = harness();
    let result = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
    h.state_mut().open_path(result);
    for (tool, input) in [
        (
            "display.color_by",
            serde_json::json!({"by": "field", "field": "pressure"}),
        ),
        ("slice.set", serde_json::json!({"axis": "z"})),
        ("arrows.set", serde_json::json!({})),
    ] {
        let r = h.state_mut().call_envelope(tool, input.clone());
        assert!(r.ok(), "{tool} {input}: {:?}", r.diagnostics);
    }
    h.run_steps(3);
    let bar = "Colours of the arrows: |velocity|";
    assert!(named(&h, bar), "the arrows' colour bar");
    h.get_by_label(bar).click();
    h.run_steps(3);
    assert_eq!(problems(&h), Vec::<String>::new(), "the arrows' menu");
    h.get_by_label("Banded").click();
    h.run_steps(3);
    let d = &h.state().session.display;
    assert_eq!(
        d.style_of("velocity").mode,
        FieldMode::Banded,
        "arrows' field"
    );
    assert_ne!(
        d.style_of("pressure").mode,
        FieldMode::Banded,
        "not the mesh's"
    );
    assert!(named(&h, "Bands"), "the menu stays open with the bands");

    h.get_by_role_and_label(Role::RadioButton, "Solid").click();
    h.run_steps(3);
    assert_eq!(
        h.state().session.display.arrows.as_ref().unwrap().color,
        ArrowColor::Solid
    );
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!named(&h, bar), "no bar for solid arrows");
    assert_eq!(problems(&h), Vec::<String>::new(), "solid arrows");
}

/// Invariant (V0.5, quality colouring): coloured by quality, the legend
/// shows a named histogram, and its "Hide cells below" box, its "Worst
/// 10 %" button and the box again set and clear the threshold through
/// `display.threshold`, so fewer cells show and then all of them again.
/// Breaks if the box or the button calls nothing or the surface is not
/// rebuilt; the tool tests do not click, and the name test only checks
/// that the legend has names.
#[test]
fn quality_legend_and_threshold() {
    let mut h = harness();
    h.state_mut().call(
        "display.color_by",
        serde_json::json!({"by": "quality", "metric": "skewness"}),
    );
    h.run_steps(3);
    assert!(named_from(&h, "Histogram of "), "the histogram has a name");
    let cells = |h: &Harness<'_, MurmelApp>| h.state().session.mesh.as_ref().unwrap().surface.cells;
    let all = cells(&h);
    assert_eq!(all, 776);
    h.get_by_role_and_label(Role::CheckBox, "Hide cells below")
        .click();
    h.run_steps(3);
    let t = h.state().session.display.threshold.expect("a threshold");
    assert!(cells(&h) < all, "fewer cells at {t}");
    assert!(
        shows(&h, &format!("{} of 776 cells shown", cells(&h))),
        "the count of the cells shown"
    );
    h.get_by_label("Worst 10 %").click();
    h.run_steps(3);
    let worst = h.state().session.display.threshold.expect("a threshold");
    assert_ne!(worst, t, "the worst tenth");
    let shown = cells(&h);
    assert!(shown < all && shown > 0, "{shown}");
    assert_eq!(problems(&h), Vec::<String>::new(), "threshold");
    h.get_by_role_and_label(Role::CheckBox, "Hide cells below")
        .click();
    h.run_steps(3);
    assert_eq!(h.state().session.display.threshold, None);
    assert_eq!(cells(&h), all);
}

/// Invariant (zoom to box, the maintainer's card of 2026-11-25): W arms
/// the rubber band, the next left drag in the viewport zooms into the box
/// instead of orbiting, and the mouse navigates again afterwards (one box
/// per press); Esc drops an armed band; the toolbar button has a name.
/// Breaks if the viewport's navigation takes the drag first, the band
/// stays armed after one box, or W is not bound; the tool test runs the
/// tool, not the pointer and the key.
#[test]
fn zoom_box_by_key_and_drag() {
    let mut h = harness();
    assert!(named(&h, "Zoom to box"), "toolbar button");
    let centre = h.state().viewport_rect.center();
    let cam = h.state().session.camera.clone();
    h.input_mut().events.push(egui::Event::PointerMoved(centre));
    h.run_steps(2);
    h.key_press(egui::Key::W);
    h.run_steps(2);
    assert!(h.state().zoom_armed, "W arms the band");
    drag(
        &mut h,
        centre - egui::vec2(60.0, 40.0),
        egui::vec2(120.0, 80.0),
        6,
    );
    let now = h.state().session.camera.clone();
    assert!(!h.state().zoom_armed, "one box per press");
    assert!(h.state().band.is_none());
    assert!(
        now.distance < 0.5 * cam.distance,
        "zoomed in: {} -> {}",
        cam.distance,
        now.distance
    );
    assert_eq!(
        (now.yaw_deg, now.pitch_deg),
        (cam.yaw_deg, cam.pitch_deg),
        "no orbit"
    );
    // The next drag orbits again.
    drag(&mut h, centre, egui::vec2(40.0, 0.0), 4);
    assert_ne!(
        h.state().session.camera.yaw_deg,
        now.yaw_deg,
        "the drag orbits"
    );
    // Esc drops an armed band.
    h.key_press(egui::Key::W);
    h.run_steps(2);
    assert!(h.state().zoom_armed);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!h.state().zoom_armed, "Esc cancels");
}

/// Invariant (configurable shortcuts, the maintainer's cards of
/// 2026-11-25): in the shortcut editor a key is recorded by pressing it,
/// saved to [keys] in settings.toml and runs its command at once; a key
/// another command has stops at a warning naming it, and only Replace
/// moves it; a mouse input another binding has stops at a warning too,
/// and Cancel leaves the bindings as they were; the editor has no
/// unnamed controls. Breaks if recording lets the key run a command,
/// the window keeps the old table after a change, or a clash is applied
/// without asking; the tool test checks the tool, not the window.
#[test]
fn shortcut_editor_records_and_warns() {
    let dir = std::env::temp_dir().join(format!("murmel-app-keys-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut h = harness();
    h.state_mut().session.config_dir = Some(dir.clone());
    h.state_mut().open_bindings(crate::shortcuts::Tab::Keys);
    h.run_steps(3);
    assert_eq!(problems(&h), Vec::<String>::new(), "keys tab");
    // Record Shift + F for Fit view (a free key).
    h.get_by_label("Add a key to Fit view").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F);
    h.run_steps(3);
    assert_eq!(h.state().keys_of("view.fit"), ["F", "Shift+F"]);
    let (st, _, _) = murmel_engine::settings::load(&dir, false);
    assert_eq!(st.keys.get("view.fit").unwrap(), &["F", "Shift+F"]);
    // The new key fits the view at once (the editor stays open).
    h.state_mut()
        .call("view.zoom", serde_json::json!({"factor": 0.3}));
    h.run_steps(2);
    let d = h.state().session.camera.distance;
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F);
    h.run_steps(30);
    assert!(
        h.state().session.camera.distance > 2.0 * d,
        "Shift + F fits"
    );
    // W is Zoom to box: a warning, nothing moves until Replace. (The
    // pointer leaves the row first: the tooltip of the new key's button
    // would cover Add key.)
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(1430.0, 890.0)));
    h.run_steps(3);
    h.get_by_label("Add a key to Fit view").click();
    h.run_steps(2);
    h.key_press(egui::Key::W);
    h.run_steps(3);
    assert!(!h.state().zoom_armed, "a recorded key runs nothing");
    let clash = h.state().bindings.as_ref().unwrap().clash.clone().unwrap();
    assert_eq!(
        (clash.key.as_str(), clash.others.clone()),
        ("W", vec!["Zoom to box (rubber band)".to_string()])
    );
    assert_eq!(h.state().keys_of("window.zoom_box"), ["W"]);
    h.get_by_label("Replace").click();
    h.run_steps(3);
    assert_eq!(h.state().keys_of("view.fit"), ["F", "Shift+F", "W"]);
    assert!(h.state().keys_of("window.zoom_box").is_empty());
    // Reset all puts the built-in keys back.
    h.get_by_label("Reset all keys").click();
    h.run_steps(3);
    assert_eq!(h.state().keys_of("window.zoom_box"), ["W"]);
    assert!(h.state().session.keys.is_empty());
    // Mouse: give the second binding the first one's input; Cancel.
    h.state_mut().open_bindings(crate::shortcuts::Tab::Mouse);
    h.run_steps(3);
    assert_eq!(problems(&h), Vec::<String>::new(), "mouse tab");
    let before = h.state().session.navigation.clone();
    let mut draft = before.bindings.clone();
    draft[1].gesture = draft[0].gesture;
    draft[1].button = draft[0].button;
    draft[1].mods = draft[0].mods;
    h.state_mut().apply_mouse_for_test(draft, 1);
    h.run_steps(3);
    assert!(h.state().bindings.as_ref().unwrap().mouse_clash.is_some());
    assert_eq!(h.state().session.navigation, before, "nothing applied yet");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().bindings.as_ref().unwrap().mouse_clash.is_none());
    assert_eq!(h.state().session.navigation, before);
    let _ = std::fs::remove_dir_all(dir);
}

/// Invariant: the menu bar opens its menus by name, an item runs its
/// command, Undo names the step it undoes, and an item that cannot work
/// without a mesh is greyed out once the mesh is closed. Breaks when the
/// bar stops drawing the table of murmel-tools, an item no longer
/// reaches its command, Undo loses its step name, or a greyed item still
/// runs. The table test in murmel-tools checks only the data.
#[test]
fn menu_bar_runs_commands() {
    let mut h = harness();
    // Click the menu item whose name starts with `start` (the key shown
    // beside it may be part of the name).
    let click = |h: &mut Harness<'_, MurmelApp>, start: &str| {
        let start = start.to_string();
        h.get_by(move |n| {
            n.role() == Role::Button && n.label().is_some_and(|l| l.starts_with(&start))
        })
        .click();
        h.run_steps(3);
    };
    let yaw = |h: &Harness<'_, MurmelApp>| h.state().session.camera.yaw_deg;
    let before = yaw(&h);
    h.state_mut().call(
        "view.orbit",
        serde_json::json!({"yaw_deg": 30.0, "pitch_deg": 0.0}),
    );
    h.run_steps(2);
    assert_ne!(yaw(&h), before);
    // Edit ▸ Undo names the orbit and takes it back.
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("Edit"))
        .click();
    h.run_steps(3);
    click(&mut h, "Undo Orbit");
    assert!(
        (yaw(&h) - before).abs() < 1e-9,
        "Undo walked back the orbit"
    );
    // File ▸ Close mesh closes it; then Fit view is greyed out.
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("File"))
        .click();
    h.run_steps(3);
    click(&mut h, "Close mesh");
    assert!(h.state().session.mesh.is_none(), "Close mesh ran");
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("View"))
        .click();
    h.run_steps(3);
    let fit = h.get_by(|n| {
        n.role() == Role::Button && n.label().is_some_and(|l| l.starts_with("Fit view"))
    });
    assert!(fit.accesskit_node().is_disabled(), "Fit view needs a mesh");
    assert_eq!(problems(&h), Vec::<String>::new());
}

/// Invariant: a control's tip names it and says what it does after a
/// moment, shows more of its help page after a longer rest or at once
/// with Shift, and starts over when the pointer comes back. Breaks if the
/// rich tip never came, came at once without Shift, or stayed rich on
/// the next visit.
#[test]
fn tips_grow_with_rest_and_shift() {
    let mut h = harness();
    let fit = h.state().i18n.tr("toolbar-fit");
    // The tip drawn in the last frame: its title and whether it is rich.
    let shown = |h: &mut Harness<'_, MurmelApp>| {
        let slot = egui::Id::unique(crate::tips::SHOWN);
        h.ctx
            .data_mut(|d| d.remove_temp::<crate::tips::Shown>(slot));
        h.step();
        h.ctx
            .data(|d| d.get_temp::<crate::tips::Shown>(slot))
            .map(|s| {
                assert_eq!(s.title, "Fit view");
                assert_eq!(s.page, Some("fit-zoom"));
                s.rich
            })
    };
    let hover_fit = |h: &mut Harness<'_, MurmelApp>| {
        let f = fit.clone();
        h.get_by(move |n| n.role() == Role::Button && n.label().as_deref() == Some(&f))
            .hover();
    };
    hover_fit(&mut h);
    h.run_steps(2);
    assert_eq!(shown(&mut h), Some(false), "the quick tip");
    h.run_steps(5);
    assert_eq!(shown(&mut h), Some(true), "rich after a rest");
    // Away and back: quick again; Shift makes it rich at once.
    h.hover_at(egui::pos2(700.0, 450.0));
    h.run_steps(2);
    assert_eq!(shown(&mut h), None, "the tip closed");
    hover_fit(&mut h);
    h.run_steps(2);
    assert_eq!(shown(&mut h), Some(false), "quick again");
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    assert_eq!(shown(&mut h), Some(true), "Shift shows more");
    assert_eq!(problems(&h), Vec::<String>::new());
}

/// Invariant: F1 on a tip opens its page in the Help panel, F1 with no
/// tip opens the contents or closes the panel, a page's links and Back
/// walk the manual, search finds pages by their words, and a page's
/// commands run from it. Breaks if F1 still opened the shortcut sheet,
/// the panel lost its way back, or a page could not be reached.
#[test]
fn f1_enters_the_manual() {
    let mut h = harness();
    let page = |h: &Harness<'_, MurmelApp>| h.state().help_view.as_ref().map(|v| v.page);
    h.key_press(egui::Key::F1);
    h.run_steps(2);
    assert_eq!(page(&h), Some(None), "F1: the contents");
    assert!(named(&h, "Help"), "the pane has a name");
    assert_eq!(problems(&h), Vec::<String>::new(), "the contents");
    h.get_by(|n| n.role() == Role::Link && n.label().as_deref() == Some("Clip planes"))
        .click_accesskit();
    h.run_steps(2);
    assert_eq!(page(&h), Some(Some("clip")));
    assert_eq!(problems(&h), Vec::<String>::new(), "a page");
    // Its commands run from the page.
    let on = |h: &Harness<'_, MurmelApp>| h.state().session.display.clip().is_some();
    assert!(!on(&h));
    h.get_by(|n| {
        n.role() == Role::Button
            && n.label().is_some_and(|l| l.starts_with("Clip normal to X"))
            && pane_of(n).as_deref() == Some("Help")
    })
    .click_accesskit();
    h.run_steps(2);
    assert!(on(&h), "the page's command ran");
    h.get_by_label("Back").click_accesskit();
    h.run_steps(2);
    assert_eq!(page(&h), Some(None), "Back: the contents");
    h.get_by_label("Search the manual")
        .type_text("streamline seed");
    h.run_steps(2);
    assert!(
        h.query_by(|n| n.role() == Role::Link && n.label().as_deref() == Some("Streamlines"))
            .is_some()
    );
    h.key_press(egui::Key::F1);
    h.run_steps(2);
    assert_eq!(page(&h), None, "F1 again closes it");
    // F1 on a tip: its page.
    let fit = h.state().i18n.tr("toolbar-fit");
    h.get_by(move |n| n.role() == Role::Button && n.label().as_deref() == Some(&fit))
        .hover();
    h.run_steps(3);
    h.key_press(egui::Key::F1);
    h.run_steps(2);
    assert_eq!(page(&h), Some(Some("fit-zoom")), "F1 on the Fit tip");
}

/// Invariant: File ▸ Page setup opens a window whose controls all have
/// names, and a choice there changes the page of Print and Export PDF at
/// once. Breaks if the window's choices did not reach settings.set_page.
#[test]
fn page_setup_from_the_file_menu() {
    let mut h = harness();
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("File"))
        .click();
    h.run_steps(3);
    h.get_by(|n| {
        n.role() == Role::Button && n.label().is_some_and(|l| l.starts_with("Page setup"))
    })
    .click();
    h.run_steps(3);
    assert!(named(&h, "Paper"), "the paper box");
    assert_eq!(problems(&h), Vec::<String>::new());
    h.get_by_label("Portrait").click();
    h.run_steps(2);
    assert_eq!(
        h.state().session.page.orientation,
        murmel_engine::settings::Orientation::Portrait
    );
}
