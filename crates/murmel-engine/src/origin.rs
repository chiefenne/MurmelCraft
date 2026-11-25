//! Where an open file comes from: one of our three tools (KoflCraft,
//! FoehnCraft, MurmelCraft) or another tool. File ▸ Open is for our
//! files; the formats of other tools (an OpenFOAM polyMesh, a VTK file
//! written by ParaView or VTK) come in through File ▸ Import, and the
//! window tags them "imported" (the maintainer's cards of 2026-11-25).
//!
//! The origin is read from the marks our tools leave in their files, not
//! from the extension: FoehnCraft writes `foehncraft_format` on
//! `/VTKHDF`, KoflCraft writes `koflcraft_version` (field data of its
//! `.vtu`, an attribute of its `.vtkhdf`) and a `mesh-evidence.json`
//! beside the mesh, and MurmelCraft's blocked files carry
//! `murmelcraft_blocks` (docs/blocked-vtkhdf.md).

use std::collections::BTreeMap;
use std::path::Path;

use murmel_mesh::evidence::Evidence;
use serde::{Deserialize, Serialize};

/// The container a mesh was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// A VTK unstructured grid (`.vtu`).
    Vtu,
    /// A VTKHDF file (`.vtkhdf`, `.hdf`, `.h5`).
    Vtkhdf,
    /// An OpenFOAM polyMesh (a case, `constant` or `polyMesh` folder).
    Openfoam,
}

impl Format {
    pub fn key(self) -> &'static str {
        match self {
            Format::Vtu => "vtu",
            Format::Vtkhdf => "vtkhdf",
            Format::Openfoam => "openfoam",
        }
    }
}

/// The tool that wrote the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Koflcraft,
    Foehncraft,
    Murmelcraft,
    /// An OpenFOAM polyMesh of another tool.
    Openfoam,
    /// A VTK file of another tool (ParaView, VTK, …).
    Vtk,
}

impl Origin {
    pub const ALL: [Origin; 5] = [
        Origin::Koflcraft,
        Origin::Foehncraft,
        Origin::Murmelcraft,
        Origin::Openfoam,
        Origin::Vtk,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Origin::Koflcraft => "koflcraft",
            Origin::Foehncraft => "foehncraft",
            Origin::Murmelcraft => "murmelcraft",
            Origin::Openfoam => "openfoam",
            Origin::Vtk => "vtk",
        }
    }

    /// One of our three tools.
    pub fn ours(self) -> bool {
        matches!(
            self,
            Origin::Koflcraft | Origin::Foehncraft | Origin::Murmelcraft
        )
    }
}

/// What an open mesh is: its container and the tool that wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub format: Format,
    pub origin: Origin,
}

impl Source {
    /// Read from `path` and the mesh's metadata and evidence.
    pub fn of(
        path: &Path,
        metadata: &BTreeMap<String, String>,
        evidence: Option<&Evidence>,
    ) -> Source {
        let format = if metadata.contains_key("vtkhdf")
            || metadata.contains_key("murmelcraft_blocks")
            || (path.is_file() && murmel_mesh::vtkhdf::is_vtkhdf(path))
        {
            Format::Vtkhdf
        } else if metadata.contains_key("openfoam_format") {
            Format::Openfoam
        } else {
            Format::Vtu
        };
        let has = |k: &str| metadata.contains_key(k);
        let origin = if has("foehncraft_format") {
            Origin::Foehncraft
        } else if has("koflcraft_version")
            || evidence.is_some_and(|e| e.koflcraft_version.is_some())
        {
            Origin::Koflcraft
        } else if has("murmelcraft_blocks") {
            Origin::Murmelcraft
        } else if format == Format::Openfoam {
            Origin::Openfoam
        } else {
            Origin::Vtk
        };
        Source { format, origin }
    }

    /// Came in through a format of another tool: an OpenFOAM polyMesh
    /// (KoflCraft's export too), or a VTK file none of our tools wrote.
    pub fn imported(self) -> bool {
        self.format == Format::Openfoam || !self.origin.ours()
    }

    /// The English name shown in Properties: "KoflCraft mesh (.vtu)",
    /// "OpenFOAM polyMesh (binary), imported".
    pub fn describe(self, metadata: &BTreeMap<String, String>) -> String {
        let blocked = metadata.contains_key("murmelcraft_blocks");
        let container = match self.format {
            Format::Vtu => ".vtu".to_string(),
            Format::Vtkhdf if blocked => "blocked .vtkhdf".to_string(),
            Format::Vtkhdf => ".vtkhdf".to_string(),
            Format::Openfoam => match metadata.get("openfoam_format") {
                Some(f) => format!("OpenFOAM polyMesh, {f}"),
                None => "OpenFOAM polyMesh".to_string(),
            },
        };
        let name = match self.origin {
            Origin::Koflcraft => format!("KoflCraft mesh ({container})"),
            Origin::Foehncraft => format!("FoehnCraft result ({container})"),
            Origin::Murmelcraft => format!("MurmelCraft blocked mesh ({container})"),
            Origin::Openfoam => match metadata.get("openfoam_format") {
                Some(f) => format!("OpenFOAM polyMesh ({f})"),
                None => "OpenFOAM polyMesh".to_string(),
            },
            Origin::Vtk => format!("VTK file ({container})"),
        };
        if self.imported() {
            format!("{name}, imported")
        } else {
            name
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(keys: &[&str]) -> BTreeMap<String, String> {
        keys.iter()
            .map(|k| (k.to_string(), "x".to_string()))
            .collect()
    }

    /// Invariant: the marks decide, FoehnCraft before KoflCraft before
    /// MurmelCraft's blocked layout, and only a file without any of them
    /// (or a polyMesh) is imported. Breaks if a KoflCraft blocked file
    /// (it carries both `koflcraft_version` and `murmelcraft_blocks`) were
    /// named MurmelCraft's, or a ParaView .vtu counted as ours.
    #[test]
    fn marks_decide_the_origin() {
        let p = Path::new("/nowhere/m.vtu");
        let s = |k: &[&str]| Source::of(p, &meta(k), None);
        assert_eq!(s(&[]).origin, Origin::Vtk);
        assert!(s(&[]).imported());
        assert_eq!(s(&["koflcraft_version"]).origin, Origin::Koflcraft);
        assert!(!s(&["koflcraft_version"]).imported());
        let kb = s(&["vtkhdf", "koflcraft_version", "murmelcraft_blocks"]);
        assert_eq!((kb.format, kb.origin), (Format::Vtkhdf, Origin::Koflcraft));
        let f = s(&["vtkhdf", "foehncraft_format", "koflcraft_version"]);
        assert_eq!(f.origin, Origin::Foehncraft);
        let m = s(&["vtkhdf", "murmelcraft_blocks"]);
        assert_eq!(m.origin, Origin::Murmelcraft);
        assert!(!m.imported());
        // A blocked copy of an OpenFOAM mesh is a .vtkhdf of ours.
        let bo = s(&["vtkhdf", "murmelcraft_blocks", "openfoam_format"]);
        assert_eq!(
            (bo.format, bo.origin),
            (Format::Vtkhdf, Origin::Murmelcraft)
        );
        let of = s(&["openfoam_format"]);
        assert_eq!((of.format, of.origin), (Format::Openfoam, Origin::Openfoam));
        assert!(of.imported());
        // KoflCraft's polyMesh export: KoflCraft's, through Import.
        let ko = s(&["openfoam_format", "koflcraft_version"]);
        assert_eq!(ko.origin, Origin::Koflcraft);
        assert!(ko.imported());
        assert_eq!(
            ko.describe(&meta(&["openfoam_format"])),
            "KoflCraft mesh (OpenFOAM polyMesh, x), imported"
        );
    }
}
