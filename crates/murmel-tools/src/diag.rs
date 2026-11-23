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
