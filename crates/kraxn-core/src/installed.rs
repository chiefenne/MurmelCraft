//! What is installed: the version folders of each product, the default
//! one, and the receipt Kraxn leaves in each version folder.
//!
//! The folders are the record: a version is installed when its folder
//! holds the product's check program in `bin/`, whoever put it there
//! (Kraxn, or `tools/install-shared.sh` on Linux).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalogue::{Catalogue, Product};
use crate::diag::{Res, diag};
use crate::layout::Layout;
use crate::release::compare_versions;

/// The receipt's file in a version folder.
pub const RECEIPT: &str = "kraxn.toml";

/// What Kraxn installed into a version folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub format: String,
    pub format_version: String,
    pub product: String,
    pub version: String,
    /// `release` (downloaded), `folder` (offline) or `build` (from source).
    pub source: String,
    /// The release file, for `release` and `folder`.
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub sha256: String,
    /// Whether a SHA256SUMS.txt confirmed the file.
    #[serde(default)]
    pub verified: bool,
    /// The cargo features of a build.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// A build for this computer's processor only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native: bool,
    /// The compiler of a build (`rustc --version`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rustc: String,
    pub kraxn: String,
}

impl Receipt {
    pub fn new(product: &str, version: &str, source: &str) -> Self {
        Self {
            format: "kraxn.receipt".into(),
            format_version: "1.0".into(),
            product: product.into(),
            version: version.into(),
            source: source.into(),
            file: String::new(),
            sha256: String::new(),
            verified: false,
            features: Vec::new(),
            native: false,
            rustc: String::new(),
            kraxn: env!("CARGO_PKG_VERSION").into(),
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub version: String,
    pub dir: PathBuf,
    pub default: bool,
    /// `None` when another tool installed it (install-shared.sh, by hand).
    pub receipt: Option<Receipt>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledProduct {
    pub product: String,
    pub title: String,
    pub dir: PathBuf,
    /// Newest first.
    pub versions: Vec<Version>,
}

impl InstalledProduct {
    pub fn default_version(&self) -> Option<&Version> {
        self.versions.iter().find(|v| v.default)
    }

    pub fn get(&self, version: &str) -> Option<&Version> {
        self.versions.iter().find(|v| v.version == version)
    }
}

/// The default version of `p`: where `current` points (macOS, Linux) or
/// what `default.txt` says (Windows).
pub fn default_of(layout: &Layout, p: &Product) -> Option<String> {
    if layout.platform.unix() {
        let t = std::fs::read_link(layout.current_link(p)).ok()?;
        Some(t.file_name()?.to_string_lossy().to_string())
    } else {
        let t = std::fs::read_to_string(layout.default_file(p)).ok()?;
        Some(t.trim().to_string()).filter(|v| !v.is_empty())
    }
}

fn is_version_dir(layout: &Layout, p: &Product, dir: &Path) -> bool {
    let check = p.check.first().map(String::as_str).unwrap_or("");
    dir.join("bin").join(layout.platform.exe(check)).is_file() || dir.join(RECEIPT).is_file()
}

/// The installed versions of `p` in `layout`.
pub fn product(layout: &Layout, p: &Product) -> InstalledProduct {
    let dir = layout.product_dir(p);
    let default = default_of(layout, p);
    let mut versions: Vec<Version> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let d = e.path();
            if name == "current" || name == "modulefiles" || !is_version_dir(layout, p, &d) {
                return None;
            }
            let receipt = std::fs::read_to_string(d.join(RECEIPT))
                .ok()
                .and_then(|t| toml::from_str(&t).ok());
            Some(Version {
                default: default.as_deref() == Some(name.as_str()),
                version: name,
                dir: d,
                receipt,
            })
        })
        .collect();
    versions.sort_by(|a, b| compare_versions(&b.version, &a.version));
    InstalledProduct {
        product: p.id.clone(),
        title: p.title.clone(),
        dir,
        versions,
    }
}

/// Every product of the catalogue with at least one version in `layout`.
pub fn all(layout: &Layout, c: &Catalogue) -> Vec<InstalledProduct> {
    c.products
        .iter()
        .map(|p| product(layout, p))
        .filter(|i| !i.versions.is_empty())
        .collect()
}

/// The installed version `version` of `p`, or KRX009 listing those there are.
pub fn require(layout: &Layout, p: &Product, version: &str) -> Res<Version> {
    let i = product(layout, p);
    i.get(version.trim_start_matches('v'))
        .cloned()
        .ok_or_else(|| {
            let have: Vec<&str> = i.versions.iter().map(|v| v.version.as_str()).collect();
            diag(
                "KRX009",
                format!(
                    "{} {version} is not installed in {} (installed: {})",
                    p.title,
                    i.dir.display(),
                    if have.is_empty() {
                        "none".into()
                    } else {
                        have.join(", ")
                    }
                ),
            )
        })
}
