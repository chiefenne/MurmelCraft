//! Diagnostics: every failure Kraxn reports has a registered code and a
//! sentence that says what to do (AGENTS.md, "Correctness").

use serde::{Deserialize, Serialize};

/// One finding of a tool: a registered code and a message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Diagnostic {}

pub type Res<T> = Result<T, Diagnostic>;

/// A diagnostic with `code`, which must be listed in [`CODES`].
pub fn diag(code: &str, message: impl Into<String>) -> Diagnostic {
    debug_assert!(CODES.iter().any(|(c, _)| *c == code), "unregistered {code}");
    Diagnostic {
        code: code.into(),
        message: message.into(),
    }
}

/// Every code Kraxn reports, with what it means (`kraxn codes`).
pub const CODES: &[(&str, &str)] = &[
    ("TOOL001", "unknown tool"),
    ("TOOL002", "the input of a tool is not valid"),
    ("KRX001", "unknown product (not in the catalogue)"),
    (
        "KRX002",
        "the release has no file for this system, or the version does not exist",
    ),
    ("KRX003", "a download or the list of releases failed"),
    ("KRX004", "a downloaded file does not match its SHA-256 sum"),
    (
        "KRX005",
        "no SHA256SUMS.txt lists the file, so it cannot be checked",
    ),
    (
        "KRX006",
        "an archive cannot be read or holds an unsafe path",
    ),
    ("KRX007", "a file system step failed"),
    (
        "KRX008",
        "a step needs administrator rights and did not get them",
    ),
    ("KRX009", "the version is not installed"),
    ("KRX010", "the plan is not valid"),
    ("KRX011", "the program does not start after the install"),
    (
        "KRX012",
        "the offline folder holds no archive for the product",
    ),
    ("KRX013", "the version is installed already"),
    ("KRX014", "this system is not supported"),
];
