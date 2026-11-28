//! The extras beside the programs: Python for FoehnCraft's scripts (a
//! venv with Gmsh, NumPy, Matplotlib and h5py, or the distribution's
//! packages), ffmpeg for MurmelCraft's MP4 movies, and KoflCraft's and
//! FoehnCraft's examples from the release's source.
//!
//! Python and ffmpeg are tools like a source build's: found, or installed
//! before the programs (apt and dnf with administrator rights, Homebrew
//! and winget as you). ffmpeg and the system's Python packages are
//! optional: when Kraxn cannot install them, the install goes on and says
//! so. The venv and the examples go into the version folder, so removing
//! the version removes them too.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::actions::{Action, Step};
use crate::build::{self, Pkg, ToolState};
use crate::catalogue::Product;
use crate::fetch::Source;
use crate::layout::{Env, Layout, Scope};
use crate::plan::{Plan, PyMode};
use crate::platform::{Os, Platform};

/// The Python that makes venvs: `python3` on macOS and Linux, `python` on
/// Windows (not the Store's stand-in in `WindowsApps`, which opens the
/// Store instead of running).
pub fn python(platform: &Platform, dirs: &[PathBuf]) -> Option<PathBuf> {
    let name = if platform.os == Os::Windows {
        "python"
    } else {
        "python3"
    };
    let file = platform.exe(name);
    dirs.iter()
        .filter(|d| !d.components().any(|c| c.as_os_str() == "WindowsApps"))
        .map(|d| d.join(&file))
        .find(|p| p.is_file())
}

/// Whether `python -c code` ends well.
pub fn runs(python: &Path, code: &str) -> bool {
    Command::new(python)
        .args(["-c", code])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// What happens with one product's Python.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PyPlan {
    /// The product has no scripts, or the plan says `python = "none"`.
    No,
    /// A venv in the version folder; pip takes the packages from `wheels`
    /// when set (the folder of downloads), from PyPI otherwise.
    Venv { wheels: Option<PathBuf> },
    /// The distribution's packages.
    System,
    /// No venv, and why (offline without a `wheels` folder).
    Skip(String),
}

pub fn py_plan(plan: &Plan, p: &Product, source: &Source) -> PyPlan {
    let Some(spec) = &p.python else {
        return PyPlan::No;
    };
    match plan.python {
        PyMode::None => PyPlan::No,
        PyMode::System => PyPlan::System,
        PyMode::Venv => match source {
            Source::Online { .. } => PyPlan::Venv { wheels: None },
            Source::Folder(d) => {
                let w = d.join("wheels");
                if w.is_dir() {
                    PyPlan::Venv { wheels: Some(w) }
                } else {
                    PyPlan::Skip(format!(
                        "{}'s Python is left out: pip needs the internet or a folder of wheels at {} (pip download -d wheels {}); run Kraxn again online, or choose the system's Python",
                        p.title,
                        w.display(),
                        spec.pip.join(" ")
                    ))
                }
            }
        },
    }
}

/// Python and ffmpeg as the plan's products need them, each with the step
/// that installs it when it is missing.
pub fn check(
    platform: &Platform,
    env: &Env,
    plan: &Plan,
    source: &Source,
    products: &[&Product],
) -> Vec<ToolState> {
    let dirs = build::tool_dirs(env);
    let mut v = Vec::new();
    let plans: Vec<(&Product, PyPlan)> = products
        .iter()
        .map(|p| (*p, py_plan(plan, p, source)))
        .collect();
    let venv: Vec<&Product> = plans
        .iter()
        .filter(|(_, x)| matches!(x, PyPlan::Venv { .. }))
        .map(|(p, _)| *p)
        .collect();
    if !venv.is_empty() {
        v.push(venv_python(platform, &dirs));
        if platform.os == Os::Linux {
            for p in &venv {
                v.push(python_libs(platform, &dirs, p, &lib_dirs()));
            }
        }
    }
    for (p, x) in &plans {
        if *x == PyPlan::System {
            v.push(system_packages(platform, &dirs, p));
        }
    }
    if plan.ffmpeg && products.iter().any(|p| p.ffmpeg) {
        v.push(ffmpeg(platform, &dirs));
    }
    v
}

fn manual_or(state: &mut ToolState, step: Option<Step>, manual: &str) {
    if state.ok {
        return;
    }
    match step {
        Some(s) => state.fix = Some(s),
        None => state.manual = Some(manual.into()),
    }
}

fn venv_python(platform: &Platform, dirs: &[PathBuf]) -> ToolState {
    let py = python(platform, dirs);
    let found = py.as_ref().map(|p| {
        build::first_line(p, &["--version"])
            .map(|l| format!("{l} ({})", p.display()))
            .unwrap_or_else(|| p.display().to_string())
    });
    let ok = py
        .as_ref()
        .is_some_and(|p| runs(p, "import venv, ensurepip"));
    let mut t = ToolState {
        id: "python".into(),
        need: "Python 3 with venv (for the scripts)".into(),
        found,
        ok,
        fix: None,
        manual: None,
        optional: false,
    };
    let step = match platform.os {
        // Debian and Ubuntu ship venv without pip unless python3-venv is in
        Os::Linux => build::linux_pkg(platform, dirs).map(|(pkg, exe)| {
            let list: &[&str] = match pkg {
                Pkg::Apt => &["python3", "python3-venv"],
                Pkg::Dnf => &["python3"],
            };
            build::pkg_step(pkg, &exe, "Python 3", list)
        }),
        Os::Macos => {
            build::find(platform, dirs, "brew").map(|b| build::brew_step(&b, "Python 3", "python"))
        }
        Os::Windows => build::find(platform, dirs, "winget")
            .map(|w| build::winget_step(&w, "Python 3", "Python.Python.3.13", &[])),
    };
    manual_or(
        &mut t,
        step,
        "install Python 3 from https://www.python.org/downloads/ (or choose no Python)",
    );
    t
}

/// Where Linux keeps shared libraries.
fn lib_dirs() -> Vec<PathBuf> {
    [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib64",
        "/usr/lib",
        "/lib/x86_64-linux-gnu",
        "/lib/aarch64-linux-gnu",
        "/lib64",
    ]
    .iter()
    .map(PathBuf::from)
    .collect()
}

/// The system libraries the pip packages load (Gmsh: libGLU and X11's).
fn python_libs(platform: &Platform, dirs: &[PathBuf], p: &Product, libs: &[PathBuf]) -> ToolState {
    let spec = p
        .python
        .as_ref()
        .map(|s| (s.libs.clone(), s.apt_libs.clone(), s.dnf_libs.clone()));
    let (names, apt, dnf) = spec.unwrap_or_default();
    let missing: Vec<&String> = names
        .iter()
        .filter(|n| !libs.iter().any(|d| d.join(n).exists()))
        .collect();
    let mut t = ToolState {
        id: format!("{}-python-libs", p.id),
        need: format!(
            "the libraries {}'s Python loads ({})",
            p.title,
            names.join(", ")
        ),
        found: (!missing.is_empty()).then(|| {
            format!(
                "missing {}",
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }),
        ok: missing.is_empty(),
        fix: None,
        manual: None,
        optional: false,
    };
    let step = build::linux_pkg(platform, dirs).and_then(|(pkg, exe)| {
        let list = match pkg {
            Pkg::Apt => &apt,
            Pkg::Dnf => &dnf,
        };
        let list: Vec<&str> = list.iter().map(String::as_str).collect();
        (!list.is_empty()).then(|| build::pkg_step(pkg, &exe, "the libraries Gmsh loads", &list))
    });
    manual_or(
        &mut t,
        step,
        &format!(
            "install the packages that hold {} with your package manager (or choose no Python)",
            names.join(", ")
        ),
    );
    t
}

fn system_packages(platform: &Platform, dirs: &[PathBuf], p: &Product) -> ToolState {
    let spec = p
        .python
        .clone()
        .unwrap_or_else(|| unreachable!("checked by py_plan"));
    let py = python(platform, dirs);
    let ok = py.as_ref().is_some_and(|x| runs(x, &spec.check));
    let mut t = ToolState {
        id: format!("{}-python-packages", p.id),
        need: format!(
            "Python with {} from the distribution (for {}'s scripts)",
            spec.pip.join(", "),
            p.title
        ),
        found: py.map(|x| x.display().to_string()),
        ok,
        fix: None,
        manual: None,
        optional: true,
    };
    let step = if platform.os == Os::Linux {
        build::linux_pkg(platform, dirs).and_then(|(pkg, exe)| {
            let list = match pkg {
                Pkg::Apt => &spec.apt,
                Pkg::Dnf => &spec.dnf,
            };
            let list: Vec<&str> = list.iter().map(String::as_str).collect();
            (!list.is_empty()).then(|| {
                build::pkg_step(pkg, &exe, &format!("{}'s Python packages", p.title), &list)
            })
        })
    } else {
        None
    };
    manual_or(
        &mut t,
        step,
        &format!("pip install {} (or choose the venv)", spec.pip.join(" ")),
    );
    t
}

fn ffmpeg(platform: &Platform, dirs: &[PathBuf]) -> ToolState {
    let found = build::find(platform, dirs, "ffmpeg");
    let mut t = ToolState {
        id: "ffmpeg".into(),
        need: "ffmpeg (for MP4 movies; GIF works without it)".into(),
        ok: found.is_some(),
        found: found.map(|p| p.display().to_string()),
        fix: None,
        manual: None,
        optional: true,
    };
    let step = match platform.os {
        Os::Linux => build::linux_pkg(platform, dirs).map(|(pkg, exe)| {
            // Fedora's free build (RPM Fusion's `ffmpeg` replaces it)
            let name = match pkg {
                Pkg::Apt => "ffmpeg",
                Pkg::Dnf => "ffmpeg-free",
            };
            build::pkg_step(pkg, &exe, "ffmpeg", &[name])
        }),
        Os::Macos => {
            build::find(platform, dirs, "brew").map(|b| build::brew_step(&b, "ffmpeg", "ffmpeg"))
        }
        Os::Windows => build::find(platform, dirs, "winget")
            .map(|w| build::winget_step(&w, "ffmpeg", "Gyan.FFmpeg", &[])),
    };
    manual_or(
        &mut t,
        step,
        "install ffmpeg from https://ffmpeg.org/download.html for MP4 movies",
    );
    t
}

/// The venv's Python.
pub fn venv_python_in(platform: &Platform, venv: &Path) -> PathBuf {
    if platform.os == Os::Windows {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

/// The venv of a new version: made by `python`, the packages from PyPI
/// (or `wheels`), then the catalogue's check.
pub fn venv_steps(
    layout: &Layout,
    p: &Product,
    version: &str,
    python: &Path,
    wheels: Option<&Path>,
) -> Vec<Step> {
    let Some(spec) = &p.python else {
        return Vec::new();
    };
    let admin = layout.scope == Scope::System;
    let venv = layout.version_dir(p, version).join("venv");
    let vpy = venv_python_in(&layout.platform, &venv);
    let env = vec![("PIP_DISABLE_PIP_VERSION_CHECK".to_string(), "1".to_string())];
    let run = |program: &Path, args: Vec<String>| Action::Run {
        program: program.to_path_buf(),
        args,
        env: env.clone(),
        optional: false,
    };
    let mut a = vec![run(
        python,
        vec!["-m".into(), "venv".into(), venv.display().to_string()],
    )];
    let mut pip: Vec<String> = ["-m", "pip", "install", "--quiet"]
        .map(String::from)
        .to_vec();
    match wheels {
        Some(w) => {
            pip.extend([
                "--no-index".into(),
                "--find-links".into(),
                w.display().to_string(),
            ]);
        }
        None => {
            a.push(run(
                &vpy,
                ["-m", "pip", "install", "--quiet", "--upgrade", "pip"]
                    .map(String::from)
                    .to_vec(),
            ));
        }
    }
    pip.extend(spec.pip.iter().cloned());
    a.push(run(&vpy, pip));
    if admin {
        a.push(Action::Readable { path: venv.clone() });
    }
    vec![
        Step {
            title: format!(
                "Python for {} {version}: a venv with {} in {}",
                p.title,
                spec.pip.join(", "),
                venv.display()
            ),
            admin,
            actions: a,
        },
        Step {
            title: format!("Check {}'s Python ({})", p.title, spec.pip.join(", ")),
            admin: false,
            actions: vec![run(&vpy, vec!["-c".into(), spec.check.clone()])],
        },
    ]
}

/// Copies the examples folder of the unpacked source into the version.
pub fn examples_step(layout: &Layout, p: &Product, version: &str, from: &Path) -> Step {
    let admin = layout.scope == Scope::System;
    let to = layout.version_dir(p, version).join("examples");
    let mut a = vec![Action::CopyDir {
        from: from.to_path_buf(),
        to: to.clone(),
    }];
    if admin {
        a.push(Action::Readable { path: to.clone() });
    }
    Step {
        title: format!("Copy {}'s examples into {}", p.title, to.display()),
        admin,
        actions: a,
    }
}

/// The release's source in a folder of downloads: Kraxn's own name
/// (`<archive>-<tag>-source.tar.gz`) or GitHub's "Source code" download
/// (`<Repo>-<version>.tar.gz`), up to three folders deep.
pub fn source_archive(dir: &Path, p: &Product, tag: &str, version: &str) -> Option<PathBuf> {
    let repo = p.repo.rsplit('/').next().unwrap_or(&p.title);
    let names = [
        format!("{}-{tag}-source.tar.gz", p.archive),
        format!("{repo}-{version}.tar.gz"),
    ];
    fn walk(d: &Path, names: &[String], depth: usize) -> Option<PathBuf> {
        let mut dirs = Vec::new();
        for e in std::fs::read_dir(d).ok()?.flatten() {
            let path = e.path();
            if path.is_dir() {
                dirs.push(path);
            } else if names.iter().any(|n| e.file_name().to_str() == Some(n)) {
                return Some(path);
            }
        }
        if depth == 0 {
            return None;
        }
        dirs.sort();
        dirs.iter().find_map(|d| walk(d, names, depth - 1))
    }
    walk(dir, &names, 3)
}

/// What a product gets beside its programs.
#[derive(Clone, Debug, Default)]
pub struct Beside {
    /// The venv: the Python that makes it and the wheels folder, if any.
    pub venv: Option<(PathBuf, Option<PathBuf>)>,
    /// The examples folder of the unpacked source.
    pub examples: Option<PathBuf>,
}

impl Beside {
    pub fn steps(&self, layout: &Layout, p: &Product, version: &str) -> Vec<Step> {
        let mut v = Vec::new();
        if let Some((py, wheels)) = &self.venv {
            v.extend(venv_steps(layout, p, version, py, wheels.as_deref()));
        }
        if let Some(from) = &self.examples {
            v.push(examples_step(layout, p, version, from));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;

    fn linux() -> Platform {
        Platform::parse("linux", "x86_64").unwrap()
    }

    /// Invariant: a venv plan makes the venv with the Python found, pip
    /// installs the catalogue's packages (from the wheels folder when
    /// offline, with `--no-index`), the check runs the catalogue's code,
    /// and a system install marks the venv step for administrator rights
    /// but not the check. Offline without wheels the venv is skipped with
    /// a reason; `python = "none"` and a product without scripts need
    /// nothing. Breaks if pip would reach for the internet offline, or a
    /// venv in /opt were made without rights.
    #[test]
    fn venv_steps_follow_the_plan() {
        let c = Catalogue::builtin().unwrap();
        let foehn = c.get("foehncraft").unwrap();
        let kofl = c.get("koflcraft").unwrap();
        let t = std::env::temp_dir().join(format!("kraxn-extras-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        let mut plan = Plan::default();
        let online = Source::Online {
            api: String::new(),
            token: None,
        };
        let folder = Source::Folder(t.clone());
        assert_eq!(
            py_plan(&plan, foehn, &online),
            PyPlan::Venv { wheels: None }
        );
        assert_eq!(py_plan(&plan, kofl, &online), PyPlan::No);
        assert!(matches!(py_plan(&plan, foehn, &folder), PyPlan::Skip(_)));
        std::fs::create_dir_all(t.join("wheels")).unwrap();
        assert_eq!(
            py_plan(&plan, foehn, &folder),
            PyPlan::Venv {
                wheels: Some(t.join("wheels"))
            }
        );
        plan.python = PyMode::None;
        assert_eq!(py_plan(&plan, foehn, &online), PyPlan::No);

        let layout = Layout::new(Scope::System, linux(), &Env::default(), Some(&t)).unwrap();
        let steps = venv_steps(
            &layout,
            foehn,
            "1.3.0",
            Path::new("/usr/bin/python3"),
            Some(&t.join("wheels")),
        );
        assert_eq!(steps.len(), 2);
        assert!(steps[0].admin && !steps[1].admin);
        let json = serde_json::to_string(&steps).unwrap();
        assert!(json.contains("--no-index") && json.contains("gmsh") && json.contains("h5py"));
        assert!(json.contains("Foehncraft/1.3.0/venv/bin/python"), "{json}");
        assert!(
            !json.contains("--upgrade"),
            "offline pip must not upgrade itself"
        );
        let _ = std::fs::remove_dir_all(&t);
    }

    /// Invariant: the libraries Gmsh loads count as there only when every
    /// one is in a library folder, and a missing ffmpeg or system package
    /// is optional (a note, not a stop) while a missing Python for a venv
    /// is not. Breaks if a missing libGLU slipped through (the venv's
    /// check would fail at the end) or ffmpeg stopped an install.
    #[test]
    fn tools_for_the_extras() {
        let c = Catalogue::builtin().unwrap();
        let foehn = c.get("foehncraft").unwrap();
        let t = std::env::temp_dir().join(format!("kraxn-libs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        let libs = foehn.python.as_ref().unwrap().libs.clone();
        let none: Vec<PathBuf> = Vec::new();
        let s = python_libs(&linux(), &none, foehn, std::slice::from_ref(&t));
        assert!(!s.ok && s.blocks(), "{s:?}");
        for l in &libs {
            std::fs::write(t.join(l), b"").unwrap();
        }
        assert!(python_libs(&linux(), &none, foehn, std::slice::from_ref(&t)).ok);
        let f = ffmpeg(&linux(), &none);
        assert!(!f.ok && f.optional && !f.blocks());
        let p = venv_python(&linux(), &none);
        assert!(!p.ok && p.blocks());
        let _ = std::fs::remove_dir_all(&t);
    }

    /// Invariant: the release's source is found in a folder of downloads
    /// under Kraxn's name or GitHub's "Source code" name, in a subfolder
    /// too. Breaks if offline installs lost their examples.
    #[test]
    fn source_archive_in_a_folder() {
        let c = Catalogue::builtin().unwrap();
        let kofl = c.get("koflcraft").unwrap();
        let t = std::env::temp_dir().join(format!("kraxn-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(t.join("a/b")).unwrap();
        assert_eq!(source_archive(&t, kofl, "v0.3.1", "0.3.1"), None);
        std::fs::write(t.join("a/b/KoflCraft-0.3.1.tar.gz"), b"").unwrap();
        assert_eq!(
            source_archive(&t, kofl, "v0.3.1", "0.3.1"),
            Some(t.join("a/b/KoflCraft-0.3.1.tar.gz"))
        );
        let _ = std::fs::remove_dir_all(&t);
    }
}
