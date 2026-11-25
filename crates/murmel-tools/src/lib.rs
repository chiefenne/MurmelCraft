//! The tool registry of MurmelCraft (UI spec, "Viewer tools in the
//! registry"). Every control in the window calls one of these tools; the
//! CLI (`murmel call ID --input in.json`) and the MCP server
//! (`murmel mcp`, crate `murmel-mcp`) call the same ids. Every call ends in the same result envelope
//! (`murmelcraft.tool-result`), as in KoflCraft (AG-12) and FoehnCraft
//! (D196).
//!
//! Calls of the tools in `JOURNALED` are the steps of the session's
//! journal, which `journal.undo` and `journal.redo` walk.
//!
//! Tools change the session only; the kind `write-files` marks the tools
//! that also write files and names them (`outputs`), so an approval table
//! can judge a call before it runs; `reads` names the input fields that are
//! paths the call reads, so the MCP server can confine both.

pub mod bench;
mod blocks;
pub mod charts;
pub mod commands;
mod defaults;
mod derive;
mod diag;
pub mod export;
mod extract;
pub mod live;
mod markdown;
pub mod movie;
mod parcel;
pub mod probe;
mod region;
pub mod series;
mod stream;
mod tools;

use std::time::Instant;

use murmel_engine::{Session, Snapshot};
use serde::Serialize;
use serde_json::{Value, json};

pub use diag::{Code, Severity, code, codes};
pub use markdown::markdown;
pub use tools::{JOURNALED, MAX_CLIPS, NOT_JOURNALED, READS_ONLY, reads_only};

pub const RESULT_FORMAT: &str = "murmelcraft.tool-result";
pub const CATALOGUE_FORMAT: &str = "murmelcraft.tool-catalogue";
pub const FORMAT_VERSION: &str = "1.0";

/// What a tool does beyond the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Reads files and changes the session only.
    Read,
    /// Also writes files (named by `outputs` or the tool's summary).
    WriteFiles,
}

/// A failed call: a registered code and a message.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<&'static str>,
}

impl Diagnostic {
    pub fn new(code_id: &'static str, message: impl Into<String>) -> Self {
        let c = code(code_id);
        Self {
            code: code_id,
            severity: c.map_or(Severity::Error, |c| c.severity),
            message: message.into(),
            fix: c.and_then(|c| c.fix),
        }
    }
}

pub type Run = Result<Value, Diagnostic>;
type RunFn = fn(Value, &mut Session) -> Run;

/// One registered tool.
pub struct Tool {
    pub id: &'static str,
    pub version: u32,
    pub action: Action,
    /// Translation key of the tool's name in the window.
    pub label_key: &'static str,
    pub summary: &'static str,
    /// Input fields that name files the call writes.
    pub outputs: &'static [&'static str],
    /// Input fields that name files or folders the call reads.
    pub reads: &'static [&'static str],
    input_schema: fn() -> Value,
    run: RunFn,
}

impl Tool {
    pub fn input_schema(&self) -> Value {
        (self.input_schema)()
    }

    /// The files a call with `input` would write, before it runs.
    pub fn effects(&self, input: &Value) -> Vec<String> {
        self.outputs
            .iter()
            .filter_map(|f| input[*f].as_str().map(str::to_string))
            .collect()
    }
}

/// The result of every call.
#[derive(Clone, Debug, Serialize)]
pub struct Envelope {
    pub format: String,
    pub format_version: String,
    pub tool: String,
    pub tool_version: u32,
    /// `finished` or `failed`.
    pub status: String,
    /// 0 finished, 1 failed, 2 unknown tool or bad input.
    pub exit_code: i32,
    pub result: Value,
    pub diagnostics: Vec<Diagnostic>,
    pub seconds: f64,
}

impl Envelope {
    /// A failed call of `tool` (also a call refused before it ran).
    pub fn failed(tool: &str, version: u32, d: Diagnostic) -> Self {
        let exit_code = if d.code.starts_with("TOOL") { 2 } else { 1 };
        Self {
            format: RESULT_FORMAT.into(),
            format_version: FORMAT_VERSION.into(),
            tool: tool.into(),
            tool_version: version,
            status: "failed".into(),
            exit_code,
            result: Value::Null,
            diagnostics: vec![d],
            seconds: 0.0,
        }
    }

    pub fn ok(&self) -> bool {
        self.exit_code == 0
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

pub fn tools() -> &'static [Tool] {
    tools::TOOLS
}

pub fn find(id: &str) -> Option<&'static Tool> {
    tools::TOOLS.iter().find(|t| t.id == id)
}

/// Run tool `id` with `input` on `session`.
pub fn call(id: &str, input: Value, session: &mut Session) -> Envelope {
    let Some(tool) = find(id) else {
        return Envelope::failed(id, 0, Diagnostic::new("TOOL001", format!("no tool {id}")));
    };
    let start = Instant::now();
    // A clip, slice or iso preview (the window's clip and slice drags, a
    // new iso value) ends with the exact surface before any other tool
    // reads or draws it; moving a plane or slice again, or another iso
    // value, keeps it going.
    if !matches!(
        tool.id,
        "clip.set" | "clip.rotate" | "slice.set" | "slice.rotate" | "iso.set"
    ) {
        session.settle();
    }
    let input = if input.is_null() { json!({}) } else { input };
    // A call that changes the camera or the display is a step of the
    // journal (undo and redo).
    let before = tools::journaled(tool.id).then(|| Snapshot::of(session));
    let mut e = match (tool.run)(input, session) {
        Ok(result) => {
            if let Some(b) = before {
                let after = Snapshot::of(session);
                session.journal.record(tool.id, b, &after);
            }
            Envelope {
                format: RESULT_FORMAT.into(),
                format_version: FORMAT_VERSION.into(),
                tool: tool.id.into(),
                tool_version: tool.version,
                status: "finished".into(),
                exit_code: 0,
                result,
                diagnostics: Vec::new(),
                seconds: 0.0,
            }
        }
        Err(d) => Envelope::failed(tool.id, tool.version, d),
    };
    e.seconds = start.elapsed().as_secs_f64();
    e
}

/// `system.commands`: every tool with its schema.
pub fn catalogue() -> Value {
    json!({
        "format": CATALOGUE_FORMAT,
        "format_version": FORMAT_VERSION,
        "tools": tools::TOOLS.iter().map(|t| json!({
            "id": t.id,
            "version": t.version,
            "action": t.action,
            "label_key": t.label_key,
            "summary": t.summary,
            "outputs": t.outputs,
            "reads": t.reads,
            "input_schema": t.input_schema(),
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests;
