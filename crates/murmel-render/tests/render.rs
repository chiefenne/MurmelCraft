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
use murmel_render::overlay::{self, Overlays};
use murmel_render::{Shoot, diff_share, read_png, render_session, render_session_with, write_png};

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
    check_shot(name, &shot);
}

fn check_shot(name: &str, shot: &murmel_render::Shot) {
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
    s.display.selection = Item::parse("patch:outlet", &s.mesh.as_ref().unwrap().mesh)
        .into_iter()
        .collect();
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
        s.display.clips = vec![Clip {
            origin: b.center(),
            normal: [1.0, 0.0, 0.0],
            mode,
            enabled: true,
        }];
        s.redraw();
        check_session(&format!("channel-clip-{}", mode.key()), &s);
    }
}

/// Several clip planes: a flat cut along x and a flat cut along y
/// take a quarter out of the channel, so the corner where the two
/// sections meet shows (cells by type, iso view).
#[test]
fn channel_two_flat_clips() {
    let mut s = session(StandardView::ISO);
    s.display.color_by = ColorBy::CellType;
    let b = s.mesh.as_ref().unwrap().mesh.bounds();
    // Normals towards the eye, as clip.set by axis gives them.
    let e = s.camera.to_eye();
    s.display.clips = [[e.x.signum(), 0.0, 0.0], [0.0, e.y.signum(), 0.0]]
        .map(|normal| Clip {
            origin: b.center(),
            normal,
            mode: ClipMode::Flat,
            enabled: true,
        })
        .to_vec();
    s.redraw();
    check_session("channel-two-flat-clips", &s);
}

/// V0.4 picking: a crinkle cut by cell type with one core polyhedron
/// picked in the middle of the view: tinted, and outlined in the
/// selection colour over everything (its hidden edges too).
#[test]
fn channel_picked_cell() {
    let mut s = session(StandardView::ISO);
    s.display.color_by = ColorBy::CellType;
    let b = s.mesh.as_ref().unwrap().mesh.bounds();
    s.display.clips = vec![Clip {
        origin: b.center(),
        normal: [1.0, 0.0, 0.0],
        mode: ClipMode::Crinkle,
        enabled: true,
    }];
    s.redraw();
    let m = s.mesh.as_ref().unwrap();
    let hit = murmel_engine::pick::pick(&m.surface, &s.camera, [0.0, 0.0], s.aspect()).unwrap();
    s.display.selection = vec![Item::Cell(hit.cell)];
    s.redraw();
    check_session("channel-picked-cell", &s);
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

/// V0.5 quality: colour by non-orthogonality on the quality ramp, with
/// the threshold at a third of the range, so the best cells are gone and
/// the faces they lay open show the colours of the cells behind.
#[test]
fn channel_quality_threshold() {
    let mut s = session(StandardView::ISO);
    s.display.color_by = ColorBy::Quality;
    s.display.metric = murmel_engine::Metric::NonOrthogonality;
    s.redraw();
    let (lo, hi) = s.display.quality_range().unwrap();
    s.display.threshold = Some(lo + (hi - lo) / 3.0);
    s.redraw();
    check_session("channel-quality-threshold", &s);
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

/// V1.0 OpenFOAM reader: the 81 dual polyhedra of FoehnCraft's
/// `polyDualMesh` case, colour by cell type with a flat clip through the
/// middle, so the cut polyhedra show that the reversed neighbour faces
/// close every cell. A reader that left a face the wrong way round or
/// lost a patch would open holes or change the section.
#[test]
fn openfoam_dual_polyhedra_clipped() {
    let mut s = Session::new(Tokens::builtin().unwrap());
    s.viewport = [480.0, 320.0];
    s.open(&root().join("test-fixtures/openfoam/dual_polyhedra"))
        .unwrap();
    s.camera.set_standard(StandardView::ISO);
    s.fit();
    s.display.color_by = ColorBy::CellType;
    let b = s.mesh.as_ref().unwrap().mesh.bounds();
    s.display.clips = vec![Clip {
        normal: [1.0, 0.0, 0.0],
        origin: b.center(),
        mode: ClipMode::Flat,
        enabled: true,
    }];
    s.redraw();
    check_session("openfoam-dual-clip-flat", &s);
}

/// Export (the maintainer's cards of 2026-11-24, "same look as on
/// screen"): the channel coloured by patch at 1.5 times a 320 x 213
/// viewport, edges 1.5 pixels wide, with the patch legend top left and
/// the axis triad bottom left drawn over it at 1.5 times the window's size.
/// A change to the wide-line shader, the overlay layout, its font or its
/// tokens moves pixels here and in no other test (the window paints its
/// legend with egui).
#[test]
fn export_scaled_with_legend_and_triad() {
    let mut s = session(StandardView::ISO);
    s.display.color_by = ColorBy::Patch;
    s.redraw();
    let mut shot = render_session_with(
        &s,
        &Shoot {
            width: 480,
            height: 320,
            transparent: false,
            line_width: 1.5,
        },
    )
    .unwrap();
    let legend = murmel_engine::Legend::of(&s).unwrap();
    let i18n = murmel_engine::i18n::I18n::english().unwrap();
    overlay::draw(
        &mut shot.rgba,
        480,
        320,
        &Overlays {
            legend: Some(&legend),
            triad: Some(&s.camera),
            i18n: &i18n,
            tokens: &s.tokens,
            dark: true,
            scale: 1.5,
        },
    )
    .unwrap();
    check_shot("channel-export-scaled", &shot);
}

/// Invariant: a transparent export has alpha 0 where nothing is drawn,
/// 255 on the faces, and its anti-aliased silhouette keeps the faces'
/// colour (divided by alpha in linear light, not darkened towards the
/// black it was cleared to). Breaks when the background is drawn anyway or
/// the edge pixels stay premultiplied; the reference images are opaque.
#[test]
fn transparent_background() {
    let mut s = session(StandardView::ISO);
    s.display.render_mode = RenderMode::Shaded;
    s.redraw();
    let shot = render_session_with(
        &s,
        &Shoot {
            width: 240,
            height: 160,
            transparent: true,
            line_width: 1.0,
        },
    )
    .unwrap();
    let px = |x: u32, y: u32| {
        let i = 4 * (y * 240 + x) as usize;
        [
            shot.rgba[i],
            shot.rgba[i + 1],
            shot.rgba[i + 2],
            shot.rgba[i + 3],
        ]
    };
    assert_eq!(px(0, 0)[3], 0);
    assert_eq!(px(239, 159)[3], 0);
    assert_eq!(px(120, 80)[3], 255);
    let edge: Vec<[u8; 4]> = shot
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .filter(|p| p[3] > 32 && p[3] < 224)
        .collect();
    assert!(!edge.is_empty(), "no anti-aliased silhouette");
    let lum = |p: &[u8; 4]| p[0] as u32 + p[1] as u32 + p[2] as u32;
    let darkest_face = shot
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] == 255)
        .map(lum)
        .min()
        .unwrap();
    let dark_edges = edge.iter().filter(|p| lum(p) + 24 < darkest_face).count();
    assert!(
        dark_edges * 10 < edge.len(),
        "{dark_edges} of {} edge pixels darker than every face",
        edge.len()
    );
}

/// Invariant: lines two pixels wide cover clearly more of the picture
/// (summed alpha) than one-pixel lines, and the feature edges are still
/// drawn: the quads of
/// `vs_wide` widen every segment. Breaks when the width uniform or the
/// instance layout is wrong (lines vanish or stay thin).
#[test]
fn wide_lines_are_wider() {
    let mut s = session(StandardView::ISO);
    s.display.render_mode = RenderMode::FeatureEdges;
    s.redraw();
    let shoot = |w: f32| {
        render_session_with(
            &s,
            &Shoot {
                width: 240,
                height: 160,
                transparent: true,
                line_width: w,
            },
        )
        .unwrap()
    };
    let covered = |shot: &murmel_render::Shot| {
        shot.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0)
            .count()
    };
    let (thin, wide) = (covered(&shoot(1.0)), covered(&shoot(2.0)));
    assert!(thin > 300, "the edges are missing: {thin}");
    assert!(wide as f64 > 1.5 * thin as f64, "thin {thin}, wide {wide}");
}
