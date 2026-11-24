//! The tools. Inputs are typed structs (unknown fields refused);
//! each tool has a JSON Schema of its input for the catalogue.

use std::path::PathBuf;

use murmel_engine::display::{SavedView, VIEW_NAME_MAX, all_items, view_name};
use murmel_engine::field::{
    self, Component, FieldChoice, FieldMode, FieldSet, FieldStyle, RangeMode,
};
use murmel_engine::measure::{self, SNAP_RADIUS_PX};
use murmel_engine::navigation::{Binding, Navigation, PRESETS};
use murmel_engine::session_file::{SessionFile, absolute};
use murmel_engine::settings::{Agents, Settings, save_navigation, save_settings};
use murmel_engine::{
    Anchor, Clip, ClipMode, ColorBy, CubeSettings, Item, MeasureKind, Measurement, MeshColors,
    Metric, PickTarget, Placement, Probe, Projection, Quality, RenderMode, Rgb, ScreenAxis,
    Session, SnapKind, StandardView, Theme, Visibility,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{Action, Diagnostic, Run, Tool};

pub(crate) fn parse<T: DeserializeOwned>(input: Value) -> Result<T, Diagnostic> {
    serde_json::from_value(input).map_err(|e| Diagnostic::new("TOOL002", e.to_string()))
}

fn mesh_err(e: murmel_mesh::MeshError) -> Diagnostic {
    Diagnostic::new(e.code, e.message)
}

pub(crate) fn object(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

fn no_input() -> Value {
    object(json!({}), &[])
}

pub(crate) fn camera_json(s: &Session) -> Value {
    let c = &s.camera;
    json!({
        "target": c.target,
        "distance": c.distance,
        "yaw_deg": c.yaw_deg,
        "pitch_deg": c.pitch_deg,
        "roll_deg": c.roll_deg,
        "view": c.standard().map(|v| v.name()),
        "projection": c.projection,
        "orthographic": c.is_orthographic(),
        "fov_deg": c.fov_deg,
        "eye": c.eye().to_array(),
        "viewport": s.viewport,
    })
}

pub(crate) fn need_mesh(s: &Session) -> Result<(), Diagnostic> {
    if s.mesh.is_none() {
        return Err(Diagnostic::new("VIEW001", "no mesh is open"));
    }
    Ok(())
}

// ---- system -------------------------------------------------------------

fn system_info(_: Value, _: &mut Session) -> Run {
    Ok(json!({
        "format": "murmelcraft.info",
        "format_version": crate::FORMAT_VERSION,
        "version": env!("CARGO_PKG_VERSION"),
        "platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        "debug_assertions": cfg!(debug_assertions),
    }))
}

fn diag_codes(_: Value, _: &mut Session) -> Run {
    Ok(json!({
        "format": "murmelcraft.codes",
        "format_version": crate::FORMAT_VERSION,
        "codes": crate::diag::codes(),
    }))
}

// ---- mesh ---------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenInput {
    path: PathBuf,
}

fn mesh_summary(s: &Session) -> Value {
    let Some(m) = &s.mesh else {
        return Value::Null;
    };
    let mut warnings = Vec::new();
    if let Some(e) = &m.evidence_problem {
        warnings.push(json!(Diagnostic::new(e.code, e.message.clone())));
    }
    json!({
        "path": m.path,
        "format": if m.mesh.metadata.contains_key("openfoam_format") {
            "openfoam"
        } else if m.mesh.metadata.contains_key("vtkhdf") {
            "vtkhdf"
        } else {
            "vtu"
        },
        "fields": m.fields.names(),
        "files": m.files.iter().map(|(n, d)| json!({"name": n, "sha256": d})).collect::<Vec<_>>(),
        "summary": m.mesh.summary(),
        "metadata": m.mesh.metadata,
        "evidence": m.evidence,
        "validity": m.validity.key(),
        "digest": m.digest,
        "items": std::iter::once(Item::Mesh).chain(all_items(&m.mesh)).map(|i| i.id(&m.mesh)).collect::<Vec<_>>(),
        "surface": {"triangles": m.surface.triangles(), "edges": m.surface.edges()},
        "warnings": warnings,
    })
}

fn mesh_open(input: Value, s: &mut Session) -> Run {
    let i: OpenInput = parse(input)?;
    s.open(&i.path).map_err(mesh_err)?;
    Ok(mesh_summary(s))
}

fn mesh_close(_: Value, s: &mut Session) -> Run {
    let was = s.mesh.as_ref().map(|m| m.path.clone());
    s.close();
    Ok(json!({"closed": was}))
}

fn mesh_info(_: Value, s: &mut Session) -> Run {
    need_mesh(s)?;
    Ok(mesh_summary(s))
}

// ---- view ---------------------------------------------------------------

fn view_get(_: Value, s: &mut Session) -> Run {
    Ok(camera_json(s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetCamera {
    target: Option<[f64; 3]>,
    distance: Option<f64>,
    yaw_deg: Option<f64>,
    pitch_deg: Option<f64>,
    roll_deg: Option<f64>,
    projection: Option<Projection>,
    fov_deg: Option<f64>,
    viewport: Option<[f64; 2]>,
}

pub(crate) fn finite(v: f64, name: &str) -> Result<f64, Diagnostic> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(Diagnostic::new("TOOL002", format!("{name} must be finite")))
    }
}

fn view_set(input: Value, s: &mut Session) -> Run {
    let i: SetCamera = parse(input)?;
    let c = &mut s.camera;
    if let Some(t) = i.target {
        for v in t {
            finite(v, "target")?;
        }
        c.target = t;
    }
    if let Some(d) = i.distance {
        if finite(d, "distance")? <= 0.0 {
            return Err(Diagnostic::new("TOOL002", "distance must be positive"));
        }
        c.distance = d;
    }
    if let Some(y) = i.yaw_deg {
        c.yaw_deg = finite(y, "yaw_deg")?;
        c.snapped = false;
    }
    if let Some(p) = i.pitch_deg {
        c.pitch_deg = finite(p, "pitch_deg")?.clamp(-90.0, 90.0);
        c.snapped = false;
    }
    if let Some(r) = i.roll_deg {
        c.roll_deg = finite(r, "roll_deg")?.rem_euclid(360.0);
        if c.roll_deg > 180.0 {
            c.roll_deg -= 360.0;
        }
    }
    if let Some(p) = i.projection {
        c.projection = p;
    }
    if let Some(f) = i.fov_deg {
        c.fov_deg = finite(f, "fov_deg")?.clamp(1.0, 120.0);
    }
    if let Some([w, h]) = i.viewport {
        if finite(w, "viewport")? < 1.0 || finite(h, "viewport")? < 1.0 {
            return Err(Diagnostic::new(
                "TOOL002",
                "viewport must be at least 1 x 1",
            ));
        }
        s.viewport = [w, h];
    }
    Ok(camera_json(s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Orbit {
    #[serde(default)]
    yaw_deg: f64,
    #[serde(default)]
    pitch_deg: f64,
    /// Turn about this world point instead of the target.
    #[serde(default)]
    pivot: Option<[f64; 3]>,
}

fn view_orbit(input: Value, s: &mut Session) -> Run {
    let i: Orbit = parse(input)?;
    let (dyaw, dpitch) = (
        finite(i.yaw_deg, "yaw_deg")?,
        finite(i.pitch_deg, "pitch_deg")?,
    );
    match i.pivot {
        Some(p) => {
            for v in p {
                finite(v, "pivot")?;
            }
            s.camera.orbit_about(dyaw, dpitch, p);
        }
        None => s.camera.orbit(dyaw, dpitch),
    }
    Ok(camera_json(s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pan {
    #[serde(default)]
    dx_px: f64,
    #[serde(default)]
    dy_px: f64,
}

fn view_pan(input: Value, s: &mut Session) -> Run {
    let i: Pan = parse(input)?;
    let h = s.viewport[1];
    s.camera
        .pan(finite(i.dx_px, "dx_px")?, finite(i.dy_px, "dy_px")?, h);
    Ok(camera_json(s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Zoom {
    factor: f64,
    cursor_ndc: Option<[f64; 2]>,
}

fn view_zoom(input: Value, s: &mut Session) -> Run {
    let i: Zoom = parse(input)?;
    if finite(i.factor, "factor")? <= 0.0 {
        return Err(Diagnostic::new("TOOL002", "factor must be positive"));
    }
    let aspect = s.aspect();
    s.camera.zoom(i.factor, i.cursor_ndc, aspect);
    Ok(camera_json(s))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Fit {
    item: Option<String>,
}

fn view_fit(input: Value, s: &mut Session) -> Run {
    let i: Fit = parse(input)?;
    need_mesh(s)?;
    match i.item {
        Some(id) => {
            let item = item(s, &id)?;
            if !s.fit_item(item) {
                s.fit();
            }
        }
        None => {
            s.fit();
        }
    }
    Ok(camera_json(s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Standard {
    view: StandardView,
}

fn view_standard(input: Value, s: &mut Session) -> Run {
    let i: Standard = parse(input)?;
    s.camera.set_standard(i.view);
    if i.view == StandardView::Home || s.mesh.is_some() {
        s.fit();
    }
    Ok(camera_json(s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rotate {
    axis: ScreenAxis,
    deg: f64,
}

fn view_rotate(input: Value, s: &mut Session) -> Run {
    let i: Rotate = parse(input)?;
    s.camera.rotate(i.axis, finite(i.deg, "deg")?);
    Ok(camera_json(s))
}

// ---- saved views --------------------------------------------------------

fn views_json(s: &Session) -> Value {
    json!(
        s.display
            .views
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>()
    )
}

fn view_index(s: &Session, name: &str) -> Result<usize, Diagnostic> {
    s.display
        .views
        .iter()
        .position(|v| v.name == name.trim())
        .ok_or_else(|| Diagnostic::new("VIEW003", format!("no saved view is called {name:?}")))
}

fn new_view_name(name: &str) -> Result<String, Diagnostic> {
    view_name(name).ok_or_else(|| {
        Diagnostic::new(
            "VIEW004",
            format!("{name:?} is empty, longer than {VIEW_NAME_MAX} characters or holds control characters"),
        )
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewSave {
    /// Left out: the first free "View N".
    name: Option<String>,
}

fn views_save(input: Value, s: &mut Session) -> Run {
    let i: ViewSave = parse(input)?;
    need_mesh(s)?;
    let name = match i.name {
        Some(n) => new_view_name(&n)?,
        None => s.display.next_view_name("View"),
    };
    let camera = s.camera.clone();
    let updated = match s.display.views.iter_mut().find(|v| v.name == name) {
        Some(v) => {
            v.camera = camera;
            true
        }
        None => {
            s.display.views.push(SavedView {
                name: name.clone(),
                camera,
            });
            false
        }
    };
    Ok(json!({"name": name, "updated": updated, "views": views_json(s)}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewName {
    name: String,
}

fn views_go(input: Value, s: &mut Session) -> Run {
    let i: ViewName = parse(input)?;
    need_mesh(s)?;
    let k = view_index(s, &i.name)?;
    s.camera = s.display.views[k].camera.clone();
    Ok(json!({"name": s.display.views[k].name, "camera": camera_json(s)}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewRename {
    name: String,
    to: String,
}

fn views_rename(input: Value, s: &mut Session) -> Run {
    let i: ViewRename = parse(input)?;
    need_mesh(s)?;
    let k = view_index(s, &i.name)?;
    let to = new_view_name(&i.to)?;
    if s.display
        .views
        .iter()
        .enumerate()
        .any(|(j, v)| j != k && v.name == to)
    {
        return Err(Diagnostic::new(
            "VIEW004",
            format!("another saved view is called {to:?}"),
        ));
    }
    s.display.views[k].name = to.clone();
    Ok(json!({"name": to, "views": views_json(s)}))
}

fn views_delete(input: Value, s: &mut Session) -> Run {
    let i: ViewName = parse(input)?;
    need_mesh(s)?;
    let k = view_index(s, &i.name)?;
    let gone = s.display.views.remove(k);
    Ok(json!({"deleted": gone.name, "views": views_json(s)}))
}

fn views_list(_: Value, s: &mut Session) -> Run {
    let views: Vec<Value> = s
        .display
        .views
        .iter()
        .map(|v| {
            let c = &v.camera;
            json!({
                "name": v.name,
                "target": c.target,
                "distance": c.distance,
                "yaw_deg": c.yaw_deg,
                "pitch_deg": c.pitch_deg,
                "roll_deg": c.roll_deg,
                "projection": c.projection,
                "view": c.standard().map(|v| v.name()),
            })
        })
        .collect();
    Ok(json!({"views": views}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Screenshot {
    path: PathBuf,
    #[serde(default = "default_w")]
    width: u32,
    #[serde(default = "default_h")]
    height: u32,
    /// Open this mesh first (one-shot use from the command line).
    mesh: Option<PathBuf>,
    /// Go to this standard view (and fit) first.
    view: Option<StandardView>,
    theme: Option<Theme>,
}

fn default_w() -> u32 {
    1280
}
fn default_h() -> u32 {
    800
}

fn view_screenshot(input: Value, s: &mut Session) -> Run {
    let i: Screenshot = parse(input)?;
    if let Some(m) = &i.mesh {
        s.open(m).map_err(mesh_err)?;
    }
    need_mesh(s)?;
    s.viewport = [i.width as f64, i.height as f64];
    if let Some(v) = i.view {
        s.camera.set_standard(v);
        s.fit();
    } else if i.mesh.is_some() {
        s.fit();
    }
    let keep = s.theme;
    if let Some(t) = i.theme {
        s.theme = t;
    }
    let shot = murmel_render::render_session(s, i.width, i.height);
    s.theme = keep;
    let shot = shot.map_err(|e| Diagnostic::new(e.code, e.message))?;
    murmel_render::write_png(&i.path, shot.width, shot.height, &shot.rgba)
        .map_err(|e| Diagnostic::new(e.code, e.message))?;
    Ok(json!({
        "files": [i.path],
        "width": shot.width,
        "height": shot.height,
        "adapter": shot.adapter,
        "camera": camera_json(s),
    }))
}

// ---- display, selection and properties ----------------------------------

fn item(s: &Session, id: &str) -> Result<Item, Diagnostic> {
    let m = s
        .mesh
        .as_ref()
        .ok_or_else(|| Diagnostic::new("VIEW001", "no mesh is open"))?;
    Item::parse(id, &m.mesh)
        .filter(|&i| s.display.has(i))
        .ok_or_else(|| Diagnostic::new("VIEW002", format!("no item {id} in the open mesh")))
}

pub(crate) fn display_json(s: &Session) -> Value {
    let d = &s.display;
    let Some(m) = &s.mesh else {
        return json!({"color_by": d.color_by, "render_mode": d.render_mode});
    };
    let colors: serde_json::Map<String, Value> = d
        .colors
        .iter()
        .map(|(i, c)| (i.id(&m.mesh), json!(c.hex())))
        .collect();
    let (cells, faces) = d.visible_counts(&m.mesh);
    json!({
        "color_by": d.color_by,
        "render_mode": d.render_mode,
        "clip": clip_json(s),
        "clips": (0..d.clips.len()).map(|k| plane_json(s, k)).collect::<Vec<_>>(),
        "hidden": d.hidden.iter().map(|i| i.id(&m.mesh)).collect::<Vec<_>>(),
        "selection": d.selection.iter().map(|i| i.id(&m.mesh)).collect::<Vec<_>>(),
        "isolated": d.isolated.iter().map(|i| i.id(&m.mesh)).collect::<Vec<_>>(),
        "pick": d.pick,
        "visible_cells": cells,
        "visible_faces": faces,
        "colors": colors,
        "quality": quality_json(s),
        "field": field_shown_json(s),
        "slices": (0..d.slices.len()).map(|k| crate::extract::slice_json(s, k)).collect::<Vec<_>>(),
        "isos": (0..d.isos.len()).map(|k| crate::extract::iso_json(s, k)).collect::<Vec<_>>(),
        "mesh_look": d.mesh_look,
        "arrows": crate::extract::arrows_json(s),
        "regions": crate::region::regions_json(s),
        "streams": (0..d.streams.len()).map(|k| crate::stream::stream_json(s, k)).collect::<Vec<_>>(),
    })
}

/// The metric, its range over the cells (in colour mode quality) and the
/// threshold.
fn quality_json(s: &Session) -> Value {
    let d = &s.display;
    json!({
        "metric": d.metric,
        "range": d.quality_range().map(|(a, b)| [a, b]),
        "threshold": d.threshold,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ColorByInput {
    by: ColorBy,
    /// The metric of colour mode quality; left out keeps the current one.
    #[serde(default)]
    metric: Option<Metric>,
    /// The field of colour mode field; left out keeps the current one
    /// (the first field of the file when none was chosen).
    #[serde(default)]
    field: Option<String>,
    /// The scalar of a vector field; left out keeps the current one.
    #[serde(default)]
    component: Option<Component>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderModeInput {
    mode: RenderMode,
}

fn display_render_mode(input: Value, s: &mut Session) -> Run {
    let i: RenderModeInput = parse(input)?;
    s.display.render_mode = i.mode;
    s.redraw();
    Ok(display_json(s))
}

fn display_color_by(input: Value, s: &mut Session) -> Run {
    let i: ColorByInput = parse(input)?;
    if i.by == ColorBy::Field {
        let choice = field_choice(s, i.field.as_deref(), i.component)?;
        crate::defaults::adopt_style(s, &choice.name);
        s.display.field = Some(choice);
    } else if i.field.is_some() || i.component.is_some() {
        return Err(Diagnostic::new(
            "TOOL002",
            "field and component go with by: field",
        ));
    }
    s.display.color_by = i.by;
    if let Some(m) = i.metric {
        if m != s.display.metric {
            // A threshold is a value of one metric; it means nothing in
            // another.
            s.display.threshold = None;
        }
        s.display.metric = m;
    }
    s.redraw();
    Ok(display_json(s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThresholdInput {
    /// Hide cells whose metric is below this value; null shows them again.
    value: Option<f64>,
    #[serde(default)]
    metric: Option<Metric>,
}

fn display_threshold(input: Value, s: &mut Session) -> Run {
    let i: ThresholdInput = parse(input)?;
    need_mesh(s)?;
    if let Some(v) = i.value
        && !v.is_finite()
    {
        return Err(Diagnostic::new("TOOL002", "value must be a finite number"));
    }
    s.display.color_by = ColorBy::Quality;
    if let Some(m) = i.metric {
        s.display.metric = m;
    }
    s.display.threshold = i.value;
    s.redraw();
    Ok(display_json(s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QualityInput {
    #[serde(default)]
    metric: Option<Metric>,
    #[serde(default = "default_bins")]
    bins: usize,
    /// Also count the cells at or above this value.
    #[serde(default)]
    above: Option<f64>,
}

fn default_bins() -> usize {
    20
}

fn inspect_quality(input: Value, s: &mut Session) -> Run {
    let i: QualityInput = parse(input)?;
    if !(1..=200).contains(&i.bins) {
        return Err(Diagnostic::new("TOOL002", "bins must be between 1 and 200"));
    }
    let Some(m) = &s.mesh else {
        return Err(Diagnostic::new("VIEW001", "no mesh is open"));
    };
    if s.display.quality.is_none() {
        s.display.quality = Some(std::sync::Arc::new(Quality::compute(&m.mesh)));
    }
    let Some(q) = &s.display.quality else {
        return Err(Diagnostic::new("VIEW001", "no mesh is open"));
    };
    let metric = i.metric.unwrap_or(s.display.metric);
    let stats = q.stats(metric, i.bins);
    let above = i
        .above
        .map(|a| q.values(metric).iter().filter(|&&x| x as f64 >= a).count());
    Ok(json!({
        "metric": metric,
        "unit": metric.unit(),
        "cells": stats.cells,
        "min": stats.min,
        "max": stats.max,
        "mean": stats.mean,
        "histogram": stats.histogram,
        "above": i.above.map(|a| json!({"value": a, "cells": above})),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetColorInput {
    item: String,
    /// `#RRGGBB`; null (or left out) goes back to the colour mode's own.
    #[serde(default)]
    color: Option<Rgb>,
}

fn display_set_color(input: Value, s: &mut Session) -> Run {
    let i: SetColorInput = parse(input)?;
    let it = item(s, &i.item)?;
    if matches!(it, Item::Clip(_)) {
        return Err(Diagnostic::new(
            "VIEW002",
            "a clip plane has no colour; it cuts the mesh",
        ));
    }
    if matches!(it, Item::Slice(_)) {
        return Err(Diagnostic::new(
            "VIEW002",
            "a slice takes the colours of the cells it cuts",
        ));
    }
    if matches!(it, Item::Measure(_) | Item::Probe(_)) {
        return Err(Diagnostic::new(
            "VIEW002",
            "measurements and probes take the measure colour of the theme",
        ));
    }
    match i.color {
        Some(c) => s.display.colors.insert(it, c),
        None => s.display.colors.remove(&it),
    };
    s.redraw();
    Ok(display_json(s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VisibilityInput {
    action: Visibility,
    #[serde(default)]
    items: Vec<String>,
}

fn display_visibility(input: Value, s: &mut Session) -> Run {
    let i: VisibilityInput = parse(input)?;
    need_mesh(s)?;
    let items = i
        .items
        .iter()
        .map(|id| item(s, id))
        .collect::<Result<Vec<_>, _>>()?;
    if items.is_empty() && i.action != Visibility::ShowAll {
        return Err(Diagnostic::new(
            "TOOL002",
            "items is empty; name at least one item, or use show_all",
        ));
    }
    if let Some(m) = &s.mesh {
        s.display.set_visibility(i.action, &items, &m.mesh);
    }
    s.redraw();
    Ok(display_json(s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectInput {
    item: String,
    /// Add to the selection (Shift); an item already selected leaves it.
    #[serde(default)]
    add: bool,
}

fn select_item(input: Value, s: &mut Session) -> Run {
    let i: SelectInput = parse(input)?;
    let it = item(s, &i.item)?;
    s.display.select(it, i.add);
    s.redraw();
    Ok(display_json(s))
}

fn select_clear(_: Value, s: &mut Session) -> Run {
    s.display.selection.clear();
    s.redraw();
    Ok(display_json(s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AtInput {
    ndc: [f64; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PickInput {
    ndc: [f64; 2],
    #[serde(default)]
    add: bool,
    target: Option<PickTarget>,
}

/// What lies under `ndc`, as JSON: the cell, its type and zone, the face,
/// its patch and role, the point.
fn hit_json(
    s: &Session,
    ndc: [f64; 2],
) -> Result<Option<(murmel_engine::pick::Hit, Value)>, Diagnostic> {
    for v in ndc {
        finite(v, "ndc")?;
    }
    let Some(m) = &s.mesh else {
        return Err(Diagnostic::new("VIEW001", "no mesh is open"));
    };
    let Some(hit) = murmel_engine::pick::pick(&m.surface, &s.camera, ndc, s.aspect()) else {
        return Ok(None);
    };
    let mesh = &m.mesh;
    let none = murmel_mesh::NO_CELL;
    let cell = (hit.cell != none).then_some(hit.cell as usize);
    let face = (hit.face != none).then_some(hit.face as usize);
    let patch = face
        .and_then(|u| mesh.face_boundary.get(u).copied())
        .filter(|&b| b != none)
        .and_then(|b| mesh.boundary_patch.get(b as usize))
        .and_then(|&p| mesh.patches.get(p as usize));
    let v = json!({
        "point": hit.point,
        "cell": cell.map(|c| format!("cell:{c}")),
        "kind": cell.and_then(|c| mesh.cell_kind.get(c)).map(|k| k.name()),
        "zone": cell.and_then(|c| mesh.cell_zone.get(c)).and_then(|&z| mesh.zones.get(z as usize)),
        "face": face.map(|u| format!("face:{u}")),
        "patch": patch.map(|p| p.name.clone()),
        "role": patch.map(|p| p.role.name()),
        "section": face.is_none(),
    });
    Ok(Some((hit, v)))
}

fn inspect_at(input: Value, s: &mut Session) -> Run {
    let i: AtInput = parse(input)?;
    let hit = hit_json(s, i.ndc)?;
    Ok(json!({"hit": hit.map(|h| h.1)}))
}

fn select_pick(input: Value, s: &mut Session) -> Run {
    let i: PickInput = parse(input)?;
    let target = i.target.unwrap_or(s.display.pick);
    let hit = hit_json(s, i.ndc)?;
    let none = murmel_mesh::NO_CELL;
    let picked = hit.as_ref().and_then(|(h, _)| match target {
        PickTarget::Cell if h.cell != none => Some(Item::Cell(h.cell)),
        // A section belongs to its cell: a face pick there takes the cell.
        PickTarget::Face if h.face != none => Some(Item::Face(h.face)),
        _ if h.cell != none => Some(Item::Cell(h.cell)),
        // A surface without cells: the face.
        _ if h.face != none => Some(Item::Face(h.face)),
        _ => None,
    });
    match picked {
        Some(it) => s.display.select(it, i.add),
        // A click on empty space clears, as in CAD tools; Shift keeps.
        None if !i.add => s.display.selection.clear(),
        None => {}
    }
    s.redraw();
    let mut out = display_json(s);
    out["hit"] = hit.map_or(Value::Null, |h| h.1);
    out["picked"] = json!(picked.and_then(|it| s.mesh.as_ref().map(|m| it.id(&m.mesh))));
    Ok(out)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilterInput {
    target: PickTarget,
}

fn select_filter(input: Value, s: &mut Session) -> Run {
    let i: FilterInput = parse(input)?;
    s.display.pick = i.target;
    Ok(display_json(s))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct InspectInput {
    item: Option<String>,
}

fn inspect_item(input: Value, s: &mut Session) -> Run {
    let i: InspectInput = parse(input)?;
    need_mesh(s)?;
    let it = match &i.item {
        Some(id) => item(s, id)?,
        None => s.display.current().unwrap_or(Item::Mesh),
    };
    let Some(m) = &s.mesh else {
        return Err(Diagnostic::new("VIEW001", "no mesh is open"));
    };
    Ok(json!({
        "item": it.id(&m.mesh),
        "rows": murmel_engine::inspect::rows(m, &s.display, it),
        "fields": murmel_engine::inspect::field_values(m, &s.display, it),
    }))
}

// ---- clip plane ---------------------------------------------------------

/// The current clip plane, or null.
fn clip_json(s: &Session) -> Value {
    s.display
        .current_clip()
        .map_or(Value::Null, |k| plane_json(s, k))
}

/// Clip plane `k` (0-based) with its id and the counts of the cut.
fn plane_json(s: &Session, k: usize) -> Value {
    let (Some(c), Some(m)) = (s.display.clips.get(k), &s.mesh) else {
        return Value::Null;
    };
    let (offset, range) = c.offset(&m.mesh.bounds());
    json!({
        "id": Item::Clip(k as u32).id(&m.mesh),
        "plane": k + 1,
        "enabled": c.enabled,
        "mode": c.mode,
        "origin": c.origin,
        "normal": c.normal,
        "offset": offset,
        "range": range,
        "cells_shown": m.surface.cells,
        "open_faces": m.surface.open_faces,
    })
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum ClipAxis {
    X,
    Y,
    Z,
    View,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipSet {
    axis: Option<ClipAxis>,
    normal: Option<[f64; 3]>,
    origin: Option<[f64; 3]>,
    offset: Option<f64>,
    mode: Option<ClipMode>,
    #[serde(default)]
    flip: bool,
    enabled: Option<bool>,
    /// The plane to change, counted from 1; the current one when left out.
    plane: Option<u32>,
    /// Add a plane instead of changing one.
    #[serde(default)]
    new: bool,
}

/// The index of plane `n` (counted from 1), or of the current plane.
pub(crate) fn plane_index(s: &Session, n: Option<u32>) -> Result<Option<usize>, Diagnostic> {
    match n {
        None => Ok(s.display.current_clip()),
        Some(n) if n >= 1 && (n as usize) <= s.display.clips.len() => Ok(Some(n as usize - 1)),
        Some(n) => Err(Diagnostic::new(
            "CLIP001",
            format!(
                "no clip plane {n}; there are {} (clip:1 is the first)",
                s.display.clips.len()
            ),
        )),
    }
}

fn clip_set(input: Value, s: &mut Session) -> Run {
    let i: ClipSet = parse(input)?;
    need_mesh(s)?;
    if i.axis.is_some() && i.normal.is_some() {
        return Err(Diagnostic::new(
            "TOOL002",
            "give either axis or normal, not both",
        ));
    }
    if i.origin.is_some() && i.offset.is_some() {
        return Err(Diagnostic::new(
            "TOOL002",
            "give either origin or offset, not both",
        ));
    }
    let bounds = s
        .mesh
        .as_ref()
        .map(|m| m.mesh.bounds())
        .unwrap_or_else(murmel_mesh::Bounds::empty);
    if i.new && i.plane.is_some() {
        return Err(Diagnostic::new(
            "TOOL002",
            "give either new or plane, not both",
        ));
    }
    if i.new && s.display.clips.len() >= MAX_CLIPS {
        return Err(Diagnostic::new(
            "CLIP002",
            format!("{MAX_CLIPS} clip planes are the most"),
        ));
    }
    let at = if i.new {
        None
    } else {
        plane_index(s, i.plane)?
    };
    let mut c = at.map(|k| s.display.clips[k]).unwrap_or_else(|| {
        crate::defaults::fresh(
            s,
            murmel_engine::defaults::Kind::Clip,
            Clip {
                origin: bounds.center(),
                normal: [1.0, 0.0, 0.0],
                mode: ClipMode::Crinkle,
                enabled: true,
            },
        )
    });
    if let Some(a) = i.axis {
        // The half nearer the eye is cut away, so the cut faces the
        // camera.
        let to_eye = s.camera.to_eye().to_array();
        let toward = |k: usize| {
            let mut n = [0.0; 3];
            n[k] = if to_eye[k] < -1e-9 { -1.0 } else { 1.0 };
            n
        };
        c.normal = match a {
            ClipAxis::X => toward(0),
            ClipAxis::Y => toward(1),
            ClipAxis::Z => toward(2),
            ClipAxis::View => to_eye,
        };
        c.enabled = true;
    }
    if let Some(n) = i.normal {
        for v in n {
            finite(v, "normal")?;
        }
        c.normal = murmel_engine::clip::unit(n)
            .ok_or_else(|| Diagnostic::new("TOOL002", "normal must not be zero"))?;
        c.enabled = true;
    }
    if i.flip {
        c.normal = c.normal.map(|v| -v);
    }
    if let Some(o) = i.origin {
        for v in o {
            finite(v, "origin")?;
        }
        c.origin = o;
    }
    if let Some(off) = i.offset {
        c.set_offset(&bounds, finite(off, "offset")?);
    }
    if let Some(m) = i.mode {
        c.mode = m;
    }
    if let Some(e) = i.enabled {
        c.enabled = e;
    }
    match at {
        Some(k) => s.display.clips[k] = c,
        None => {
            // The new plane becomes the current one: a clip plane
            // selected before would stay current otherwise.
            s.display.selection.retain(|i| !matches!(i, Item::Clip(_)));
            s.display.clips.push(c);
        }
    }
    s.redraw();
    Ok(display_json(s))
}

/// The axis of `clip.rotate`: a screen axis (x right, y up, z towards
/// the viewer, as in `view.rotate`) or a world vector.
#[derive(Deserialize)]
#[serde(untagged)]
enum TiltAxis {
    Screen(ScreenAxis),
    World([f64; 3]),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipRotate {
    axis: TiltAxis,
    deg: f64,
    pivot: Option<[f64; 3]>,
    /// The plane to tilt, counted from 1; the current one when left out.
    plane: Option<u32>,
}

fn clip_rotate(input: Value, s: &mut Session) -> Run {
    let i: ClipRotate = parse(input)?;
    need_mesh(s)?;
    let Some(k) = plane_index(s, i.plane)? else {
        return Err(Diagnostic::new(
            "TOOL002",
            "there is no clip plane to tilt; set one with clip.set first",
        ));
    };
    let mut c = s.display.clips[k];
    let deg = finite(i.deg, "deg")?;
    let axis = match i.axis {
        TiltAxis::Screen(a) => {
            let (right, up) = s.camera.basis();
            match a {
                ScreenAxis::X => right,
                ScreenAxis::Y => up,
                ScreenAxis::Z => s.camera.to_eye(),
            }
            .to_array()
        }
        TiltAxis::World(v) => {
            for x in v {
                finite(x, "axis")?;
            }
            v
        }
    };
    let bounds = s
        .mesh
        .as_ref()
        .map(|m| m.mesh.bounds())
        .unwrap_or_else(murmel_mesh::Bounds::empty);
    let pivot = match i.pivot {
        Some(p) => {
            for x in p {
                finite(x, "pivot")?;
            }
            p
        }
        None => c.pivot(&bounds),
    };
    c.rotate(axis, deg, pivot)
        .ok_or_else(|| Diagnostic::new("TOOL002", "axis must not be zero"))?;
    s.display.clips[k] = c;
    s.redraw();
    Ok(display_json(s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipClear {
    plane: Option<u32>,
    #[serde(default)]
    all: bool,
}

fn clip_clear(input: Value, s: &mut Session) -> Run {
    let i: ClipClear = parse(input)?;
    if i.all {
        if i.plane.is_some() {
            return Err(Diagnostic::new(
                "TOOL002",
                "give either all or plane, not both",
            ));
        }
        s.display.clips.clear();
        s.display.selection.retain(|i| !matches!(i, Item::Clip(_)));
    } else if let Some(k) = plane_index(s, i.plane)? {
        s.display.remove_clip(k);
    }
    s.redraw();
    Ok(display_json(s))
}

/// The most clip planes a display holds (each one walks the mesh).
pub const MAX_CLIPS: usize = 8;

// ---- settings -----------------------------------------------------------

fn config_dir(s: &Session) -> Result<PathBuf, Diagnostic> {
    s.config_dir.clone().ok_or_else(|| {
        Diagnostic::new(
            "SET001",
            "applied for this session, not saved: no config folder (set MURMEL_CONFIG_DIR)",
        )
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetTheme {
    theme: Theme,
}

fn settings_set_theme(input: Value, s: &mut Session) -> Run {
    let i: SetTheme = parse(input)?;
    // Applied live first; see settings.set_bindings.
    s.theme = i.theme;
    let path = save_session_settings(s)?;
    Ok(json!({"theme": i.theme, "files": [path]}))
}

/// Write settings.toml from the session (theme, mesh colours, view cube), so one
/// setting never drops another.
pub(crate) fn save_session_settings(s: &Session) -> Result<PathBuf, Diagnostic> {
    let dir = config_dir(s)?;
    save_settings(
        &dir,
        &Settings {
            theme: s.theme,
            colors: s.colors.clone(),
            view_cube: s.view_cube.clone(),
            agents: Agents { enabled: s.agents },
            defaults: s.defaults.clone(),
            formulas: s.formulas.clone(),
            ..Settings::default()
        },
    )
    .map_err(|e| {
        Diagnostic::new(
            "SET001",
            format!("applied for this session, not saved: {e}"),
        )
    })
}

fn settings_set_colors(mut input: Value, s: &mut Session) -> Run {
    // `reset` (back to the built-in colours first) is taken out; the rest
    // are the colours, unknown fields refused.
    let reset = match input.as_object_mut().and_then(|o| o.remove("reset")) {
        None => false,
        Some(Value::Bool(b)) => b,
        Some(_) => return Err(Diagnostic::new("TOOL002", "reset must be true or false")),
    };
    let new: MeshColors = parse(input)?;
    let mut colors = if reset {
        MeshColors::default()
    } else {
        s.colors.clone()
    };
    colors.merge(new);
    s.set_colors(colors)
        .map_err(|e| Diagnostic::new("TOOL002", e))?;
    let path = save_session_settings(s)?;
    Ok(json!({"colors": s.colors, "files": [path]}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetAgents {
    enabled: bool,
}

fn settings_set_agents(input: Value, s: &mut Session) -> Run {
    let i: SetAgents = parse(input)?;
    // Applied live first: the window opens or closes its socket on the
    // next frame (crate murmel-tools, module live).
    s.agents = i.enabled;
    let path = save_session_settings(s)?;
    Ok(json!({"agents": {"enabled": s.agents}, "files": [path]}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetViewCube {
    placement: Option<Placement>,
    auto_hide: Option<bool>,
    distance_px: Option<f64>,
    position: Option<[f64; 2]>,
}

fn settings_set_view_cube(input: Value, s: &mut Session) -> Run {
    let i: SetViewCube = parse(input)?;
    let mut c: CubeSettings = s.view_cube.clone();
    if let Some(p) = i.placement {
        c.placement = p;
    }
    if let Some(a) = i.auto_hide {
        c.auto_hide = a;
    }
    if let Some(d) = i.distance_px {
        let d = finite(d, "distance_px")?;
        if !(0.0..=murmel_engine::cube::MAX_DISTANCE).contains(&d) {
            return Err(Diagnostic::new(
                "TOOL002",
                format!(
                    "distance_px must be 0 to {} pixels",
                    murmel_engine::cube::MAX_DISTANCE
                ),
            ));
        }
        c.distance_px = d;
    }
    if let Some([x, y]) = i.position {
        c.position = Some([finite(x, "position")?, finite(y, "position")?]);
    }
    // Applied live first; see settings.set_bindings.
    s.view_cube = c;
    let path = save_session_settings(s)?;
    Ok(json!({"view_cube": s.view_cube, "files": [path]}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetBindings {
    preset: Option<String>,
    bindings: Option<Vec<Binding>>,
    zoom_to_cursor: Option<bool>,
    invert_zoom: Option<bool>,
    orbit_speed: Option<f64>,
    zoom_speed: Option<f64>,
}

fn settings_set_bindings(input: Value, s: &mut Session) -> Run {
    let i: SetBindings = parse(input)?;
    let mut nav = match (&i.preset, i.bindings) {
        (Some(_), Some(_)) => {
            return Err(Diagnostic::new(
                "TOOL002",
                "give either preset or bindings, not both",
            ));
        }
        (Some(p), None) => Navigation::preset(p).ok_or_else(|| {
            Diagnostic::new(
                "TOOL002",
                format!("unknown preset {p}; presets: {}", PRESETS.join(", ")),
            )
        })?,
        (None, Some(b)) => Navigation {
            bindings: b,
            preset: "custom".into(),
            ..s.navigation.clone()
        },
        (None, None) => s.navigation.clone(),
    };
    if let Some(v) = i.zoom_to_cursor {
        nav.zoom_to_cursor = v;
    }
    if let Some(v) = i.invert_zoom {
        nav.invert_zoom = v;
    }
    if let Some(v) = i.orbit_speed {
        nav.orbit_speed = finite(v, "orbit_speed")?.clamp(0.01, 5.0);
    }
    if let Some(v) = i.zoom_speed {
        nav.zoom_speed = finite(v, "zoom_speed")?.clamp(1e-4, 0.1);
    }
    if let Some((a, b)) = nav.conflicts().first() {
        return Err(Diagnostic::new(
            "SET002",
            format!("{:?} and {:?} share one input", a.action, b.action),
        ));
    }
    // Applied live first; a file that cannot be written fails the call
    // but the session keeps the bindings (as the theme does).
    s.navigation = nav;
    let dir = config_dir(s)?;
    let path = save_navigation(&dir, &s.navigation).map_err(|e| {
        Diagnostic::new(
            "SET001",
            format!("applied for this session, not saved: {e}"),
        )
    })?;
    Ok(json!({"navigation": s.navigation, "files": [path]}))
}

fn settings_get(_: Value, s: &mut Session) -> Run {
    Ok(json!({
        "theme": s.theme,
        "colors": s.colors,
        "navigation": s.navigation,
        "view_cube": s.view_cube,
        "agents": {"enabled": s.agents},
        "presets": PRESETS,
        "defaults": s.defaults,
        "defaultable": crate::defaults::defaultable(),
        "formulas": s.formulas,
        "config_dir": s.config_dir,
    }))
}

// ---- session files ------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionPath {
    path: PathBuf,
}

fn session_save(input: Value, s: &mut Session) -> Run {
    let i: SessionPath = parse(input)?;
    // The file names the mesh by its digest.
    s.wait_digest();
    let f = SessionFile::of(s, &i.path);
    std::fs::write(&i.path, f.to_json())
        .map_err(|e| Diagnostic::new("SESS002", format!("{}: {e}", i.path.display())))?;
    Ok(json!({"files": [i.path], "session": f}))
}

/// The same file or folder, links followed where both exist.
fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => absolute(a) == absolute(b),
    }
}

fn session_load(input: Value, s: &mut Session) -> Run {
    let i: SessionPath = parse(input)?;
    let text = std::fs::read_to_string(&i.path)
        .map_err(|e| Diagnostic::new("SESS001", format!("{}: {e}", i.path.display())))?;
    let f = SessionFile::parse(&text)
        .map_err(|r| Diagnostic::new("SESS001", format!("{}: {}", i.path.display(), r.0)))?;
    let mut warnings = Vec::new();
    match (f.mesh_path(&i.path), &f.mesh) {
        (Some(p), Some(saved)) => {
            // The open mesh is recognised by its digest.
            s.wait_digest();
            let open = s
                .mesh
                .as_ref()
                .is_some_and(|m| m.digest == saved.sha256 && same_path(&m.path, &p));
            if !open {
                s.open(&p).map_err(|e| {
                    Diagnostic::new(e.code, format!("the session's mesh: {}", e.message))
                })?;
                s.wait_digest();
            }
            if s.mesh.as_ref().is_some_and(|m| m.digest != saved.sha256) {
                warnings.push(json!(Diagnostic::new(
                    "SESS003",
                    format!(
                        "{} changed since the session was saved; items it no longer has are skipped",
                        p.display()
                    ),
                )));
            }
        }
        _ => s.close(),
    }
    let mut f = f;
    if let Some(p) = f.parcels_path(&i.path)
        && let Some(v) = f.display.parcels.as_mut()
    {
        v.path = p;
    }
    let series = f.series.clone().zip(f.series_path(&i.path));
    let mut missing = s.restore(f);
    // The series walked, at the step shown (its files are open already).
    if let Some((r, p)) = series {
        match murmel_engine::series::Series::read(&p) {
            Ok(mut walked) => {
                walked.current = walked.index_of_step(r.step).unwrap_or(0);
                walked.playback.rate = r.rate;
                walked.playback.looping = r.looping;
                s.series = Some(walked);
            }
            Err(_) => missing.push(format!("series:{}", p.display())),
        }
    }
    if !missing.is_empty() {
        warnings.push(json!(Diagnostic::new(
            "SESS004",
            format!("not in the mesh, skipped: {}", missing.join(", ")),
        )));
    }
    Ok(json!({
        "files": [i.path],
        "mesh": s.mesh.as_ref().map(|m| &m.path),
        "camera": camera_json(s),
        "display": display_json(s),
        "series": crate::series::series_json(s),
        "view_cube": s.view_cube,
        "skipped": missing,
        "warnings": warnings,
    }))
}

// ---- measuring and probing ---------------------------------------------

/// A point given to a measuring tool: exactly one of a screen point
/// (`ndc`, snapped), an anchor as `measure.snap` returned it (the window
/// snaps while the pointer moves, so a later click keeps what it showed
/// even after the view turned), typed coordinates, a node, the midpoint
/// of two nodes, a face centre or a cell centre.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct AnchorInput {
    ndc: Option<[f64; 2]>,
    anchor: Option<Anchor>,
    /// What `ndc` may snap to, in any order (they are tried in a fixed
    /// priority); left out: all but cell centres.
    snap: Option<Vec<SnapKind>>,
    radius_px: Option<f64>,
    point: Option<[f64; 3]>,
    node: Option<u32>,
    edge: Option<[u32; 2]>,
    face: Option<u32>,
    cell: Option<u32>,
}

fn anchor_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "description": "Exactly one of: ndc (a screen point, -1..1 with y up, snapped), anchor (as measure.snap returns it), point (typed x y z), node, edge (the midpoint of two nodes), face (a face centre), cell (a cell centre).",
        "properties": {
            "ndc": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2},
            "anchor": {
                "type": "object",
                "properties": {
                    "point": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
                    "snap": {"enum": ["node", "edge_midpoint", "face_centre", "cell_centre", "edge", "surface", "typed"]},
                    "node": {"type": "integer", "minimum": 0},
                    "edge": {"type": "array", "items": {"type": "integer", "minimum": 0}, "minItems": 2, "maxItems": 2},
                    "face": {"type": "integer", "minimum": 0},
                    "cell": {"type": "integer", "minimum": 0}
                },
                "required": ["point"],
                "additionalProperties": false
            },
            "snap": {"type": "array", "items": {"enum": ["node", "edge_midpoint", "face_centre", "cell_centre", "edge", "surface"]}},
            "radius_px": {"type": "number", "minimum": 0, "maximum": 200},
            "point": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
            "node": {"type": "integer", "minimum": 0},
            "edge": {"type": "array", "items": {"type": "integer", "minimum": 0}, "minItems": 2, "maxItems": 2},
            "face": {"type": "integer", "minimum": 0},
            "cell": {"type": "integer", "minimum": 0}
        }
    })
}

/// The anchor `i` names in the open mesh. `locate` finds the cell
/// holding typed coordinates (for probes).
fn anchor(s: &Session, i: AnchorInput, locate: bool) -> Result<Anchor, Diagnostic> {
    let m = s
        .mesh
        .as_ref()
        .ok_or_else(|| Diagnostic::new("VIEW001", "no mesh is open"))?;
    let mesh = &m.mesh;
    let given = [
        i.ndc.is_some(),
        i.anchor.is_some(),
        i.point.is_some(),
        i.node.is_some(),
        i.edge.is_some(),
        i.face.is_some(),
        i.cell.is_some(),
    ]
    .iter()
    .filter(|&&b| b)
    .count();
    if given != 1 {
        return Err(Diagnostic::new(
            "TOOL002",
            "give exactly one of ndc, anchor, point, node, edge, face, cell",
        ));
    }
    if (i.snap.is_some() || i.radius_px.is_some()) && i.ndc.is_none() {
        return Err(Diagnostic::new("TOOL002", "snap and radius_px go with ndc"));
    }
    let none = |what: &str, n: String| {
        Diagnostic::new("MEAS002", format!("the open mesh has no {what} {n}"))
    };
    if let Some(ndc) = i.ndc {
        for v in ndc {
            finite(v, "ndc")?;
        }
        let r = finite(i.radius_px.unwrap_or(SNAP_RADIUS_PX), "radius_px")?.clamp(0.0, 200.0);
        let kinds = i.snap.unwrap_or_else(|| SnapKind::DEFAULT.to_vec());
        return measure::snap(mesh, &m.surface, &s.camera, ndc, s.viewport, &kinds, r).ok_or_else(
            || {
                Diagnostic::new(
                    "MEAS001",
                    format!("nothing of the mesh to snap to under ndc {ndc:?}"),
                )
            },
        );
    }
    if let Some(mut a) = i.anchor {
        for v in a.point {
            finite(v, "anchor point")?;
        }
        // References the mesh does not have are dropped; a probe's cell
        // is found again.
        let cells = mesh.cell_count();
        a.node = a.node.filter(|&n| (n as usize) < mesh.points.len());
        a.edge = a
            .edge
            .filter(|e| e.iter().all(|&n| (n as usize) < mesh.points.len()));
        a.face = a.face.filter(|&u| !mesh.face_nodes(u as usize).is_empty());
        a.cell = a.cell.filter(|&c| (c as usize) < cells);
        if locate && a.cell.is_none() {
            a.cell = measure::locate(mesh, a.point);
        }
        return Ok(a);
    }
    if let Some(p) = i.point {
        for v in p {
            finite(v, "point")?;
        }
        let mut a = Anchor::typed(p);
        if locate {
            a.cell = measure::locate(mesh, p);
        }
        return Ok(a);
    }
    if let Some(n) = i.node {
        return Anchor::node(mesh, n).ok_or_else(|| none("node", n.to_string()));
    }
    if let Some([a, b]) = i.edge {
        return Anchor::edge_midpoint(mesh, a, b)
            .ok_or_else(|| none("nodes", format!("{a} and {b}")));
    }
    if let Some(u) = i.face {
        return Anchor::face_centre(mesh, u).ok_or_else(|| none("face", u.to_string()));
    }
    let c = i.cell.unwrap_or_default();
    Anchor::cell_centre(mesh, c).ok_or_else(|| none("cell", c.to_string()))
}

fn measure_json(m: &Measurement) -> Value {
    json!({
        "id": format!("measure:{}", m.n),
        "kind": m.kind,
        "name": m.name,
        "value": m.value(),
        "unit": match m.kind {
            MeasureKind::Distance => "m",
            MeasureKind::Angle => "°",
        },
        "delta": m.delta(),
        "points": m.points,
        "visible": m.visible,
    })
}

fn probe_json(s: &Session, p: &Probe) -> Value {
    json!({
        "id": format!("probe:{}", p.n),
        "name": p.name,
        "at": p.at,
        "visible": p.visible,
        "rows": s.mesh.as_ref().map(|m| murmel_engine::inspect::probe_rows(&m.mesh, &p.at)),
        "fields": probe_fields(s, &p.at),
    })
}

/// The field values at an anchor inside a cell (none outside the mesh).
fn probe_fields(s: &Session, at: &Anchor) -> Vec<field::FieldValue> {
    match (&s.mesh, at.cell) {
        (Some(m), Some(c)) => m.fields.point_values(&m.mesh, c as usize, at.point),
        _ => Vec::new(),
    }
}

fn measures_json(s: &Session) -> Value {
    json!({
        "measurements": s.display.measurements.iter().map(measure_json).collect::<Vec<_>>(),
        "probes": s.display.probes.iter().map(|p| probe_json(s, p)).collect::<Vec<_>>(),
    })
}

/// A name for a measurement or a probe, as for saved views.
fn measure_name(name: Option<String>) -> Result<Option<String>, Diagnostic> {
    match name {
        None => Ok(None),
        Some(n) => view_name(&n).map(Some).ok_or_else(|| {
            Diagnostic::new(
                "MEAS003",
                format!("name {n:?} is empty, longer than {VIEW_NAME_MAX} characters or holds control characters"),
            )
        }),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapInput {
    ndc: [f64; 2],
    snap: Option<Vec<SnapKind>>,
    radius_px: Option<f64>,
}

fn measure_snap(input: Value, s: &mut Session) -> Run {
    let i: SnapInput = parse(input)?;
    need_mesh(s)?;
    let a = anchor(
        s,
        AnchorInput {
            ndc: Some(i.ndc),
            snap: i.snap,
            radius_px: i.radius_px,
            ..AnchorInput::default()
        },
        false,
    );
    match a {
        Ok(a) => Ok(json!({"anchor": a})),
        Err(d) if d.code == "MEAS001" => Ok(json!({"anchor": null})),
        Err(d) => Err(d),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasureAdd {
    kind: MeasureKind,
    points: Vec<AnchorInput>,
    name: Option<String>,
}

fn measure_add(input: Value, s: &mut Session) -> Run {
    let i: MeasureAdd = parse(input)?;
    need_mesh(s)?;
    if i.points.len() != i.kind.points() {
        return Err(Diagnostic::new(
            "TOOL002",
            format!(
                "a {} takes {} points, not {}",
                i.kind.key(),
                i.kind.points(),
                i.points.len()
            ),
        ));
    }
    let name = measure_name(i.name)?;
    let points = i
        .points
        .into_iter()
        .map(|p| anchor(s, p, false))
        .collect::<Result<Vec<_>, _>>()?;
    let m = Measurement {
        n: s.display.next_measure(),
        kind: i.kind,
        name,
        points,
        visible: true,
    };
    let out = measure_json(&m);
    s.display.measurements.push(m);
    Ok(out)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeAdd {
    at: AnchorInput,
    name: Option<String>,
}

fn probe_add(input: Value, s: &mut Session) -> Run {
    let i: ProbeAdd = parse(input)?;
    need_mesh(s)?;
    let name = measure_name(i.name)?;
    let p = Probe {
        n: s.display.next_probe(),
        name,
        at: anchor(s, i.at, true)?,
        visible: true,
    };
    let out = probe_json(s, &p);
    s.display.probes.push(p);
    Ok(out)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeAt {
    at: AnchorInput,
}

fn probe_at(input: Value, s: &mut Session) -> Run {
    let i: ProbeAt = parse(input)?;
    need_mesh(s)?;
    let at = anchor(s, i.at, true)?;
    let rows = s
        .mesh
        .as_ref()
        .map(|m| murmel_engine::inspect::probe_rows(&m.mesh, &at));
    let fields = probe_fields(s, &at);
    Ok(json!({"at": at, "rows": rows, "fields": fields}))
}

/// A measurement or probe of the open mesh named by `id`.
fn measure_item(s: &Session, id: &str) -> Result<Item, Diagnostic> {
    match item(s, id)? {
        it @ (Item::Measure(_) | Item::Probe(_)) => Ok(it),
        _ => Err(Diagnostic::new(
            "VIEW002",
            format!("{id} is not a measurement (measure:<n>) or a probe (probe:<n>)"),
        )),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasureRename {
    item: String,
    /// Null or left out goes back to "Distance n", "Angle n", "Probe n".
    #[serde(default)]
    name: Option<String>,
}

fn measure_rename(input: Value, s: &mut Session) -> Run {
    let i: MeasureRename = parse(input)?;
    let it = measure_item(s, &i.item)?;
    let name = measure_name(i.name)?;
    let d = &mut s.display;
    match it {
        Item::Measure(n) => {
            for m in d.measurements.iter_mut().filter(|m| m.n == n) {
                m.name.clone_from(&name);
            }
        }
        Item::Probe(n) => {
            for p in d.probes.iter_mut().filter(|p| p.n == n) {
                p.name.clone_from(&name);
            }
        }
        _ => {}
    }
    Ok(json!({"item": i.item, "name": name}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasureRemove {
    #[serde(default)]
    items: Vec<String>,
    #[serde(default)]
    all: bool,
}

fn measure_remove(input: Value, s: &mut Session) -> Run {
    let i: MeasureRemove = parse(input)?;
    need_mesh(s)?;
    if i.all == !i.items.is_empty() {
        return Err(Diagnostic::new("TOOL002", "give either items or all: true"));
    }
    let items: Vec<Item> = if i.all {
        let d = &s.display;
        d.measurements
            .iter()
            .map(|m| Item::Measure(m.n))
            .chain(d.probes.iter().map(|p| Item::Probe(p.n)))
            .collect()
    } else {
        i.items
            .iter()
            .map(|id| measure_item(s, id))
            .collect::<Result<_, _>>()?
    };
    let removed = s.display.remove_measures(&items);
    let mut out = measures_json(s);
    out["removed"] = json!(removed);
    Ok(out)
}

fn measure_list(_: Value, s: &mut Session) -> Run {
    need_mesh(s)?;
    Ok(measures_json(s))
}

// ---- fields -------------------------------------------------------------

/// The fields of the open mesh; FIELD001 when it has none.
pub(crate) fn need_fields(s: &Session) -> Result<&FieldSet, Diagnostic> {
    need_mesh(s)?;
    match s.fields() {
        Some(f) if !f.is_empty() => Ok(f),
        _ => Err(Diagnostic::new(
            "FIELD001",
            "the open mesh has no fields: it was read without results",
        )),
    }
}

/// The field `name` (or the one chosen now, or the first of the file)
/// with `component` (or the one chosen now for the same field).
pub(crate) fn field_choice(
    s: &Session,
    name: Option<&str>,
    component: Option<Component>,
) -> Result<FieldChoice, Diagnostic> {
    let set = need_fields(s)?;
    let current = s.display.field.as_ref();
    let name = match name {
        Some(n) => n.to_string(),
        None => current
            .map(|c| c.name.clone())
            .filter(|n| set.get(n).is_some())
            .or_else(|| set.names().first().map(|n| n.to_string()))
            .unwrap_or_default(),
    };
    let Some(f) = set.get(&name) else {
        return Err(Diagnostic::new(
            "FIELD002",
            format!(
                "no field {name}; the open mesh has {}",
                set.names().join(", ")
            ),
        ));
    };
    let component = match component {
        Some(c) if !f.is_vector() && c != Component::Magnitude => {
            return Err(Diagnostic::new(
                "FIELD003",
                format!("{name} is a scalar; it has no component {}", c.name()),
            ));
        }
        Some(c) => c,
        None => current
            .filter(|c| c.name == name && f.is_vector())
            .map_or(Component::Magnitude, |c| c.component),
    };
    Ok(FieldChoice { name, component })
}

/// The field colouring shown now: the field, its colour range and map,
/// and its style (null when the view is not coloured by a field).
fn field_shown_json(s: &Session) -> Value {
    let Some(v) = s.field_values() else {
        return Value::Null;
    };
    json!({
        "field": v.choice.name,
        "component": v.choice.component,
        "title": v.choice.title(v.vector),
        "unit": v.unit,
        "location": v.location,
        "range": s.field_range().map(|(a, b)| [a, b]),
        "field_range": v.range.map(|(a, b)| [a, b]),
        "map": s.field_map().map(|(m, _)| m.id.clone()),
        "style": s.field_style(),
    })
}

fn field_list(_: Value, s: &mut Session) -> Run {
    need_mesh(s)?;
    let Some(set) = s.fields() else {
        return Ok(json!({"fields": []}));
    };
    let fields: Vec<Value> = set
        .fields()
        .iter()
        .map(|f| {
            let comps: Vec<Component> = if f.is_vector() {
                Component::ALL.to_vec()
            } else {
                vec![Component::Magnitude]
            };
            let ranges: serde_json::Map<String, Value> = comps
                .iter()
                .map(|&c| (c.name().to_string(), json!(f.range(c).map(|(a, b)| [a, b]))))
                .collect();
            json!({
                "name": f.name,
                "location": f.location,
                "components": f.components,
                "unit": f.unit,
                "count": f.len(),
                "range": f.range(Component::Magnitude).map(|(a, b)| [a, b]),
                "ranges": if f.is_vector() { Value::Object(ranges) } else { Value::Null },
                "style": s.display.field_styles.get(&f.name),
            })
        })
        .collect();
    let r = &set.results;
    Ok(json!({
        "fields": fields,
        "time": r.time,
        "step": r.step,
        "iteration": r.iteration,
        "attributes": r.attrs,
        "colormaps": murmel_engine::colormap::all().iter().map(|m| json!({"id": m.id, "kind": m.kind})).collect::<Vec<_>>(),
        "shown": field_shown_json(s),
    }))
}

/// What the colour range of `field.style` spans.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RangeKey {
    Field,
    Shown,
    Symmetric,
    Fixed,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldStyleInput {
    /// The field to style; left out, the field chosen now.
    #[serde(default)]
    field: Option<String>,
    /// Start again from the default style.
    #[serde(default)]
    reset: bool,
    #[serde(default)]
    mode: Option<FieldMode>,
    #[serde(default)]
    bands: Option<u32>,
    #[serde(default)]
    lines: Option<bool>,
    #[serde(default)]
    line_count: Option<u32>,
    /// A colour map id, or `auto`.
    #[serde(default)]
    map: Option<String>,
    #[serde(default)]
    reverse: Option<bool>,
    #[serde(default)]
    range: Option<RangeKey>,
    #[serde(default)]
    min: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
}

fn field_style(input: Value, s: &mut Session) -> Run {
    let i: FieldStyleInput = parse(input)?;
    let set = need_fields(s)?;
    let name = match i
        .field
        .clone()
        .or_else(|| s.display.field.as_ref().map(|c| c.name.clone()))
    {
        Some(n) => n,
        None => {
            return Err(Diagnostic::new(
                "FIELD002",
                "no field is chosen; pass field, or colour by one first",
            ));
        }
    };
    if set.get(&name).is_none() {
        return Err(Diagnostic::new(
            "FIELD002",
            format!(
                "no field {name}; the open mesh has {}",
                set.names().join(", ")
            ),
        ));
    }
    let bad = |m: String| Err(Diagnostic::new("FIELD004", m));
    // Reset goes back to your default style.
    let mut st = if i.reset {
        crate::defaults::fresh(
            s,
            murmel_engine::defaults::Kind::FieldStyle,
            FieldStyle::default(),
        )
    } else {
        s.display
            .field_styles
            .get(&name)
            .cloned()
            .unwrap_or_default()
    };
    if let Some(m) = i.mode {
        st.mode = m;
    }
    if let Some(b) = i.bands {
        if !(field::MIN_BANDS..=field::MAX_BANDS).contains(&b) {
            return bad(format!(
                "bands {b} is not between {} and {}",
                field::MIN_BANDS,
                field::MAX_BANDS
            ));
        }
        st.bands = b;
    }
    if let Some(l) = i.lines {
        st.lines = l;
    }
    if let Some(n) = i.line_count {
        if !(1..=field::MAX_LINES).contains(&n) {
            return bad(format!(
                "line_count {n} is not between 1 and {}",
                field::MAX_LINES
            ));
        }
        st.line_count = n;
    }
    if let Some(m) = &i.map {
        if m == "auto" {
            st.map = None;
        } else if murmel_engine::colormap::get(m).is_some() {
            st.map = Some(m.clone());
        } else {
            return bad(format!(
                "no colour map {m}; there are auto, {}",
                murmel_engine::colormap::ids().join(", ")
            ));
        }
    }
    if let Some(r) = i.reverse {
        st.reverse = r;
    }
    let range = match (i.range, i.min.is_some() || i.max.is_some()) {
        (Some(r), _) => Some(r),
        (None, true) => Some(RangeKey::Fixed),
        (None, false) => None,
    };
    match range {
        Some(RangeKey::Field) => st.range = RangeMode::Field,
        Some(RangeKey::Shown) => st.range = RangeMode::Shown,
        Some(RangeKey::Symmetric) => st.range = RangeMode::Symmetric,
        Some(RangeKey::Fixed) => {
            // A bound left out keeps the range shown now (or the field's).
            let now = if s.display.field.as_ref().is_some_and(|c| c.name == name) {
                s.field_range()
            } else {
                None
            }
            .or_else(|| set.get(&name).and_then(|f| f.range(Component::Magnitude)));
            let (min, max) = match (i.min.or(now.map(|r| r.0)), i.max.or(now.map(|r| r.1))) {
                (Some(a), Some(b)) => (a, b),
                _ => return bad("give min and max of the fixed range".into()),
            };
            if !(min.is_finite() && max.is_finite() && min < max) {
                return bad(format!(
                    "the fixed range {min} to {max} needs finite min < max"
                ));
            }
            st.range = RangeMode::Fixed { min, max };
        }
        None => {}
    }
    s.display.field_styles.insert(name.clone(), st);
    // The style is drawn by the GPU from uniforms; the surface stays.
    // Only range mode shown needs the drawn range, which every build
    // keeps.
    let mut out = field_shown_json(s);
    if out.is_null() {
        out = json!({"field": name, "style": s.display.field_styles.get(&name), "shown": false});
    }
    Ok(out)
}

// ---- journal ------------------------------------------------------------

/// The tools whose calls are steps of the journal: they change the camera
/// or the display (UI spec: "Ctrl + Z, Ctrl + Shift + Z: undo, redo
/// (camera and display)"). Every other tool is in `NOT_JOURNALED` with
/// the reason (a test checks).
pub const JOURNALED: &[&str] = &[
    "view.set_camera",
    "view.orbit",
    "view.pan",
    "view.zoom",
    "view.fit",
    "view.standard",
    "view.rotate",
    "views.save",
    "views.go",
    "views.rename",
    "views.delete",
    "display.render_mode",
    "display.color_by",
    "display.threshold",
    "field.style",
    "slice.set",
    "slice.rotate",
    "slice.clear",
    "iso.set",
    "iso.clear",
    "arrows.set",
    "arrows.clear",
    "stream.set",
    "stream.clear",
    "parcels.open",
    "parcels.set",
    "parcels.clear",
    "region.set",
    "region.clear",
    "display.mesh_look",
    "display.set_color",
    "display.visibility",
    "clip.set",
    "clip.rotate",
    "clip.clear",
    "measure.add",
    "measure.rename",
    "measure.remove",
    "probe.add",
];

/// Tools that make no step, and why.
pub const NOT_JOURNALED: &[(&str, &str)] = &[
    ("system.info", READS_ONLY),
    ("system.commands", READS_ONLY),
    ("diag.codes", READS_ONLY),
    ("mesh.open", "a new mesh starts a new journal"),
    ("mesh.close", "closing the mesh ends its journal"),
    ("mesh.info", READS_ONLY),
    ("view.get_camera", READS_ONLY),
    ("views.list", READS_ONLY),
    (
        "view.screenshot",
        "writes a file; a mesh or view it is given is for the picture",
    ),
    ("export.image", "writes a file; the view stays as it is"),
    (
        "export.movie",
        "writes files; the camera and the clip planes go back as they were",
    ),
    (
        "select.item",
        "the selection is not undone, as in CAD tools",
    ),
    (
        "select.pick",
        "the selection is not undone, as in CAD tools",
    ),
    (
        "select.clear",
        "the selection is not undone, as in CAD tools",
    ),
    (
        "select.filter",
        "a mode of picking, not a change of the view",
    ),
    ("field.list", READS_ONLY),
    (
        "field.derive",
        "adds or removes a field of the result; the view stays as it is",
    ),
    ("field.catalogue", READS_ONLY),
    (
        "field.calculate",
        "adds or removes a field of the result; the view stays as it is",
    ),
    ("inspect.at", READS_ONLY),
    ("inspect.quality", READS_ONLY),
    ("inspect.item", READS_ONLY),
    ("settings.get", READS_ONLY),
    ("settings.set_theme", "settings are files, not undone"),
    ("settings.set_colors", "settings are files, not undone"),
    ("settings.set_view_cube", "settings are files, not undone"),
    ("settings.set_agents", "settings are files, not undone"),
    ("settings.set_bindings", "settings are files, not undone"),
    ("settings.set_default", "settings are files, not undone"),
    ("settings.reset_default", "settings are files, not undone"),
    ("settings.keep_formula", "settings are files, not undone"),
    ("settings.forget_formula", "settings are files, not undone"),
    ("journal.undo", "walks the journal"),
    ("journal.redo", "walks the journal"),
    ("journal.group", "groups the steps"),
    ("journal.list", READS_ONLY),
    ("session.save", "writes a file; the view stays as it is"),
    ("session.load", "a loaded session starts a new journal"),
    ("measure.snap", READS_ONLY),
    ("measure.list", READS_ONLY),
    ("probe.at", READS_ONLY),
    ("region.list", READS_ONLY),
    ("blocks.make", "writes a file; the view stays as it is"),
    (
        "series.open",
        "opens files of a run; the view stays as it is",
    ),
    (
        "series.go",
        "shows another time step of the run; the view stays as it is",
    ),
    ("series.play", "how the window plays the series, not a view"),
];

/// The reason in `NOT_JOURNALED` of a tool that changes nothing: no file,
/// no camera, no display, no selection.
pub const READS_ONLY: &str = "reads only";

/// The tool only reads (the MCP server marks it read-only and idempotent).
pub fn reads_only(id: &str) -> bool {
    NOT_JOURNALED
        .iter()
        .any(|(t, why)| *t == id && *why == READS_ONLY)
}

pub(crate) fn journaled(id: &str) -> bool {
    JOURNALED.contains(&id)
}

fn journal_json(s: &Session) -> Value {
    json!({"undo": s.journal.undo.len(), "redo": s.journal.redo.len()})
}

fn journal_undo(_: Value, s: &mut Session) -> Run {
    let undone = s.undo();
    let mut out = journal_json(s);
    out["undone"] = json!(undone);
    out["camera"] = camera_json(s);
    Ok(out)
}

fn journal_redo(_: Value, s: &mut Session) -> Run {
    let redone = s.redo();
    let mut out = journal_json(s);
    out["redone"] = json!(redone);
    out["camera"] = camera_json(s);
    Ok(out)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    open: bool,
}

fn journal_group(input: Value, s: &mut Session) -> Run {
    let i: Group = parse(input)?;
    s.journal.group(i.open);
    Ok(json!({"open": s.journal.group_open()}))
}

fn journal_list(_: Value, s: &mut Session) -> Run {
    let tools = |v: &[murmel_engine::journal::Step]| -> Vec<String> {
        v.iter().map(|st| st.tool.clone()).collect()
    };
    Ok(json!({
        "undo": tools(&s.journal.undo),
        "redo": tools(&s.journal.redo),
        "group_open": s.journal.group_open(),
        "max_steps": murmel_engine::journal::MAX_STEPS,
    }))
}

// ---- the catalogue ------------------------------------------------------

pub static TOOLS: &[Tool] = &[
    Tool {
        id: "system.info",
        version: 1,
        action: Action::Read,
        label_key: "tool-system-info",
        summary: "Version and platform of this build.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: system_info,
    },
    Tool {
        id: "system.commands",
        version: 1,
        action: Action::Read,
        label_key: "tool-system-commands",
        summary: "The catalogue of every tool with its input schema.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: |_, _| Ok(crate::catalogue()),
    },
    Tool {
        id: "diag.codes",
        version: 1,
        action: Action::Read,
        label_key: "tool-diag-codes",
        summary: "The diagnostic codes with their severity, meaning and fix.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: diag_codes,
    },
    Tool {
        id: "mesh.open",
        version: 4,
        action: Action::Read,
        label_key: "tool-mesh-open",
        summary: "Load a mesh and the mesh-evidence.json beside it, replace the open mesh and fit the view; returns counts, zones, patches, the fields read with it and the files read. `path` is a .vtu (KoflCraft's foehncraft.vtu, with its cell and point data as fields), a FoehnCraft result (.vtkhdf: mesh and fields in one HDF5 file) or an OpenFOAM mesh: a case, constant or polyMesh folder, a .foam file in the case or a polyMesh file (ascii, binary, gzip). In the window the SHA-256 of the files is taken after the mesh opens: until it is, `validity` is `checking` and `digest` and the files' `sha256` are empty.",
        outputs: &[],
        reads: &["path"],
        input_schema: || {
            object(
                json!({"path": {"type": "string", "description": "A .vtu or .vtkhdf file, or an OpenFOAM case, constant or polyMesh folder (or a .foam file in the case)"}}),
                &["path"],
            )
        },
        run: mesh_open,
    },
    Tool {
        id: "mesh.close",
        version: 1,
        action: Action::Read,
        label_key: "tool-mesh-close",
        summary: "Drop the open mesh from the session.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: mesh_close,
    },
    Tool {
        id: "mesh.info",
        version: 1,
        action: Action::Read,
        label_key: "tool-mesh-info",
        summary: "Counts, zones, patches, metadata and evidence of the open mesh.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: mesh_info,
    },
    Tool {
        id: "view.get_camera",
        version: 1,
        action: Action::Read,
        label_key: "tool-view-get-camera",
        summary: "The camera: target, distance, yaw, pitch, roll, the cube view it looks along, projection, eye and viewport.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: view_get,
    },
    Tool {
        id: "view.set_camera",
        version: 1,
        action: Action::Read,
        label_key: "tool-view-set-camera",
        summary: "Set any of target, distance, yaw, pitch, roll, projection, field of view and viewport size.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "target": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
                    "distance": {"type": "number", "exclusiveMinimum": 0},
                    "yaw_deg": {"type": "number"},
                    "pitch_deg": {"type": "number", "minimum": -90, "maximum": 90},
                    "roll_deg": {"type": "number"},
                    "projection": {"enum": ["orthographic", "perspective", "perspective_ortho_faces"]},
                    "fov_deg": {"type": "number", "minimum": 1, "maximum": 120},
                    "viewport": {"type": "array", "items": {"type": "number", "minimum": 1}, "minItems": 2, "maxItems": 2}
                }),
                &[],
            )
        },
        run: view_set,
    },
    Tool {
        id: "view.orbit",
        version: 2,
        action: Action::Read,
        label_key: "tool-view-orbit",
        summary: "Turn the camera about the up axis (yaw) and tilt it (pitch), in degrees, about the target or about a world point pivot (the window passes the point under the cursor, or the centre of the shown faces over empty space).",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "yaw_deg": {"type": "number"},
                    "pitch_deg": {"type": "number"},
                    "pivot": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3}
                }),
                &[],
            )
        },
        run: view_orbit,
    },
    Tool {
        id: "view.pan",
        version: 1,
        action: Action::Read,
        label_key: "tool-view-pan",
        summary: "Move the view by a drag in pixels (x right, y down); the scene follows the pointer.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({"dx_px": {"type": "number"}, "dy_px": {"type": "number"}}),
                &[],
            )
        },
        run: view_pan,
    },
    Tool {
        id: "view.zoom",
        version: 1,
        action: Action::Read,
        label_key: "tool-view-zoom",
        summary: "Scale the camera distance by factor (below 1 zooms in), keeping the point under cursor_ndc (-1..1, y up) in place.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "factor": {"type": "number", "exclusiveMinimum": 0},
                    "cursor_ndc": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2}
                }),
                &["factor"],
            )
        },
        run: view_zoom,
    },
    Tool {
        id: "view.fit",
        version: 1,
        action: Action::Read,
        label_key: "tool-view-fit",
        summary: "Frame the faces shown, or the shown faces of one item (mesh, zone:<name>, kind:<cell type>, patch:<name>), from the current direction.",
        outputs: &[],
        reads: &[],
        input_schema: || object(json!({"item": {"type": "string"}}), &[]),
        run: view_fit,
    },
    Tool {
        id: "view.standard",
        version: 1,
        action: Action::Read,
        label_key: "tool-view-standard",
        summary: "Go to one of the 26 view-cube views (faces like front, edges like front_top, corners like front_right_top; parts in any order), iso or home, and fit; the roll is reset, and face views snap to orthographic in perspective_ortho_faces.",
        outputs: &[],
        reads: &[],
        input_schema: || object(json!({"view": {"enum": StandardView::names()}}), &["view"]),
        run: view_standard,
    },
    Tool {
        id: "view.rotate",
        version: 1,
        action: Action::Read,
        label_key: "tool-view-rotate",
        summary: "Rotate the scene by deg degrees about a screen axis (x right, y up, z towards the viewer; right-hand rule, so +90 about z turns it counter-clockwise on screen). The view cube's roll arrows send z ±90.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({"axis": {"enum": ["x", "y", "z"]}, "deg": {"type": "number"}}),
                &["axis", "deg"],
            )
        },
        run: view_rotate,
    },
    Tool {
        id: "views.save",
        version: 1,
        action: Action::Read,
        label_key: "tool-views-save",
        summary: "Save the camera as a named view of the open mesh (a bookmark in the model tree); name left out takes the first free \"View N\", a name that exists gets the current camera. Saved views go into session files; opening another mesh drops them.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({"name": {"type": "string", "minLength": 1, "maxLength": VIEW_NAME_MAX}}),
                &[],
            )
        },
        run: views_save,
    },
    Tool {
        id: "views.go",
        version: 1,
        action: Action::Read,
        label_key: "tool-views-go",
        summary: "Go to a saved view: its target, distance, angles, roll and projection (the window flies there).",
        outputs: &[],
        reads: &[],
        input_schema: || object(json!({"name": {"type": "string"}}), &["name"]),
        run: views_go,
    },
    Tool {
        id: "views.rename",
        version: 1,
        action: Action::Read,
        label_key: "tool-views-rename",
        summary: "Rename a saved view; the new name must be free.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({"name": {"type": "string"}, "to": {"type": "string", "minLength": 1, "maxLength": VIEW_NAME_MAX}}),
                &["name", "to"],
            )
        },
        run: views_rename,
    },
    Tool {
        id: "views.delete",
        version: 1,
        action: Action::Read,
        label_key: "tool-views-delete",
        summary: "Delete a saved view (undo brings it back).",
        outputs: &[],
        reads: &[],
        input_schema: || object(json!({"name": {"type": "string"}}), &["name"]),
        run: views_delete,
    },
    Tool {
        id: "views.list",
        version: 1,
        action: Action::Read,
        label_key: "tool-views-list",
        summary: "The saved views of the open mesh, in the order they were saved, with their cameras.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: views_list,
    },
    Tool {
        id: "view.screenshot",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-view-screenshot",
        summary: "Render the current view offscreen (wgpu) to a PNG at path; mesh and view set up the session first for one-shot use.",
        outputs: &["path"],
        reads: &["mesh"],
        input_schema: || {
            object(
                json!({
                    "path": {"type": "string"},
                    "width": {"type": "integer", "minimum": 1, "maximum": 8192, "default": 1280},
                    "height": {"type": "integer", "minimum": 1, "maximum": 8192, "default": 800},
                    "mesh": {"type": "string"},
                    "view": {"enum": StandardView::names()},
                    "theme": {"enum": ["dark", "light", "system"]}
                }),
                &["path"],
            )
        },
        run: view_screenshot,
    },
    Tool {
        id: "export.image",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-export-image",
        summary: "Export the view as a PNG at path for reports and posters: the viewport's size times scale (or width and height in pixels), lines, legend and text grown with the scale so the picture looks like the window, the colour legend (top left) and the axis triad (bottom left) each optional, the background transparent if asked, the resolution (dpi) stored for print.",
        outputs: &["path"],
        reads: &[],
        input_schema: crate::export::export_image_schema,
        run: crate::export::export_image,
    },
    Tool {
        id: "export.movie",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-export-movie",
        summary: "Export a movie at path (.mp4, .gif, or a folder of PNG frames) with the look of export.image (size, scale, legend, axes, lines): the camera turning about its target (orbit; a whole turn loops), a clip plane sweeping through the mesh (clip), or a flight through saved views that eases into each and holds there (views). MP4 is H.264 made by the ffmpeg installed on the machine (EXPORT002 without one); GIF and PNG are written by MurmelCraft. The view stays as it was.",
        outputs: &["path"],
        reads: &[],
        input_schema: crate::movie::export_movie_schema,
        run: crate::movie::export_movie,
    },
    Tool {
        id: "display.render_mode",
        version: 1,
        action: Action::Read,
        label_key: "tool-display-render-mode",
        summary: "Draw the mesh shaded with edges (the default), shaded, as a wireframe of every edge, or as feature edges only (creases over 30 degrees, patch borders, open borders).",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({"mode": {"enum": ["shaded_edges", "shaded", "wireframe", "feature_edges"]}}),
                &["mode"],
            )
        },
        run: display_render_mode,
    },
    Tool {
        id: "display.color_by",
        version: 3,
        action: Action::Read,
        label_key: "tool-display-color-by",
        summary: "Colour the boundary faces in one colour (solid, the default), by patch, by patch role, by the type of their cell, by the cell zone of their cell (zone; on the patch palette, zone colours set with display.set_color win), by the quality of their cell (metric: non_orthogonality in degrees, skewness, volume_ratio or face_count; worst over the cell's faces, on a ramp over the metric's range), or by a field of the results (field: its name, default the one chosen before or the file's first; component of a vector: magnitude, x, y or z). Changing the metric clears the threshold. A field is drawn in its style (field.style).",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "by": {"enum": ["solid", "patch", "role", "cell_type", "zone", "quality", "field"]},
                    "metric": {"enum": ["non_orthogonality", "skewness", "volume_ratio", "face_count"]},
                    "field": {"type": "string", "description": "A field name (field.list); with by: field"},
                    "component": {"enum": ["magnitude", "x", "y", "z"], "description": "The scalar of a vector field; with by: field"}
                }),
                &["by"],
            )
        },
        run: display_color_by,
    },
    Tool {
        id: "field.list",
        version: 1,
        action: Action::Read,
        label_key: "tool-field-list",
        summary: "The fields of the open mesh (name, location cell, point or boundary, components, unit, count, range per component, style), the result's time, step or iteration and file attributes, the colour maps, and the field colouring shown now (colour range, map, style).",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: field_list,
    },
    Tool {
        id: "field.derive",
        version: 1,
        action: Action::Read,
        label_key: "tool-field-derive",
        summary: "Compute a derived field from the open result's fields and add it to them, as if the file had it (colour by it, slice it, iso-surfaces, arrows and streamlines of it; saved in sessions as its recipe and computed again on loading). quantity: vorticity (the vector field vorticity, ω = ∇ × u, 1/s), q-criterion (q_criterion, Q = ½ (‖Ω‖² − ‖S‖²), vortices where Q > 0, 1/s²), lambda2 (lambda2, the middle eigenvalue of S² + Ω², vortices where λ₂ < 0, 1/s²), total-pressure (total_pressure, p + ½ ρ u·u, Pa) or pressure-coefficient (pressure_coefficient, (p − p_ref) / (½ ρ_ref U_ref²)). S and Ω are the symmetric and antisymmetric parts of the velocity gradient, taken in each cell by least squares from the cells around it (the vortex quantities are cell fields); the pressure quantities are taken wherever the pressure is (cells, nodes, boundary faces). velocity and pressure name the input fields; left out, they are found by name (velocity, U, u; pressure, p, static_pressure). A pressure in m²/s² is kinematic and takes no density; a density field (density, rho) wins over the reference density. reference: density (kg/m³), pressure (p_ref) and velocity (U_ref); values left out are the field's own when it is made again, else FoehnCraft's (case.resolved.toml beside the result), else your defaults (settings.set_default kind reference), else air at sea level (1.225), 0 and the largest speed of the velocity; the result's reference_from says where each came from. Computing a field again replaces it. remove: true removes the derived field (refused while an iso-surface, the arrows or a streamline set is made from it). A field the file already has is not computed again. field.catalogue lists what can be derived.",
        outputs: &[],
        reads: &[],
        input_schema: crate::derive::field_derive_schema,
        run: crate::derive::field_derive,
    },
    Tool {
        id: "field.catalogue",
        version: 1,
        action: Action::Read,
        label_key: "tool-field-catalogue",
        summary: "What can be derived from the open result (field.derive): for each quantity its group (vortices, pressure), field name, formula, unit, inputs (the field each reads now and the fields that could be it), the reference values it uses, the inputs missing, whether the file has the field already and the recipe of the field when it is derived; the reference values a new field starts from, with where each came from (case, defaults or built_in); your formulas (formulas, each with whether its field is computed now), the fields computed from formulas (calculated, with the fields each reads and whether it is kept among your formulas) and the calculator's functions (field.calculate).",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: crate::derive::field_catalogue,
    },
    Tool {
        id: "field.calculate",
        version: 1,
        action: Action::Read,
        label_key: "tool-field-calculate",
        summary: "Compute the field of a formula of your own from the open result's fields and add it to them, as field.derive does (saved in sessions as its formula and computed again on loading). name: the field it makes (a letter or _ first, then letters, digits and _; not a field of the file or of a derived quantity). formula: numbers, pi, the coordinates x, y, z and the position vector pos (of each cell centroid, node or face centre), field names bare or quoted (\"p rgh\"; a field wins over a built-in name), + - * / ^, the comparisons < <= > >= (1 or 0), components U.x U.y U.z, vectors [a, b, c], and the functions sqrt abs exp log log10 sin cos tan asin acos atan atan2 sinh cosh tanh sign floor ceil min max if(c, a, b) mag dot cross grad curl div (field.catalogue lists them with their arguments). The result is a scalar or a vector; it sits wherever every field it reads is (cells, nodes, boundary faces), in the cells when it takes a derivative (grad, curl and div are least-squares gradients, exact for linear fields) or when no one place has every field (node fields then as each cell's mean). Values that are not finite are no value (no_value counts them). formula left out: your formula of that name (settings.keep_formula). Computing a field again replaces it and computes every field made from it again (made_again); a formula reading its own field, or one made from it, is refused. remove: true removes it (refused while an item or another field is made from it). Errors say the column.",
        outputs: &[],
        reads: &[],
        input_schema: crate::derive::field_calculate_schema,
        run: crate::derive::field_calculate,
    },
    Tool {
        id: "field.style",
        version: 1,
        action: Action::Read,
        label_key: "tool-field-style",
        summary: "Set how a field is drawn (the one chosen now, or `field`): mode flat (one colour per cell, the solver's values), smooth (node values rebuilt from the cells' least-squares gradients, the default) or banded (bands steps, 2 to 64, default 12); contour lines on or off (lines) and how many (line_count, 1 to 64, default 12, splitting the range into line_count + 1 even steps); the colour map (map: an id of field.list or auto, which takes rainbow_desaturated, or rainbow_uniform for a range centred on zero) and reverse; the colour range (range: field, the whole field's min to max, the default; shown, what is drawn now; symmetric, -max|v| to +max|v|; fixed, min to max, a bound left out keeping the range shown now). reset starts from the default style. Each field keeps its own style.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "field": {"type": "string"},
                    "reset": {"type": "boolean"},
                    "mode": {"enum": ["flat", "smooth", "banded"]},
                    "bands": {"type": "integer", "minimum": 2, "maximum": 64},
                    "lines": {"type": "boolean"},
                    "line_count": {"type": "integer", "minimum": 1, "maximum": 64},
                    "map": {"type": "string", "description": "auto, or a colour map id (field.list)"},
                    "reverse": {"type": "boolean"},
                    "range": {"enum": ["field", "shown", "symmetric", "fixed"]},
                    "min": {"type": "number"},
                    "max": {"type": "number"}
                }),
                &[],
            )
        },
        run: field_style,
    },
    Tool {
        id: "display.threshold",
        version: 1,
        action: Action::Read,
        label_key: "tool-display-threshold",
        summary: "Hide the cells whose quality metric is below value, so only the bad cells and the faces around them show; null shows them again. Switches the colouring to quality (and to metric, if given).",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "value": {"oneOf": [{"type": "number"}, {"type": "null"}]},
                    "metric": {"enum": ["non_orthogonality", "skewness", "volume_ratio", "face_count"]}
                }),
                &["value"],
            )
        },
        run: display_threshold,
    },
    Tool {
        id: "display.set_color",
        version: 1,
        action: Action::Read,
        label_key: "tool-display-set-color",
        summary: "Set the colour (#RRGGBB) of an item of the open mesh (mesh, zone:<name>, kind:<cell type>, patch:<name>, iso:<n>, arrows, stream:<n>), or null to go back to the colour mode's own; the mesh and zone colours show in solid, a zone colour also in zone, a patch colour in solid and patch, a cell-type colour in cell_type, an iso-surface's, the arrows' and a streamline set's colours when they are solid.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "item": {"type": "string"},
                    "color": {"oneOf": [{"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"}, {"type": "null"}]}
                }),
                &["item"],
            )
        },
        run: display_set_color,
    },
    Tool {
        id: "display.visibility",
        version: 1,
        action: Action::Read,
        label_key: "tool-display-visibility",
        summary: "Show, hide or isolate items by id (mesh, zone:<name>, kind:<cell type>, patch:<name>, cell:<n>, face:<n>), or show all; isolating cells or faces shows only them. Returns the hidden and isolated items and the visible counts.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "action": {"enum": ["show", "hide", "isolate", "show_all"]},
                    "items": {"type": "array", "items": {"type": "string"}}
                }),
                &["action"],
            )
        },
        run: display_visibility,
    },
    Tool {
        id: "clip.set",
        version: 2,
        action: Action::Read,
        label_key: "tool-clip-set",
        summary: "Set a clip plane (plane: its number, clip:1 is the first; left out: the current plane, the one selected last or else added last; new: add one, up to 8): its normal by axis (x, y, z or view; the half nearer the eye is cut away, so the cut faces the camera) or as a vector, its place by origin or by offset along the normal from the centre of the mesh, the mode (crinkle keeps whole cells, flat slices them), flip (swap the side cut away) and enabled. The side the normal points to is cut away; fields left out keep their value (a new plane: x through the centre, crinkle). A cell shows when every enabled plane keeps it; flat planes slice in the order they were added.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "axis": {"enum": ["x", "y", "z", "view"]},
                    "normal": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
                    "origin": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
                    "offset": {"type": "number"},
                    "mode": {"enum": ["crinkle", "flat"]},
                    "flip": {"type": "boolean"},
                    "enabled": {"type": "boolean"},
                    "plane": {"type": "integer", "minimum": 1},
                    "new": {"type": "boolean"}
                }),
                &[],
            )
        },
        run: clip_set,
    },
    Tool {
        id: "clip.rotate",
        version: 1,
        action: Action::Read,
        label_key: "tool-clip-rotate",
        summary: "Tilt a clip plane (plane: its number; left out: the current plane) by deg degrees about an axis through pivot (right-hand rule). The axis is a screen axis (x right, y up, z towards the viewer, as in view.rotate) or a world vector; the pivot defaults to the centre of the plane inside the mesh's box, so the plane stays in the mesh. The window's tilt arcs and Alt + arrow keys call it.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "axis": {"oneOf": [
                        {"enum": ["x", "y", "z"]},
                        {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3}
                    ]},
                    "deg": {"type": "number"},
                    "pivot": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
                    "plane": {"type": "integer", "minimum": 1}
                }),
                &["axis", "deg"],
            )
        },
        run: clip_rotate,
    },
    Tool {
        id: "clip.clear",
        version: 2,
        action: Action::Read,
        label_key: "tool-clip-clear",
        summary: "Remove a clip plane (plane: its number; left out: the current plane), or all of them (all); the planes after it move up one.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({"plane": {"type": "integer", "minimum": 1}, "all": {"type": "boolean"}}),
                &[],
            )
        },
        run: clip_clear,
    },
    Tool {
        id: "slice.set",
        version: 1,
        action: Action::Read,
        label_key: "tool-slice-set",
        summary: "Add or change a slice (slice: its number, slice:1 is the first; left out: the current slice, the one selected last or else added last; new: add one, up to 8): a plane through the cells shown that cuts nothing away and draws their sections in the colours of the mesh (the field shown in its style, or the colour mode's cell colours). Its normal by axis (x, y, z or view) or as a vector, its place by origin or by offset along the normal from the centre of the mesh, flip and enabled, as for clip.set. A series: count planes (1 to 64) spacing apart along the normal, the middle one at the origin; a count without spacing spreads them evenly through the mesh. While slices or iso-surfaces show, the mesh draws as a faint outline (display.mesh_look). Clip planes cut slices too.",
        outputs: &[],
        reads: &[],
        input_schema: crate::extract::slice_set_schema,
        run: crate::extract::slice_set,
    },
    Tool {
        id: "slice.rotate",
        version: 1,
        action: Action::Read,
        label_key: "tool-slice-rotate",
        summary: "Tilt a slice (slice: its number; left out: the current slice) by deg degrees about an axis through pivot, as clip.rotate tilts a clip plane: a screen axis (x right, y up, z towards the viewer) or a world vector; the pivot defaults to the centre of the plane inside the mesh's box. The window's tilt arcs call it.",
        outputs: &[],
        reads: &[],
        input_schema: crate::extract::slice_rotate_schema,
        run: crate::extract::slice_rotate,
    },
    Tool {
        id: "slice.clear",
        version: 1,
        action: Action::Read,
        label_key: "tool-slice-clear",
        summary: "Remove a slice (slice: its number; left out: the current slice), or all of them (all); the slices after it move up one.",
        outputs: &[],
        reads: &[],
        input_schema: crate::extract::slice_clear_schema,
        run: crate::extract::slice_clear,
    },
    Tool {
        id: "iso.set",
        version: 1,
        action: Action::Read,
        label_key: "tool-iso-set",
        summary: "Add or change an iso-surface (iso: its number, iso:1 is the first; left out: the current one; new: add one, up to 8): the surface where field (with component for a vector; left out: its own field, else the field shown, else the file's first) takes value (left out: its own value, or the middle of the field's range for a new one or a new field). color solid gives it a colour of its own (the patch hues in turn, or display.set_color on iso:<n>); field colours it by the field on the colour bar in that field's style. Made by marching tetrahedra in every cell shown (cell centroid, face centres and nodes), so it closes across cells; clip planes cut it.",
        outputs: &[],
        reads: &[],
        input_schema: crate::extract::iso_set_schema,
        run: crate::extract::iso_set,
    },
    Tool {
        id: "iso.clear",
        version: 1,
        action: Action::Read,
        label_key: "tool-iso-clear",
        summary: "Remove an iso-surface (iso: its number; left out: the current one), or all of them (all); those after it move up one.",
        outputs: &[],
        reads: &[],
        input_schema: crate::extract::iso_clear_schema,
        run: crate::extract::iso_clear,
    },
    Tool {
        id: "arrows.set",
        version: 1,
        action: Action::Read,
        label_key: "tool-arrows-set",
        summary: "Show or change the vector arrows: shaded 3D arrows of a vector field (field: left out, their own, else the field shown if it is a vector, else the file's first vector) on every slice shown, on an even square grid anchored at the mesh's centre, and on the mesh surface shown when surface is true (one boundary face per cube of the grid). density is the arrows across the mesh's largest extent (4 to 200, 25 by default); the length follows the magnitude, the field's largest being 0.9 of a spacing times scale (above 0, up to 100). in_plane draws only the part of the vector in each slice's plane, the longest of those then 0.9 of a spacing. color solid gives them a colour of their own (display.set_color on arrows), field the colour of the field on the colour bar there. enabled false keeps the settings and draws none. At most 10 000 are drawn (capped in the result).",
        outputs: &[],
        reads: &[],
        input_schema: crate::extract::arrows_set_schema,
        run: crate::extract::arrows_set,
    },
    Tool {
        id: "arrows.clear",
        version: 1,
        action: Action::Read,
        label_key: "tool-arrows-clear",
        summary: "Remove the vector arrows and their settings.",
        outputs: &[],
        reads: &[],
        input_schema: crate::extract::arrows_clear_schema,
        run: crate::extract::arrows_clear,
    },
    Tool {
        id: "stream.set",
        version: 1,
        action: Action::Read,
        label_key: "tool-stream-set",
        summary: "Add or change a set of streamlines (stream: its number, stream:1 is the first; left out: the current set, the one selected last or else added last; new: add one, up to 8). The lines follow a vector field (field: left out, the set's own, else the field shown if it is a vector, else the file's first vector) through the cells' tetrahedra (the field slices show) by fourth-order Runge-Kutta in arc length, a quarter of a cell per step, forward, backward or both ways from each seed (direction, both by default), and stop where they leave the mesh (or the regions shown), where the speed falls below 0.1 % of the field's largest, at max_length (a length, or null for three mesh diagonals) or after 10 000 steps. seed: line (from, to: count seeds evenly, ends included; a new line lies across the mesh near the low end of its longest side), sphere (centre, radius: its centre and Halton points through the ball), patch (patch: a boundary patch's name, its faces thinned evenly to about count; a new one takes the first inlet), plane (origin, normal: an even grid of about count points inside the mesh). Values of another shape than the set's are refused unless seed names that shape. count: seeds wanted, 1 to 1000 (50 by default); seeds outside the mesh are left out. look: lines (thin lines, the default) or tubes (shaded, tube_radius a length or null for 1/400 of the mesh diagonal); arrowheads: cones pointing downstream along the lines. color: field (the default: color_field and component, left out the traced field's magnitude, with a colour bar of its own in that field's style, field.style) or solid (display.set_color on stream:<n>). enabled false keeps the settings and draws none. While streamlines show, the mesh draws as its faint outline unless display.mesh_look asks for faces. The result's stream says how many seeds were placed and how the lines ended.",
        outputs: &[],
        reads: &[],
        input_schema: crate::stream::stream_set_schema,
        run: crate::stream::stream_set,
    },
    Tool {
        id: "stream.clear",
        version: 1,
        action: Action::Read,
        label_key: "tool-stream-clear",
        summary: "Remove a set of streamlines (stream: its number; left out: the current set), or all of them (all); those after it move up one.",
        outputs: &[],
        reads: &[],
        input_schema: crate::stream::stream_clear_schema,
        run: crate::stream::stream_clear,
    },
    Tool {
        id: "parcels.open",
        version: 1,
        action: Action::Read,
        label_key: "tool-parcels-open",
        summary: "Show the spray parcels of a parcel file with the open mesh: FoehnCraft's spray/parcels_<step>.vtkhdf (VTKHDF PolyData: points with velocity, diameter, droplets, age, parcel_id and later temperature and injector as point data; time and step as attributes) or any VTKHDF PolyData of points. Each parcel is a small shaded sphere. A file opened while parcels show keeps their settings (the next file of a run looks the same); the first one is coloured by diameter (else its first field) with a colour bar of its own. While parcels show, the mesh draws as its faint outline unless display.mesh_look asks for faces. The result's parcels lists the file's time, step, count, the parcels drawn and each field's range.",
        outputs: &[],
        reads: &["path"],
        input_schema: crate::parcel::parcels_open_schema,
        run: crate::parcel::parcels_open,
    },
    Tool {
        id: "parcels.set",
        version: 1,
        action: Action::Read,
        label_key: "tool-parcels-set",
        summary: "Change how the parcels show. color: a parcel field (component: magnitude, x, y or z of a vector) or solid (display.set_color on parcels); its colour bar keeps its own map (map: a colour map id or auto), reverse and range (field: all parcels of the file, the default; shown: the parcels drawn; symmetric; fixed with min and max), apart from the gas field of the same name. size: fixed (every parcel 1/400 of the mesh diagonal times scale) or diameter (the largest droplet diameter that size, the others in proportion); scale above 0 up to 100. Filters: diameter [min, max] in metres and injector (a number, needs the file's injector field), null for all. Parcels on the cut-away side of a clip plane are left out. At most 200 000 are drawn, thinned evenly beyond (capped in the result). enabled false keeps the settings and draws none. A field the file lacks is refused with the list of its fields.",
        outputs: &[],
        reads: &[],
        input_schema: crate::parcel::parcels_set_schema,
        run: crate::parcel::parcels_set,
    },
    Tool {
        id: "parcels.clear",
        version: 1,
        action: Action::Read,
        label_key: "tool-parcels-clear",
        summary: "Remove the parcels and their settings.",
        outputs: &[],
        reads: &[],
        input_schema: crate::parcel::parcels_clear_schema,
        run: crate::parcel::parcels_clear,
    },
    Tool {
        id: "series.open",
        version: 1,
        action: Action::Read,
        label_key: "tool-series-open",
        summary: "Open a time series of one run and show its first step (or step): FoehnCraft's series.json (format foehncraft.series 1: entries of step, time and files fields and parcels, relative to its folder; other keys ignored) or an output folder (its series.json, else its fields_<step>.vtkhdf and spray/parcels_<step>.vtkhdf, matched by step). Each step opens its fields file as the next time step of the open mesh (same cells: the display, derived fields, camera and journal stay, only the values change) and its parcel file with the parcel settings kept; a step without a fields file keeps the mesh shown, one without parcels draws none. The result's series gives the count, the index, step and time shown, its files, the first and last step and time, and the playback. Opening another mesh ends the series.",
        outputs: &[],
        reads: &["path"],
        input_schema: crate::series::series_open_schema,
        run: crate::series::series_open,
    },
    Tool {
        id: "series.go",
        version: 1,
        action: Action::Read,
        label_key: "tool-series-go",
        summary: "Show another step of the open series: index (from 0), step, time (the nearest entry with a time), by (entries on, negative back; wraps round while the player loops, else holds at the ends) or to first or last; exactly one. A step the series lacks is refused with the steps it has.",
        outputs: &[],
        reads: &[],
        input_schema: crate::series::series_go_schema,
        run: crate::series::series_go,
    },
    Tool {
        id: "series.play",
        version: 1,
        action: Action::Read,
        label_key: "tool-series-play",
        summary: "Play or pause the open series in the window (playing), at rate steps a second (0.1 to 60, default 4), looping or stopping at the last step (loop, default on). The window's player bar and the status bar show it; on the command line and for scripts the setting is kept and steps are made with series.go, and export.movie with motion series writes a movie of the steps.",
        outputs: &[],
        reads: &[],
        input_schema: crate::series::series_play_schema,
        run: crate::series::series_play,
    },
    Tool {
        id: "region.set",
        version: 1,
        action: Action::Read,
        label_key: "tool-region-set",
        summary: "Add or change a region of interest (region: its number, region:1 is the first; left out: the current region, the one selected last or else added last; new: add one, up to 16). While a region is enabled, only the cells of the enabled regions show and slices, iso-surfaces, arrows and quality are computed only there; several enabled regions show their union. shape: box (center, size: full lengths along x, y, z; a new one is half the mesh's box), sphere (center, radius), slab (the cells between two planes thickness apart, the middle one through origin with normal), zone (zone: a cell zone's name), patch (patch: its name, layers: the cells along it and layers - 1 rings further in, 1 to 64), cells (cells: ids, left out: the cells picked now; rings: rings of face neighbours added, 0 to 64). A cell belongs to a box, sphere or slab when its bounding box touches the shape, so the shape is fully covered (also when it is thinner than a cell) and the border shows whole cells. cut: crinkle (the default, whole cells) or flat (box, sphere and slab only: the border cells are clipped to the shape and closed by their sections, as a flat clip plane cuts; a box's and a slab's cut is exact, a sphere's follows it to within a cell, and a sphere with no mesh node inside shows whole cells, with a note; a cell a whole-cell region takes stays whole, and where flat regions overlap a cell is cut by the one its centre lies deepest in). Changing shape replaces the region's shape; fields of another shape are refused. enabled false keeps the region but shows the whole mesh. The result lists each region with the cells it takes, its halo (the cells around it a mesh read in pieces also loads) and the size of that part of the mesh model.",
        outputs: &[],
        reads: &[],
        input_schema: crate::region::region_set_schema,
        run: crate::region::region_set,
    },
    Tool {
        id: "region.clear",
        version: 1,
        action: Action::Read,
        label_key: "tool-region-clear",
        summary: "Remove a region (region: its number; left out: the current region), or all of them (all); the regions after it move up one. With no region enabled the whole mesh shows.",
        outputs: &[],
        reads: &[],
        input_schema: crate::region::region_clear_schema,
        run: crate::region::region_clear,
    },
    Tool {
        id: "region.list",
        version: 1,
        action: Action::Read,
        label_key: "tool-region-list",
        summary: "The regions of interest: each with its shape, whether it is enabled, and what it takes (cells, halo cells, points and the size of that part of the mesh model in MiB); shown is what the enabled regions take together (null while none is enabled and the whole mesh shows).",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: crate::region::region_list,
    },
    Tool {
        id: "blocks.make",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-blocks-make",
        summary: "Write a mesh and its fields as a blocked VTKHDF file (docs/blocked-vtkhdf.md, layout 1), the layout from which MurmelCraft reads a region of a large mesh without reading it all: the cells sorted along a 3-D Hilbert curve and cut into blocks of block_cells owned cells (default 65536, 16384 to 1048576), each block a partition with its own points and one layer of ghost cells (the point neighbours), global point and cell ids, the bounds and field ranges of every block, and the boundary patches as PolyData with their owner cells. mesh: the mesh to convert (a .vtu, an OpenFOAM case or polyMesh folder, or a VTKHDF file; it is read, not opened); left out, the open mesh and its fields. The file is an ordinary VTKHDF 2.4 file that ParaView 5.13 and later open; it opens whole with mesh.open. The result counts the blocks, ghost cells, linear cells and polyhedra written.",
        outputs: &["out"],
        reads: &["mesh"],
        input_schema: crate::blocks::blocks_make_schema,
        run: crate::blocks::blocks_make,
    },
    Tool {
        id: "display.mesh_look",
        version: 1,
        action: Action::Read,
        label_key: "tool-display-mesh-look",
        summary: "What the mesh looks like while slices or iso-surfaces show: outline (the default: its feature edges only, faint, so what is inside stands out; a click then picks the slices) or faces (as the render mode says).",
        outputs: &[],
        reads: &[],
        input_schema: crate::extract::mesh_look_schema,
        run: crate::extract::mesh_look,
    },
    Tool {
        id: "select.item",
        version: 2,
        action: Action::Read,
        label_key: "tool-select-item",
        summary: "Select an item by id (mesh, zone:<name>, kind:<cell type>, patch:<name>, cell:<n>, face:<n>); add puts it beside the selection (Shift; again takes it out). Its faces are tinted, a cell or face is outlined, and inspect.item shows the last one selected.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({"item": {"type": "string"}, "add": {"type": "boolean"}}),
                &["item"],
            )
        },
        run: select_item,
    },
    Tool {
        id: "select.pick",
        version: 1,
        action: Action::Read,
        label_key: "tool-select-pick",
        summary: "Select the cell (or, with target face, the face) under a screen point ndc (-1..1, y up), as drawn now (clip and hidden items included); add keeps the selection (Shift). A section of a flat cut picks its cell. Nothing under the point clears the selection unless add. Returns hit and picked.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "ndc": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2},
                    "add": {"type": "boolean"},
                    "target": {"enum": ["cell", "face"]}
                }),
                &["ndc"],
            )
        },
        run: select_pick,
    },
    Tool {
        id: "select.filter",
        version: 1,
        action: Action::Read,
        label_key: "tool-select-filter",
        summary: "Choose what a click in the viewport picks: cell (the default) or face.",
        outputs: &[],
        reads: &[],
        input_schema: || object(json!({"target": {"enum": ["cell", "face"]}}), &["target"]),
        run: select_filter,
    },
    Tool {
        id: "inspect.at",
        version: 1,
        action: Action::Read,
        label_key: "tool-inspect-at",
        summary: "What lies under a screen point ndc (-1..1, y up), without selecting: the point, the cell with its type and zone, the face with its patch and role (null when nothing is there). The hover tooltip and the cursor coordinates use it.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({"ndc": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2}}),
                &["ndc"],
            )
        },
        run: inspect_at,
    },
    Tool {
        id: "inspect.quality",
        version: 1,
        action: Action::Read,
        label_key: "tool-inspect-quality",
        summary: "Statistics of a cell quality metric over all cells (the current one when left out): min, max, mean and a histogram of bins equal-width bins (default 20); with above, the number of cells at or above that value.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "metric": {"enum": ["non_orthogonality", "skewness", "volume_ratio", "face_count"]},
                    "bins": {"type": "integer", "minimum": 1, "maximum": 200},
                    "above": {"type": "number"}
                }),
                &[],
            )
        },
        run: inspect_quality,
    },
    Tool {
        id: "select.clear",
        version: 1,
        action: Action::Read,
        label_key: "tool-select-clear",
        summary: "Clear the selection.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: select_clear,
    },
    Tool {
        id: "inspect.item",
        version: 1,
        action: Action::Read,
        label_key: "tool-inspect-item",
        summary: "The property rows of an item (the selection, or the mesh when nothing is selected): the same data as the Properties panel; with results, the field values of a cell, a boundary face (its boundary values and its cell's) or a probe (interpolated at its point).",
        outputs: &[],
        reads: &[],
        input_schema: || object(json!({"item": {"type": "string"}}), &[]),
        run: inspect_item,
    },
    Tool {
        id: "journal.undo",
        version: 1,
        action: Action::Read,
        label_key: "tool-journal-undo",
        summary: "Undo the last step of the camera or the display (a call of a journalled tool, or one group of calls such as a mouse drag); the selection stays. undone is the tool of the step, null when there was none.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: journal_undo,
    },
    Tool {
        id: "journal.redo",
        version: 1,
        action: Action::Read,
        label_key: "tool-journal-redo",
        summary: "Redo the last undone step; a new step after an undo drops the steps to redo. redone is the tool of the step, null when there was none.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: journal_redo,
    },
    Tool {
        id: "journal.group",
        version: 1,
        action: Action::Read,
        label_key: "tool-journal-group",
        summary: "Open or close a group: the calls inside one group undo as one step (the window groups each mouse gesture and each run of wheel steps).",
        outputs: &[],
        reads: &[],
        input_schema: || object(json!({"open": {"type": "boolean"}}), &["open"]),
        run: journal_group,
    },
    Tool {
        id: "journal.list",
        version: 1,
        action: Action::Read,
        label_key: "tool-journal-list",
        summary: "The steps to undo and to redo (oldest first, by tool), whether a group is open, and how many steps are kept.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: journal_list,
    },
    Tool {
        id: "settings.get",
        version: 1,
        action: Action::Read,
        label_key: "tool-settings-get",
        summary: "The theme, the mesh colours, the navigation bindings, the view cube placement, whether agents may drive the window, the presets, your defaults for new items (and which values of each kind can be defaults), your formulas and the config folder.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: settings_get,
    },
    Tool {
        id: "settings.set_theme",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-settings-set-theme",
        summary: "Switch the theme live (dark, light, system) and write settings.toml in the config folder.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({"theme": {"enum": ["dark", "light", "system"]}}),
                &["theme"],
            )
        },
        run: settings_set_theme,
    },
    Tool {
        id: "settings.set_colors",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-settings-set-colors",
        summary: "Set the user's mesh colours (#RRGGBB) that replace the built-in ones for every mesh: solid (the default mesh colour), the patch palette, role and cell-type colours, edge darkening, the quality ramp (good to bad), the clip handles per theme (gizmo.dark and gizmo.light: slide arrow and tilt arcs), the measurements per theme (measure.dark and measure.light: line, probe, halo); reset goes back to the built-in colours first. Writes settings.toml in the config folder.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "solid": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"},
                    "patch": {"type": "array", "items": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"}, "minItems": 1},
                    "role": {"type": "object", "propertyNames": {"enum": ["wall", "inlet", "outlet", "symmetry", "interface", "unspecified"]}, "additionalProperties": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"}},
                    "kind": {"type": "object", "propertyNames": {"enum": ["polyhedron", "prism", "hexahedron", "tetrahedron", "pyramid"]}, "additionalProperties": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"}},
                    "edge_darken": {"type": "number", "minimum": 0, "maximum": 1},
                    "quality": {"type": "array", "items": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"}, "minItems": 2},
                    "gizmo": {"type": "object", "additionalProperties": false, "properties": {
                        "dark": {"type": "object", "propertyNames": {"enum": ["slide", "tilt"]}, "additionalProperties": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"}},
                        "light": {"type": "object", "propertyNames": {"enum": ["slide", "tilt"]}, "additionalProperties": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"}}
                    }},
                    "measure": {"type": "object", "additionalProperties": false, "properties": {
                        "dark": {"type": "object", "propertyNames": {"enum": ["line", "probe", "halo"]}, "additionalProperties": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"}},
                        "light": {"type": "object", "propertyNames": {"enum": ["line", "probe", "halo"]}, "additionalProperties": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"}}
                    }},
                    "reset": {"type": "boolean"}
                }),
                &[],
            )
        },
        run: settings_set_colors,
    },
    Tool {
        id: "settings.set_view_cube",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-settings-set-view-cube",
        summary: "Place the view cube: a corner of the viewport (top_right, top_left, bottom_left, bottom_right), a floating panel dragged anywhere over it (position: top left in pixels from the viewport's top left), or hidden; auto_hide hides it while the pointer is farther than distance_px (0 to 2000) from it. Applied live; writes settings.toml in the config folder.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "placement": {"enum": ["top_right", "top_left", "bottom_left", "bottom_right", "floating", "hidden"]},
                    "auto_hide": {"type": "boolean"},
                    "distance_px": {"type": "number", "minimum": 0, "maximum": 2000},
                    "position": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2}
                }),
                &[],
            )
        },
        run: settings_set_view_cube,
    },
    Tool {
        id: "settings.set_agents",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-settings-set-agents",
        summary: "Let agents drive the open window (murmel mcp --live) or stop them: off closes the window's socket and every agent's connection. Applied live; writes settings.toml in the config folder.",
        outputs: &[],
        reads: &[],
        input_schema: || object(json!({"enabled": {"type": "boolean"}}), &["enabled"]),
        run: settings_set_agents,
    },
    Tool {
        id: "settings.set_default",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-settings-set-default",
        summary: "Keep values as your defaults: new items of their kind start from them (arrows, slice, iso, stream, clip, region), a field shown for the first time takes the default field style (field_style), and the window starts with the default view (view: render_mode, mesh_look, projection); items already open keep theirs. Either kind with values ({\"density\": 60}), or item (an item's id, such as arrows or slice:1) with keys to take from it (left out: every value its kind can keep), or kind view or field_style (with field, else the field shown) to take the values now. Only an item's look, density, count and modes can be defaults, never where it is or its field (settings.get lists them per kind); a value at its built-in default forgets the default. Writes settings.toml in the config folder.",
        outputs: &[],
        reads: &[],
        input_schema: crate::defaults::set_default_schema,
        run: crate::defaults::settings_set_default,
    },
    Tool {
        id: "settings.reset_default",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-settings-reset-default",
        summary: "Forget your defaults: one value (kind and key), every value of a kind (kind), or all (no input), so new items start from the built-in values again. Items already open keep theirs. Writes settings.toml in the config folder.",
        outputs: &[],
        reads: &[],
        input_schema: crate::defaults::reset_default_schema,
        run: crate::defaults::settings_reset_default,
    },
    Tool {
        id: "settings.keep_formula",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-settings-keep-formula",
        summary: "Keep a formula among your formulas (name, formula, unit; as field.calculate takes them), offered for every result: the Derived fields window lists them and field.calculate computes one by its name alone. A formula of the same name is replaced. The formula is checked for its grammar, not for the fields it names (a result without them refuses it when computed). Writes settings.toml in the config folder.",
        outputs: &[],
        reads: &[],
        input_schema: crate::derive::keep_formula_schema,
        run: crate::derive::settings_keep_formula,
    },
    Tool {
        id: "settings.forget_formula",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-settings-forget-formula",
        summary: "Forget one of your formulas, by name. A field already computed from it stays. Writes settings.toml in the config folder.",
        outputs: &[],
        reads: &[],
        input_schema: crate::derive::forget_formula_schema,
        run: crate::derive::settings_forget_formula,
    },
    Tool {
        id: "settings.set_bindings",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-settings-set-bindings",
        summary: "Choose a navigation preset (three_button, apple, fusion, paraview) or a custom binding table and write navigation.toml in the config folder.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "preset": {"enum": PRESETS},
                    "bindings": {"type": "array", "items": {
                        "type": "object",
                        "properties": {
                            "action": {"enum": ["orbit", "pan", "zoom", "roll", "pick", "context_menu", "fit"]},
                            "gesture": {"enum": ["drag", "click", "double_click", "scroll", "pinch"]},
                            "button": {"enum": ["primary", "secondary", "middle"]},
                            "mods": {"type": "object", "properties": {
                                "cmd": {"type": "boolean"}, "shift": {"type": "boolean"},
                                "ctrl": {"type": "boolean"}, "alt": {"type": "boolean"}
                            }}
                        },
                        "required": ["action", "gesture"]
                    }},
                    "zoom_to_cursor": {"type": "boolean"},
                    "invert_zoom": {"type": "boolean"},
                    "orbit_speed": {"type": "number"},
                    "zoom_speed": {"type": "number"}
                }),
                &[],
            )
        },
        run: settings_set_bindings,
    },
    Tool {
        id: "session.save",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-session-save",
        summary: "Write the session to a .murmel file (JSON, format murmelcraft.session): the mesh (path relative to the file and absolute, SHA-256), the camera, the colour and render mode, the quality metric and threshold, the pick target, the hidden, isolated and selected items and item colours by id, the clip plane, the saved views, the field shown and each field's style, the mesh colours and the view cube placement.",
        outputs: &["path"],
        reads: &[],
        input_schema: || {
            object(
                json!({"path": {"type": "string", "description": "The session file to write, usually *.murmel"}}),
                &["path"],
            )
        },
        run: session_save,
    },
    Tool {
        id: "session.load",
        version: 1,
        action: Action::Read,
        label_key: "tool-session-load",
        summary: "Load a .murmel session file: open its mesh unless that mesh is already open (tried relative to the file first, then at its absolute path), then restore the camera, the display, the selection and the clip plane; the mesh colours and the view cube placement apply to this session and are not written to settings.toml. A mesh changed since the save (SESS003) or items it no longer has (SESS004) give warnings. A new journal starts.",
        outputs: &[],
        reads: &["path"],
        input_schema: || {
            object(
                json!({"path": {"type": "string", "description": "A session file (*.murmel)"}}),
                &["path"],
            )
        },
        run: session_load,
    },
    Tool {
        id: "measure.snap",
        version: 1,
        action: Action::Read,
        label_key: "tool-measure-snap",
        summary: "The point a click at a screen point would measure from: the face under it gives its nodes, edge midpoints, centre (and its cell's centre), projected to the screen; the first kind in the order node, edge_midpoint, face_centre, cell_centre, edge, surface with a candidate within radius_px (default 12) wins; snap lists the kinds allowed (default all but cell_centre). Null over empty space. Changes nothing.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            let a = anchor_schema();
            object(
                json!({
                    "ndc": a["properties"]["ndc"],
                    "snap": a["properties"]["snap"],
                    "radius_px": a["properties"]["radius_px"],
                }),
                &["ndc"],
            )
        },
        run: measure_snap,
    },
    Tool {
        id: "measure.add",
        version: 1,
        action: Action::Read,
        label_key: "tool-measure-add",
        summary: "Measure in the open mesh and keep the result as a tree item measure:<n>: a distance between two points (with dx, dy, dz) or the angle at the second of three points (degrees). Each point is a screen point snapped as measure.snap does, typed coordinates, a node, the midpoint of two nodes, a face centre or a cell centre. Kept in session files; undo removes it.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "kind": {"enum": ["distance", "angle"]},
                    "points": {"type": "array", "items": anchor_schema(), "minItems": 2, "maxItems": 3},
                    "name": {"type": "string", "minLength": 1, "maxLength": VIEW_NAME_MAX},
                }),
                &["kind", "points"],
            )
        },
        run: measure_add,
    },
    Tool {
        id: "measure.rename",
        version: 1,
        action: Action::Read,
        label_key: "tool-measure-rename",
        summary: "Name a measurement or a probe (measure:<n>, probe:<n>); a null name goes back to \"Distance n\", \"Angle n\" or \"Probe n\".",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "item": {"type": "string"},
                    "name": {"type": ["string", "null"], "minLength": 1, "maxLength": VIEW_NAME_MAX},
                }),
                &["item"],
            )
        },
        run: measure_rename,
    },
    Tool {
        id: "measure.remove",
        version: 1,
        action: Action::Read,
        label_key: "tool-measure-remove",
        summary: "Remove measurements and probes: the items named (measure:<n>, probe:<n>), or all of them. Undo brings them back.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "items": {"type": "array", "items": {"type": "string"}},
                    "all": {"type": "boolean"},
                }),
                &[],
            )
        },
        run: measure_remove,
    },
    Tool {
        id: "measure.list",
        version: 1,
        action: Action::Read,
        label_key: "tool-measure-list",
        summary: "The measurements (value, unit, points and what they snapped to) and the probes (point, cell and what it reads) of the open mesh.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: measure_list,
    },
    Tool {
        id: "probe.add",
        version: 1,
        action: Action::Read,
        label_key: "tool-probe-add",
        summary: "Place a probe (tree item probe:<n>) that reads the cell holding a point: the point, the cell, its type, zone, centroid and the distance to it, its volume and worst non-orthogonality and skewness, and with results every field's value at the point (inverse distance weighted from the cell's nodes). The point is given as for measure.add; typed coordinates are located in the mesh (generalised winding number). Kept in session files; undo removes it.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "at": anchor_schema(),
                    "name": {"type": "string", "minLength": 1, "maxLength": VIEW_NAME_MAX},
                }),
                &["at"],
            )
        },
        run: probe_add,
    },
    Tool {
        id: "probe.at",
        version: 1,
        action: Action::Read,
        label_key: "tool-probe-at",
        summary: "What a probe at a point would read, without keeping it: the point given as for measure.add, the cell holding it and that cell's values. Changes nothing.",
        outputs: &[],
        reads: &[],
        input_schema: || object(json!({"at": anchor_schema()}), &["at"]),
        run: probe_at,
    },
];
