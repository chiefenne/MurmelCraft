//! The user's settings files: `settings.toml` (theme, mesh colours, view
//! cube placement, whether agents may drive the window, the defaults new
//! items start from, your formulas, the memory budget, your keyboard
//! shortcuts) and
//! `navigation.toml` (mouse bindings), in the MurmelCraft config folder.
//! `MURMEL_CONFIG_DIR` overrides the folder (tests, portable installs).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::calc::Formula;
use crate::cube::CubeSettings;
use crate::defaults::Defaults;
use crate::keys::KeyMap;
use crate::memory::Memory;
use crate::navigation::Navigation;
use crate::theme::{MeshColors, Theme};

pub const SETTINGS_FORMAT: &str = "murmelcraft.settings";

/// The platform's config folder for MurmelCraft, if one can be found.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("MURMEL_CONFIG_DIR") {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Application Support/MurmelCraft"));
    }
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("MurmelCraft"));
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".config")))
        .map(|c| c.join("murmelcraft"))
}

/// The platform's cache folder for MurmelCraft, if one can be found:
/// `MURMEL_CACHE_DIR`, else `~/Library/Caches/MurmelCraft` on macOS,
/// `%LOCALAPPDATA%\MurmelCraft\cache` on Windows, and
/// `$XDG_CACHE_HOME/murmelcraft` or `~/.cache/murmelcraft` elsewhere.
/// What is kept there can be made again (the preview grids of overviews,
/// [`crate::preview_cache`]).
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("MURMEL_CACHE_DIR") {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Caches/MurmelCraft"));
    }
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA")
            .map(|a| PathBuf::from(a).join("MurmelCraft").join("cache"));
    }
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".cache")))
        .map(|c| c.join("murmelcraft"))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub format: String,
    #[serde(default)]
    pub theme: Theme,
    /// Mesh colours that replace the built-in tokens.
    #[serde(default)]
    pub colors: MeshColors,
    /// Where the view cube is and whether it hides itself.
    #[serde(default)]
    pub view_cube: CubeSettings,
    /// Whether the window lets agents drive it (the live bridge).
    #[serde(default)]
    pub agents: Agents,
    /// Where new items start (`[defaults.<kind>]`, [`crate::defaults`]).
    #[serde(default, skip_serializing_if = "Defaults::is_empty")]
    pub defaults: Defaults,
    /// Your formulas (`[[formulas]]`, [`crate::calc`]), offered for
    /// every result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formulas: Vec<Formula>,
    /// The memory budget (`[memory]`, [`crate::memory`]).
    #[serde(default, skip_serializing_if = "Memory::is_default")]
    pub memory: Memory,
    /// Your keyboard shortcuts (`[keys]`, [`crate::keys`]): the keys of
    /// the commands you changed, by command id.
    #[serde(default, skip_serializing_if = "KeyMap::is_empty")]
    pub keys: KeyMap,
    /// When tips appear (`[help]`).
    #[serde(default, skip_serializing_if = "Help::is_default")]
    pub help: Help,
}

/// `[help]` in settings.toml: how long the pointer rests on a control
/// before its tip appears, and how much longer before the tip shows more
/// (the maintainer's card of 2026-11-25: 250 ms, then 1 s; Shift shows
/// more at once).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Help {
    #[serde(default = "tip_ms")]
    pub tip_ms: u32,
    #[serde(default = "rich_ms")]
    pub rich_ms: u32,
}

/// The longest delay either can have.
pub const MAX_TIP_MS: u32 = 10_000;

fn tip_ms() -> u32 {
    250
}

fn rich_ms() -> u32 {
    1000
}

impl Default for Help {
    fn default() -> Self {
        Self {
            tip_ms: tip_ms(),
            rich_ms: rich_ms(),
        }
    }
}

impl Help {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `[agents]` in settings.toml: the window listens for `murmel mcp --live`
/// while `enabled` (the default; the maintainer's choice of 2026-11-24).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agents {
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl Default for Agents {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            format: SETTINGS_FORMAT.into(),
            theme: Theme::default(),
            colors: MeshColors::default(),
            view_cube: CubeSettings::default(),
            agents: Agents::default(),
            defaults: Defaults::default(),
            formulas: Vec::new(),
            memory: Memory::default(),
            keys: KeyMap::new(),
            help: Help::default(),
        }
    }
}

/// Read `settings.toml` and `navigation.toml` from `dir`; a missing file
/// gives the default, a broken one an error naming it.
pub fn load(dir: &Path, macos: bool) -> (Settings, Navigation, Vec<String>) {
    let mut problems = Vec::new();
    let settings = match std::fs::read_to_string(dir.join("settings.toml")) {
        Ok(t) => toml::from_str(&t).unwrap_or_else(|e| {
            problems.push(format!("settings.toml: {e}"));
            Settings::default()
        }),
        Err(_) => Settings::default(),
    };
    let nav = match std::fs::read_to_string(dir.join("navigation.toml")) {
        Ok(t) => Navigation::from_toml(&t).unwrap_or_else(|e| {
            problems.push(format!("navigation.toml: {e}"));
            Navigation::platform_default(macos)
        }),
        Err(_) => Navigation::platform_default(macos),
    };
    (settings, nav, problems)
}

fn write(dir: &Path, name: &str, text: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let p = dir.join(name);
    std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(p)
}

pub fn save_settings(dir: &Path, s: &Settings) -> Result<PathBuf, String> {
    let text = toml::to_string_pretty(s).map_err(|e| e.to_string())?;
    write(dir, "settings.toml", &text)
}

pub fn save_navigation(dir: &Path, n: &Navigation) -> Result<PathBuf, String> {
    write(dir, "navigation.toml", &n.to_toml()?)
}
