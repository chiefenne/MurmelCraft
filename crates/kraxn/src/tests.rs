//! The text mode and the command line, end to end on a temporary folder.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use kraxn_core::actions::NoElevation;
use kraxn_core::fetch::sha256_file;
use kraxn_core::{Catalogue, Env, Platform};

use crate::i18n::I18n;
use crate::runner::Runner;
use crate::text::Text;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kraxn-bin-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A KoflCraft release in `dir` whose `kofl` exits 0.
fn kofl_release(dir: &Path, v: &str) {
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

fn runner(home: &Path) -> Runner {
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
        "1\n1\n\n2\n{}\n2\n\n\n{}\n\n4\n5\n",
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
