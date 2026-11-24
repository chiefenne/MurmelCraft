//! Charts and statistics of a FoehnCraft run (D268, second milestone):
//! the tables a transient run writes beside its fields, read without the
//! solver, drawn as time-series charts and size histograms, and summed up
//! at any step for the statistics panel.
//!
//! The tables (FoehnCraft `docs/interfaces.md`, each a CSV file whose
//! first line names its format and version and whose second line names
//! the units):
//!
//! - `datums.csv`, format `foehncraft-datums 1`: `step,time` and the
//!   column group `<datum>.<quantity>` of every datum: gas reports, and
//!   with sprays `liquid_kg_pos`, `liquid_kg_neg` (cumulative), `liquid_rate`
//!   (net over the interval), `crossings`, `d10`, `d32`, `dv50`.
//! - `spray.csv`, format `foehncraft-spray 1`: one row per time step, the
//!   mass book, the Sauter diameter `d32_m` of the liquid in the flow and
//!   per injector `<name>.penetration_m`.
//! - `spray_hist_<datum>.csv`, format `foehncraft-spray-histogram 1`: per
//!   interval the rows `step,time,bin,d_lo,d_hi,droplets,mass`, bin −1
//!   below the first edge and bin `bins` above the last.
//!
//! One chart model ([`Chart`]) feeds every front end: [`Chart::layout`]
//! places it in a rectangle as primitives ([`Prim`]) that the window
//! (egui), the PNG export (vello_cpu) and [`Chart::svg`] draw alike. The
//! axis ticks are Heckbert's nice numbers (P. S. Heckbert, "Nice numbers
//! for graph labels", Graphics Gems, 1990); a log axis ticks its decades.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

use crate::i18n::I18n;
use crate::theme::Rgb;

/// The largest table read, in bytes (a table is read whole).
const MAX_TABLE: u64 = 512 << 20;
/// The most series one chart shows: the chart palette's hues, never
/// cycled (`assets/tokens.toml`, `[chart]`).
pub const MAX_SERIES: usize = 6;
/// The tables' names in an output folder.
pub const DATUMS: &str = "datums.csv";
pub const SPRAY: &str = "spray.csv";
const HIST_PREFIX: &str = "spray_hist_";

/// Why a run's tables cannot be read or charted (CHART001, CHART003).
#[derive(Clone, Debug, PartialEq)]
pub struct ChartError(pub String);

impl std::fmt::Display for ChartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(m: impl Into<String>) -> Result<T, ChartError> {
    Err(ChartError(m.into()))
}

/// One CSV table of FoehnCraft: its format line, units line, columns and
/// rows of numbers (a cell that is no number is NaN; `inf` is infinite).
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub path: PathBuf,
    /// The format named on the first line (`foehncraft-datums`).
    pub format: String,
    pub version: u32,
    /// The case named on the first line.
    pub case: Option<String>,
    /// The units line, as written.
    pub units: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<f64>>,
}

impl Table {
    /// Parse `text` of `path`, which must be of `format` version 1.
    pub fn parse(path: &Path, text: &str, format: &str) -> Result<Table, ChartError> {
        let name = path.display();
        let mut fmt = None;
        let mut units = String::new();
        let mut lines = text.lines();
        let mut header = None;
        for line in lines.by_ref() {
            if let Some(c) = line.strip_prefix('#') {
                let c = c.trim();
                if let Some(u) = c.strip_prefix("units:") {
                    units = u.trim().to_string();
                } else if let Some(f) = c.strip_prefix("format ") {
                    fmt = Some(f.to_string());
                }
                continue;
            }
            if !line.trim().is_empty() {
                header = Some(line);
                break;
            }
        }
        let Some(fmt) = fmt else {
            return err(format!("{name}: no format line (# format {format} 1; …)"));
        };
        let mut parts = fmt.split(';').map(str::trim);
        let head = parts.next().unwrap_or("");
        let mut hw = head.split_whitespace();
        let (got, ver) = (hw.next().unwrap_or(""), hw.next().unwrap_or(""));
        if got != format {
            return err(format!("{name}: format {got}, expected {format}"));
        }
        let version: u32 = ver.parse().unwrap_or(0);
        if version != 1 {
            return err(format!(
                "{name}: {format} version {ver}; MurmelCraft reads version 1"
            ));
        }
        let case = parts
            .find_map(|p| p.strip_prefix("case "))
            .map(str::to_string);
        let Some(header) = header else {
            return err(format!("{name}: no header row"));
        };
        let columns: Vec<String> = header.split(',').map(|c| c.trim().to_string()).collect();
        if columns.len() < 2 || columns[0] != "step" || columns[1] != "time" {
            return err(format!("{name}: the header does not start with step,time"));
        }
        let mut rows = Vec::new();
        for (k, line) in lines.enumerate() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let row: Vec<f64> = line
                .split(',')
                .map(|c| c.trim().parse::<f64>().unwrap_or(f64::NAN))
                .collect();
            if row.len() != columns.len() {
                // A run still writing may have cut its last row short.
                if lines_left_empty(text, k) {
                    break;
                }
                return err(format!(
                    "{name}: row {} has {} cells, the header {}",
                    k + 1,
                    row.len(),
                    columns.len()
                ));
            }
            rows.push(row);
        }
        Ok(Table {
            path: path.to_path_buf(),
            format: format.to_string(),
            version,
            case,
            units,
            columns,
            rows,
        })
    }

    pub fn read(path: &Path, format: &str) -> Result<Table, ChartError> {
        let text = read_text(path)?;
        Self::parse(path, &text, format)
    }

    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    /// The values of column `c` against time.
    pub fn points(&self, c: usize) -> Vec<[f64; 2]> {
        self.rows
            .iter()
            .filter(|r| r[1].is_finite() && r[c].is_finite())
            .map(|r| [r[1], r[c]])
            .collect()
    }

    /// The last row at or before `step` (the first row when `step` is
    /// before them all), by index.
    pub fn row_at(&self, step: u64) -> Option<usize> {
        if self.rows.is_empty() {
            return None;
        }
        let k = self.rows.partition_point(|r| r[0] <= step as f64);
        Some(k.saturating_sub(1))
    }

    /// The value of `column` in row `r`, when finite.
    pub fn get(&self, r: usize, column: &str) -> Option<f64> {
        let c = self.column(column)?;
        self.rows.get(r)?.get(c).copied().filter(|v| v.is_finite())
    }
}

/// Whether the text after data row `k` (0-based) holds no further rows,
/// i.e. row `k` is the last one: a cut-short last row of a run that is
/// still writing.
fn lines_left_empty(text: &str, k: usize) -> bool {
    let rows: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .collect();
    // rows[0] is the header.
    rows.len() == k + 2
}

fn read_text(path: &Path) -> Result<String, ChartError> {
    let meta =
        std::fs::metadata(path).map_err(|e| ChartError(format!("{}: {e}", path.display())))?;
    if meta.len() > MAX_TABLE {
        return err(format!(
            "{}: {} bytes, more than the {} a table may have",
            path.display(),
            meta.len(),
            MAX_TABLE
        ));
    }
    std::fs::read_to_string(path).map_err(|e| ChartError(format!("{}: {e}", path.display())))
}

/// One interval of a size histogram.
#[derive(Clone, Debug, PartialEq)]
pub struct Interval {
    pub step: u64,
    pub time: f64,
    /// The bin edges, `bins + 1` of them.
    pub edges: Vec<f64>,
    /// Droplets and mass per bin.
    pub droplets: Vec<f64>,
    pub mass: Vec<f64>,
    /// Below the first edge and above the last: droplets, mass.
    pub under: [f64; 2],
    pub over: [f64; 2],
}

/// The size histograms of one datum, interval by interval.
#[derive(Clone, Debug, PartialEq)]
pub struct Histogram {
    pub datum: String,
    pub path: PathBuf,
    pub intervals: Vec<Interval>,
}

impl Histogram {
    pub fn from_table(datum: &str, t: &Table) -> Result<Histogram, ChartError> {
        let col = |n: &str| {
            t.column(n)
                .ok_or_else(|| ChartError(format!("{}: no column {n}", t.path.display())))
        };
        let (cs, ct, cb, clo, chi, cn, cm) = (
            col("step")?,
            col("time")?,
            col("bin")?,
            col("d_lo")?,
            col("d_hi")?,
            col("droplets")?,
            col("mass")?,
        );
        let mut intervals: Vec<Interval> = Vec::new();
        for r in &t.rows {
            let step = r[cs].max(0.0) as u64;
            if intervals.last().is_none_or(|i| i.step != step) {
                intervals.push(Interval {
                    step,
                    time: r[ct],
                    edges: Vec::new(),
                    droplets: Vec::new(),
                    mass: Vec::new(),
                    under: [0.0; 2],
                    over: [0.0; 2],
                });
            }
            let Some(i) = intervals.last_mut() else {
                continue;
            };
            let (n, m) = (finite(r[cn]), finite(r[cm]));
            let bin = r[cb];
            if bin < 0.0 {
                i.under = [n, m];
            } else if !r[chi].is_finite() {
                i.over = [n, m];
            } else {
                if i.edges.is_empty() {
                    i.edges.push(r[clo]);
                }
                i.edges.push(r[chi]);
                i.droplets.push(n);
                i.mass.push(m);
            }
        }
        Ok(Histogram {
            datum: datum.to_string(),
            path: t.path.clone(),
            intervals,
        })
    }

    /// The interval ending at or before `step`, by index.
    pub fn interval_at(&self, step: u64) -> Option<usize> {
        if self.intervals.is_empty() {
            return None;
        }
        let k = self.intervals.partition_point(|i| i.step <= step);
        Some(k.saturating_sub(1))
    }

    /// The sum of the intervals up to and with index `k` (the same bins
    /// throughout, else the intervals of other bins are left out).
    pub fn cumulative(&self, k: usize) -> Option<Interval> {
        let last = self.intervals.get(k)?;
        let mut sum = Interval {
            droplets: vec![0.0; last.droplets.len()],
            mass: vec![0.0; last.mass.len()],
            under: [0.0; 2],
            over: [0.0; 2],
            ..last.clone()
        };
        for i in &self.intervals[..=k] {
            if i.edges != last.edges {
                continue;
            }
            for b in 0..sum.droplets.len() {
                sum.droplets[b] += i.droplets[b];
                sum.mass[b] += i.mass[b];
            }
            for j in 0..2 {
                sum.under[j] += i.under[j];
                sum.over[j] += i.over[j];
            }
        }
        Some(sum)
    }
}

fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

impl Interval {
    pub fn total(&self) -> [f64; 2] {
        [
            self.droplets.iter().sum::<f64>() + self.under[0] + self.over[0],
            self.mass.iter().sum::<f64>() + self.under[1] + self.over[1],
        ]
    }

    /// The diameter below which the share `q` (0..1) of the mass lies,
    /// from the bins: linear in log d within a bin. None when the share
    /// falls below or above the histogram, or there is no mass.
    pub fn mass_quantile(&self, q: f64) -> Option<f64> {
        let total = self.total()[1];
        if total <= 0.0 {
            return None;
        }
        let mut acc = self.under[1];
        if acc >= q * total {
            return None;
        }
        for (b, m) in self.mass.iter().enumerate() {
            if acc + m >= q * total && *m > 0.0 {
                let f = (q * total - acc) / m;
                let (lo, hi) = (self.edges[b], self.edges[b + 1]);
                return Some(if lo > 0.0 {
                    (lo.ln() + f * (hi / lo).ln()).exp()
                } else {
                    lo + f * (hi - lo)
                });
            }
            acc += m;
        }
        None
    }
}

/// What a datum is: a surface liquid crosses, or a volume that holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DatumKind {
    Surface,
    Volume,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Datum {
    pub name: String,
    pub kind: DatumKind,
    /// Whether it has the spray columns.
    pub liquid: bool,
}

/// The tables of one run's output folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub folder: PathBuf,
    pub case: Option<String>,
    pub datums_table: Option<Table>,
    pub spray: Option<Table>,
    pub histograms: BTreeMap<String, Histogram>,
    pub datums: Vec<Datum>,
    pub injectors: Vec<String>,
    /// The end-of-run statistics of `result.json` (its first solve's
    /// `datums` and `spray`), when it has them.
    pub result: Option<Value>,
    /// Tables of the folder that could not be read, with why.
    pub skipped: Vec<String>,
}

impl Run {
    /// Read the tables of `path`: an output folder, or one of its tables
    /// or its `series.json` (the folder it lies in).
    pub fn read(path: &Path) -> Result<Run, ChartError> {
        let folder = if path.is_dir() {
            path.to_path_buf()
        } else if path.is_file() {
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
        } else {
            return err(format!("{} does not exist", path.display()));
        };
        let mut run = Run {
            folder: folder.clone(),
            case: None,
            datums_table: None,
            spray: None,
            histograms: BTreeMap::new(),
            datums: Vec::new(),
            injectors: Vec::new(),
            result: None,
            skipped: Vec::new(),
        };
        let d = folder.join(DATUMS);
        if d.is_file() {
            run.datums_table = Some(Table::read(&d, "foehncraft-datums")?);
        }
        let s = folder.join(SPRAY);
        if s.is_file() {
            run.spray = Some(Table::read(&s, "foehncraft-spray")?);
        }
        if run.datums_table.is_none() && run.spray.is_none() {
            return err(format!(
                "{} has neither {DATUMS} nor {SPRAY} (a transient FoehnCraft run with [[datum]] or [spray] writes them)",
                folder.display()
            ));
        }
        if let Some(t) = &run.datums_table {
            run.datums = datums_of(t);
        }
        if let Some(t) = &run.spray {
            run.injectors = t
                .columns
                .iter()
                .filter_map(|c| c.strip_suffix(".penetration_m"))
                .map(str::to_string)
                .collect();
        }
        let mut names: Vec<PathBuf> = std::fs::read_dir(&folder)
            .map_err(|e| ChartError(format!("{}: {e}", folder.display())))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        names.sort();
        for p in names {
            let Some(datum) = p
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix(HIST_PREFIX))
                .and_then(|n| n.strip_suffix(".csv"))
                .map(str::to_string)
            else {
                continue;
            };
            match Table::read(&p, "foehncraft-spray-histogram")
                .and_then(|t| Histogram::from_table(&datum, &t))
            {
                Ok(h) => {
                    run.histograms.insert(datum, h);
                }
                Err(e) => run.skipped.push(e.0),
            }
        }
        run.case = run
            .datums_table
            .as_ref()
            .and_then(|t| t.case.clone())
            .or_else(|| run.spray.as_ref().and_then(|t| t.case.clone()));
        run.result = read_result(&folder.join("result.json"));
        Ok(run)
    }

    /// The first and last step and time over the tables.
    pub fn span(&self) -> Option<([u64; 2], [f64; 2])> {
        let mut s = [u64::MAX, 0];
        let mut t = [f64::INFINITY, f64::NEG_INFINITY];
        for tab in [&self.datums_table, &self.spray].into_iter().flatten() {
            for r in [tab.rows.first(), tab.rows.last()].into_iter().flatten() {
                s[0] = s[0].min(r[0] as u64);
                s[1] = s[1].max(r[0] as u64);
                t[0] = t[0].min(r[1]);
                t[1] = t[1].max(r[1]);
            }
        }
        (s[0] <= s[1]).then_some((s, t))
    }

    /// The time of `step`: from the spray table (every step), else the
    /// datums table (its row at or before).
    pub fn time_of(&self, step: u64) -> Option<f64> {
        for t in [&self.spray, &self.datums_table].into_iter().flatten() {
            if let Some(r) = t.row_at(step) {
                return Some(t.rows[r][1]);
            }
        }
        None
    }

    pub fn datum(&self, name: &str) -> Option<&Datum> {
        self.datums.iter().find(|d| d.name == name)
    }

    /// What the result of `charts.open` lists.
    pub fn summary(&self) -> Value {
        let table = |t: &Option<Table>| {
            t.as_ref()
                .map(|t| json!({"path": t.path, "rows": t.rows.len(), "columns": t.columns.len()}))
        };
        let span = self.span();
        json!({
            "folder": self.folder,
            "case": self.case,
            "tables": {
                "datums": table(&self.datums_table),
                "spray": table(&self.spray),
                "histograms": self.histograms.keys().collect::<Vec<_>>(),
            },
            "datums": self.datums,
            "injectors": self.injectors,
            "steps": span.map(|s| s.0),
            "times": span.map(|s| s.1),
            "end_of_run_statistics": self.result.is_some(),
            "skipped": self.skipped,
            "charts": ChartKind::ALL.iter().map(|k| k.key()).collect::<Vec<_>>(),
        })
    }
}

/// The datums of a `datums.csv` header, in order.
fn datums_of(t: &Table) -> Vec<Datum> {
    let mut out: Vec<Datum> = Vec::new();
    for c in &t.columns[2..] {
        let Some((name, q)) = c.rsplit_once('.') else {
            continue;
        };
        let k = match out.iter().position(|d| d.name == name) {
            Some(k) => k,
            None => {
                out.push(Datum {
                    name: name.to_string(),
                    kind: DatumKind::Surface,
                    liquid: false,
                });
                out.len() - 1
            }
        };
        match q {
            "volume" => out[k].kind = DatumKind::Volume,
            "liquid_kg_pos" | "liquid_kg" => out[k].liquid = true,
            _ => {}
        }
    }
    out
}

fn read_result(path: &Path) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_TABLE {
        return None;
    }
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let solve = v.get("solves")?.get(0)?;
    let datums = solve.get("datums").cloned().unwrap_or(Value::Null);
    let spray = solve.get("spray").cloned().unwrap_or(Value::Null);
    (!datums.is_null() || !spray.is_null()).then(|| json!({"datums": datums, "spray": spray}))
}

// ---------------------------------------------------------------- charts

/// The charts of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    /// Net liquid mass flow through the surface datums (kg/s, per interval).
    MassFlow,
    /// Net liquid mass through the surface datums so far (kg).
    Cumulative,
    /// Penetration of each injector (m).
    Penetration,
    /// Sauter mean diameter D32 of the liquid through each datum and in
    /// the flow (m).
    Diameters,
    /// The size histogram of one datum.
    Histogram,
    /// Columns chosen by name, all of one unit.
    Columns,
}

impl ChartKind {
    pub const ALL: [ChartKind; 6] = [
        ChartKind::MassFlow,
        ChartKind::Cumulative,
        ChartKind::Penetration,
        ChartKind::Diameters,
        ChartKind::Histogram,
        ChartKind::Columns,
    ];

    pub fn key(self) -> &'static str {
        match self {
            ChartKind::MassFlow => "mass_flow",
            ChartKind::Cumulative => "cumulative",
            ChartKind::Penetration => "penetration",
            ChartKind::Diameters => "diameters",
            ChartKind::Histogram => "histogram",
            ChartKind::Columns => "columns",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            ChartKind::MassFlow => "chart-mass-flow",
            ChartKind::Cumulative => "chart-cumulative",
            ChartKind::Penetration => "chart-penetration",
            ChartKind::Diameters => "chart-diameters",
            ChartKind::Histogram => "chart-histogram",
            ChartKind::Columns => "chart-columns",
        }
    }
}

/// What a histogram counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Weight {
    #[default]
    Mass,
    Droplets,
}

/// Which chart to draw and of what.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct ChartSpec {
    pub chart: ChartKind,
    /// The datums of a time-series chart (empty: the first six that have
    /// the quantity), or the one datum of a histogram (empty: the first).
    #[serde(default)]
    pub datums: Vec<String>,
    /// The injectors of the penetration chart (empty: the first six).
    #[serde(default)]
    pub injectors: Vec<String>,
    /// Histogram: mass or droplets.
    #[serde(default)]
    pub weight: Weight,
    /// Histogram: all intervals up to the step (true, the default) or the
    /// interval of the step alone.
    #[serde(default = "yes")]
    pub cumulative: bool,
    /// The `columns` chart: `datums:<column>` or `spray:<column>`.
    #[serde(default)]
    pub columns: Vec<String>,
}

fn yes() -> bool {
    true
}

impl ChartSpec {
    pub fn new(chart: ChartKind) -> Self {
        Self {
            chart,
            datums: Vec::new(),
            injectors: Vec::new(),
            weight: Weight::Mass,
            cumulative: true,
            columns: Vec::new(),
        }
    }
}

/// An axis: its title, unit (scaled), range in data units and ticks.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Axis {
    pub title: String,
    /// The unit the tick labels show (`µm` for a length in metres of a
    /// few micrometres), empty for none.
    pub unit: String,
    /// The factor from data units to the unit shown.
    pub factor: f64,
    pub min: f64,
    pub max: f64,
    pub log: bool,
    pub ticks: Vec<f64>,
}

impl Axis {
    /// Where `v` lies along the axis, 0 at min and 1 at max.
    pub fn position(&self, v: f64) -> f64 {
        if self.log {
            let (a, b) = (self.min.max(f64::MIN_POSITIVE).log10(), self.max.log10());
            (v.max(f64::MIN_POSITIVE).log10() - a) / (b - a)
        } else {
            (v - self.min) / (self.max - self.min)
        }
    }

    pub fn tick_label(&self, v: f64) -> String {
        let s = v * self.factor;
        if self.log {
            return short(s);
        }
        let step = self
            .ticks
            .windows(2)
            .map(|w| (w[1] - w[0]).abs() * self.factor)
            .fold(f64::INFINITY, f64::min);
        let digits = if step.is_finite() && step > 0.0 {
            (-step.log10().floor()).max(0.0) as usize
        } else {
            2
        };
        if s.abs() >= 1e5 || (s != 0.0 && s.abs() < 1e-3) {
            format!("{s:.1e}")
        } else {
            format!("{s:.digits$}")
        }
    }

    pub fn title_with_unit(&self) -> String {
        if self.unit.is_empty() {
            self.title.clone()
        } else {
            format!("{} ({})", self.title, self.unit)
        }
    }
}

fn short(v: f64) -> String {
    let t = format!("{v:.3}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    t.to_string()
}

/// One line of a chart, coloured by its place in the chart.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Line {
    pub name: String,
    /// (time, value) in data units.
    pub points: Vec<[f64; 2]>,
}

/// One bar of a histogram: from `lo` to `hi` along x, `value` high.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Bar {
    pub lo: f64,
    pub hi: f64,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Chart {
    pub kind: ChartKind,
    pub title: String,
    pub x: Axis,
    pub y: Axis,
    pub lines: Vec<Line>,
    pub bars: Vec<Bar>,
    /// The step's time on a time axis (the player's step).
    pub cursor: Option<f64>,
    /// What the chart leaves out or cannot show, for the result and the
    /// window's note (translated).
    pub notes: Vec<String>,
}

/// The SI prefix that suits values up to `max` of a unit in `base`
/// (`m`, `kg`, `kg/s`): the unit shown and the factor.
fn scaled_unit(base: &str, max: f64) -> (String, f64) {
    let max = max.abs();
    let (units, factors): (&[&str], &[f64]) = match base {
        "m" => (&["m", "mm", "µm"], &[1.0, 1e3, 1e6]),
        "kg" => (&["kg", "g", "mg"], &[1.0, 1e3, 1e6]),
        "kg/s" => (&["kg/s", "g/s", "mg/s"], &[1.0, 1e3, 1e6]),
        _ => return (base.to_string(), 1.0),
    };
    for (u, f) in units.iter().zip(factors) {
        if max * f >= 1.0 || *f == factors[factors.len() - 1] {
            return (u.to_string(), *f);
        }
    }
    (base.to_string(), 1.0)
}

/// The unit of a column of a table, from its quantity.
pub fn unit_of(column: &str) -> &'static str {
    let q = column.rsplit_once('.').map_or(column, |(_, q)| q);
    match q {
        "time" => "s",
        "step" | "parcels" | "lost_parcels" | "rebounds" | "substeps" | "crossings"
        | "cut_error" | "droplets" => "-",
        "area" => "m²",
        "volume" => "m³",
        "mass_flow" | "liquid_rate" => "kg/s",
        "volume_flow" => "m³/s",
        "pressure_area" => "Pa",
        _ if q.starts_with("momentum_flow") => "N",
        _ if q.starts_with("velocity_") => "m/s",
        _ if q.ends_with("_kg") || q.starts_with("liquid_kg") || q == "mass" => "kg",
        _ if q.ends_with("_m")
            || matches!(q, "d10" | "d32" | "dv50" | "com_x" | "com_y" | "com_z") =>
        {
            "m"
        }
        _ => "",
    }
}

/// Heckbert's nice number near `x`: 1, 2, 5 or 10 times a power of ten.
fn nice(x: f64, round: bool) -> f64 {
    if x <= 0.0 || !x.is_finite() {
        return 1.0;
    }
    let e = x.log10().floor();
    let f = x / 10f64.powf(e);
    let n = if round {
        if f < 1.5 {
            1.0
        } else if f < 3.0 {
            2.0
        } else if f < 7.0 {
            5.0
        } else {
            10.0
        }
    } else if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    };
    n * 10f64.powf(e)
}

/// A linear axis over `lo..hi` widened to nice ticks (about `n`).
fn linear_axis(title: String, base: &str, lo: f64, hi: f64, n: usize) -> Axis {
    let (mut lo, mut hi) = (lo, hi);
    if !lo.is_finite() || !hi.is_finite() {
        (lo, hi) = (0.0, 1.0);
    }
    if hi - lo <= f64::EPSILON * hi.abs().max(1e-300) {
        let pad = if hi == 0.0 { 1.0 } else { hi.abs() * 0.1 };
        lo -= pad;
        hi += pad;
    }
    let range = nice(hi - lo, false);
    let step = nice(range / (n.max(2) - 1) as f64, true);
    let min = (lo / step).floor() * step;
    let max = (hi / step).ceil() * step;
    let mut ticks = Vec::new();
    let mut k = 0;
    while min + k as f64 * step <= max + step * 1e-9 && k < 64 {
        let v = min + k as f64 * step;
        ticks.push(if v.abs() < step * 1e-9 { 0.0 } else { v });
        k += 1;
    }
    let big = lo.abs().max(hi.abs());
    let (unit, factor) = scaled_unit(base, big);
    Axis {
        title,
        unit: if base == "-" { String::new() } else { unit },
        factor,
        min,
        max,
        log: false,
        ticks,
    }
}

/// A log axis over `lo..hi` (both above zero), ticked at its decades.
fn log_axis(title: String, base: &str, lo: f64, hi: f64) -> Axis {
    let (a, b) = (
        lo.log10().floor(),
        hi.log10().ceil().max(lo.log10().floor() + 1.0),
    );
    let ticks = (a as i32..=b as i32).map(|e| 10f64.powi(e)).collect();
    let (unit, factor) = scaled_unit(base, (lo * hi).sqrt());
    Axis {
        title,
        unit,
        factor,
        min: 10f64.powf(a),
        max: 10f64.powf(b),
        log: true,
        ticks,
    }
}

/// The names in `want` (all of `have` when empty) that `have` holds,
/// at most [`MAX_SERIES`]; unknown names are refused with the list.
fn pick(
    want: &[String],
    have: &[String],
    what: &str,
    notes: &mut Vec<String>,
    i18n: &I18n,
) -> Result<Vec<String>, ChartError> {
    for w in want {
        if !have.contains(w) {
            return err(format!(
                "no {what} {w}; the run has {}",
                if have.is_empty() {
                    "none".to_string()
                } else {
                    have.join(", ")
                }
            ));
        }
    }
    let all: Vec<String> = if want.is_empty() {
        have.to_vec()
    } else {
        want.to_vec()
    };
    if all.len() > MAX_SERIES {
        if !want.is_empty() {
            return err(format!(
                "{} {what}s asked; a chart shows at most {MAX_SERIES}",
                all.len()
            ));
        }
        notes.push(i18n.tr_args(
            "chart-note-first",
            &[
                ("shown", (MAX_SERIES as f64).into()),
                ("count", (all.len() as f64).into()),
            ],
        ));
    }
    Ok(all.into_iter().take(MAX_SERIES).collect())
}

impl Run {
    /// Build the chart `spec` with the cursor at `step` (none: no
    /// cursor; a histogram then shows the last interval).
    pub fn chart(
        &self,
        spec: &ChartSpec,
        step: Option<u64>,
        i18n: &I18n,
    ) -> Result<Chart, ChartError> {
        let mut notes = Vec::new();
        let cursor = step.and_then(|s| self.time_of(s));
        let time_title = i18n.tr("chart-axis-time");
        match spec.chart {
            ChartKind::MassFlow | ChartKind::Cumulative | ChartKind::Diameters => {
                let t = self.datums_table.as_ref();
                let (q, title_key, axis_key, base) = match spec.chart {
                    ChartKind::MassFlow => {
                        ("liquid_rate", "chart-mass-flow", "chart-axis-rate", "kg/s")
                    }
                    ChartKind::Cumulative => {
                        ("liquid_kg_pos", "chart-cumulative", "chart-axis-mass", "kg")
                    }
                    _ => ("d32", "chart-diameters", "chart-axis-d32", "m"),
                };
                let have: Vec<String> = self
                    .datums
                    .iter()
                    .filter(|d| d.kind == DatumKind::Surface && d.liquid)
                    .map(|d| d.name.clone())
                    .collect();
                // The diameters chart adds the liquid in the flow.
                let flow = spec.chart == ChartKind::Diameters
                    && self
                        .spray
                        .as_ref()
                        .is_some_and(|s| s.column("d32_m").is_some());
                let names = {
                    let mut room = have.clone();
                    if flow && spec.datums.is_empty() {
                        room.truncate(MAX_SERIES - 1);
                        if have.len() > MAX_SERIES - 1 {
                            notes.push(i18n.tr_args(
                                "chart-note-first",
                                &[
                                    ("shown", ((MAX_SERIES - 1) as f64).into()),
                                    ("count", (have.len() as f64).into()),
                                ],
                            ));
                        }
                        pick(&[], &room, "datum", &mut notes, i18n)?
                    } else {
                        let p = pick(&spec.datums, &have, "datum with liquid", &mut notes, i18n)?;
                        if flow && p.len() == MAX_SERIES {
                            return err(format!(
                                "the diameters chart shows the flow and at most {} datums",
                                MAX_SERIES - 1
                            ));
                        }
                        p
                    }
                };
                let mut lines = Vec::new();
                if let Some(t) = t {
                    for n in &names {
                        let pts = if spec.chart == ChartKind::Cumulative {
                            match (
                                t.column(&format!("{n}.liquid_kg_pos")),
                                t.column(&format!("{n}.liquid_kg_neg")),
                            ) {
                                (Some(p), Some(m)) => t
                                    .rows
                                    .iter()
                                    .filter(|r| r[1].is_finite() && r[p].is_finite())
                                    .map(|r| [r[1], r[p] - finite(r[m])])
                                    .collect(),
                                _ => continue,
                            }
                        } else {
                            match t.column(&format!("{n}.{q}")) {
                                Some(c) => {
                                    let mut p = t.points(c);
                                    if spec.chart == ChartKind::Diameters {
                                        // No crossings yet: zero means none.
                                        p.retain(|v| v[1] > 0.0);
                                    }
                                    p
                                }
                                None => continue,
                            }
                        };
                        lines.push(Line {
                            name: n.clone(),
                            points: pts,
                        });
                    }
                }
                if flow
                    && let Some(s) = &self.spray
                    && let Some(c) = s.column("d32_m")
                {
                    let mut p = s.points(c);
                    p.retain(|v| v[1] > 0.0);
                    lines.push(Line {
                        name: i18n.tr("chart-in-flow"),
                        points: p,
                    });
                }
                if lines.is_empty() {
                    notes.push(i18n.tr("chart-note-no-liquid"));
                }
                Ok(time_chart(
                    spec.chart,
                    i18n.tr(title_key),
                    time_title,
                    i18n.tr(axis_key),
                    base,
                    lines,
                    cursor,
                    notes,
                    spec.chart != ChartKind::Diameters,
                ))
            }
            ChartKind::Penetration => {
                let Some(s) = &self.spray else {
                    return err(format!("the run has no {SPRAY}"));
                };
                let names = pick(
                    &spec.injectors,
                    &self.injectors,
                    "injector",
                    &mut notes,
                    i18n,
                )?;
                let lines = names
                    .iter()
                    .filter_map(|n| {
                        let c = s.column(&format!("{n}.penetration_m"))?;
                        Some(Line {
                            name: n.clone(),
                            points: s.points(c),
                        })
                    })
                    .collect();
                Ok(time_chart(
                    spec.chart,
                    i18n.tr("chart-penetration"),
                    time_title,
                    i18n.tr("chart-axis-length"),
                    "m",
                    lines,
                    cursor,
                    notes,
                    true,
                ))
            }
            ChartKind::Columns => {
                if spec.columns.is_empty() {
                    return err(
                        "the columns chart needs columns: datums:<column> or spray:<column>",
                    );
                }
                if spec.columns.len() > MAX_SERIES {
                    return err(format!("a chart shows at most {MAX_SERIES} columns"));
                }
                let mut unit: Option<&str> = None;
                let mut lines = Vec::new();
                for c in &spec.columns {
                    let (t, name) = self.column_ref(c)?;
                    let u = unit_of(name);
                    match unit {
                        Some(v) if v != u => {
                            return err(format!(
                                "{c} is in {u}, the columns before in {v}; one chart has one axis, so chart them apart"
                            ));
                        }
                        _ => unit = Some(u),
                    }
                    let Some(k) = t.column(name) else {
                        continue;
                    };
                    lines.push(Line {
                        name: c.clone(),
                        points: t.points(k),
                    });
                }
                let u = unit.unwrap_or("");
                Ok(time_chart(
                    spec.chart,
                    i18n.tr("chart-columns"),
                    time_title,
                    i18n.tr("chart-axis-value"),
                    u,
                    lines,
                    cursor,
                    notes,
                    false,
                ))
            }
            ChartKind::Histogram => self.histogram_chart(spec, step, i18n, notes),
        }
    }

    /// `datums:<column>` or `spray:<column>` to its table and column.
    fn column_ref<'a>(&'a self, c: &'a str) -> Result<(&'a Table, &'a str), ChartError> {
        let (t, name) = match c.split_once(':') {
            Some(("datums", n)) => (self.datums_table.as_ref(), n),
            Some(("spray", n)) => (self.spray.as_ref(), n),
            _ => {
                return err(format!(
                    "{c}: name a column as datums:<column> or spray:<column>"
                ));
            }
        };
        let Some(t) = t else {
            return err(format!("{c}: the run has no such table"));
        };
        if t.column(name).is_none() || name == "step" {
            let some: Vec<&str> = t.columns[1..].iter().take(16).map(String::as_str).collect();
            return err(format!(
                "{c}: no such column; {} has {}{}",
                t.path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                some.join(", "),
                if t.columns.len() > 17 { ", …" } else { "" }
            ));
        }
        Ok((t, name))
    }

    fn histogram_chart(
        &self,
        spec: &ChartSpec,
        step: Option<u64>,
        i18n: &I18n,
        mut notes: Vec<String>,
    ) -> Result<Chart, ChartError> {
        let have: Vec<String> = self.histograms.keys().cloned().collect();
        let name = match spec.datums.as_slice() {
            [] => have.first().cloned().ok_or_else(|| {
                ChartError("the run has no size histograms (spray_hist_<datum>.csv)".into())
            })?,
            [one] => {
                if !self.histograms.contains_key(one) {
                    return err(format!(
                        "no histogram of datum {one}; the run has {}",
                        if have.is_empty() {
                            "none".into()
                        } else {
                            have.join(", ")
                        }
                    ));
                }
                one.clone()
            }
            _ => return err("a histogram shows one datum"),
        };
        let h = &self.histograms[&name];
        let k = match step {
            Some(s) => h.interval_at(s),
            None => h.intervals.len().checked_sub(1),
        };
        let Some(k) = k else {
            return err(format!("the histogram of {name} has no intervals"));
        };
        let iv = if spec.cumulative {
            h.cumulative(k).unwrap_or_else(|| h.intervals[k].clone())
        } else {
            h.intervals[k].clone()
        };
        let values = match spec.weight {
            Weight::Mass => &iv.mass,
            Weight::Droplets => &iv.droplets,
        };
        let total = iv.total()[if spec.weight == Weight::Mass { 1 } else { 0 }];
        let bars: Vec<Bar> = values
            .iter()
            .enumerate()
            .map(|(b, v)| Bar {
                lo: iv.edges[b],
                hi: iv.edges[b + 1],
                value: if total > 0.0 { 100.0 * v / total } else { 0.0 },
            })
            .collect();
        let (lo, hi) = (
            iv.edges.first().copied().unwrap_or(1e-6).max(1e-12),
            iv.edges.last().copied().unwrap_or(1e-3),
        );
        let (under, over) = match spec.weight {
            Weight::Mass => (iv.under[1], iv.over[1]),
            Weight::Droplets => (iv.under[0], iv.over[0]),
        };
        if total <= 0.0 {
            notes.push(i18n.tr("chart-note-no-crossings"));
        } else if under + over > 0.0 {
            notes.push(i18n.tr_args(
                "chart-note-outside",
                &[(
                    "share",
                    format!("{:.1}", 100.0 * (under + over) / total).into(),
                )],
            ));
        }
        let top = bars.iter().map(|b| b.value).fold(0.0, f64::max).max(1.0);
        let mut y = linear_axis(
            i18n.tr(match spec.weight {
                Weight::Mass => "chart-axis-mass-share",
                Weight::Droplets => "chart-axis-droplet-share",
            }),
            "%",
            0.0,
            top,
            5,
        );
        y.min = 0.0;
        let title = i18n.tr_args(
            if spec.cumulative {
                "chart-histogram-cumulative"
            } else {
                "chart-histogram-interval"
            },
            &[
                ("datum", name.clone().into()),
                ("step", (iv.step as f64).into()),
            ],
        );
        Ok(Chart {
            kind: ChartKind::Histogram,
            title,
            x: log_axis(i18n.tr("chart-axis-diameter"), "m", lo, hi),
            y,
            lines: Vec::new(),
            bars,
            cursor: None,
            notes,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn time_chart(
    kind: ChartKind,
    title: String,
    x_title: String,
    y_title: String,
    base: &str,
    lines: Vec<Line>,
    cursor: Option<f64>,
    notes: Vec<String>,
    from_zero: bool,
) -> Chart {
    let (mut x0, mut x1, mut y0, mut y1) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for l in &lines {
        for p in &l.points {
            x0 = x0.min(p[0]);
            x1 = x1.max(p[0]);
            y0 = y0.min(p[1]);
            y1 = y1.max(p[1]);
        }
    }
    if !x0.is_finite() {
        (x0, x1, y0, y1) = (0.0, 1.0, 0.0, 1.0);
    }
    if from_zero {
        y0 = y0.min(0.0);
        y1 = y1.max(0.0);
    }
    let mut x = linear_axis(x_title, "s", x0, x1, 6);
    // Time runs from the first row to the last, not to a rounded tick.
    x.min = x0.min(x.min.max(x0));
    x.max = x1;
    x.ticks
        .retain(|t| *t >= x.min - 1e-12 && *t <= x.max + 1e-12);
    if x.max <= x.min {
        x.max = x.min + 1.0;
    }
    x.unit = "s".into();
    x.factor = 1.0;
    let y = linear_axis(y_title, base, y0, y1, 5);
    Chart {
        kind,
        title,
        x,
        y,
        lines,
        bars: Vec::new(),
        cursor,
        notes,
    }
}

// ----------------------------------------------------------- layout

/// The colour role of a primitive; each front end maps it to its tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// Series hue `k` (`[chart] series`).
    Series(usize),
    Text,
    TextSecondary,
    /// Grid and axis lines (the theme's `line`).
    Grid,
    /// The step cursor (the theme's `accent`).
    Cursor,
    /// The chart's background (the theme's `bg_panel`).
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

/// A drawing primitive in points, y down from the top left.
#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    Rect {
        rect: [f64; 4],
        ink: Ink,
    },
    Polyline {
        points: Vec<[f64; 2]>,
        width: f64,
        ink: Ink,
    },
    /// `y` is the baseline.
    Text {
        text: String,
        at: [f64; 2],
        size: f64,
        anchor: Anchor,
        ink: Ink,
    },
}

/// Text sizes of charts, points.
pub const TITLE_PT: f64 = 13.0;
pub const LABEL_PT: f64 = 11.0;

impl Chart {
    /// Place the chart in a `w` × `h` rectangle (points) at the origin:
    /// the title and the y axis' title on top, the legend under the title
    /// (two or more series), tick labels left and below. `measure` gives
    /// the width of a text at a size in points. With `background` the
    /// rectangle is filled first.
    pub fn layout(
        &self,
        w: f64,
        h: f64,
        background: bool,
        measure: &dyn Fn(&str, f64) -> f64,
    ) -> Vec<Prim> {
        let mut out = Vec::new();
        if background {
            out.push(Prim::Rect {
                rect: [0.0, 0.0, w, h],
                ink: Ink::Background,
            });
        }
        let pad = 10.0;
        let mut top = pad + TITLE_PT;
        out.push(Prim::Text {
            text: self.title.clone(),
            at: [pad, top],
            size: TITLE_PT,
            anchor: Anchor::Start,
            ink: Ink::Text,
        });
        // The legend: a swatch and the name, wrapped to the width.
        if self.lines.len() >= 2 {
            top += 8.0;
            let mut x = pad;
            let row = LABEL_PT + 6.0;
            top += LABEL_PT;
            for (k, l) in self.lines.iter().enumerate() {
                let tw = measure(&l.name, LABEL_PT) + 14.0 + 12.0;
                if x + tw > w - pad && x > pad {
                    x = pad;
                    top += row;
                }
                out.push(Prim::Rect {
                    rect: [x, top - 8.0, x + 10.0, top + 0.5],
                    ink: Ink::Series(k),
                });
                out.push(Prim::Text {
                    text: l.name.clone(),
                    at: [x + 14.0, top],
                    size: LABEL_PT,
                    anchor: Anchor::Start,
                    ink: Ink::TextSecondary,
                });
                x += tw;
            }
        }
        top += 8.0 + LABEL_PT;
        out.push(Prim::Text {
            text: self.y.title_with_unit(),
            at: [pad, top],
            size: LABEL_PT,
            anchor: Anchor::Start,
            ink: Ink::TextSecondary,
        });
        let y_labels: Vec<String> = self.y.ticks.iter().map(|t| self.y.tick_label(*t)).collect();
        let lw = y_labels
            .iter()
            .map(|l| measure(l, LABEL_PT))
            .fold(0.0, f64::max);
        let x0 = pad + lw + 6.0;
        let x1 = w - pad - 6.0;
        let y0 = top + 10.0;
        let y1 = h - pad - 2.0 * LABEL_PT - 10.0;
        if x1 - x0 < 20.0 || y1 - y0 < 20.0 {
            return out;
        }
        let px = |v: f64| x0 + (x1 - x0) * self.x.position(v);
        let py = |v: f64| y1 - (y1 - y0) * self.y.position(v);
        // Grid and tick labels.
        for (t, l) in self.y.ticks.iter().zip(&y_labels) {
            let y = py(*t);
            out.push(Prim::Polyline {
                points: vec![[x0, y], [x1, y]],
                width: if *t == 0.0 { 1.0 } else { 0.5 },
                ink: Ink::Grid,
            });
            out.push(Prim::Text {
                text: l.clone(),
                at: [x0 - 6.0, y + 0.35 * LABEL_PT],
                size: LABEL_PT,
                anchor: Anchor::End,
                ink: Ink::TextSecondary,
            });
        }
        for t in &self.x.ticks {
            let x = px(*t);
            out.push(Prim::Polyline {
                points: vec![[x, y1], [x, y1 + 4.0]],
                width: 1.0,
                ink: Ink::Grid,
            });
            out.push(Prim::Text {
                text: self.x.tick_label(*t),
                at: [x, y1 + 6.0 + LABEL_PT],
                size: LABEL_PT,
                anchor: Anchor::Middle,
                ink: Ink::TextSecondary,
            });
        }
        out.push(Prim::Polyline {
            points: vec![[x0, y1], [x1, y1]],
            width: 1.0,
            ink: Ink::Grid,
        });
        out.push(Prim::Text {
            text: self.x.title_with_unit(),
            at: [x1, h - pad],
            size: LABEL_PT,
            anchor: Anchor::End,
            ink: Ink::TextSecondary,
        });
        // Bars, a gap of one point between them.
        for b in &self.bars {
            if b.value <= 0.0 {
                continue;
            }
            let (a, c) = (px(b.lo) + 0.5, px(b.hi) - 0.5);
            out.push(Prim::Rect {
                rect: [a, py(b.value), c.max(a + 0.5), y1],
                ink: Ink::Series(0),
            });
        }
        // Lines, thinned to about two points a point of width.
        let most = ((x1 - x0) * 2.0).max(16.0) as usize;
        for (k, l) in self.lines.iter().enumerate() {
            let stride = l.points.len().div_ceil(most).max(1);
            let mut pts: Vec<[f64; 2]> = l
                .points
                .iter()
                .step_by(stride)
                .map(|p| [px(p[0]), py(p[1])])
                .collect();
            if let Some(last) = l.points.last()
                && (l.points.len() - 1) % stride != 0
            {
                pts.push([px(last[0]), py(last[1])]);
            }
            match pts.len() {
                0 => {}
                1 => out.push(Prim::Rect {
                    rect: [
                        pts[0][0] - 2.0,
                        pts[0][1] - 2.0,
                        pts[0][0] + 2.0,
                        pts[0][1] + 2.0,
                    ],
                    ink: Ink::Series(k),
                }),
                _ => out.push(Prim::Polyline {
                    points: pts,
                    width: 2.0,
                    ink: Ink::Series(k),
                }),
            }
        }
        if let Some(t) = self.cursor
            && t >= self.x.min
            && t <= self.x.max
        {
            let x = px(t);
            out.push(Prim::Polyline {
                points: vec![[x, y0], [x, y1]],
                width: 1.5,
                ink: Ink::Cursor,
            });
        }
        out
    }

    /// The chart as an SVG document, `w` × `h` points, in the colours
    /// given (series hues, then text, secondary text, grid, cursor and
    /// background). Text widths are estimated (Inter: about 0.55 em a
    /// character), so a viewer's own font fits.
    pub fn svg(&self, w: f64, h: f64, colors: &ChartColorsResolved) -> String {
        let measure = |t: &str, pt: f64| t.chars().count() as f64 * 0.55 * pt;
        let prims = self.layout(w, h, true, &measure);
        let mut s = String::new();
        let _ = writeln!(
            s,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" font-family=\"Inter, sans-serif\">"
        );
        let _ = writeln!(s, "<title>{}</title>", xml(&self.title));
        for p in prims {
            match p {
                Prim::Rect { rect, ink } => {
                    let _ = writeln!(
                        s,
                        "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"{}\"/>",
                        rect[0],
                        rect[1],
                        rect[2] - rect[0],
                        rect[3] - rect[1],
                        colors.of(ink).hex()
                    );
                }
                Prim::Polyline { points, width, ink } => {
                    let pts: Vec<String> = points
                        .iter()
                        .map(|p| format!("{:.2},{:.2}", p[0], p[1]))
                        .collect();
                    let _ = writeln!(
                        s,
                        "<polyline points=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{width}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>",
                        pts.join(" "),
                        colors.of(ink).hex()
                    );
                }
                Prim::Text {
                    text,
                    at,
                    size,
                    anchor,
                    ink,
                } => {
                    let a = match anchor {
                        Anchor::Start => "start",
                        Anchor::Middle => "middle",
                        Anchor::End => "end",
                    };
                    let _ = writeln!(
                        s,
                        "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"{size}\" text-anchor=\"{a}\" fill=\"{}\">{}</text>",
                        at[0],
                        at[1],
                        colors.of(ink).hex(),
                        xml(&text)
                    );
                }
            }
        }
        s.push_str("</svg>\n");
        s
    }

    /// The chart's data for tools: axes, lines and bars in data units.
    pub fn data(&self) -> Value {
        json!({
            "chart": self.kind.key(),
            "title": self.title,
            "x": self.x,
            "y": self.y,
            "lines": self.lines,
            "bars": self.bars,
            "cursor": self.cursor,
            "notes": self.notes,
        })
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The colours a chart is drawn in, resolved from the tokens of a theme.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartColorsResolved {
    pub series: Vec<Rgb>,
    pub text: Rgb,
    pub text_secondary: Rgb,
    pub grid: Rgb,
    pub cursor: Rgb,
    pub background: Rgb,
}

impl ChartColorsResolved {
    pub fn new(tokens: &crate::Tokens, dark: bool) -> Self {
        let p = tokens.palette(dark);
        Self {
            series: tokens.chart.colors(dark).series.clone(),
            text: p.text_primary,
            text_secondary: p.text_secondary,
            grid: p.line,
            cursor: p.accent,
            background: p.bg_panel,
        }
    }

    pub fn of(&self, ink: Ink) -> Rgb {
        match ink {
            Ink::Series(k) => self.series.get(k).copied().unwrap_or(self.text_secondary),
            Ink::Text => self.text,
            Ink::TextSecondary => self.text_secondary,
            Ink::Grid => self.grid,
            Ink::Cursor => self.cursor,
            Ink::Background => self.background,
        }
    }
}

/// The charts of a session: the run's tables and what the window's chart
/// panel shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Charts {
    pub run: Run,
    /// The chart of the panel.
    pub spec: ChartSpec,
    /// Whether the window shows the chart panel.
    pub visible: bool,
}

impl Charts {
    pub fn new(run: Run) -> Self {
        let chart = if run.datums.iter().any(|d| d.liquid) {
            ChartKind::MassFlow
        } else if !run.injectors.is_empty() {
            ChartKind::Penetration
        } else {
            ChartKind::Columns
        };
        let mut spec = ChartSpec::new(chart);
        if chart == ChartKind::Columns
            && let Some(t) = &run.datums_table
            && let Some(c) = t.columns.iter().find(|c| c.ends_with(".mass_flow"))
        {
            spec.columns = vec![format!("datums:{c}")];
        }
        Self {
            run,
            spec,
            visible: true,
        }
    }
}

// ------------------------------------------------------ statistics

impl Run {
    /// The statistics at `step` (none: the last row): the spray's mass
    /// book and injectors from `spray.csv`, each datum's liquid from
    /// `datums.csv`, its histogram up to the step, and at the run's last
    /// step the end-of-run statistics of `result.json`. Each table says
    /// which of its rows it took (the last at or before the step).
    pub fn statistics(&self, step: Option<u64>) -> Value {
        let last = self.span().map(|s| s.0[1]).unwrap_or(0);
        let step = step.unwrap_or(last);
        let mut out = json!({
            "step": step,
            "time": self.time_of(step),
            "case": self.case,
        });
        if let Some(s) = &self.spray
            && let Some(r) = s.row_at(step)
        {
            let g = |c: &str| s.get(r, c);
            let injectors: Vec<Value> = self
                .injectors
                .iter()
                .map(|n| {
                    json!({
                        "name": n,
                        "parcels": g(&format!("{n}.parcels")),
                        "in_flow_kg": g(&format!("{n}.in_flow_kg")),
                        "penetration_m": g(&format!("{n}.penetration_m")),
                        "farthest_m": g(&format!("{n}.farthest_m")),
                    })
                })
                .collect();
            out["spray"] = json!({
                "row_step": s.rows[r][0],
                "parcels": g("parcels"),
                "injected_kg": g("injected_kg"),
                "in_flow_kg": g("in_flow_kg"),
                "deposited_kg": g("deposited_kg"),
                "escaped_kg": g("escaped_kg"),
                "lost_kg": g("lost_kg"),
                "lost_parcels": g("lost_parcels"),
                "d32_m": g("d32_m").filter(|v| *v > 0.0),
                "centre_of_mass_m": [g("com_x"), g("com_y"), g("com_z")],
                "injectors": injectors,
            });
        }
        let mut datums = Vec::new();
        for d in &self.datums {
            let mut v = json!({"name": d.name, "kind": d.kind});
            if let Some(t) = &self.datums_table
                && let Some(r) = t.row_at(step)
            {
                let g = |q: &str| t.get(r, &format!("{}.{q}", d.name));
                v["row_step"] = json!(t.rows[r][0]);
                match d.kind {
                    DatumKind::Surface => {
                        v["gas_mass_flow_kg_s"] = json!(g("mass_flow"));
                        if d.liquid {
                            let (p, n) = (g("liquid_kg_pos"), g("liquid_kg_neg"));
                            v["liquid"] = json!({
                                "kg_pos": p,
                                "kg_neg": n,
                                "kg_net": p.zip(n).map(|(p, n)| p - n),
                                "rate_kg_s": g("liquid_rate"),
                                "crossings": g("crossings"),
                                "d10_m": g("d10").filter(|v| *v > 0.0),
                                "d32_m": g("d32").filter(|v| *v > 0.0),
                                "dv50_m": g("dv50").filter(|v| *v > 0.0),
                            });
                        }
                    }
                    DatumKind::Volume => {
                        v["liquid"] = json!({
                            "kg": g("liquid_kg"),
                            "parcels": g("parcels"),
                            "d32_m": g("d32").filter(|v| *v > 0.0),
                        });
                    }
                }
            }
            if let Some(h) = self.histograms.get(&d.name)
                && let Some(k) = h.interval_at(step)
                && let Some(c) = h.cumulative(k)
            {
                let tot = c.total();
                v["histogram"] = json!({
                    "interval_step": h.intervals[k].step,
                    "droplets": tot[0],
                    "mass_kg": tot[1],
                    "dv10_m": c.mass_quantile(0.1),
                    "dv50_m": c.mass_quantile(0.5),
                    "dv90_m": c.mass_quantile(0.9),
                    "method": "from the cumulative histogram bins, linear in log d within a bin",
                });
            }
            datums.push(v);
        }
        out["datums"] = Value::Array(datums);
        if step >= last
            && let Some(r) = &self.result
        {
            out["end_of_run"] = r.clone();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    const DATUMS_CSV: &str = "# format foehncraft-datums 1; Foehncraft 1.0.1; case t\n\
# units: step -; time s\n\
step,time,a.area,a.mass_flow,a.liquid_kg_pos,a.liquid_kg_neg,a.liquid_rate,a.crossings,a.d10,a.d32,a.dv50,b.volume,b.liquid_kg,b.parcels,b.d32\n\
10,0.05,1,2,0,0,0,0,0,0,0,1,0,0,0\n\
20,0.10,1,2,1e-3,0,2e-2,5,1e-5,3e-5,4e-5,1,2e-4,7,2e-5\n\
30,0.15,1,2,3e-3,1e-4,4e-2,9,1e-5,3e-5,4e-5,1,1e-4,3,2e-5\n";

    /// Invariant: a datums table's datums, kinds and spray columns come
    /// from its header, and the row of a step is the last at or before
    /// it. Breaks if the group parsing or `row_at` slips by one; no other
    /// test reads a header.
    #[test]
    fn datums_and_rows_from_the_header() {
        let t = Table::parse(Path::new("datums.csv"), DATUMS_CSV, "foehncraft-datums").unwrap();
        assert_eq!(t.case.as_deref(), Some("t"));
        let d = datums_of(&t);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].kind, d[0].liquid), (DatumKind::Surface, true));
        assert_eq!((d[1].kind, d[1].liquid), (DatumKind::Volume, true));
        assert_eq!(t.row_at(5), Some(0));
        assert_eq!(t.row_at(20), Some(1));
        assert_eq!(t.row_at(29), Some(1));
        assert_eq!(t.row_at(1000), Some(2));
    }

    /// Invariant: another format or version is refused with what was
    /// found, so a changed solver output never draws wrong numbers.
    #[test]
    fn other_formats_and_versions_are_refused() {
        let v2 = DATUMS_CSV.replace("datums 1", "datums 2");
        let e = Table::parse(Path::new("d.csv"), &v2, "foehncraft-datums").unwrap_err();
        assert!(e.0.contains("version 2"), "{e}");
        let e = Table::parse(Path::new("d.csv"), DATUMS_CSV, "foehncraft-spray").unwrap_err();
        assert!(e.0.contains("expected foehncraft-spray"), "{e}");
    }

    /// Invariant: the last row a running solver has cut short is left
    /// out; a short row in the middle is an error.
    #[test]
    fn a_cut_last_row_is_left_out() {
        let cut = format!("{DATUMS_CSV}40,0.2,1,2");
        let t = Table::parse(Path::new("d.csv"), &cut, "foehncraft-datums").unwrap();
        assert_eq!(t.rows.len(), 3);
        let mid = DATUMS_CSV.replace("20,0.10,1,2,1e-3", "20,0.10");
        assert!(Table::parse(Path::new("d.csv"), &mid, "foehncraft-datums").is_err());
    }

    fn hist() -> Histogram {
        let text = "# format foehncraft-spray-histogram 1; Foehncraft 1.0.1; case t\n\
step,time,bin,d_lo,d_hi,droplets,mass\n\
10,0.05,-1,0,1e-6,0,0\n10,0.05,0,1e-6,1e-5,10,1\n10,0.05,1,1e-5,1e-4,1,3\n10,0.05,2,1e-4,inf,0,0\n\
20,0.10,-1,0,1e-6,0,0\n20,0.10,0,1e-6,1e-5,0,0\n20,0.10,1,1e-5,1e-4,2,6\n20,0.10,2,1e-4,inf,0,0\n";
        let t = Table::parse(Path::new("h.csv"), text, "foehncraft-spray-histogram").unwrap();
        Histogram::from_table("a", &t).unwrap()
    }

    /// Invariant: intervals split by step, the under and over bins stay
    /// out of the bars, the cumulative histogram sums the intervals, and
    /// the mass median interpolates in log d. Breaks if the bin −1 or
    /// `inf` row became a bar or the sum skipped an interval.
    #[test]
    fn histograms_sum_and_give_quantiles() {
        let h = hist();
        assert_eq!(h.intervals.len(), 2);
        assert_eq!(h.intervals[0].edges, vec![1e-6, 1e-5, 1e-4]);
        let c = h.cumulative(1).unwrap();
        assert_eq!(c.mass, vec![1.0, 9.0]);
        assert_eq!(c.total(), [13.0, 10.0]);
        // Half the mass (5) lies 4/9 into the second bin.
        let d = c.mass_quantile(0.5).unwrap();
        let want = (1e-5f64.ln() + 4.0 / 9.0 * 10f64.ln()).exp();
        assert!((d - want).abs() < 1e-15, "{d} {want}");
        assert_eq!(h.interval_at(15), Some(0));
    }

    /// Invariant: nice ticks cover the data and step by 1, 2 or 5 times
    /// a power of ten; a length of micrometres shows in µm.
    #[test]
    fn nice_ticks_and_units() {
        let a = linear_axis("x".into(), "m", 0.0, 7.3e-5, 5);
        assert!(a.min <= 0.0 && a.max >= 7.3e-5);
        assert_eq!(a.unit, "µm");
        let step = a.ticks[1] - a.ticks[0];
        let m = step / 10f64.powf(step.log10().floor());
        assert!(
            [1.0, 2.0, 5.0].iter().any(|n| (m - n).abs() < 1e-9),
            "{step}"
        );
        assert_eq!(a.tick_label(2e-5), "20");
        let l = log_axis("d".into(), "m", 1e-6, 1e-3);
        assert_eq!(l.ticks.len(), 4);
        assert!((l.position(1e-6)).abs() < 1e-12 && (l.position(1e-3) - 1.0).abs() < 1e-12);
    }

    /// Invariant: the layout keeps every primitive inside its rectangle
    /// and draws one polyline per series with its own hue in order.
    #[test]
    fn layout_stays_inside() {
        let i = I18n::english().unwrap();
        let x = linear_axis("t".into(), "s", 0.0, 1.0, 6);
        let y = linear_axis("v".into(), "kg/s", 0.0, 2e-3, 5);
        let c = Chart {
            kind: ChartKind::MassFlow,
            title: i.tr("chart-mass-flow"),
            x,
            y,
            lines: vec![
                Line {
                    name: "a".into(),
                    points: vec![[0.0, 0.0], [1.0, 1e-3]],
                },
                Line {
                    name: "b".into(),
                    points: vec![[0.0, 2e-3], [1.0, 0.0]],
                },
            ],
            bars: vec![],
            cursor: Some(0.5),
            notes: vec![],
        };
        let p = c.layout(400.0, 240.0, true, &|t, pt| t.len() as f64 * 0.55 * pt);
        let mut series = vec![];
        for q in &p {
            match q {
                Prim::Rect { rect, .. } => {
                    assert!(
                        rect[0] >= 0.0 && rect[2] <= 400.0 && rect[1] >= 0.0 && rect[3] <= 240.0
                    )
                }
                Prim::Polyline { points, ink, .. } => {
                    for v in points {
                        assert!(v[0] >= 0.0 && v[0] <= 400.0 && v[1] >= 0.0 && v[1] <= 240.0);
                    }
                    if let Ink::Series(k) = ink {
                        series.push(*k);
                    }
                }
                Prim::Text { at, .. } => assert!(at[1] <= 240.0),
            }
        }
        assert_eq!(series, vec![0, 1]);
        assert!(p.iter().any(|q| matches!(
            q,
            Prim::Polyline {
                ink: Ink::Cursor,
                ..
            }
        )));
    }

    fn fixture() -> Run {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/spray-run");
        Run::read(&dir).unwrap()
    }

    /// Invariant: the tables FoehnCraft wrote (#265, `spray_datums`) read
    /// with their datums, injector and histograms, and the statistics at
    /// the last step agree with the end-of-run numbers of `result.json`
    /// (cumulative liquid through the outlet = its `liquid.mass_pos`).
    /// Breaks when FoehnCraft's layout changes; no synthetic table checks
    /// the real header.
    #[test]
    fn reads_a_foehncraft_spray_run() {
        let r = fixture();
        assert_eq!(r.case.as_deref(), Some("spray_datums"));
        let names: Vec<&str> = r.datums.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            ["x06", "x12", "disc", "x16", "outlet", "floor", "nozzle_box"]
        );
        assert_eq!(r.datums[6].kind, DatumKind::Volume);
        assert_eq!(r.injectors, ["nozzle"]);
        assert_eq!(r.histograms.len(), 3);
        let s = r.statistics(None);
        assert_eq!(s["step"], 300);
        let outlet = s["datums"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == "outlet")
            .unwrap();
        let end = r.result.as_ref().unwrap()["datums"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == "outlet")
            .unwrap()["liquid"]["mass_pos"]
            .as_f64()
            .unwrap();
        let ours = outlet["liquid"]["kg_pos"].as_f64().unwrap();
        assert!((ours - end).abs() <= 1e-12 * end, "{ours} {end}");
        assert!(s["end_of_run"].is_object());
        // The binned median is near the solver's exact one.
        let dv50 = outlet["histogram"]["dv50_m"].as_f64().unwrap();
        let exact = outlet["liquid"]["dv50_m"].as_f64().unwrap();
        assert!((dv50 / exact - 1.0).abs() < 0.1, "{dv50} {exact}");
        // Earlier steps take the row at or before.
        let s = r.statistics(Some(155));
        assert_eq!(s["spray"]["row_step"], 155.0);
        assert_eq!(s["datums"][0]["row_step"], 150.0);
        assert!(s.get("end_of_run").is_none());
    }

    /// Invariant: every chart of the fixture builds, a chart never has
    /// more series than hues, a mixed-unit columns chart is refused, and
    /// an unknown datum is refused with the names the run has.
    #[test]
    fn charts_of_a_spray_run() {
        let r = fixture();
        let i = I18n::english().unwrap();
        for k in ChartKind::ALL {
            let mut spec = ChartSpec::new(k);
            if k == ChartKind::Columns {
                spec.columns = vec!["spray:in_flow_kg".into(), "spray:escaped_kg".into()];
            }
            let c = r.chart(&spec, Some(150), &i).unwrap();
            assert!(c.lines.len() <= MAX_SERIES);
            assert!(!c.lines.is_empty() || !c.bars.is_empty(), "{k:?}");
            let svg = c.svg(
                640.0,
                360.0,
                &ChartColorsResolved::new(&crate::Tokens::builtin().unwrap(), false),
            );
            assert!(svg.starts_with("<svg"));
        }
        let mut spec = ChartSpec::new(ChartKind::Columns);
        spec.columns = vec!["spray:in_flow_kg".into(), "spray:d32_m".into()];
        let e = r.chart(&spec, None, &i).unwrap_err();
        assert!(e.0.contains("one axis"), "{e}");
        let mut spec = ChartSpec::new(ChartKind::MassFlow);
        spec.datums = vec!["nope".into()];
        let e = r.chart(&spec, None, &i).unwrap_err();
        assert!(e.0.contains("x06"), "{e}");
    }
}
