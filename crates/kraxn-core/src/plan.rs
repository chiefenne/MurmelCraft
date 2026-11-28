//! A plan: everything one install decides, as a TOML file. The window and
//! the text mode fill one in, show its steps and save it on request;
//! `kraxn apply plan.toml` repeats it on other computers.
//!
//! ```toml
//! format = "kraxn.plan"
//! format_version = "1.0"
//! scope = "system"              # user (default) or system
//! # prefix = "/opt"             # another install root
//! # offline_dir = "/srv/kraxn"  # install from downloaded files
//! path = true                   # put the programs on the PATH
//! menu = true                   # a menu entry for MurmelCraft's window
//! file_types = true             # .murmel, .vtu and .vtkhdf open in it
//! python = "venv"               # FoehnCraft's Python: venv, system or none
//! ffmpeg = true                 # ffmpeg for MurmelCraft's MP4 movies
//! examples = true               # KoflCraft's and FoehnCraft's examples
//! assistants = ["claude-code"]  # register the MCP servers with these
//! # assistant_dir = "/home/me/cfd"  # the folder they may use (default:
//! #                                 # ~/murmel-work, ~/kofl-work, ~/foehn-work)
//!
//! [[install]]
//! product = "foehncraft"
//! version = "latest"            # or 1.3.0
//! # from = "source"             # build with cargo instead of the release
//! # features = ["mpi"]          # cargo features of a source build
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::catalogue::Catalogue;
use crate::diag::{Res, diag};
use crate::layout::Scope;

fn yes() -> bool {
    true
}

fn latest() -> String {
    "latest".into()
}

fn format() -> String {
    "kraxn.plan".into()
}

fn format_version() -> String {
    "1.0".into()
}

/// Python for the scripts of a product that has some (FoehnCraft).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PyMode {
    /// `<version>/venv` with pip's packages (Gmsh, NumPy, …).
    #[default]
    Venv,
    /// The distribution's packages (apt, dnf), no pip.
    System,
    /// No Python.
    None,
}

/// Where a product's programs come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// The release archive (GitHub or the offline folder).
    #[default]
    Release,
    /// The release's source, built here with cargo.
    Source,
}

impl Origin {
    fn is_release(&self) -> bool {
        *self == Origin::Release
    }
}

/// One product to install.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Want {
    pub product: String,
    /// `latest` or a version such as `1.3.0`.
    #[serde(default = "latest")]
    pub version: String,
    /// Make it the default version (`current`).
    #[serde(default = "yes")]
    pub make_default: bool,
    /// Install again over the same version.
    #[serde(default)]
    pub replace: bool,
    /// The release archive (default) or a build from the source.
    #[serde(default, skip_serializing_if = "Origin::is_release")]
    pub from: Origin,
    /// Cargo features of a source build (the catalogue's `features`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// A source build for this computer's processor only
    /// (`-C target-cpu=native`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native: bool,
}

impl Want {
    pub fn new(product: &str) -> Self {
        Self {
            product: product.into(),
            version: latest(),
            make_default: true,
            replace: false,
            from: Origin::Release,
            features: Vec::new(),
            native: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    #[serde(default = "format")]
    pub format: String,
    #[serde(default = "format_version")]
    pub format_version: String,
    #[serde(default)]
    pub scope: Scope,
    /// Another install root than the default of the scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<PathBuf>,
    /// Install from this folder of downloaded release files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline_dir: Option<PathBuf>,
    /// Refuse files that no SHA256SUMS.txt lists (default: yes).
    #[serde(default = "yes")]
    pub require_checksums: bool,
    /// Put the programs on the PATH (default: yes).
    #[serde(default = "yes")]
    pub path: bool,
    /// A menu entry for a program with a window (default: yes).
    #[serde(default = "yes")]
    pub menu: bool,
    /// With the menu entry: the file types it opens (default: yes).
    #[serde(default = "yes")]
    pub file_types: bool,
    /// Python for the scripts (default: a venv).
    #[serde(default)]
    pub python: PyMode,
    /// ffmpeg for MP4 movies, when it is missing (default: yes).
    #[serde(default = "yes")]
    pub ffmpeg: bool,
    /// The examples from the release's source (default: yes).
    #[serde(default = "yes")]
    pub examples: bool,
    /// The AI assistants that get the programs' MCP servers (default:
    /// none): claude-code, claude-desktop, codex, gemini.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assistants: Vec<crate::assist::HostId>,
    /// The folder the assistants may read and write (default: one per
    /// program in the home folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_dir: Option<PathBuf>,
    #[serde(default, rename = "install")]
    pub installs: Vec<Want>,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            format: format(),
            format_version: format_version(),
            scope: Scope::User,
            prefix: None,
            offline_dir: None,
            require_checksums: true,
            path: true,
            menu: true,
            file_types: true,
            python: PyMode::Venv,
            ffmpeg: true,
            examples: true,
            assistants: Vec::new(),
            assistant_dir: None,
            installs: Vec::new(),
        }
    }
}

impl Plan {
    pub fn parse(text: &str) -> Res<Self> {
        toml::from_str(text).map_err(|e| diag("KRX010", format!("the plan is not valid: {e}")))
    }

    pub fn to_toml(&self) -> String {
        let mut s = String::from(
            "# Kraxn plan: `kraxn apply THIS-FILE` installs the same on another computer.\n",
        );
        s.push_str(&toml::to_string(self).unwrap_or_default());
        s
    }

    /// Problems that stop the plan before anything runs.
    pub fn check(&self, c: &Catalogue) -> Res<()> {
        if self.format != "kraxn.plan" {
            return Err(diag("KRX010", "the plan's format is not kraxn.plan"));
        }
        if self.format_version.split('.').next() != Some("1") {
            return Err(diag(
                "KRX010",
                format!(
                    "the plan has format_version {}, this Kraxn reads 1.x; update Kraxn",
                    self.format_version
                ),
            ));
        }
        if self.installs.is_empty() {
            return Err(diag(
                "KRX010",
                "the plan installs nothing; add an [[install]]",
            ));
        }
        for (i, w) in self.installs.iter().enumerate() {
            let p = c.get(&w.product)?;
            if w.from == Origin::Release && (!w.features.is_empty() || w.native) {
                return Err(diag(
                    "KRX010",
                    format!(
                        "{}: features and native need from = \"source\" (the release is built already)",
                        w.product
                    ),
                ));
            }
            if let Some(f) = w.features.iter().find(|f| !p.features.contains(f)) {
                return Err(diag(
                    "KRX010",
                    format!(
                        "{} has no feature {f} (it has: {})",
                        w.product,
                        if p.features.is_empty() {
                            "none".to_string()
                        } else {
                            p.features.join(", ")
                        }
                    ),
                ));
            }
            if w.from == Origin::Source && self.offline_dir.is_some() {
                return Err(diag(
                    "KRX010",
                    format!(
                        "{}: a source build needs the internet (cargo downloads the crates); install the release, or leave out offline_dir",
                        w.product
                    ),
                ));
            }
            if self.installs[..i].iter().any(|o| o.product == w.product) {
                return Err(diag(
                    "KRX010",
                    format!("the plan names {} twice", w.product),
                ));
            }
        }
        if let Some(p) = &self.prefix
            && !p.is_absolute()
        {
            return Err(diag(
                "KRX010",
                format!("prefix {} must be a full path", p.display()),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: a short plan takes the defaults (user scope, latest,
    /// default version, PATH, checksums required, a Python venv, ffmpeg,
    /// examples), survives a save and
    /// load unchanged, and an unknown field or product is refused with
    /// KRX010 / KRX001. Breaks if a default flips (an unattended install
    /// would do something else than the window showed) or a typo in a
    /// plan were ignored silently.
    #[test]
    fn plans_take_defaults_and_refuse_unknowns() {
        let c = Catalogue::builtin().unwrap();
        let p = Plan::parse("[[install]]\nproduct = \"koflcraft\"\n").unwrap();
        assert_eq!(p.scope, Scope::User);
        assert!(p.path && p.require_checksums && p.ffmpeg && p.examples);
        assert_eq!(p.python, PyMode::Venv);
        assert_eq!(p.installs[0], Want::new("koflcraft"));
        p.check(&c).unwrap();
        assert_eq!(Plan::parse(&p.to_toml()).unwrap(), p);
        let e = Plan::parse("scope = \"user\"\npaht = true\n").unwrap_err();
        assert_eq!(e.code, "KRX010");
        let q = Plan::parse("[[install]]\nproduct = \"solid\"\n").unwrap();
        assert_eq!(q.check(&c).unwrap_err().code, "KRX001");
        assert_eq!(Plan::default().check(&c).unwrap_err().code, "KRX010");
    }

    /// Invariant: a source build round-trips with its features, a feature
    /// the product lacks, features on a release and a source build from
    /// the offline folder are refused. Breaks if a plan could ask for a
    /// build cargo would refuse later, after the tools were installed.
    #[test]
    fn source_builds_are_checked() {
        let c = Catalogue::builtin().unwrap();
        let p = Plan::parse(
            "[[install]]\nproduct = \"foehncraft\"\nfrom = \"source\"\nfeatures = [\"mpi\"]\nnative = true\n",
        )
        .unwrap();
        p.check(&c).unwrap();
        assert_eq!(p.installs[0].from, Origin::Source);
        assert_eq!(Plan::parse(&p.to_toml()).unwrap(), p);
        let bad = |t: &str| Plan::parse(t).unwrap().check(&c).unwrap_err().code;
        assert_eq!(
            bad("[[install]]\nproduct = \"koflcraft\"\nfrom = \"source\"\nfeatures = [\"mpi\"]\n"),
            "KRX010"
        );
        assert_eq!(
            bad("[[install]]\nproduct = \"foehncraft\"\nfeatures = [\"mpi\"]\n"),
            "KRX010"
        );
        assert_eq!(
            bad(
                "offline_dir = \"/srv\"\n[[install]]\nproduct = \"koflcraft\"\nfrom = \"source\"\n"
            ),
            "KRX010"
        );
    }
}
