//! End-to-end tests of installs into a temporary folder, from a folder of
//! downloads and from a local stand-in for GitHub.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use super::*;
use crate::actions::NoElevation;
use crate::archive::tests::make_tar;
use crate::fetch::sha256_file;
use crate::plan::Want;

struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("kraxn-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Self(d)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn linux() -> Platform {
    Platform::parse("linux", "x86_64").unwrap()
}

/// A Foehncraft release `v` in `dir`: the archive with a `foehn` that
/// exits 0 (or 3 when `broken`), README.md, LICENSE and SHA256SUMS.txt.
fn release(dir: &Path, v: &str, broken: bool) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let name = format!("foehncraft-{v}-linux-x86_64.tar.gz");
    let script = if broken {
        "#!/bin/sh\nexit 3\n"
    } else {
        "#!/bin/sh\nexit 0\n"
    };
    make_tar(
        &dir.join(&name),
        &format!("foehncraft-{v}-linux-x86_64"),
        &[
            ("foehn", script.as_bytes(), 0o755),
            ("README.md", b"r", 0o644),
            ("LICENSE", b"l", 0o644),
        ],
    );
    let sum = sha256_file(&dir.join(&name)).unwrap();
    std::fs::write(dir.join("SHA256SUMS.txt"), format!("{sum}  {name}\n")).unwrap();
    dir.join(name)
}

fn plan(root: &Path, offline: &Path, version: &str) -> Plan {
    Plan {
        prefix: Some(root.to_path_buf()),
        offline_dir: Some(offline.to_path_buf()),
        installs: vec![Want {
            version: version.into(),
            ..Want::new("foehncraft")
        }],
        ..Plan::default()
    }
}

fn env(home: &Path) -> Env {
    Env {
        home: Some(home.to_path_buf()),
        ..Env::default()
    }
}

fn run(ctx: &Ctx, plan: &Plan, work: &Path) -> Res<Report> {
    let source = ctx.source(plan);
    apply(ctx, plan, &source, work, &mut NoElevation, &mut |_| {})
}

/// Invariant: an offline install of two versions side by side gives each
/// its folder, receipt and env files, `current` and the PATH link follow
/// the default, the shell block is written once, the new program starts;
/// removing the default version makes the other one the default, and
/// removing the product takes its folder, links and the PATH block away.
/// Breaks if any of these steps is skipped or points to the wrong
/// version; the unit tests check each piece, only this one the whole.
#[cfg(unix)]
#[test]
fn offline_install_versions_default_and_removal() {
    let t = Tmp::new("e2e");
    let (root, offline, home) = (t.0.join("root"), t.0.join("dl"), t.0.join("home"));
    std::fs::create_dir_all(&home).unwrap();
    release(&offline.join("a"), "1.2.0", false);
    release(&offline.join("b"), "1.3.0", false);
    let c = Catalogue::builtin().unwrap();
    let e = env(&home);
    let ctx = Ctx {
        catalogue: &c,
        platform: linux(),
        env: &e,
    };

    let r = run(&ctx, &plan(&root, &offline, "1.2.0"), &t.0.join("work")).unwrap();
    assert!(r.ok, "{r:?}");
    let p = &r.products[0];
    assert!(
        p.verified && !p.already && p.check.as_ref().unwrap().ok,
        "{p:?}"
    );
    let v12 = root.join("Foehncraft/1.2.0");
    assert!(v12.join("bin/foehn").is_file() && v12.join("README.md").is_file());
    assert!(v12.join("env.sh").is_file() && v12.join(RECEIPT).is_file());
    assert_eq!(
        std::fs::read_link(root.join("Foehncraft/current")).unwrap(),
        Path::new("1.2.0")
    );
    assert_eq!(
        std::fs::read_link(root.join("bin/foehn")).unwrap(),
        root.join("Foehncraft/current/bin/foehn")
    );
    let profile = std::fs::read_to_string(home.join(".profile")).unwrap();
    assert!(
        profile.contains(&root.join("bin").display().to_string()),
        "{profile}"
    );

    // the newer one beside it, not the default
    let mut p2 = plan(&root, &offline, "latest");
    p2.installs[0].make_default = false;
    let r = run(&ctx, &p2, &t.0.join("work")).unwrap();
    assert!(r.ok && r.products[0].version == "1.3.0", "{r:?}");
    let layout = ctx.layout(&p2).unwrap();
    let foehn = c.get("foehncraft").unwrap();
    let have = installed::product(&layout, foehn);
    let vs: Vec<(&str, bool)> = have
        .versions
        .iter()
        .map(|v| (v.version.as_str(), v.default))
        .collect();
    assert_eq!(vs, [("1.3.0", false), ("1.2.0", true)]);
    assert_eq!(have.versions[0].receipt.as_ref().unwrap().source, "folder");
    assert_eq!(
        std::fs::read_to_string(home.join(".profile")).unwrap(),
        profile
    );

    // the same again: nothing copied
    let r = run(&ctx, &p2, &t.0.join("work")).unwrap();
    assert!(r.ok && r.products[0].already, "{r:?}");

    // remove the default: 1.3.0 takes over
    let steps = remove(&ctx, &layout, "foehncraft", Some("1.2.0")).unwrap();
    assert!(run_all(&steps, &mut NoElevation).iter().all(|o| o.ok));
    assert!(!v12.exists());
    assert_eq!(
        installed::default_of(&layout, foehn).as_deref(),
        Some("1.3.0")
    );
    assert!(root.join("bin/foehn").exists());

    // the product: folder, link and the PATH block go
    let steps = remove(&ctx, &layout, "foehncraft", None).unwrap();
    assert!(run_all(&steps, &mut NoElevation).iter().all(|o| o.ok));
    assert!(!root.join("Foehncraft").exists());
    assert!(std::fs::symlink_metadata(root.join("bin/foehn")).is_err());
    assert!(
        !std::fs::read_to_string(home.join(".profile"))
            .unwrap()
            .contains("kraxn")
    );
    assert_eq!(
        remove(&ctx, &layout, "foehncraft", None).unwrap_err().code,
        "KRX009"
    );
}

/// Invariant: a file whose SHA-256 differs from SHA256SUMS.txt stops the
/// install with KRX004 before anything is copied, and a program that does
/// not start makes the report fail with its check. Breaks if a damaged
/// download were installed, or a broken program reported as success
/// (AGENTS.md: no partial result shown as success).
#[cfg(unix)]
#[test]
fn bad_sums_and_broken_programs_fail() {
    let t = Tmp::new("bad");
    let (root, offline, home) = (t.0.join("root"), t.0.join("dl"), t.0.join("home"));
    std::fs::create_dir_all(&home).unwrap();
    let a = release(&offline, "1.3.0", false);
    std::fs::write(
        offline.join("SHA256SUMS.txt"),
        format!(
            "{}  {}\n",
            "0".repeat(64),
            a.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
    let c = Catalogue::builtin().unwrap();
    let e = env(&home);
    let ctx = Ctx {
        catalogue: &c,
        platform: linux(),
        env: &e,
    };
    let err = run(&ctx, &plan(&root, &offline, "1.3.0"), &t.0.join("work")).unwrap_err();
    assert_eq!(err.code, "KRX004");
    assert!(!root.join("Foehncraft").exists());

    std::fs::remove_file(offline.join("SHA256SUMS.txt")).unwrap();
    let err = run(&ctx, &plan(&root, &offline, "1.3.0"), &t.0.join("work")).unwrap_err();
    assert_eq!(err.code, "KRX005");

    let broken = t.0.join("dl2");
    release(&broken, "1.3.0", true);
    let r = run(&ctx, &plan(&root, &broken, "1.3.0"), &t.0.join("work")).unwrap();
    assert!(!r.ok);
    assert!(!r.products[0].check.as_ref().unwrap().ok);
}

/// Invariant: online, Kraxn reads the release list from the GitHub API,
/// downloads the archive and SHA256SUMS.txt from the URLs it names and
/// installs the newest release; a release list Kraxn cannot reach says
/// KRX003. Breaks if the API path, the JSON fields or the download stop
/// working; the offline tests do not touch the network code.
#[cfg(unix)]
#[test]
fn online_install_from_a_github_stand_in() {
    let t = Tmp::new("online");
    let (root, files, home) = (t.0.join("root"), t.0.join("files"), t.0.join("home"));
    std::fs::create_dir_all(&home).unwrap();
    let a = release(&files, "1.3.0", false);
    let archive = std::fs::read(&a).unwrap();
    let sums = std::fs::read(files.join("SHA256SUMS.txt")).unwrap();
    let addr = serve_with_list(vec![
        ("/dl/a".to_string(), archive),
        ("/dl/s".to_string(), sums),
        (
            "/repos/chiefenne/Foehncraft/releases?per_page=50".to_string(),
            Vec::new(),
        ),
    ]);
    let c = Catalogue::builtin().unwrap();
    let e = env(&home);
    let ctx = Ctx {
        catalogue: &c,
        platform: linux(),
        env: &e,
    };
    let mut p = plan(&root, &files, "latest");
    p.offline_dir = None;
    let source = Source::Online {
        api: addr.clone(),
        token: None,
    };
    let r = apply(
        &ctx,
        &p,
        &source,
        &t.0.join("work"),
        &mut NoElevation,
        &mut |_| {},
    )
    .unwrap();
    assert!(r.ok && r.products[0].verified, "{r:?}");
    assert!(root.join("Foehncraft/1.3.0/bin/foehn").is_file());
    let none = Source::Online {
        api: "http://127.0.0.1:9".into(),
        token: None,
    };
    let err = apply(
        &ctx,
        &p,
        &none,
        &t.0.join("work"),
        &mut NoElevation,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(err.code, "KRX003");
}

/// A one-thread HTTP server for `responses` (path, body; 404 otherwise),
/// with the release list (the last response) written to point at
/// the server's own `/dl/a` and `/dl/s`.
fn serve_with_list(mut responses: Vec<(String, Vec<u8>)>) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let list = format!(
        r#"[{{"tag_name": "v1.3.0", "prerelease": false, "assets": [
            {{"name": "foehncraft-1.3.0-linux-x86_64.tar.gz", "browser_download_url": "{base}/dl/a", "size": 0}},
            {{"name": "SHA256SUMS.txt", "browser_download_url": "{base}/dl/s", "size": 0}}]}}]"#
    );
    if let Some(last) = responses.last_mut() {
        last.1 = list.into_bytes();
    }
    spawn_server(l, responses);
    base
}

/// Answers each request on `l` with the body of its path (404 otherwise).
fn spawn_server(l: TcpListener, responses: Vec<(String, Vec<u8>)>) {
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let mut line = String::new();
            let mut r = BufReader::new(s.try_clone().unwrap());
            if r.read_line(&mut line).is_err() {
                continue;
            }
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                    break;
                }
            }
            let path = line.split(' ').nth(1).unwrap_or("").to_string();
            let (status, body) = match responses.iter().find(|(p, _)| *p == path) {
                Some((_, b)) => ("200 OK", b.clone()),
                None => ("404 Not Found", b"no".to_vec()),
            };
            let _ = write!(
                s,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(&body);
        }
    });
}

/// Invariant: the preview of a system install marks the copy as needing
/// administrator rights, but not the user's own shell files, and shows
/// the download as `<download>/…`. Breaks if Kraxn asked for rights for
/// user files (they would end up owned by root) or a system copy ran
/// without them.
#[test]
fn preview_marks_admin_steps() {
    let c = Catalogue::builtin().unwrap();
    let t = Tmp::new("preview");
    let e = env(&t.0);
    let ctx = Ctx {
        catalogue: &c,
        platform: linux(),
        env: &e,
    };
    let mut p = Plan {
        scope: Scope::System,
        installs: vec![Want::new("koflcraft")],
        ..Plan::default()
    };
    p.prefix = Some(t.0.join("opt"));
    let r = Release {
        product: "koflcraft".into(),
        version: "0.3.1".into(),
        tag: "v0.3.1".into(),
        prerelease: false,
        files: vec![],
    };
    let kofl = c.get("koflcraft").unwrap();
    let steps = preview(&ctx, &p, &ctx.source(&p), &[(kofl, r)], &[]).unwrap();
    assert!(steps[0].admin && steps[0].title.starts_with("Install KoflCraft 0.3.1"));
    let shown: Vec<String> = steps[0]
        .actions
        .iter()
        .map(crate::actions::describe)
        .collect();
    assert!(
        shown
            .iter()
            .any(|s| s.contains("<download>/koflcraft-0.3.1-linux-x86_64/kofl")),
        "{shown:?}"
    );
    // a system install on Linux leaves the user's start files alone
    assert!(steps.iter().all(|s| {
        s.actions
            .iter()
            .all(|a| !matches!(a, Action::ShellBlock { .. }))
    }));
}

/// A folder of stand-ins for the build tools: `cc`, a `rustc` of
/// `rust`, and a `cargo` that "builds" KoflCraft's programs (scripts
/// that exit 0) into `--root` and prints a Compiling line.
#[cfg(unix)]
fn fake_build_tools(dir: &Path, rust: &str) {
    use std::os::unix::fs::PermissionsExt;
    let cargo = r#"root=""; feat=""
while [ $# -gt 0 ]; do case "$1" in --root) root="$2"; shift;; --features) feat="$2"; shift;; esac; shift; done
echo "   Compiling kofl v0.3.1 $feat" >&2
mkdir -p "$root/bin"
for p in kofl mesh-mcp; do printf '#!/bin/sh\nexit 0\n' > "$root/bin/$p"; chmod 755 "$root/bin/$p"; done"#;
    std::fs::create_dir_all(dir).unwrap();
    for (name, body) in [
        ("cc", "exit 0".to_string()),
        ("rustc", format!("echo 'rustc {rust} (fake 2026-11-01)'")),
        ("cargo", cargo.to_string()),
    ] {
        let f = dir.join(name);
        std::fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Invariant: a source build (from = "source") takes the release's tag
/// even when the release has no file for this system, downloads GitHub's
/// archive of the tag, runs cargo with its output reported line by line,
/// installs the built programs with a receipt that says "build", and
/// starts the check; a Rust older than the product needs gets rustup
/// offered. Breaks if a build went through the release path, cargo's
/// output were lost, or the build ran with a Rust too old for it.
#[cfg(unix)]
#[test]
fn source_build_installs_the_built_programs() {
    let t = Tmp::new("source");
    let (root, home, tools) = (t.0.join("root"), t.0.join("home"), t.0.join("tools"));
    std::fs::create_dir_all(&home).unwrap();
    fake_build_tools(&tools, "1.99.0");
    let tar = t.0.join("src.tar.gz");
    make_tar(
        &tar,
        "chiefenne-KoflCraft-abc1234",
        &[
            ("Cargo.toml", b"[workspace]", 0o644),
            ("README.md", b"r", 0o644),
            ("LICENSE", b"l", 0o644),
        ],
    );
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    // the release has only a Windows file: nothing for Linux
    let list = format!(
        r#"[{{"tag_name": "v0.3.1", "prerelease": false, "assets": [
            {{"name": "koflcraft-0.3.1-windows-x86_64.zip", "browser_download_url": "{base}/dl/w", "size": 0}}]}}]"#
    );
    spawn_server(
        l,
        vec![
            (
                "/repos/chiefenne/KoflCraft/releases?per_page=50".into(),
                list.into_bytes(),
            ),
            (
                "/repos/chiefenne/KoflCraft/tarball/v0.3.1".into(),
                std::fs::read(&tar).unwrap(),
            ),
        ],
    );
    let c = Catalogue::builtin().unwrap();
    let e = Env {
        home: Some(home.clone()),
        path: vec![tools.clone(), "/usr/bin".into(), "/bin".into()],
        ..Env::default()
    };
    let ctx = Ctx {
        catalogue: &c,
        platform: linux(),
        env: &e,
    };
    let plan = Plan {
        prefix: Some(root.clone()),
        path: false,
        installs: vec![Want {
            from: crate::plan::Origin::Source,
            ..Want::new("koflcraft")
        }],
        ..Plan::default()
    };
    let source = Source::Online {
        api: base,
        token: None,
    };
    let work = t.0.join("work");
    let tools_now = super::tools(&ctx, &plan, &work).unwrap();
    assert!(tools_now.iter().all(|t| t.ok), "{tools_now:?}");
    let resolved = resolve(&ctx, &plan, &source).unwrap();
    let shown = preview(&ctx, &plan, &source, &resolved, &tools_now).unwrap();
    assert!(
        shown[1]
            .title
            .starts_with("Build KoflCraft 0.3.1 with cargo"),
        "{shown:?}"
    );
    let mut log = Vec::new();
    let r = apply(&ctx, &plan, &source, &work, &mut NoElevation, &mut |ev| {
        if let Event::Log { line } = ev {
            log.push(line);
        }
    })
    .unwrap();
    assert!(r.ok, "{r:?}");
    assert!(r.products[0].built && r.products[0].check.as_ref().unwrap().ok);
    assert!(log.iter().any(|l| l.contains("Compiling kofl")), "{log:?}");
    let v = root.join("KoflCraft/0.3.1");
    assert!(v.join("bin/kofl").is_file() && v.join("bin/mesh-mcp").is_file());
    assert!(v.join("README.md").is_file() && v.join("LICENSE").is_file());
    let receipt = std::fs::read_to_string(v.join(RECEIPT)).unwrap();
    assert!(receipt.contains("source = \"build\""), "{receipt}");
    assert!(receipt.contains("chiefenne/KoflCraft@v0.3.1"), "{receipt}");
    assert!(receipt.contains("rustc 1.99.0"), "{receipt}");

    // an old Rust without rustup: rustup-init is offered
    fake_build_tools(&tools, "1.80.0");
    let tools_old = super::tools(&ctx, &plan, &work).unwrap();
    let rust = tools_old.iter().find(|t| t.id == "rust").unwrap();
    assert!(
        !rust.ok && rust.fix.is_some(),
        "rustup-init is offered: {rust:?}"
    );
}

/// Invariant: installing MurmelCraft for you on Linux writes its menu
/// entry (desktop file, icon, file types) into `~/.local/share`, and
/// removing it takes them away; `menu = false` writes none. Breaks if
/// the window got no entry, or a removed program left one behind.
#[cfg(unix)]
#[test]
fn menu_entry_comes_and_goes_with_the_window() {
    let t = Tmp::new("menu");
    let (root, offline, home) = (t.0.join("root"), t.0.join("dl"), t.0.join("home"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&offline).unwrap();
    let name = "murmelcraft-0.7.1-linux-x86_64.tar.gz";
    let ok: &[u8] = b"#!/bin/sh\nexit 0\n";
    make_tar(
        &offline.join(name),
        "murmelcraft-0.7.1-linux-x86_64",
        &[
            ("murmelcraft", ok, 0o755),
            ("murmel", ok, 0o755),
            ("murmel-mcp", ok, 0o755),
            ("LICENSE", b"l", 0o644),
        ],
    );
    let sum = sha256_file(&offline.join(name)).unwrap();
    std::fs::write(offline.join("SHA256SUMS.txt"), format!("{sum}  {name}\n")).unwrap();
    let c = Catalogue::builtin().unwrap();
    let e = env(&home);
    let ctx = Ctx {
        catalogue: &c,
        platform: linux(),
        env: &e,
    };
    let mut plan = Plan {
        prefix: Some(root.clone()),
        offline_dir: Some(offline.clone()),
        path: false,
        installs: vec![Want::new("murmelcraft")],
        ..Plan::default()
    };
    let share = home.join(".local/share");
    let desktop = share.join("applications/io.github.chiefenne.murmelcraft.desktop");
    plan.menu = false;
    let r = run(&ctx, &plan, &t.0.join("w0")).unwrap();
    assert!(r.ok, "{r:?}");
    assert!(!desktop.exists());
    let l = ctx.layout(&plan).unwrap();
    run_all(
        &remove(&ctx, &l, "murmelcraft", None).unwrap(),
        &mut NoElevation,
    );

    plan.menu = true;
    let r = run(&ctx, &plan, &t.0.join("w1")).unwrap();
    assert!(r.ok, "{r:?}");
    let text = std::fs::read_to_string(&desktop).unwrap();
    assert!(
        text.contains("MurmelCraft/current/bin/murmelcraft"),
        "{text}"
    );
    assert!(
        share
            .join("icons/hicolor/256x256/apps/io.github.chiefenne.murmelcraft.png")
            .is_file()
    );
    assert!(
        share
            .join("mime/packages/io.github.chiefenne.murmelcraft.xml")
            .is_file()
    );
    let o = run_all(
        &remove(&ctx, &l, "murmelcraft", None).unwrap(),
        &mut NoElevation,
    );
    assert!(o.iter().all(|o| o.ok), "{o:?}");
    assert!(!desktop.exists() && !root.join("MurmelCraft").exists());
}
