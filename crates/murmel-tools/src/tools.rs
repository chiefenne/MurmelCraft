//! The tools of V0.1. Inputs are typed structs (unknown fields refused);
//! each tool has a JSON Schema of its input for the catalogue.

use std::path::PathBuf;

use murmel_engine::navigation::{Binding, Navigation, PRESETS};
use murmel_engine::settings::{Settings, save_navigation, save_settings};
use murmel_engine::{Projection, Session, StandardView, Theme};
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
        "format": "vtu",
        "summary": m.mesh.summary(),
        "metadata": m.mesh.metadata,
        "evidence": m.evidence,
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
}

fn view_orbit(input: Value, s: &mut Session) -> Run {
    let i: Orbit = parse(input)?;
    s.camera.orbit(
        finite(i.yaw_deg, "yaw_deg")?,
        finite(i.pitch_deg, "pitch_deg")?,
    );
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

fn view_fit(_: Value, s: &mut Session) -> Run {
    need_mesh(s)?;
    s.fit();
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
    let dir = config_dir(s)?;
    let path = save_settings(
        &dir,
        &Settings {
            theme: i.theme,
            ..Settings::default()
        },
    )
    .map_err(|e| {
        Diagnostic::new(
            "SET001",
            format!("applied for this session, not saved: {e}"),
        )
    })?;
    Ok(json!({"theme": i.theme, "files": [path]}))
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
        "navigation": s.navigation,
        "presets": PRESETS,
        "config_dir": s.config_dir,
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
        input_schema: no_input,
        run: diag_codes,
    },
    Tool {
        id: "mesh.open",
        version: 1,
        action: Action::Read,
        label_key: "tool-mesh-open",
        summary: "Load a .vtu (KoflCraft's foehncraft.vtu) and the mesh-evidence.json beside it, replace the open mesh and fit the view; returns counts, zones and patches.",
        outputs: &[],
        input_schema: || object(json!({"path": {"type": "string"}}), &["path"]),
        run: mesh_open,
    },
    Tool {
        id: "mesh.close",
        version: 1,
        action: Action::Read,
        label_key: "tool-mesh-close",
        summary: "Drop the open mesh from the session.",
        outputs: &[],
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
        input_schema: no_input,
        run: mesh_info,
    },
    Tool {
        id: "view.get_camera",
        version: 1,
        action: Action::Read,
        label_key: "tool-view-get-camera",
        summary: "The camera: target, distance, yaw, pitch, projection, eye and viewport.",
        outputs: &[],
        input_schema: no_input,
        run: view_get,
    },
    Tool {
        id: "view.set_camera",
        version: 1,
        action: Action::Read,
        label_key: "tool-view-set-camera",
        summary: "Set any of target, distance, yaw, pitch, projection, field of view and viewport size.",
        outputs: &[],
        input_schema: || {
            object(
                json!({
                    "target": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
                    "distance": {"type": "number", "exclusiveMinimum": 0},
                    "yaw_deg": {"type": "number"},
                    "pitch_deg": {"type": "number", "minimum": -90, "maximum": 90},
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
        version: 1,
        action: Action::Read,
        label_key: "tool-view-orbit",
        summary: "Turn the camera about the up axis (yaw) and tilt it (pitch), in degrees.",
        outputs: &[],
        input_schema: || {
            object(
                json!({"yaw_deg": {"type": "number"}, "pitch_deg": {"type": "number"}}),
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
        summary: "Frame the open mesh from the current direction.",
        outputs: &[],
        input_schema: no_input,
        run: view_fit,
    },
    Tool {
        id: "view.standard",
        version: 1,
        action: Action::Read,
        label_key: "tool-view-standard",
        summary: "Go to a standard view (front, back, left, right, top, bottom, iso, home) and fit; face views snap to orthographic in perspective_ortho_faces.",
        outputs: &[],
        input_schema: || {
            object(
                json!({"view": {"enum": ["front", "back", "left", "right", "top", "bottom", "iso", "home"]}}),
                &["view"],
            )
        },
        run: view_standard,
    },
    Tool {
        id: "view.screenshot",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-view-screenshot",
        summary: "Render the current view offscreen (wgpu) to a PNG at path; mesh and view set up the session first for one-shot use.",
        outputs: &["path"],
        input_schema: || {
            object(
                json!({
                    "path": {"type": "string"},
                    "width": {"type": "integer", "minimum": 1, "maximum": 8192, "default": 1280},
                    "height": {"type": "integer", "minimum": 1, "maximum": 8192, "default": 800},
                    "mesh": {"type": "string"},
                    "view": {"enum": ["front", "back", "left", "right", "top", "bottom", "iso", "home"]},
                    "theme": {"enum": ["dark", "light", "system"]}
                }),
                &["path"],
            )
        },
        run: view_screenshot,
    },
    Tool {
        id: "settings.get",
        version: 1,
        action: Action::Read,
        label_key: "tool-settings-get",
        summary: "The theme, the navigation bindings, the presets and the config folder.",
        outputs: &[],
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
        input_schema: || {
            object(
                json!({"theme": {"enum": ["dark", "light", "system"]}}),
                &["theme"],
            )
        },
        run: settings_set_theme,
    },
    Tool {
        id: "settings.set_bindings",
        version: 1,
        action: Action::WriteFiles,
        label_key: "tool-settings-set-bindings",
        summary: "Choose a navigation preset (three_button, apple, fusion, paraview) or a custom binding table and write navigation.toml in the config folder.",
        outputs: &[],
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
];
