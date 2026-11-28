//! The text mode: the same questions as the window, in a terminal (over
//! SSH on a server without a desktop). Every answer goes into a plan, and
//! every action is a tool call (`plan.check`, `plan.apply`, ...).

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use kraxn_core::install::Event;
use kraxn_core::tools::Envelope;
use kraxn_core::{Layout, Origin, Plan, Scope, Want};
use serde_json::{Value, json};

use crate::i18n::I18n;
use crate::runner::Runner;

pub struct Text<'a, R: BufRead, W: Write> {
    pub t: I18n,
    pub input: R,
    pub out: W,
    pub runner: &'a mut Runner,
}

fn eof() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "end of input")
}

impl<R: BufRead, W: Write> Text<'_, R, W> {
    fn say(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")
    }

    fn line(&mut self, prompt: &str) -> io::Result<String> {
        write!(self.out, "{prompt}: ")?;
        self.out.flush()?;
        let mut s = String::new();
        if self.input.read_line(&mut s)? == 0 {
            return Err(eof());
        }
        Ok(s.trim().to_string())
    }

    /// One of `options` (numbered from 1); Enter takes `default`.
    fn choose(&mut self, title: &str, options: &[String], default: usize) -> io::Result<usize> {
        self.say(&format!("\n{title}"))?;
        for (i, o) in options.iter().enumerate() {
            self.say(&format!("  {}) {o}", i + 1))?;
        }
        loop {
            let p = self.t.tr_args(
                "ask-choice-default",
                &[("default", (default + 1).to_string())],
            );
            let a = self.line(&p)?;
            if a.is_empty() {
                return Ok(default);
            }
            match a.parse::<usize>() {
                Ok(n) if n >= 1 && n <= options.len() => return Ok(n - 1),
                _ => {
                    let m = self.t.tr("ask-bad-choice");
                    self.say(&m)?;
                }
            }
        }
    }

    fn yes(&mut self, question: &str, default: bool) -> io::Result<bool> {
        let key = if default {
            "ask-yes-no-yes"
        } else {
            "ask-yes-no-no"
        };
        let p = self.t.tr_args(key, &[("question", question.to_string())]);
        let a = self.line(&p)?.to_lowercase();
        Ok(match a.as_str() {
            "" => default,
            _ => a.starts_with('y') || a.starts_with('j'),
        })
    }

    fn fail(&mut self, e: &Envelope) -> io::Result<()> {
        let m = e
            .diagnostics
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        let s = self.t.tr_args("done-failed", &[("message", m)]);
        self.say(&s)
    }

    /// The main menu until Quit (or the end of the input).
    pub fn run(&mut self) -> io::Result<()> {
        let s = self
            .t
            .tr_args("app-line", &[("version", env!("CARGO_PKG_VERSION").into())]);
        self.say(&s)?;
        let p = self.runner.platform;
        let s = self.t.tr_args(
            "app-system",
            &[("system", p.label()), ("files", p.asset_key().into())],
        );
        self.say(&s)?;
        loop {
            let options = [
                self.t.tr("menu-install"),
                self.t.tr("menu-default"),
                self.t.tr("menu-remove"),
                self.t.tr("menu-list"),
                self.t.tr("menu-quit"),
            ];
            let title = self.t.tr("menu-title");
            let r = match self.choose(&title, &options, 0) {
                Ok(c) => match c {
                    0 => self.install(),
                    1 => self.change_default(),
                    2 => self.remove(),
                    3 => self.list(),
                    _ => return Ok(()),
                },
                Err(e) => Err(e),
            };
            match r {
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(e) => return Err(e),
                Ok(()) => {}
            }
        }
    }

    fn layout(&self, scope: Scope, prefix: Option<&std::path::Path>) -> Option<Layout> {
        Layout::new(scope, self.runner.platform, &self.runner.env, prefix).ok()
    }

    /// For whom, and in which folder.
    fn ask_scope(&mut self) -> io::Result<(Scope, Option<PathBuf>)> {
        let root = |me: &Self, s| {
            me.layout(s, None)
                .map(|l| l.root.display().to_string())
                .unwrap_or_default()
        };
        let (ru, rs) = (root(self, Scope::User), root(self, Scope::System));
        let options = [
            self.t.tr_args("scope-user", &[("root", ru.clone())]),
            self.t.tr_args("scope-system", &[("root", rs.clone())]),
        ];
        let title = self.t.tr("scope-title");
        let scope = if self.choose(&title, &options, 0)? == 0 {
            Scope::User
        } else {
            Scope::System
        };
        let p = self.t.tr_args(
            "folder-ask",
            &[("root", if scope == Scope::User { ru } else { rs })],
        );
        let f = self.line(&p)?;
        Ok((scope, (!f.is_empty()).then(|| PathBuf::from(f))))
    }

    fn ask_source(&mut self) -> io::Result<Option<PathBuf>> {
        let options = [self.t.tr("source-online"), self.t.tr("source-folder")];
        let title = self.t.tr("source-title");
        if self.choose(&title, &options, 0)? == 0 {
            return Ok(None);
        }
        loop {
            let p = self.t.tr("source-folder-ask");
            let f = PathBuf::from(self.line(&p)?);
            if f.is_dir() {
                return Ok(Some(f));
            }
            let m = self.t.tr("source-folder-missing");
            self.say(&m)?;
        }
    }

    fn installed(&mut self, scope: Scope, prefix: &Option<PathBuf>) -> Value {
        let mut input = json!({"scope": scope});
        if let Some(p) = prefix {
            input["prefix"] = json!(p);
        }
        self.runner.quiet("installed.list", &input).result
    }

    fn install(&mut self) -> io::Result<()> {
        let (scope, prefix) = self.ask_scope()?;
        let offline = self.ask_source()?;
        let have = self.installed(scope, &prefix);
        let products = self.runner.catalogue.products.clone();
        let mut latest: Vec<Option<String>> = Vec::new();
        // the newest release to build from source, when there is no file
        let mut buildable: Vec<Option<String>> = Vec::new();
        let mut rows = Vec::new();
        for p in &products {
            let mut input = json!({"product": p.id});
            if let Some(d) = &offline {
                input["offline_dir"] = json!(d);
            }
            let e = self.runner.quiet("release.list", &input);
            let l = e.result["releases"].as_array().and_then(|rs| {
                rs.iter()
                    .find(|r| !r["prerelease"].as_bool().unwrap_or(false) && r["file"].is_string())
                    .and_then(|r| r["version"].as_str().map(str::to_string))
            });
            let b = e.result["releases"]
                .as_array()
                .and_then(|rs| {
                    rs.iter()
                        .find(|r| !r["prerelease"].as_bool().unwrap_or(false))
                        .and_then(|r| r["version"].as_str().map(str::to_string))
                })
                .filter(|_| offline.is_none() && !p.build_paths.is_empty());
            let versions: Vec<String> = have["products"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|i| i["product"] == p.id)
                .flat_map(|i| i["versions"].as_array().cloned().unwrap_or_default())
                .filter_map(|v| v["version"].as_str().map(str::to_string))
                .collect();
            let latest_s = match (&l, &b) {
                (Some(v), _) => self.t.tr_args("products-latest", &[("version", v.clone())]),
                (None, Some(v)) => self
                    .t
                    .tr_args("products-buildable", &[("version", v.clone())]),
                (None, None) => self.t.tr("products-no-file"),
            };
            let inst = if versions.is_empty() {
                String::new()
            } else {
                self.t
                    .tr_args("products-installed", &[("versions", versions.join(", "))])
            };
            let line = self.t.tr(&format!("product-{}", p.id));
            rows.push(format!(
                "{}\n     {}",
                self.t.tr_args(
                    "products-row",
                    &[
                        ("title", p.title.clone()),
                        ("latest", latest_s),
                        ("installed", inst)
                    ]
                ),
                line
            ));
            latest.push(l);
            buildable.push(b);
        }
        let title = self.t.tr("products-title");
        self.say(&format!("\n{title}"))?;
        for (i, r) in rows.iter().enumerate() {
            self.say(&format!("  {}) {r}", i + 1))?;
        }
        let p = self.t.tr("products-ask");
        let a = self.line(&p)?;
        let chosen: Vec<usize> = if a.is_empty() {
            (0..products.len())
                .filter(|&i| latest[i].is_some())
                .collect()
        } else {
            a.split([',', ' '])
                .filter_map(|n| n.trim().parse::<usize>().ok())
                .filter(|n| *n >= 1 && *n <= products.len())
                .map(|n| n - 1)
                .collect()
        };
        if chosen.is_empty() {
            let m = self.t.tr("products-none");
            return self.say(&m);
        }
        let mut plan = Plan {
            scope,
            prefix: prefix.clone(),
            offline_dir: offline,
            ..Plan::default()
        };
        let mut any_source = false;
        for i in chosen {
            let p = &products[i];
            let source = match (&latest[i], &buildable[i]) {
                (None, None) => {
                    let m = format!("{}: {}", p.title, self.t.tr("products-no-file"));
                    self.say(&m)?;
                    continue;
                }
                (None, Some(v)) => {
                    let q = self.t.tr_args(
                        "build-ask",
                        &[("title", p.title.clone()), ("version", v.clone())],
                    );
                    if !self.yes(&q, true)? {
                        continue;
                    }
                    true
                }
                (Some(_), Some(_)) => {
                    let q = self.t.tr_args("from-ask", &[("title", p.title.clone())]);
                    self.line(&format!("{q} [r]"))?
                        .to_lowercase()
                        .starts_with('s')
                }
                (Some(_), None) => false,
            };
            let l = if source {
                buildable[i].clone()
            } else {
                latest[i].clone()
            }
            .unwrap_or_else(|| "latest".into());
            let q = self.t.tr_args(
                "version-ask",
                &[("title", p.title.clone()), ("latest", l.clone())],
            );
            let v = self.line(&q)?;
            let mut w = Want::new(&p.id);
            if !v.is_empty() {
                w.version = v.trim_start_matches('v').to_string();
            }
            if source {
                any_source = true;
                w.from = Origin::Source;
                for f in &p.features {
                    let about = self.t.tr(&format!("feature-{f}"));
                    let q = self.t.tr_args(
                        "feature-ask",
                        &[
                            ("title", p.title.clone()),
                            ("feature", f.clone()),
                            ("about", about),
                        ],
                    );
                    if self.yes(&q, false)? {
                        w.features.push(f.clone());
                    }
                }
            }
            let others = have["products"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|x| x["product"] == p.id);
            if others {
                let shown = if w.version == "latest" {
                    l.clone()
                } else {
                    w.version.clone()
                };
                let q = self.t.tr_args(
                    "default-ask",
                    &[("title", p.title.clone()), ("version", shown)],
                );
                w.make_default = self.yes(&q, true)?;
            }
            plan.installs.push(w);
        }
        if plan.installs.is_empty() {
            let m = self.t.tr("products-none");
            return self.say(&m);
        }
        if any_source {
            let q = self.t.tr("native-ask");
            let native = self.yes(&q, false)?;
            for w in plan
                .installs
                .iter_mut()
                .filter(|w| w.from == Origin::Source)
            {
                w.native = native;
            }
        }
        let q = self.t.tr("path-ask");
        plan.path = self.yes(&q, true)?;
        self.confirm_and_apply(plan)
    }

    fn show_steps(&mut self, steps: &Value) -> io::Result<()> {
        let title = self.t.tr("steps-title");
        self.say(&format!("\n{title}"))?;
        let admin = self.t.tr("steps-admin");
        for s in steps.as_array().into_iter().flatten() {
            let mark = if s["admin"].as_bool().unwrap_or(false) {
                format!(" {admin}")
            } else {
                String::new()
            };
            self.say(&format!("  - {}{mark}", s["title"].as_str().unwrap_or("")))?;
            for a in s["actions"].as_array().into_iter().flatten() {
                self.say(&format!("      {}", a.as_str().unwrap_or("")))?;
            }
        }
        Ok(())
    }

    /// Shows the plan's steps, offers to save it, runs it on a yes.
    pub fn confirm_and_apply(&mut self, plan: Plan) -> io::Result<()> {
        let input = json!({"plan": plan});
        let e = self.runner.quiet("plan.check", &input);
        if !e.ok() {
            return self.fail(&e);
        }
        if let Some(tools) = e.result["tools"].as_array().filter(|t| !t.is_empty()) {
            let title = self.t.tr("tools-title");
            self.say(&format!("\n{title}"))?;
            for t in tools {
                let line = tool_line(&self.t, t);
                self.say(&format!("  {line}"))?;
            }
            if e.result["blocked"].as_bool().unwrap_or(false) {
                let m = self.t.tr("tools-blocked");
                return self.say(&m);
            }
        }
        let steps = e.result["steps"].clone();
        self.show_steps(&steps)?;
        let q = self.t.tr("save-ask");
        let f = self.line(&q)?;
        if !f.is_empty() {
            match std::fs::write(&f, plan.to_toml()) {
                Ok(()) => {
                    let m = self.t.tr_args("saved", &[("file", f.clone())]);
                    self.say(&m)?;
                }
                Err(err) => {
                    let m = self
                        .t
                        .tr_args("done-failed", &[("message", format!("{f}: {err}"))]);
                    self.say(&m)?;
                }
            }
        }
        let q = self.t.tr("go-ask");
        if !self.yes(&q, true)? {
            let m = self.t.tr("cancelled");
            return self.say(&m);
        }
        self.apply(&input)
    }

    /// Runs `plan.apply` with progress lines and prints the report.
    pub fn apply(&mut self, input: &Value) -> io::Result<()> {
        let mut lines: Vec<String> = Vec::new();
        let mut last = (String::new(), u64::MAX);
        let t = &self.t;
        let out = &mut self.out;
        let e = self.runner.call("plan.apply", input, &mut |ev| match ev {
            Event::Info { text } => {
                let _ = writeln!(out, "{text}");
            }
            Event::Download { file, done, total } => {
                let pct = (done * 100).checked_div(total).unwrap_or(0);
                let tenth = pct / 10;
                if last.0 != file || last.1 != tenth {
                    last = (file.clone(), tenth);
                    let _ = writeln!(
                        out,
                        "{}",
                        t.tr_args(
                            "run-download",
                            &[("file", file), ("percent", (tenth * 10).to_string())]
                        )
                    );
                }
            }
            Event::Step { title, ok } => {
                let k = if ok { "run-step-ok" } else { "run-step-failed" };
                lines.push(t.tr_args(k, &[("title", title)]))
            }
            Event::Log { line } => {
                let _ = writeln!(out, "    {line}");
            }
        });
        for l in lines {
            self.say(&l)?;
        }
        // a failed run still reports what it did
        let report = e.result.clone();
        for p in report["products"].as_array().into_iter().flatten() {
            let title = self
                .runner
                .catalogue
                .get(p["product"].as_str().unwrap_or(""))
                .map(|x| x.title.clone())
                .unwrap_or_default();
            let v = p["version"].as_str().unwrap_or("").to_string();
            if p["already"].as_bool().unwrap_or(false) {
                let s = self
                    .t
                    .tr_args("done-already", &[("title", title), ("version", v)]);
                self.say(&s)?;
                continue;
            }
            let s = self.t.tr_args(
                "done-version",
                &[
                    ("title", title),
                    ("version", v),
                    ("dir", p["dir"].as_str().unwrap_or("").into()),
                ],
            );
            self.say(&s)?;
            let s = format!("  {}", done_source(&self.t, p));
            self.say(&s)?;
            if let Some(c) = p["check"].as_object() {
                let s = self.t.tr_args(
                    "done-started",
                    &[
                        ("command", c["command"].as_str().unwrap_or("").into()),
                        ("message", c["message"].as_str().unwrap_or("").into()),
                    ],
                );
                self.say(&format!("  {s}"))?;
            }
        }
        for n in report["notes"].as_array().into_iter().flatten() {
            self.say(n.as_str().unwrap_or(""))?;
        }
        if e.ok() {
            let s = self
                .t
                .tr_args("done-ok", &[("what", self.t.tr("menu-install"))]);
            self.say(&s)
        } else {
            self.fail(&e)
        }
    }

    fn list(&mut self) -> io::Result<()> {
        for scope in [Scope::User, Scope::System] {
            let v = self.installed(scope, &None);
            let who = self.t.tr(if scope == Scope::User {
                "list-scope-user"
            } else {
                "list-scope-system"
            });
            let s = self.t.tr_args(
                "list-title",
                &[
                    ("scope", who),
                    ("root", v["root"].as_str().unwrap_or("").into()),
                ],
            );
            self.say(&format!("\n{s}"))?;
            let list = v["products"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                let s = self.t.tr("list-none");
                self.say(&format!("  {s}"))?;
            }
            for p in list {
                for ver in p["versions"].as_array().into_iter().flatten() {
                    let d = if ver["default"].as_bool().unwrap_or(false) {
                        format!(" {}", self.t.tr("list-default"))
                    } else {
                        String::new()
                    };
                    let other = if ver["receipt"].is_null() {
                        format!(" ({})", self.t.tr("list-other"))
                    } else {
                        String::new()
                    };
                    let s = self.t.tr_args(
                        "list-row",
                        &[
                            ("title", p["title"].as_str().unwrap_or("").into()),
                            ("version", ver["version"].as_str().unwrap_or("").into()),
                            ("default", d),
                        ],
                    );
                    self.say(&format!("  {s}{other}"))?;
                }
            }
        }
        Ok(())
    }

    /// Scope, then one of its installed products.
    fn pick_installed(&mut self) -> io::Result<Option<(Scope, Option<PathBuf>, Value)>> {
        let (scope, prefix) = self.ask_scope()?;
        let v = self.installed(scope, &prefix);
        let list = v["products"].as_array().cloned().unwrap_or_default();
        if list.is_empty() {
            let s = self.t.tr("list-none");
            self.say(&s)?;
            return Ok(None);
        }
        let names: Vec<String> = list
            .iter()
            .map(|p| p["title"].as_str().unwrap_or("").to_string())
            .collect();
        let title = self.t.tr("pick-product");
        let i = self.choose(&title, &names, 0)?;
        Ok(Some((scope, prefix, list[i].clone())))
    }

    fn run_steps_tool(&mut self, id: &str, mut input: Value) -> io::Result<()> {
        input["dry_run"] = json!(true);
        let e = self.runner.quiet(id, &input);
        if !e.ok() {
            return self.fail(&e);
        }
        let steps = e.result["steps"].clone();
        self.show_steps(&steps)?;
        let q = self.t.tr("go-ask");
        if !self.yes(&q, true)? {
            let m = self.t.tr("cancelled");
            return self.say(&m);
        }
        input["dry_run"] = json!(false);
        let e = self.runner.quiet(id, &input);
        if !e.ok() {
            return self.fail(&e);
        }
        let s = self.t.tr_args("done-ok", &[("what", id.to_string())]);
        self.say(&s)
    }

    fn change_default(&mut self) -> io::Result<()> {
        let Some((scope, prefix, p)) = self.pick_installed()? else {
            return Ok(());
        };
        let versions: Vec<String> = p["versions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["version"].as_str().map(str::to_string))
            .collect();
        let title = self.t.tr("default-title");
        let i = self.choose(&title, &versions, 0)?;
        let mut input = json!({"product": p["product"], "version": versions[i], "scope": scope});
        if let Some(pr) = prefix {
            input["prefix"] = json!(pr);
        }
        self.run_steps_tool("version.activate", input)
    }

    fn remove(&mut self) -> io::Result<()> {
        let Some((scope, prefix, p)) = self.pick_installed()? else {
            return Ok(());
        };
        let mut versions: Vec<String> = p["versions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["version"].as_str().map(str::to_string))
            .collect();
        let all = self.t.tr_args(
            "remove-all",
            &[("title", p["title"].as_str().unwrap_or("").into())],
        );
        versions.push(all);
        let title = self.t.tr("remove-title");
        let i = self.choose(&title, &versions, 0)?;
        let mut input = json!({"product": p["product"], "scope": scope});
        if i + 1 < versions.len() {
            input["version"] = json!(versions[i]);
        }
        if let Some(pr) = prefix {
            input["prefix"] = json!(pr);
        }
        self.run_steps_tool("version.remove", input)
    }
}

/// One tool of a source build, as a line: there, installed by Kraxn, or
/// what you do.
pub fn tool_line(t: &I18n, tool: &Value) -> String {
    let s = |k: &str| tool[k].as_str().unwrap_or("").to_string();
    if tool["ok"].as_bool().unwrap_or(false) {
        t.tr_args("tools-ok", &[("need", s("need")), ("found", s("found"))])
    } else if tool["fix"].is_object() {
        let found = if s("found").is_empty() {
            t.tr("tools-missing")
        } else {
            s("found")
        };
        t.tr_args("tools-fix", &[("need", s("need")), ("found", found)])
    } else {
        t.tr_args(
            "tools-manual",
            &[("need", s("need")), ("manual", s("manual"))],
        )
    }
}

/// Where an installed product came from: built, or the release file and
/// whether its SHA-256 was checked.
pub fn done_source(t: &I18n, p: &Value) -> String {
    if p["built"].as_bool().unwrap_or(false) {
        return t.tr_args(
            "done-built",
            &[("file", p["file"].as_str().unwrap_or("").into())],
        );
    }
    t.tr(if p["verified"].as_bool().unwrap_or(false) {
        "done-checked"
    } else {
        "done-unchecked"
    })
}
