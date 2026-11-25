//! `murmel`: the command line of the tool registry. It runs the same tool
//! ids as the window, headless.
//!
//! ```text
//! murmel commands [--markdown]
//! murmel keys [--markdown]
//! murmel call TOOL_ID [--input FILE|-|JSON]
//! murmel run --script FILE
//! murmel mcp --read-root DIR --write-root DIR [--live]
//! murmel diag codes
//! murmel info
//! murmel bench make --cells N --out DIR [--formats vtu,polymesh-binary,…]
//! murmel bench run MESH [--repeat N] [--no-render] [--no-fields] [--size WxH] [--json]
//! murmel blocks make MESH --out FILE [--block-cells N]
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
  murmel mcp --root DIR                 the MCP server for agents on stdio, confined
                                        to DIR (or --read-root DIR --write-root DIR);
                                        --live drives the open window instead;
                                        `murmel mcp --help` for more
  murmel diag codes                     the diagnostic codes
  murmel info                           version and platform
  murmel bench make --cells N --out DIR [--formats LIST]
                                        write a synthetic polyhedral mesh of N cells
                                        (formats vtu, vtu-binary, polymesh,
                                        polymesh-binary, blocked; default all);
                                        blocked is a blocked VTKHDF file written
                                        without the mesh in memory, any size up
                                        to 684 M cells
  murmel bench run MESH [--repeat N] [--no-render] [--no-fields] [--size WxH] [--json]
                                        time reading, the surface, picking and
                                        drawing of a mesh (docs/performance.md)
  murmel blocks make MESH --out FILE [--block-cells N]
                                        write MESH and its fields as a blocked
                                        VTKHDF file (docs/blocked-vtkhdf.md), the
                                        layout read by regions of a large mesh

Example: a headless screenshot of a KoflCraft mesh
  murmel call view.screenshot --input '{\"mesh\": \"foehncraft.vtu\", \"view\": \"iso\", \"path\": \"shot.png\"}'

Settings are read from MURMEL_CONFIG_DIR or the platform's config folder. The
digest (BLAKE3, or SHA-256 for older evidence) of each mesh file hashed is
remembered there (checked-files.json), so a file opened again unchanged is
not hashed again. The preview grid of a mesh opened as an overview is kept
in MURMEL_CACHE_DIR or the platform's cache folder, so it is not sampled
again.";

fn session() -> Result<Session, String> {
    let tokens = Tokens::builtin()?;
    let mut s = Session::new(tokens);
    // A mesh file hashed before and unchanged since is not hashed again.
    s.remember_checked = true;
    // An overview's preview grid, kept so it is not sampled again.
    s.cache_dir = settings::cache_dir();
    if let Some(dir) = settings::config_dir() {
        let (st, nav, problems) = settings::load(&dir, cfg!(target_os = "macos"));
        for p in problems {
            eprintln!("warning: {p}");
        }
        s.theme = st.theme;
        s.navigation = nav;
        // All of the file, so a settings tool run here writes it back
        // whole.
        s.view_cube = st.view_cube;
        s.agents = st.agents.enabled;
        s.defaults = st.defaults;
        s.formulas = st.formulas;
        s.memory_budget = st.memory.budget_gib;
        murmel_engine::defaults::start_view(&mut s);
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
        ["bench", rest @ ..] => return bench(rest),
        ["blocks", "make", mesh, rest @ ..] => {
            let value = |name: &str| {
                rest.iter()
                    .position(|a| *a == name)
                    .and_then(|i| rest.get(i + 1))
                    .copied()
            };
            let Some(out) = value("--out") else {
                return Ok(usage_error("blocks make needs --out FILE"));
            };
            let mut input = json!({"mesh": mesh, "out": out});
            if let Some(n) = value("--block-cells") {
                let n: u64 = n
                    .replace('_', "")
                    .parse()
                    .map_err(|_| "--block-cells takes a whole number")?;
                input["block_cells"] = json!(n);
            }
            murmel_tools::call("blocks.make", input, &mut s)
        }
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

/// `murmel bench make …` and `murmel bench run …`.
fn bench(args: &[&str]) -> Result<ExitCode, String> {
    let flag = |name: &str| args.iter().position(|a| *a == name);
    let value = |name: &str| flag(name).and_then(|i| args.get(i + 1)).copied();
    match args.first() {
        Some(&"make") => {
            let cells: usize = value("--cells")
                .ok_or("bench make needs --cells N")?
                .replace('_', "")
                .parse()
                .map_err(|_| "--cells takes a whole number")?;
            let out = value("--out").ok_or("bench make needs --out DIR")?;
            let formats: Vec<String> = match value("--formats") {
                Some(f) => f.split(',').map(|s| s.trim().to_string()).collect(),
                None => murmel_tools::bench::FORMATS
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            };
            // the blocked writer's steps, each time one moves on by 1 %
            let start = std::time::Instant::now();
            let shown = std::sync::Mutex::new((String::new(), usize::MAX));
            let report_step = |step: &str, done: usize, total: usize| {
                let pct = done * 100 / total.max(1);
                if let Ok(mut s) = shown.lock()
                    && (s.0 != step || s.1 != pct)
                {
                    *s = (step.to_string(), pct);
                    eprintln!(
                        "{:>7.1} s  {step} {done}/{total} ({pct} %)",
                        start.elapsed().as_secs_f64()
                    );
                }
            };
            let report = murmel_tools::bench::make(
                cells,
                std::path::Path::new(out),
                &formats,
                &report_step,
            )?;
            print(&report);
            Ok(ExitCode::SUCCESS)
        }
        Some(&"run") => {
            let mesh = args.get(1).ok_or("bench run needs a mesh path")?;
            let mut opts = murmel_tools::bench::Options::default();
            if let Some(r) = value("--repeat") {
                opts.repeat = r.parse().map_err(|_| "--repeat takes a whole number")?;
            }
            if flag("--no-fields").is_some() {
                opts.fields = false;
            }
            if flag("--no-render").is_some() {
                opts.render = false;
            }
            if let Some(s) = value("--size") {
                let (w, h) = s.split_once('x').ok_or("--size takes WxH")?;
                opts.size = [
                    w.parse().map_err(|_| "--size takes WxH")?,
                    h.parse().map_err(|_| "--size takes WxH")?,
                ];
            }
            let report = murmel_tools::bench::run(std::path::Path::new(mesh), &opts)?;
            if flag("--json").is_some() {
                print(&report);
            } else {
                print!("{}", murmel_tools::bench::table(&report));
            }
            Ok(ExitCode::SUCCESS)
        }
        _ => Ok(usage_error("bench takes make or run")),
    }
}

fn main() -> ExitCode {
    let mut os = std::env::args_os().skip(1);
    if os.next().is_some_and(|a| a == "mcp") {
        return murmel_mcp::main(os.collect());
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("murmel: {e}");
            ExitCode::from(2)
        }
    }
}
