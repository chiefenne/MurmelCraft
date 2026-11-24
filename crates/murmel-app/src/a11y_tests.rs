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
