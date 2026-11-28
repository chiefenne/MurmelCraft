//! Downloads over HTTPS (the system's certificate store, the proxy of
//! `HTTPS_PROXY` / `ALL_PROXY`) and the SHA-256 check of every file.

use std::io::{Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::catalogue::Product;
use crate::diag::{Res, diag};
use crate::release::{Release, ReleaseFile, parse_github, scan_folder, sum_for};

/// Where releases come from.
#[derive(Clone, Debug)]
pub enum Source {
    /// GitHub releases. `api` is `https://api.github.com` (tests point it
    /// at a local server); `token`, from `GITHUB_TOKEN` or `GH_TOKEN`,
    /// lifts GitHub's limit of 60 requests an hour.
    Online { api: String, token: Option<String> },
    /// A folder of downloaded release files (no internet).
    Folder(std::path::PathBuf),
}

impl Source {
    pub fn github() -> Self {
        let token = ["GITHUB_TOKEN", "GH_TOKEN"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|t| !t.is_empty()));
        let api = std::env::var("KRAXN_GITHUB_API")
            .ok()
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| "https://api.github.com".into());
        Source::Online { api, token }
    }

    /// The releases of `product`, newest first.
    pub fn releases(&self, product: &Product) -> Res<Vec<Release>> {
        match self {
            Source::Folder(dir) => scan_folder(product, dir),
            Source::Online { api, token } => {
                let url = format!("{api}/repos/{}/releases?per_page=50", product.repo);
                let mut req = agent()
                    .get(&url)
                    .header("Accept", "application/vnd.github+json")
                    .header("User-Agent", USER_AGENT);
                if let Some(t) = token {
                    req = req.header("Authorization", &format!("Bearer {t}"));
                }
                let mut resp = req.call().map_err(|e| net_error(&url, e))?;
                let body = resp
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| net_error(&url, e))?;
                parse_github(product, &body)
            }
        }
    }
}

const USER_AGENT: &str = concat!("kraxn/", env!("CARGO_PKG_VERSION"));

fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .proxy(ureq::Proxy::try_from_env())
        .build()
        .into()
}

fn net_error(url: &str, e: impl std::fmt::Display) -> crate::diag::Diagnostic {
    let mut m = format!("{url}: {e}");
    if m.contains("403") || m.contains("429") {
        m.push_str(
            " (GitHub limits requests without a sign-in to 60 an hour: wait, or set GITHUB_TOKEN)",
        );
    } else {
        m.push_str(" (no internet? Install from a folder of downloaded files instead)");
    }
    diag("KRX003", m)
}

/// Copies or downloads `file` to `dest`, calling `progress(done, total)`
/// on the way.
pub fn fetch(file: &ReleaseFile, dest: &Path, progress: &mut dyn FnMut(u64, u64)) -> Res<()> {
    let io = |e: std::io::Error| diag("KRX007", format!("{}: {e}", dest.display()));
    if let Some(p) = &file.path {
        std::fs::copy(p, dest).map_err(|e| diag("KRX007", format!("copy {}: {e}", p.display())))?;
        let n = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
        progress(n, n);
        return Ok(());
    }
    let url = file.url.as_deref().ok_or_else(|| {
        diag(
            "KRX003",
            format!("{} has neither a URL nor a path", file.name),
        )
    })?;
    let mut resp = agent()
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| net_error(url, e))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(file.size);
    let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
    let mut out = std::fs::File::create(dest).map_err(io)?;
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| net_error(url, e))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(io)?;
        done += n as u64;
        progress(done, total);
    }
    out.flush().map_err(io)?;
    if total > 0 && done != total {
        return Err(diag(
            "KRX003",
            format!("{url}: the download stopped after {done} of {total} bytes"),
        ));
    }
    Ok(())
}

/// The SHA-256 of a file, as lower-case hex.
pub fn sha256_file(path: &Path) -> Res<String> {
    let mut f = std::fs::File::open(path)
        .map_err(|e| diag("KRX007", format!("{}: {e}", path.display())))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| diag("KRX007", format!("{}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// What the check of a download found.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Checked {
    pub sha256: String,
    /// `true` when a checksum file listed the file and the sums agree;
    /// `false` only when the plan allowed files without sums.
    pub verified: bool,
}

/// Checks `archive` (named `name`) against the release's checksum files,
/// which are fetched into `work`. Without a listing the file fails with
/// KRX005, unless `require` is false.
pub fn verify(
    release: &Release,
    name: &str,
    archive: &Path,
    work: &Path,
    require: bool,
) -> Res<Checked> {
    let have = sha256_file(archive)?;
    for (i, s) in release.sums_files().into_iter().enumerate() {
        let dest = work.join(format!("sums-{i}.txt"));
        fetch(s, &dest, &mut |_, _| {})?;
        let text = std::fs::read_to_string(&dest)
            .map_err(|e| diag("KRX007", format!("{}: {e}", dest.display())))?;
        if let Some(want) = sum_for(&text, name) {
            if want != have {
                return Err(diag(
                    "KRX004",
                    format!(
                        "{name} does not match {}: expected {want}, the file has {have}. Delete it and download again",
                        s.name
                    ),
                ));
            }
            return Ok(Checked {
                sha256: have,
                verified: true,
            });
        }
    }
    if require {
        return Err(diag(
            "KRX005",
            format!(
                "no SHA256SUMS.txt beside {name} lists it, so Kraxn cannot check it; put the release's SHA256SUMS.txt beside it"
            ),
        ));
    }
    Ok(Checked {
        sha256: have,
        verified: false,
    })
}
