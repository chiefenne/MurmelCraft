//! Translation keys: every string Kraxn shows comes from
//! `assets/i18n/en/kraxn.ftl` (AGENTS.md).

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};

const EN: &str = include_str!("../../../assets/i18n/en/kraxn.ftl");

pub struct I18n {
    bundle: FluentBundle<FluentResource>,
}

impl I18n {
    pub fn english() -> Result<Self, String> {
        let res = FluentResource::try_new(EN.to_string())
            .map_err(|(_, errs)| format!("kraxn.ftl: {errs:?}"))?;
        let lang: unic_langid::LanguageIdentifier =
            "en-US".parse().map_err(|e| format!("language id: {e:?}"))?;
        let mut bundle = FluentBundle::new(vec![lang]);
        bundle.set_use_isolating(false);
        bundle
            .add_resource(res)
            .map_err(|errs| format!("kraxn.ftl: {errs:?}"))?;
        Ok(Self { bundle })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn has(&self, key: &str) -> bool {
        self.bundle.get_message(key).is_some()
    }

    /// The string of `key` (the key itself when it is missing).
    pub fn tr(&self, key: &str) -> String {
        self.tr_args(key, &[])
    }

    pub fn tr_args(&self, key: &str, args: &[(&str, String)]) -> String {
        let Some(pattern) = self.bundle.get_message(key).and_then(|m| m.value()) else {
            return key.to_string();
        };
        let mut fa = FluentArgs::new();
        for (k, v) in args {
            fa.set(*k, v.clone());
        }
        let mut errs = Vec::new();
        self.bundle
            .format_pattern(pattern, Some(&fa), &mut errs)
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: every key the text mode and the window use is in the
    /// English file. Breaks when a key is added in code only (the raw key
    /// would show); the test reads the sources for `tr("` and `tr_args("`.
    #[test]
    fn every_key_is_translated() {
        let t = I18n::english().unwrap();
        let mut seen = 0;
        for s in [
            include_str!("text.rs"),
            include_str!("main.rs"),
            include_str!("window.rs"),
        ] {
            for part in s
                .split("tr(\"")
                .skip(1)
                .chain(s.split("tr_args(\"").skip(1))
            {
                let key = part.split('"').next().unwrap_or("");
                assert!(t.has(key), "missing key {key}");
                seen += 1;
            }
        }
        assert!(seen > 30, "{seen}");
        assert_eq!(
            t.tr_args("app-line", &[("version", "0.1.0".into())]),
            "Kraxn 0.1.0 installs MurmelCraft, KoflCraft and FoehnCraft."
        );
    }
}
