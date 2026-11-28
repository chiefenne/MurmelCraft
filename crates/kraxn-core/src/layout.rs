//! Where things go: the install root, the product and version folders and
//! the folder of program links on the PATH, for each system and scope.
//!
//! ```text
//! <root>/<Title>/<version>/bin/    the programs of one version
//! <root>/<Title>/<version>/kraxn.toml   what Kraxn installed there
//! <root>/<Title>/current -> <version>   the default (macOS and Linux)
//! <root>/<Title>/default.txt            the default (Windows)
//! <bin>/<program>                       links to the default's programs
//! ```
//!
//! On Linux with the system scope this is the layout of
//! `tools/install-shared.sh` (`/opt/<Title>/<version>`, env.sh, env.csh,
//! module files, use.sh), so either one sees the other's versions.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalogue::Product;
use crate::diag::{Res, diag};
use crate::platform::{Os, Platform};

/// For you alone, or for every user of the computer (needs administrator
/// rights for the copy into the system folders).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[default]
    User,
    System,
}

/// The parts of the environment that decide the default folders, taken
/// from the process or set by tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    pub home: Option<PathBuf>,
    /// `%LOCALAPPDATA%` on Windows.
    pub local_app_data: Option<PathBuf>,
    /// `%ProgramFiles%` on Windows.
    pub program_files: Option<PathBuf>,
    /// `PATH`, split.
    pub path: Vec<PathBuf>,
    /// Where tools often are besides the PATH (Homebrew, CMake on
    /// Windows): a program started from the Dock gets a short PATH.
    pub tool_dirs: Vec<PathBuf>,
}

impl Env {
    pub fn from_process() -> Self {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            home: var("HOME").or_else(|| var("USERPROFILE")),
            local_app_data: var("LOCALAPPDATA"),
            program_files: var("ProgramFiles"),
            path: std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect())
                .unwrap_or_default(),
            tool_dirs: default_tool_dirs(),
        }
    }

    pub fn on_path(&self, dir: &Path) -> bool {
        self.path.iter().any(|p| p == dir)
    }
}

fn default_tool_dirs() -> Vec<PathBuf> {
    let fixed: &[&str] = if cfg!(target_os = "macos") {
        &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
    } else if cfg!(windows) {
        &[]
    } else {
        &[
            "/usr/local/bin",
            "/usr/bin",
            "/usr/lib64/openmpi/bin",
            "/usr/lib/x86_64-linux-gnu/openmpi/bin",
            "/usr/lib/aarch64-linux-gnu/openmpi/bin",
        ]
    };
    let mut v: Vec<PathBuf> = fixed.iter().map(PathBuf::from).collect();
    if let Some(pf) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
        v.push(pf.join("CMake").join("bin"));
        v.push(pf.join("Git").join("cmd"));
    }
    if let Some(l) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        v.push(l.join("Microsoft").join("WindowsApps"));
    }
    v
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub scope: Scope,
    pub platform: Platform,
    /// The folder that holds one folder per product.
    pub root: PathBuf,
    /// The folder of program links that goes on the PATH.
    pub bin: PathBuf,
}

impl Layout {
    /// The default folders of `scope`, or below `prefix` if given
    /// (`<prefix>/<Title>/...` and `<prefix>/bin`).
    pub fn new(scope: Scope, platform: Platform, env: &Env, prefix: Option<&Path>) -> Res<Self> {
        if let Some(p) = prefix {
            if !p.is_absolute() {
                return Err(diag(
                    "KRX010",
                    format!("the install folder {} must be a full path", p.display()),
                ));
            }
            return Ok(Self {
                scope,
                platform,
                root: p.to_path_buf(),
                bin: p.join("bin"),
            });
        }
        let need = |v: &Option<PathBuf>, what: &str| {
            v.clone().ok_or_else(|| {
                diag(
                    "KRX010",
                    format!("{what} is not set, so Kraxn cannot find the default install folder; give one"),
                )
            })
        };
        let (root, bin) = match (platform.os, scope) {
            (Os::Windows, Scope::User) => {
                let r = need(&env.local_app_data, "LOCALAPPDATA")?
                    .join("Programs")
                    .join("Kraxn");
                (r.clone(), r.join("bin"))
            }
            (Os::Windows, Scope::System) => {
                let r = need(&env.program_files, "ProgramFiles")?.join("Kraxn");
                (r.clone(), r.join("bin"))
            }
            (_, Scope::User) => {
                let h = need(&env.home, "HOME")?;
                (h.join(".local").join("opt"), h.join(".local").join("bin"))
            }
            (_, Scope::System) => (PathBuf::from("/opt"), PathBuf::from("/usr/local/bin")),
        };
        Ok(Self {
            scope,
            platform,
            root,
            bin,
        })
    }

    pub fn product_dir(&self, p: &Product) -> PathBuf {
        self.root.join(&p.title)
    }

    pub fn version_dir(&self, p: &Product, version: &str) -> PathBuf {
        self.product_dir(p).join(version)
    }

    /// `current`, the link to the default version (macOS and Linux).
    pub fn current_link(&self, p: &Product) -> PathBuf {
        self.product_dir(p).join("current")
    }

    /// `default.txt`, the default version on Windows (no links there
    /// without administrator rights or developer mode).
    pub fn default_file(&self, p: &Product) -> PathBuf {
        self.product_dir(p).join("default.txt")
    }

    /// The module files (Environment Modules, Lmod) of the product.
    pub fn modulefiles(&self, p: &Product) -> PathBuf {
        self.product_dir(p).join("modulefiles").join(&p.archive)
    }

    /// The program link on the PATH.
    pub fn bin_link(&self, program: &str) -> PathBuf {
        self.bin.join(self.platform.exe(program))
    }

    /// Where a link in `bin` points: through `current` on macOS and Linux
    /// (a new default needs no new links), straight to the version's file
    /// on Windows (hard links).
    pub fn bin_target(&self, p: &Product, version: &str, program: &str) -> PathBuf {
        let exe = self.platform.exe(program);
        if self.platform.unix() {
            self.current_link(p).join("bin").join(exe)
        } else {
            self.version_dir(p, version).join("bin").join(exe)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;

    fn env() -> Env {
        Env {
            home: Some("/home/ann".into()),
            local_app_data: Some(r"C:\Users\ann\AppData\Local".into()),
            program_files: Some(r"C:\Program Files".into()),
            path: vec![],
            tool_dirs: vec![],
        }
    }

    /// Invariant: the system scope on Linux puts a version where
    /// install-shared.sh puts it (/opt/<Title>/<version>), and the user
    /// scope under ~/.local/opt with links in ~/.local/bin. Breaks if the
    /// folders move, which would hide installs of the script from Kraxn
    /// and the other way round; no other test pins the folders.
    #[test]
    fn folders_match_install_shared() {
        let c = Catalogue::builtin().unwrap();
        let foehn = c.get("foehncraft").unwrap();
        let linux = Platform::parse("linux", "x86_64").unwrap();
        let sys = Layout::new(Scope::System, linux, &env(), None).unwrap();
        assert_eq!(
            sys.version_dir(foehn, "1.3.0"),
            Path::new("/opt/Foehncraft/1.3.0")
        );
        assert_eq!(
            sys.current_link(foehn),
            Path::new("/opt/Foehncraft/current")
        );
        assert_eq!(
            sys.modulefiles(foehn),
            Path::new("/opt/Foehncraft/modulefiles/foehncraft")
        );
        let user = Layout::new(Scope::User, linux, &env(), None).unwrap();
        assert_eq!(user.root, Path::new("/home/ann/.local/opt"));
        assert_eq!(
            user.bin_link("foehn"),
            Path::new("/home/ann/.local/bin/foehn")
        );
        assert_eq!(
            user.bin_target(foehn, "1.3.0", "foehn"),
            Path::new("/home/ann/.local/opt/Foehncraft/current/bin/foehn")
        );
        let win = Platform::parse("windows", "x86_64").unwrap();
        let w = Layout::new(Scope::User, win, &env(), None).unwrap();
        assert_eq!(
            w.bin_target(foehn, "1.3.0", "foehn"),
            Path::new(r"C:\Users\ann\AppData\Local")
                .join("Programs/Kraxn/Foehncraft/1.3.0/bin/foehn.exe")
        );
        let rel = Layout::new(Scope::User, linux, &env(), Some(Path::new("opt")));
        assert_eq!(rel.map_err(|d| d.code), Err("KRX010".into()));
    }
}
