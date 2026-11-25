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

/// The channel's FoehnCraft result (`fields.vtkhdf` layout), framed in
/// the iso view.
fn field_session(name: &str, component: murmel_mesh::Component) -> Session {
    let mut s = Session::new(Tokens::builtin().unwrap());
    s.viewport = [480.0, 320.0];
    s.open(&root().join("test-fixtures/vtkhdf/channel_fields.vtkhdf"))
        .unwrap();
    s.camera.set_standard(StandardView::ISO);
    s.fit();
    s.display.color_by = ColorBy::Field;
    s.display.field = Some(murmel_engine::FieldChoice {
        name: name.into(),
        component,
    });
    s.redraw();
    s
}

/// Stage 1 field colouring, the first look (decided 2026-11-24): the
/// channel's pressure in smooth colours on viridis over the whole
/// field's range, mesh edges dark over the colours. A wrong node value,
/// range, colour table or texel lookup moves the colours here.
#[test]
fn field_pressure_smooth() {
    let s = field_session("pressure", murmel_mesh::Component::Magnitude);
    check_session("field-pressure-smooth", &s);
}

/// Flat colours (one per cell, the solver's values) on a flat cut along
/// x, so the sections show the cells' own values; the speed (velocity
/// magnitude) without edges.
#[test]
fn field_speed_flat_clipped() {
    let mut s = field_session("velocity", murmel_mesh::Component::Magnitude);
    s.display.render_mode = RenderMode::Shaded;
    s.display.field_styles.insert(
        "velocity".into(),
        murmel_engine::FieldStyle {
            mode: murmel_engine::FieldMode::Flat,
            ..Default::default()
        },
    );
    let b = s.mesh.as_ref().unwrap().mesh.bounds();
    s.display.clips = vec![Clip {
        origin: b.center(),
        normal: [1.0, 0.0, 0.0],
        mode: ClipMode::Flat,
        enabled: true,
    }];
    s.redraw();
    check_session("field-speed-flat-clip", &s);
}

/// Banded colours with contour lines: 12 bands of the pressure with 11
/// lines on their edges, on the reversed turbo map, shaded without
/// edges. Lines that lost their anti-aliasing, sat off the band edges or
/// vanished would show here.
#[test]
fn field_pressure_banded_lines() {
    let mut s = field_session("pressure", murmel_mesh::Component::Magnitude);
    s.display.render_mode = RenderMode::Shaded;
    s.display.field_styles.insert(
        "pressure".into(),
        murmel_engine::FieldStyle {
            mode: murmel_engine::FieldMode::Banded,
            bands: 12,
            lines: true,
            line_count: 11,
            map: Some("turbo".into()),
            reverse: true,
            ..Default::default()
        },
    );
    s.redraw();
    check_session("field-pressure-banded-lines", &s);
}

/// An exported picture of a field (the movie and picture exports draw
/// the legend with [`overlay`], the window with egui): the speed in 8
/// bands with 7 contour lines, and its colour bar with the field's name,
/// unit, ticks and the line marks. A ramp legend drawn upside down, with
/// the wrong bands or without the marks would show here.
#[test]
fn field_export_with_legend() {
    let mut s = field_session("velocity", murmel_mesh::Component::Magnitude);
    s.display.field_styles.insert(
        "velocity".into(),
        murmel_engine::FieldStyle {
            mode: murmel_engine::FieldMode::Banded,
            bands: 8,
            lines: true,
            line_count: 7,
            ..Default::default()
        },
    );
    s.redraw();
    let mut shot = render_session_with(&s, &Shoot::new(480, 320)).unwrap();
    let legend = murmel_engine::Legend::of(&s).unwrap();
    let i18n = murmel_engine::i18n::I18n::english().unwrap();
    overlay::draw(
        &mut shot.rgba,
        480,
        320,
        &Overlays {
            legend: Some(&legend),
            own_bars: &[],
            triad: None,
            caption: None,
            i18n: &i18n,
            tokens: &s.tokens,
            dark: true,
            scale: 1.0,
        },
    )
    .unwrap();
    check_shot("field-export-legend", &shot);
}

/// Stage 2 slices (decided 2026-11-24: placed like clip planes, the mesh
/// a faint outline, the mesh's field style, a series with count and
/// spacing): three cross-sections of the channel's speed spread along
/// the flow, in smooth colours, inside the mesh's faint feature edges.
/// The duct's peak shows in the middle of each section although the core
/// is one cell across: slices cut the cells' tetrahedra, whose centroid
/// carries the cell's own value. A wrong section, series spacing, value
/// inside a cell or outline colour moves pixels here.
#[test]
fn field_slice_series() {
    let mut s = field_session("velocity", murmel_mesh::Component::Magnitude);
    let b = s.mesh.as_ref().unwrap().mesh.bounds();
    let mut slice = murmel_engine::Slice::new(b.center(), [1.0, 0.0, 0.0]);
    slice.spread(&b, 3);
    s.display.slices = vec![slice];
    s.redraw();
    check_session("field-slice-series", &s);
}

/// Stage 2 iso-surfaces (decided 2026-11-24: one value each, solid or
/// coloured by the field shown): the tube where the speed is 1.2 m/s in
/// its own colour, and the plane where the pressure is 0 Pa (half way
/// along the duct) coloured by the speed, over the faint outline.
/// Marching tetrahedra that leave holes, flip normals or lose the colour
/// mode would show here.
#[test]
fn field_iso_surfaces() {
    let mut s = field_session("velocity", murmel_mesh::Component::Magnitude);
    let choice = |name: &str| murmel_engine::FieldChoice {
        name: name.into(),
        component: murmel_mesh::Component::Magnitude,
    };
    s.display.isos = vec![
        murmel_engine::Iso {
            field: choice("speed"),
            value: 1.2,
            enabled: true,
            color: murmel_engine::IsoColor::Solid,
        },
        murmel_engine::Iso {
            field: choice("pressure"),
            value: 0.0,
            enabled: true,
            color: murmel_engine::IsoColor::Field,
        },
    ];
    s.redraw();
    check_session("field-iso-surfaces", &s);
}

/// Stage 3 vector arrows with the defaults of the review of 2026-11-24
/// (the maintainer's card: a floor on the length, coloured by their own
/// magnitude): the duct's velocity on its horizontal mid-plane over the
/// pressure, 20 across the slice, longest in the core, the slow ones at
/// the side walls a quarter of that and dark in the colour map. Arrows
/// that leave the grid, lose their length or floor, turn inside out
/// (dark on the lit side) or lose their own colours move pixels here.
#[test]
fn field_arrows_on_a_slice() {
    let mut s = field_session("pressure", murmel_mesh::Component::Magnitude);
    let b = s.mesh.as_ref().unwrap().mesh.bounds();
    let mut slice = murmel_engine::Slice::new(b.center(), [0.0, 0.0, 1.0]);
    slice.origin[2] += 0.013;
    s.display.slices = vec![slice];
    let mut a = murmel_engine::Arrows::new("velocity");
    a.density = 20;
    s.display.arrows = Some(a);
    s.redraw();
    check_session("field-arrows-slice", &s);
}

/// Arrows all the same length (the maintainer's card of 2026-11-24),
/// coloured by their own magnitude, at the default density of 40 across
/// the slice: the industrial look, where the colour alone carries the
/// speed. Lengths that follow the magnitude or a grid that thins out
/// move pixels here.
#[test]
fn field_arrows_equal_length() {
    let mut s = field_session("pressure", murmel_mesh::Component::Magnitude);
    let b = s.mesh.as_ref().unwrap().mesh.bounds();
    let mut slice = murmel_engine::Slice::new(b.center(), [0.0, 0.0, 1.0]);
    slice.origin[2] += 0.013;
    s.display.slices = vec![slice];
    let mut a = murmel_engine::Arrows::new("velocity");
    a.length = murmel_engine::glyph::Length::Equal;
    s.display.arrows = Some(a);
    s.redraw();
    check_session("field-arrows-equal", &s);
}

/// Stage 3 vector arrows on the surface (on request), coloured by the
/// field on the colour bar: one arrow per cube of a 12-across grid on
/// the duct's faces, over the speed-coloured mesh. Thinning that keeps
/// every face, arrows on hidden sides, or a lost field colour move
/// pixels here; the slice picture has neither.
#[test]
fn field_arrows_on_the_surface() {
    let mut s = field_session("speed", murmel_mesh::Component::Magnitude);
    let mut a = murmel_engine::Arrows::new("velocity");
    a.density = 12;
    a.surface = true;
    a.length = murmel_engine::glyph::Length::Magnitude;
    a.color = murmel_engine::glyph::ArrowColor::Field;
    s.display.arrows = Some(a);
    s.redraw();
    check_session("field-arrows-surface", &s);
}

/// Stage 4 streamlines (decided 2026-11-24: thin lines by default,
/// traced both ways, coloured by their own field with their own colour
/// bar): 24 seeds on the default line across the duct near its inlet,
/// coloured by |velocity| over the mesh drawn as its faint outline, with
/// the exported picture's legend and the streamlines' bar beside it. A
/// trace that stops early, leaves the duct, loses its colours or its
/// depth test, or a bar laid over the legend moves pixels here.
#[test]
fn field_streamlines_lines() {
    let mut s = field_session("pressure", murmel_mesh::Component::Magnitude);
    s.display.color_by = ColorBy::Solid;
    let mesh = s.mesh.as_ref().unwrap().mesh.clone();
    let mut st = murmel_engine::Stream::new(
        "velocity",
        murmel_engine::Seed::default_for("line", &mesh).unwrap(),
    );
    st.count = 24;
    s.display.streams = vec![st];
    s.redraw();
    let mut shot = render_session(&s, 480, 320).unwrap();
    let legend = murmel_engine::Legend::of(&s);
    let streams: Vec<_> = murmel_engine::Legend::own_bars(&s)
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    assert_eq!(streams.len(), 1, "one bar of its own");
    let i18n = murmel_engine::i18n::I18n::english().unwrap();
    overlay::draw(
        &mut shot.rgba,
        480,
        320,
        &Overlays {
            legend: legend.as_ref(),
            own_bars: &streams,
            triad: None,
            caption: None,
            i18n: &i18n,
            tokens: &s.tokens,
            dark: true,
            scale: 1.0,
        },
    )
    .unwrap();
    check_shot("field-streamlines-lines", &shot);
}

/// Stage 4 streamlines as shaded tubes with arrowheads (on request), in
/// their solid colour token, from a ball of 12 seeds in the middle of the
/// duct, inside the mesh's faint outline: tubes that twist (frames by
/// double reflection), turn inside out, lose their heads or take the
/// field's colours instead of the token move pixels here.
#[test]
fn field_streamlines_tubes() {
    let mut s = field_session("pressure", murmel_mesh::Component::Magnitude);
    let mesh = s.mesh.as_ref().unwrap().mesh.clone();
    let mut st = murmel_engine::Stream::new(
        "velocity",
        murmel_engine::Seed::default_for("sphere", &mesh).unwrap(),
    );
    st.count = 12;
    st.look = murmel_engine::Look::Tubes;
    st.radius = Some(0.015 * mesh.bounds().radius());
    st.arrowheads = true;
    st.color = None;
    s.display.streams = vec![st];
    s.redraw();
    check_session("field-streamlines-tubes", &s);
}

/// Spray parcels (D268): 300 parcels of a jet that enters the duct from
/// its upper wall and bends downstream, as shaded spheres sized by their
/// diameter and coloured by it on inferno with a colour bar of their
/// own, inside the mesh's faint outline. A sphere turned inside out, a
/// wrong size or colour, a parcel outside the duct or a lost bar moves
/// pixels here.
#[test]
fn spray_parcels_by_diameter() {
    let mut s = field_session("pressure", murmel_mesh::Component::Magnitude);
    s.display.color_by = ColorBy::Solid;
    let b = s.mesh.as_ref().unwrap().mesh.bounds();
    let at = |t: [f64; 3]| [0, 1, 2].map(|k| b.min[k] + t[k] * (b.max[k] - b.min[k]));
    // A jet from (0.1, 0.95) bending along x, spread by a fixed pattern
    // (the golden ratio's fractions), diameters 20 to 200 µm.
    let n = 300;
    let mut points = Vec::with_capacity(n);
    let mut diameter = Vec::with_capacity(n);
    for i in 0..n {
        let u = i as f64 / n as f64;
        let f = |k: f64| (k * 0.618_033_988_749_895 * (i + 1) as f64).fract();
        let x = 0.08 + 0.85 * u;
        let y = 0.95 - 0.75 * (1.0 - (-4.0 * u).exp()) + 0.12 * (f(1.0) - 0.5);
        let z = 0.5 + 0.6 * (f(2.0) - 0.5);
        points.push(at([x, y, z]));
        diameter.push(20e-6 + 180e-6 * f(3.0));
    }
    let p = murmel_mesh::Parcels {
        points,
        fields: vec![murmel_mesh::Field {
            name: "diameter".into(),
            unit: Some("m".into()),
            location: murmel_mesh::Location::Point,
            components: 1,
            values: diameter,
        }],
        ..Default::default()
    };
    let mut v = murmel_engine::parcel::ParcelView::new("parcels.vtkhdf".into());
    v.color = murmel_engine::parcel::ParcelView::default_color(&p);
    v.size = murmel_engine::parcel::Size::Diameter;
    v.scale = 3.0;
    v.map = Some("inferno".into());
    s.display.parcels = Some(v);
    s.display.parcel_data = Some(std::sync::Arc::new(p));
    s.redraw();
    assert_eq!(s.mesh.as_ref().unwrap().surface.parcels, n);
    let mut shot = render_session(&s, 480, 320).unwrap();
    let bars: Vec<_> = murmel_engine::Legend::own_bars(&s)
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    assert_eq!(bars.len(), 1, "one bar of their own");
    let i18n = murmel_engine::i18n::I18n::english().unwrap();
    overlay::draw(
        &mut shot.rgba,
        480,
        320,
        &Overlays {
            legend: None,
            own_bars: &bars,
            triad: None,
            caption: None,
            i18n: &i18n,
            tokens: &s.tokens,
            dark: true,
            scale: 1.0,
        },
    )
    .unwrap();
    check_shot("spray-parcels-by-diameter", &shot);
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
            own_bars: &[],
            triad: Some(&s.camera),
            caption: None,
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

/// The clip preview (the window's clip drag) cuts on the GPU: a flat
/// plane moved from near the far end to the middle, before its exact cut
/// is built, looks from the front (the plane seen edge-on) like the exact
/// cut at the middle, and unlike the cut it left. Breaks when the shader
/// ignores the preview planes, puts them in the wrong place or cuts the
/// wrong side away.
#[test]
fn clip_preview_cuts_on_the_gpu() {
    let at = |s: &Session, f: f64| {
        let b = s.mesh.as_ref().unwrap().mesh.bounds();
        let c = b.center();
        Clip {
            origin: [b.min[0] + f * (b.max[0] - b.min[0]), c[1], c[2]],
            normal: [1.0, 0.0, 0.0],
            mode: ClipMode::Flat,
            enabled: true,
        }
    };
    let mut exact = session(StandardView::FRONT);
    exact.display.clips = vec![at(&exact, 0.5)];
    exact.redraw();
    let mut moving = session(StandardView::FRONT);
    moving.display.clips = vec![at(&moving, 0.9)];
    moving.redraw();
    moving.defer_clips = true;
    moving.preview_grids_wait = true;
    moving.display.clips = vec![at(&moving, 0.5)];
    moving.redraw();
    assert!(moving.preview.is_some());
    let want = render_session(&exact, 480, 320).unwrap();
    let preview = render_session(&moving, 480, 320).unwrap();
    let share = diff_share(&preview.rgba, &want.rgba, 24);
    assert!(share < 0.01, "{:.2} % of pixels differ", 100.0 * share);
    moving.preview = None;
    let left = render_session(&moving, 480, 320).unwrap();
    assert!(diff_share(&left.rgba, &want.rgba, 24) > 0.05);
}

/// The live cut of a clip drag (the maintainer's card of 2026-11-25): a
/// crinkle plane moved from near the far end to the middle, drawn as the
/// surface cut on the GPU with the thin layer of the cells it crosses
/// over it, looks from an oblique view like the exact crinkle cut, cell
/// for cell; without the layer (the cut seen hollow) it does not. Breaks
/// when the layer is not drawn, the planes cut it, or it sits elsewhere.
#[test]
fn clip_drag_draws_the_exact_cut_live() {
    let at = |s: &Session, f: f64| {
        let b = s.mesh.as_ref().unwrap().mesh.bounds();
        let c = b.center();
        let n = [0.8f64, 0.0, 0.6];
        Clip {
            origin: [b.min[0] + f * (b.max[0] - b.min[0]), c[1], c[2]],
            normal: n,
            mode: ClipMode::Crinkle,
            enabled: true,
        }
    };
    let mut exact = session(StandardView::ISO);
    exact.display.clips = vec![at(&exact, 0.5)];
    exact.redraw();
    let drag = || {
        let mut s = session(StandardView::ISO);
        s.display.clips = vec![at(&s, 0.9)];
        s.redraw();
        s.defer_clips = true;
        s.display.clips = vec![at(&s, 0.5)];
        s.redraw();
        s
    };
    let want = render_session(&exact, 480, 320).unwrap();
    // Before the first layer: the cut seen hollow.
    let hollow = drag();
    assert!(hollow.drawn().unwrap().layer.is_none());
    let shot = render_session(&hollow, 480, 320).unwrap();
    assert!(diff_share(&shot.rgba, &want.rgba, 24) > 0.02);
    // The layer comes long before the exact cut (built once the plane
    // rests for 120 ms).
    let mut moving = drag();
    let start = std::time::Instant::now();
    while moving.drawn().unwrap().layer.is_none() {
        assert!(start.elapsed().as_secs() < 60, "no layer");
        moving.poll_surface();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(moving.preview.is_some());
    let live = render_session(&moving, 480, 320).unwrap();
    let share = diff_share(&live.rgba, &want.rgba, 24);
    assert!(share < 0.01, "{:.2} % of pixels differ", 100.0 * share);
}

/// The slice preview (the window's slice drag) draws the moving slice
/// from the field's grid on the GPU: a slice through the channel moved
/// from near one end to the middle, before its exact faces are built,
/// looks like the exact slice at the middle (the speed, wall to core) in
/// flat and in smooth colours, and unlike the surface with the slice where it was. Breaks
/// when the shader reads the wrong grid cell or value, leaves the old
/// slice in, or draws the plane outside the mesh.
#[test]
fn slice_preview_draws_from_the_grid() {
    for mode in [
        murmel_engine::FieldMode::Flat,
        murmel_engine::FieldMode::Smooth,
    ] {
        let make = |f: f64| {
            let mut s = field_session("velocity", murmel_mesh::Component::Magnitude);
            s.display.render_mode = RenderMode::Shaded;
            s.display.field_styles.insert(
                "velocity".into(),
                murmel_engine::FieldStyle {
                    mode,
                    ..Default::default()
                },
            );
            let b = s.mesh.as_ref().unwrap().mesh.bounds();
            let c = b.center();
            let at = [b.min[0] + f * (b.max[0] - b.min[0]), c[1], c[2]];
            s.display
                .slices
                .push(murmel_engine::Slice::new(at, [1.0, 0.0, 0.0]));
            s.redraw();
            s
        };
        let exact = make(0.5);
        let mut moving = make(0.9);
        moving.defer_clips = true;
        moving.preview_grids_wait = true;
        let b = moving.mesh.as_ref().unwrap().mesh.bounds();
        moving.display.slices[0].origin[0] = b.min[0] + 0.5 * (b.max[0] - b.min[0]);
        moving.redraw();
        assert!(moving.extras_preview.is_some());
        let want = render_session(&exact, 480, 320).unwrap();
        let preview = render_session(&moving, 480, 320).unwrap();
        let out = root().join("target/render-test");
        std::fs::create_dir_all(&out).unwrap();
        let name = format!("slice-preview-{mode:?}").to_lowercase();
        write_png(&out.join(format!("{name}.png")), 480, 320, &preview.rgba).unwrap();
        write_png(&out.join(format!("{name}-exact.png")), 480, 320, &want.rgba).unwrap();
        let share = diff_share(&preview.rgba, &want.rgba, 24);
        // Smooth colours mix the cells' and nodes' values otherwise than
        // the exact slice's tetrahedra do.
        assert!(
            share < 0.04,
            "{mode:?}: {:.2} % of pixels differ",
            100.0 * share
        );
        moving.extras_preview = None;
        let left = render_session(&moving, 480, 320).unwrap();
        assert!(diff_share(&left.rgba, &want.rgba, 24) > 0.02);
    }
}

/// The iso preview (a new iso value in the window) ray casts the
/// iso-surface through its field's grid on the GPU: the speed's
/// iso-surface moved from one value to another, before its exact faces
/// are built, looks like the exact surface at the new value, solid and
/// coloured by the pressure shown, and unlike the surface at the old
/// value. Breaks when the rays start or stop in the wrong place, the
/// level is on another scale than the grid, the depth is wrong (the mesh
/// would hide it or it would hide the mesh) or the old surface stays.
#[test]
fn iso_preview_ray_casts_the_grid() {
    for color in [
        murmel_engine::IsoColor::Solid,
        murmel_engine::IsoColor::Field,
    ] {
        let make = |f: f64| {
            let mut s = field_session("pressure", murmel_mesh::Component::Magnitude);
            s.display.render_mode = RenderMode::Shaded;
            let speed = murmel_engine::FieldChoice {
                name: "velocity".into(),
                component: murmel_mesh::Component::Magnitude,
            };
            let (lo, hi) = s
                .fields()
                .unwrap()
                .values_cached(&s.mesh.as_ref().unwrap().mesh, &speed)
                .unwrap()
                .range
                .unwrap();
            s.display.isos.push(murmel_engine::Iso {
                field: speed,
                value: lo + f * (hi - lo),
                enabled: true,
                color,
            });
            s.redraw();
            (s, lo, hi)
        };
        let (exact, lo, hi) = make(0.5);
        let (mut moving, _, _) = make(0.8);
        moving.defer_clips = true;
        moving.preview_grids_wait = true;
        moving.display.isos[0].value = lo + 0.5 * (hi - lo);
        moving.redraw();
        assert!(moving.iso_drawn().is_some());
        let want = render_session(&exact, 480, 320).unwrap();
        let preview = render_session(&moving, 480, 320).unwrap();
        let out = root().join("target/render-test");
        std::fs::create_dir_all(&out).unwrap();
        let name = format!("iso-preview-{color:?}").to_lowercase();
        write_png(&out.join(format!("{name}.png")), 480, 320, &preview.rgba).unwrap();
        write_png(&out.join(format!("{name}-exact.png")), 480, 320, &want.rgba).unwrap();
        moving.extras_preview = None;
        let left = render_session(&moving, 480, 320).unwrap();
        // On this coarse mesh the grid's values between cells and nodes
        // differ from the exact surface's tetrahedra: the preview is
        // nearer the new surface than the old surface is, by far.
        let share = diff_share(&preview.rgba, &want.rgba, 24);
        let old = diff_share(&left.rgba, &want.rgba, 24);
        assert!(
            share < 0.1 && share < 0.4 * old,
            "{color:?}: {:.2} % of pixels differ ({:.2} % for the old value)",
            100.0 * share,
            100.0 * old
        );
    }
}
