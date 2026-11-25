//! Translation keys (UI spec, Principles): the window's strings come from
//! the engine's Fluent file (`murmel_engine::i18n`).

pub use murmel_engine::i18n::{I18n, count, modifier_names};

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: every `{ $key-<command> }` of the English file names a
    /// command of the table, and the window gives such a string the keys
    /// you set. Breaks if a command is renamed and a tip keeps the old
    /// name (it would show the raw variable), or the keys stop reaching
    /// the strings after a change. No other test formats key variables.
    #[test]
    fn key_variables_name_commands() {
        const FTL: &str = include_str!("../../../assets/i18n/en/murmel.ftl");
        let all = murmel_tools::commands::commands();
        let vars: Vec<String> = all
            .iter()
            .map(|c| murmel_engine::i18n::key_variable(&c.id))
            .collect();
        let mut seen = 0;
        for part in FTL.split("{ $key-").skip(1) {
            let name = format!("key-{}", part.split(' ').next().unwrap_or(""));
            assert!(vars.contains(&name), "{name} names no command");
            seen += 1;
        }
        assert!(seen > 20, "{seen}");
        let mut t = I18n::english().unwrap();
        t.set_keys(all.iter().map(|c| (c.id.clone(), c.keys.join(", "))));
        assert_eq!(t.tr("toolbar-fit-tip"), "Fit the mesh in the view (F)");
        t.set_keys([("view.fit".to_string(), "Shift + F".to_string())]);
        assert_eq!(
            t.tr("toolbar-fit-tip"),
            "Fit the mesh in the view (Shift + F)"
        );
    }

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
            include_str!("opening.rs"),
            include_str!("field.rs"),
            include_str!("extracts.rs"),
            include_str!("clip_handle.rs"),
            include_str!("streams.rs"),
            include_str!("stream_handle.rs"),
            include_str!("derived.rs"),
            include_str!("defaults.rs"),
            include_str!("shortcuts.rs"),
            include_str!("measure.rs"),
            include_str!("probe_history.rs"),
            include_str!("menus.rs"),
            include_str!("tips.rs"),
            include_str!("help_panel.rs"),
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
        for c in murmel_engine::help::CHAPTERS {
            assert!(t.has(&format!("help-chapter-{c}")), "missing chapter {c}");
        }
        // The name and the one-line tip of every help page.
        for p in murmel_engine::help::pages() {
            assert!(t.has(&p.title_key()), "missing key {}", p.title_key());
            assert!(t.has(&p.line_key()), "missing key {}", p.line_key());
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
        {
            use murmel_engine::navigation::{Action, Button, Gesture};
            for a in Action::ALL {
                assert!(t.has(&format!("action-{}", a.key())));
            }
            for b in Button::ALL {
                assert!(t.has(&format!("button-{}", b.key())));
            }
            for g in Gesture::ALL {
                assert!(t.has(&format!("gesture-{}", g.key())));
            }
            for why in [
                "quit",
                "hide",
                "minimise",
                "spotlight",
                "preferences",
                "screenshot",
            ] {
                assert!(t.has(&format!("keys-os-{why}")));
            }
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
        for v in ["valid", "no_evidence", "failed", "checking"] {
            assert!(t.has(&format!("validity-{v}")));
        }
        for a in [
            "mesh", "zone", "kind", "patch", "cell", "face", "clip", "measure", "probe", "slice",
            "iso", "arrows", "stream", "region",
        ] {
            assert!(t.has(&format!("props-area-{a}")));
        }
        for c in murmel_engine::IsoColor::ALL {
            assert!(t.has(&format!("iso-color-{}", c.key())));
        }
        for c in murmel_engine::glyph::ArrowColor::ALL {
            assert!(t.has(&format!("iso-color-{}", c.key())));
        }
        for l in murmel_engine::glyph::Length::ALL {
            assert!(t.has(&format!("arrow-length-{}", l.key())));
        }
        for k in murmel_engine::defaults::Kind::ALL {
            assert!(t.has(&format!("defaults-kind-{}", k.key())));
            for key in k.keys() {
                assert!(t.has(&format!("default-{}-{key}", k.key())), "{key}");
            }
        }
        for l in murmel_engine::MeshLook::ALL {
            assert!(t.has(&format!("mesh-look-{}", l.key())));
        }
        for k in murmel_engine::region::Shape::KINDS {
            assert!(t.has(&format!("region-shape-{k}")));
        }
        for k in murmel_engine::Seed::KINDS {
            assert!(t.has(&format!("stream-seed-{k}")));
            assert!(t.has(&format!("tree-stream-from-{k}")));
            assert!(t.has(&format!("stream-seed-kind-{k}")));
        }
        for d in murmel_engine::Direction::ALL {
            assert!(t.has(&format!("stream-direction-{}", d.key())));
        }
        for l in [murmel_engine::Look::Lines, murmel_engine::Look::Tubes] {
            assert!(t.has(&format!("stream-look-{}", l.key())));
        }
        // The Derived fields window: each quantity, group, input and
        // source of reference values.
        use murmel_engine::derive::{Group, Input, Quantity};
        for q in Quantity::ALL {
            assert!(t.has(&format!("derived-{}", q.key())));
            assert!(t.has(&format!("derived-formula-{}", q.key())));
        }
        for g in Group::ALL {
            assert!(t.has(&format!("derived-group-{}", g.key())));
        }
        for i in Input::ALL {
            assert!(t.has(&format!("derived-input-{}", i.key())));
            assert!(t.has(&format!("derived-no-{}", i.key())));
        }
        for s in ["case", "defaults", "built_in", "edited"] {
            assert!(t.has(&format!("derived-from-{s}")));
        }
        for k in murmel_engine::derive::REFERENCE_KEYS {
            assert!(t.has(&format!("derived-ref-{k}")));
        }
        for f in murmel_engine::calc::FUNCTIONS {
            assert!(t.has(&format!("calc-fn-{}", f.name)), "calc-fn-{}", f.name);
        }
        for n in ["x", "y", "z", "pos", "pi"] {
            assert!(t.has(&format!("calc-name-{n}")));
        }
        // Every property row of every item of the channel mesh, of a
        // region of every shape, and of its field results with a slice
        // series and an iso-surface, has a label.
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
            let (zone, patch) = {
                let m = &s.mesh.as_ref().unwrap().mesh;
                (m.zones.first().cloned(), m.patches[0].name.clone())
            };
            let mut shapes = vec![
                serde_json::json!({"new": true, "shape": "box"}),
                serde_json::json!({"new": true, "shape": "sphere"}),
                serde_json::json!({"new": true, "shape": "slab"}),
                serde_json::json!({"new": true, "shape": "patch", "patch": patch, "layers": 2}),
                serde_json::json!({"new": true, "shape": "cells", "cells": [0], "rings": 1}),
            ];
            if let Some(z) = zone {
                shapes.push(serde_json::json!({"new": true, "shape": "zone", "zone": z}));
            }
            for (k, input) in shapes.into_iter().enumerate() {
                let e = murmel_tools::call("region.set", input, &mut s);
                assert!(e.ok(), "region.set: {:?}", e.diagnostics);
                extra.push(murmel_engine::Item::Region(k as u32));
            }
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
                    ("arrows.set", serde_json::json!({})),
                    (
                        "stream.set",
                        serde_json::json!({"new": true, "seed": "line", "count": 4, "look": "tubes"}),
                    ),
                    (
                        "stream.set",
                        serde_json::json!({"new": true, "seed": "sphere", "color": "field"}),
                    ),
                    (
                        "stream.set",
                        serde_json::json!({"new": true, "seed": "patch"}),
                    ),
                    (
                        "stream.set",
                        serde_json::json!({"new": true, "seed": "plane", "count": 9}),
                    ),
                ] {
                    let e = murmel_tools::call(tool, input, &mut s);
                    assert!(e.ok(), "{tool}: {:?}", e.diagnostics);
                }
                extra.extend([
                    murmel_engine::Item::Slice(0),
                    murmel_engine::Item::Iso(0),
                    murmel_engine::Item::Arrows,
                    murmel_engine::Item::Stream(0),
                    murmel_engine::Item::Stream(1),
                    murmel_engine::Item::Stream(2),
                    murmel_engine::Item::Stream(3),
                ]);
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
