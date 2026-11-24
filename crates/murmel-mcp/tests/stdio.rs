//! The MCP server as a client sees it: the `murmel-mcp` program on stdio.

// Helpers outside #[test] functions unwrap too; a failure is a test failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use base64::Engine as _;
use serde_json::{Value, json};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("murmel-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Client {
    fn start(write_root: &Path) -> Self {
        Self::spawn(write_root, None)
    }

    /// The server with --live, driving the window listening at `socket`.
    fn start_live(write_root: &Path, socket: &Path) -> Self {
        Self::spawn(write_root, Some(socket))
    }

    fn spawn(write_root: &Path, live: Option<&Path>) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_murmel-mcp"));
        if let Some(socket) = live {
            cmd.arg("--live").env("MURMEL_LIVE_SOCKET", socket);
        }
        let mut child = cmd
            .arg("--read-root")
            .arg(root().join("test-fixtures"))
            .arg("--write-root")
            .arg(write_root)
            // no settings of the machine running the tests
            .env("MURMEL_CONFIG_DIR", write_root.join("config"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next: 0,
        }
    }

    fn send_raw(&mut self, line: &str) {
        self.stdin.write_all(line.as_bytes()).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send_raw(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        );
        let r = self.read();
        assert_eq!(r["id"], id, "{r}");
        r
    }

    fn initialize(&mut self) {
        let r = self.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {},
                   "clientInfo": {"name": "test", "version": "1"}}),
        );
        assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(r["result"]["serverInfo"]["name"], "murmelcraft");
        self.send_raw(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    }

    /// tools/call; returns the MCP result.
    fn call(&mut self, tool: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({"name": tool, "arguments": args}));
        assert!(r.get("error").is_none(), "{tool}: {r}");
        r["result"].clone()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn envelope(r: &Value) -> &Value {
    &r["structuredContent"]
}

/// Invariant: the server lists exactly the registry's tools, by id, with
/// their input schemas, and refuses calls before the handshake. Breaks when
/// a tool is added to the registry but not offered (or offered under
/// another name), and nothing else starts the real program.
#[test]
fn lists_the_registry_after_the_handshake() {
    let w = scratch("list");
    let mut c = Client::start(&w);
    let early = c.request("tools/list", json!({}));
    assert_eq!(early["error"]["code"], -32002);
    c.initialize();
    assert_eq!(c.request("ping", json!({}))["result"], json!({}));
    let tools = c.request("tools/list", json!({}))["result"]["tools"].clone();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let ids: Vec<&str> = murmel_tools::tools().iter().map(|t| t.id).collect();
    assert_eq!(names, ids);
    for t in tools.as_array().unwrap() {
        assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        assert!(!t["title"].as_str().unwrap().is_empty(), "{}", t["name"]);
    }
    let get = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "view.get_camera");
    assert_eq!(get.unwrap()["annotations"]["readOnlyHint"], true);
    let orbit = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "view.orbit");
    assert_eq!(orbit.unwrap()["annotations"]["readOnlyHint"], false);
    assert_eq!(c.request("nope/nope", json!({}))["error"]["code"], -32601);
    assert_eq!(c.call_unknown(), -32602);
}

impl Client {
    fn call_unknown(&mut self) -> i64 {
        let r = self.request("tools/call", json!({"name": "view.nope", "arguments": {}}));
        r["error"]["code"].as_i64().unwrap()
    }
}

/// Invariant: an agent drives one session across calls (open, display,
/// camera, undo) and a screenshot comes back as a PNG image of the
/// requested size, written below the write root, also without a path.
/// Breaks when each call gets a fresh session (undo would find nothing),
/// or the image content is dropped, and no other test runs the renderer
/// through MCP.
#[test]
fn drives_one_session_and_returns_screenshots() {
    let w = scratch("drive");
    let mut c = Client::start(&w);
    c.initialize();
    let open = c.call(
        "mesh.open",
        json!({"path": root().join("test-fixtures/channel/foehncraft.vtu")}),
    );
    assert_eq!(open["isError"], false, "{open}");
    assert_eq!(envelope(&open)["result"]["validity"], "valid");
    // relative paths start at the write root; read roots accept fixtures
    let r = c.call("display.color_by", json!({"by": "cell_type"}));
    assert_eq!(r["isError"], false, "{r}");
    let before = envelope(&c.call("view.get_camera", json!({})))["result"]["yaw_deg"].clone();
    assert_eq!(
        c.call("view.orbit", json!({"yaw_deg": 30.0, "pitch_deg": 0.0}))["isError"],
        false
    );
    let undo = c.call("journal.undo", json!({}));
    assert_eq!(envelope(&undo)["result"]["undone"], "view.orbit", "{undo}");
    let after = envelope(&c.call("view.get_camera", json!({})))["result"]["yaw_deg"].clone();
    assert_eq!(before, after);

    let shot = c.call(
        "view.screenshot",
        json!({"path": "iso.png", "width": 320, "height": 200, "view": "iso"}),
    );
    assert_eq!(shot["isError"], false, "{shot}");
    let file = w.join("iso.png");
    assert_eq!(envelope(&shot)["result"]["files"][0], json!(file));
    let image = shot["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["type"] == "image")
        .expect("an image block");
    assert_eq!(image["mimeType"], "image/png");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(bytes, std::fs::read(&file).unwrap());
    let info = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap()
        .info()
        .clone();
    assert_eq!((info.width, info.height), (320, 200));

    let unnamed = c.call("view.screenshot", json!({"width": 64, "height": 48}));
    assert_eq!(unnamed["isError"], false, "{unnamed}");
    assert!(w.join("screenshots/shot-0001.png").is_file());
}

/// Invariant: paths outside the roots are refused before the tool runs
/// (MCP001, nothing written or opened), a failing tool is an MCP tool
/// error with the registry's diagnostic, settings are not saved, and bad
/// frames get JSON-RPC errors without ending the session. Breaks when the
/// server passes paths through unchecked or writes the person's settings.
#[test]
fn refuses_paths_outside_the_roots() {
    let w = scratch("refuse");
    let outside = scratch("refuse-outside");
    std::fs::copy(
        root().join("test-fixtures/box/foehncraft.vtu"),
        outside.join("box.vtu"),
    )
    .unwrap();
    let mut c = Client::start(&w);
    c.initialize();
    let r = c.call("mesh.open", json!({"path": outside.join("box.vtu")}));
    assert_eq!(r["isError"], true);
    assert_eq!(envelope(&r)["diagnostics"][0]["code"], "MCP001", "{r}");
    assert_eq!(
        c.call("mesh.info", json!({}))["structuredContent"]["diagnostics"][0]["code"],
        "VIEW001"
    );

    let r = c.call(
        "view.screenshot",
        json!({"mesh": root().join("test-fixtures/box/foehncraft.vtu"),
               "path": outside.join("shot.png"), "width": 32, "height": 32}),
    );
    assert_eq!(envelope(&r)["diagnostics"][0]["code"], "MCP001", "{r}");
    assert!(!outside.join("shot.png").exists());
    let r = c.call("view.screenshot", json!({"path": "../x.png"}));
    assert_eq!(envelope(&r)["diagnostics"][0]["code"], "MCP001", "{r}");

    let r = c.call("settings.set_theme", json!({"theme": "light"}));
    assert_eq!(envelope(&r)["diagnostics"][0]["code"], "SET001", "{r}");
    assert!(!w.join("config/settings.toml").exists());
    let theme = c.call("settings.get", json!({}));
    assert_eq!(envelope(&theme)["result"]["theme"], "light", "{theme}");

    c.send_raw("{not json");
    assert_eq!(c.read()["error"]["code"], -32700);
    c.send_raw(&"x".repeat(murmel_mcp::MAX_REQUEST_BYTES + 10));
    assert_eq!(c.read()["error"]["code"], -32600);
    let r = c.call("view.zoom", json!("not an object"));
    assert_eq!(envelope(&r)["diagnostics"][0]["code"], "TOOL002");
    assert_eq!(c.request("ping", json!({}))["result"], json!({}));
}

/// A stand-in for the window: a listener on a headless session, run on
/// this thread while `drive` talks to the server on another.
fn with_window<T: Send + 'static>(
    socket: &Path,
    drive: impl FnOnce() -> T + Send + 'static,
) -> (T, murmel_engine::Session) {
    use murmel_tools::live::Listener;
    let l = Listener::start(socket, std::sync::Arc::new(|| {})).unwrap();
    let mut s = murmel_engine::Session::new(murmel_engine::Tokens::builtin().unwrap());
    s.config_dir = None;
    let worker = std::thread::spawn(drive);
    while !worker.is_finished() {
        for j in l.jobs() {
            let e = murmel_tools::call(&j.tool, j.input.clone(), &mut s);
            j.answer(e.to_json());
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    (worker.join().unwrap(), s)
}

/// Invariant: with --live the server forwards calls to the window and
/// returns its envelopes and screenshots as images; the roots are still
/// checked before a call leaves (a path outside never reaches the
/// window); without a window a call fails with LIVE001 and the server
/// lives on; the agent is told it drives the person's window. Breaks when
/// --live runs a headless session after all, or the roots are checked
/// only in the headless case.
#[test]
fn live_drives_the_window_and_keeps_the_roots() {
    let out = scratch("live");
    let socket = out.join("sock/live.sock");
    let mut c = Client::start_live(&out, &socket);
    let r = c.request(
        "initialize",
        json!({"protocolVersion": "2025-11-25", "capabilities": {},
               "clientInfo": {"name": "test", "version": "1"}}),
    );
    let text = r["result"]["instructions"].as_str().unwrap();
    assert!(text.contains("open MurmelCraft window"), "{text}");
    c.send_raw(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    // no window yet
    let r = c.call("view.get_camera", json!({}));
    assert_eq!(r["isError"], true);
    assert_eq!(envelope(&r)["diagnostics"][0]["code"], "LIVE001");

    let channel = root().join("test-fixtures/channel/foehncraft.vtu");
    // a real mesh, but outside the roots
    let outside = scratch("live-outside").join("foehncraft.vtu");
    std::fs::copy(&channel, &outside).unwrap();
    let ((opened, refused, shot, mut c), window) = with_window(&socket, move || {
        let opened = c.call("mesh.open", json!({"path": channel}));
        let refused = c.call("mesh.open", json!({"path": outside}));
        let shot = c.call("view.screenshot", json!({"width": 96, "height": 64}));
        (opened, refused, shot, c)
    });
    assert_eq!(opened["isError"], false, "{opened}");
    assert_eq!(envelope(&refused)["diagnostics"][0]["code"], "MCP001");
    assert_eq!(shot["isError"], false, "{shot}");
    let png = shot["content"][1]["data"].as_str().unwrap();
    let png = base64::engine::general_purpose::STANDARD
        .decode(png)
        .unwrap();
    assert_eq!(&png[1..4], b"PNG");
    assert!(out.join("screenshots/shot-0001.png").exists());
    // the calls ran on the window's session
    assert!(window.mesh.is_some());

    // the window has gone again
    let r = c.call("view.get_camera", json!({}));
    assert_eq!(envelope(&r)["diagnostics"][0]["code"], "LIVE001", "{r}");
}
