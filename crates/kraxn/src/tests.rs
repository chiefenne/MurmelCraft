//! The text mode and the command line, end to end on a temporary folder.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use kraxn_core::actions::NoElevation;
use kraxn_core::fetch::sha256_file;
use kraxn_core::{Catalogue, Env, Platform};

use crate::i18n::I18n;
use crate::runner::Runner;
use crate::text::Text;

pub(crate) fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kraxn-bin-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A KoflCraft release in `dir` whose `kofl` exits 0.
pub(crate) fn kofl_release(dir: &Path, v: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let top = format!("koflcraft-{v}-linux-x86_64");
    let name = format!("{top}.tar.gz");
    let f = std::fs::File::create(dir.join(&name)).unwrap();
    let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
    let mut b = tar::Builder::new(gz);
    for (n, data, mode) in [
        ("kofl", &b"#!/bin/sh\nexit 0\n"[..], 0o755),
        ("mesh-mcp", &b"#!/bin/sh\n"[..], 0o755),
        ("LICENSE", &b"l"[..], 0o644),
    ] {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(mode);
        h.set_cksum();
        b.append_data(&mut h, format!("{top}/{n}"), data).unwrap();
    }
    b.into_inner().unwrap().finish().unwrap();
    let sum = sha256_file(&dir.join(&name)).unwrap();
    std::fs::write(dir.join("SHA256SUMS.txt"), format!("{sum}  {name}\n")).unwrap();
}

pub(crate) fn runner(home: &Path) -> Runner {
    Runner {
        catalogue: Catalogue::builtin().unwrap(),
        platform: Platform::parse("linux", "x86_64").unwrap(),
        env: Env {
            home: Some(home.to_path_buf()),
            ..Env::default()
        },
        work: home.join("work"),
        elevator: Box::new(NoElevation),
        source: None,
    }
}

fn session(r: &mut Runner, answers: &str) -> String {
    let mut out = Vec::new();
    {
        let mut t = Text {
            t: I18n::english().unwrap(),
            input: Cursor::new(answers.as_bytes().to_vec()),
            out: &mut out,
            runner: r,
        };
        t.run().unwrap();
    }
    String::from_utf8(out).unwrap()
}

/// Invariant: answering the text mode's questions (for you, from a
/// folder, KoflCraft, latest, PATH, save the plan, start) installs
/// KoflCraft from the folder, reports the SHA-256 check and the start
/// check, and saves a plan that `kraxn apply` reads back; the list then
/// shows it as the default, and removing it in the text mode leaves
/// nothing. Breaks if a question is skipped or misread (the answers
/// would shift), or the text mode stopped calling the tools.
#[cfg(unix)]
#[test]
fn text_mode_installs_lists_and_removes() {
    let d = tmp("text");
    let home = d.join("home");
    std::fs::create_dir_all(&home).unwrap();
    kofl_release(&d.join("dl"), "0.3.1");
    let mut r = runner(&home);
    let plan_file = d.join("plan.toml");
    let answers = format!(
        "1\n1\n\n2\n{}\n2\n\n\n\n{}\n\n4\n5\n",
        d.join("dl").display(),
        plan_file.display()
    );
    let out = session(&mut r, &answers);
    assert!(out.contains("Kraxn will do this:"), "{out}");
    assert!(out.contains("KoflCraft 0.3.1 in "), "{out}");
    assert!(
        out.contains("SHA-256 checked against SHA256SUMS.txt"),
        "{out}"
    );
    assert!(out.contains("started and ended without an error"), "{out}");
    assert!(out.contains("KoflCraft 0.3.1 (default)"), "{out}");
    assert!(home.join(".local/opt/KoflCraft/0.3.1/bin/kofl").is_file());
    assert!(home.join(".local/bin/kofl").exists());
    let plan = kraxn_core::Plan::parse(&std::fs::read_to_string(&plan_file).unwrap()).unwrap();
    assert_eq!(plan.installs[0].product, "koflcraft");
    assert_eq!(plan.offline_dir.as_deref(), Some(d.join("dl").as_path()));

    // remove: for you, default folder, KoflCraft, all versions, yes
    let out = session(&mut r, "3\n1\n\n1\n2\n\n5\n");
    assert!(out.contains("rm -rf"), "{out}");
    assert!(!home.join(".local/opt/KoflCraft").exists(), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Invariant: docs/kraxn-tools.md is what `kraxn tools --markdown`
/// prints. Breaks when a tool or code changes without the page.
#[test]
fn tools_page_is_up_to_date() {
    let page = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/kraxn-tools.md"
    ))
    .unwrap_or_default();
    assert_eq!(
        page,
        kraxn_core::tools::markdown(),
        "run: cargo run -p kraxn -- tools --markdown > docs/kraxn-tools.md"
    );
}

/// A local stand-in for GitHub: each path answers with its body, every
/// other path with 404. Returns the base URL.
#[cfg(unix)]
fn serve(routes: impl FnOnce(&str) -> Vec<(String, Vec<u8>)>) -> String {
    use std::io::{BufRead, BufReader, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let routes = routes(&base);
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
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
            let (status, body) = match routes.iter().find(|(p, _)| *p == path) {
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
    base
}

/// Stand-ins for `cc`, `rustc` 1.99 and a `cargo` that "builds"
/// KoflCraft's programs into `--root`.
#[cfg(unix)]
pub(crate) fn fake_build_tools(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let cargo = r#"root=""
while [ $# -gt 0 ]; do case "$1" in --root) root="$2"; shift;; esac; shift; done
echo "   Compiling kofl v0.3.1" >&2
mkdir -p "$root/bin"
for p in kofl mesh-mcp; do printf '#!/bin/sh\nexit 0\n' > "$root/bin/$p"; chmod 755 "$root/bin/$p"; done"#;
    std::fs::create_dir_all(dir).unwrap();
    for (name, body) in [
        ("cc", "exit 0"),
        ("rustc", "echo 'rustc 1.99.0 (fake 2026-11-01)'"),
        ("cargo", cargo),
    ] {
        let f = dir.join(name);
        std::fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// A KoflCraft 0.3.1 release with no file for Linux, and its source, on
/// a local stand-in for GitHub; a runner that uses it and the fake tools.
#[cfg(unix)]
pub(crate) fn source_runner(d: &Path, home: &Path) -> Runner {
    let tools = d.join("tools");
    fake_build_tools(&tools);
    let src = d.join("src.tar.gz");
    {
        let f = std::fs::File::create(&src).unwrap();
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        let mut b = tar::Builder::new(gz);
        for (n, data) in [("README.md", &b"r"[..]), ("LICENSE", &b"l"[..])] {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, format!("chiefenne-KoflCraft-abc/{n}"), data)
                .unwrap();
        }
        b.into_inner().unwrap().finish().unwrap();
    }
    let tar = std::fs::read(&src).unwrap();
    let base = serve(|base| {
        vec![
            (
                "/repos/chiefenne/KoflCraft/releases?per_page=50".into(),
                format!(
                    r#"[{{"tag_name": "v0.3.1", "prerelease": false, "assets": [
                    {{"name": "koflcraft-0.3.1-windows-x86_64.zip", "browser_download_url": "{base}/w", "size": 0}}]}}]"#
                )
                .into_bytes(),
            ),
            ("/repos/chiefenne/KoflCraft/tarball/v0.3.1".into(), tar),
        ]
    });
    let mut r = runner(home);
    r.env.path = vec![tools, "/usr/bin".into(), "/bin".into()];
    r.source = Some(kraxn_core::fetch::Source::Online {
        api: base,
        token: None,
    });
    r
}

/// Invariant: online, a product with no release file for this computer
/// is offered as a source build; saying yes shows the tools, builds it
/// with cargo (its output shown), installs the programs and says they
/// were built here. Breaks if the text mode dropped such a product, the
/// questions shifted, or the report claimed a SHA256SUMS check.
#[cfg(unix)]
#[test]
fn text_mode_builds_from_source() {
    let d = tmp("text-source");
    let home = d.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut r = source_runner(&d, &home);
    // install, for you, default folder, GitHub, KoflCraft, build: yes,
    // version: latest, native: no, PATH: yes, examples: yes, no plan
    // file, start; quit
    let out = session(&mut r, "1\n1\n\n1\n2\n\n\n\n\n\n\n\n5\n");
    assert!(
        out.contains("Kraxn can build 0.3.1 from the source"),
        "{out}"
    );
    assert!(out.contains("Tools this install needs:"), "{out}");
    assert!(
        out.contains("Rust 1.88 or newer: there (rustc 1.99.0)"),
        "{out}"
    );
    assert!(out.contains("Compiling kofl v0.3.1"), "{out}");
    assert!(
        out.contains("built here from the source (chiefenne/KoflCraft@v0.3.1)"),
        "{out}"
    );
    assert!(home.join(".local/opt/KoflCraft/0.3.1/bin/kofl").is_file());
    let _ = std::fs::remove_dir_all(&d);
}
