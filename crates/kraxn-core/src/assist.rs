//! The AI assistants on this computer and the family's MCP servers in
//! them: Kraxn finds Claude Code, Claude Desktop, ChatGPT's Codex and
//! Gemini CLI, registers each installed program's server (MurmelCraft's
//! `murmel mcp --live`, KoflCraft's `kofl mcp`, FoehnCraft's `foehn mcp`)
//! under the same name the READMEs use, keeps a copy of every settings
//! file before it changes it, and after the install starts each server
//! once and asks it the MCP `initialize` question.
//!
//! Claude Code is changed with its own `claude mcp` command (its settings
//! file holds much else); Claude Desktop's and Gemini's JSON and Codex's
//! TOML are edited directly. The servers point at the default version
//! (`current` on macOS and Linux, the PATH folder's links on Windows), so
//! a new default needs no new registration.

use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::actions::{Action, Step};
use crate::build;
use crate::catalogue::Product;
use crate::diag::{Res, diag};
use crate::layout::{Env, Layout};
use crate::platform::{Os, Platform};

/// An assistant Kraxn knows how to set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostId {
    ClaudeCode,
    ClaudeDesktop,
    Codex,
    Gemini,
}

impl HostId {
    pub const ALL: [HostId; 4] = [
        HostId::ClaudeCode,
        HostId::ClaudeDesktop,
        HostId::Codex,
        HostId::Gemini,
    ];

    pub fn title(self) -> &'static str {
        match self {
            HostId::ClaudeCode => "Claude Code",
            HostId::ClaudeDesktop => "Claude Desktop",
            HostId::Codex => "ChatGPT (Codex)",
            HostId::Gemini => "Gemini CLI",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            HostId::ClaudeCode => "claude-code",
            HostId::ClaudeDesktop => "claude-desktop",
            HostId::Codex => "codex",
            HostId::Gemini => "gemini",
        }
    }
}

/// An assistant as found on this computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub id: HostId,
    pub title: String,
    /// Its program (Claude Code) or settings folder was found.
    pub found: bool,
    /// The settings file Kraxn changes (it may not exist yet).
    pub config: Option<PathBuf>,
    /// The `claude` program (Claude Code only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<PathBuf>,
}

/// One MCP server: its name in the assistants, program and arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    /// The folder the assistant may read and write.
    pub dir: PathBuf,
}

/// Folders where assistants' programs are, besides the tool folders
/// (npm's and the installers' own).
fn host_dirs(env: &Env) -> Vec<PathBuf> {
    let mut v = build::tool_dirs(env);
    if let Some(h) = &env.home {
        v.push(h.join(".local").join("bin"));
        v.push(h.join(".claude").join("local"));
        v.push(h.join(".npm-global").join("bin"));
    }
    if let Some(a) = &env.app_data {
        v.push(a.join("npm"));
    }
    v
}

fn find_program(platform: &Platform, dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    build::find(platform, dirs, name).or_else(|| {
        (platform.os == Os::Windows)
            .then(|| {
                dirs.iter()
                    .map(|d| d.join(format!("{name}.cmd")))
                    .find(|p| p.is_file())
            })
            .flatten()
    })
}

/// The assistants Kraxn knows, each with whether it is on this computer.
pub fn detect(platform: &Platform, env: &Env) -> Vec<Host> {
    let dirs = host_dirs(env);
    let home = env.home.clone();
    HostId::ALL
        .iter()
        .filter_map(|&id| {
            let host = |found, config, program| Host {
                id,
                title: id.title().into(),
                found,
                config,
                program,
            };
            match id {
                HostId::ClaudeCode => {
                    let program = find_program(platform, &dirs, "claude");
                    let config = home.as_ref().map(|h| h.join(".claude.json"));
                    Some(host(program.is_some(), config, program))
                }
                HostId::ClaudeDesktop => {
                    let folder = match platform.os {
                        Os::Macos => home
                            .as_ref()
                            .map(|h| h.join("Library/Application Support/Claude")),
                        Os::Windows => env.app_data.as_ref().map(|a| a.join("Claude")),
                        // no Claude Desktop for Linux
                        Os::Linux => None,
                    }?;
                    Some(host(
                        folder.is_dir(),
                        Some(folder.join("claude_desktop_config.json")),
                        None,
                    ))
                }
                HostId::Codex => {
                    let folder = home.as_ref()?.join(".codex");
                    let found = folder.is_dir() || find_program(platform, &dirs, "codex").is_some();
                    Some(host(found, Some(folder.join("config.toml")), None))
                }
                HostId::Gemini => {
                    let folder = home.as_ref()?.join(".gemini");
                    let found =
                        folder.is_dir() || find_program(platform, &dirs, "gemini").is_some();
                    Some(host(found, Some(folder.join("settings.json")), None))
                }
            }
        })
        .collect()
}

/// The product's server, pointing at the default version, and the folder
/// it may use (`dir`, else `~/<the catalogue's folder>`). On Windows
/// without the PATH links (`linked`) it names `version`'s own program.
pub fn server(
    layout: &Layout,
    env: &Env,
    p: &Product,
    version: &str,
    linked: bool,
    dir: Option<&Path>,
) -> Option<Server> {
    let m = p.mcp.as_ref()?;
    let windows = layout.platform.os == Os::Windows;
    let dir = match dir {
        Some(d) => d.to_path_buf(),
        None => env.home.as_ref()?.join(&m.dir),
    };
    let command = if layout.platform.unix() {
        layout.current_link(p).join("bin").join(&m.program)
    } else if linked {
        layout.bin_link(&m.program)
    } else {
        layout.bin_target(p, version, &m.program)
    };
    let (name, args) = match (&m.windows_name, windows) {
        (Some(n), true) => (n.clone(), m.windows_args.clone()),
        _ => (m.name.clone(), m.args.clone()),
    };
    let d = dir.display().to_string();
    Some(Server {
        name,
        command,
        args: args.iter().map(|a| a.replace("{dir}", &d)).collect(),
        dir,
    })
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// A JSON settings file with `servers` in its `mcpServers` (and `remove`
/// taken out). Keys come back sorted; Kraxn keeps a copy of the old file.
pub fn json_with(text: Option<&str>, servers: &[Server], remove: &[String]) -> Res<String> {
    let mut v: Value = match text.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => serde_json::from_str(t)
            .map_err(|e| diag("KRX018", format!("the settings are not valid JSON: {e}")))?,
        None => json!({}),
    };
    let obj = v
        .as_object_mut()
        .ok_or_else(|| diag("KRX018", "the settings are not a JSON object"))?;
    let m = obj
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| diag("KRX018", "mcpServers is not a JSON object"))?;
    for r in remove {
        m.remove(r);
    }
    for s in servers {
        m.insert(
            s.name.clone(),
            json!({"command": s.command, "args": s.args}),
        );
    }
    let mut out = serde_json::to_string_pretty(&v).map_err(|e| diag("KRX018", e.to_string()))?;
    out.push('\n');
    Ok(out)
}

/// The names in a JSON settings file's `mcpServers`.
pub fn json_names(text: &str) -> Vec<String> {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.get("mcpServers")?.as_object().cloned())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

fn toml_header_names(line: &str) -> Option<String> {
    let l = line.trim();
    let inner = l.strip_prefix('[')?.strip_suffix(']')?.trim();
    let rest = inner.strip_prefix("mcp_servers.")?;
    let name = if let Some(q) = rest.strip_prefix('"') {
        q.split('"').next()?.to_string()
    } else {
        rest.split('.').next()?.trim().to_string()
    };
    Some(name)
}

/// Codex's config.toml with `servers` as `[mcp_servers.NAME]` tables
/// (replacing tables of the same names, and `remove`), the rest of the
/// file as it was.
pub fn toml_with(text: Option<&str>, servers: &[Server], remove: &[String]) -> Res<String> {
    let text = text.unwrap_or("");
    toml::from_str::<toml::Table>(text)
        .map_err(|e| diag("KRX018", format!("the settings are not valid TOML: {e}")))?;
    let drop: Vec<&str> = servers
        .iter()
        .map(|s| s.name.as_str())
        .chain(remove.iter().map(String::as_str))
        .collect();
    let mut out = String::new();
    let mut skipping = false;
    for line in text.lines() {
        if line.trim_start().starts_with('[') {
            skipping = toml_header_names(line).is_some_and(|n| drop.contains(&n.as_str()));
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    for s in servers {
        if !out.is_empty() {
            out.push('\n');
        }
        let q = |x: &str| toml::Value::String(x.to_string()).to_string();
        out.push_str(&format!(
            "[mcp_servers.{}]\ncommand = {}\nargs = [{}]\n",
            s.name,
            q(&s.command.display().to_string()),
            s.args.iter().map(|a| q(a)).collect::<Vec<_>>().join(", ")
        ));
    }
    toml::from_str::<toml::Table>(&out)
        .map_err(|e| diag("KRX018", format!("Kraxn made invalid TOML: {e}")))?;
    Ok(out)
}

/// The names of Codex's `[mcp_servers.NAME]` tables.
pub fn toml_names(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text.lines().filter_map(toml_header_names).collect();
    v.dedup();
    v
}

/// The names registered in Claude Code (`mcpServers` of `~/.claude.json`,
/// the user's scope).
fn claude_code_names(host: &Host) -> Vec<String> {
    host.config
        .as_deref()
        .and_then(read)
        .map(|t| json_names(&t))
        .unwrap_or_default()
}

/// The program a host starts for `name`.
pub fn registered_command(host: &Host, name: &str) -> Option<PathBuf> {
    let text = host.config.as_deref().and_then(read)?;
    let cmd = match host.id {
        HostId::Codex => {
            let t: toml::Table = toml::from_str(&text).ok()?;
            t.get("mcp_servers")?
                .get(name)?
                .get("command")?
                .as_str()?
                .to_string()
        }
        _ => {
            let v: Value = serde_json::from_str(&text).ok()?;
            v.get("mcpServers")?
                .get(name)?
                .get("command")?
                .as_str()?
                .to_string()
        }
    };
    Some(PathBuf::from(cmd))
}

/// The registered names in a host.
pub fn names(host: &Host) -> Vec<String> {
    let text = host.config.as_deref().and_then(read);
    match host.id {
        HostId::ClaudeCode => claude_code_names(host),
        HostId::Codex => text.map(|t| toml_names(&t)).unwrap_or_default(),
        _ => text.map(|t| json_names(&t)).unwrap_or_default(),
    }
}

fn backup(config: &Path) -> Option<Action> {
    config.is_file().then(|| {
        let mut name = config.file_name().unwrap_or_default().to_os_string();
        name.push(".kraxn-backup");
        Action::CopyFile {
            from: config.to_path_buf(),
            to: config.with_file_name(name),
            executable: false,
        }
    })
}

fn run(program: &Path, args: Vec<String>) -> Action {
    Action::Run {
        program: program.to_path_buf(),
        args,
        env: Vec::new(),
        optional: false,
    }
}

/// The steps that put `servers` into `host` (as you, never with
/// administrator rights: the settings are yours) and make their folders.
pub fn register_steps(host: &Host, servers: &[Server]) -> Res<Vec<Step>> {
    if servers.is_empty() {
        return Ok(Vec::new());
    }
    let mut a: Vec<Action> = servers
        .iter()
        .map(|s| Action::MakeDir {
            path: s.dir.clone(),
        })
        .collect();
    let config = host.config.clone().unwrap_or_default();
    a.extend(backup(&config));
    match host.id {
        HostId::ClaudeCode => {
            let claude = host
                .program
                .clone()
                .ok_or_else(|| diag("KRX018", "Claude Code's claude program was not found"))?;
            let have = claude_code_names(host);
            for s in servers {
                if have.contains(&s.name) {
                    a.push(run(
                        &claude,
                        ["mcp", "remove", "--scope", "user", &s.name]
                            .map(String::from)
                            .to_vec(),
                    ));
                }
                let mut args: Vec<String> = ["mcp", "add", "--scope", "user", &s.name, "--"]
                    .map(String::from)
                    .to_vec();
                args.push(s.command.display().to_string());
                args.extend(s.args.iter().cloned());
                a.push(run(&claude, args));
            }
        }
        HostId::Codex => {
            if let Some(dir) = config.parent() {
                a.push(Action::MakeDir {
                    path: dir.to_path_buf(),
                });
            }
            a.push(Action::Write {
                path: config.clone(),
                text: toml_with(read(&config).as_deref(), servers, &[])?,
                executable: false,
            });
        }
        HostId::ClaudeDesktop | HostId::Gemini => {
            if let Some(dir) = config.parent() {
                a.push(Action::MakeDir {
                    path: dir.to_path_buf(),
                });
            }
            a.push(Action::Write {
                path: config.clone(),
                text: json_with(read(&config).as_deref(), servers, &[])?,
                executable: false,
            });
        }
    }
    let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
    let mut title = format!("Register {} with {}", names.join(", "), host.title);
    if host.id == HostId::ClaudeDesktop {
        title.push_str(" (restart Claude Desktop to see it)");
    }
    Ok(vec![Step {
        title,
        admin: false,
        actions: a,
    }])
}

/// The steps that take `names` out of every host that has them (when a
/// product is removed), each settings file copied first; only entries
/// whose program `ours` claims (another install's stay).
pub fn unregister_steps(
    hosts: &[Host],
    names: &[String],
    ours: &dyn Fn(&Path) -> bool,
) -> Vec<Step> {
    let mut steps = Vec::new();
    for h in hosts {
        let gone: Vec<String> = names
            .iter()
            .filter(|n| registered_command(h, n).is_some_and(|c| ours(&c)))
            .cloned()
            .collect();
        if gone.is_empty() {
            continue;
        }
        let config = h.config.clone().unwrap_or_default();
        let mut a: Vec<Action> = backup(&config).into_iter().collect();
        let text = read(&config);
        match h.id {
            HostId::ClaudeCode => match &h.program {
                Some(claude) => {
                    for n in &gone {
                        a.push(run(
                            claude,
                            ["mcp", "remove", "--scope", "user", n]
                                .map(String::from)
                                .to_vec(),
                        ));
                    }
                }
                None => continue,
            },
            HostId::Codex => match toml_with(text.as_deref(), &[], &gone) {
                Ok(t) => a.push(Action::Write {
                    path: config.clone(),
                    text: t,
                    executable: false,
                }),
                Err(_) => continue,
            },
            _ => match json_with(text.as_deref(), &[], &gone) {
                Ok(t) => a.push(Action::Write {
                    path: config.clone(),
                    text: t,
                    executable: false,
                }),
                Err(_) => continue,
            },
        }
        steps.push(Step {
            title: format!("Take {} out of {}", gone.join(", "), h.title),
            admin: false,
            actions: a,
        });
    }
    steps
}

/// The result of starting a server once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCheck {
    pub name: String,
    pub command: String,
    pub ok: bool,
    pub message: String,
}

/// Starts `s`, asks it MCP's `initialize` and waits up to `wait` for the
/// answer; the server is stopped again either way.
pub fn start_check(s: &Server, wait: Duration) -> ServerCheck {
    let command = format!("{} {}", s.command.display(), s.args.join(" "));
    let done = |ok: bool, message: String| ServerCheck {
        name: s.name.clone(),
        command: command.clone(),
        ok,
        message,
    };
    let child = Command::new(&s.command)
        .args(&s.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return done(false, format!("did not start: {e}")),
    };
    let hello = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "kraxn", "version": env!("CARGO_PKG_VERSION")}}
    });
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{hello}");
        // stdin stays open until the answer: some servers end on EOF
        std::mem::forget(stdin);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(out).read_line(&mut line);
            let _ = tx.send(line);
        });
    }
    let answer = rx.recv_timeout(wait);
    let _ = child.kill();
    let _ = child.wait();
    match answer {
        Ok(line) => match serde_json::from_str::<Value>(&line) {
            Ok(v) if v.get("result").is_some() => {
                let name = v["result"]["serverInfo"]["name"]
                    .as_str()
                    .unwrap_or("the server");
                done(true, format!("{name} answered MCP's initialize"))
            }
            Ok(v) => done(false, format!("answered with an error: {}", v["error"])),
            Err(_) if line.is_empty() => done(false, "ended without an answer".into()),
            Err(_) => done(false, format!("answered something else: {}", line.trim())),
        },
        Err(_) => done(false, format!("no answer within {} s", wait.as_secs())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str) -> Server {
        Server {
            name: name.into(),
            command: PathBuf::from("/opt/KoflCraft/current/bin/kofl"),
            args: vec![
                "mcp".into(),
                "--read-root".into(),
                "/home/me/kofl-work".into(),
            ],
            dir: PathBuf::from("/home/me/kofl-work"),
        }
    }

    /// Invariant: a JSON settings file keeps its other keys and servers,
    /// gets the new server (replacing one of the same name) and loses
    /// removed ones; an empty or missing file becomes a new one; a file
    /// that is not JSON is refused (KRX018) instead of overwritten.
    /// Breaks if Kraxn dropped a person's other settings or servers.
    #[test]
    fn json_settings_keep_the_rest() {
        let old = r#"{"theme": "dark", "mcpServers": {"other": {"command": "x"}, "koflcraft": {"command": "old"}}}"#;
        let new = json_with(Some(old), &[server("koflcraft")], &[]).unwrap();
        let v: Value = serde_json::from_str(&new).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(
            v["mcpServers"]["koflcraft"]["command"],
            "/opt/KoflCraft/current/bin/kofl"
        );
        assert_eq!(v["mcpServers"]["koflcraft"]["args"][0], "mcp");
        let gone = json_with(Some(&new), &[], &["koflcraft".into()]).unwrap();
        assert_eq!(json_names(&gone), vec!["other".to_string()]);
        assert!(
            json_with(None, &[server("a")], &[])
                .unwrap()
                .contains("\"a\"")
        );
        assert_eq!(
            json_with(Some("{not json"), &[server("a")], &[])
                .unwrap_err()
                .code,
            "KRX018"
        );
    }

    /// Invariant: Codex's config.toml keeps its other lines and comments,
    /// a table of the same name (and its sub-tables) is replaced, removed
    /// names go, and the result parses with the paths quoted. Breaks if
    /// Kraxn left two tables of one name (Codex would refuse the file) or
    /// lost the person's settings.
    #[test]
    fn codex_toml_keeps_the_rest() {
        let old = "# mine\nmodel = \"o5\"\n\n[mcp_servers.koflcraft]\ncommand = \"old\"\n\n[mcp_servers.koflcraft.env]\nA = \"1\"\n\n[mcp_servers.other]\ncommand = \"x\"\n";
        let new = toml_with(Some(old), &[server("koflcraft")], &[]).unwrap();
        assert!(new.starts_with("# mine\nmodel = \"o5\"\n"), "{new}");
        let t: toml::Table = toml::from_str(&new).unwrap();
        let m = t["mcp_servers"].as_table().unwrap();
        assert_eq!(m["other"]["command"].as_str(), Some("x"));
        assert_eq!(
            m["koflcraft"]["command"].as_str(),
            Some("/opt/KoflCraft/current/bin/kofl")
        );
        assert!(m["koflcraft"].get("env").is_none());
        assert_eq!(toml_names(&new), vec!["other", "koflcraft"]);
        let gone = toml_with(Some(&new), &[], &["koflcraft".into()]).unwrap();
        assert_eq!(toml_names(&gone), vec!["other"]);
        assert!(toml_with(Some("a = ["), &[], &[]).is_err());
    }

    /// Invariant: hosts are found by their program or settings folder,
    /// Claude Desktop is not offered on Linux, a registration backs up an
    /// existing settings file first and never asks for administrator
    /// rights, and Claude Code goes through its own `claude mcp` (removing
    /// an old entry of the same name first). Breaks if Kraxn changed a
    /// settings file without a copy, or ran `claude mcp add` onto an
    /// existing name (Claude Code refuses that).
    #[cfg(unix)]
    #[test]
    fn hosts_and_registration() {
        let t = std::env::temp_dir().join(format!("kraxn-assist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let home = t.join("home");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::create_dir_all(home.join("bin")).unwrap();
        let claude = home.join("bin/claude");
        std::fs::write(&claude, "#!/bin/sh\n").unwrap();
        std::fs::write(
            home.join(".claude.json"),
            r#"{"mcpServers": {"koflcraft": {"command": "old"}}}"#,
        )
        .unwrap();
        std::fs::write(home.join(".codex/config.toml"), "model = \"o5\"\n").unwrap();
        let env = Env {
            home: Some(home.clone()),
            path: vec![home.join("bin")],
            ..Env::default()
        };
        let linux = Platform::parse("linux", "x86_64").unwrap();
        let hosts = detect(&linux, &env);
        let found: Vec<HostId> = hosts.iter().filter(|h| h.found).map(|h| h.id).collect();
        assert_eq!(found, vec![HostId::ClaudeCode, HostId::Codex]);
        assert!(hosts.iter().all(|h| h.id != HostId::ClaudeDesktop));

        let cc = &hosts[0];
        let steps = register_steps(cc, &[server("koflcraft")]).unwrap();
        assert!(!steps[0].admin);
        let shown: Vec<String> = steps[0]
            .actions
            .iter()
            .map(crate::actions::describe)
            .collect();
        let at = |p: &str| shown.iter().position(|s| s.contains(p)).unwrap();
        assert!(at(".claude.json.kraxn-backup") < at("mcp remove --scope user koflcraft"));
        assert!(at("mcp remove") < at("mcp add --scope user koflcraft -- /opt/KoflCraft"));

        let codex = &hosts[1];
        let mut s = server("koflcraft");
        s.dir = t.join("work");
        let steps = register_steps(codex, &[s]).unwrap();
        crate::actions::run_steps(&steps);
        assert!(home.join(".codex/config.toml.kraxn-backup").is_file());
        assert_eq!(names(codex), vec!["koflcraft"]);
        assert!(t.join("work").is_dir());
        let kofl = |c: &Path| c.starts_with("/opt/KoflCraft");
        let un = unregister_steps(&hosts, &["koflcraft".into()], &kofl);
        // Claude Code's entry is another install's ("old")
        assert_eq!(un.len(), 1, "{un:?}");
        assert!(un[0].title.contains("ChatGPT"));
        let _ = std::fs::remove_dir_all(&t);
    }

    /// Invariant: the start check passes for a program that answers
    /// `initialize` with a result and fails, with the reason, for one that
    /// cannot start or says nothing. Breaks if a broken registration were
    /// reported as working.
    #[cfg(unix)]
    #[test]
    fn start_check_asks_initialize() {
        let t = std::env::temp_dir().join(format!("kraxn-mcp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        let good = t.join("good");
        std::fs::write(
            &good,
            "#!/bin/sh\nread l\necho '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"serverInfo\":{\"name\":\"kofl\"}}}'\n",
        )
        .unwrap();
        let quiet = t.join("quiet");
        std::fs::write(&quiet, "#!/bin/sh\nexit 0\n").unwrap();
        for f in [&good, &quiet] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let s = |c: &Path| Server {
            name: "k".into(),
            command: c.to_path_buf(),
            args: vec![],
            dir: t.clone(),
        };
        let w = Duration::from_secs(5);
        let ok = start_check(&s(&good), w);
        assert!(ok.ok && ok.message.contains("kofl"), "{ok:?}");
        assert!(!start_check(&s(&quiet), w).ok);
        assert!(!start_check(&s(&t.join("missing")), w).ok);
        let _ = std::fs::remove_dir_all(&t);
    }
}
