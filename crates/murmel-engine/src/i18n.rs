//! Translation keys (UI spec, Principles): every visible string comes from
//! a Fluent file. V0.1 ships English (`assets/i18n/en/murmel.ftl`). The
//! window and the tools (exported legends) share it.

use std::collections::HashMap;

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};

const EN: &str = include_str!("../../../assets/i18n/en/murmel.ftl");

pub struct I18n {
    bundle: FluentBundle<FluentResource>,
    /// The messages that name a command's keys (`{ $key-view-fit }`),
    /// with the variables each names, so only they get those arguments.
    keyed: HashMap<String, Vec<String>>,
    /// The key arguments: `key-<command id, . and : as ->` to the keys
    /// as shown (your keys, set by the window).
    keys: HashMap<String, String>,
}

/// The Fluent variable that holds the keys of command `id`
/// (`view.fit` → `key-view-fit`, `clip.set:new` → `key-clip-set-new`).
pub fn key_variable(id: &str) -> String {
    format!("key-{}", id.replace(['.', ':'], "-"))
}

impl I18n {
    pub fn english() -> Result<Self, String> {
        let res = FluentResource::try_new(EN.to_string())
            .map_err(|(_, errs)| format!("murmel.ftl: {errs:?}"))?;
        let lang: unic_langid::LanguageIdentifier =
            "en-US".parse().map_err(|e| format!("language id: {e:?}"))?;
        let mut bundle = FluentBundle::new(vec![lang]);
        bundle.set_use_isolating(false);
        bundle
            .add_resource(res)
            .map_err(|errs| format!("murmel.ftl: {errs:?}"))?;
        // Messages (and their continuation lines) that use a key variable.
        let mut keyed: HashMap<String, Vec<String>> = HashMap::new();
        let mut current = "";
        for line in EN.lines() {
            if let Some((id, _)) = line.split_once(" = ")
                && !line.starts_with(char::is_whitespace)
            {
                current = id.trim();
            }
            for part in line.split("{ $key-").skip(1) {
                let var = format!("key-{}", part.split(' ').next().unwrap_or(""));
                keyed.entry(current.to_string()).or_default().push(var);
            }
        }
        Ok(Self {
            bundle,
            keyed,
            keys: HashMap::new(),
        })
    }

    /// The keys of the commands as shown (the window passes your keys),
    /// by command id; a string names them as `{ $key-<id> }`.
    pub fn set_keys(&mut self, keys: impl IntoIterator<Item = (String, String)>) {
        self.keys = keys
            .into_iter()
            .map(|(id, shown)| (key_variable(&id), shown))
            .collect();
    }

    /// The string of `key`; the key itself when it is missing, so a gap
    /// shows instead of an empty label.
    pub fn tr(&self, key: &str) -> String {
        self.tr_args(key, &[])
    }

    pub fn tr_args(&self, key: &str, args: &[(&str, FluentValue<'_>)]) -> String {
        let Some(msg) = self.bundle.get_message(key) else {
            return key.to_string();
        };
        let Some(pattern) = msg.value() else {
            return key.to_string();
        };
        // The modifier names of this platform, so a string that names a
        // key says Cmd and Option on macOS (`{ $ctrl } + O`).
        let mut fa = FluentArgs::new();
        let (ctrl, alt) = modifier_names();
        fa.set("ctrl", ctrl);
        fa.set("alt", alt);
        for var in self.keyed.get(key).into_iter().flatten() {
            if let Some(shown) = self.keys.get(var) {
                fa.set(var.clone(), shown.clone());
            }
        }
        for (k, v) in args {
            fa.set(*k, v.clone());
        }
        let mut errors = Vec::new();
        self.bundle
            .format_pattern(pattern, Some(&fa), &mut errors)
            .into_owned()
    }

    /// The English file has `key` (tests check that every key the window
    /// uses is there).
    pub fn has(&self, key: &str) -> bool {
        self.bundle.has_message(key)
    }
}

/// The names of the Ctrl and Alt keys of the key table on this platform:
/// Cmd and Option on macOS (the table's Ctrl is the Command key there).
pub fn modifier_names() -> (&'static str, &'static str) {
    if cfg!(target_os = "macos") {
        ("Cmd", "Option")
    } else {
        ("Ctrl", "Alt")
    }
}

/// A count with thin-space thousands groups (412 803).
pub fn count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('\u{2009}');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_grouped() {
        assert_eq!(count(7), "7");
        assert_eq!(count(412803), "412\u{2009}803");
        assert_eq!(count(1870221), "1\u{2009}870\u{2009}221");
    }
}
