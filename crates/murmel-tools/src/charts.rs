//! The tools of charts and statistics (D268, second milestone):
//! `charts.open`, `charts.show`, `charts.data`, `charts.export` and
//! `stats.at`. The tables are read and charted by
//! [`murmel_engine::chart`]; the step charted and summed is the time
//! series' step when one is open (the player), else the step given, else
//! the run's last.

use std::path::{Path, PathBuf};

use murmel_engine::Session;
use murmel_engine::chart::{
    ChartColorsResolved, ChartKind, ChartSpec, Charts, MAX_SERIES, Run as RunTables, Weight,
};
use murmel_engine::theme::Theme;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::export::i18n;
use crate::tools::{object, parse};
use crate::{Diagnostic, Run};

fn no_charts() -> Diagnostic {
    Diagnostic::new(
        "CHART002",
        "no run is charted; open an output folder with charts.open (series.open opens its charts too)",
    )
}

fn bad(m: impl Into<String>) -> Diagnostic {
    Diagnostic::new("CHART003", m)
}

/// The largest chart picture, pixels a side.
const MAX_SIDE: u32 = 8192;

/// The step the player shows, if a series is open.
pub fn player_step(s: &Session) -> Option<u64> {
    s.series.as_ref().map(|r| r.entry().step)
}

/// The charts as tools report them (null without).
pub fn charts_json(s: &Session) -> Value {
    let Some(c) = &s.charts else {
        return Value::Null;
    };
    json!({
        "run": c.run.summary(),
        "spec": c.spec,
        "visible": c.visible,
        "step": player_step(s),
    })
}

/// Open the charts of `path` (a folder, or a file in it) unless they are
/// open already; quietly nothing when the folder has no tables (used by
/// `series.open`). Returns whether charts are open for the folder.
pub fn open_beside(s: &mut Session, path: &Path) -> bool {
    let folder = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    if s.charts.as_ref().is_some_and(|c| c.run.folder == folder) {
        return true;
    }
    match RunTables::read(&folder) {
        Ok(r) => {
            s.charts = Some(Charts::new(r));
            true
        }
        Err(_) => false,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenInput {
    path: PathBuf,
}

pub(crate) fn charts_open_schema() -> Value {
    object(
        json!({"path": {"type": "string", "description": "A FoehnCraft output folder, or a table or series.json in it"}}),
        &["path"],
    )
}

pub(crate) fn charts_open(input: Value, s: &mut Session) -> Run {
    let i: OpenInput = parse(input)?;
    let r = RunTables::read(&i.path).map_err(|e| Diagnostic::new("CHART001", e.0))?;
    s.charts = Some(Charts::new(r));
    Ok(json!({"charts": charts_json(s)}))
}

/// The chart fields an input may give; missing ones keep the panel's
/// chart (`charts.show`) or take the defaults (the others).
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SpecInput {
    #[serde(default)]
    chart: Option<ChartKind>,
    #[serde(default)]
    datums: Option<Vec<String>>,
    #[serde(default)]
    injectors: Option<Vec<String>>,
    #[serde(default)]
    weight: Option<Weight>,
    #[serde(default)]
    cumulative: Option<bool>,
    #[serde(default)]
    columns: Option<Vec<String>>,
}

impl SpecInput {
    /// `base` with the fields given; a new chart kind starts from its
    /// defaults.
    fn apply(self, base: &ChartSpec) -> ChartSpec {
        let mut s = match self.chart {
            Some(k) if k != base.chart => ChartSpec::new(k),
            _ => base.clone(),
        };
        if let Some(v) = self.datums {
            s.datums = v;
        }
        if let Some(v) = self.injectors {
            s.injectors = v;
        }
        if let Some(v) = self.weight {
            s.weight = v;
        }
        if let Some(v) = self.cumulative {
            s.cumulative = v;
        }
        if let Some(v) = self.columns {
            s.columns = v;
        }
        s
    }
}

fn spec_props() -> serde_json::Map<String, Value> {
    let v = json!({
        "chart": {"enum": ChartKind::ALL.map(ChartKind::key)},
        "datums": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_SERIES,
                   "description": "Datum names; a histogram takes one"},
        "injectors": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_SERIES},
        "weight": {"enum": ["mass", "droplets"]},
        "cumulative": {"type": "boolean"},
        "columns": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_SERIES,
                    "description": "datums:<column> or spray:<column>, all of one unit"}
    });
    match v {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

fn with(extra: Value) -> Value {
    let mut p = spec_props();
    if let Value::Object(e) = extra {
        p.extend(e);
    }
    Value::Object(p)
}

/// Split an input into the chart fields and the rest.
fn split(input: Value, rest: &[&str]) -> Result<(SpecInput, Value), Diagnostic> {
    let Value::Object(mut m) = input else {
        return Err(Diagnostic::new("TOOL002", "the input is no object"));
    };
    let mut other = serde_json::Map::new();
    for k in rest {
        if let Some(v) = m.remove(*k) {
            other.insert(k.to_string(), v);
        }
    }
    Ok((parse(Value::Object(m))?, Value::Object(other)))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct StepInput {
    #[serde(default)]
    step: Option<u64>,
}

pub(crate) fn charts_show_schema() -> Value {
    object(with(json!({"visible": {"type": "boolean"}})), &[])
}

pub(crate) fn charts_show(input: Value, s: &mut Session) -> Run {
    let (spec, rest) = split(input, &["visible"])?;
    let visible = rest.get("visible").and_then(Value::as_bool);
    let step = player_step(s);
    let c = s.charts.as_mut().ok_or_else(no_charts)?;
    let new = spec.apply(&c.spec);
    let i18n = i18n()?;
    let chart = c.run.chart(&new, step, &i18n).map_err(|e| bad(e.0))?;
    c.spec = new;
    if let Some(v) = visible {
        c.visible = v;
    }
    Ok(json!({"charts": charts_json(s), "chart": chart.data()}))
}

pub(crate) fn charts_data_schema() -> Value {
    object(
        with(json!({"step": {"type": "integer", "minimum": 0,
            "description": "The step of the cursor and the histogram; left out: the player's step, else none (a histogram then takes the last interval)"}})),
        &[],
    )
}

/// The spec of an input over the panel's, and the step.
fn spec_and_step(
    input: Value,
    s: &Session,
    rest_keys: &[&str],
) -> Result<(ChartSpec, Option<u64>, Value), Diagnostic> {
    let mut keys = vec!["step"];
    keys.extend_from_slice(rest_keys);
    let (spec, mut rest) = split(input, &keys)?;
    let st: StepInput = parse(json!({"step": rest
        .as_object_mut()
        .and_then(|m| m.remove("step"))
        .unwrap_or(Value::Null)}))?;
    let c = s.charts.as_ref().ok_or_else(no_charts)?;
    Ok((
        spec.apply(&c.spec),
        st.step.or_else(|| player_step(s)),
        rest,
    ))
}

pub(crate) fn charts_data(input: Value, s: &mut Session) -> Run {
    let (spec, step, _) = spec_and_step(input, s, &[])?;
    let c = s.charts.as_ref().ok_or_else(no_charts)?;
    let chart = c.run.chart(&spec, step, &i18n()?).map_err(|e| bad(e.0))?;
    Ok(json!({"step": step, "spec": spec, "chart": chart.data()}))
}

pub(crate) fn charts_export_schema() -> Value {
    object(
        with(json!({
            "path": {"type": "string", "description": "A .png or .svg file"},
            "step": {"type": "integer", "minimum": 0},
            "width": {"type": "number", "minimum": 160, "maximum": 4096, "description": "Points, default 720"},
            "height": {"type": "number", "minimum": 120, "maximum": 4096, "description": "Points, default 405"},
            "scale": {"type": "number", "exclusiveMinimum": 0, "maximum": 8, "description": "PNG pixels per point, default 2"},
            "theme": {"enum": ["dark", "light"], "description": "Default: the session's theme"}
        })),
        &["path"],
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportRest {
    path: PathBuf,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
    #[serde(default)]
    scale: Option<f64>,
    #[serde(default)]
    theme: Option<Theme>,
}

pub(crate) fn charts_export(input: Value, s: &mut Session) -> Run {
    let (spec, step, rest) =
        spec_and_step(input, s, &["path", "width", "height", "scale", "theme"])?;
    let e: ExportRest = parse(rest)?;
    let c = s.charts.as_ref().ok_or_else(no_charts)?;
    let chart = c.run.chart(&spec, step, &i18n()?).map_err(|e| bad(e.0))?;
    let (w, h) = (e.width.unwrap_or(720.0), e.height.unwrap_or(405.0));
    if !(160.0..=4096.0).contains(&w) || !(120.0..=4096.0).contains(&h) {
        return Err(bad(format!(
            "{w} x {h} points is outside 160..4096 x 120..4096"
        )));
    }
    let dark = e.theme.unwrap_or(s.theme).is_dark(None);
    let colors = ChartColorsResolved::new(&s.tokens, dark);
    let ext = e
        .path
        .extension()
        .and_then(|x| x.to_str())
        .map(str::to_ascii_lowercase);
    let (pw, ph) = match ext.as_deref() {
        Some("svg") => {
            std::fs::write(&e.path, chart.svg(w, h, &colors))
                .map_err(|x| Diagnostic::new("EXPORT002", format!("{}: {x}", e.path.display())))?;
            (w, h)
        }
        Some("png") => {
            let scale = e.scale.unwrap_or(2.0);
            if !(scale > 0.0 && scale <= 8.0) {
                return Err(bad(format!("scale {scale} is outside 0 to 8")));
            }
            let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
            if pw > MAX_SIDE || ph > MAX_SIDE || pw == 0 || ph == 0 {
                return Err(bad(format!("{pw} x {ph} pixels is outside 1..{MAX_SIDE}")));
            }
            let rgba = murmel_render::overlay::chart_picture(&chart, pw, ph, scale as f32, &colors)
                .map_err(|x| Diagnostic::new(x.code, x.message))?;
            murmel_render::write_png_with(&e.path, pw, ph, &rgba, None)
                .map_err(|x| Diagnostic::new(x.code, x.message))?;
            (pw as f64, ph as f64)
        }
        _ => {
            return Err(bad(format!(
                "{}: a chart is written as .png or .svg",
                e.path.display()
            )));
        }
    };
    Ok(json!({
        "files": [e.path],
        "width": pw,
        "height": ph,
        "step": step,
        "spec": spec,
        "notes": chart.notes,
    }))
}

pub(crate) fn stats_at_schema() -> Value {
    object(
        json!({"step": {"type": "integer", "minimum": 0,
            "description": "Left out: the player's step, else the run's last"}}),
        &[],
    )
}

pub(crate) fn stats_at(input: Value, s: &mut Session) -> Run {
    let i: StepInput = parse(input)?;
    let c = s.charts.as_ref().ok_or_else(no_charts)?;
    let step = i.step.or_else(|| player_step(s));
    Ok(json!({"statistics": c.run.statistics(step)}))
}
