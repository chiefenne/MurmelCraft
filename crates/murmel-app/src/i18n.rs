//! Translation keys (UI spec, Principles): every visible string comes from
//! a Fluent file. V0.1 ships English (`assets/i18n/en/murmel.ftl`).

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};

const EN: &str = include_str!("../../../assets/i18n/en/murmel.ftl");

pub struct I18n {
    bundle: FluentBundle<FluentResource>,
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
        Ok(Self { bundle })
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
        let mut fa = FluentArgs::new();
        for (k, v) in args {
            fa.set(*k, v.clone());
        }
        let mut errors = Vec::new();
        self.bundle
            .format_pattern(pattern, Some(&fa), &mut errors)
            .into_owned()
    }

    #[cfg(test)]
    pub fn has(&self, key: &str) -> bool {
        self.bundle.has_message(key)
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

    /// Invariant: every key the window uses and every tool's label key is
    /// in the English file (the window would show a raw key otherwise).
    #[test]
    fn every_key_is_translated() {
        let t = I18n::english().unwrap();
        let src = [
            include_str!("app.rs"),
            include_str!("viewport.rs"),
            include_str!("main.rs"),
        ];
        for s in src {
            for part in s
                .split("tr(\"")
                .skip(1)
                .chain(s.split("tr_args(\"").skip(1))
            {
                let key = part.split('"').next().unwrap_or("");
                assert!(t.has(key), "missing key {key}");
            }
        }
        for tool in murmel_tools::tools() {
            assert!(t.has(tool.label_key), "missing key {}", tool.label_key);
        }
        for p in ["orthographic", "perspective", "perspective_ortho_faces"] {
            assert!(t.has(&format!("projection-{p}")));
            assert!(t.has(&format!("projection-short-{p}")));
        }
        for p in murmel_engine::navigation::PRESETS {
            assert!(t.has(&format!("preset-{p}")));
        }
    }

    #[test]
    fn counts_are_grouped() {
        assert_eq!(count(7), "7");
        assert_eq!(count(412803), "412\u{2009}803");
        assert_eq!(count(1870221), "1\u{2009}870\u{2009}221");
    }
}
