//! Render test (UI spec, Principles: every view testable in CI by image
//! comparison). The channel fixture is opened, framed in the iso view and
//! rendered headless; the image must match the stored reference within a
//! tolerance that absorbs rasteriser differences between GPU drivers.
//!
//! Keep rule: the invariant is the picture of the default view (shaded
//! with edges, the solid default colour, gradient, fit). A change to the camera, the
//! shader, the tokens or the reader that moves pixels fails here and
//! nowhere else. To accept an intended change, run with MURMEL_BLESS=1 and
//! commit the new reference with the reason in the pull request.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use murmel_engine::{
    Clip, ClipMode, ColorBy, Item, RenderMode, ScreenAxis, Session, StandardView, Tokens,
    Visibility,
};
use murmel_render::{diff_share, read_png, render_session, write_png};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn session(view: StandardView) -> Session {
    let mut s = Session::new(Tokens::builtin().unwrap());
    s.viewport = [480.0, 320.0];
    s.open(&root().join("test-fixtures/channel/foehncraft.vtu"))
        .unwrap();
    s.camera.set_standard(view);
    s.fit();
    s
}

fn check(name: &str, view: StandardView) {
    check_session(name, &session(view));
}

fn check_session(name: &str, s: &Session) {
    let shot = render_session(s, 480, 320).unwrap();
    let reference =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/reference/{name}.png"));
    let out = root().join(format!("target/render-test/{name}.png"));
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    write_png(&out, shot.width, shot.height, &shot.rgba).unwrap();
    if std::env::var_os("MURMEL_BLESS").is_some() {
        write_png(&reference, shot.width, shot.height, &shot.rgba).unwrap();
        return;
    }
    let (w, h, want) = read_png(&reference).unwrap();
    assert_eq!((w, h), (shot.width, shot.height));
    let share = diff_share(&shot.rgba, &want, 24);
    assert!(
        share < 0.01,
        "{name}: {:.2} % of pixels differ (adapter {}); actual image in {}",
        100.0 * share,
        shot.adapter,
        out.display()
    );
}

#[test]
fn channel_iso_shaded_with_edges() {
    check("channel-iso", StandardView::ISO);
}

#[test]
fn channel_front_orthographic() {
    check("channel-front", StandardView::FRONT);
}

/// V0.2 display: colour by patch role with the walls hidden shows the
/// inlet (blue) and outlet (orange) ends and nothing else.
#[test]
fn channel_roles_without_walls() {
    let mut s = session(StandardView::ISO);
    s.display.color_by = ColorBy::Role;
    let walls = Item::parse("patch:walls", &s.mesh.as_ref().unwrap().mesh).unwrap();
    let m = s.mesh.as_ref().unwrap().mesh.clone();
    s.display.set_visibility(Visibility::Hide, &[walls], &m);
    s.redraw();
    check_session("channel-roles-no-walls", &s);
}

/// V0.2 display: colour by cell type (prism layers amber, the core
/// polyhedra teal) with the outlet selected (tinted, edges in the
/// selection colour).
#[test]
fn channel_cell_types_outlet_selected() {
    let mut s = session(StandardView::ISO);
    s.display.color_by = ColorBy::CellType;
    s.display.selection = Item::parse("patch:outlet", &s.mesh.as_ref().unwrap().mesh);
    s.redraw();
    check_session("channel-cell-types-selected", &s);
}

/// V0.3 render modes 2 to 4 (shaded, wireframe, feature edges) in the
/// cell-type colours, so prism layers and core read in each.
#[test]
fn channel_render_modes() {
    for mode in [
        RenderMode::Shaded,
        RenderMode::Wireframe,
        RenderMode::FeatureEdges,
    ] {
        let mut s = session(StandardView::ISO);
        s.display.color_by = ColorBy::CellType;
        s.display.render_mode = mode;
        s.redraw();
        check_session(&format!("channel-mode-{}", mode.key()), &s);
    }
}

/// V0.3 views: an edge view and a corner view of the cube, and the front
/// view rolled by 90 degrees (the inlet end on top) through
/// `Camera::rotate`.
#[test]
fn channel_cube_views_and_roll() {
    let edge = StandardView::parse("front_top").unwrap();
    check("channel-front-top", edge);
    check(
        "channel-back-left-bottom",
        StandardView::parse("back_left_bottom").unwrap(),
    );
    let mut s = session(StandardView::FRONT);
    s.camera.rotate(ScreenAxis::Z, 90.0);
    s.fit();
    check_session("channel-front-rolled", &s);
}

/// V0.4 clip plane: the channel cut at x = centre and coloured by cell
/// type, seen from the iso view (the cut faces the camera). The crinkle cut shows whole cells (the
/// amber prism layers along the walls, the teal core between them); the
/// flat cut shows the cells' sections in the plane.
#[test]
fn channel_clip_by_cell_type() {
    for mode in ClipMode::ALL {
        let mut s = session(StandardView::ISO);
        s.display.color_by = ColorBy::CellType;
        let b = s.mesh.as_ref().unwrap().mesh.bounds();
        s.display.clip = Some(Clip {
            origin: b.center(),
            normal: [1.0, 0.0, 0.0],
            mode,
            enabled: true,
        });
        s.redraw();
        check_session(&format!("channel-clip-{}", mode.key()), &s);
    }
}

/// V0.4 cell types: with the prism layers hidden the core's polyhedra
/// show as a closed block (V0.2 left holes where the layers were).
#[test]
fn channel_prisms_hidden_show_the_core() {
    let mut s = session(StandardView::ISO);
    s.display.color_by = ColorBy::CellType;
    let m = s.mesh.as_ref().unwrap().mesh.clone();
    s.display.set_visibility(
        Visibility::Hide,
        &[Item::Kind(murmel_mesh::CellKind::Prism)],
        &m,
    );
    s.redraw();
    check_session("channel-prisms-hidden", &s);
}

/// The image is not a blank gradient: the mesh covers a good part of the
/// view and its edges are darker than its faces.
#[test]
fn the_mesh_fills_the_view() {
    let shot = render_session(&session(StandardView::ISO), 240, 160).unwrap();
    let bg = render_session(&Session::new(Tokens::builtin().unwrap()), 240, 160).unwrap();
    let covered = diff_share(&shot.rgba, &bg.rgba, 8);
    assert!(covered > 0.15, "mesh covers only {:.1} %", 100.0 * covered);
}

/// Invariant: offscreen renders from several threads at once share the
/// one device (`Gpu::shared` hands out the same one) and all give the same
/// picture. Each render used to create and drop its own Vulkan instance;
/// Mesa 26 lavapipe crashed the test process (SIGSEGV) when that happened
/// on several threads at once, which no single-threaded test shows.
#[test]
fn renders_on_many_threads_share_one_device() {
    let a = murmel_render::Gpu::shared().unwrap() as *const _;
    let b = murmel_render::Gpu::shared().unwrap() as *const _;
    assert_eq!(a, b);
    let s = session(StandardView::ISO);
    let first = render_session(&s, 160, 120).unwrap().rgba;
    std::thread::scope(|scope| {
        let runs: Vec<_> = (0..6)
            .map(|_| scope.spawn(|| render_session(&s, 160, 120).unwrap().rgba))
            .collect();
        for r in runs {
            assert!(r.join().unwrap() == first);
        }
    });
}
