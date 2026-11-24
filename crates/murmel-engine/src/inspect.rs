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

/// Centre (area-weighted, from the fan about the node average) and area
/// vector of a polygon; the area vector follows the node order.
pub fn face_centre_area(mesh: &Mesh, face: &[u32]) -> ([f64; 3], [f64; 3]) {
    let p: Vec<[f64; 3]> = face.iter().map(|&i| mesh.points[i as usize]).collect();
    if p.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }
    let k = p.len() as f64;
    let m = [0, 1, 2].map(|d| p.iter().map(|q| q[d]).sum::<f64>() / k);
    let mut s = [0.0; 3];
    let mut c = [0.0; 3];
    let mut total = 0.0;
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        let n = cross(sub(a, m), sub(b, m));
        let w = norm(n);
        for d in 0..3 {
            s[d] += 0.5 * n[d];
            c[d] += w * (m[d] + a[d] + b[d]) / 3.0;
        }
        total += w;
    }
    let centre = if total > 0.0 { c.map(|v| v / total) } else { m };
    (centre, s)
}

/// The centroid of cell `c`: the volume-weighted centroids of the
/// pyramids its faces span with the average of the face centres.
pub fn cell_centroid(mesh: &Mesh, c: usize) -> [f64; 3] {
    let faces: Vec<([f64; 3], [f64; 3])> =
        mesh.cell(c).map(|f| face_centre_area(mesh, f)).collect();
    if faces.is_empty() {
        return [0.0; 3];
    }
    let k = faces.len() as f64;
    let apex = [0, 1, 2].map(|d| faces.iter().map(|f| f.0[d]).sum::<f64>() / k);
    let (mut vol, mut acc) = (0.0, [0.0; 3]);
    for (fc, s) in &faces {
        let v = dot(*s, sub(*fc, apex)).abs() / 3.0;
        vol += v;
        for d in 0..3 {
            acc[d] += v * (0.75 * fc[d] + 0.25 * apex[d]);
        }
    }
    if vol > 0.0 {
        acc.map(|v| v / vol)
    } else {
        apex
    }
}

/// Non-orthogonality (degrees) and skewness of distinct face `u`, given
/// the centroids of its owner and neighbour. Non-orthogonality is the
/// angle between the face's area vector and the vector joining the cell
/// centres (to the face centre on the boundary); skewness is the distance
/// from the face centre to where that line crosses the face plane, over
/// the length of the line (Jasak, "Error analysis and estimation for the
/// finite volume method", PhD thesis, Imperial College 1996, ch. 3;
/// Ferziger and Perić, "Computational methods for fluid dynamics",
/// sec. 8.6). Skewness is `None` on the boundary.
pub fn face_quality(
    centre: [f64; 3],
    area: [f64; 3],
    owner: [f64; 3],
    neighbour: Option<[f64; 3]>,
) -> (f64, Option<f64>) {
    let d = sub(neighbour.unwrap_or(centre), owner);
    let (ld, la) = (norm(d), norm(area));
    if ld <= 0.0 || la <= 0.0 {
        return (0.0, None);
    }
    let cos = (dot(d, area) / (ld * la)).abs().clamp(0.0, 1.0);
    let non_ortho = cos.acos().to_degrees();
    let skew = neighbour.and_then(|_| {
        let dn = dot(d, area);
        (dn.abs() > 0.0).then(|| {
            let t = dot(sub(centre, owner), area) / dn;
            let hit = [0, 1, 2].map(|k| owner[k] + t * d[k]);
            norm(sub(centre, hit)) / ld
        })
    });
    (non_ortho, skew)
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
            let format = match m.metadata.get("openfoam_format") {
                Some(f) => format!("OpenFOAM polyMesh ({f})"),
                None => "VTK unstructured grid (.vtu)".into(),
            };
            let mut v = vec![row("file", file), row("format", format)];
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
            if let Some(t) = m
                .metadata
                .get(&format!("openfoam_patch_type.{}", patch.name))
            {
                v.push(row("openfoam_type", t.clone()));
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
        Item::Cell(c) => cell_rows(m, display, c as usize),
        Item::Clip(k) => {
            let Some(c) = display.clips.get(k as usize) else {
                return Vec::new();
            };
            let (offset, range) = c.offset(&m.bounds());
            vec![
                row("name", format!("{}", k + 1)),
                row("mode", c.mode.key()),
                row("enabled", c.enabled),
                unit("origin", vec3(c.origin, 6), "m"),
                row("normal", vec3(c.normal, 4)),
                unit("offset", round(offset, 6), "m"),
                unit("offset_min", round(range[0], 6), "m"),
                unit("offset_max", round(range[1], 6), "m"),
                row("current", display.current_clip() == Some(k as usize)),
            ]
        }
        Item::Face(u) => face_rows(m, display, u as usize),
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

/// The worst non-orthogonality (degrees) and skewness over the faces of
/// cell `c`, as the Properties panel shows them.
pub fn cell_quality(m: &Mesh, c: usize) -> (f64, f64) {
    let centroid = cell_centroid(m, c);
    let (mut worst_no, mut worst_skew) = (0.0f64, 0.0f64);
    for f in m.cell_face_range(c) {
        let Some(&u) = m.cell_face_id.get(f) else {
            continue;
        };
        let other = m.across(u as usize, c as u32);
        let (fc, s) = face_centre_area(m, m.faces.get(f));
        let nb = (other != NO_CELL).then(|| cell_centroid(m, other as usize));
        let (no, skew) = face_quality(fc, s, centroid, nb);
        worst_no = worst_no.max(no);
        if let Some(sk) = skew {
            worst_skew = worst_skew.max(sk);
        }
    }
    (worst_no, worst_skew)
}

/// The rows of cell `c` (UI spec, Properties: a cell).
fn cell_rows(m: &Mesh, display: &Display, c: usize) -> Vec<Row> {
    if c >= m.cell_count() {
        return Vec::new();
    }
    let centroid = cell_centroid(m, c);
    let mut nodes: Vec<u32> = m.cell(c).flatten().copied().collect();
    nodes.sort_unstable();
    nodes.dedup();
    let neighbours: Vec<u32> = m
        .cell_face_range(c)
        .filter_map(|f| m.cell_face_id.get(f))
        .map(|&u| m.across(u as usize, c as u32))
        .filter(|&o| o != NO_CELL)
        .collect();
    let (worst_no, worst_skew) = cell_quality(m, c);
    let mut neighbours = neighbours;
    neighbours.sort_unstable();
    neighbours.dedup();
    let kind = m.cell_kind.get(c).map_or("polyhedron", |k| k.name());
    let zone = m
        .cell_zone
        .get(c)
        .and_then(|&z| m.zones.get(z as usize))
        .cloned()
        .unwrap_or_default();
    vec![
        row("id", format!("cell:{c}")),
        row("type", kind),
        row("zone", zone),
        row("faces", m.cell_face_range(c).len()),
        row("vertices", nodes.len()),
        unit("volume", cell_volume(m, c), "m³"),
        unit("centroid", vec3(centroid, 6), "m"),
        row("neighbours", neighbours.len()),
        row(
            "neighbour_ids",
            neighbours
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        unit("non_orthogonality_max", round(worst_no, 2), "°"),
        row("skewness_max", round(worst_skew, 4)),
        row("hidden", !display.cell_visible(m, c)),
    ]
}

/// The rows of distinct face `u` (UI spec, Properties: a face).
fn face_rows(m: &Mesh, display: &Display, u: usize) -> Vec<Row> {
    if u >= m.unique_faces {
        return Vec::new();
    }
    let nodes = m.face_nodes(u);
    let (fc, s) = face_centre_area(m, nodes);
    let area = norm(s);
    let owner = m.face_owner.get(u).copied().unwrap_or(NO_CELL);
    let neighbour = m.face_neighbour.get(u).copied().unwrap_or(NO_CELL);
    let mut v = vec![row("id", format!("face:{u}"))];
    v.push(row(
        "owner",
        (owner != NO_CELL).then(|| format!("cell:{owner}")),
    ));
    if neighbour != NO_CELL {
        v.push(row("neighbour", format!("cell:{neighbour}")));
    }
    if let Some(&b) = m.face_boundary.get(u).filter(|&&b| b != NO_CELL)
        && let Some(p) = m
            .boundary_patch
            .get(b as usize)
            .and_then(|&p| m.patches.get(p as usize))
    {
        v.push(row("patch", p.name.clone()));
        v.push(row("role", p.role.name()));
    }
    v.push(unit("area", area, "m²"));
    v.push(row(
        "normal",
        (area > 0.0).then(|| vec3(s.map(|x| x / area), 4)),
    ));
    v.push(unit("centre", vec3(fc, 6), "m"));
    v.push(row("vertices", nodes.len()));
    if owner != NO_CELL {
        let oc = cell_centroid(m, owner as usize);
        let nc = (neighbour != NO_CELL).then(|| cell_centroid(m, neighbour as usize));
        let (no, skew) = face_quality(fc, s, oc, nc);
        v.push(unit("non_orthogonality", round(no, 2), "°"));
        v.push(row("skewness", skew.map(|x| round(x, 4))));
    }
    v.push(row("hidden", display.is_hidden(Item::Face(u as u32))));
    v
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

    /// Invariant: cell centroids weighted by volume give the centroid of
    /// the channel's box (the centre of its bounds), and each cell's
    /// centroid lies inside its node bounds; a face's two cells see it the
    /// same way (same non-orthogonality). Picking shows these numbers in
    /// the Properties panel; a wrong pyramid weight would move them.
    #[test]
    fn centroids_and_face_quality() {
        let m = channel();
        let (mut vol, mut acc) = (0.0, [0.0; 3]);
        for c in 0..m.cell_count() {
            let v = cell_volume(&m, c);
            let x = cell_centroid(&m, c);
            let mut b = Bounds::empty();
            for f in m.cell(c) {
                for &n in f {
                    b.add(m.points[n as usize]);
                }
            }
            assert!((0..3).all(|k| x[k] >= b.min[k] - 1e-9 && x[k] <= b.max[k] + 1e-9));
            vol += v;
            for k in 0..3 {
                acc[k] += v * x[k];
            }
        }
        let want = m.bounds().center();
        for k in 0..3 {
            assert!((acc[k] / vol - want[k]).abs() < 1e-9, "{acc:?} {vol}");
        }
        // An orthogonal pair: a unit cube's face against a centre one
        // unit away is orthogonal and unskewed; offset sideways it is not.
        let (no, sk) = face_quality(
            [0.5, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0; 3],
            Some([1.0, 0.0, 0.0]),
        );
        assert!(no.abs() < 1e-9 && sk.unwrap().abs() < 1e-9);
        let (no, sk) = face_quality(
            [0.5, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0; 3],
            Some([1.0, 1.0, 0.0]),
        );
        assert!((no - 45.0).abs() < 1e-9);
        assert!((sk.unwrap() - 0.5 / 2f64.sqrt()).abs() < 1e-9);
        // The rows of a cell and of its faces are there and labelled.
        let open = crate::session::OpenMesh {
            path: "channel.vtu".into(),
            surface: Default::default(),
            mesh: m.clone(),
            evidence: None,
            evidence_problem: None,
            digest: String::new(),
            files: Vec::new(),
            validity: Validity::NoEvidence,
        };
        let d = Display::default();
        let rows = rows(&open, &d, Item::Cell(0));
        assert_eq!(rows[0].value, json!("cell:0"));
        assert!(rows.iter().any(|r| r.key == "skewness_max"));
        let f = m.cell_face_id[m.cell_face_range(0).start];
        let rows = super::rows(&open, &d, Item::Face(f));
        assert!(rows.iter().any(|r| r.key == "owner"));
    }
}
