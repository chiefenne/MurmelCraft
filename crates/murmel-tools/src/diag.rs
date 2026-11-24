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
        code: "SLICE001",
        severity: Severity::Error,
        meaning: "No slice has this number.",
        fix: Some("Slices count from 1 (slice:1); the slices list of the result names them."),
    },
    Code {
        code: "SLICE002",
        severity: Severity::Error,
        meaning: "The display has the most slices it holds (8; a series counts once).",
        fix: Some(
            "Remove a slice with slice.clear, change one with slice.set, or make a series (count).",
        ),
    },
    Code {
        code: "SLICE003",
        severity: Severity::Error,
        meaning: "A series has a count outside 1 to 64 or a spacing that is not positive.",
        fix: Some(
            "Give count 1 to 64, and a positive spacing or none (the planes then spread through the mesh).",
        ),
    },
    Code {
        code: "ISO001",
        severity: Severity::Error,
        meaning: "No iso-surface has this number.",
        fix: Some("Iso-surfaces count from 1 (iso:1); the isos list of the result names them."),
    },
    Code {
        code: "ISO002",
        severity: Severity::Error,
        meaning: "The display has the most iso-surfaces it holds (8).",
        fix: Some("Remove one with iso.clear, or change one with iso.set."),
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
        code: "EXPORT001",
        severity: Severity::Error,
        meaning: "An export setting is out of range (scale, line width, dpi).",
        fix: Some(
            "Use a scale of 0.25 to 8, a line width of 1 to 32 pixels and a dpi of 1 to 10000; a movie 1 to 60 frames a second and at most 36000 frames, an MP4 without transparency.",
        ),
    },
    Code {
        code: "EXPORT002",
        severity: Severity::Error,
        meaning: "An MP4 needs ffmpeg, and none was found (MURMEL_FFMPEG, PATH, /opt/homebrew/bin, /usr/local/bin).",
        fix: Some(
            "Install ffmpeg (macOS: brew install ffmpeg), set MURMEL_FFMPEG to its path, or write a GIF or PNG frames.",
        ),
    },
    Code {
        code: "EXPORT003",
        severity: Severity::Error,
        meaning: "ffmpeg could not be started or failed while writing the MP4; the message ends with what it said.",
        fix: Some("Check the path and the free space; try another ffmpeg with MURMEL_FFMPEG."),
    },
    Code {
        code: "EXPORT004",
        severity: Severity::Error,
        meaning: "A movie file or a frame could not be written.",
        fix: Some("Check that the folder exists and is writable, and the free space."),
    },
    Code {
        code: "EXPORT005",
        severity: Severity::Error,
        meaning: "The movie has nothing to move: no clip plane to sweep, fewer than two saved views, or a view name that is not saved.",
        fix: Some("Add a clip plane (clip.set) or save views (views.save); views.list names them."),
    },
    Code {
        code: "EXPORT006",
        severity: Severity::Error,
        meaning: "The movie was cancelled; a half-written MP4 or GIF is removed, PNG frames written stay.",
        fix: None,
    },
    Code {
        code: "EXPORT007",
        severity: Severity::Error,
        meaning: "The window is making a movie already; it makes one at a time.",
        fix: Some("Wait for the movie, or cancel it in the Export movie window."),
    },
    Code {
        code: "MEAS001",
        severity: Severity::Error,
        meaning: "Nothing of the mesh lies under the screen point, so there is no point to measure from.",
        fix: Some(
            "Point at a face of the mesh, or give typed coordinates (point), a node, a face or a cell.",
        ),
    },
    Code {
        code: "MEAS002",
        severity: Severity::Error,
        meaning: "The open mesh has no node, face or cell with this number.",
        fix: Some("mesh.info gives the counts; numbers start at 0."),
    },
    Code {
        code: "MEAS003",
        severity: Severity::Error,
        meaning: "The name of a measurement or probe is empty, longer than 64 characters or holds control characters.",
        fix: None,
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
    Code {
        code: "FIELD001",
        severity: Severity::Error,
        meaning: "The open mesh has no fields: it was read without results.",
        fix: Some("Open a result: FoehnCraft's fields.vtkhdf, or a .vtu with cell or point data."),
    },
    Code {
        code: "FIELD002",
        severity: Severity::Error,
        meaning: "The open mesh has no field of this name, or no field is chosen.",
        fix: Some("field.list names the fields; pass `field`."),
    },
    Code {
        code: "FIELD003",
        severity: Severity::Error,
        meaning: "A component (x, y, z) was asked of a scalar field.",
        fix: Some("Leave `component` out, or give `magnitude`, for a scalar."),
    },
    Code {
        code: "FIELD004",
        severity: Severity::Error,
        meaning: "A field style value is out of range: bands outside 2 to 64, contour lines outside 1 to 64, an unknown colour map, or a fixed range whose min is not below its max.",
        fix: Some("field.list names the colour maps; give finite min < max."),
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
