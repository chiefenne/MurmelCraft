//! The system Kraxn runs on and the release files that fit it.

use serde::{Deserialize, Serialize};

use crate::diag::{Res, diag};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Macos,
    Linux,
    Windows,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    X86_64,
    Aarch64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
}

impl Platform {
    /// The system this program was built for.
    pub fn this() -> Res<Self> {
        Self::parse(std::env::consts::OS, std::env::consts::ARCH)
    }

    /// From Rust's names (`std::env::consts::OS` and `ARCH`).
    pub fn parse(os: &str, arch: &str) -> Res<Self> {
        let os = match os {
            "macos" => Os::Macos,
            "linux" => Os::Linux,
            "windows" => Os::Windows,
            o => return Err(diag("KRX014", format!("Kraxn does not support {o}"))),
        };
        let arch = match arch {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            a => return Err(diag("KRX014", format!("Kraxn does not support {a}"))),
        };
        Ok(Self { os, arch })
    }

    /// The part of a release file name that names the system. The Mac file
    /// holds both architectures; Windows on Arm runs the x86-64 programs
    /// (Windows' own emulation).
    pub fn asset_key(&self) -> &'static str {
        match (self.os, self.arch) {
            (Os::Macos, _) => "macos-universal",
            (Os::Linux, Arch::X86_64) => "linux-x86_64",
            (Os::Linux, Arch::Aarch64) => "linux-aarch64",
            (Os::Windows, _) => "windows-x86_64",
        }
    }

    pub fn archive_ext(&self) -> &'static str {
        match self.os {
            Os::Windows => "zip",
            _ => "tar.gz",
        }
    }

    /// A program's file name on this system.
    pub fn exe(&self, name: &str) -> String {
        match self.os {
            Os::Windows => format!("{name}.exe"),
            _ => name.to_string(),
        }
    }

    pub fn unix(&self) -> bool {
        self.os != Os::Windows
    }

    /// A short name for people, e.g. `Linux (aarch64)`.
    pub fn label(&self) -> String {
        let os = match self.os {
            Os::Macos => "macOS",
            Os::Linux => "Linux",
            Os::Windows => "Windows",
        };
        let arch = match self.arch {
            Arch::X86_64 => "x86-64",
            Arch::Aarch64 => "Arm64",
        };
        format!("{os} ({arch})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: each system maps to the file name part the Executables
    /// workflows write (macos-universal, linux-x86_64, linux-aarch64,
    /// windows-x86_64). Breaks if a key is renamed on one side only; the
    /// release tests use these keys but would not notice a wrong mapping.
    #[test]
    fn asset_keys_match_the_release_files() {
        let k = |o, a| Platform::parse(o, a).map(|p| p.asset_key());
        assert_eq!(k("macos", "aarch64"), Ok("macos-universal"));
        assert_eq!(k("macos", "x86_64"), Ok("macos-universal"));
        assert_eq!(k("linux", "x86_64"), Ok("linux-x86_64"));
        assert_eq!(k("linux", "aarch64"), Ok("linux-aarch64"));
        assert_eq!(k("windows", "aarch64"), Ok("windows-x86_64"));
        assert_eq!(
            k("freebsd", "x86_64").map_err(|d| d.code),
            Err("KRX014".into())
        );
    }
}
