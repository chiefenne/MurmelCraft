//! `kraxn`: installs MurmelCraft, KoflCraft and FoehnCraft. The text mode
//! asks the same questions as the window in a terminal, a plan file
//! repeats an install, and `kraxn call` runs any tool with JSON.

mod elevate;
mod i18n;
mod runner;
mod style;
mod text;
mod window;

use std::io::{BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kraxn_core::{Catalogue, Env, Plan, Platform, tools};
use serde_json::{Value, json};

use crate::runner::Runner;

const USAGE: &str = "\
kraxn: installs MurmelCraft, KoflCraft and FoehnCraft

  kraxn                     the window (in a terminal without a desktop: the questions there)
  kraxn window              the window
  kraxn text                the questions in this terminal
  kraxn apply PLAN [--yes] [--dry-run]
                            install what a plan file says (--yes: no question)
  kraxn call TOOL [--input JSON]
                            run one tool, JSON in and out
  kraxn tools [--markdown]  the tools (docs/kraxn-tools.md)
  kraxn codes               the diagnostic codes
  kraxn --version           this version

Plan files, scopes and folders: docs/kraxn.md";

fn work_dir() -> PathBuf {
    std::env::temp_dir().join(format!("kraxn-work-{}", std::process::id()))
}

fn runner(gui: bool) -> Result<Runner, String> {
    let platform = Platform::this().map_err(|e| e.to_string())?;
    Ok(Runner {
        catalogue: Catalogue::builtin().map_err(|e| e.to_string())?,
        platform,
        env: Env::from_process(),
        work: work_dir(),
        elevator: Box::new(elevate::SystemElevator {
            os: platform.os,
            gui,
        }),
        source: None,
    })
}

fn text_mode(r: &mut Runner) -> Result<(), String> {
    let mut t = text::Text {
        t: i18n::I18n::english()?,
        input: std::io::stdin().lock(),
        out: std::io::stdout(),
        runner: r,
    };
    let res = t.run().map_err(|e| e.to_string());
    let _ = std::fs::remove_dir_all(work_dir());
    res
}

fn apply(r: &mut Runner, file: &Path, yes: bool, dry: bool) -> Result<bool, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let plan = Plan::parse(&text).map_err(|e| e.to_string())?;
    let mut t = text::Text {
        t: i18n::I18n::english()?,
        input: std::io::stdin().lock(),
        out: std::io::stdout(),
        runner: r,
    };
    let input = json!({"plan": plan});
    if dry || yes {
        let e = t.runner.quiet("plan.check", &input);
        println!("{}", serde_json::to_string_pretty(&e).unwrap_or_default());
        if dry || !e.ok() {
            return Ok(e.ok());
        }
        t.apply(&input).map_err(|e| e.to_string())?;
    } else {
        t.confirm_and_apply(plan).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_dir_all(work_dir());
    Ok(true)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let fail = |m: String| {
        eprintln!("kraxn: {m}");
        ExitCode::from(1)
    };
    match a.as_slice() {
        [] | ["window"]
            if a == ["window"] || Platform::this().is_ok_and(|p| tools::has_desktop(&p)) =>
        {
            match runner(true).and_then(window::run) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(e),
            }
        }
        [] | ["text"] => {
            if !std::io::stdin().is_terminal() && a.is_empty() {
                // piped input still works, but say what this is
                eprintln!("{USAGE}\n");
            }
            match runner(false).and_then(|mut r| text_mode(&mut r)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(e),
            }
        }
        ["--version" | "-V"] => {
            println!("kraxn {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        ["help" | "--help" | "-h"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        ["tools"] => {
            for t in tools::tools() {
                println!("{:<18} {}", t.id, t.summary);
            }
            ExitCode::SUCCESS
        }
        ["tools", "--markdown"] => {
            print!("{}", tools::markdown());
            ExitCode::SUCCESS
        }
        ["codes"] => {
            for (c, m) in kraxn_core::diag::CODES {
                println!("{c}  {m}");
            }
            ExitCode::SUCCESS
        }
        ["call", tool, rest @ ..] => {
            let input: Value = match rest {
                [] => json!({}),
                ["--input", "-"] | ["--stdin"] => {
                    let mut s = String::new();
                    for l in std::io::stdin().lock().lines().map_while(Result::ok) {
                        s.push_str(&l);
                    }
                    match serde_json::from_str(&s) {
                        Ok(v) => v,
                        Err(e) => return fail(format!("the input is not JSON: {e}")),
                    }
                }
                ["--input", j] => match serde_json::from_str(j) {
                    Ok(v) => v,
                    Err(e) => return fail(format!("--input is not JSON: {e}")),
                },
                _ => return fail(format!("call takes --input JSON\n\n{USAGE}")),
            };
            let mut r = match runner(false) {
                Ok(r) => r,
                Err(e) => return fail(e),
            };
            let e = r.call(tool, &input, &mut |ev| {
                eprintln!("{}", serde_json::to_string(&ev).unwrap_or_default())
            });
            let _ = std::fs::remove_dir_all(work_dir());
            println!("{}", serde_json::to_string_pretty(&e).unwrap_or_default());
            ExitCode::from(e.exit_code as u8)
        }
        ["apply", file, flags @ ..] => {
            let yes = flags.contains(&"--yes");
            let dry = flags.contains(&"--dry-run");
            if let Some(f) = flags.iter().find(|f| !matches!(**f, "--yes" | "--dry-run")) {
                return fail(format!("unknown option {f}\n\n{USAGE}"));
            }
            match runner(false).and_then(|mut r| apply(&mut r, Path::new(file), yes, dry)) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::from(1),
                Err(e) => fail(e),
            }
        }
        ["admin-run", input, output] => {
            match elevate::admin_run(Path::new(input), Path::new(output)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(e.to_string()),
            }
        }
        _ => fail(format!("unknown command\n\n{USAGE}")),
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod window_tests;
