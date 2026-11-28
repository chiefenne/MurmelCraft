//! Injector and nozzle devices (FoehnCraft D343 to D345, plan stage
//! ID2m): the placed surface of each device of a spray run, its hole
//! axes and the outlines of its injectors' cones, drawn with the mesh and
//! under the parcels.
//!
//! - **What.** The cells of a `spray/devices.vtu` file
//!   ([`murmel_mesh::devices`]): the surface (triangles) shaded and lit
//!   like the mesh, the hole axes and the cones in the lines that draw in
//!   every render mode, as streamlines do. A time series that lists the
//!   file under `static.devices` opens it with the series; the device
//!   does not move, so it stays while the player walks the steps.
//! - **Shading.** Each corner of a face takes the mean of the normals of
//!   the faces that meet at that point within [`CREASE`] degrees of its
//!   own face (area-weighted), so a tip drawn as an STL of many facets
//!   shades round while its edges stay sharp (the crease angle of
//!   Gouraud shading with feature edges, as in Botsch, Kobbelt, Pauly,
//!   Alliez, Lévy, "Polygon Mesh Processing", 2010, section 1.4).
//! - **Colour.** The surface takes `mesh.field.device` (or the colour
//!   set on `devices`), the lines `mesh.field.device_line`.
//! - **Which.** Each of the three parts can be hidden (`devices.set`);
//!   the clip planes drop the faces and line pieces whose middle they cut
//!   away, so a clip through the tip shows its holes.
//! - **The mesh.** While the device shows, the mesh draws as its faint
//!   outline (the tip sits on the chamber's wall, most of it outside the
//!   cells) unless `display.mesh_look` asks for its faces.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use murmel_mesh::Bounds;
use murmel_mesh::devices::{Devices, Part};
use serde::{Deserialize, Serialize};

use crate::display::{Display, Item};
use crate::surface::Surface;
use crate::theme::{Rgb, Tokens};

/// Faces that meet at a point at more than this angle (degrees) keep
/// their own normals there: an edge of the device stays sharp.
pub const CREASE: f64 = 35.0;

fn yes() -> bool {
    true
}

/// The devices shown (`devices.open`, `devices.set`, or the series'
/// `static.devices`): the file and which of its parts draw.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceView {
    /// The device file.
    pub path: PathBuf,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The placed surface (triangles).
    #[serde(default = "yes")]
    pub surface: bool,
    /// The hole axes.
    #[serde(default = "yes")]
    pub axes: bool,
    /// The outlines of the injectors' cones.
    #[serde(default = "yes")]
    pub cones: bool,
}

impl DeviceView {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            enabled: true,
            surface: true,
            axes: true,
            cones: true,
        }
    }

    /// Whether cells of `part` draw.
    pub fn shows(&self, part: Part) -> bool {
        match part {
            Part::Surface => self.surface,
            Part::Axis => self.axes,
            Part::Cone => self.cones,
        }
    }
}

/// The solid colour of the device surface.
pub fn surface_color(display: &Display, tokens: &Tokens) -> Rgb {
    display
        .colors
        .get(&Item::Devices)
        .copied()
        .unwrap_or(tokens.mesh.field.device)
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

fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let n = dot(a, a).sqrt();
    (n > 0.0 && n.is_finite()).then(|| a.map(|x| x / n))
}

/// The triangles of the surface faces (fans), each with its corners.
fn triangles(d: &Devices) -> Vec<[[f64; 3]; 3]> {
    let mut out = Vec::with_capacity(d.triangles());
    for c in d.cells.iter().filter(|c| c.part == Part::Surface) {
        let p = &d.points[c.points.clone()];
        for k in 1..p.len().saturating_sub(1) {
            out.push([p[0], p[k], p[k + 1]]);
        }
    }
    out
}

/// The normal at each corner of `tris`: the area-weighted mean of the
/// normals of the triangles at the same point (by its exact
/// coordinates, as FoehnCraft writes each triangle's corners anew)
/// within [`CREASE`] of the triangle's own. A triangle of no area takes
/// the normal of the first of its neighbours, else +z.
pub fn corner_normals(tris: &[[[f64; 3]; 3]]) -> Vec<[[f64; 3]; 3]> {
    // The area vector (twice the area, along the normal) of each one.
    let area: Vec<[f64; 3]> = tris
        .iter()
        .map(|t| cross(sub(t[1], t[0]), sub(t[2], t[0])))
        .collect();
    let key = |p: [f64; 3]| p.map(f64::to_bits);
    let mut at: HashMap<[u64; 3], Vec<u32>> = HashMap::with_capacity(tris.len() * 2);
    for (k, t) in tris.iter().enumerate() {
        for p in t {
            at.entry(key(*p)).or_default().push(k as u32);
        }
    }
    let cos = CREASE.to_radians().cos();
    tris.iter()
        .enumerate()
        .map(|(k, t)| {
            let own = unit(area[k]);
            t.map(|p| {
                let near = at.get(&key(p)).map_or(&[][..], Vec::as_slice);
                let mut sum = [0.0; 3];
                for &j in near {
                    let a = area[j as usize];
                    let keep = match (own, unit(a)) {
                        (Some(o), Some(u)) => dot(o, u) >= cos,
                        _ => false,
                    };
                    if keep {
                        sum = [sum[0] + a[0], sum[1] + a[1], sum[2] + a[2]];
                    }
                }
                unit(sum)
                    .or(own)
                    .or_else(|| near.iter().find_map(|&j| unit(area[j as usize])))
                    .unwrap_or([0.0, 0.0, 1.0])
            })
        })
        .collect()
}

/// Draw the devices of `display` into `s`: the surface into its
/// triangles, the hole axes and cones into its lines.
pub fn append(s: &mut Surface, tokens: &Tokens, display: &Display) {
    let (Some(v), Some(d)) = (display.devices.as_ref(), display.device_data.as_deref()) else {
        return;
    };
    if !v.enabled || d.cells.is_empty() {
        return;
    }
    let planes: Vec<_> = display.active_clips().copied().collect();
    let keeps = |p: [f64; 3]| planes.iter().all(|c| c.keeps(p));
    let picked = display.selection.contains(&Item::Devices);
    let sel = tokens.mesh.selection;
    let mix = tokens.mesh.selection_mix.clamp(0.0, 1.0);
    let mixed = |c: Rgb| -> Rgb {
        if picked {
            Rgb(std::array::from_fn(|k| {
                (c.0[k] as f32 * (1.0 - mix) + sel.0[k] as f32 * mix).round() as u8
            }))
        } else {
            c
        }
    };
    let o = s.origin;
    let rel = |p: [f64; 3]| -> [f32; 3] { [0, 1, 2].map(|k| (p[k] - o[k]) as f32) };
    let mut vb = s.visible_bounds.unwrap_or_else(Bounds::empty);
    if v.surface {
        let color = mixed(surface_color(display, tokens)).linear();
        let all = triangles(d);
        let normals = corner_normals(&all);
        let mut drawn = 0usize;
        for (t, n) in all.iter().zip(&normals) {
            let mid = [0, 1, 2].map(|k| (t[0][k] + t[1][k] + t[2][k]) / 3.0);
            if !keeps(mid) {
                continue;
            }
            let start = s.positions.len() as u32;
            for (p, n) in t.iter().zip(n) {
                vb.add(*p);
                s.positions.push(rel(*p));
                s.normals.push(n.map(|x| x as f32));
                s.colors.push(color);
                s.values.push([0.0; 4]);
            }
            s.indices.extend([start, start + 1, start + 2]);
            s.tri_cell.push(murmel_mesh::NO_CELL);
            s.tri_face.push(murmel_mesh::NO_CELL);
            drawn += 1;
        }
        s.device_triangles = drawn;
    }
    let line = mixed(tokens.mesh.field.device_line).linear();
    let mut segments = 0usize;
    for c in d
        .cells
        .iter()
        .filter(|c| c.part != Part::Surface && v.shows(c.part))
    {
        let p = &d.points[c.points.clone()];
        for w in p.windows(2) {
            let mid = [0, 1, 2].map(|k| (w[0][k] + w[1][k]) / 2.0);
            if !keeps(mid) {
                continue;
            }
            vb.add(w[0]);
            vb.add(w[1]);
            s.stream_positions.extend([rel(w[0]), rel(w[1])]);
            s.stream_colors.extend([line, line]);
            segments += 1;
        }
    }
    s.device_segments = segments;
    if !vb.is_empty() {
        s.visible_bounds = Some(vb);
    }
}

/// Whether the devices of `display` draw anything.
pub fn shown(display: &Display) -> bool {
    display
        .devices
        .as_ref()
        .is_some_and(|v| v.enabled && (v.surface || v.axes || v.cones))
        && display
            .device_data
            .as_ref()
            .is_some_and(|d| !d.cells.is_empty())
}

/// What the Properties panel and the device tools say of the devices:
/// the file, the counts of each part and what the last build drew.
pub fn summary(display: &Display, surface: &Surface) -> Option<serde_json::Value> {
    let v = display.devices.as_ref()?;
    let d = display.device_data.as_deref()?;
    let b = d.bounds();
    Some(serde_json::json!({
        "id": "devices",
        "path": v.path,
        "blake3": d.blake3,
        "devices": d.count(),
        "triangles": d.triangles(),
        "holes": d.holes(),
        "axes": d.cells_of(Part::Axis),
        "cones": d.cells_of(Part::Cone),
        "bounds": b.map(|b| [b.min, b.max]),
        "enabled": v.enabled,
        "show": {"surface": v.surface, "axes": v.axes, "cones": v.cones},
        "drawn": {"triangles": surface.device_triangles, "segments": surface.device_segments},
    }))
}

/// Shares the devices read once between every copy of the display.
pub fn shared(d: Devices) -> Arc<Devices> {
    Arc::new(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: corners where faces meet within the crease angle share
    /// one normal (a facetted cylinder shades round), corners across a
    /// sharper edge keep their face's (the end face of the tip stays
    /// flat). Breaks if every corner took its face normal (the tip would
    /// draw as facets) or the normals of a cap and a side were averaged
    /// (its rim would look soft). No other test looks at the shading.
    #[test]
    fn crease_angle_normals() {
        // Two faces 20 degrees apart sharing an edge, and one at 90.
        let a = [0.0, 0.0, 0.0];
        let b = [1.0, 0.0, 0.0];
        let t = 20f64.to_radians();
        let tris = [
            [a, b, [0.0, 1.0, 0.0]],
            [b, a, [0.0, -t.cos(), t.sin()]],
            [a, b, [0.0, 0.0, -1.0]],
        ];
        let n = corner_normals(&tris);
        // The first two meet at 160 degrees between their planes, their
        // normals 20 degrees apart: one mean normal at the shared corners.
        let close = |p: [f64; 3], q: [f64; 3]| (0..3).all(|k| (p[k] - q[k]).abs() < 1e-12);
        assert!(close(n[0][0], n[1][1]), "{:?} {:?}", n[0][0], n[1][1]);
        assert!(n[0][0][2] > 0.98 && n[0][0][1] > 0.1);
        // The corner of the first face away from the edge is its own.
        assert!(close(n[0][2], [0.0, 0.0, 1.0]));
        // The third face is 90 degrees from the first: its own normal.
        assert!(close(n[2][0], [0.0, 1.0, 0.0]));
    }
}
