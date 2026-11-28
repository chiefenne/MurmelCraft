//! The tools of injector devices (FoehnCraft D343 to D345, plan stage
//! ID2m): `devices.open`, `devices.set` and `devices.clear`. The drawing
//! is the engine's ([`murmel_engine::device`]); the file is read by
//! [`murmel_mesh::devices`]. A time series whose manifest lists
//! `static.devices` opens them itself ([`crate::series`]).

use std::path::PathBuf;
use std::sync::Arc;

use murmel_engine::device::DeviceView;
use murmel_engine::{Item, Session};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{display_json, need_mesh, object, parse};
use crate::{Diagnostic, Run};

/// The devices with what the last build drew (null without devices).
pub(crate) fn devices_json(s: &Session) -> Value {
    let Some(m) = &s.mesh else {
        return Value::Null;
    };
    murmel_engine::device::summary(&s.display, &m.surface).unwrap_or(Value::Null)
}

fn done(s: &mut Session) -> Run {
    s.redraw();
    let mut out = display_json(s);
    out["devices"] = devices_json(s);
    Ok(out)
}

/// Read a device file into the display; which parts show stays when
/// devices showed before.
pub fn open_devices(s: &mut Session, path: PathBuf) -> Result<(), Diagnostic> {
    let d = murmel_mesh::read_devices(&path).map_err(|e| Diagnostic::new(e.code, e.message))?;
    let v = match s.display.devices.clone() {
        Some(old) => DeviceView {
            path: path.clone(),
            ..old
        },
        None => DeviceView::new(path),
    };
    s.display.devices = Some(v);
    s.display.device_data = Some(Arc::new(d));
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenInput {
    path: PathBuf,
}

pub(crate) fn devices_open(input: Value, s: &mut Session) -> Run {
    let i: OpenInput = parse(input)?;
    need_mesh(s)?;
    open_devices(s, i.path)?;
    done(s)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetInput {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    surface: Option<bool>,
    #[serde(default)]
    axes: Option<bool>,
    #[serde(default)]
    cones: Option<bool>,
}

pub(crate) fn devices_set(input: Value, s: &mut Session) -> Run {
    let i: SetInput = parse(input)?;
    need_mesh(s)?;
    let Some(mut v) = s.display.devices.clone() else {
        return Err(Diagnostic::new(
            "DEVICE001",
            "no devices are open; open FoehnCraft's spray/devices.vtu with devices.open, or a series that lists it",
        ));
    };
    if let Some(x) = i.enabled {
        v.enabled = x;
    }
    if let Some(x) = i.surface {
        v.surface = x;
    }
    if let Some(x) = i.axes {
        v.axes = x;
    }
    if let Some(x) = i.cones {
        v.cones = x;
    }
    s.display.devices = Some(v);
    done(s)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClearInput {}

pub(crate) fn devices_clear(input: Value, s: &mut Session) -> Run {
    let ClearInput {} = parse(input)?;
    s.display.devices = None;
    s.display.device_data = None;
    s.display.selection.retain(|&i| i != Item::Devices);
    s.display.colors.remove(&Item::Devices);
    done(s)
}

// ---- schemas ------------------------------------------------------------

pub(crate) fn devices_open_schema() -> Value {
    object(
        json!({"path": {"type": "string", "description": "A device file: FoehnCraft's spray/devices.vtu (ASCII VTK XML UnstructuredGrid of triangles and lines, cell fields device, part, hole, injector)"}}),
        &["path"],
    )
}

pub(crate) fn devices_set_schema() -> Value {
    object(
        json!({
            "enabled": {"type": "boolean", "description": "false keeps the settings and draws nothing"},
            "surface": {"type": "boolean", "description": "the placed surface of each device"},
            "axes": {"type": "boolean", "description": "one line per hole along its axis"},
            "cones": {"type": "boolean", "description": "the outline of each injector's cone"},
        }),
        &[],
    )
}

pub(crate) fn devices_clear_schema() -> Value {
    object(json!({}), &[])
}
