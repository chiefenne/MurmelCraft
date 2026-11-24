//! The tools of regions of interest (large-mesh post-processing, stage 1):
//! `region.set`, `region.clear` and `region.list`. The shapes and the
//! cells they take are the engine's ([`murmel_engine::region`]).

use murmel_engine::region::{Estimate, MAX_REGIONS, MAX_RINGS, Region, Shape};
use murmel_engine::{ClipMode, Item, Session};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{display_json, finite, need_mesh, object, parse};
use crate::{Diagnostic, Run};

fn bounds(s: &Session) -> murmel_mesh::Bounds {
    s.mesh
        .as_ref()
        .map(|m| m.mesh.bounds())
        .unwrap_or_else(murmel_mesh::Bounds::empty)
}

fn estimate_json(e: &Estimate) -> Value {
    json!({
        "cells": e.cells,
        "halo_cells": e.halo,
        "points": e.points,
        "model_mib": (e.bytes as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0,
    })
}

/// Region `k` (0-based) with its id, shape and what it takes.
pub(crate) fn region_json(s: &Session, k: usize) -> Value {
    let (Some(r), Some(m)) = (s.display.regions.get(k), &s.mesh) else {
        return Value::Null;
    };
    let estimate = s
        .display
        .region_cells
        .as_ref()
        .filter(|rc| rc.regions == s.display.regions)
        .and_then(|rc| rc.estimates.get(k))
        .map(estimate_json);
    let whole_instead = s
        .display
        .region_cells
        .as_ref()
        .filter(|rc| rc.regions == s.display.regions)
        .is_some_and(|rc| rc.whole_instead.get(k) == Some(&true));
    let mut v = json!({
        "id": Item::Region(k as u32).id(&m.mesh),
        "region": k + 1,
        "enabled": r.enabled,
        "cut": r.cut,
        "shape": r.shape,
        "takes": estimate,
        "current": s.display.current_region() == Some(k),
    });
    if whole_instead {
        v["note"] = json!(
            "no node of the mesh lies inside this sphere (it is smaller than the cells around it), so it shows whole cells; make it larger to cut it flat"
        );
    }
    v
}

/// Every region, and what the enabled ones take together (`null` while
/// none is enabled: the whole mesh shows).
pub(crate) fn regions_json(s: &Session) -> Value {
    let union = s
        .display
        .region_cells
        .as_ref()
        .filter(|rc| rc.filtering())
        .map(|rc| estimate_json(&rc.union));
    json!({
        "regions": (0..s.display.regions.len()).map(|k| region_json(s, k)).collect::<Vec<_>>(),
        "shown": union,
    })
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Box,
    Sphere,
    Slab,
    Zone,
    Patch,
    Cells,
}

impl Kind {
    fn of(shape: &Shape) -> Kind {
        match shape {
            Shape::Box { .. } => Kind::Box,
            Shape::Sphere { .. } => Kind::Sphere,
            Shape::Slab { .. } => Kind::Slab,
            Shape::Zone { .. } => Kind::Zone,
            Shape::Patch { .. } => Kind::Patch,
            Shape::Cells { .. } => Kind::Cells,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegionSet {
    region: Option<u32>,
    #[serde(default)]
    new: bool,
    shape: Option<Kind>,
    center: Option<[f64; 3]>,
    size: Option<[f64; 3]>,
    radius: Option<f64>,
    origin: Option<[f64; 3]>,
    normal: Option<[f64; 3]>,
    thickness: Option<f64>,
    zone: Option<String>,
    patch: Option<String>,
    layers: Option<u32>,
    cells: Option<Vec<u32>>,
    rings: Option<u32>,
    enabled: Option<bool>,
    cut: Option<ClipMode>,
}

/// The index of region `n` (counted from 1), or of the current region.
fn region_index(s: &Session, n: Option<u32>) -> Result<Option<usize>, Diagnostic> {
    match n {
        None => Ok(s.display.current_region()),
        Some(n) if n >= 1 && (n as usize) <= s.display.regions.len() => Ok(Some(n as usize - 1)),
        Some(n) => Err(Diagnostic::new(
            "REGION001",
            format!(
                "no region {n}; there are {} (region:1 is the first)",
                s.display.regions.len()
            ),
        )),
    }
}

fn not_for(field: &str, kind: &str) -> Diagnostic {
    Diagnostic::new(
        "TOOL002",
        format!("{field} does not apply to a {kind} region"),
    )
}

fn vec3(v: [f64; 3], name: &str) -> Result<[f64; 3], Diagnostic> {
    for x in v {
        finite(x, name)?;
    }
    Ok(v)
}

/// The cells picked now (the selection's `cell:<n>`), for a cell region
/// without `cells`.
fn picked_cells(s: &Session) -> Vec<u32> {
    s.display
        .selection
        .iter()
        .filter_map(|i| match i {
            Item::Cell(c) => Some(*c),
            _ => None,
        })
        .collect()
}

pub(crate) fn region_set(input: Value, s: &mut Session) -> Run {
    let i: RegionSet = parse(input)?;
    need_mesh(s)?;
    if i.new && i.region.is_some() {
        return Err(Diagnostic::new(
            "TOOL002",
            "give either new or region, not both",
        ));
    }
    let at = if i.new {
        None
    } else {
        region_index(s, i.region)?
    };
    if at.is_none() && s.display.regions.len() >= MAX_REGIONS {
        return Err(Diagnostic::new(
            "REGION003",
            format!("{MAX_REGIONS} regions are the most"),
        ));
    }
    let b = bounds(s);
    let old = at.map(|k| s.display.regions[k].clone());
    let kind = i
        .shape
        .or_else(|| old.as_ref().map(|r| Kind::of(&r.shape)))
        .unwrap_or(Kind::Box);
    // The shape to change: the region's own when the kind stays, else a
    // new one of that kind.
    let mut shape = match (&old, kind) {
        (Some(r), k) if Kind::of(&r.shape) == k => r.shape.clone(),
        (_, Kind::Box) => Shape::default_box(&b),
        (_, Kind::Sphere) => Shape::default_sphere(&b),
        (_, Kind::Slab) => Shape::default_slab(&b),
        (_, Kind::Zone) => Shape::Zone {
            zone: String::new(),
        },
        (_, Kind::Patch) => Shape::Patch {
            patch: String::new(),
            layers: 1,
        },
        (_, Kind::Cells) => Shape::Cells {
            cells: Vec::new(),
            rings: 0,
        },
    };
    let name = shape.kind();
    match &mut shape {
        Shape::Box { center, size } => {
            if let Some(c) = i.center {
                *center = vec3(c, "center")?;
            }
            if let Some(v) = i.size {
                *size = vec3(v, "size")?;
            }
        }
        Shape::Sphere { center, radius } => {
            if let Some(c) = i.center {
                *center = vec3(c, "center")?;
            }
            if let Some(r) = i.radius {
                *radius = finite(r, "radius")?;
            }
        }
        Shape::Slab {
            origin,
            normal,
            thickness,
        } => {
            if let Some(o) = i.origin {
                *origin = vec3(o, "origin")?;
            }
            if let Some(n) = i.normal {
                *normal = murmel_engine::clip::unit(vec3(n, "normal")?)
                    .ok_or_else(|| Diagnostic::new("TOOL002", "normal must not be zero"))?;
            }
            if let Some(t) = i.thickness {
                *thickness = finite(t, "thickness")?;
            }
        }
        Shape::Zone { zone } => {
            if let Some(z) = &i.zone {
                zone.clone_from(z);
            }
        }
        Shape::Patch { patch, layers } => {
            if let Some(p) = &i.patch {
                patch.clone_from(p);
            }
            if let Some(n) = i.layers {
                *layers = n;
            }
        }
        Shape::Cells { cells, rings } => {
            match &i.cells {
                Some(c) => cells.clone_from(c),
                None if cells.is_empty() => *cells = picked_cells(s),
                None => {}
            }
            if let Some(n) = i.rings {
                *rings = n;
            }
        }
    }
    // Fields that belong to another kind are refused, not dropped.
    let given = [
        (
            "center",
            i.center.is_some(),
            matches!(kind, Kind::Box | Kind::Sphere),
        ),
        ("size", i.size.is_some(), kind == Kind::Box),
        ("radius", i.radius.is_some(), kind == Kind::Sphere),
        ("origin", i.origin.is_some(), kind == Kind::Slab),
        ("normal", i.normal.is_some(), kind == Kind::Slab),
        ("thickness", i.thickness.is_some(), kind == Kind::Slab),
        ("zone", i.zone.is_some(), kind == Kind::Zone),
        ("patch", i.patch.is_some(), kind == Kind::Patch),
        ("layers", i.layers.is_some(), kind == Kind::Patch),
        ("cells", i.cells.is_some(), kind == Kind::Cells),
        ("rings", i.rings.is_some(), kind == Kind::Cells),
    ];
    if let Some((field, ..)) = given.iter().find(|(_, set, fits)| *set && !fits) {
        return Err(not_for(field, name));
    }
    let Some(m) = &s.mesh else {
        return Err(Diagnostic::new("VIEW001", "no mesh is open"));
    };
    if let Some(why) = shape.problem(&m.mesh) {
        let why = match &shape {
            Shape::Cells { cells, .. } if cells.is_empty() => {
                "a cell region needs cells: give cells, or pick cells first".to_string()
            }
            Shape::Zone { zone } if zone.is_empty() => {
                "a zone region needs zone, the name of a cell zone".to_string()
            }
            Shape::Patch { patch, .. } if patch.is_empty() => {
                "a patch region needs patch, the name of a patch".to_string()
            }
            _ => why,
        };
        return Err(Diagnostic::new("REGION002", why));
    }
    if i.cut == Some(ClipMode::Flat) && !shape.cuts_flat() {
        return Err(Diagnostic::new(
            "TOOL002",
            format!(
                "a {name} region has no border to cut flat; only box, sphere and slab regions cut flat"
            ),
        ));
    }
    let mut region = old.unwrap_or_else(|| {
        crate::defaults::fresh(
            s,
            murmel_engine::defaults::Kind::Region,
            Region::new(shape.clone()),
        )
    });
    region.shape = shape;
    if let Some(e) = i.enabled {
        region.enabled = e;
    }
    if let Some(c) = i.cut {
        region.cut = c;
    }
    // A region that becomes a set of cells keeps whole cells.
    if !region.shape.cuts_flat() {
        region.cut = ClipMode::Crinkle;
    }
    match at {
        Some(k) => s.display.regions[k] = region,
        None => {
            s.display
                .selection
                .retain(|i| !matches!(i, Item::Region(_)));
            s.display.regions.push(region);
        }
    }
    done(s)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Clear {
    region: Option<u32>,
    #[serde(default)]
    all: bool,
}

pub(crate) fn region_clear(input: Value, s: &mut Session) -> Run {
    let i: Clear = parse(input)?;
    if i.all {
        if i.region.is_some() {
            return Err(Diagnostic::new(
                "TOOL002",
                "give either all or region, not both",
            ));
        }
        s.display.regions.clear();
        s.display
            .selection
            .retain(|i| !matches!(i, Item::Region(_)));
    } else if let Some(k) = region_index(s, i.region)? {
        s.display.remove_region(k);
    }
    done(s)
}

pub(crate) fn region_list(_: Value, s: &mut Session) -> Run {
    need_mesh(s)?;
    Ok(regions_json(s))
}

/// Redraw (which finds the regions' cells again), then the display.
fn done(s: &mut Session) -> Run {
    s.redraw();
    Ok(display_json(s))
}

fn vec3_schema() -> Value {
    json!({"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3})
}

pub(crate) fn region_set_schema() -> Value {
    object(
        json!({
            "region": {"type": "integer", "minimum": 1},
            "new": {"type": "boolean"},
            "shape": {"enum": Shape::KINDS},
            "center": vec3_schema(),
            "size": vec3_schema(),
            "radius": {"type": "number", "exclusiveMinimum": 0},
            "origin": vec3_schema(),
            "normal": vec3_schema(),
            "thickness": {"type": "number", "exclusiveMinimum": 0},
            "zone": {"type": "string"},
            "patch": {"type": "string"},
            "layers": {"type": "integer", "minimum": 1, "maximum": MAX_RINGS},
            "cells": {"type": "array", "items": {"type": "integer", "minimum": 0}},
            "rings": {"type": "integer", "minimum": 0, "maximum": MAX_RINGS},
            "enabled": {"type": "boolean"},
            "cut": {"enum": ["crinkle", "flat"]}
        }),
        &[],
    )
}

pub(crate) fn region_clear_schema() -> Value {
    object(
        json!({
            "region": {"type": "integer", "minimum": 1},
            "all": {"type": "boolean"}
        }),
        &[],
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use murmel_engine::{Item, Session, Tokens};
    use serde_json::json;

    use crate::call;

    fn channel() -> Session {
        let mut s = Session::new(Tokens::builtin().unwrap());
        s.config_dir = None;
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/vtkhdf/channel_fields.vtkhdf");
        assert!(call("mesh.open", json!({ "path": path }), &mut s).ok());
        s
    }

    fn cells(s: &Session) -> usize {
        s.mesh.as_ref().unwrap().mesh.cell_count()
    }

    /// Invariant: an enabled region limits the cells shown to those it
    /// takes, and every triangle drawn (mesh faces, slices, iso-surfaces)
    /// belongs to one of them; turning it off or removing it shows the
    /// whole mesh again, and undo brings it back.
    #[test]
    fn a_region_limits_what_shows_and_what_filters_compute() {
        let mut s = channel();
        let all = cells(&s);
        assert_eq!(
            s.display.visible_counts(&s.mesh.as_ref().unwrap().mesh).0,
            all
        );
        let by = json!({"by": "field", "field": "pressure"});
        assert!(call("display.color_by", by, &mut s).ok());
        assert!(call("slice.set", json!({"axis": "x"}), &mut s).ok());
        assert!(call("iso.set", json!({"field": "speed"}), &mut s).ok());
        let e = call("region.set", json!({"shape": "sphere"}), &mut s);
        assert!(e.ok(), "{:?}", e.diagnostics);
        let r = &e.result["regions"]["regions"][0];
        assert_eq!(r["id"], "region:1");
        let taken = r["takes"]["cells"].as_u64().unwrap() as usize;
        assert!(taken > 0 && taken < all, "{taken} of {all}");
        assert!(r["takes"]["halo_cells"].as_u64().unwrap() > 0);
        assert_eq!(e.result["visible_cells"], json!(taken));
        let m = &s.mesh.as_ref().unwrap();
        let rc = s.display.region_cells.clone().unwrap();
        assert!(!m.surface.tri_cell.is_empty());
        for &c in &m.surface.tri_cell {
            assert!(
                c == murmel_mesh::NO_CELL || rc.contains(c as usize),
                "triangle of cell {c} outside the region"
            );
        }
        // Off: the whole mesh again, the region kept.
        let e = call("region.set", json!({"enabled": false}), &mut s);
        assert_eq!(e.result["visible_cells"], json!(all));
        assert_eq!(e.result["regions"]["shown"], json!(null));
        assert!(call("journal.undo", json!({}), &mut s).ok());
        assert_eq!(
            s.display.visible_counts(&s.mesh.as_ref().unwrap().mesh).0,
            taken
        );
        assert!(call("region.clear", json!({}), &mut s).ok());
        assert!(s.display.regions.is_empty() && s.display.region_cells.is_none());
        assert_eq!(
            s.display.visible_counts(&s.mesh.as_ref().unwrap().mesh).0,
            all
        );
    }

    /// Invariant: every shape of the first stage can be made through the
    /// tool; a zone region takes its zone, a patch region the cells along
    /// the patch, a cell region the picked cells and their rings.
    #[test]
    fn every_shape_through_the_tool() {
        let mut s = channel();
        let m = s.mesh.as_ref().unwrap().mesh.clone();
        let zone = m.zones[0].clone();
        let patch = m.patches[0].name.clone();
        let inputs = [
            json!({"new": true, "shape": "box", "center": m.bounds().center(), "size": [0.5, 0.5, 0.5]}),
            json!({"new": true, "shape": "sphere", "radius": 0.3}),
            json!({"new": true, "shape": "slab", "normal": [0, 0, 2], "thickness": 0.2}),
            json!({"new": true, "shape": "zone", "zone": zone}),
            json!({"new": true, "shape": "patch", "patch": patch, "layers": 2}),
            json!({"new": true, "shape": "cells", "cells": [0, 1], "rings": 1}),
        ];
        for input in inputs {
            let e = call("region.set", input.clone(), &mut s);
            assert!(e.ok(), "{input}: {:?}", e.diagnostics);
        }
        assert_eq!(s.display.regions.len(), 6);
        // The slab's normal is made a unit vector.
        assert_eq!(e_shape(&s, 2)["normal"], json!([0.0, 0.0, 1.0]));
        let zone_cells = m.cell_zone.iter().filter(|&&z| z == 0).count();
        let e = call("region.list", json!({}), &mut s);
        assert_eq!(e.result["regions"][3]["takes"]["cells"], json!(zone_cells));
        // A cell region from the picked cells.
        assert!(call("select.item", json!({"item": "cell:3"}), &mut s).ok());
        let e = call("region.set", json!({"new": true, "shape": "cells"}), &mut s);
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(e_shape(&s, 6)["cells"], json!([3]));
        assert_eq!(e_shape(&s, 6)["rings"], json!(0));
        // Changing a field of the current region keeps its kind.
        let e = call("region.set", json!({"region": 2, "radius": 0.4}), &mut s);
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(e_shape(&s, 1)["radius"], json!(0.4));
        assert_eq!(Item::parse("region:2", &m), Some(Item::Region(1)));
    }

    fn e_shape(s: &Session, k: usize) -> serde_json::Value {
        serde_json::to_value(&s.display.regions[k].shape).unwrap()
    }

    #[test]
    fn refusals_name_their_code() {
        let mut s = channel();
        let code = |s: &mut Session, input: serde_json::Value| {
            let e = call("region.set", input, s);
            assert!(!e.ok());
            e.diagnostics[0].code
        };
        assert_eq!(code(&mut s, json!({"region": 3})), "REGION001");
        assert_eq!(
            code(&mut s, json!({"shape": "zone", "zone": "nowhere"})),
            "REGION002"
        );
        assert_eq!(code(&mut s, json!({"shape": "zone"})), "REGION002");
        assert_eq!(code(&mut s, json!({"shape": "cells"})), "REGION002");
        assert_eq!(
            code(&mut s, json!({"shape": "box", "size": [1, 0, 1]})),
            "REGION002"
        );
        assert_eq!(
            code(&mut s, json!({"shape": "sphere", "size": [1, 1, 1]})),
            "TOOL002"
        );
        for _ in 0..murmel_engine::region::MAX_REGIONS {
            assert!(call("region.set", json!({"new": true}), &mut s).ok());
        }
        assert_eq!(code(&mut s, json!({"new": true})), "REGION003");
        assert!(call("region.clear", json!({"all": true}), &mut s).ok());
        assert!(s.display.regions.is_empty());
    }

    /// Invariant (stage 1b, card 6): a box, sphere or slab region cuts
    /// flat on request and back to whole cells, one undo step each, the
    /// cells it takes unchanged; a zone, patch or cell region refuses a
    /// flat cut (TOOL002), and a flat region that becomes a set of cells
    /// keeps whole cells. Breaks when the cut is dropped, refused for
    /// shapes that have a border, or accepted for ones that have none.
    #[test]
    fn a_region_cuts_flat_or_whole() {
        let mut s = channel();
        let e = call("region.set", json!({"shape": "box"}), &mut s);
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(e.result["regions"]["regions"][0]["cut"], "crinkle");
        let taken = e.result["visible_cells"].clone();
        let e = call("region.set", json!({"cut": "flat"}), &mut s);
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(e.result["regions"]["regions"][0]["cut"], "flat");
        assert_eq!(e.result["visible_cells"], taken);
        assert!(s.display.region_cells.as_ref().unwrap().cuts_flat());
        assert!(call("journal.undo", json!({}), &mut s).ok());
        assert_eq!(s.display.regions[0].cut, murmel_engine::ClipMode::Crinkle);
        assert!(call("journal.redo", json!({}), &mut s).ok());
        assert_eq!(s.display.regions[0].cut, murmel_engine::ClipMode::Flat);

        let zone = s.mesh.as_ref().unwrap().mesh.zones[0].clone();
        let z = json!({"new": true, "shape": "zone", "zone": zone, "cut": "flat"});
        let e = call("region.set", z, &mut s);
        assert_eq!(e.diagnostics[0].code, "TOOL002", "{:?}", e.diagnostics);
        let e = call(
            "region.set",
            json!({"region": 1, "shape": "zone", "zone": zone}),
            &mut s,
        );
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(e.result["regions"]["regions"][0]["cut"], "crinkle");
        assert!(!s.display.region_cells.as_ref().unwrap().cuts_flat());
    }

    /// Invariant (session format 1.7): regions come back from a session
    /// file with the same cells shown; a region of a zone the mesh does
    /// not have is skipped and reported.
    #[test]
    fn regions_round_trip_through_a_session() {
        let dir =
            std::env::temp_dir().join(format!("murmel-region-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("regions.murmel");
        let mut s = channel();
        let zone = s.mesh.as_ref().unwrap().mesh.zones[0].clone();
        assert!(
            call(
                "region.set",
                json!({"shape": "sphere", "radius": 0.3, "cut": "flat"}),
                &mut s
            )
            .ok()
        );
        let z = json!({"new": true, "shape": "zone", "zone": zone, "enabled": false});
        assert!(call("region.set", z, &mut s).ok());
        let e = call("session.save", json!({"path": file}), &mut s);
        assert!(e.ok(), "{:?}", e.diagnostics);
        let text = std::fs::read_to_string(&file).unwrap();
        let version = murmel_engine::session_file::SESSION_VERSION;
        assert!(
            text.contains(&format!("\"format_version\": \"{version}\"")),
            "{text}"
        );
        assert!(text.contains("\"kind\": \"sphere\""), "{text}");
        assert_eq!(text.matches("\"cut\": \"flat\"").count(), 1, "{text}");

        let mut t = Session::new(Tokens::builtin().unwrap());
        t.config_dir = None;
        let e = call("session.load", json!({"path": file}), &mut t);
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(t.display.regions, s.display.regions);
        let (ms, mt) = (
            &s.mesh.as_ref().unwrap().mesh,
            &t.mesh.as_ref().unwrap().mesh,
        );
        assert_eq!(s.display.visible_counts(ms), t.display.visible_counts(mt));

        let text = text.replace(&format!("\"zone\": \"{zone}\""), "\"zone\": \"nowhere\"");
        std::fs::write(&file, text).unwrap();
        let mut u = Session::new(Tokens::builtin().unwrap());
        u.config_dir = None;
        let e = call("session.load", json!({"path": file}), &mut u);
        assert!(e.ok(), "{:?}", e.diagnostics);
        assert_eq!(e.result["skipped"], json!(["region:2"]));
        assert_eq!(u.display.regions.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
