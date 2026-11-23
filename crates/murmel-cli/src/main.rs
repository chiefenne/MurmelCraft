//! `murmel`: the command line of the tool registry. It runs the same tool
//! ids as the window, headless.
//!
//! ```text
//! murmel commands [--markdown]
//! murmel keys [--markdown]
//! murmel call TOOL_ID [--input FILE|-|JSON]
//! murmel run --script FILE
//! murmel diag codes
//! murmel info
//! ```

use std::io::Read;
use std::process::ExitCode;

use murmel_engine::{Session, Tokens, settings};
use serde_json::{Value, json};

const USAGE: &str = "Usage: murmel COMMAND
  murmel commands [--markdown]          list every tool (JSON, or docs/tools.md)
  murmel keys [--markdown]              the window's commands and keys (JSON, or docs/keys.md)
  murmel call TOOL_ID [--input FILE|-|JSON]
                                        run one tool and print its result envelope
  murmel run --script FILE              run a JSON list of {\"tool\", \"input\"} calls
                                        in one session; stops at the first failure
  murmel diag codes                     the diagnostic codes
  murmel info                           version and platform

Example: a headless screenshot of a KoflCraft mesh
  murmel call view.screenshot --input '{\"mesh\": \"foehncraft.vtu\", \"view\": \"iso\", \"path\": \"shot.png\"}'

Settings are read from MURMEL_CONFIG_DIR or the platform's config folder.";

fn session() -> Result<Session, String> {
    let tokens = Tokens::builtin()?;
    let mut s = Session::new(tokens);
    if let Some(dir) = settings::config_dir() {
        let (st, nav, problems) = settings::load(&dir, cfg!(target_os = "macos"));
        for p in problems {
            eprintln!("warning: {p}");
        }
        s.theme = st.theme;
        s.navigation = nav;
        if let Err(e) = s.set_colors(st.colors) {
            eprintln!("warning: settings.toml colors: {e}");
        }
    }
    Ok(s)
}

fn print(v: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
    );
}

fn read_input(arg: Option<&str>) -> Result<Value, String> {
    let Some(a) = arg else {
        return Ok(json!({}));
    };
    let text = if a == "-" {
        let mut t = String::new();
        std::io::stdin()
            .read_to_string(&mut t)
            .map_err(|e| format!("stdin: {e}"))?;
        t
    } else if a.trim_start().starts_with('{') {
        a.to_string()
    } else {
        std::fs::read_to_string(a).map_err(|e| format!("{a}: {e}"))?
    };
    serde_json::from_str(&text).map_err(|e| format!("input is not JSON: {e}"))
}

fn usage_error(msg: &str) -> ExitCode {
    eprintln!("murmel: {msg}\n\n{USAGE}");
    ExitCode::from(2)
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut s = session()?;
    let envelope = match a.as_slice() {
        [] | ["help"] | ["--help"] | ["-h"] => {
            println!("{USAGE}");
            return Ok(ExitCode::SUCCESS);
        }
        ["commands"] => murmel_tools::call("system.commands", json!({}), &mut s),
        ["commands", "--markdown"] => {
            print!("{}", murmel_tools::markdown());
            return Ok(ExitCode::SUCCESS);
        }
        ["keys", "--markdown"] => {
            print!("{}", murmel_tools::commands::keys_markdown());
            return Ok(ExitCode::SUCCESS);
        }
        ["keys"] => {
            let list: Vec<Value> = murmel_tools::commands::commands()
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "label": murmel_tools::commands::english(&c.label_key),
                        "keys": c.keys,
                        "tool": c.tool(),
                    })
                })
                .collect();
            print(
                &json!({"format": "murmelcraft.keys", "format_version": "1.0", "commands": list}),
            );
            return Ok(ExitCode::SUCCESS);
        }
        ["info"] => murmel_tools::call("system.info", json!({}), &mut s),
        ["diag", "codes"] => murmel_tools::call("diag.codes", json!({}), &mut s),
        ["call", id] => murmel_tools::call(id, json!({}), &mut s),
        ["call", id, "--input", input] => murmel_tools::call(id, read_input(Some(input))?, &mut s),
        ["run", "--script", file] => {
            let script = read_input(Some(file))?;
            let Some(calls) = script.as_array() else {
                return Ok(usage_error("the script must be a JSON list of calls"));
            };
            let mut results = Vec::new();
            let mut code = 0;
            for c in calls {
                let id = c["tool"].as_str().unwrap_or("");
                let e = murmel_tools::call(id, c["input"].clone(), &mut s);
                code = e.exit_code;
                results.push(e.to_json());
                if code != 0 {
                    break;
                }
            }
            print(
                &json!({"format": "murmelcraft.script-result", "format_version": "1.0", "calls": results}),
            );
            return Ok(ExitCode::from(code.clamp(0, 255) as u8));
        }
        _ => return Ok(usage_error("unknown command")),
    };
    print(&envelope.to_json());
    Ok(ExitCode::from(envelope.exit_code.clamp(0, 255) as u8))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("murmel: {e}");
            ExitCode::from(2)
        }
    }
}
