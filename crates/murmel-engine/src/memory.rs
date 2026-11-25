//! The memory budget (the maintainer's card of 2026-11-24: a mesh opens
//! whole up to a memory budget, half the computer's memory unless you set
//! one; a blocked VTKHDF file that would need more opens as an overview).
//! `[memory] budget_gib` in settings.toml sets it (`settings.set_memory`).

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// The budget when the computer's memory cannot be found: half of a
/// 32 GB computer.
pub const FALLBACK_BYTES: u64 = 16 << 30;

/// The budgets one can set, in GiB.
pub const MIN_GIB: f64 = 0.25;
pub const MAX_GIB: f64 = 4096.0;

/// `[memory]` in settings.toml.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Memory {
    /// The budget in GiB; left out, half the computer's memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_gib: Option<f64>,
    /// Convert a mesh that is not blocked and needs more than the budget
    /// without asking ([`crate::convert`]); off, the window asks.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub convert_large: bool,
}

impl Memory {
    pub fn is_default(&self) -> bool {
        *self == Memory::default()
    }
}

/// The computer's memory in bytes, if it can be found (Linux: MemTotal in
/// `/proc/meminfo`; macOS: `sysctl -n hw.memsize`). Found once.
pub fn system_bytes() -> Option<u64> {
    static TOTAL: OnceLock<Option<u64>> = OnceLock::new();
    *TOTAL.get_or_init(find_system_bytes)
}

fn find_system_bytes() -> Option<u64> {
    if cfg!(target_os = "linux") {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        return meminfo_total(&text);
    }
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        return String::from_utf8_lossy(&out.stdout).trim().parse().ok();
    }
    None
}

/// MemTotal of `/proc/meminfo`, in bytes.
fn meminfo_total(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// The budget in bytes: `budget_gib` when set, else half of `system`
/// (the computer's memory), else [`FALLBACK_BYTES`].
pub fn budget_bytes_of(budget_gib: Option<f64>, system: Option<u64>) -> u64 {
    match budget_gib {
        Some(g) if g.is_finite() && g > 0.0 => (g * GIB) as u64,
        _ => system.map_or(FALLBACK_BYTES, |s| s / 2),
    }
}

/// The budget in bytes on this computer.
pub fn budget_bytes(budget_gib: Option<f64>) -> u64 {
    budget_bytes_of(budget_gib, system_bytes())
}

/// `bytes` in GiB, to one decimal.
pub fn gib(bytes: u64) -> f64 {
    (bytes as f64 / GIB * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the budget is the one set, else half the computer's
    /// memory, else 16 GiB; MemTotal is read in KiB. Breaks when a budget
    /// set is ignored, the half is taken of KiB (a budget 1,024 times too
    /// small opens every mesh as an overview), or a broken setting (zero,
    /// NaN) gives a budget of nothing. The session tests use the budget
    /// only through the open.
    #[test]
    fn the_budget_is_set_or_half_the_memory() {
        let text = "MemTotal:       32768000 kB\nMemFree:  1000 kB\n";
        let total = meminfo_total(text);
        assert_eq!(total, Some(32_768_000 * 1024));
        assert_eq!(budget_bytes_of(None, total), 16_384_000 * 1024);
        assert_eq!(budget_bytes_of(Some(2.0), total), 2 << 30);
        assert_eq!(budget_bytes_of(None, None), FALLBACK_BYTES);
        assert_eq!(budget_bytes_of(Some(0.0), None), FALLBACK_BYTES);
        assert_eq!(budget_bytes_of(Some(f64::NAN), total), 16_384_000 * 1024);
        assert_eq!(meminfo_total("MemFree: 3 kB"), None);
        assert_eq!(gib(3 << 29), 1.5);
    }
}
