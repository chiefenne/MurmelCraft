//! Measuring and probing (UI spec addition, decided 2026-11-24 in the
//! thread "Measure and probe the mesh"): distances between two points,
//! angles at a vertex between two more, and probes that read the cell
//! holding a point. They are items of the model tree (`measure:<n>`,
//! `probe:<n>`), part of the undo steps and of session files.
//!
//! Every point is an [`Anchor`]: a world point and what it snapped to (a
//! node, an edge's midpoint, the point of an edge nearest the pointer, a
//! face centre, a cell centre, a point of the surface, or typed
//! coordinates). [`snap`] looks at the face under the pointer (the
//! picked triangle's face, [`crate::pick`]), projects its nodes, edge
//! midpoints and centre (and its cell's centre) to the screen, and takes
//! the first kind in [`SnapKind::SNAP`] order with a candidate within the
//! snap radius. An edge snaps to the point of the edge nearest the
//! pointer's ray (closest points of two lines). Without a candidate the
//! point of the surface under the pointer is taken.
//!
//! A probe reads the cell holding its point. A picked point names its cell
//! already; typed coordinates are located by [`locate`]: the generalised
//! winding number of each candidate cell's closed surface (Jacobson,
//! Kavan and Sorkine-Hornung, "Robust inside-outside segmentation using
//! generalized winding numbers", ACM Trans. Graph. 32(4), 2013), the sum
//! of the solid angles of the fan triangles of its faces (Van Oosterom
//! and Strackee, "The solid angle of a plane triangle", IEEE Trans.
//! Biomed. Eng. 30(2), 1983) over 4π. Every cell lists its faces outward
//! (`murmel_mesh`), so the number is one inside and zero outside; a
//! bounding box test skips most cells.

use glam::DVec3;
use murmel_mesh::{Mesh, NO_CELL};
use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::inspect::{cell_centroid, face_centre_area};
use crate::pick::pick;
use crate::surface::Surface;

/// What an anchor snapped to.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SnapKind {
    /// A mesh node (vertex).
    Node,
    /// The midpoint of an edge of a face.
    EdgeMidpoint,
    /// The centre of a face (area-weighted).
    FaceCentre,
    /// The centroid of the cell behind the face.
    CellCentre,
    /// The point of an edge nearest the pointer.
    Edge,
    /// The point of the surface under the pointer.
    Surface,
    /// Coordinates typed or given by a tool, not snapped.
    #[default]
    Typed,
}

impl SnapKind {
    /// The kinds snapping tries, in priority order.
    pub const SNAP: [SnapKind; 6] = [
        SnapKind::Node,
        SnapKind::EdgeMidpoint,
        SnapKind::FaceCentre,
        SnapKind::CellCentre,
        SnapKind::Edge,
        SnapKind::Surface,
    ];

    /// The kinds snapped to unless asked otherwise: all but cell centres,
    /// which lie inside the cell (in a view onto a face, right behind its
    /// centre).
    pub const DEFAULT: [SnapKind; 5] = [
        SnapKind::Node,
        SnapKind::EdgeMidpoint,
        SnapKind::FaceCentre,
        SnapKind::Edge,
        SnapKind::Surface,
    ];

    pub fn key(self) -> &'static str {
        match self {
            SnapKind::Node => "node",
            SnapKind::EdgeMidpoint => "edge_midpoint",
            SnapKind::FaceCentre => "face_centre",
            SnapKind::CellCentre => "cell_centre",
            SnapKind::Edge => "edge",
            SnapKind::Surface => "surface",
            SnapKind::Typed => "typed",
        }
    }
}

/// A point of a measurement or a probe: where it is and what it snapped
/// to. The point is what counts; the references say where it came from
/// (and name the probe's cell).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub point: [f64; 3],
    #[serde(default)]
    pub snap: SnapKind,
    /// The node, for a node snap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<u32>,
    /// The edge's two nodes, for an edge or midpoint snap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<[u32; 2]>,
    /// The distinct face under the pointer (`face:<n>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<u32>,
    /// The cell under the pointer, or the cell holding typed coordinates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<u32>,
}

impl Anchor {
    /// Typed coordinates, not snapped.
    pub fn typed(point: [f64; 3]) -> Self {
        Anchor {
            point,
            ..Anchor::default()
        }
    }

    /// Node `n` of `mesh`.
    pub fn node(mesh: &Mesh, n: u32) -> Option<Self> {
        Some(Anchor {
            point: *mesh.points.get(n as usize)?,
            snap: SnapKind::Node,
            node: Some(n),
            ..Anchor::default()
        })
    }

    /// The midpoint of the edge from node `a` to node `b`.
    pub fn edge_midpoint(mesh: &Mesh, a: u32, b: u32) -> Option<Self> {
        let (p, q) = (mesh.points.get(a as usize)?, mesh.points.get(b as usize)?);
        Some(Anchor {
            point: [0, 1, 2].map(|k| 0.5 * (p[k] + q[k])),
            snap: SnapKind::EdgeMidpoint,
            edge: Some([a, b]),
            ..Anchor::default()
        })
    }

    /// The centre of distinct face `u`.
    pub fn face_centre(mesh: &Mesh, u: u32) -> Option<Self> {
        let nodes = mesh.face_nodes(u as usize);
        if nodes.is_empty() {
            return None;
        }
        let owner = mesh.face_owner.get(u as usize).copied();
        Some(Anchor {
            point: face_centre_area(mesh, nodes).0,
            snap: SnapKind::FaceCentre,
            face: Some(u),
            cell: owner.filter(|&c| c != NO_CELL),
            ..Anchor::default()
        })
    }

    /// The centroid of cell `c`.
    pub fn cell_centre(mesh: &Mesh, c: u32) -> Option<Self> {
        ((c as usize) < mesh.cell_count()).then(|| Anchor {
            point: cell_centroid(mesh, c as usize),
            snap: SnapKind::CellCentre,
            cell: Some(c),
            ..Anchor::default()
        })
    }

    /// Whether the point has finite coordinates.
    pub fn is_finite(&self) -> bool {
        self.point.iter().all(|v| v.is_finite())
    }
}

/// What a measurement measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasureKind {
    /// The distance between two points.
    Distance,
    /// The angle at the second of three points.
    Angle,
}

impl MeasureKind {
    pub const ALL: [MeasureKind; 2] = [MeasureKind::Distance, MeasureKind::Angle];

    /// How many points it takes.
    pub fn points(self) -> usize {
        match self {
            MeasureKind::Distance => 2,
            MeasureKind::Angle => 3,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            MeasureKind::Distance => "distance",
            MeasureKind::Angle => "angle",
        }
    }
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

/// A distance or an angle, kept in the tree as `measure:<n>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measurement {
    /// The number of its id; never reused while the mesh is open.
    pub n: u32,
    pub kind: MeasureKind,
    /// A name given by the user; the tree shows "Distance <n>" or
    /// "Angle <n>" without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Two points for a distance; three for an angle, the vertex second.
    pub points: Vec<Anchor>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub visible: bool,
}

impl Measurement {
    fn p(&self, i: usize) -> DVec3 {
        self.points
            .get(i)
            .map_or(DVec3::ZERO, |a| DVec3::from(a.point))
    }

    /// The distance (model units) or the angle (degrees, 0 to 180).
    pub fn value(&self) -> f64 {
        match self.kind {
            MeasureKind::Distance => (self.p(1) - self.p(0)).length(),
            MeasureKind::Angle => angle_deg(self.p(0), self.p(1), self.p(2)),
        }
    }

    /// The vector from the first point to the second, for a distance.
    pub fn delta(&self) -> Option<[f64; 3]> {
        (self.kind == MeasureKind::Distance).then(|| (self.p(1) - self.p(0)).to_array())
    }

    /// Why it cannot be kept: the wrong number of points, a point not
    /// finite, or a name that `view_name` refuses.
    pub fn problem(&self) -> Option<String> {
        if self.points.len() != self.kind.points() {
            return Some(format!(
                "a {} takes {} points, not {}",
                self.kind.key(),
                self.kind.points(),
                self.points.len()
            ));
        }
        if !self.points.iter().all(Anchor::is_finite) {
            return Some("every point needs finite coordinates".into());
        }
        if let Some(n) = &self.name
            && crate::display::view_name(n).as_deref() != Some(n.as_str())
        {
            return Some(format!(
                "name {n:?} is empty, too long, untrimmed or holds control characters"
            ));
        }
        None
    }
}

/// The angle at `v` between the rays to `a` and `b`, degrees; zero when
/// a ray has no length.
pub fn angle_deg(a: DVec3, v: DVec3, b: DVec3) -> f64 {
    let (x, y) = (a - v, b - v);
    if x.length_squared() == 0.0 || y.length_squared() == 0.0 {
        return 0.0;
    }
    // atan2 of the cross and dot products stays exact near 0° and 180°,
    // where acos of the cosine loses digits.
    x.cross(y).length().atan2(x.dot(y)).to_degrees()
}

/// A point that reads the cell holding it, kept in the tree as
/// `probe:<n>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    /// The number of its id; never reused while the mesh is open.
    pub n: u32,
    /// A name given by the user; the tree shows "Probe <n>" without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The point; `at.cell` is the cell holding it (`None` outside the
    /// mesh).
    pub at: Anchor,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub visible: bool,
}

/// How far from a candidate the pointer may be, in pixels, for a snap.
pub const SNAP_RADIUS_PX: f64 = 12.0;

/// The anchor under the pointer at `ndc` (-1..1, y up) of a viewport of
/// `viewport` pixels: the first kind of `kinds`, in [`SnapKind::SNAP`]
/// order, with a candidate within `radius_px`; `None` when the pointer is
/// over empty space or no kind of `kinds` fits.
pub fn snap(
    mesh: &Mesh,
    surface: &Surface,
    camera: &Camera,
    ndc: [f64; 2],
    viewport: [f64; 2],
    kinds: &[SnapKind],
    radius_px: f64,
) -> Option<Anchor> {
    let aspect = viewport[0].max(1.0) / viewport[1].max(1.0);
    let hit = pick(surface, camera, ndc, aspect)?;
    let cell = (hit.cell != NO_CELL && (hit.cell as usize) < mesh.cell_count()).then_some(hit.cell);
    let face = (hit.face != NO_CELL).then_some(hit.face);
    let to_px = |p: [f64; 3]| -> Option<[f64; 2]> {
        let q = camera.project(surface.origin, surface.radius, aspect, p)?;
        Some([
            (q[0] + 1.0) * 0.5 * viewport[0],
            (1.0 - q[1]) * 0.5 * viewport[1],
        ])
    };
    let cursor = [
        (ndc[0] + 1.0) * 0.5 * viewport[0],
        (1.0 - ndc[1]) * 0.5 * viewport[1],
    ];
    let near = |p: [f64; 3]| -> Option<f64> {
        let s = to_px(p)?;
        let d = ((s[0] - cursor[0]).powi(2) + (s[1] - cursor[1]).powi(2)).sqrt();
        (d <= radius_px).then_some(d)
    };
    // The nearest candidate within the radius.
    let best = |cands: Vec<Anchor>| -> Option<Anchor> {
        cands
            .into_iter()
            .filter_map(|a| near(a.point).map(|d| (d, a)))
            .min_by(|x, y| x.0.total_cmp(&y.0))
            .map(|(_, a)| a)
    };
    let nodes: &[u32] = face.map_or(&[], |u| mesh.face_nodes(u as usize));
    let edges: Vec<[u32; 2]> = (0..nodes.len())
        .map(|i| [nodes[i], nodes[(i + 1) % nodes.len()]])
        .filter(|e| e[0] != e[1])
        .collect();
    let with_cell = |mut a: Anchor| {
        a.face = face;
        a.cell = cell;
        a
    };
    for &kind in SnapKind::SNAP.iter().filter(|k| kinds.contains(k)) {
        let found = match kind {
            SnapKind::Node => best(
                nodes
                    .iter()
                    .filter_map(|&n| Anchor::node(mesh, n))
                    .collect(),
            ),
            SnapKind::EdgeMidpoint => best(
                edges
                    .iter()
                    .filter_map(|e| Anchor::edge_midpoint(mesh, e[0], e[1]))
                    .collect(),
            ),
            SnapKind::FaceCentre => {
                face.and_then(|u| best(Anchor::face_centre(mesh, u).into_iter().collect()))
            }
            SnapKind::CellCentre => {
                cell.and_then(|c| best(Anchor::cell_centre(mesh, c).into_iter().collect()))
            }
            SnapKind::Edge => {
                let (start, dir) = camera.ray(ndc, aspect);
                best(
                    edges
                        .iter()
                        .filter_map(|e| {
                            let a = DVec3::from(*mesh.points.get(e[0] as usize)?);
                            let b = DVec3::from(*mesh.points.get(e[1] as usize)?);
                            let t = nearest_on_segment(start, dir, a, b);
                            Some(Anchor {
                                point: (a + (b - a) * t).to_array(),
                                snap: SnapKind::Edge,
                                edge: Some(*e),
                                ..Anchor::default()
                            })
                        })
                        .collect(),
                )
            }
            SnapKind::Surface => Some(Anchor {
                point: hit.point,
                snap: SnapKind::Surface,
                ..Anchor::default()
            }),
            SnapKind::Typed => None,
        };
        if let Some(a) = found {
            return Some(with_cell(a));
        }
    }
    None
}

/// The parameter (0..1) of the point of segment `a`..`b` nearest the line
/// through `o` along `d` (closest points of two lines, clamped).
fn nearest_on_segment(o: DVec3, d: DVec3, a: DVec3, b: DVec3) -> f64 {
    let e = b - a;
    let w = a - o;
    let (dd, de, ee) = (d.dot(d), d.dot(e), e.dot(e));
    let (dw, ew) = (d.dot(w), e.dot(w));
    let den = dd * ee - de * de;
    if ee <= 0.0 {
        return 0.0;
    }
    let t = if den.abs() <= 1e-15 * dd * ee {
        // Parallel: the foot of the line's start on the segment.
        -ew / ee
    } else {
        (de * dw - dd * ew) / den
    };
    t.clamp(0.0, 1.0)
}

/// The generalised winding number of cell `c` about `p`: about 1 inside,
/// 0 outside, 1/2 on a face (see the module notes).
pub fn winding_number(mesh: &Mesh, c: usize, p: [f64; 3]) -> f64 {
    let p = DVec3::from(p);
    let mut sum = 0.0;
    for face in mesh.cell(c) {
        let q: Vec<DVec3> = face
            .oriented()
            .filter_map(|i| mesh.points.get(i as usize))
            .map(|&x| DVec3::from(x) - p)
            .collect();
        if q.len() < 3 {
            continue;
        }
        let m = q.iter().copied().sum::<DVec3>() / q.len() as f64;
        for i in 0..q.len() {
            sum += solid_angle(m, q[i], q[(i + 1) % q.len()]);
        }
    }
    sum / (4.0 * std::f64::consts::PI)
}

/// The signed solid angle of triangle `a b c` seen from the origin (Van
/// Oosterom and Strackee 1983); positive when it turns counter-clockwise
/// seen from the origin's side its normal points away from.
fn solid_angle(a: DVec3, b: DVec3, c: DVec3) -> f64 {
    let (la, lb, lc) = (a.length(), b.length(), c.length());
    let num = a.dot(b.cross(c));
    let den = la * lb * lc + a.dot(b) * lc + a.dot(c) * lb + b.dot(c) * la;
    2.0 * num.atan2(den)
}

/// The cell of `mesh` holding `p`, if any: among the cells whose box
/// holds it, the one whose winding number is nearest one (on a face
/// between two cells, the first). Every cell is tested once (no search
/// structure yet), which is fast enough for a click or a typed point.
pub fn locate(mesh: &Mesh, p: [f64; 3]) -> Option<u32> {
    locate_where(mesh, p, |_| true)
}

/// The cell holding `p` among the cells of `mesh` that `keep` takes, as
/// [`locate`] finds it (the owned cells of blocks read with their halo:
/// a halo cell lacks some of its neighbours).
pub fn locate_where(mesh: &Mesh, p: [f64; 3], keep: impl Fn(usize) -> bool) -> Option<u32> {
    if !p.iter().all(|v| v.is_finite()) {
        return None;
    }
    let tol = 1e-9 * mesh.bounds().radius().max(f64::MIN_POSITIVE);
    let mut best: Option<(f64, u32)> = None;
    for c in (0..mesh.cell_count()).filter(|&c| keep(c)) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for i in mesh.cell_points(c) {
            {
                if let Some(q) = mesh.points.get(i as usize) {
                    for k in 0..3 {
                        lo[k] = lo[k].min(q[k]);
                        hi[k] = hi[k].max(q[k]);
                    }
                }
            }
        }
        if (0..3).any(|k| p[k] < lo[k] - tol || p[k] > hi[k] + tol) {
            continue;
        }
        let w = winding_number(mesh, c, p).abs();
        if w < 0.25 {
            continue;
        }
        let miss = (1.0 - w).abs();
        if best.is_none_or(|(b, _)| miss < b - 1e-9) {
            best = Some((miss, c as u32));
        }
        if miss < 1e-6 {
            break;
        }
    }
    best.map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::StandardView;
    use crate::display::Display;
    use crate::theme::Tokens;

    fn mesh(p: &str) -> Mesh {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
            .join(p);
        murmel_mesh::read(&p).unwrap()
    }

    /// Invariant: a distance is the length between its points and an
    /// angle the angle at its middle point, exact at 0°, 90° and 180°
    /// (an acos of the cosine would read 0.0000012° for parallel rays);
    /// the wrong number of points or a point not finite is a problem.
    #[test]
    fn values_and_problems() {
        let a = |p: [f64; 3]| Anchor::typed(p);
        let d = Measurement {
            n: 1,
            kind: MeasureKind::Distance,
            name: None,
            points: vec![a([0.0, 0.0, 0.0]), a([3.0, 4.0, 12.0])],
            visible: true,
        };
        assert_eq!(d.value(), 13.0);
        assert_eq!(d.delta(), Some([3.0, 4.0, 12.0]));
        assert!(d.problem().is_none());
        let mut ang = Measurement {
            kind: MeasureKind::Angle,
            points: vec![a([1.0, 0.0, 0.0]), a([0.0, 0.0, 0.0]), a([0.0, 2.0, 0.0])],
            ..d.clone()
        };
        assert!((ang.value() - 90.0).abs() < 1e-12);
        ang.points[2] = a([-5.0, 0.0, 0.0]);
        assert!((ang.value() - 180.0).abs() < 1e-12);
        ang.points[2] = a([7.0, 0.0, 0.0]);
        assert_eq!(ang.value(), 0.0);
        assert!(ang.delta().is_none());
        let mut bad = d.clone();
        bad.points.pop();
        assert!(bad.problem().is_some());
        bad = d.clone();
        bad.points[1].point[2] = f64::NAN;
        assert!(bad.problem().is_some());
        bad = d.clone();
        bad.name = Some(" padded ".into());
        assert!(bad.problem().is_some());
    }

    /// Invariant: every cell centroid of the channel (hexahedra, prisms
    /// and polyhedra) lies in its own cell and in no other, so `locate`
    /// finds it; a point outside the box lies in no cell. A winding
    /// number of faces turned the wrong way, or a box test that skips
    /// the cell, finds another cell or none.
    #[test]
    fn locate_finds_the_cell_of_each_centroid() {
        let m = mesh("channel/foehncraft.vtu");
        for c in 0..m.cell_count() {
            let p = cell_centroid(&m, c);
            let w = winding_number(&m, c, p);
            assert!((w.abs() - 1.0).abs() < 1e-9, "cell {c}: winding {w}");
            assert_eq!(locate(&m, p), Some(c as u32), "cell {c}");
        }
        let b = m.bounds();
        assert_eq!(locate(&m, [b.max[0] + 1.0, b.max[1], b.max[2]]), None);
        assert_eq!(locate(&m, [f64::NAN, 0.0, 0.0]), None);
        // An OpenFOAM mesh, whose neighbour faces the reader turned.
        let m = mesh("openfoam/channel_ascii");
        for c in (0..m.cell_count()).step_by(7) {
            assert_eq!(locate(&m, cell_centroid(&m, c)), Some(c as u32));
        }
    }

    /// Invariant: looking at the channel's front face from the front,
    /// the pointer over a corner node snaps to that node, over the
    /// middle of an edge to its midpoint, over the face centre to the
    /// centre, a few pixels off an edge to the edge, and elsewhere to the
    /// surface; with only `surface` allowed every point is free. The
    /// anchors name the face and cell under the pointer.
    #[test]
    fn snaps_in_priority_order() {
        let m = mesh("channel/foehncraft.vtu");
        let t = Tokens::builtin().unwrap();
        let s = Surface::build(&m, &t, &Display::default());
        let mut cam = Camera {
            projection: crate::Projection::Orthographic,
            ..Camera::default()
        };
        cam.set_standard(StandardView::FRONT);
        let vp = [800.0, 600.0];
        cam.fit(&m.bounds(), vp[0] / vp[1]);
        let aspect = vp[0] / vp[1];
        // The face under the centre of the view and its first node.
        let hit = pick(&s, &cam, [0.0, 0.0], aspect).unwrap();
        let nodes = m.face_nodes(hit.face as usize).to_vec();
        let ndc_of = |p: [f64; 3]| cam.project(s.origin, s.radius, aspect, p).unwrap();
        let all = SnapKind::DEFAULT;
        // Just inside the face from its first node, a few pixels off.
        let n0 = m.points[nodes[0] as usize];
        let fc = face_centre_area(&m, &nodes).0;
        let toward = |p: [f64; 3], q: [f64; 3], f: f64| [0, 1, 2].map(|k| p[k] + (q[k] - p[k]) * f);
        let near_node = ndc_of(toward(n0, fc, 0.02));
        let a = snap(&m, &s, &cam, near_node, vp, &all, SNAP_RADIUS_PX).unwrap();
        assert_eq!(a.snap, SnapKind::Node, "{a:?}");
        assert_eq!(a.point, n0);
        assert_eq!(a.face, Some(hit.face));
        assert_eq!(a.cell, Some(hit.cell));
        let c = snap(&m, &s, &cam, ndc_of(fc), vp, &all, SNAP_RADIUS_PX).unwrap();
        assert_eq!(c.snap, SnapKind::FaceCentre);
        let mid = Anchor::edge_midpoint(&m, nodes[0], nodes[1]).unwrap().point;
        let e = snap(
            &m,
            &s,
            &cam,
            ndc_of(toward(mid, fc, 0.02)),
            vp,
            &all,
            SNAP_RADIUS_PX,
        )
        .unwrap();
        assert_eq!(e.snap, SnapKind::EdgeMidpoint);
        assert_eq!(e.point, mid);
        // A quarter along the edge, 3 px off it towards the centre, at
        // right angles to it on the screen (orthographic, so the foot of
        // that offset on the edge is the quarter point again).
        let to_px = |n: [f64; 2]| [(n[0] + 1.0) * 0.5 * vp[0], (1.0 - n[1]) * 0.5 * vp[1]];
        let to_ndc = |p: [f64; 2]| [p[0] / vp[0] * 2.0 - 1.0, 1.0 - p[1] / vp[1] * 2.0];
        let q = toward(n0, m.points[nodes[1] as usize], 0.25);
        let (e0, e1) = (
            to_px(ndc_of(n0)),
            to_px(ndc_of(m.points[nodes[1] as usize])),
        );
        let (qp, cp) = (to_px(ndc_of(q)), to_px(ndc_of(fc)));
        let l = ((e1[0] - e0[0]).powi(2) + (e1[1] - e0[1]).powi(2)).sqrt();
        let mut perp = [-(e1[1] - e0[1]) / l, (e1[0] - e0[0]) / l];
        if perp[0] * (cp[0] - qp[0]) + perp[1] * (cp[1] - qp[1]) < 0.0 {
            perp = [-perp[0], -perp[1]];
        }
        let off = to_ndc([qp[0] + 3.0 * perp[0], qp[1] + 3.0 * perp[1]]);
        let g = snap(&m, &s, &cam, off, vp, &all, SNAP_RADIUS_PX).unwrap();
        assert_eq!(g.snap, SnapKind::Edge, "{g:?}");
        let gd: f64 = (0..3)
            .map(|k| (g.point[k] - q[k]).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!(gd < 1e-6 * m.bounds().radius(), "{g:?} vs {q:?}");
        let free = snap(
            &m,
            &s,
            &cam,
            near_node,
            vp,
            &[SnapKind::Surface],
            SNAP_RADIUS_PX,
        )
        .unwrap();
        assert_eq!(free.snap, SnapKind::Surface);
        assert_ne!(free.point, n0);
        assert!(snap(&m, &s, &cam, [0.99, 0.99], vp, &all, SNAP_RADIUS_PX).is_none());
        let cc = snap(
            &m,
            &s,
            &cam,
            ndc_of(fc),
            vp,
            &[SnapKind::CellCentre],
            SNAP_RADIUS_PX,
        );
        assert!(cc.is_none_or(|a| a.snap == SnapKind::CellCentre));
    }

    /// Invariant: the nearest point of a segment to a line is its foot,
    /// clamped to the ends; parallel lines take the foot of the line's
    /// start.
    #[test]
    fn nearest_point_of_a_segment() {
        let (o, d) = (DVec3::new(0.5, 1.0, 5.0), DVec3::new(0.0, 0.0, -1.0));
        let (a, b) = (DVec3::ZERO, DVec3::new(2.0, 0.0, 0.0));
        assert!((nearest_on_segment(o, d, a, b) - 0.25).abs() < 1e-12);
        let o2 = DVec3::new(9.0, 0.0, 5.0);
        assert_eq!(nearest_on_segment(o2, d, a, b), 1.0);
        let par = nearest_on_segment(DVec3::new(1.0, 1.0, 0.0), DVec3::X, a, b);
        assert!((par - 0.5).abs() < 1e-12);
    }
}
