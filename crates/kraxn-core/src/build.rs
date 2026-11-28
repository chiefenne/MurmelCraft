//! Source builds: the tools a build needs (Rust, a C compiler, CMake,
//! MPI), the steps that install the missing ones, the release's source
//! from GitHub and `cargo install` into a folder of the work area, with
//! cargo's output as it comes. The built programs then go into place by
//! the same steps as a release's.
//!
//! The build itself always runs as you: Rust goes into your `~/.cargo`
//! with rustup, also for an install for every user; only the system
//! packages (apt, dnf) ask for administrator rights.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::actions::{Action, Step, tail};
use crate::catalogue::Product;
use crate::diag::{Res, diag};
use crate::fetch::{Source, fetch};
use crate::layout::Env;
use crate::platform::{Arch, Os, Platform};
use crate::release::{ReleaseFile, compare_versions};

/// One product of a plan that is built from its source.
#[derive(Clone, Debug)]
pub struct BuildWant<'a> {
    pub product: &'a Product,
    pub features: Vec<String>,
}

/// A tool a build needs, as found on this computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolState {
    /// `rust`, `c-compiler`, `cmake` or `mpi`.
    pub id: String,
    /// What is needed, e.g. "Rust 1.98 or newer".
    pub need: String,
    /// What was found (a version or a path), if anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<String>,
    pub ok: bool,
    /// The step that installs or updates it, when Kraxn can.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<Step>,
    /// What you do yourself when Kraxn cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual: Option<String>,
}

impl ToolState {
    /// Missing, and only you can install it.
    pub fn blocks(&self) -> bool {
        !self.ok && self.fix.is_none()
    }
}

/// Folders searched for tools besides the PATH: programs started from
/// the Dock or a desktop menu often get a short PATH without Homebrew's
/// or rustup's folders.
pub fn tool_dirs(env: &Env) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = &env.home {
        v.push(h.join(".cargo").join("bin"));
    }
    v.extend(env.path.iter().cloned());
    v.extend(env.tool_dirs.iter().cloned());
    let mut seen = Vec::new();
    v.retain(|d| {
        let new = !seen.contains(d);
        seen.push(d.clone());
        new
    });
    v
}

/// The first `name` in `dirs`.
pub fn find(platform: &Platform, dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    let file = platform.exe(name);
    dirs.iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

/// The first line a program prints for `args`, if it ends well.
fn first_line(program: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    out.lines().next().map(|l| l.trim().to_string())
}

/// The first `x.y.z` in a line such as `rustc 1.98.0 (abc 2026-11-01)`.
pub fn version_in(line: &str) -> Option<String> {
    line.split_whitespace()
        .map(|w| w.trim_start_matches('v'))
        .find(|w| w.split('.').count() >= 2 && w.split('.').all(|p| p.parse::<u32>().is_ok()))
        .map(str::to_string)
}

fn at_least(have: &str, min: &str) -> bool {
    compare_versions(have, min) != std::cmp::Ordering::Less
}

/// Rust's name for the system (rustup's download folders).
pub fn rust_triple(platform: &Platform) -> &'static str {
    match (platform.os, platform.arch) {
        (Os::Macos, Arch::Aarch64) => "aarch64-apple-darwin",
        (Os::Macos, Arch::X86_64) => "x86_64-apple-darwin",
        (Os::Linux, Arch::Aarch64) => "aarch64-unknown-linux-gnu",
        (Os::Linux, Arch::X86_64) => "x86_64-unknown-linux-gnu",
        (Os::Windows, Arch::Aarch64) => "aarch64-pc-windows-msvc",
        (Os::Windows, Arch::X86_64) => "x86_64-pc-windows-msvc",
    }
}

/// Where rustup-init comes from (`KRAXN_RUSTUP_DIST` for tests).
fn rustup_dist() -> String {
    std::env::var("KRAXN_RUSTUP_DIST")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "https://static.rust-lang.org/rustup/dist".into())
}

/// The package manager of this Linux.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pkg {
    Apt,
    Dnf,
}

fn linux_pkg(platform: &Platform, dirs: &[PathBuf]) -> Option<(Pkg, PathBuf)> {
    if let Some(p) = find(platform, dirs, "apt-get") {
        return Some((Pkg::Apt, p));
    }
    find(platform, dirs, "dnf").map(|p| (Pkg::Dnf, p))
}

/// The step that installs system packages on Linux (with administrator
/// rights).
fn pkg_step(pkg: Pkg, exe: &Path, title: &str, packages: &[&str]) -> Step {
    let pk: Vec<String> = packages.iter().map(|s| s.to_string()).collect();
    let actions = match pkg {
        Pkg::Apt => {
            let env = vec![("DEBIAN_FRONTEND".to_string(), "noninteractive".to_string())];
            let mut args = vec![
                "install".to_string(),
                "-y".into(),
                "--no-install-recommends".into(),
            ];
            args.extend(pk);
            vec![
                Action::Run {
                    program: exe.to_path_buf(),
                    args: vec!["update".into(), "-q".into()],
                    env: env.clone(),
                },
                Action::Run {
                    program: exe.to_path_buf(),
                    args,
                    env,
                },
            ]
        }
        Pkg::Dnf => {
            let mut args = vec!["install".to_string(), "-y".into()];
            args.extend(pk);
            vec![Action::Run {
                program: exe.to_path_buf(),
                args,
                env: Vec::new(),
            }]
        }
    };
    Step {
        title: format!("Install {title} ({})", packages.join(" ")),
        admin: true,
        actions,
    }
}

fn brew_step(brew: &Path, title: &str, formula: &str) -> Step {
    Step {
        title: format!("Install {title} with Homebrew"),
        admin: false,
        actions: vec![Action::Run {
            program: brew.to_path_buf(),
            args: vec!["install".into(), formula.into()],
            env: Vec::new(),
        }],
    }
}

fn winget_step(winget: &Path, title: &str, id: &str, extra: &[&str]) -> Step {
    let mut args: Vec<String> = [
        "install",
        "--id",
        id,
        "-e",
        "--silent",
        "--accept-package-agreements",
        "--accept-source-agreements",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(extra.iter().map(|s| s.to_string()));
    Step {
        // the installers ask Windows for the rights themselves
        title: format!("Install {title} with winget"),
        admin: false,
        actions: vec![Action::Run {
            program: winget.to_path_buf(),
            args,
            env: Vec::new(),
        }],
    }
}

/// The tools for building `wants`, as found now, each with the step that
/// installs it when it is missing. `work` holds downloaded installers.
pub fn check_tools(
    platform: &Platform,
    env: &Env,
    wants: &[BuildWant],
    work: &Path,
) -> Res<Vec<ToolState>> {
    if wants.is_empty() {
        return Ok(Vec::new());
    }
    let dirs = tool_dirs(env);
    let mpi = wants.iter().any(|w| w.features.iter().any(|f| f == "mpi"));
    if mpi && platform.os == Os::Windows {
        return Err(diag(
            "KRX010",
            "MPI builds (feature mpi) are not supported on Windows; leave the feature out",
        ));
    }
    let cpp = wants.iter().any(|w| w.product.needs_cmake);
    let min_rust = wants
        .iter()
        .map(|w| w.product.min_rust.as_str())
        .max_by(|a, b| compare_versions(a, b))
        .unwrap_or("1.0")
        .to_string();
    let mut out = vec![c_compiler(platform, env, &dirs, cpp)];
    out.push(rust(platform, &dirs, &min_rust, work));
    if cpp {
        out.push(cmake(platform, &dirs));
    }
    if mpi {
        out.push(mpi_tool(platform, &dirs));
    }
    Ok(out)
}

fn c_compiler(platform: &Platform, env: &Env, dirs: &[PathBuf], cpp: bool) -> ToolState {
    let need = if cpp {
        "a C and C++ compiler and linker"
    } else {
        "a C compiler and linker"
    };
    let mut t = ToolState {
        id: "c-compiler".into(),
        need: need.into(),
        found: None,
        ok: false,
        fix: None,
        manual: None,
    };
    match platform.os {
        Os::Linux => {
            let cc = find(platform, dirs, "cc");
            let cxx = find(platform, dirs, "c++");
            t.ok = cc.is_some() && (!cpp || cxx.is_some());
            t.found = cc.map(|p| p.display().to_string());
            if !t.ok {
                match linux_pkg(platform, dirs) {
                    Some((Pkg::Apt, exe)) => {
                        t.fix = Some(pkg_step(Pkg::Apt, &exe, "the compilers", &["build-essential"]))
                    }
                    Some((Pkg::Dnf, exe)) => {
                        let mut p = vec!["gcc", "make"];
                        if cpp {
                            p.push("gcc-c++");
                        }
                        t.fix = Some(pkg_step(Pkg::Dnf, &exe, "the compilers", &p));
                    }
                    None => {
                        t.manual = Some(
                            "install gcc (and g++), make and the C library headers with your distribution's package manager, then check again".into(),
                        )
                    }
                }
            }
        }
        Os::Macos => {
            let path = first_line(Path::new("/usr/bin/xcode-select"), &["-p"]);
            t.ok = path.is_some();
            t.found = path;
            if !t.ok {
                t.manual = Some(
                    "install Apple's Command Line Tools: run `xcode-select --install` in Terminal, finish Apple's installer, then check again".into(),
                );
            }
        }
        Os::Windows => {
            let vswhere = std::env::var_os("ProgramFiles(x86)")
                .map(PathBuf::from)
                .or_else(|| env.program_files.clone())
                .map(|p| {
                    p.join("Microsoft Visual Studio")
                        .join("Installer")
                        .join("vswhere.exe")
                });
            let found = vswhere.as_deref().and_then(|v| {
                first_line(
                    v,
                    &[
                        "-latest",
                        "-products",
                        "*",
                        "-requires",
                        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                        "-property",
                        "installationPath",
                    ],
                )
                .filter(|l| !l.is_empty())
            });
            t.ok = found.is_some();
            t.found = found;
            if !t.ok {
                match find(platform, dirs, "winget") {
                    Some(w) => {
                        t.fix = Some(winget_step(
                            &w,
                            "the Visual Studio C++ Build Tools",
                            "Microsoft.VisualStudio.2022.BuildTools",
                            &[
                                "--override",
                                "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended",
                            ],
                        ))
                    }
                    None => {
                        t.manual = Some(
                            "install the Visual Studio C++ Build Tools (https://visualstudio.microsoft.com/visual-cpp-build-tools/, workload \"Desktop development with C++\"), then check again".into(),
                        )
                    }
                }
            }
        }
    }
    t
}

fn rust(platform: &Platform, dirs: &[PathBuf], min: &str, work: &Path) -> ToolState {
    let mut t = ToolState {
        id: "rust".into(),
        need: format!("Rust {min} or newer"),
        found: None,
        ok: false,
        fix: None,
        manual: None,
    };
    let rustc = find(platform, dirs, "rustc");
    let have = rustc
        .as_deref()
        .and_then(|r| first_line(r, &["--version"]))
        .and_then(|l| version_in(&l));
    t.ok = have.as_deref().is_some_and(|v| at_least(v, min))
        && find(platform, dirs, "cargo").is_some();
    t.found = have.map(|v| format!("rustc {v}"));
    if t.ok {
        return t;
    }
    if let Some(rustup) = find(platform, dirs, "rustup") {
        t.fix = Some(Step {
            title: "Update Rust with rustup (stable)".into(),
            admin: false,
            actions: vec![Action::Run {
                program: rustup,
                args: ["toolchain", "install", "stable", "--profile", "minimal"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                env: Vec::new(),
            }],
        });
        return t;
    }
    let exe = platform.exe("rustup-init");
    let url = format!("{}/{}/{exe}", rustup_dist(), rust_triple(platform));
    let to = work.join("rustup").join(&exe);
    t.fix = Some(Step {
        title: "Install Rust with rustup into your home folder (~/.cargo)".into(),
        admin: false,
        actions: vec![
            Action::Download {
                sha256_url: Some(format!("{url}.sha256")),
                url,
                to: to.clone(),
            },
            Action::Run {
                program: to,
                args: [
                    "-y",
                    "--no-modify-path",
                    "--profile",
                    "minimal",
                    "--default-toolchain",
                    "stable",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
                env: Vec::new(),
            },
        ],
    });
    t
}

fn cmake(platform: &Platform, dirs: &[PathBuf]) -> ToolState {
    let min = "3.10";
    let mut t = ToolState {
        id: "cmake".into(),
        need: format!("CMake {min} or newer"),
        found: None,
        ok: false,
        fix: None,
        manual: None,
    };
    let have = find(platform, dirs, "cmake")
        .and_then(|c| first_line(&c, &["--version"]))
        .and_then(|l| version_in(&l));
    t.ok = have.as_deref().is_some_and(|v| at_least(v, min));
    t.found = have.map(|v| format!("cmake {v}"));
    if t.ok {
        return t;
    }
    match platform.os {
        Os::Linux => match linux_pkg(platform, dirs) {
            Some((pkg, exe)) => t.fix = Some(pkg_step(pkg, &exe, "CMake", &["cmake"])),
            None => {
                t.manual = Some(
                    "install CMake with your distribution's package manager, then check again"
                        .into(),
                )
            }
        },
        Os::Macos => match find(platform, dirs, "brew") {
            Some(b) => t.fix = Some(brew_step(&b, "CMake", "cmake")),
            None => {
                t.manual = Some(
                    "install CMake (https://cmake.org/download/, or Homebrew from https://brew.sh and then `brew install cmake`), then check again".into(),
                )
            }
        },
        Os::Windows => match find(platform, dirs, "winget") {
            Some(w) => t.fix = Some(winget_step(&w, "CMake", "Kitware.CMake", &[])),
            None => {
                t.manual = Some(
                    "install CMake (https://cmake.org/download/), then check again".into(),
                )
            }
        },
    }
    t
}

/// libclang, which the MPI bindings need, in the usual folders of Linux.
fn libclang_linux() -> Option<PathBuf> {
    let dirs = [
        "/usr/lib64",
        "/usr/lib",
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
    ];
    let mut cands: Vec<PathBuf> = dirs.iter().map(PathBuf::from).collect();
    if let Ok(rd) = std::fs::read_dir("/usr/lib") {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with("llvm-") {
                cands.push(e.path().join("lib"));
            }
        }
    }
    cands.into_iter().find_map(|d| {
        std::fs::read_dir(&d).ok()?.flatten().find_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            (n.starts_with("libclang") && n.contains(".so")).then(|| e.path())
        })
    })
}

fn mpi_tool(platform: &Platform, dirs: &[PathBuf]) -> ToolState {
    let mut t = ToolState {
        id: "mpi".into(),
        need: "Open MPI and libclang (feature mpi)".into(),
        found: None,
        ok: false,
        fix: None,
        manual: None,
    };
    let mpicc = find(platform, dirs, "mpicc");
    let clang_ok = platform.os != Os::Linux || libclang_linux().is_some();
    t.ok = mpicc.is_some() && clang_ok;
    t.found = mpicc.map(|p| p.display().to_string());
    if t.ok {
        return t;
    }
    match platform.os {
        Os::Linux => match linux_pkg(platform, dirs) {
            Some((Pkg::Apt, exe)) => {
                t.fix = Some(pkg_step(
                    Pkg::Apt,
                    &exe,
                    "Open MPI and libclang",
                    &["libopenmpi-dev", "openmpi-bin", "libclang-dev"],
                ))
            }
            Some((Pkg::Dnf, exe)) => {
                t.fix = Some(pkg_step(
                    Pkg::Dnf,
                    &exe,
                    "Open MPI and libclang",
                    &["openmpi-devel", "clang-devel"],
                ))
            }
            None => {
                t.manual = Some(
                    "install Open MPI (with its headers) and libclang, then check again".into(),
                )
            }
        },
        Os::Macos => match find(platform, dirs, "brew") {
            Some(b) => t.fix = Some(brew_step(&b, "Open MPI", "open-mpi")),
            None => {
                t.manual = Some(
                    "install Homebrew (https://brew.sh), then `brew install open-mpi`, then check again".into(),
                )
            }
        },
        Os::Windows => {}
    }
    t
}

/// The cargo a build uses after the tools are in place, and the PATH it
/// runs with.
pub struct Toolbox {
    pub cargo: PathBuf,
    pub path: Vec<PathBuf>,
    /// `stable` when rustup has it (an older default toolchain stays).
    pub toolchain: Option<String>,
    pub rustc: String,
}

pub fn toolbox(platform: &Platform, env: &Env) -> Res<Toolbox> {
    let dirs = tool_dirs(env);
    let cargo = find(platform, &dirs, "cargo").ok_or_else(|| {
        diag(
            "KRX015",
            "cargo is not there after the tool install; install Rust (https://rustup.rs) and check again",
        )
    })?;
    let toolchain = find(platform, &dirs, "rustup")
        .and_then(|r| first_line(&r, &["run", "stable", "rustc", "--version"]))
        .map(|_| "stable".to_string());
    let rustc = find(platform, &dirs, "rustc")
        .and_then(|r| first_line(&r, &["--version"]))
        .unwrap_or_default();
    Ok(Toolbox {
        cargo,
        path: dirs,
        toolchain,
        rustc,
    })
}

/// Downloads the source of `tag` (GitHub's archive of the tag) into
/// `work` and unpacks it; returns the source folder and the archive's
/// SHA-256.
pub fn fetch_source(
    source: &Source,
    p: &Product,
    tag: &str,
    work: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Res<(PathBuf, String)> {
    let Source::Online { api, .. } = source else {
        return Err(diag(
            "KRX010",
            "a source build needs the internet (cargo downloads the crates)",
        ));
    };
    let name = format!("{}-{tag}-source.tar.gz", p.archive);
    let file = ReleaseFile {
        name: name.clone(),
        url: Some(format!("{api}/repos/{}/tarball/{tag}", p.repo)),
        path: None,
        size: 0,
    };
    let archive = work.join(&name);
    fetch(&file, &archive, progress)?;
    let sha = crate::fetch::sha256_file(&archive)?;
    let dest = work.join("source");
    let _ = std::fs::remove_dir_all(&dest);
    let top = crate::archive::unpack(&archive, &dest)?;
    Ok((top, sha))
}

/// The `cargo install` runs of one product, as people read them.
pub fn cargo_lines(p: &Product, features: &[String]) -> Vec<String> {
    p.build_paths
        .iter()
        .map(|bp| {
            let mut s = format!("cargo install --locked --path {bp}");
            if !features.is_empty() {
                s.push_str(&format!(" --features {}", features.join(",")));
            }
            s
        })
        .collect()
}

/// Builds the product in `src` with `cargo install --root <work>/stage`,
/// sending each line cargo prints to `log`; returns the folder with the
/// programs, README.md and LICENSE.
pub fn cargo_build(
    tb: &Toolbox,
    p: &Product,
    features: &[String],
    native: bool,
    src: &Path,
    work: &Path,
    log: &mut dyn FnMut(String),
) -> Res<PathBuf> {
    let stage = work.join("stage");
    let _ = std::fs::remove_dir_all(&stage);
    let path = std::env::join_paths(&tb.path)
        .map_err(|e| diag("KRX016", format!("PATH for cargo: {e}")))?;
    for bp in &p.build_paths {
        let mut c = Command::new(&tb.cargo);
        c.current_dir(src)
            .args(["install", "--locked", "--force", "--root"])
            .arg(&stage)
            .arg("--path")
            .arg(src.join(bp))
            .env("PATH", &path)
            .env("CARGO_TARGET_DIR", work.join("target"))
            .env("CARGO_TERM_COLOR", "never")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if !features.is_empty() {
            c.arg("--features").arg(features.join(","));
        }
        if let Some(t) = &tb.toolchain {
            c.env("RUSTUP_TOOLCHAIN", t);
        }
        if native {
            let flags = std::env::var("RUSTFLAGS").unwrap_or_default();
            c.env("RUSTFLAGS", format!("{flags} -C target-cpu=native").trim());
        }
        log(format!(
            "$ {}",
            cargo_lines(p, features)
                .into_iter()
                .find(|l| l.contains(bp.as_str()))
                .unwrap_or_default()
        ));
        let mut child = c.spawn().map_err(|e| {
            diag(
                "KRX016",
                format!("{} did not start: {e}", tb.cargo.display()),
            )
        })?;
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            let tx = tx.clone();
            readers.push(std::thread::spawn(move || {
                for l in BufReader::new(out).lines().map_while(Result::ok) {
                    let _ = tx.send(l);
                }
            }));
        }
        if let Some(err) = child.stderr.take() {
            let tx = tx.clone();
            readers.push(std::thread::spawn(move || {
                for l in BufReader::new(err).lines().map_while(Result::ok) {
                    let _ = tx.send(l);
                }
            }));
        }
        drop(tx);
        let mut all = String::new();
        for l in rx {
            all.push_str(&l);
            all.push('\n');
            log(l);
        }
        for r in readers {
            let _ = r.join();
        }
        let status = child
            .wait()
            .map_err(|e| diag("KRX016", format!("cargo: {e}")))?;
        if !status.success() {
            return Err(diag(
                "KRX016",
                format!(
                    "the build of {} ({bp}) ended with {status}:\n{}",
                    p.title,
                    tail(&all, 15)
                ),
            ));
        }
    }
    let bin = stage.join("bin");
    for d in ["README.md", "LICENSE"] {
        if src.join(d).is_file() {
            std::fs::copy(src.join(d), bin.join(d))
                .map_err(|e| diag("KRX007", format!("{}: {e}", bin.join(d).display())))?;
        }
    }
    Ok(bin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;

    /// Invariant: version numbers are read from the tools' usual first
    /// lines. Breaks if a new rustc or cmake line format hides the
    /// version and Kraxn would reinstall a tool that is there.
    #[test]
    fn versions_from_tool_lines() {
        assert_eq!(
            version_in("rustc 1.98.0 (05f9846f8 2026-11-10)").as_deref(),
            Some("1.98.0")
        );
        assert_eq!(
            version_in("cmake version 3.28.3").as_deref(),
            Some("3.28.3")
        );
        assert_eq!(version_in("no version here"), None);
        assert!(at_least("1.98.1", "1.98") && !at_least("1.89.0", "1.98"));
    }

    /// A folder with shell scripts standing in for the tools.
    #[cfg(unix)]
    fn fake_tools(dir: &Path, tools: &[(&str, &str)]) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        for (name, body) in tools {
            let f = dir.join(name);
            std::fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// Invariant: on Linux an old Rust with rustup is updated through
    /// rustup, a missing Rust is installed with a checked rustup-init into
    /// the home folder (not as administrator), CMake only for products
    /// that need it, and MPI only for the mpi feature (refused on
    /// Windows). Breaks if a build would start without its tools, or a
    /// tool were installed that the plan does not need.
    #[cfg(unix)]
    #[test]
    fn tools_and_their_fixes() {
        let d = std::env::temp_dir().join(format!("kraxn-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let c = Catalogue::builtin().unwrap();
        let linux = Platform::parse("linux", "x86_64").unwrap();
        let kofl = BuildWant {
            product: c.get("koflcraft").unwrap(),
            features: Vec::new(),
        };
        let foehn = BuildWant {
            product: c.get("foehncraft").unwrap(),
            features: vec!["mpi".into()],
        };
        // an old rustc with rustup; no home, so ~/.cargo does not count
        let bin = d.join("bin");
        fake_tools(
            &bin,
            &[
                ("rustc", "echo 'rustc 1.80.0 (x 2024-07-01)'"),
                ("cargo", "exit 0"),
                ("rustup", "exit 0"),
                ("cc", "exit 0"),
            ],
        );
        let env = Env {
            path: vec![bin.clone()],
            ..Env::default()
        };
        let t = check_tools(&linux, &env, std::slice::from_ref(&kofl), &d).unwrap();
        let ids: Vec<&str> = t.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["c-compiler", "rust"]);
        assert!(t[0].ok, "{t:?}");
        let r = &t[1];
        assert!(!r.ok && r.found.as_deref() == Some("rustc 1.80.0"));
        let fix = r.fix.as_ref().unwrap();
        assert!(!fix.admin);
        assert!(
            matches!(&fix.actions[0], Action::Run { program, .. } if program.ends_with("rustup"))
        );

        // no Rust at all: rustup-init, checked against its .sha256
        std::fs::remove_file(bin.join("rustup")).unwrap();
        std::fs::remove_file(bin.join("rustc")).unwrap();
        let t = check_tools(&linux, &env, std::slice::from_ref(&kofl), &d).unwrap();
        let fix = t[1].fix.as_ref().unwrap();
        match &fix.actions[0] {
            Action::Download {
                url, sha256_url, ..
            } => {
                assert!(
                    url.ends_with("/x86_64-unknown-linux-gnu/rustup-init"),
                    "{url}"
                );
                assert_eq!(
                    sha256_url.as_deref(),
                    Some(format!("{url}.sha256").as_str())
                );
            }
            a => panic!("{a:?}"),
        }

        // Foehncraft with MPI: CMake and MPI too, and a C++ compiler
        let t = check_tools(&linux, &env, std::slice::from_ref(&foehn), &d).unwrap();
        let ids: Vec<&str> = t.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["c-compiler", "rust", "cmake", "mpi"]);
        assert!(!t[0].ok, "no c++ in the fake folder");
        let win = Platform::parse("windows", "x86_64").unwrap();
        assert_eq!(
            check_tools(&win, &env, &[foehn], &d).unwrap_err().code,
            "KRX010"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
