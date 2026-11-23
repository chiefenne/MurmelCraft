//! The properties of an item (UI spec, "Properties panel"): label and
//! value rows that the panel shows and `inspect.item` returns. Labels are
//! keys (`prop-<key>` in the translations); values are JSON numbers,
//! strings or arrays; units name the SI unit of a number.

use murmel_mesh::{Bounds, Mesh, NO_CELL};
use serde::Serialize;
use serde_json::{Value, json};

use crate::display::{Display, Item};
use crate::session::{OpenMesh, Validity};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    pub key: &'static str,
    pub value: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<&'static str>,
}

fn row(key: &'static str, value: impl Into<Value>) -> Row {
    Row {
        key,
        value: value.into(),
        unit: None,
    }
}

fn unit(key: &'static str, value: impl Into<Value>, unit: &'static str) -> Row {
    Row {
        key,
        value: value.into(),
        unit: Some(unit),
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// Twice the vector area of a polygon (Newell), about its node average.
fn area_vector(mesh: &Mesh, face: &[u32]) -> [f64; 3] {
    let p: Vec<[f64; 3]> = face.iter().map(|&i| mesh.points[i as usize]).collect();
    let mut n = [0.0; 3];
    for i in 0..p.len() {
        let c = cross(p[i], p[(i + 1) % p.len()]);
        for k in 0..3 {
            n[k] += c[k];
        }
    }
    n
}

/// Six times the signed volume a polygon spans with the origin `o`,
/// as a fan about its node average (divergence theorem).
fn signed_volume6(mesh: &Mesh, face: &[u32], o: [f64; 3]) -> f64 {
    let p: Vec<[f64; 3]> = face
        .iter()
        .map(|&i| sub(mesh.points[i as usize], o))
        .collect();
    let k = p.len() as f64;
    let c = [0, 1, 2].map(|d| p.iter().map(|q| q[d]).sum::<f64>() / k);
    (0..p.len())
        .map(|i| dot(c, cross(p[i], p[(i + 1) % p.len()])))
        .sum()
}

/// The volume of cell `c` (faces of a VTK polyhedron point outwards; the
/// absolute value keeps an inside-out cell from cancelling others).
pub fn cell_volume(mesh: &Mesh, c: usize) -> f64 {
    let o = mesh.cell(c).next().map_or([0.0; 3], |f| {
        f.first().map_or([0.0; 3], |&i| mesh.points[i as usize])
    });
    (mesh
        .cell(c)
        .map(|f| signed_volume6(mesh, f, o))
        .sum::<f64>()
        / 6.0)
        .abs()
}

/// The volume the boundary encloses.
pub fn enclosed_volume(mesh: &Mesh) -> f64 {
    let o = mesh.bounds().center();
    (mesh
        .boundary
        .iter()
        .map(|f| signed_volume6(mesh, f, o))
        .sum::<f64>()
        / 6.0)
        .abs()
}

/// Area, mean normal, spread and bounds of the boundary faces of patch `p`.
#[derive(Clone, Debug, PartialEq)]
pub struct PatchStats {
    pub faces: usize,
    pub area: f64,
    /// The area-weighted mean normal; `None` when the faces close on
    /// themselves (a tube or a box), where their normals cancel.
    pub mean_normal: Option<[f64; 3]>,
    /// Largest angle between a face normal and the mean normal, degrees.
    pub spread_deg: Option<f64>,
    pub bounds: Bounds,
}

pub fn patch_stats(mesh: &Mesh, p: u32) -> PatchStats {
    let mut faces = 0;
    let mut area = 0.0;
    let mut sum = [0.0; 3];
    let mut normals = Vec::new();
    let mut b = Bounds::empty();
    for (f, face) in mesh.boundary.iter().enumerate() {
        if mesh.boundary_patch.get(f) != Some(&p) {
            continue;
        }
        faces += 1;
        let n = area_vector(mesh, face);
        let a = 0.5 * norm(n);
        area += a;
        for k in 0..3 {
            sum[k] += 0.5 * n[k];
        }
        if a > 0.0 {
            normals.push(n.map(|v| 0.5 * v / a));
        }
        for &i in face {
            b.add(mesh.points[i as usize]);
        }
    }
    let l = norm(sum);
    let mean = (l > 1e-6 * area).then(|| sum.map(|v| v / l));
    let spread = mean.map(|m| {
        normals
            .iter()
            .map(|n| dot(*n, m).clamp(-1.0, 1.0).acos().to_degrees())
            .fold(0.0, f64::max)
    });
    PatchStats {
        faces,
        area,
        mean_normal: mean,
        spread_deg: spread,
        bounds: b,
    }
}

fn round(v: f64, digits: i32) -> f64 {
    let s = 10f64.powi(digits);
    (v * s).round() / s
}

fn vec3(v: [f64; 3], digits: i32) -> Value {
    json!(v.map(|x| round(x, digits) + 0.0))
}

/// The rows of `item` in the open mesh.
pub fn rows(open: &OpenMesh, display: &Display, item: Item) -> Vec<Row> {
    let m = &open.mesh;
    let ev = open.evidence.as_ref();
    match item {
        Item::Mesh => {
            let file = open
                .path
                .file_name()
                .map_or(String::new(), |f| f.to_string_lossy().into_owned());
            let mut v = vec![
                row("file", file),
                row("format", "VTK unstructured grid (.vtu)"),
            ];
            if let Some(ver) = ev
                .and_then(|e| e.koflcraft_version.clone())
                .or_else(|| m.metadata.get("koflcraft_version").cloned())
            {
                v.push(row("koflcraft_version", ver));
            }
            if let Some(g) = ev.and_then(|e| e.generator.clone()) {
                v.push(row("generator", g));
            }
            v.push(row("validity", open.validity.key()));
            if let Validity::Failed(why) = &open.validity {
                v.push(row("validity_problems", why.join("; ")));
            }
            v.push(row("cells", m.cell_count()));
            if m.cell_count() > 0 {
                v.push(row(
                    "faces_interior",
                    m.unique_faces.saturating_sub(m.boundary.len()),
                ));
            }
            v.push(row("faces_boundary", m.boundary.len()));
            v.push(row("vertices", m.points.len()));
            if m.cell_count() > 0 {
                let cells: f64 = (0..m.cell_count()).map(|c| cell_volume(m, c)).sum();
                v.push(unit("volume", round(cells, 9), "m³"));
                if cells > 0.0 {
                    let err = (cells - enclosed_volume(m)).abs() / cells;
                    v.push(row("volume_error", format!("{err:.1e}")));
                }
            }
            if let Some(n) = ev.and_then(|e| e.inverted_faces) {
                v.push(row("inverted_faces", n));
            }
            let (cells, faces) = display.visible_counts(m);
            v.push(row("visible_cells", cells));
            v.push(row("visible_faces", faces));
            v.push(row("digest", open.digest.clone()));
            v
        }
        Item::Patch(p) => {
            let Some(patch) = m.patches.get(p as usize) else {
                return Vec::new();
            };
            let s = patch_stats(m, p);
            let ep = ev.and_then(|e| e.patch(&patch.name));
            let mut v = vec![
                row("name", patch.name.clone()),
                row("role", patch.role.name()),
            ];
            if let Some(t) = ep.and_then(|e| e.foehncraft_type.clone()) {
                v.push(row("foehncraft_type", t));
            }
            v.push(row("faces", s.faces));
            v.push(unit("area", round(s.area, 9), "m²"));
            v.push(row("mean_normal", s.mean_normal.map(|n| vec3(n, 4))));
            v.push(unit(
                "normal_spread",
                s.spread_deg.map(|d| round(d, 2)),
                "°",
            ));
            if !s.bounds.is_empty() {
                v.push(unit("bounds_min", vec3(s.bounds.min, 6), "m"));
                v.push(unit("bounds_max", vec3(s.bounds.max, 6), "m"));
            }
            v.push(row("hidden", display.is_hidden(item)));
            v
        }
        Item::Zone(z) => {
            let cells: Vec<usize> = (0..m.cell_count())
                .filter(|&c| m.cell_zone.get(c) == Some(&z))
                .collect();
            let vol: f64 = cells.iter().map(|&c| cell_volume(m, c)).sum();
            vec![
                row("name", m.zones.get(z as usize).cloned().unwrap_or_default()),
                row("cells", cells.len()),
                unit("volume", round(vol, 9), "m³"),
                row("hidden", display.is_hidden(item)),
            ]
        }
        Item::Kind(k) => {
            let cells = m.cell_kind.iter().filter(|&&x| x == k).count();
            let faces = m
                .boundary_owner
                .iter()
                .filter(|&&c| c != NO_CELL && m.cell_kind.get(c as usize) == Some(&k))
                .count();
            vec![
                row("name", k.name()),
                row("cells", cells),
                row("faces_boundary", faces),
                row("hidden", display.is_hidden(item)),
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> Mesh {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        murmel_mesh::read(&p).unwrap()
    }

    /// Invariant: the channel's cells fill the volume its boundary
    /// encloses, 8 m³ as KoflCraft's evidence says, and the patch areas
    /// match the evidence (inlet 2 m², walls 24 m²). A plane patch has one
    /// normal (spread 0).
    #[test]
    fn channel_volumes_and_patches() {
        let m = channel();
        let cells: f64 = (0..m.cell_count()).map(|c| cell_volume(&m, c)).sum();
        assert!((cells - 8.0).abs() < 1e-9, "{cells}");
        assert!((enclosed_volume(&m) - 8.0).abs() < 1e-9);
        let inlet = patch_stats(&m, 0);
        assert_eq!(inlet.faces, 85);
        assert!((inlet.area - 2.0).abs() < 1e-9);
        assert!(inlet.spread_deg.unwrap() < 1e-6);
        assert!((norm(inlet.mean_normal.unwrap()) - 1.0).abs() < 1e-9);
        // The walls are a closed tube: their normals cancel, so there is
        // no mean normal to show rather than a direction of round-off.
        let walls = patch_stats(&m, 2);
        assert!((walls.area - 24.0).abs() < 1e-9);
        assert_eq!(walls.mean_normal, None);
    }
}
