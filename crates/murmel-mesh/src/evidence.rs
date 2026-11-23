//! `mesh-evidence.json` (`koflcraft.mesh-evidence` 1.0) beside a mesh:
//! the KoflCraft version, generator and the summary the validity badge and
//! the mesh properties show.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::MeshError;

pub const FORMAT: &str = "koflcraft.mesh-evidence";

/// The parts of the evidence the viewer shows; `raw` keeps the rest.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Evidence {
    pub koflcraft_version: Option<String>,
    pub generator: Option<String>,
    pub cells: Option<u64>,
    pub boundary_faces: Option<u64>,
    pub total_volume: Option<f64>,
    pub inverted_faces: Option<u64>,
    #[serde(skip)]
    pub raw: Value,
}

impl Evidence {
    pub fn parse(text: &str) -> Result<Self, MeshError> {
        let raw: Value = serde_json::from_str(text)
            .map_err(|e| MeshError::new("MESH011", format!("mesh-evidence.json: {e}")))?;
        if raw["format"] != FORMAT {
            return Err(MeshError::new(
                "MESH011",
                format!(
                    "mesh-evidence.json has format {}, not {FORMAT}",
                    raw["format"]
                ),
            ));
        }
        let s = &raw["summary"];
        Ok(Self {
            koflcraft_version: raw["koflcraft_version"].as_str().map(str::to_string),
            generator: raw["generator"].as_str().map(str::to_string),
            cells: s["cells"].as_u64(),
            boundary_faces: s["boundary_faces"].as_u64(),
            total_volume: s["total_volume"].as_f64(),
            inverted_faces: s["inverted_faces"].as_u64(),
            raw,
        })
    }

    /// The evidence beside `mesh_path`: `Ok(None)` when there is none.
    pub fn beside(mesh_path: &Path) -> Result<Option<Self>, MeshError> {
        let Some(dir) = mesh_path.parent() else {
            return Ok(None);
        };
        let p = dir.join("mesh-evidence.json");
        if !p.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&p)
            .map_err(|e| MeshError::new("MESH011", format!("{}: {e}", p.display())))?;
        Self::parse(&text).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_channel_evidence() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        let e = Evidence::beside(&p).unwrap().unwrap();
        assert_eq!(e.cells, Some(776));
        assert_eq!(e.inverted_faces, Some(0));
        assert_eq!(e.generator.as_deref(), Some("straight_channel"));
    }

    #[test]
    fn wrong_format_is_mesh011() {
        assert_eq!(Evidence::parse("{}").unwrap_err().code, "MESH011");
        assert_eq!(Evidence::parse("nope").unwrap_err().code, "MESH011");
    }
}
