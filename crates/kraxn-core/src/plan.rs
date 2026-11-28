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
//!
//! [[install]]
//! product = "foehncraft"
//! version = "latest"            # or 1.3.0
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
}

impl Want {
    pub fn new(product: &str) -> Self {
        Self {
            product: product.into(),
            version: latest(),
            make_default: true,
            replace: false,
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
            c.get(&w.product)?;
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
    /// default version, PATH, checksums required), survives a save and
    /// load unchanged, and an unknown field or product is refused with
    /// KRX010 / KRX001. Breaks if a default flips (an unattended install
    /// would do something else than the window showed) or a typo in a
    /// plan were ignored silently.
    #[test]
    fn plans_take_defaults_and_refuse_unknowns() {
        let c = Catalogue::builtin().unwrap();
        let p = Plan::parse("[[install]]\nproduct = \"koflcraft\"\n").unwrap();
        assert_eq!(p.scope, Scope::User);
        assert!(p.path && p.require_checksums);
        assert_eq!(p.installs[0], Want::new("koflcraft"));
        p.check(&c).unwrap();
        assert_eq!(Plan::parse(&p.to_toml()).unwrap(), p);
        let e = Plan::parse("scope = \"user\"\npaht = true\n").unwrap_err();
        assert_eq!(e.code, "KRX010");
        let q = Plan::parse("[[install]]\nproduct = \"solid\"\n").unwrap();
        assert_eq!(q.check(&c).unwrap_err().code, "KRX001");
        assert_eq!(Plan::default().check(&c).unwrap_err().code, "KRX010");
    }
}
