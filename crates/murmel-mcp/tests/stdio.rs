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
        let mut child = Command::new(env!("CARGO_BIN_EXE_murmel-mcp"))
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

/// Invariant: the mesh a session file names is kept within the read
/// roots like a path in a tool input: a session in the write root that
/// names a mesh outside every root is refused (MCP001) and nothing
/// opens, one that names a fixture loads. Breaks when session.load opens
/// the mesh inside the file unchecked, which would let a written file
/// read past the roots; the input-path checks never see that path.
#[test]
fn session_files_stay_inside_the_roots() {
    let w = scratch("session");
    let outside = scratch("session-outside");
    std::fs::copy(
        root().join("test-fixtures/box/foehncraft.vtu"),
        outside.join("box.vtu"),
    )
    .unwrap();
    let mut c = Client::start(&w);
    c.initialize();
    let r = c.call(
        "mesh.open",
        json!({"path": root().join("test-fixtures/box/foehncraft.vtu")}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let r = c.call("session.save", json!({"path": "box.murmel"}));
    assert_eq!(r["isError"], false, "{r}");
    let text = std::fs::read_to_string(w.join("box.murmel")).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["mesh"]["path"] = json!(outside.join("box.vtu"));
    v["mesh"]["absolute"] = json!(outside.join("box.vtu"));
    std::fs::write(w.join("escape.murmel"), v.to_string()).unwrap();

    assert_eq!(c.call("mesh.close", json!({}))["isError"], false);
    let r = c.call("session.load", json!({"path": "escape.murmel"}));
    assert_eq!(envelope(&r)["diagnostics"][0]["code"], "MCP001", "{r}");
    assert_eq!(
        c.call("mesh.info", json!({}))["structuredContent"]["diagnostics"][0]["code"],
        "VIEW001"
    );
    let r = c.call("session.load", json!({"path": "box.murmel"}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(c.call("mesh.info", json!({}))["isError"], false);
}
