//! Your keyboard shortcuts: `[keys]` in settings.toml holds the keys of
//! the commands you changed, by command id (`view.fit = ["F"]`); every
//! other command keeps its built-in keys, so new keys of later versions
//! still arrive. A command with `[]` has no key. The command table itself
//! lives in `murmel-tools` (`commands::with_keys` lays this map over it);
//! here are the key texts: their form, the keys a shortcut may end in,
//! and the ones that are refused or that macOS takes first.
//!
//! A key text is the modifiers in the order `Ctrl`, `Alt`, `Shift`, then
//! one key, joined by `+`: `Ctrl+Shift+S`, `Alt+7`, `W`. `Ctrl` is the
//! Command key on macOS and `Alt` the Option key.

use std::collections::BTreeMap;

/// `[keys]`: command id to its keys, for the commands you changed.
pub type KeyMap = BTreeMap<String, Vec<String>>;

/// The keys a shortcut may end in (egui's key names).
pub const KEY_NAMES: &[&str] = &[
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "F13",
    "F14",
    "F15",
    "F16",
    "F17",
    "F18",
    "F19",
    "F20",
    "Left",
    "Right",
    "Up",
    "Down",
    "Escape",
    "Backspace",
    "Enter",
    "Insert",
    "Delete",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Space",
    "Comma",
    "Period",
    "Minus",
    "Plus",
    "Equals",
    "Semicolon",
    "Slash",
    "Backslash",
    "OpenBracket",
    "CloseBracket",
    "Backtick",
    "Quote",
];

/// Keys no command may take: Tab and Shift + Tab move the keyboard focus
/// between controls (accessibility).
pub const RESERVED: &[&str] = &["Tab", "Shift+Tab"];

/// Write `text` in the one form the table uses: modifiers `Ctrl`, `Alt`,
/// `Shift` in that order, then the key. Accepts any order and case,
/// `Cmd`/`Command` for Ctrl and `Option`/`Opt` for Alt. An error says
/// what is wrong, for the person who typed it.
pub fn canonical(text: &str) -> Result<String, String> {
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (key, mods) = match parts.split_last() {
        Some((k, m)) if !k.is_empty() => (*k, m),
        _ => return Err(format!("{text:?} names no key")),
    };
    let (mut ctrl, mut alt, mut shift) = (false, false, false);
    for m in mods {
        let flag = match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" | "command" => &mut ctrl,
            "alt" | "option" | "opt" => &mut alt,
            "shift" => &mut shift,
            other => {
                return Err(format!(
                    "{other:?} in {text:?} is not a modifier (Ctrl, Alt, Shift)"
                ));
            }
        };
        if *flag {
            return Err(format!("{text:?} names {m} twice"));
        }
        *flag = true;
    }
    let name = if key.eq_ignore_ascii_case("tab") {
        "Tab"
    } else {
        KEY_NAMES
            .iter()
            .find(|n| n.eq_ignore_ascii_case(key))
            .copied()
            .or(match key.to_ascii_lowercase().as_str() {
                "esc" => Some("Escape"),
                "del" => Some("Delete"),
                "return" => Some("Enter"),
                "arrowleft" => Some("Left"),
                "arrowright" => Some("Right"),
                "arrowup" => Some("Up"),
                "arrowdown" => Some("Down"),
                _ => None,
            })
            .ok_or_else(|| format!("{key:?} in {text:?} is not a key MurmelCraft knows"))?
    };
    let mut out = String::new();
    for (on, m) in [(ctrl, "Ctrl+"), (alt, "Alt+"), (shift, "Shift+")] {
        if on {
            out.push_str(m);
        }
    }
    out.push_str(name);
    Ok(out)
}

/// Whether a key (in canonical form) is refused for every command.
pub fn reserved(key: &str) -> bool {
    RESERVED.contains(&key)
}

/// Why macOS may take a key before the window sees it, as the end of a
/// translation key (`keys-os-<id>`); `None` when it does not.
pub fn macos_note(key: &str) -> Option<&'static str> {
    Some(match key {
        "Ctrl+Q" => "quit",
        "Ctrl+H" | "Ctrl+Alt+H" => "hide",
        "Ctrl+M" => "minimise",
        "Ctrl+Space" | "Ctrl+Alt+Space" => "spotlight",
        "Ctrl+Comma" => "preferences",
        "Ctrl+Shift+3" | "Ctrl+Shift+4" | "Ctrl+Shift+5" => "screenshot",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: a key text has one form whatever order and case it was
    /// typed in, with the Mac names as aliases, and nonsense is refused
    /// with a message. Breaks if two spellings of one key could be bound
    /// to two commands (the clash check compares texts). No other test
    /// parses typed keys.
    #[test]
    fn keys_have_one_form() {
        assert_eq!(canonical("shift+ctrl+s").unwrap(), "Ctrl+Shift+S");
        assert_eq!(canonical("Cmd + Option + 7").unwrap(), "Ctrl+Alt+7");
        assert_eq!(canonical("w").unwrap(), "W");
        assert_eq!(canonical("esc").unwrap(), "Escape");
        assert_eq!(canonical("f12").unwrap(), "F12");
        assert_eq!(canonical("Shift+Tab").unwrap(), "Shift+Tab");
        assert!(reserved(&canonical("shift+tab").unwrap()));
        assert!(canonical("Ctrl+").is_err());
        assert!(canonical("Hyper+K").is_err());
        assert!(canonical("Ctrl+Cmd+K").is_err(), "Cmd is Ctrl");
        assert!(canonical("Ctrl+Ü").is_err());
        assert_eq!(macos_note("Ctrl+Q"), Some("quit"));
        assert_eq!(macos_note("W"), None);
    }
}
