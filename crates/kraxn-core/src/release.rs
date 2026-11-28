//! Releases: the versions of a product, the file of each one for this
//! system and the SHA-256 sums beside it, from GitHub or from a folder of
//! downloaded files (offline installs).
//!
//! Release files are named `<archive>-<version>-<system>.<ext>`, e.g.
//! `foehncraft-1.3.0-linux-x86_64.tar.gz`, with `SHA256SUMS.txt` beside
//! them (the Executables workflow of each repository).

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalogue::Product;
use crate::diag::{Res, diag};
use crate::platform::Platform;

/// The checksum file of a release.
pub const SUMS: &str = "SHA256SUMS.txt";

/// One file of a release: online by its URL, offline by its path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub product: String,
    /// Without the `v` of the tag.
    pub version: String,
    pub tag: String,
    pub prerelease: bool,
    pub files: Vec<ReleaseFile>,
}

impl Release {
    pub fn file(&self, name: &str) -> Option<&ReleaseFile> {
        self.files.iter().find(|f| f.name == name)
    }

    /// The archive of `product` for `platform`, if this release has one.
    pub fn archive_for(&self, product: &Product, platform: &Platform) -> Option<&ReleaseFile> {
        self.file(&file_name(product, &self.version, platform))
    }

    /// The checksum files: `SHA256SUMS.txt`, and offline also
    /// `SHA256SUMS-<anything>.txt`, so the sums of several products can lie
    /// in one folder.
    pub fn sums_files(&self) -> Vec<&ReleaseFile> {
        self.files
            .iter()
            .filter(|f| {
                f.name == SUMS || (f.name.starts_with("SHA256SUMS") && f.name.ends_with(".txt"))
            })
            .collect()
    }
}

/// The release file of `product` `version` for `platform`.
pub fn file_name(product: &Product, version: &str, platform: &Platform) -> String {
    format!(
        "{}-{}-{}.{}",
        product.archive,
        version,
        platform.asset_key(),
        platform.archive_ext()
    )
}

/// The folder at the top of that archive (the name without the extension).
pub fn top_folder(product: &Product, version: &str, platform: &Platform) -> String {
    format!("{}-{}-{}", product.archive, version, platform.asset_key())
}

/// Version order: numbers compare as numbers (1.10.0 after 1.9.0), a
/// version with a suffix (`1.0.0-rc.1`) comes before the same one without.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let split = |v: &str| -> (Vec<u64>, String) {
        let v = v.trim_start_matches('v');
        let (core, pre) = v.split_once('-').unwrap_or((v, ""));
        (
            core.split('.').map(|n| n.parse().unwrap_or(0)).collect(),
            pre.to_string(),
        )
    };
    let (an, ap) = split(a);
    let (bn, bp) = split(b);
    an.cmp(&bn)
        .then_with(|| match (ap.is_empty(), bp.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => ap.cmp(&bp),
        })
}

/// Newest first.
pub fn sort_newest_first(releases: &mut [Release]) {
    releases.sort_by(|a, b| compare_versions(&b.version, &a.version));
}

/// The release that `wanted` names (`latest` = the newest that is not a
/// pre-release and has a file for `platform`).
pub fn pick<'a>(
    releases: &'a [Release],
    product: &Product,
    platform: &Platform,
    wanted: &str,
) -> Res<&'a Release> {
    let wanted_v = wanted.trim_start_matches('v');
    let found = if wanted == "latest" {
        releases
            .iter()
            .filter(|r| !r.prerelease && r.archive_for(product, platform).is_some())
            .max_by(|a, b| compare_versions(&a.version, &b.version))
    } else {
        releases.iter().find(|r| r.version == wanted_v)
    };
    let r = found.ok_or_else(|| {
        let have: Vec<&str> = releases.iter().map(|r| r.version.as_str()).collect();
        diag(
            "KRX002",
            format!(
                "{} has no release {wanted} with a file for {} (found: {})",
                product.title,
                platform.label(),
                if have.is_empty() {
                    "none".to_string()
                } else {
                    have.join(", ")
                }
            ),
        )
    })?;
    if r.archive_for(product, platform).is_none() {
        return Err(diag(
            "KRX002",
            format!(
                "{} {} has no file for {} ({} is missing); build it from source or pick another version",
                product.title,
                r.version,
                platform.label(),
                file_name(product, &r.version, platform)
            ),
        ));
    }
    Ok(r)
}

/// The releases of `product` from the GitHub API's list
/// (`GET /repos/{owner}/{repo}/releases`); drafts are left out.
pub fn parse_github(product: &Product, json: &str) -> Res<Vec<Release>> {
    #[derive(Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
        #[serde(default)]
        size: u64,
    }
    #[derive(Deserialize)]
    struct Rel {
        tag_name: String,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
        #[serde(default)]
        assets: Vec<Asset>,
    }
    let list: Vec<Rel> = serde_json::from_str(json).map_err(|e| {
        diag(
            "KRX003",
            format!(
                "the list of {} releases is not what GitHub sends: {e}",
                product.title
            ),
        )
    })?;
    let mut out: Vec<Release> = list
        .into_iter()
        .filter(|r| !r.draft)
        .map(|r| Release {
            product: product.id.clone(),
            version: r.tag_name.trim_start_matches('v').to_string(),
            tag: r.tag_name,
            prerelease: r.prerelease,
            files: r
                .assets
                .into_iter()
                .map(|a| ReleaseFile {
                    name: a.name,
                    url: Some(a.browser_download_url),
                    path: None,
                    size: a.size,
                })
                .collect(),
        })
        .collect();
    sort_newest_first(&mut out);
    Ok(out)
}

/// The releases of `product` in a folder of downloaded files: its
/// archives for any system and the checksum files, in the folder itself
/// or one level below (one folder per product or per release).
pub fn scan_folder(product: &Product, dir: &Path) -> Res<Vec<Release>> {
    let mut files: Vec<PathBuf> = Vec::new();
    let read = |d: &Path| -> Res<Vec<PathBuf>> {
        let rd = std::fs::read_dir(d).map_err(|e| {
            diag(
                "KRX012",
                format!("cannot read the folder {}: {e}", d.display()),
            )
        })?;
        Ok(rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
    };
    for p in read(dir)? {
        if p.is_dir() {
            files.extend(read(&p)?.into_iter().filter(|f| f.is_file()));
        } else {
            files.push(p);
        }
    }
    let prefix = format!("{}-", product.archive);
    let mut out: Vec<Release> = Vec::new();
    for f in &files {
        let Some(name) = f.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(rest) = name.strip_prefix(&prefix) else {
            continue;
        };
        let Some(stem) = rest
            .strip_suffix(".tar.gz")
            .or_else(|| rest.strip_suffix(".zip"))
        else {
            continue;
        };
        // <version>-<os>-<arch>: the system part is the last two pieces
        let parts: Vec<&str> = stem.rsplitn(3, '-').collect();
        if parts.len() != 3 {
            continue;
        }
        let version = parts[2].to_string();
        // the sums beside this archive belong to its release
        let mut rel_files = vec![ReleaseFile {
            name: name.to_string(),
            url: None,
            path: Some(f.clone()),
            size: f.metadata().map(|m| m.len()).unwrap_or(0),
        }];
        if let Some(parent) = f.parent() {
            for s in files.iter().filter(|s| s.parent() == Some(parent)) {
                let sn = s.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if sn.starts_with("SHA256SUMS") && sn.ends_with(".txt") {
                    rel_files.push(ReleaseFile {
                        name: sn.to_string(),
                        url: None,
                        path: Some(s.clone()),
                        size: 0,
                    });
                }
            }
        }
        match out.iter_mut().find(|r| r.version == version) {
            Some(r) => {
                for rf in rel_files {
                    if r.file(&rf.name).is_none() {
                        r.files.push(rf);
                    }
                }
            }
            None => out.push(Release {
                product: product.id.clone(),
                tag: format!("v{version}"),
                version,
                prerelease: false,
                files: rel_files,
            }),
        }
    }
    if out.is_empty() {
        return Err(diag(
            "KRX012",
            format!(
                "{} holds no {} release archive ({}<version>-<system>.tar.gz or .zip)",
                dir.display(),
                product.title,
                prefix
            ),
        ));
    }
    sort_newest_first(&mut out);
    Ok(out)
}

/// The SHA-256 of `file` in a checksum file (`sha256sum` format: hash,
/// two spaces or a space and `*`, name).
pub fn sum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, name) = l.trim().split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        (name == file && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;

    fn foehn() -> Product {
        Catalogue::builtin()
            .unwrap()
            .get("foehncraft")
            .unwrap()
            .clone()
    }

    /// Invariant: version order is numeric and a pre-release comes before
    /// its release, so `latest` is 1.10.0 over 1.9.0 and 1.3.0 over
    /// 1.3.0-rc.1. Breaks with a string comparison; `pick` relies on it.
    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(compare_versions("1.10.0", "1.9.0"), Ordering::Greater);
        assert_eq!(compare_versions("v1.3.0", "1.3.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.3.0-rc.1", "1.3.0"), Ordering::Less);
        assert_eq!(compare_versions("0.7.0", "0.10.0"), Ordering::Less);
    }

    /// Invariant: from GitHub's list, drafts are dropped, `latest` is the
    /// newest release that is not a pre-release and has this system's
    /// file, and a version without the file says KRX002 with its name.
    /// Breaks if `latest` took a pre-release or a release without the
    /// file (the download would fail later with a worse message).
    #[test]
    fn latest_skips_drafts_prereleases_and_missing_files() {
        let p = foehn();
        let json = r#"[
          {"tag_name": "v1.4.0", "draft": true, "prerelease": false, "assets": []},
          {"tag_name": "v1.4.0-rc.1", "prerelease": true, "assets": [
            {"name": "foehncraft-1.4.0-rc.1-linux-x86_64.tar.gz", "browser_download_url": "u", "size": 1}]},
          {"tag_name": "v1.3.0", "prerelease": false, "assets": [
            {"name": "foehncraft-1.3.0-linux-x86_64.tar.gz", "browser_download_url": "https://x/a", "size": 9},
            {"name": "SHA256SUMS.txt", "browser_download_url": "https://x/s", "size": 1}]},
          {"tag_name": "v1.2.0", "prerelease": false, "assets": []}
        ]"#;
        let rels = parse_github(&p, json).unwrap();
        assert_eq!(rels.len(), 3);
        let linux = Platform::parse("linux", "x86_64").unwrap();
        assert_eq!(pick(&rels, &p, &linux, "latest").unwrap().version, "1.3.0");
        assert_eq!(
            pick(&rels, &p, &linux, "v1.4.0-rc.1").unwrap().version,
            "1.4.0-rc.1"
        );
        let e = pick(&rels, &p, &linux, "1.2.0").unwrap_err();
        assert_eq!(e.code, "KRX002");
        assert!(
            e.message.contains("foehncraft-1.2.0-linux-x86_64.tar.gz"),
            "{e}"
        );
        let arm = Platform::parse("linux", "aarch64").unwrap();
        assert_eq!(pick(&rels, &p, &arm, "latest").unwrap_err().code, "KRX002");
    }

    /// Invariant: a checksum line matches its file whether written with
    /// two spaces or ` *` (binary mode), and only the exact name matches.
    /// Breaks if `a.tar.gz` matched `a.tar.gz.sig`, or `*` names failed.
    #[test]
    fn sums_lines_match_exact_names() {
        let h = "a".repeat(64);
        let text = format!(
            "{h}  foehncraft-1.3.0-linux-x86_64.tar.gz\n{} *x.zip\n",
            "B".repeat(64)
        );
        assert_eq!(
            sum_for(&text, "foehncraft-1.3.0-linux-x86_64.tar.gz"),
            Some(h)
        );
        assert_eq!(sum_for(&text, "x.zip"), Some("b".repeat(64)));
        assert_eq!(sum_for(&text, "foehncraft-1.3.0-linux-x86_64"), None);
    }

    /// Invariant: a folder of downloads (flat, or one folder per
    /// product) yields its archives as releases with the sums beside
    /// them; files of other products and other names are ignored, and a
    /// folder without the product says KRX012.
    #[test]
    fn offline_folder_finds_archives_and_sums() {
        let d = std::env::temp_dir().join(format!("kraxn-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("foehn")).unwrap();
        std::fs::write(d.join("foehn/foehncraft-1.3.0-linux-x86_64.tar.gz"), b"x").unwrap();
        std::fs::write(d.join("foehn/foehncraft-1.3.0-windows-x86_64.zip"), b"x").unwrap();
        std::fs::write(d.join("foehn/SHA256SUMS.txt"), b"").unwrap();
        std::fs::write(d.join("koflcraft-0.3.1-linux-x86_64.tar.gz"), b"x").unwrap();
        std::fs::write(d.join("notes.txt"), b"x").unwrap();
        let rels = scan_folder(&foehn(), &d).unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].version, "1.3.0");
        assert_eq!(rels[0].files.len(), 3);
        assert_eq!(rels[0].sums_files().len(), 1);
        let mur = Catalogue::builtin()
            .unwrap()
            .get("murmelcraft")
            .unwrap()
            .clone();
        assert_eq!(scan_folder(&mur, &d).unwrap_err().code, "KRX012");
        let _ = std::fs::remove_dir_all(&d);
    }
}
