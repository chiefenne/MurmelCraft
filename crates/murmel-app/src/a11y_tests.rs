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
    h.get_by_label(forget).click();
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
    // F6 from the viewport reaches the player next.
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
    h.get_by(|n| {
        n.label()
            .is_some_and(|l| l.starts_with("Half of this computer's memory"))
    })
    .click();
    h.run_steps(3);
    assert_eq!(h.state().session.memory_budget, None);
    assert!(!named(&h, "Budget"));
    let _ = std::fs::remove_dir_all(dir);
}
