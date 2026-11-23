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
            include_str!("panels.rs"),
            include_str!("commands.rs"),
            include_str!("cube.rs"),
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
        // The command table, the views of the cube and the render modes.
        for c in murmel_tools::commands::commands() {
            assert!(t.has(&c.label_key), "missing key {}", c.label_key);
            assert!(t.has(&format!("group-{}", c.group)));
        }
        for v in murmel_engine::StandardView::cube() {
            assert!(t.has(&format!("view-{}", v.name())));
        }
        for m in murmel_engine::RenderMode::ALL {
            assert!(t.has(&format!("render-mode-{}", m.key())));
        }
        for p in murmel_engine::navigation::PRESETS {
            assert!(t.has(&format!("preset-{p}")));
        }
        for c in murmel_engine::ColorBy::ALL {
            assert!(t.has(&format!("color-by-{}", c.key())));
        }
        for m in murmel_engine::Metric::ALL {
            assert!(t.has(&format!("metric-{}", m.key())));
        }
        for k in murmel_mesh::CellKind::ALL {
            assert!(t.has(&format!("kind-{}", k.name())));
        }
        for r in murmel_mesh::PatchRole::ALL {
            assert!(t.has(&format!("role-{}", r.name())));
        }
        for v in ["valid", "no_evidence", "failed"] {
            assert!(t.has(&format!("validity-{v}")));
        }
        for a in ["mesh", "zone", "kind", "patch", "cell", "face"] {
            assert!(t.has(&format!("props-area-{a}")));
        }
        // Every property row of every item of the channel has a label.
        let mut s = murmel_engine::Session::new(murmel_engine::Tokens::builtin().unwrap());
        s.config_dir = None;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        s.open(&path).unwrap();
        let (Some(m), d) = (&s.mesh, &s.display) else {
            unreachable!()
        };
        let items = std::iter::once(murmel_engine::Item::Mesh)
            .chain(murmel_engine::display::all_items(&m.mesh))
            .chain([
                murmel_engine::Item::Cell(0),
                murmel_engine::Item::Face(0),
                murmel_engine::Item::Face(m.mesh.boundary_face_id[0]),
            ]);
        for item in items {
            for r in murmel_engine::inspect::rows(m, d, item) {
                assert!(
                    t.has(&format!("prop-{}", r.key)),
                    "missing key prop-{}",
                    r.key
                );
            }
        }
    }

    #[test]
    fn counts_are_grouped() {
        assert_eq!(count(7), "7");
        assert_eq!(count(412803), "412\u{2009}803");
        assert_eq!(count(1870221), "1\u{2009}870\u{2009}221");
    }
}
