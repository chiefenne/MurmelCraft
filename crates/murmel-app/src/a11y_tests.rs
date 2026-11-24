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
        .type_text("40");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(
        h.state().session.display.arrows.as_ref().unwrap().density,
        40
    );
    assert_eq!(h.state().session.journal.undo.len(), before + 1);
    assert!(named(&h, "Length"), "length box");
    assert_eq!(problems(&h), Vec::<String>::new(), "arrows on a slice");
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
