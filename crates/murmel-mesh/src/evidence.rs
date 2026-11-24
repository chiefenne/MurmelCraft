//! `mesh-evidence.json` (`koflcraft.mesh-evidence` 1.0 or 1.1) beside a
//! mesh: the KoflCraft version, generator and the summary the validity
//! badge and the mesh properties show. 1.1 names the hash of its `files`
//! in `file_hash` (`blake3`); 1.0 has none and holds SHA-256 digests,
//! still checked as such.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::{FileHash, MeshError};

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
    pub interior_faces: Option<u64>,
    pub max_relative_closure: Option<f64>,
    pub patches: Vec<EvidencePatch>,
    /// The hash of `files`: `file_hash`, SHA-256 when it is missing (1.0).
    pub file_hash: FileHash,
    /// File name and digest (hex, of `file_hash`) of each file KoflCraft
    /// wrote.
    pub files: Vec<(String, String)>,
    pub limitations: Vec<String>,
    #[serde(skip)]
    pub raw: Value,
}

/// A patch as the evidence summarises it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EvidencePatch {
    pub name: String,
    pub role: Option<String>,
    pub foehncraft_type: Option<String>,
    pub faces: Option<u64>,
    pub area: Option<f64>,
}

impl Evidence {
    /// The evidence of the patch called `name`.
    pub fn patch(&self, name: &str) -> Option<&EvidencePatch> {
        self.patches.iter().find(|p| p.name == name)
    }

    /// The digest the evidence records for the file called `name`.
    pub fn digest(&self, name: &str) -> Option<&str> {
        self.files
            .iter()
            .find(|f| f.0 == name)
            .map(|f| f.1.as_str())
    }

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
        let file_hash = match &raw["file_hash"] {
            Value::Null => FileHash::Sha256,
            v => v.as_str().and_then(FileHash::from_key).ok_or_else(|| {
                MeshError::new(
                    "MESH011",
                    format!("mesh-evidence.json names an unknown file_hash {v}"),
                )
            })?,
        };
        let s = &raw["summary"];
        Ok(Self {
            koflcraft_version: raw["koflcraft_version"].as_str().map(str::to_string),
            generator: raw["generator"].as_str().map(str::to_string),
            cells: s["cells"].as_u64(),
            boundary_faces: s["boundary_faces"].as_u64(),
            total_volume: s["total_volume"].as_f64(),
            inverted_faces: s["inverted_faces"].as_u64(),
            interior_faces: s["interior_faces"].as_u64(),
            max_relative_closure: s["max_relative_closure"].as_f64(),
            patches: s["patches"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| {
                    Some(EvidencePatch {
                        name: p["name"].as_str()?.to_string(),
                        role: p["role"].as_str().map(str::to_string),
                        foehncraft_type: p["foehncraft_type"].as_str().map(str::to_string),
                        faces: p["faces"].as_u64(),
                        area: p["area"].as_f64(),
                    })
                })
                .collect(),
            file_hash,
            files: raw["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| Some((f[0].as_str()?.to_string(), f[1].as_str()?.to_string())))
                .collect(),
            limitations: raw["limitations"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|l| l.as_str().map(str::to_string))
                .collect(),
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
        let walls = e.patch("walls").unwrap();
        assert_eq!(walls.faces, Some(256));
        assert_eq!(walls.foehncraft_type.as_deref(), Some("wall"));
        assert_eq!(e.digest("foehncraft.vtu").map(str::len), Some(64));
        assert_eq!(e.file_hash, FileHash::Sha256);
    }

    /// Invariant: evidence 1.1 names its hash in `file_hash`, 1.0 (none)
    /// means SHA-256, and a hash MurmelCraft does not know is MESH011.
    /// Breaks if the key is ignored: KoflCraft's BLAKE3 digests would be
    /// compared with SHA-256 ones and every new bundle would fail its
    /// check. The fixtures hold only one version each.
    #[test]
    fn file_hash_names_the_digests() {
        let with = |h: Value| {
            let mut v = serde_json::json!({"format": FORMAT, "files": [["a.vtu", "00"]]});
            if !h.is_null() {
                v["file_hash"] = h;
            }
            Evidence::parse(&v.to_string())
        };
        assert_eq!(with(Value::Null).unwrap().file_hash, FileHash::Sha256);
        assert_eq!(with("blake3".into()).unwrap().file_hash, FileHash::Blake3);
        assert_eq!(with("sha256".into()).unwrap().file_hash, FileHash::Sha256);
        assert_eq!(with("md5".into()).unwrap_err().code, "MESH011");
        assert_eq!(with(3.into()).unwrap_err().code, "MESH011");
    }

    #[test]
    fn wrong_format_is_mesh011() {
        assert_eq!(Evidence::parse("{}").unwrap_err().code, "MESH011");
        assert_eq!(Evidence::parse("nope").unwrap_err().code, "MESH011");
    }
}
