//! The MCP side: JSON-RPC 2.0 messages in, responses out. The methods are
//! `initialize`, `ping`, `tools/list` and `tools/call` of the MCP
//! specification (2025-11-25, and 2025-06-18 for older clients). One
//! session lives as long as the connection, so the camera, the display,
//! the selection and the journal carry from one call to the next.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use murmel_engine::Session;
use murmel_tools::{Diagnostic, Envelope, Tool};
use serde_json::{Map, Value, json};

use crate::roots::Roots;

pub const PROTOCOL_VERSION: &str = "2025-11-25";
const OLDER_VERSIONS: &[&str] = &["2025-06-18"];

/// PNGs up to this size come back inline as an MCP image; larger ones are
/// only written (the text says so).
pub const MAX_INLINE_IMAGE_BYTES: u64 = 4 * 1024 * 1024;

/// Folder below the first write root for screenshots called without a path.
pub const SHOT_DIR: &str = "screenshots";

const INSTRUCTIONS: &str = "MurmelCraft is a viewer for KoflCraft and OpenFOAM meshes. \
This server runs one headless viewer session: start with mesh.open, then set the view \
(view.standard, view.orbit, view.set_camera), the display (display.*, clip.*) and the \
selection (select.*), and call view.screenshot to see the result as an image. \
mesh.info, inspect.item and inspect.quality report counts and properties. \
journal.undo and journal.redo walk the camera and display steps. Paths must lie below \
the server's read roots (meshes) or write roots (screenshots); relative paths start \
at the first write root. Settings tools change this session only.";

enum Phase {
    Initialize,
    InitializedNotification,
    Ready,
}

pub struct Server {
    session: Session,
    roots: Roots,
    phase: Phase,
    shots: u32,
}

pub fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The tool's description as an MCP client sees it.
fn description(t: &Tool) -> String {
    let mut d = t.summary.to_string();
    if t.id == "view.screenshot" {
        d.push_str(
            " Over MCP the picture also comes back as an image; path may be left out, \
             and the server then writes screenshots/shot-NNNN.png in its first write root.",
        );
    }
    if !t.reads.is_empty() || !t.outputs.is_empty() {
        d.push_str(" Paths: reads below the server's read roots, writes below its write roots.");
    }
    if t.id.starts_with("settings.set_") {
        d.push_str(" Over MCP the change applies to this session only and is not saved (SET001).");
    }
    d
}

/// The input schema as an MCP client sees it: the registry's, with `path`
/// optional for view.screenshot (the server names the file).
fn input_schema(t: &Tool) -> Value {
    let mut s = t.input_schema();
    if t.id == "view.screenshot"
        && let Some(r) = s.get_mut("required").and_then(Value::as_array_mut)
    {
        r.retain(|f| f != "path");
    }
    s
}

fn envelope_schema() -> Value {
    json!({
        "type": "object",
        "required": ["format", "format_version", "tool", "tool_version", "status",
                     "exit_code", "result", "diagnostics", "seconds"],
        "properties": {
            "format": {"const": murmel_tools::RESULT_FORMAT},
            "format_version": {"type": "string"},
            "tool": {"type": "string"},
            "tool_version": {"type": "integer"},
            "status": {"enum": ["finished", "failed"]},
            "exit_code": {"type": "integer"},
            "result": {},
            "diagnostics": {"type": "array", "items": {"type": "object"}},
            "seconds": {"type": "number"}
        }
    })
}

/// Every registry tool as an MCP tool: name = the dotted id.
pub fn listing() -> Vec<Value> {
    murmel_tools::tools()
        .iter()
        .map(|t| {
            let read_only = murmel_tools::reads_only(t.id);
            json!({
                "name": t.id,
                "title": murmel_tools::commands::english(t.label_key),
                "description": description(t),
                "inputSchema": input_schema(t),
                "outputSchema": envelope_schema(),
                "annotations": {
                    "readOnlyHint": read_only,
                    "destructiveHint": false,
                    "idempotentHint": read_only,
                    "openWorldHint": false
                },
                "_meta": {
                    "murmelcraft/version": t.version,
                    "murmelcraft/action": t.action,
                    "murmelcraft/reads": t.reads,
                    "murmelcraft/outputs": t.outputs,
                    "murmelcraft/journaled": murmel_tools::JOURNALED.contains(&t.id),
                }
            })
        })
        .collect()
}

impl Server {
    /// The session's config folder is dropped: the settings tools (which
    /// write files but name none) change the session only over MCP. Files
    /// named inside other files (a session's mesh) are kept to the roots.
    pub fn new(mut session: Session, roots: Roots) -> Self {
        session.config_dir = None;
        session.read_roots = Some(
            roots
                .read_roots()
                .iter()
                .chain(roots.write_roots())
                .cloned()
                .collect(),
        );
        Self {
            session,
            roots,
            phase: Phase::Initialize,
            shots: 0,
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// A free name for a screenshot without a path.
    fn next_shot(&mut self) -> Result<PathBuf, Diagnostic> {
        let dir = self.roots.home().join(SHOT_DIR);
        std::fs::create_dir_all(&dir)
            .map_err(|e| Diagnostic::new("MCP002", format!("{}: {e}", dir.display())))?;
        loop {
            self.shots += 1;
            let p = dir.join(format!("shot-{:04}.png", self.shots));
            if !p.exists() {
                return Ok(p);
            }
        }
    }

    /// Resolve and check the path fields of a call; the input then holds
    /// absolute real paths.
    fn confine(&mut self, t: &Tool, args: &mut Map<String, Value>) -> Result<(), Diagnostic> {
        if t.id == "view.screenshot" && !args.contains_key("path") {
            let p = self.next_shot()?;
            args.insert("path".into(), json!(p));
        }
        for (fields, write) in [(t.reads, false), (t.outputs, true)] {
            for f in fields {
                let Some(v) = args.get(*f) else { continue };
                let Some(p) = v.as_str() else {
                    // the tool's own parsing reports the wrong type
                    continue;
                };
                let p = Path::new(p);
                let real = if write {
                    self.roots.write(p)?
                } else {
                    self.roots.read(p)?
                };
                args.insert((*f).into(), json!(real));
            }
        }
        Ok(())
    }

    fn call(&mut self, t: &'static Tool, args: Value) -> (Envelope, Option<PathBuf>) {
        let mut args = match args {
            Value::Null => Map::new(),
            Value::Object(m) => m,
            _ => {
                let d = Diagnostic::new("TOOL002", "arguments must be an object");
                return (Envelope::failed(t.id, t.version, d), None);
            }
        };
        if let Err(d) = self.confine(t, &mut args) {
            return (Envelope::failed(t.id, t.version, d), None);
        }
        let image = (t.id == "view.screenshot")
            .then(|| args.get("path").and_then(Value::as_str).map(PathBuf::from))
            .flatten();
        let e = murmel_tools::call(t.id, Value::Object(args), &mut self.session);
        let image = image.filter(|_| e.ok());
        (e, image)
    }

    fn tools_call(&mut self, params: &Value) -> Result<Value, (i32, String)> {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Err((-32602, "tools/call requires a tool name".into()));
        };
        let Some(tool) = murmel_tools::find(name) else {
            return Err((-32602, format!("unknown tool {name}")));
        };
        let args = params.get("arguments").cloned().unwrap_or(Value::Null);
        let (envelope, image) = self.call(tool, args);
        let body = envelope.to_json();
        let mut content = vec![json!({"type": "text", "text": body.to_string()})];
        if let Some(p) = image {
            match std::fs::metadata(&p) {
                Ok(m) if m.len() <= MAX_INLINE_IMAGE_BYTES => match std::fs::read(&p) {
                    Ok(bytes) => content.push(json!({
                        "type": "image",
                        "mimeType": "image/png",
                        "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                    })),
                    Err(e) => content.push(json!({"type": "text",
                        "text": format!("the PNG was written but could not be read back: {e}")})),
                },
                Ok(m) => content.push(json!({"type": "text", "text": format!(
                    "the PNG is {} bytes, over the {MAX_INLINE_IMAGE_BYTES}-byte inline limit; it is only written to {}",
                    m.len(), p.display())})),
                Err(e) => content.push(json!({"type": "text",
                    "text": format!("the PNG could not be found after writing: {e}")})),
            }
        }
        Ok(json!({
            "content": content,
            "structuredContent": body,
            "isError": !envelope.ok(),
        }))
    }

    /// Handle one message; `None` for a notification.
    pub fn handle(&mut self, frame: &[u8]) -> Option<Value> {
        let request: Value = match serde_json::from_slice(frame) {
            Ok(v) => v,
            Err(_) => return Some(rpc_error(Value::Null, -32700, "invalid JSON")),
        };
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str);
        let valid_id = id
            .as_ref()
            .is_none_or(|id| id.is_string() || id.is_i64() || id.is_u64());
        let (Some(method), true, Some("2.0")) = (
            method,
            valid_id,
            request.get("jsonrpc").and_then(Value::as_str),
        ) else {
            // a response from the client (we send no requests) is ignored
            if request.get("method").is_none() && request.get("id").is_some() {
                return None;
            }
            return Some(rpc_error(Value::Null, -32600, "invalid JSON-RPC request"));
        };
        let Some(id) = id else {
            if method == "notifications/initialized"
                && matches!(self.phase, Phase::InitializedNotification)
            {
                self.phase = Phase::Ready;
            }
            return None;
        };
        let params = request.get("params").unwrap_or(&Value::Null);
        let result = match method {
            "initialize" if matches!(self.phase, Phase::Initialize) => {
                let Some(version) = params.get("protocolVersion").and_then(Value::as_str) else {
                    return Some(rpc_error(id, -32602, "initialize requires protocolVersion"));
                };
                let version = if OLDER_VERSIONS.contains(&version) {
                    version
                } else {
                    PROTOCOL_VERSION
                };
                self.phase = Phase::InitializedNotification;
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {
                        "name": "murmelcraft",
                        "title": "MurmelCraft",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "instructions": INSTRUCTIONS,
                })
            }
            "initialize" => return Some(rpc_error(id, -32600, "server is already initialized")),
            "ping" => json!({}),
            _ if !matches!(self.phase, Phase::Ready) => {
                return Some(rpc_error(id, -32002, "server is not initialized"));
            }
            "tools/list" => json!({"tools": listing()}),
            "tools/call" => match self.tools_call(params) {
                Ok(r) => r,
                Err((code, msg)) => return Some(rpc_error(id, code, &msg)),
            },
            _ => return Some(rpc_error(id, -32601, "method not found")),
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }
}
