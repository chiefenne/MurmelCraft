//! Translation keys (UI spec, Principles): the window's strings come from
//! the engine's Fluent file (`murmel_engine::i18n`).

pub use murmel_engine::i18n::{I18n, count, modifier_names};

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
            include_str!("focus.rs"),
            include_str!("agents.rs"),
            include_str!("export_dialog.rs"),
            include_str!("movie_dialog.rs"),
            include_str!("field.rs"),
            include_str!("extracts.rs"),
            include_str!("clip_handle.rs"),
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
        for a in [
            "mesh", "zone", "kind", "patch", "cell", "face", "clip", "measure", "probe", "slice",
            "iso",
        ] {
            assert!(t.has(&format!("props-area-{a}")));
        }
        for c in murmel_engine::IsoColor::ALL {
            assert!(t.has(&format!("iso-color-{}", c.key())));
        }
        // Every property row of every item of the channel mesh, and of
        // its field results with a slice series and an iso-surface, has a
        // label.
        for file in ["channel/foehncraft.vtu", "vtkhdf/channel_fields.vtkhdf"] {
            let mut s = murmel_engine::Session::new(murmel_engine::Tokens::builtin().unwrap());
            s.config_dir = None;
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../test-fixtures")
                .join(file);
            s.open(&path).unwrap();
            let mut extra = vec![
                murmel_engine::Item::Mesh,
                murmel_engine::Item::Cell(0),
                murmel_engine::Item::Face(0),
            ];
            if file.ends_with(".vtkhdf") {
                for (tool, input) in [
                    (
                        "slice.set",
                        serde_json::json!({"new": true, "axis": "x", "count": 3}),
                    ),
                    (
                        "iso.set",
                        serde_json::json!({"new": true, "field": "pressure"}),
                    ),
                ] {
                    let e = murmel_tools::call(tool, input, &mut s);
                    assert!(e.ok(), "{tool}: {:?}", e.diagnostics);
                }
                extra.extend([murmel_engine::Item::Slice(0), murmel_engine::Item::Iso(0)]);
            }
            let (Some(m), d) = (&s.mesh, &s.display) else {
                unreachable!()
            };
            extra.push(murmel_engine::Item::Face(m.mesh.boundary_face_id[0]));
            let items = extra
                .into_iter()
                .chain(murmel_engine::display::all_items(&m.mesh));
            for item in items {
                for r in murmel_engine::inspect::rows(m, d, item) {
                    assert!(
                        t.has(&format!("prop-{}", r.key)),
                        "missing key prop-{} ({file})",
                        r.key
                    );
                }
            }
        }
    }
}
