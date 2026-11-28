//! The tool registry: every step of the window and the text mode is one
//! of these tools, JSON in and out, so `kraxn call`, a plan file and the
//! window do the same thing (headless first, AGENTS.md).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::actions::{Elevator, describe, run_all};
use crate::catalogue::Catalogue;
use crate::diag::{CODES, Diagnostic, Res, diag};
use crate::fetch::Source;
use crate::install::{self, Ctx, Event};
use crate::installed;
use crate::layout::{Env, Layout, Scope};
use crate::plan::Plan;
use crate::platform::Platform;

pub const RESULT_FORMAT: &str = "kraxn.result";

/// The answer of every tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub format: String,
    pub format_version: String,
    pub tool: String,
    /// `finished` or `failed`.
    pub status: String,
    /// 0 finished, 1 failed, 2 unknown tool or bad input.
    pub exit_code: i32,
    pub result: Value,
    pub diagnostics: Vec<Diagnostic>,
}

/// A failed call: its diagnostic and what it did up to there (a failed
/// `plan.apply` still reports its steps and checks).
pub struct Fail {
    pub diag: Diagnostic,
    pub result: Value,
}

impl From<Diagnostic> for Fail {
    fn from(diag: Diagnostic) -> Self {
        Self {
            diag,
            result: Value::Null,
        }
    }
}

type Out = Result<Value, Fail>;

impl Envelope {
    fn of(tool: &str, r: Out) -> Self {
        let (status, exit_code, result, diagnostics) = match r {
            Ok(v) => ("finished", 0, v, Vec::new()),
            Err(Fail { diag: d, result }) => {
                let code = if d.code.starts_with("TOOL") { 2 } else { 1 };
                ("failed", code, result, vec![d])
            }
        };
        Self {
            format: RESULT_FORMAT.into(),
            format_version: "1.0".into(),
            tool: tool.into(),
            status: status.into(),
            exit_code,
            result,
            diagnostics,
        }
    }

    pub fn ok(&self) -> bool {
        self.exit_code == 0
    }
}

/// What a tool needs from its front end.
pub struct Host<'a> {
    pub catalogue: &'a Catalogue,
    pub platform: Platform,
    pub env: &'a Env,
    /// Folder for downloads and unpacked files.
    pub work: PathBuf,
    pub elevator: &'a mut dyn Elevator,
    pub progress: &'a mut dyn FnMut(Event),
    /// Instead of GitHub (tests).
    pub source: Option<Source>,
}

pub struct Tool {
    pub id: &'static str,
    pub summary: &'static str,
    /// The input fields, `name: meaning`.
    pub input: &'static [&'static str],
    /// Changes nothing on the computer.
    pub reads_only: bool,
    run: fn(&mut Host, &Value) -> Out,
}

/// Every tool.
pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            id: "system.detect",
            summary: "The system, its folders for both scopes, whether Kraxn runs as administrator and whether a desktop is there.",
            input: &[],
            reads_only: true,
            run: system_detect,
        },
        Tool {
            id: "catalogue.list",
            summary: "The products Kraxn installs, with their programs and source-build needs.",
            input: &[],
            reads_only: true,
            run: catalogue_list,
        },
        Tool {
            id: "release.list",
            summary: "The releases of a product, newest first, and whether each has a file for this system.",
            input: &[
                "product: murmelcraft, koflcraft or foehncraft",
                "offline_dir: a folder of downloaded files (optional)",
            ],
            reads_only: true,
            run: release_list,
        },
        Tool {
            id: "plan.check",
            summary: "Checks a plan and returns its steps, with the ones that need administrator rights marked, and for source builds the tools found and how each missing one gets installed (`blocked` when only you can), without running anything.",
            input: &["plan: the plan (as in a plan file)"],
            reads_only: true,
            run: plan_check,
        },
        Tool {
            id: "plan.apply",
            summary: "Runs a plan: download, SHA-256 check (or, for a source build, the missing tools, the source and cargo, with cargo's output as events), install, default version, PATH, and a start check of each program.",
            input: &["plan: the plan (as in a plan file)"],
            reads_only: false,
            run: plan_apply,
        },
        Tool {
            id: "installed.list",
            summary: "The installed products and versions of a scope, with the default version.",
            input: &[
                "scope: user or system (default user)",
                "prefix: another install root (optional)",
            ],
            reads_only: true,
            run: installed_list,
        },
        Tool {
            id: "version.activate",
            summary: "Makes an installed version the default (current, module files, links).",
            input: &["product", "version", "scope", "prefix (optional)"],
            reads_only: false,
            run: version_activate,
        },
        Tool {
            id: "version.remove",
            summary: "Removes an installed version, or the whole product without a version; another version becomes the default.",
            input: &[
                "product",
                "version (optional: all)",
                "scope",
                "prefix (optional)",
                "dry_run: only list the steps",
            ],
            reads_only: false,
            run: version_remove,
        },
        Tool {
            id: "diag.codes",
            summary: "Every diagnostic code with its meaning.",
            input: &[],
            reads_only: true,
            run: diag_codes,
        },
    ]
}

/// Calls the tool `id` with `input`.
pub fn call(host: &mut Host, id: &str, input: &Value) -> Envelope {
    let all = tools();
    let r = match all.iter().find(|t| t.id == id) {
        Some(t) => (t.run)(host, input),
        None => Err(diag(
            "TOOL001",
            format!("unknown tool {id}; `kraxn tools` lists them"),
        )
        .into()),
    };
    Envelope::of(id, r)
}

/// `docs/kraxn-tools.md`.
pub fn markdown() -> String {
    let mut s = String::from(
        "# Kraxn tools\n\nGenerated by `cargo run -p kraxn -- tools --markdown`; a test checks it.\n\
         Every tool takes JSON and answers with a `kraxn.result` envelope:\n\
         `kraxn call TOOL --input '{...}'`.\n\n| Tool | What it does | Input | Changes |\n|---|---|---|---|\n",
    );
    for t in tools() {
        let input = if t.input.is_empty() {
            "none".to_string()
        } else {
            t.input.join("; ")
        };
        s.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            t.id,
            t.summary,
            input,
            if t.reads_only { "nothing" } else { "files" }
        ));
    }
    s.push_str("\n## Diagnostic codes\n\n| Code | Meaning |\n|---|---|\n");
    for (c, m) in CODES {
        s.push_str(&format!("| `{c}` | {m} |\n"));
    }
    s
}

fn bad(e: impl std::fmt::Display) -> Diagnostic {
    diag("TOOL002", e.to_string())
}

fn field<'a>(v: &'a Value, k: &str) -> Res<&'a str> {
    v.get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| bad(format!("the input needs \"{k}\"")))
}

fn layout_of(h: &Host, v: &Value) -> Res<Layout> {
    let scope: Scope = match v.get("scope") {
        Some(s) => serde_json::from_value(s.clone()).map_err(bad)?,
        None => Scope::User,
    };
    let prefix = v.get("prefix").and_then(Value::as_str).map(PathBuf::from);
    Layout::new(scope, h.platform, h.env, prefix.as_deref())
}

fn ctx<'a>(h: &'a Host) -> Ctx<'a> {
    Ctx {
        catalogue: h.catalogue,
        platform: h.platform,
        env: h.env,
    }
}

fn source_of(h: &Host, plan: &Plan) -> Source {
    match (&plan.offline_dir, &h.source) {
        (Some(d), _) => Source::Folder(d.clone()),
        (None, Some(s)) => s.clone(),
        (None, None) => Source::github(),
    }
}

fn plan_of(v: &Value) -> Res<Plan> {
    let p = v
        .get("plan")
        .ok_or_else(|| bad("the input needs \"plan\""))?;
    serde_json::from_value(p.clone())
        .map_err(|e| diag("KRX010", format!("the plan is not valid: {e}")))
}

/// Whether this process has administrator rights.
pub fn is_admin() -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        // `net session` succeeds only in an elevated process
        std::process::Command::new("net")
            .arg("session")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Whether a window can open (a desktop on Linux; always on macOS and
/// Windows).
pub fn has_desktop(platform: &Platform) -> bool {
    match platform.os {
        crate::platform::Os::Linux => ["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty())),
        _ => true,
    }
}

fn system_detect(h: &mut Host, _: &Value) -> Out {
    let user = Layout::new(Scope::User, h.platform, h.env, None).ok();
    let system = Layout::new(Scope::System, h.platform, h.env, None).ok();
    Ok(json!({
        "platform": h.platform,
        "label": h.platform.label(),
        "release_files": h.platform.asset_key(),
        "admin": is_admin(),
        "desktop": has_desktop(&h.platform),
        "user": user,
        "system": system,
    }))
}

fn catalogue_list(h: &mut Host, _: &Value) -> Out {
    Ok(serde_json::to_value(&h.catalogue.products).map_err(bad)?)
}

fn release_list(h: &mut Host, v: &Value) -> Out {
    let p = h.catalogue.get(field(v, "product")?)?;
    let source = match v.get("offline_dir").and_then(Value::as_str) {
        Some(d) => Source::Folder(d.into()),
        None => h.source.clone().unwrap_or_else(Source::github),
    };
    let rels = source.releases(p)?;
    let list: Vec<Value> = rels
        .iter()
        .map(|r| {
            json!({
                "version": r.version,
                "prerelease": r.prerelease,
                "file": r.archive_for(p, &h.platform).map(|f| f.name.clone()),
                "size": r.archive_for(p, &h.platform).map(|f| f.size),
            })
        })
        .collect();
    Ok(json!({"product": p.id, "system": h.platform.asset_key(), "releases": list}))
}

fn steps_json(steps: &[crate::actions::Step]) -> Value {
    Value::Array(
        steps
            .iter()
            .map(|s| {
                json!({
                    "title": s.title,
                    "admin": s.admin,
                    "actions": s.actions.iter().map(describe).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

fn plan_check(h: &mut Host, v: &Value) -> Out {
    let plan = plan_of(v)?;
    let c = ctx(h);
    let source = source_of(h, &plan);
    let resolved = install::resolve(&c, &plan, &source)?;
    let tools = install::tools(&c, &plan, &h.work)?;
    let steps = install::preview(&c, &plan, &source, &resolved, &tools)?;
    let layout = c.layout(&plan)?;
    Ok(json!({
        "tools": tools,
        "blocked": tools.iter().any(|t| t.blocks()),
        "versions": resolved.iter().map(|(p, r)| json!({"product": p.id, "version": r.version})).collect::<Vec<_>>(),
        "root": layout.root,
        "bin": layout.bin,
        "needs_admin": steps.iter().any(|s| s.admin),
        "steps": steps_json(&steps),
    }))
}

fn plan_apply(h: &mut Host, v: &Value) -> Out {
    let plan = plan_of(v)?;
    let source = source_of(h, &plan);
    let c = Ctx {
        catalogue: h.catalogue,
        platform: h.platform,
        env: h.env,
    };
    let work = h.work.clone();
    let report = install::apply(&c, &plan, &source, &work, h.elevator, h.progress)?;
    let value = serde_json::to_value(&report).map_err(bad)?;
    if report.ok {
        return Ok(value);
    }
    let first = report
        .outcomes
        .iter()
        .find_map(|o| o.error.clone())
        .or_else(|| {
            report.products.iter().find_map(|p| {
                p.check
                    .as_ref()
                    .filter(|c| !c.ok)
                    .map(|c| diag("KRX011", format!("{}: {}", c.command, c.message)))
            })
        })
        .unwrap_or_else(|| diag("KRX007", "a step did not run"));
    Err(Fail {
        diag: first,
        result: value,
    })
}

fn installed_list(h: &mut Host, v: &Value) -> Out {
    let layout = layout_of(h, v)?;
    let list = installed::all(&layout, h.catalogue);
    Ok(json!({"root": layout.root, "bin": layout.bin, "products": list}))
}

fn run_or_list(h: &mut Host, v: &Value, steps: Vec<crate::actions::Step>) -> Out {
    if v.get("dry_run").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(json!({"steps": steps_json(&steps), "ran": false}));
    }
    let out = run_all(&steps, h.elevator);
    if let Some(e) = out.iter().find_map(|o| o.error.clone()) {
        return Err(e.into());
    }
    Ok(json!({"steps": steps_json(&steps), "ran": true}))
}

fn version_activate(h: &mut Host, v: &Value) -> Out {
    let layout = layout_of(h, v)?;
    let steps = install::activate(&ctx(h), &layout, field(v, "product")?, field(v, "version")?)?;
    run_or_list(h, v, steps)
}

fn version_remove(h: &mut Host, v: &Value) -> Out {
    let layout = layout_of(h, v)?;
    let version = v.get("version").and_then(Value::as_str);
    let steps = install::remove(&ctx(h), &layout, field(v, "product")?, version)?;
    run_or_list(h, v, steps)
}

fn diag_codes(_: &mut Host, _: &Value) -> Out {
    Ok(Value::Array(
        CODES
            .iter()
            .map(|(c, m)| json!({"code": c, "meaning": m}))
            .collect(),
    ))
}
