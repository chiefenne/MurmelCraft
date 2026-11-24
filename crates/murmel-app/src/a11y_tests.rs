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
