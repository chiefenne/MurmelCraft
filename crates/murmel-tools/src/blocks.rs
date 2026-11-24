//! `blocks.make`: write a mesh and its fields as a blocked VTKHDF file
//! (`docs/blocked-vtkhdf.md`, large-mesh post-processing stage 2), the
//! layout MurmelCraft reads a region of without the whole mesh. The
//! writer is [`murmel_mesh::blocks`].

use std::path::{Path, PathBuf};

use murmel_engine::Session;
use murmel_mesh::blocks::{self, DEFAULT_BLOCK_CELLS, MAX_BLOCK_CELLS, MIN_BLOCK_CELLS};
use murmel_mesh::{Mesh, Results};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{object, parse};
use crate::{Diagnostic, Run};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MakeInput {
    out: PathBuf,
    mesh: Option<PathBuf>,
    block_cells: Option<u64>,
}

fn mesh_err(e: murmel_mesh::MeshError) -> Diagnostic {
    Diagnostic::new(e.code, e.message)
}

/// The same file, links followed where both exist.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

pub(crate) fn blocks_make(input: Value, s: &mut Session) -> Run {
    let i: MakeInput = parse(input)?;
    let block_cells = match i.block_cells {
        None => DEFAULT_BLOCK_CELLS,
        Some(n) if (MIN_BLOCK_CELLS as u64..=MAX_BLOCK_CELLS as u64).contains(&n) => n as usize,
        Some(n) => {
            return Err(Diagnostic::new(
                "BLOCK002",
                format!("block_cells {n} is outside {MIN_BLOCK_CELLS} to {MAX_BLOCK_CELLS}"),
            ));
        }
    };
    let read;
    let (mesh, results, source): (&Mesh, &Results, PathBuf) = match &i.mesh {
        Some(p) => {
            read = murmel_mesh::read_results_with(p, false).map_err(mesh_err)?;
            (&read.0, &read.1, p.clone())
        }
        None => {
            let open = s
                .mesh
                .as_ref()
                .ok_or_else(|| Diagnostic::new("VIEW001", "no mesh is open; name one with mesh"))?;
            (&open.mesh, &open.fields.results, open.path.clone())
        }
    };
    // never write over the mesh being converted (or one of its files)
    let sources = if mesh.files.is_empty() {
        vec![source.clone()]
    } else {
        mesh.files.clone()
    };
    if sources.iter().any(|f| same_file(f, &i.out)) {
        return Err(Diagnostic::new(
            "BLOCK001",
            format!("{} is the mesh being converted", i.out.display()),
        ));
    }
    let w = blocks::write_blocked(&i.out, mesh, results, block_cells).map_err(mesh_err)?;
    Ok(json!({
        "files": [i.out],
        "source": source,
        "layout": blocks::LAYOUT,
        "written": w,
    }))
}

pub(crate) fn blocks_make_schema() -> Value {
    object(
        json!({
            "out": {"type": "string", "description": "The blocked VTKHDF file to write, usually *.vtkhdf"},
            "mesh": {"type": "string", "description": "The mesh to convert: a .vtu, an OpenFOAM case or polyMesh folder, or a VTKHDF file; left out, the open mesh and its fields"},
            "block_cells": {"type": "integer", "minimum": MIN_BLOCK_CELLS, "maximum": MAX_BLOCK_CELLS, "description": "Owned cells per block (default 65536)"}
        }),
        &["out"],
    )
}

#[cfg(test)]
mod tests {
    use crate::call;
    use serde_json::json;

    fn fixture(rel: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
            .join(rel)
    }

    fn session() -> murmel_engine::Session {
        murmel_engine::Session::new(murmel_engine::Tokens::builtin().unwrap())
    }

    /// Invariant: blocks.make converts a named mesh without touching the
    /// session, or the open mesh with its fields; the file it writes opens
    /// as the same mesh with mesh.open; and it refuses a block size out of
    /// range (BLOCK002), writing over the mesh it converts (BLOCK001) and
    /// a call with no mesh at all (VIEW001).
    #[test]
    fn blocks_make_converts_and_refuses() {
        let dir = std::env::temp_dir().join(format!("murmel-blocks-tool-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("channel.vtkhdf");
        let mut s = session();
        let e = call("blocks.make", json!({"out": out}), &mut s);
        assert_eq!(e.diagnostics[0].code, "VIEW001");
        let src = fixture("vtkhdf/channel_fields.vtkhdf");
        let e = call("blocks.make", json!({"out": out, "mesh": src}), &mut s);
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert!(s.mesh.is_none(), "a named mesh is not opened");
        let w = &e.result["written"];
        assert_eq!(w["cells"], 776);
        assert_eq!(w["blocks"], 1);
        assert_eq!(w["fields"], 8);
        // the open mesh, with its fields
        assert!(call("mesh.open", json!({"path": src}), &mut s).ok());
        let again = dir.join("again.vtkhdf");
        let e = call("blocks.make", json!({"out": again}), &mut s);
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert!(call("mesh.open", json!({"path": again}), &mut s).ok());
        let m = s.mesh.as_ref().unwrap();
        assert_eq!(m.mesh.cell_count(), 776);
        assert_eq!(
            m.mesh
                .metadata
                .get("murmelcraft_blocks")
                .map(String::as_str),
            Some("1")
        );
        assert!(m.fields.results.get("pressure").is_some());
        // refusals
        for (input, code) in [
            (
                json!({"out": dir.join("x.vtkhdf"), "block_cells": 100}),
                "BLOCK002",
            ),
            (
                json!({"out": dir.join("x.vtkhdf"), "block_cells": 2_000_000}),
                "BLOCK002",
            ),
            (json!({"out": again}), "BLOCK001"),
        ] {
            let e = call("blocks.make", input, &mut s);
            assert_eq!(e.diagnostics[0].code, code);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
