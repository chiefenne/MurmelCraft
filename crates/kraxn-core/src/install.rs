//! From a plan to steps, and running them: download, check, unpack (as
//! you), then the copy into the install folders (with administrator
//! rights for the system scope), the default version and the PATH.
//! Also the steps that change the default and remove versions.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::actions::{Action, Elevator, Outcome, Step, run_all};
use crate::catalogue::{Catalogue, Product};
use crate::diag::{Res, diag};
use crate::envfiles::{self, Extras};
use crate::fetch::{Checked, Source, fetch, verify};
use crate::installed::{self, RECEIPT, Receipt};
use crate::layout::{Env, Layout, Scope};
use crate::plan::Plan;
use crate::platform::{Os, Platform};
use crate::release::{Release, file_name, pick, top_folder};

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
            let r = pick(&rels, p, &ctx.platform, &w.version)?.clone();
            Ok((p, r))
        })
        .collect()
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
pub fn install_steps(
    layout: &Layout,
    p: &Product,
    version: &str,
    staged: &Path,
    files: &[Staged],
    receipt: &Receipt,
    replace: bool,
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
        let x = Extras::default();
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
/// release folder is shown as `<download>/<folder>`.
pub fn preview(ctx: &Ctx, plan: &Plan, resolved: &[(&Product, Release)]) -> Res<Vec<Step>> {
    let layout = ctx.layout(plan)?;
    let mut steps = Vec::new();
    for (p, r) in resolved {
        let w = plan
            .installs
            .iter()
            .find(|w| w.product == p.id)
            .ok_or_else(|| diag("KRX010", "resolved product not in the plan"))?;
        let staged = PathBuf::from("<download>").join(top_folder(p, &r.version, &ctx.platform));
        let files = expected_files(p, &ctx.platform);
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
        ));
    }
    let no_default = have.default_version().is_none();
    if make_default || no_default {
        steps.extend(default_steps(layout, p, version, &programs, plan.path));
    }
    if plan.path {
        steps.extend(path_steps(layout, p, version, &programs));
    }
    steps
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
            ));
            products.push(ProductReport {
                product: p.id.clone(),
                version: r.version.clone(),
                dir,
                file: name,
                sha256: String::new(),
                verified: false,
                already: true,
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
        ));
        products.push(ProductReport {
            product: p.id.clone(),
            version: r.version.clone(),
            dir,
            file: name,
            sha256,
            verified,
            already: false,
            check: None,
        });
    }
    if plan.path {
        steps.extend(path_env_steps(&layout, ctx.env));
    }
    let outcomes = run_all(&steps, elevator);
    for o in &outcomes {
        progress(Event::Step {
            title: o.title.clone(),
            ok: o.ok,
        });
    }
    let mut ok = outcomes.len() == steps.len() && outcomes.iter().all(|o| o.ok);
    let mut notes = Vec::new();
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
    Ok(default_steps(
        layout,
        p,
        &v.version,
        &program_names(&files, &ctx.platform),
        linked,
    ))
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
    let mut steps = vec![Step {
        title: format!("Remove {} ({})", p.title, layout.product_dir(p).display()),
        admin,
        actions: a,
    }];
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
