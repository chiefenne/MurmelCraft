//! The tools. Inputs are typed structs (unknown fields refused);
//! each tool has a JSON Schema of its input for the catalogue.

use std::path::PathBuf;

use murmel_engine::display::{SavedView, VIEW_NAME_MAX, all_items, view_name};
use murmel_engine::navigation::{Binding, Navigation, PRESETS};
use murmel_engine::session_file::{SessionFile, absolute};
use murmel_engine::settings::{Agents, Settings, save_navigation, save_settings};
use murmel_engine::{
    Clip, ClipMode, ColorBy, CubeSettings, Item, MeshColors, Metric, PickTarget, Placement,
    Projection, Quality, RenderMode, Rgb, ScreenAxis, Session, StandardView, Theme, Visibility,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{Action, Diagnostic, Run, Tool};

fn parse<T: DeserializeOwned>(input: Value) -> Result<T, Diagnostic> {
    serde_json::from_value(input).map_err(|e| Diagnostic::new("TOOL002", e.to_string()))
}

fn mesh_err(e: murmel_mesh::MeshError) -> Diagnostic {
    Diagnostic::new(e.code, e.message)
}

fn object(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

fn no_input() -> Value {
    object(json!({}), &[])
}

fn camera_json(s: &Session) -> Value {
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

fn need_mesh(s: &Session) -> Result<(), Diagnostic> {
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
        "format": if m.mesh.metadata.contains_key("openfoam_format") { "openfoam" } else { "vtu" },
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

fn finite(v: f64, name: &str) -> Result<f64, Diagnostic> {
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
        .ok_or_else(|| Diagnostic::new("VIEW002", format!("no item {id} in the open mesh")))
}

fn display_json(s: &Session) -> Value {
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
        "hidden": d.hidden.iter().map(|i| i.id(&m.mesh)).collect::<Vec<_>>(),
        "selection": d.selection.iter().map(|i| i.id(&m.mesh)).collect::<Vec<_>>(),
        "isolated": d.isolated.iter().map(|i| i.id(&m.mesh)).collect::<Vec<_>>(),
        "pick": d.pick,
        "visible_cells": cells,
        "visible_faces": faces,
        "colors": colors,
        "quality": quality_json(s),
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
    }))
}

// ---- clip plane ---------------------------------------------------------

fn clip_json(s: &Session) -> Value {
    let (Some(c), Some(m)) = (&s.display.clip, &s.mesh) else {
        return Value::Null;
    };
    let (offset, range) = c.offset(&m.mesh.bounds());
    json!({
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
    let mut c = s.display.clip.unwrap_or(Clip {
        origin: bounds.center(),
        normal: [1.0, 0.0, 0.0],
        mode: ClipMode::Crinkle,
        enabled: true,
    });
    if s.display.clip.is_none() {
        c.enabled = true;
    }
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
    s.display.clip = Some(c);
    s.redraw();
    Ok(display_json(s))
}

fn clip_clear(_: Value, s: &mut Session) -> Run {
    s.display.clip = None;
    s.redraw();
    Ok(display_json(s))
}

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
fn save_session_settings(s: &Session) -> Result<PathBuf, Diagnostic> {
    let dir = config_dir(s)?;
    save_settings(
        &dir,
        &Settings {
            theme: s.theme,
            colors: s.colors.clone(),
            view_cube: s.view_cube.clone(),
            agents: Agents { enabled: s.agents },
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
            let open = s
                .mesh
                .as_ref()
                .is_some_and(|m| m.digest == saved.sha256 && same_path(&m.path, &p));
            if !open {
                s.open(&p).map_err(|e| {
                    Diagnostic::new(e.code, format!("the session's mesh: {}", e.message))
                })?;
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
    let missing = s.restore(f);
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
        "view_cube": s.view_cube,
        "skipped": missing,
        "warnings": warnings,
    }))
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
    "display.set_color",
    "display.visibility",
    "clip.set",
    "clip.clear",
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
    ("inspect.at", READS_ONLY),
    ("inspect.quality", READS_ONLY),
    ("inspect.item", READS_ONLY),
    ("settings.get", READS_ONLY),
    ("settings.set_theme", "settings are files, not undone"),
    ("settings.set_colors", "settings are files, not undone"),
    ("settings.set_view_cube", "settings are files, not undone"),
    ("settings.set_agents", "settings are files, not undone"),
    ("settings.set_bindings", "settings are files, not undone"),
    ("journal.undo", "walks the journal"),
    ("journal.redo", "walks the journal"),
    ("journal.group", "groups the steps"),
    ("journal.list", READS_ONLY),
    ("session.save", "writes a file; the view stays as it is"),
    ("session.load", "a loaded session starts a new journal"),
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
        version: 2,
        action: Action::Read,
        label_key: "tool-mesh-open",
        summary: "Load a mesh and the mesh-evidence.json beside it, replace the open mesh and fit the view; returns counts, zones, patches and the files read. `path` is a .vtu (KoflCraft's foehncraft.vtu) or an OpenFOAM mesh: a case, constant or polyMesh folder, a .foam file in the case or a polyMesh file (ascii, binary, gzip).",
        outputs: &[],
        reads: &["path"],
        input_schema: || {
            object(
                json!({"path": {"type": "string", "description": "A .vtu file, or an OpenFOAM case, constant or polyMesh folder (or a .foam file in the case)"}}),
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
        version: 2,
        action: Action::Read,
        label_key: "tool-display-color-by",
        summary: "Colour the boundary faces in one colour (solid, the default), by patch, by patch role, by the type of their cell, by the cell zone of their cell (zone; on the patch palette, zone colours set with display.set_color win), or by the quality of their cell (metric: non_orthogonality in degrees, skewness, volume_ratio or face_count; worst over the cell's faces, on a ramp over the metric's range). Changing the metric clears the threshold.",
        outputs: &[],
        reads: &[],
        input_schema: || {
            object(
                json!({
                    "by": {"enum": ["solid", "patch", "role", "cell_type", "zone", "quality"]},
                    "metric": {"enum": ["non_orthogonality", "skewness", "volume_ratio", "face_count"]}
                }),
                &["by"],
            )
        },
        run: display_color_by,
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
        summary: "Set the colour (#RRGGBB) of an item of the open mesh (mesh, zone:<name>, kind:<cell type>, patch:<name>), or null to go back to the colour mode's own; the mesh and zone colours show in solid, a zone colour also in zone, a patch colour in solid and patch, a cell-type colour in cell_type.",
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
        version: 1,
        action: Action::Read,
        label_key: "tool-clip-set",
        summary: "Set the clip plane: its normal by axis (x, y, z or view; the half nearer the eye is cut away, so the cut faces the camera) or as a vector, its place by origin or by offset along the normal from the centre of the mesh, the mode (crinkle keeps whole cells, flat slices them), flip (swap the side cut away) and enabled. The side the normal points to is cut away; fields left out keep their value (a new plane: x through the centre, crinkle).",
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
                    "enabled": {"type": "boolean"}
                }),
                &[],
            )
        },
        run: clip_set,
    },
    Tool {
        id: "clip.clear",
        version: 1,
        action: Action::Read,
        label_key: "tool-clip-clear",
        summary: "Remove the clip plane; the whole mesh shows again.",
        outputs: &[],
        reads: &[],
        input_schema: no_input,
        run: clip_clear,
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
        summary: "The property rows of an item (the selection, or the mesh when nothing is selected): the same data as the Properties panel.",
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
        summary: "The theme, the mesh colours, the navigation bindings, the view cube placement, whether agents may drive the window, the presets and the config folder.",
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
        summary: "Set the user's mesh colours (#RRGGBB) that replace the built-in ones for every mesh: solid (the default mesh colour), the patch palette, role and cell-type colours, edge darkening, the quality ramp (good to bad); reset goes back to the built-in colours first. Writes settings.toml in the config folder.",
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
        summary: "Write the session to a .murmel file (JSON, format murmelcraft.session): the mesh (path relative to the file and absolute, SHA-256), the camera, the colour and render mode, the quality metric and threshold, the pick target, the hidden, isolated and selected items and item colours by id, the clip plane, the saved views, the mesh colours and the view cube placement.",
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
];
