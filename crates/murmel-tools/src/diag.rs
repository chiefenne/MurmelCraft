//! The registry of diagnostic codes. A code names one meaning forever and
//! is never reused. Mesh and render codes come from their crates.

use std::sync::OnceLock;

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Code {
    pub code: &'static str,
    pub severity: Severity,
    pub meaning: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<&'static str>,
}

const OWN: &[Code] = &[
    Code {
        code: "TOOL001",
        severity: Severity::Error,
        meaning: "No tool has this id.",
        fix: Some("Run `murmel commands` for the ids."),
    },
    Code {
        code: "TOOL002",
        severity: Severity::Error,
        meaning: "The input does not match the tool's input schema.",
        fix: Some("Compare the input with the schema in `murmel commands`."),
    },
    Code {
        code: "VIEW001",
        severity: Severity::Error,
        meaning: "The tool needs an open mesh and none is open.",
        fix: Some("Call mesh.open first, or pass `mesh` to view.screenshot."),
    },
    Code {
        code: "VIEW002",
        severity: Severity::Error,
        meaning: "No item of the open mesh has this id.",
        fix: Some(
            "Use `mesh`, `zone:<name>`, `kind:<cell type>` or `patch:<name>`; mesh.info lists the names.",
        ),
    },
    Code {
        code: "VIEW003",
        severity: Severity::Error,
        meaning: "No saved view of the open mesh has this name.",
        fix: Some("views.list names the saved views."),
    },
    Code {
        code: "VIEW004",
        severity: Severity::Error,
        meaning: "The view name is empty, longer than 64 characters, holds control characters, or another saved view has it.",
        fix: Some("Choose another name, or delete the other view first."),
    },
    Code {
        code: "CLIP001",
        severity: Severity::Error,
        meaning: "No clip plane has this number.",
        fix: Some("Clip planes count from 1 (clip:1); the clips list of the result names them."),
    },
    Code {
        code: "CLIP002",
        severity: Severity::Error,
        meaning: "The display has the most clip planes it holds (8).",
        fix: Some("Remove a clip plane with clip.clear, or change one with clip.set."),
    },
    Code {
        code: "MCP001",
        severity: Severity::Error,
        meaning: "The MCP server refused a path outside its read or write roots.",
        fix: Some(
            "Use a path below a --read-root (files read) or a --write-root (files written), or start the server with more roots.",
        ),
    },
    Code {
        code: "MCP002",
        severity: Severity::Error,
        meaning: "The MCP server could not resolve a path (it does not exist, or its folder does not).",
        fix: Some("Check the path; relative paths resolve from the first --write-root."),
    },
    Code {
        code: "LIVE001",
        severity: Severity::Error,
        meaning: "No MurmelCraft window listens for agents (none is open, agents are switched off in its Settings, or the platform has no live bridge yet).",
        fix: Some(
            "Open MurmelCraft and switch on Settings > Agents > Let agents drive this window, or start the MCP server without --live for a headless session.",
        ),
    },
    Code {
        code: "LIVE002",
        severity: Severity::Error,
        meaning: "The window did not answer the call: the person stopped the agents, the window closed, or it took longer than two minutes.",
        fix: Some("Check the window; the next call connects again."),
    },
    Code {
        code: "LIVE003",
        severity: Severity::Error,
        meaning: "The window could not listen for agents: another window listens already, or the socket folder is not private.",
        fix: Some(
            "Close the other window, or make the socket folder private (chmod 700); MURMEL_LIVE_SOCKET names another socket.",
        ),
    },
    Code {
        code: "SET001",
        severity: Severity::Error,
        meaning: "A settings file could not be written, or no config folder is known.",
        fix: Some("Set MURMEL_CONFIG_DIR to a writable folder."),
    },
    Code {
        code: "SET002",
        severity: Severity::Error,
        meaning: "The navigation bindings bind one input to two actions.",
        fix: Some("Give each gesture, button and modifier combination one action."),
    },
    Code {
        code: "SESS001",
        severity: Severity::Error,
        meaning: "A session file could not be read, or it is not a MurmelCraft session (format murmelcraft.session 1.x) with valid values.",
        fix: Some("Check the path; save the session again with session.save."),
    },
    Code {
        code: "SESS002",
        severity: Severity::Error,
        meaning: "A session file could not be written.",
        fix: Some("Choose a writable folder."),
    },
    Code {
        code: "SESS003",
        severity: Severity::Warning,
        meaning: "The session's mesh changed since the session was saved (its SHA-256 differs).",
        fix: Some("Check the view; save the session again to accept the new mesh."),
    },
    Code {
        code: "SESS004",
        severity: Severity::Warning,
        meaning: "Items the session names are not in the mesh; they were skipped.",
        fix: Some("Save the session again to drop them."),
    },
];

/// Every code: the tools' own, then the mesh and render codes.
pub fn codes() -> &'static [Code] {
    static ALL: OnceLock<Vec<Code>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut v = OWN.to_vec();
        for (code, meaning, fix) in murmel_mesh::CODES.iter().chain(murmel_render::CODES) {
            let severity = if *code == "MESH011" {
                Severity::Warning
            } else {
                Severity::Error
            };
            v.push(Code {
                code,
                severity,
                meaning,
                fix: Some(fix),
            });
        }
        v
    })
}

pub fn code(id: &str) -> Option<&'static Code> {
    codes().iter().find(|c| c.code == id)
}
