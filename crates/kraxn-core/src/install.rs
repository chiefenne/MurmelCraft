//! From a plan to steps, and running them: download, check, unpack (as
//! you), then the copy into the install folders (with administrator
//! rights for the system scope), the default version and the PATH.
//! Also the steps that change the default and remove versions.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::actions::{Action, Elevator, Outcome, Step, run_all};
use crate::build::{self, BuildWant, ToolState};
use crate::catalogue::{Catalogue, Product};
use crate::diag::{Res, diag};
use crate::envfiles::{self, Extras};
use crate::extras::{self, Beside, PyPlan};
use crate::fetch::{Checked, Source, fetch, verify};
use crate::installed::{self, RECEIPT, Receipt};
use crate::layout::{Env, Layout, Scope};
use crate::plan::{Origin, Plan};
use crate::platform::{Os, Platform};
use crate::release::{Release, file_name, pick, pick_source, top_folder};

/// What the front ends show while a plan runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    Info {
        text: String,
    },
    Download {
        file: String,
        done: u64,
        total: u64,
    },
    /// A step ran (`ok`) or failed.
    Step {
        title: String,
        ok: bool,
    },
    /// A line of a source build's output (cargo).
    Log {
        line: String,
    },
}

/// The fixed facts of one run: the catalogue, the system and its folders.
pub struct Ctx<'a> {
    pub catalogue: &'a Catalogue,
    pub platform: Platform,
    pub env: &'a Env,
}

impl Ctx<'_> {
    pub fn layout(&self, plan: &Plan) -> Res<Layout> {
        Layout::new(plan.scope, self.platform, self.env, plan.prefix.as_deref())
    }

    pub fn source(&self, plan: &Plan) -> Source {
        match &plan.offline_dir {
            Some(d) => Source::Folder(d.clone()),
            None => Source::github(),
        }
    }
}

/// The releases a plan names, product by product.
pub fn resolve<'c>(ctx: &'c Ctx, plan: &Plan, source: &Source) -> Res<Vec<(&'c Product, Release)>> {
    plan.check(ctx.catalogue)?;
    plan.installs
        .iter()
        .map(|w| {
            let p = ctx.catalogue.get(&w.product)?;
            let rels = source.releases(p)?;
            let r = match w.from {
                Origin::Release => pick(&rels, p, &ctx.platform, &w.version)?,
                Origin::Source => pick_source(&rels, p, &w.version)?,
            };
            Ok((p, r.clone()))
        })
        .collect()
}

/// The products of a plan that are built from their source.
pub fn build_wants<'c>(ctx: &'c Ctx, plan: &Plan) -> Res<Vec<BuildWant<'c>>> {
    plan.installs
        .iter()
        .filter(|w| w.from == Origin::Source)
        .map(|w| {
            Ok(BuildWant {
                product: ctx.catalogue.get(&w.product)?,
                features: w.features.clone(),
            })
        })
        .collect()
}

/// The tools the plan's source builds need, as found now (empty without
/// a source build). `work` is where installers are downloaded to.
pub fn tools(ctx: &Ctx, plan: &Plan, source: &Source, work: &Path) -> Res<Vec<ToolState>> {
    let mut v = build::check_tools(
        &ctx.platform,
        ctx.env,
        &build_wants(ctx, plan)?,
        &work.join("tools"),
    )?;
    let products = plan
        .installs
        .iter()
        .map(|w| ctx.catalogue.get(&w.product))
        .collect::<Res<Vec<_>>>()?;
    v.extend(extras::check(
        &ctx.platform,
        ctx.env,
        plan,
        source,
        &products,
    ));
    Ok(v)
}

/// The steps that install the missing tools; KRX015 when one of them is
/// one only you can install.
pub fn tool_steps(tools: &[ToolState]) -> Res<Vec<Step>> {
    let blocked: Vec<String> = tools
        .iter()
        .filter(|t| t.blocks())
        .map(|t| format!("{}: {}", t.need, t.manual.clone().unwrap_or_default()))
        .collect();
    if !blocked.is_empty() {
        return Err(diag(
            "KRX015",
            format!("the install needs {}", blocked.join("; ")),
        ));
    }
    Ok(tools
        .iter()
        .filter(|t| !t.ok)
        .filter_map(|t| t.fix.clone())
        .collect())
}

/// What a source build's preview shows before it runs: the download of
/// the source and the cargo runs.
fn build_preview(source: &Source, p: &Product, r: &Release, features: &[String]) -> Vec<Step> {
    vec![
        source_download_step(source, p, r, "from GitHub"),
        Step {
            title: format!(
                "Build {} {} with cargo, as you (the first build takes several minutes)",
                p.title, r.version
            ),
            admin: false,
            actions: build::cargo_lines(p, features)
                .into_iter()
                .map(|l| {
                    let mut words = l.split(' ').map(str::to_string);
                    Action::Run {
                        program: PathBuf::from(words.next().unwrap_or_default()),
                        args: words.collect(),
                        env: Vec::new(),
                        optional: false,
                    }
                })
                .collect(),
        },
    ]
}

/// The download of a release's source (GitHub's archive of the tag), as
/// the preview shows it.
fn source_download_step(source: &Source, p: &Product, r: &Release, why: &str) -> Step {
    let api = match source {
        Source::Online { api, .. } => api.clone(),
        Source::Folder(_) => "https://api.github.com".into(),
    };
    Step {
        title: format!("Download the source of {} {} {why}", p.title, r.version),
        admin: false,
        actions: vec![Action::Download {
            url: format!("{api}/repos/{}/tarball/{}", p.repo, r.tag),
            sha256_url: None,
            to: PathBuf::from("<download>").join(format!("{}-{}-source.tar.gz", p.archive, r.tag)),
        }],
    }
}

/// The product's examples folder in its source, when the plan wants it.
fn examples_folder<'p>(plan: &Plan, p: &'p Product) -> Option<&'p str> {
    p.examples.as_deref().filter(|_| plan.examples)
}

/// What the plan leaves out and why (offline without wheels or source).
pub fn notes(
    ctx: &Ctx,
    plan: &Plan,
    source: &Source,
    resolved: &[(&Product, Release)],
) -> Vec<String> {
    let _ = ctx;
    let mut v = Vec::new();
    for (p, r) in resolved {
        if let PyPlan::Skip(why) = extras::py_plan(plan, p, source) {
            v.push(why);
        }
        if examples_folder(plan, p).is_some()
            && let Source::Folder(d) = source
            && extras::source_archive(d, p, &r.tag, &r.version).is_none()
            && !plan
                .installs
                .iter()
                .any(|w| w.product == p.id && w.from == Origin::Source)
        {
            let repo = p.repo.rsplit('/').next().unwrap_or(&p.title);
            v.push(format!(
                "{}'s examples are left out: the folder has no {repo}-{}.tar.gz (the release page's \"Source code (tar.gz)\")",
                p.title, r.version
            ));
        }
    }
    v
}

/// A file of an unpacked release: its name and whether it is a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Staged {
    pub name: String,
    pub program: bool,
}

/// The files of the unpacked release folder `top`: README.md, LICENSE and
/// other documents stay beside `bin/`, everything else is a program (as
/// install-shared.sh does).
pub fn staged_files(top: &Path) -> Res<Vec<Staged>> {
    let mut out: Vec<Staged> = std::fs::read_dir(top)
        .map_err(|e| diag("KRX006", format!("{}: {e}", top.display())))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let doc = name.ends_with(".md") || name.ends_with(".txt") || name == "LICENSE";
            Staged {
                program: !doc,
                name,
            }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The files the catalogue expects in a release (for the preview, before
/// the download).
pub fn expected_files(p: &Product, platform: &Platform) -> Vec<Staged> {
    let mut v: Vec<Staged> = p
        .programs
        .iter()
        .map(|n| Staged {
            name: platform.exe(n),
            program: true,
        })
        .collect();
    for d in ["LICENSE", "README.md"] {
        v.push(Staged {
            name: d.into(),
            program: false,
        });
    }
    v
}

/// The steps that copy one version into place (not yet the default).
#[allow(clippy::too_many_arguments)]
pub fn install_steps(
    layout: &Layout,
    p: &Product,
    version: &str,
    staged: &Path,
    files: &[Staged],
    receipt: &Receipt,
    replace: bool,
    x: Extras,
) -> Vec<Step> {
    let admin = layout.scope == Scope::System;
    let dest = layout.version_dir(p, version);
    let prefix = layout.product_dir(p);
    let mut a = Vec::new();
    if replace {
        a.push(Action::RemoveDir { path: dest.clone() });
    }
    a.push(Action::MakeDir {
        path: dest.join("bin"),
    });
    for f in files {
        let to = if f.program {
            dest.join("bin").join(&f.name)
        } else {
            dest.join(&f.name)
        };
        a.push(Action::CopyFile {
            from: staged.join(&f.name),
            to,
            executable: f.program,
        });
    }
    a.push(Action::Write {
        path: dest.join(RECEIPT),
        text: receipt.to_toml(),
        executable: false,
    });
    if layout.platform.unix() {
        a.push(Action::Write {
            path: dest.join("env.sh"),
            text: envfiles::env_sh(p, version, &prefix, &dest, x),
            executable: false,
        });
        a.push(Action::Write {
            path: dest.join("env.csh"),
            text: envfiles::env_csh(p, version, &dest, x),
            executable: false,
        });
        a.push(Action::Write {
            path: layout.modulefiles(p).join(version),
            text: envfiles::modulefile(p, version, &prefix, &dest, x),
            executable: false,
        });
        a.push(Action::Write {
            path: prefix.join("use.sh"),
            text: envfiles::use_sh(p, &prefix),
            executable: false,
        });
        if admin {
            a.push(Action::Readable { path: dest.clone() });
            a.push(Action::Readable {
                path: prefix.join("modulefiles"),
            });
            a.push(Action::Readable {
                path: prefix.join("use.sh"),
            });
        }
    }
    vec![Step {
        title: format!("Install {} {version} into {}", p.title, dest.display()),
        admin,
        actions: a,
    }]
}

/// The steps that make `version` the default: `current` and the module
/// files' default on macOS and Linux; `default.txt` and the links in the
/// PATH folder on Windows (hard links name one version's files).
pub fn default_steps(
    layout: &Layout,
    p: &Product,
    version: &str,
    programs: &[String],
    linked: bool,
) -> Vec<Step> {
    let admin = layout.scope == Scope::System;
    let mut a = Vec::new();
    if layout.platform.unix() {
        a.push(Action::Link {
            target: PathBuf::from(version),
            link: layout.current_link(p),
        });
        a.push(Action::Write {
            path: layout.modulefiles(p).join(".version"),
            text: envfiles::module_version(version),
            executable: false,
        });
    } else {
        a.push(Action::Write {
            path: layout.default_file(p),
            text: format!("{version}\n"),
            executable: false,
        });
        if linked {
            for prog in programs {
                a.push(Action::Link {
                    target: layout.bin_target(p, version, prog),
                    link: layout.bin_link(prog),
                });
            }
        }
    }
    vec![Step {
        title: format!("Make {} {version} the default", p.title),
        admin,
        actions: a,
    }]
}

/// The shell start files that get Kraxn's PATH block (macOS, Linux).
pub fn shell_files(platform: &Platform, env: &Env) -> Vec<PathBuf> {
    let Some(h) = &env.home else {
        return Vec::new();
    };
    let mut v = Vec::new();
    match platform.os {
        Os::Macos => {
            v.push(h.join(".zprofile"));
            if h.join(".bash_profile").exists() {
                v.push(h.join(".bash_profile"));
            }
        }
        Os::Linux => {
            v.push(h.join(".profile"));
            for f in [".bashrc", ".zshrc"] {
                if h.join(f).exists() {
                    v.push(h.join(f));
                }
            }
        }
        Os::Windows => {}
    }
    v
}

/// The block that puts `bin` on the PATH of new shells.
pub fn path_block(bin: &Path) -> String {
    let b = bin.display();
    format!(
        "# the programs installed by Kraxn (it removes this with the last program)\ncase \":$PATH:\" in *\":{b}:\"*) ;; *) export PATH=\"{b}:$PATH\" ;; esac"
    )
}

/// The steps that put the product's programs into the PATH folder and,
/// if that folder is not on the PATH yet, put it there.
pub fn path_steps(layout: &Layout, p: &Product, version: &str, programs: &[String]) -> Vec<Step> {
    let admin = layout.scope == Scope::System;
    let mut a = vec![Action::MakeDir {
        path: layout.bin.clone(),
    }];
    for prog in programs {
        a.push(Action::Link {
            target: layout.bin_target(p, version, prog),
            link: layout.bin_link(prog),
        });
    }
    vec![Step {
        title: format!(
            "Link the {} programs into {}",
            p.title,
            layout.bin.display()
        ),
        admin,
        actions: a,
    }]
}

/// The step that puts the PATH folder on the PATH of new terminals, if it
/// is not there yet (once per plan).
pub fn path_env_steps(layout: &Layout, env: &Env) -> Vec<Step> {
    let admin = layout.scope == Scope::System;
    let mut steps = Vec::new();
    if !env.on_path(&layout.bin) {
        let actions: Vec<Action> = if layout.platform.unix() {
            if layout.scope == Scope::System {
                Vec::new()
            } else {
                shell_files(&layout.platform, env)
                    .into_iter()
                    .map(|file| Action::ShellBlock {
                        file,
                        text: Some(path_block(&layout.bin)),
                    })
                    .collect()
            }
        } else {
            vec![Action::WindowsPath {
                dir: layout.bin.clone(),
                machine: admin,
                add: true,
            }]
        };
        if !actions.is_empty() {
            steps.push(Step {
                title: format!("Put {} on the PATH of new terminals", layout.bin.display()),
                // the user's own start files, even for a system install;
                // the machine's PATH on Windows needs the rights
                admin: admin && !layout.platform.unix(),
                actions,
            });
        }
    }
    steps
}

/// Program names in the PATH folder for `p` (from the release's files).
fn program_names(files: &[Staged], platform: &Platform) -> Vec<String> {
    files
        .iter()
        .filter(|f| f.program)
        .map(|f| {
            if platform.os == Os::Windows {
                f.name.trim_end_matches(".exe").to_string()
            } else {
                f.name.clone()
            }
        })
        .collect()
}

/// All steps of a plan, for the preview (before the download): the
/// release folder is shown as `<download>/<folder>`, a build's as
/// `<build>`; first the steps that install the missing `tools`.
pub fn preview(
    ctx: &Ctx,
    plan: &Plan,
    source: &Source,
    resolved: &[(&Product, Release)],
    tools: &[ToolState],
) -> Res<Vec<Step>> {
    let layout = ctx.layout(plan)?;
    let mut steps: Vec<Step> = tools
        .iter()
        .filter(|t| !t.ok)
        .filter_map(|t| t.fix.clone())
        .collect();
    // the Python found now, or the one the tool steps put in place
    let python = extras::python(&ctx.platform, &build::tool_dirs(ctx.env)).unwrap_or_else(|| {
        PathBuf::from(if ctx.platform.os == Os::Windows {
            "python"
        } else {
            "python3"
        })
    });
    for (p, r) in resolved {
        let w = plan
            .installs
            .iter()
            .find(|w| w.product == p.id)
            .ok_or_else(|| diag("KRX010", "resolved product not in the plan"))?;
        let staged = if w.from == Origin::Source {
            steps.extend(build_preview(source, p, r, &w.features));
            PathBuf::from("<build>")
        } else {
            PathBuf::from("<download>").join(top_folder(p, &r.version, &ctx.platform))
        };
        let files = expected_files(p, &ctx.platform);
        let mut beside = Beside::default();
        if let PyPlan::Venv { wheels } = extras::py_plan(plan, p, source) {
            beside.venv = Some((python.clone(), wheels));
        }
        if let Some(ex) = examples_folder(plan, p) {
            let from = match (w.from, source) {
                (Origin::Source, _) => Some(PathBuf::from("<build>")),
                (_, Source::Online { .. }) => {
                    steps.push(source_download_step(source, p, r, "for its examples"));
                    Some(PathBuf::from("<download>"))
                }
                (_, Source::Folder(d)) => {
                    extras::source_archive(d, p, &r.tag, &r.version).map(|_| "<download>".into())
                }
            };
            beside.examples = from.map(|f| f.join(format!("{}-source", p.archive)).join(ex));
        }
        steps.extend(product_steps(
            ctx,
            plan,
            &layout,
            p,
            &r.version,
            &staged,
            &files,
            &Receipt::new(&p.id, &r.version, "release"),
            w.make_default,
            w.replace,
            &beside,
        ));
    }
    if plan.path {
        steps.extend(path_env_steps(&layout, ctx.env));
    }
    Ok(steps)
}

#[allow(clippy::too_many_arguments)]
fn product_steps(
    ctx: &Ctx,
    plan: &Plan,
    layout: &Layout,
    p: &Product,
    version: &str,
    staged: &Path,
    files: &[Staged],
    receipt: &Receipt,
    make_default: bool,
    replace: bool,
    beside: &Beside,
) -> Vec<Step> {
    let have = installed::product(layout, p);
    let already = have.get(version).is_some();
    let programs = program_names(files, &ctx.platform);
    let mut steps = Vec::new();
    if !already || replace {
        steps.extend(install_steps(
            layout,
            p,
            version,
            staged,
            files,
            receipt,
            replace && already,
            Extras {
                venv: beside.venv.is_some(),
                src: false,
            },
        ));
        steps.extend(beside.steps(layout, p, version));
    }
    let no_default = have.default_version().is_none();
    if make_default || no_default {
        steps.extend(default_steps(layout, p, version, &programs, plan.path));
        if plan.menu {
            steps.extend(crate::desktop::menu_steps(
                layout,
                ctx.env,
                p,
                &window_exe(layout, p, version),
                plan.file_types,
            ));
        }
    }
    if plan.path {
        steps.extend(path_steps(layout, p, version, &programs));
    }
    steps
}

/// The program a menu entry starts: under `current` on macOS and Linux
/// (it follows the default), the version's own on Windows.
pub fn window_exe(layout: &Layout, p: &Product, version: &str) -> PathBuf {
    let w = p.window.clone().unwrap_or_default();
    if layout.platform.unix() {
        layout.current_link(p).join("bin").join(w)
    } else {
        layout
            .version_dir(p, version)
            .join("bin")
            .join(layout.platform.exe(&w))
    }
}

/// The check after an install: the product's check command in the new
/// version folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub command: String,
    pub ok: bool,
    pub message: String,
}

pub fn smoke_check(layout: &Layout, p: &Product, version: &str) -> CheckResult {
    let dir = layout.version_dir(p, version).join("bin");
    let (prog, args) = match p.check.split_first() {
        Some((a, b)) => (a.clone(), b.to_vec()),
        None => (String::new(), Vec::new()),
    };
    let exe = dir.join(layout.platform.exe(&prog));
    let command = format!("{} {}", exe.display(), args.join(" "));
    match std::process::Command::new(&exe)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .output()
    {
        Ok(o) if o.status.success() => CheckResult {
            command,
            ok: true,
            message: "started and ended without an error".into(),
        },
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let mut message = format!("ended with {}: {err}", o.status);
            if err.contains("GLIBC") {
                message.push_str(
                    " (this Linux is older than the release supports: build from source)",
                );
            }
            CheckResult {
                command,
                ok: false,
                message,
            }
        }
        Err(e) => CheckResult {
            command,
            ok: false,
            message: format!("did not start: {e}"),
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductReport {
    pub product: String,
    pub version: String,
    pub dir: PathBuf,
    pub file: String,
    pub sha256: String,
    pub verified: bool,
    /// Built here from the source (`file` names the repository and tag).
    #[serde(default)]
    pub built: bool,
    /// Installed before this run (nothing copied).
    pub already: bool,
    pub check: Option<CheckResult>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub ok: bool,
    pub outcomes: Vec<Outcome>,
    pub products: Vec<ProductReport>,
    pub notes: Vec<String>,
}

/// Runs a plan: resolves the releases, downloads and checks each file
/// into `work`, unpacks it, runs the steps (admin ones through
/// `elevator`) and checks that each program starts.
pub fn apply(
    ctx: &Ctx,
    plan: &Plan,
    source: &Source,
    work: &Path,
    elevator: &mut dyn Elevator,
    progress: &mut dyn FnMut(Event),
) -> Res<Report> {
    let layout = ctx.layout(plan)?;
    let resolved = resolve(ctx, plan, source)?;
    std::fs::create_dir_all(work)
        .map_err(|e| diag("KRX007", format!("{}: {e}", work.display())))?;
    let mut steps = Vec::new();
    let mut products = Vec::new();
    let mut outcomes = Vec::new();
    let mut notes = notes(ctx, plan, source, &resolved);
    // the tools first (the builds', Python, ffmpeg): the rest needs them
    let wants = build_wants(ctx, plan)?;
    let tools = tools(ctx, plan, source, work)?;
    let fixes = tool_steps(&tools)?;
    if !fixes.is_empty() {
        progress(Event::Info {
            text: "Install the tools the install needs".into(),
        });
        let optional: Vec<&str> = tools
            .iter()
            .filter(|t| t.optional)
            .filter_map(|t| t.fix.as_ref().map(|f| f.title.as_str()))
            .collect();
        let (opt, req): (Vec<Step>, Vec<Step>) = fixes
            .into_iter()
            .partition(|f| optional.contains(&f.title.as_str()));
        let o = run_all(&req, elevator);
        for x in &o {
            progress(Event::Step {
                title: x.title.clone(),
                ok: x.ok,
            });
        }
        let failed = o.len() < req.len() || o.iter().any(|x| !x.ok);
        outcomes.extend(o);
        if failed {
            return Ok(Report {
                ok: false,
                outcomes,
                products,
                notes,
            });
        }
        // an optional tool that does not install is a note, not a stop
        for f in opt {
            let o = run_all(std::slice::from_ref(&f), elevator);
            let ok = o.first().is_some_and(|x| x.ok);
            progress(Event::Step {
                title: f.title.clone(),
                ok,
            });
            if !ok {
                notes.push(format!(
                    "{} did not work{}; the install went on without it",
                    f.title,
                    o.first()
                        .and_then(|x| x.error.as_ref())
                        .map(|e| format!(" ({})", e.message))
                        .unwrap_or_default()
                ));
            }
        }
        let again = self::tools(ctx, plan, source, work)?;
        if let Some(t) = again.iter().find(|t| !t.ok && !t.optional) {
            return Err(diag(
                "KRX015",
                format!(
                    "{} is still missing after its install{}; open a new terminal or sign in again, then retry",
                    t.need,
                    t.found
                        .as_ref()
                        .map(|f| format!(" (found {f})"))
                        .unwrap_or_default()
                ),
            ));
        }
    }
    for t in tools
        .iter()
        .filter(|t| !t.ok && t.optional && t.fix.is_none())
    {
        notes.push(format!(
            "{} is missing: {}",
            t.need,
            t.manual.clone().unwrap_or_default()
        ));
    }
    let toolbox = if wants.is_empty() {
        None
    } else {
        Some(build::toolbox(&ctx.platform, ctx.env)?)
    };
    let python = extras::python(&ctx.platform, &build::tool_dirs(ctx.env));
    for (p, r) in &resolved {
        let w = plan
            .installs
            .iter()
            .find(|w| w.product == p.id)
            .ok_or_else(|| diag("KRX010", "resolved product not in the plan"))?;
        let name = file_name(p, &r.version, &ctx.platform);
        let dir = layout.version_dir(p, &r.version);
        let already = installed::product(&layout, p).get(&r.version).is_some() && !w.replace;
        if already {
            progress(Event::Info {
                text: format!(
                    "{} {} is installed already in {}",
                    p.title,
                    r.version,
                    dir.display()
                ),
            });
            let files = staged_files(&dir.join("bin")).unwrap_or_default();
            steps.extend(product_steps(
                ctx,
                plan,
                &layout,
                p,
                &r.version,
                work,
                &files,
                &Receipt::new(&p.id, &r.version, "release"),
                w.make_default,
                false,
                &Beside::default(),
            ));
            products.push(ProductReport {
                product: p.id.clone(),
                version: r.version.clone(),
                dir,
                file: name,
                sha256: String::new(),
                verified: false,
                built: false,
                already: true,
                check: None,
            });
            continue;
        }
        if w.from == Origin::Source {
            let tb = toolbox
                .as_ref()
                .ok_or_else(|| diag("KRX015", "no Rust for the source build"))?;
            let pwork = work.join(&p.id);
            let _ = std::fs::remove_dir_all(&pwork);
            std::fs::create_dir_all(&pwork)
                .map_err(|e| diag("KRX007", format!("{}: {e}", pwork.display())))?;
            let src_name = format!("{}@{}", p.repo, r.tag);
            progress(Event::Info {
                text: format!(
                    "Download the source of {} {} ({src_name})",
                    p.title, r.version
                ),
            });
            let (src, sha256) =
                build::fetch_source(source, p, &r.tag, &pwork, &mut |done, total| {
                    progress(Event::Download {
                        file: src_name.clone(),
                        done,
                        total,
                    })
                })?;
            progress(Event::Info {
                text: format!(
                    "Build {} {} with cargo ({}); the first build takes several minutes",
                    p.title, r.version, tb.rustc
                ),
            });
            // cargo's target folder, apart from the source download
            let bwork = work.join("build").join(&p.id);
            let bin =
                build::cargo_build(tb, p, &w.features, w.native, &src, &bwork, &mut |line| {
                    progress(Event::Log { line })
                })?;
            let files = staged_files(&bin)?;
            let mut receipt = Receipt::new(&p.id, &r.version, "build");
            receipt.file = src_name.clone();
            receipt.sha256 = sha256.clone();
            receipt.features = w.features.clone();
            receipt.native = w.native;
            receipt.rustc = tb.rustc.clone();
            let beside = beside_for(
                plan,
                p,
                r,
                source,
                python.as_deref(),
                Some(&src),
                &pwork,
                &mut notes,
                progress,
            );
            steps.extend(product_steps(
                ctx,
                plan,
                &layout,
                p,
                &r.version,
                &bin,
                &files,
                &receipt,
                w.make_default,
                w.replace,
                &beside,
            ));
            products.push(ProductReport {
                product: p.id.clone(),
                version: r.version.clone(),
                dir,
                file: src_name,
                sha256,
                verified: false,
                built: true,
                already: false,
                check: None,
            });
            continue;
        }
        let file = r
            .archive_for(p, &ctx.platform)
            .ok_or_else(|| diag("KRX002", format!("{name} is not in the release")))?;
        let pwork = work.join(&p.id);
        let _ = std::fs::remove_dir_all(&pwork);
        std::fs::create_dir_all(&pwork)
            .map_err(|e| diag("KRX007", format!("{}: {e}", pwork.display())))?;
        let archive = pwork.join(&name);
        progress(Event::Info {
            text: format!("Download {name}"),
        });
        fetch(file, &archive, &mut |done, total| {
            progress(Event::Download {
                file: name.clone(),
                done,
                total,
            })
        })?;
        let Checked { sha256, verified } =
            verify(r, &name, &archive, &pwork, plan.require_checksums)?;
        progress(Event::Info {
            text: if verified {
                format!("{name}: SHA-256 matches SHA256SUMS.txt")
            } else {
                format!("{name}: not checked (no SHA256SUMS.txt lists it)")
            },
        });
        let top = crate::archive::unpack(&archive, &pwork.join("unpacked"))?;
        let files = staged_files(&top)?;
        let mut receipt = Receipt::new(
            &p.id,
            &r.version,
            if plan.offline_dir.is_some() {
                "folder"
            } else {
                "release"
            },
        );
        receipt.file = name.clone();
        receipt.sha256 = sha256.clone();
        receipt.verified = verified;
        let beside = beside_for(
            plan,
            p,
            r,
            source,
            python.as_deref(),
            None,
            &pwork,
            &mut notes,
            progress,
        );
        steps.extend(product_steps(
            ctx,
            plan,
            &layout,
            p,
            &r.version,
            &top,
            &files,
            &receipt,
            w.make_default,
            w.replace,
            &beside,
        ));
        products.push(ProductReport {
            product: p.id.clone(),
            version: r.version.clone(),
            dir,
            file: name,
            sha256,
            verified,
            built: false,
            already: false,
            check: None,
        });
    }
    if plan.path {
        steps.extend(path_env_steps(&layout, ctx.env));
    }
    let ran = run_all(&steps, elevator);
    for o in &ran {
        progress(Event::Step {
            title: o.title.clone(),
            ok: o.ok,
        });
    }
    let mut ok = ran.len() == steps.len() && ran.iter().all(|o| o.ok);
    outcomes.extend(ran);
    if ok {
        for pr in products.iter_mut().filter(|pr| !pr.already) {
            let p = ctx.catalogue.get(&pr.product)?;
            let c = smoke_check(&layout, p, &pr.version);
            ok &= c.ok;
            pr.check = Some(c);
        }
        if plan.path && !ctx.env.on_path(&layout.bin) {
            notes.push(format!(
                "{} is on the PATH of terminals you open from now on (not of those open already)",
                layout.bin.display()
            ));
        }
    }
    Ok(Report {
        ok,
        outcomes,
        products,
        notes,
    })
}

/// The venv and the examples of a new version: the source of a build
/// (`src`), else the release's source from GitHub or the folder of
/// downloads. What cannot be had becomes a note.
#[allow(clippy::too_many_arguments)]
fn beside_for(
    plan: &Plan,
    p: &Product,
    r: &Release,
    source: &Source,
    python: Option<&Path>,
    src: Option<&Path>,
    pwork: &Path,
    notes: &mut Vec<String>,
    progress: &mut dyn FnMut(Event),
) -> Beside {
    let mut b = Beside::default();
    if let PyPlan::Venv { wheels } = extras::py_plan(plan, p, source) {
        match python {
            Some(py) => b.venv = Some((py.to_path_buf(), wheels)),
            None => notes.push(format!(
                "{}'s Python is left out: no Python 3 found",
                p.title
            )),
        }
    }
    let Some(ex) = examples_folder(plan, p) else {
        return b;
    };
    let top = match (src, source) {
        (Some(s), _) => Ok(s.to_path_buf()),
        (None, Source::Online { .. }) => {
            let name = format!("{}@{}", p.repo, r.tag);
            progress(Event::Info {
                text: format!(
                    "Download the source of {} {} for its examples",
                    p.title, r.version
                ),
            });
            let _ = std::fs::create_dir_all(pwork.join("src"));
            build::fetch_source(source, p, &r.tag, &pwork.join("src"), &mut |done, total| {
                progress(Event::Download {
                    file: name.clone(),
                    done,
                    total,
                })
            })
            .map(|(top, _)| top)
        }
        (None, Source::Folder(d)) => match extras::source_archive(d, p, &r.tag, &r.version) {
            // `notes` already says why
            None => return b,
            Some(a) => crate::archive::unpack(&a, &pwork.join("src").join("source")),
        },
    };
    match top {
        Ok(t) if t.join(ex).is_dir() => b.examples = Some(t.join(ex)),
        Ok(t) => notes.push(format!(
            "{}'s examples are left out: the source has no {ex} folder ({})",
            p.title,
            t.display()
        )),
        Err(e) => notes.push(format!(
            "{}'s examples are left out: {}",
            p.title, e.message
        )),
    }
    b
}

/// The steps that make an installed version the default.
pub fn activate(ctx: &Ctx, layout: &Layout, product: &str, version: &str) -> Res<Vec<Step>> {
    let p = ctx.catalogue.get(product)?;
    let v = installed::require(layout, p, version)?;
    let linked = layout
        .bin
        .join(
            layout
                .platform
                .exe(p.check.first().map(String::as_str).unwrap_or("")),
        )
        .exists();
    let files = staged_files(&v.dir.join("bin")).unwrap_or_default();
    let mut steps = default_steps(
        layout,
        p,
        &v.version,
        &program_names(&files, &ctx.platform),
        linked,
    );
    // Windows' menu entry names one version's program
    if !layout.platform.unix() && crate::desktop::has_entry(layout, ctx.env, p) {
        steps.extend(crate::desktop::menu_steps(
            layout,
            ctx.env,
            p,
            &window_exe(layout, p, &v.version),
            true,
        ));
    }
    Ok(steps)
}

/// The steps that remove one version, or the whole product (`version`
/// `None`, or its last version): its folder, its module file, its links
/// in the PATH folder and, with the last product, Kraxn's PATH entries.
pub fn remove(ctx: &Ctx, layout: &Layout, product: &str, version: Option<&str>) -> Res<Vec<Step>> {
    let p = ctx.catalogue.get(product)?;
    let have = installed::product(layout, p);
    let admin = layout.scope == Scope::System;
    if have.versions.is_empty() {
        return Err(diag(
            "KRX009",
            format!("{} is not installed in {}", p.title, have.dir.display()),
        ));
    }
    if let Some(v) = version {
        let v = installed::require(layout, p, v)?;
        if have.versions.len() > 1 {
            let mut steps = vec![Step {
                title: format!("Remove {} {}", p.title, v.version),
                admin,
                actions: vec![
                    Action::RemoveDir {
                        path: v.dir.clone(),
                    },
                    Action::RemoveFile {
                        path: layout.modulefiles(p).join(&v.version),
                    },
                ],
            }];
            if v.default
                && let Some(next) = have.versions.iter().find(|o| o.version != v.version)
            {
                steps.extend(activate(ctx, layout, product, &next.version)?);
            }
            return Ok(steps);
        }
    }
    // the whole product
    let files = have
        .versions
        .first()
        .and_then(|v| staged_files(&v.dir.join("bin")).ok())
        .unwrap_or_default();
    let mut names = program_names(&files, &ctx.platform);
    for n in &p.programs {
        if !names.contains(n) {
            names.push(n.clone());
        }
    }
    let mut a = Vec::new();
    for n in names {
        let link = layout.bin_link(&n);
        let ours = if layout.platform.unix() {
            std::fs::read_link(&link)
                .map(|t| t.starts_with(layout.product_dir(p)))
                .unwrap_or(false)
        } else {
            link.exists()
        };
        if ours {
            a.push(Action::RemoveFile { path: link });
        }
    }
    a.push(Action::RemoveDir {
        path: layout.product_dir(p),
    });
    let mut steps = crate::desktop::remove_steps(layout, ctx.env, p);
    steps.push(Step {
        title: format!("Remove {} ({})", p.title, layout.product_dir(p).display()),
        admin,
        actions: a,
    });
    let others = ctx
        .catalogue
        .products
        .iter()
        .any(|o| o.id != p.id && !installed::product(layout, o).versions.is_empty());
    if !others {
        let actions: Vec<Action> = if layout.platform.unix() {
            shell_files(&layout.platform, ctx.env)
                .into_iter()
                .map(|file| Action::ShellBlock { file, text: None })
                .collect()
        } else {
            vec![Action::WindowsPath {
                dir: layout.bin.clone(),
                machine: admin,
                add: false,
            }]
        };
        if !actions.is_empty() && (layout.scope == Scope::User || !layout.platform.unix()) {
            steps.push(Step {
                title: format!("Take {} off the PATH", layout.bin.display()),
                admin: admin && !layout.platform.unix(),
                actions,
            });
        }
    }
    Ok(steps)
}

#[cfg(test)]
mod tests;
