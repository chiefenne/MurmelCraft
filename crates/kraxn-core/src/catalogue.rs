//! The catalogue: the products Kraxn installs (`assets/kraxn/catalogue.toml`).

use serde::{Deserialize, Serialize};

use crate::diag::{Res, diag};

/// The catalogue built into Kraxn.
pub const CATALOGUE: &str = include_str!("../../../assets/kraxn/catalogue.toml");

/// A kind of file the window opens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileType {
    /// Without the dot, e.g. `vtu`.
    pub ext: String,
    pub mime: String,
    pub name: String,
    /// The family's own format: the window becomes its default program.
    /// Other formats only offer the window under "Open with".
    #[serde(default)]
    pub own: bool,
}

/// One program of the family.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Product {
    pub id: String,
    /// The folder under the install root, e.g. `Foehncraft`.
    pub title: String,
    /// The start of the release file names, e.g. `foehncraft`.
    pub archive: String,
    /// `owner/name` on GitHub.
    pub repo: String,
    /// The programs in the release archive.
    pub programs: Vec<String>,
    /// The program with a window, if any.
    #[serde(default)]
    pub window: Option<String>,
    /// The menu entry's id (desktop file, macOS bundle identifier).
    #[serde(default)]
    pub app_id: String,
    /// The menu entry's line.
    #[serde(default)]
    pub summary: String,
    /// The files the window opens.
    #[serde(default)]
    pub file_types: Vec<FileType>,
    /// The smoke test after an install: program and arguments.
    pub check: Vec<String>,
    pub build_paths: Vec<String>,
    pub features: Vec<String>,
    pub min_rust: String,
    pub needs_cmake: bool,
    pub apt_runtime: Vec<String>,
    pub dnf_runtime: Vec<String>,
}

impl Product {
    /// The variable that names the install folder in env.sh and the module
    /// file, e.g. `FOEHNCRAFT_HOME` (install-shared.sh: `${TITLE^^}_HOME`).
    pub fn home_var(&self) -> String {
        format!("{}_HOME", self.title.to_uppercase())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalogue {
    pub format: String,
    pub format_version: String,
    #[serde(rename = "product")]
    pub products: Vec<Product>,
}

impl Catalogue {
    /// The catalogue built into Kraxn.
    pub fn builtin() -> Res<Self> {
        Self::parse(CATALOGUE)
    }

    pub fn parse(text: &str) -> Res<Self> {
        let c: Catalogue = toml::from_str(text)
            .map_err(|e| diag("KRX010", format!("the catalogue is not valid: {e}")))?;
        if c.format != "kraxn.catalogue" {
            return Err(diag(
                "KRX010",
                "the catalogue's format is not kraxn.catalogue",
            ));
        }
        Ok(c)
    }

    pub fn get(&self, id: &str) -> Res<&Product> {
        self.products.iter().find(|p| p.id == id).ok_or_else(|| {
            let ids: Vec<&str> = self.products.iter().map(|p| p.id.as_str()).collect();
            diag(
                "KRX001",
                format!("unknown product {id}; Kraxn installs {}", ids.join(", ")),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The value of `KEY=` in the settings block of install-shared.sh.
    fn setting(script: &str, key: &str) -> String {
        let line = script
            .lines()
            .find(|l| l.starts_with(&format!("{key}=")))
            .unwrap_or_default();
        let v = line.split_once('=').map(|x| x.1).unwrap_or("");
        let v = v.split(" #").next().unwrap_or("").trim();
        v.trim_matches('"').to_string()
    }

    /// Invariant: MurmelCraft's entry says what its own
    /// tools/install-shared.sh says (folder, file names, repository, check,
    /// build paths, run-time packages), so Kraxn and the script install
    /// the same way. Breaks when one of the two changes alone. The other
    /// two products' scripts live in their repositories; their entries
    /// were copied from them (2026-11-28).
    #[test]
    fn murmelcraft_entry_matches_install_shared() {
        let script = include_str!("../../../tools/install-shared.sh");
        let c = Catalogue::builtin().unwrap();
        let p = c.get("murmelcraft").unwrap();
        assert_eq!(setting(script, "NAME"), p.archive);
        assert_eq!(setting(script, "TITLE"), p.title);
        assert_eq!(setting(script, "REPO"), p.repo);
        assert_eq!(setting(script, "CHECK"), p.check.join(" "));
        assert_eq!(setting(script, "BUILD_PATHS"), p.build_paths.join(" "));
        assert_eq!(setting(script, "APT_RUNTIME"), p.apt_runtime.join(" "));
        assert_eq!(setting(script, "DNF_RUNTIME"), p.dnf_runtime.join(" "));
        assert_eq!(c.products.len(), 3);
        assert_eq!(c.get("solid").unwrap_err().code, "KRX001");
    }
}
