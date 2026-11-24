//! What the viewport shows (UI spec, "Display" and "Model tree"): the
//! colouring, the hidden items and the selection. Items are named by ids
//! that tools and the tree share: `mesh`, `zone:<name>`,
//! `kind:<cell type>` and `patch:<name>`, and the picked `cell:<n>` and
//! `face:<n>` (a distinct face of the mesh, interior or boundary).
//!
//! The viewport draws the outside of the cells shown: a hidden patch
//! takes its boundary faces away, a hidden zone or cell type its cells, so
//! the faces of the cells behind them show (hiding the prism layers shows
//! the polyhedral core). The clip plane takes cells away the same way.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use murmel_mesh::{Bounds, CellKind, Mesh, NO_CELL};
use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::clip::Clip;
use crate::measure::{Measurement, Probe};
use crate::quality::{Metric, Quality};
use crate::theme::{Rgb, Tokens};

/// What gives a boundary face its colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorBy {
    /// One colour for the mesh (decided 2026-11-23 as the default), with
    /// the colours set on zones and patches laid over it.
    #[default]
    Solid,
    Patch,
    Role,
    CellType,
    /// The cell zone of the face's cell (OpenFOAM `cellZones`, the VTU
    /// `region`), on the patch palette.
    Zone,
    /// The cell quality metric `Display::metric`, on the quality ramp.
    Quality,
}

impl ColorBy {
    pub const ALL: [ColorBy; 6] = [
        ColorBy::Solid,
        ColorBy::Patch,
        ColorBy::Role,
        ColorBy::CellType,
        ColorBy::Zone,
        ColorBy::Quality,
    ];

    pub fn key(self) -> &'static str {
        match self {
            ColorBy::Solid => "solid",
            ColorBy::Patch => "patch",
            ColorBy::Role => "role",
            ColorBy::CellType => "cell_type",
            ColorBy::Zone => "zone",
            ColorBy::Quality => "quality",
        }
    }
}

/// How faces and edges are drawn (UI spec, Display: keys 1 to 4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    #[default]
    ShadedEdges,
    Shaded,
    /// Every edge, no faces.
    Wireframe,
    /// Only the feature edges, no faces: creases, patch borders and open
    /// borders.
    FeatureEdges,
}

impl RenderMode {
    pub const ALL: [RenderMode; 4] = [
        RenderMode::ShadedEdges,
        RenderMode::Shaded,
        RenderMode::Wireframe,
        RenderMode::FeatureEdges,
    ];

    pub fn key(self) -> &'static str {
        match self {
            RenderMode::ShadedEdges => "shaded_edges",
            RenderMode::Shaded => "shaded",
            RenderMode::Wireframe => "wireframe",
            RenderMode::FeatureEdges => "feature_edges",
        }
    }

    pub fn faces(self) -> bool {
        matches!(self, RenderMode::ShadedEdges | RenderMode::Shaded)
    }

    pub fn edges(self) -> bool {
        self != RenderMode::Shaded
    }
}

/// One item of the model tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Item {
    Mesh,
    Zone(u32),
    Kind(CellKind),
    Patch(u32),
    /// A volume cell, by index.
    Cell(u32),
    /// A distinct face (`Mesh::face_owner` index).
    Face(u32),
    /// A clip plane, by index into `Display::clips`; its id counts from
    /// one (`clip:1` is the first plane).
    Clip(u32),
    /// A distance or an angle, by its number (`measure:<n>`).
    Measure(u32),
    /// A probe, by its number (`probe:<n>`).
    Probe(u32),
}

impl Item {
    /// The item's id, for tools and the tree.
    pub fn id(self, mesh: &Mesh) -> String {
        match self {
            Item::Mesh => "mesh".into(),
            Item::Zone(z) => format!(
                "zone:{}",
                mesh.zones.get(z as usize).map_or("?", String::as_str)
            ),
            Item::Kind(k) => format!("kind:{}", k.name()),
            Item::Patch(p) => format!(
                "patch:{}",
                mesh.patches
                    .get(p as usize)
                    .map_or("?", |p| p.name.as_str())
            ),
            Item::Cell(c) => format!("cell:{c}"),
            Item::Face(f) => format!("face:{f}"),
            Item::Clip(i) => format!("clip:{}", i + 1),
            Item::Measure(n) => format!("measure:{n}"),
            Item::Probe(n) => format!("probe:{n}"),
        }
    }

    /// A picked cell or face, as opposed to an item of the tree.
    pub fn is_element(self) -> bool {
        matches!(self, Item::Cell(_) | Item::Face(_))
    }

    /// The item an id names in `mesh`.
    pub fn parse(id: &str, mesh: &Mesh) -> Option<Item> {
        if id == "mesh" {
            return Some(Item::Mesh);
        }

        let (area, name) = id.split_once(':')?;
        match area {
            "zone" => mesh
                .zones
                .iter()
                .position(|z| z == name)
                .map(|z| Item::Zone(z as u32)),
            "kind" => CellKind::ALL
                .into_iter()
                .find(|k| k.name() == name)
                .map(Item::Kind),
            "patch" => mesh
                .patches
                .iter()
                .position(|p| p.name == name)
                .map(|p| Item::Patch(p as u32)),
            "cell" => name
                .parse::<u32>()
                .ok()
                .filter(|&c| (c as usize) < mesh.cell_count())
                .map(Item::Cell),
            "face" => name
                .parse::<u32>()
                .ok()
                .filter(|&f| (f as usize) < mesh.unique_faces)
                .map(Item::Face),
            // Whether the plane exists is the display's to say
            // (`Display::has`).
            "clip" => name
                .parse::<u32>()
                .ok()
                .filter(|&n| n >= 1)
                .map(|n| Item::Clip(n - 1)),
            // Whether it exists is the display's to say, as for planes.
            "measure" => name
                .parse::<u32>()
                .ok()
                .filter(|&n| n >= 1)
                .map(Item::Measure),
            "probe" => name
                .parse::<u32>()
                .ok()
                .filter(|&n| n >= 1)
                .map(Item::Probe),
            _ => None,
        }
    }

    fn same_area(self, other: Item) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

/// What a click in the viewport picks (the Select group of the toolbar).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PickTarget {
    /// The cell under the cursor (decided 2026-11-23).
    #[default]
    Cell,
    /// The face under the cursor (a boundary face, or an interior face a
    /// cut lays open).
    Face,
}

impl PickTarget {
    pub const ALL: [PickTarget; 2] = [PickTarget::Cell, PickTarget::Face];

    pub fn key(self) -> &'static str {
        match self {
            PickTarget::Cell => "cell",
            PickTarget::Face => "face",
        }
    }
}

/// A change of visibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Show,
    Hide,
    /// Hide every other item of the same kind (other patches, zones or
    /// cell types); for picked cells and faces, show only them.
    Isolate,
    ShowAll,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Display {
    pub color_by: ColorBy,
    pub render_mode: RenderMode,
    pub hidden: BTreeSet<Item>,
    /// Cells and faces isolated (I on a picked cell or face): when not
    /// empty, only these are drawn.
    pub isolated: BTreeSet<Item>,
    /// The selected items, in the order they were picked (Shift adds);
    /// the last one fills the Properties panel.
    pub selection: Vec<Item>,
    /// What a click in the viewport picks.
    pub pick: PickTarget,
    /// Colours set on items of the open mesh (`display.set_color`). The
    /// mesh's and the zones' colours show in colour mode solid, a patch's
    /// in solid and patch, a cell type's in cell type.
    pub colors: BTreeMap<Item, Rgb>,
    /// The clip planes (`clip.set`), in the order they were added. A cell
    /// shows when every enabled plane keeps it; flat planes slice.
    pub clips: Vec<Clip>,
    /// The metric colour mode quality shows.
    pub metric: Metric,
    /// In colour mode quality, cells better than this value of the metric
    /// are hidden, so the bad cells stand out (`display.threshold`).
    pub threshold: Option<f64>,
    /// The quality of the open mesh's cells, computed the first time
    /// colour mode quality is used (`Session::redraw`).
    pub quality: Option<Arc<Quality>>,
    /// Named camera bookmarks of the open mesh (`views.save`), in the
    /// order they were saved.
    pub views: Vec<SavedView>,
    /// Distances and angles of the open mesh (`measure.add`), in the
    /// order they were made.
    pub measurements: Vec<Measurement>,
    /// Probes of the open mesh (`probe.add`), in the order they were
    /// made.
    pub probes: Vec<Probe>,
}

/// A named camera (UI spec, model tree: "saved views"). The viewport
/// size is the window's and is not part of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedView {
    pub name: String,
    pub camera: Camera,
}

/// The longest name a saved view may have, in characters.
pub const VIEW_NAME_MAX: usize = 64;

/// A view name as given, trimmed; `None` when it is empty, too long or
/// holds control characters.
pub fn view_name(name: &str) -> Option<String> {
    let n = name.trim();
    (!n.is_empty() && n.chars().count() <= VIEW_NAME_MAX && !n.chars().any(char::is_control))
        .then(|| n.to_string())
}

impl Display {
    /// The saved view called `name`.
    pub fn view(&self, name: &str) -> Option<&SavedView> {
        self.views.iter().find(|v| v.name == name)
    }

    /// The first free name "View N" (`base` is the translated "View").
    pub fn next_view_name(&self, base: &str) -> String {
        (1..)
            .map(|n| format!("{base} {n}"))
            .find(|n| self.view(n).is_none())
            .unwrap_or_default()
    }

    pub fn is_hidden(&self, item: Item) -> bool {
        self.hidden.contains(&item)
    }

    /// Measurement `measure:<n>`.
    pub fn measurement(&self, n: u32) -> Option<&Measurement> {
        self.measurements.iter().find(|m| m.n == n)
    }

    /// Probe `probe:<n>`.
    pub fn probe(&self, n: u32) -> Option<&Probe> {
        self.probes.iter().find(|p| p.n == n)
    }

    /// The number the next measurement gets: one after the highest.
    pub fn next_measure(&self) -> u32 {
        self.measurements.iter().map(|m| m.n).max().unwrap_or(0) + 1
    }

    /// The number the next probe gets: one after the highest.
    pub fn next_probe(&self) -> u32 {
        self.probes.iter().map(|p| p.n).max().unwrap_or(0) + 1
    }

    /// Remove measurements and probes named by `items`; the selection
    /// forgets them. Returns how many went.
    pub fn remove_measures(&mut self, items: &[Item]) -> usize {
        let before = self.measurements.len() + self.probes.len();
        self.measurements
            .retain(|m| !items.contains(&Item::Measure(m.n)));
        self.probes.retain(|p| !items.contains(&Item::Probe(p.n)));
        self.selection.retain(|i| !items.contains(i));
        before - self.measurements.len() - self.probes.len()
    }

    /// Apply a visibility change to `items` of `mesh`.
    pub fn set_visibility(&mut self, v: Visibility, items: &[Item], mesh: &Mesh) {
        // A clip plane's eye turns its cut on and off, a measurement's or
        // a probe's shows it; they are never in `hidden`.
        for &i in items {
            let on = match v {
                Visibility::Show | Visibility::Isolate => Some(true),
                Visibility::Hide => Some(false),
                Visibility::ShowAll => None,
            };
            match (i, on) {
                (Item::Clip(k), Some(on)) => {
                    if let Some(c) = self.clips.get_mut(k as usize) {
                        c.enabled = on;
                    }
                }
                (Item::Measure(n), Some(on)) => {
                    for m in self.measurements.iter_mut().filter(|m| m.n == n) {
                        m.visible = on;
                    }
                }
                (Item::Probe(n), Some(on)) => {
                    for p in self.probes.iter_mut().filter(|p| p.n == n) {
                        p.visible = on;
                    }
                }
                _ => {}
            }
        }
        if v == Visibility::ShowAll {
            for m in &mut self.measurements {
                m.visible = true;
            }
            for p in &mut self.probes {
                p.visible = true;
            }
        }
        let kept: Vec<Item> = items
            .iter()
            .copied()
            .filter(|i| !matches!(i, Item::Clip(_) | Item::Measure(_) | Item::Probe(_)))
            .collect();
        let items = kept.as_slice();
        match v {
            Visibility::ShowAll => {
                self.hidden.clear();
                self.isolated.clear();
            }
            Visibility::Show => {
                for i in items {
                    self.hidden.remove(i);
                }
            }
            Visibility::Hide => {
                self.hidden.extend(items.iter().copied());
                for i in items {
                    self.isolated.remove(i);
                }
            }
            Visibility::Isolate => {
                if items.contains(&Item::Mesh) {
                    self.hidden.clear();
                    self.isolated.clear();
                    return;
                }
                let (elements, items): (Vec<Item>, Vec<Item>) =
                    items.iter().partition(|i| i.is_element());
                if !elements.is_empty() {
                    for i in &elements {
                        self.hidden.remove(i);
                    }
                    self.isolated = elements.into_iter().collect();
                }
                for &i in &items {
                    self.hidden.remove(&i);
                }
                for other in all_items(mesh) {
                    if items.iter().any(|a| a.same_area(other)) && !items.contains(&other) {
                        self.hidden.insert(other);
                    }
                }
            }
        }
    }

    /// Whether `item` is selected.
    pub fn is_selected(&self, item: Item) -> bool {
        self.selection.contains(&item)
    }

    /// The item the Properties panel shows: the last one selected.
    pub fn current(&self) -> Option<Item> {
        self.selection.last().copied()
    }

    /// Select `item`, or add it to the selection (`add`; picking it again
    /// then takes it out).
    pub fn select(&mut self, item: Item, add: bool) {
        if !add {
            self.selection = vec![item];
        } else if let Some(i) = self.selection.iter().position(|&s| s == item) {
            self.selection.remove(i);
        } else {
            self.selection.retain(|&s| s != Item::Mesh);
            self.selection.push(item);
        }
    }

    /// The values of the shown metric, when colour mode quality is on and
    /// the quality is computed.
    pub fn quality_values(&self) -> Option<&[f32]> {
        (self.color_by == ColorBy::Quality)
            .then_some(self.quality.as_deref())
            .flatten()
            .map(|q| q.values(self.metric))
    }

    /// Whether the quality threshold hides cells now.
    pub fn thresholding(&self) -> bool {
        self.threshold.is_some() && self.quality_values().is_some()
    }

    /// The range of the shown metric over all cells, which the ramp and
    /// the legend span.
    pub fn quality_range(&self) -> Option<(f64, f64)> {
        if self.color_by != ColorBy::Quality {
            return None;
        }
        self.quality.as_ref()?.range(self.metric)
    }

    /// The quality colour of cell `c`, in colour mode quality.
    fn quality_color(&self, tokens: &Tokens, c: usize, range: (f64, f64)) -> Option<Rgb> {
        let x = *self.quality_values()?.get(c)? as f64;
        let t = if range.1 > range.0 {
            (x - range.0) / (range.1 - range.0)
        } else {
            0.0
        };
        Some(tokens.quality_color(t))
    }

    /// The colour of boundary face `f` of `mesh` before shading.
    pub fn face_color(&self, mesh: &Mesh, tokens: &Tokens, f: usize) -> Rgb {
        let patch = mesh.boundary_patch.get(f).copied().unwrap_or(0);
        let owner = mesh
            .boundary_owner
            .get(f)
            .copied()
            .filter(|&c| c != NO_CELL)
            .map(|c| c as usize);
        let set = |i: Item| self.colors.get(&i).copied();
        match self.color_by {
            ColorBy::Solid => set(Item::Patch(patch))
                .or_else(|| {
                    owner
                        .and_then(|c| mesh.cell_zone.get(c))
                        .and_then(|&z| set(Item::Zone(z)))
                })
                .or_else(|| set(Item::Mesh))
                .unwrap_or(tokens.mesh.solid),
            ColorBy::Patch => {
                set(Item::Patch(patch)).unwrap_or_else(|| tokens.patch_color(patch as usize))
            }
            ColorBy::Role => tokens.role_color(
                mesh.patches
                    .get(patch as usize)
                    .map_or(murmel_mesh::PatchRole::Unspecified, |p| p.role),
            ),
            ColorBy::CellType => match owner.and_then(|c| mesh.cell_kind.get(c).copied()) {
                Some(k) => set(Item::Kind(k)).unwrap_or_else(|| tokens.kind_color(k)),
                None => tokens.mesh.role.unspecified,
            },
            ColorBy::Zone => match owner.and_then(|c| mesh.cell_zone.get(c).copied()) {
                Some(z) => set(Item::Zone(z)).unwrap_or_else(|| tokens.patch_color(z as usize)),
                None => tokens.mesh.role.unspecified,
            },
            ColorBy::Quality => owner
                .zip(self.quality_range())
                .and_then(|(c, r)| self.quality_color(tokens, c, r))
                .unwrap_or(tokens.mesh.role.unspecified),
        }
    }

    /// The colour of a face of cell `c` that is not on the boundary (a
    /// face the clip or a hidden item lays open, or a section): the cell
    /// type's colour in cell-type mode, else the zone's, the mesh's or the
    /// solid colour.
    pub fn cell_color(&self, mesh: &Mesh, tokens: &Tokens, c: usize) -> Rgb {
        let set = |i: Item| self.colors.get(&i).copied();
        if let Some(q) = self
            .quality_range()
            .and_then(|r| self.quality_color(tokens, c, r))
        {
            return q;
        }
        if self.color_by == ColorBy::CellType
            && let Some(&k) = mesh.cell_kind.get(c)
        {
            return set(Item::Kind(k)).unwrap_or_else(|| tokens.kind_color(k));
        }
        if self.color_by == ColorBy::Zone
            && let Some(&z) = mesh.cell_zone.get(c)
        {
            return set(Item::Zone(z)).unwrap_or_else(|| tokens.patch_color(z as usize));
        }
        mesh.cell_zone
            .get(c)
            .and_then(|&z| set(Item::Zone(z)))
            .or_else(|| set(Item::Mesh))
            .unwrap_or(tokens.mesh.solid)
    }

    /// The index of the clip plane that tools and the viewport handle act
    /// on: the clip plane selected last, else the one added last.
    pub fn current_clip(&self) -> Option<usize> {
        self.selection
            .iter()
            .rev()
            .find_map(|&i| match i {
                Item::Clip(k) if (k as usize) < self.clips.len() => Some(k as usize),
                _ => None,
            })
            .or_else(|| self.clips.len().checked_sub(1))
    }

    /// The current clip plane (`current_clip`), on or off.
    pub fn clip(&self) -> Option<&Clip> {
        self.clips.get(self.current_clip()?)
    }

    pub fn current_clip_mut(&mut self) -> Option<&mut Clip> {
        let k = self.current_clip()?;
        self.clips.get_mut(k)
    }

    /// The current clip plane if it cuts now.
    pub fn active_clip(&self) -> Option<&Clip> {
        self.clip().filter(|c| c.enabled)
    }

    /// Every clip plane that cuts now.
    pub fn active_clips(&self) -> impl Iterator<Item = &Clip> {
        self.clips.iter().filter(|c| c.enabled)
    }

    /// Whether `item` exists in this display (clip planes do not exist
    /// in the mesh; every other item does once `Item::parse` found it).
    pub fn has(&self, item: Item) -> bool {
        match item {
            Item::Clip(k) => (k as usize) < self.clips.len(),
            Item::Measure(n) => self.measurement(n).is_some(),
            Item::Probe(n) => self.probe(n).is_some(),
            _ => true,
        }
    }

    /// Remove clip plane `k`: the selection forgets it, and the planes
    /// after it move up one (their ids with them).
    pub fn remove_clip(&mut self, k: usize) -> Option<Clip> {
        if k >= self.clips.len() {
            return None;
        }
        let c = self.clips.remove(k);
        let k = k as u32;
        self.selection.retain(|&i| i != Item::Clip(k));
        for i in &mut self.selection {
            if let Item::Clip(j) = i
                && *j > k
            {
                *j -= 1;
            }
        }
        Some(c)
    }

    /// Whether point `p` is kept by every clip plane that cuts.
    pub fn clips_keep(&self, p: [f64; 3]) -> bool {
        self.active_clips().all(|c| c.keeps(p))
    }

    /// The cells shown: not hidden (by zone, cell type or the mesh) and
    /// kept by every clip plane that cuts.
    pub fn shown_cells(&self, mesh: &Mesh) -> Vec<bool> {
        let mut shown: Vec<bool> = (0..mesh.cell_count())
            .map(|c| self.cell_visible(mesh, c))
            .collect();
        for clip in self.active_clips() {
            let keeps: Vec<bool> = mesh.points.iter().map(|&p| clip.keeps(p)).collect();
            for (s, k) in shown.iter_mut().zip(crate::clip::kept_cells(mesh, &keeps)) {
                *s &= k;
            }
        }
        shown
    }

    /// The colour `item` shows in the current colour mode, for the legend
    /// and the tree; `None` when the mode does not colour that item.
    pub fn item_color(&self, mesh: &Mesh, tokens: &Tokens, item: Item) -> Option<Rgb> {
        let set = self.colors.get(&item).copied();
        match (self.color_by, item) {
            (ColorBy::Solid, Item::Mesh) => Some(set.unwrap_or(tokens.mesh.solid)),
            (ColorBy::Solid, Item::Zone(_) | Item::Patch(_)) => set,
            (ColorBy::Patch, Item::Patch(p)) => {
                Some(set.unwrap_or_else(|| tokens.patch_color(p as usize)))
            }
            (ColorBy::Role, Item::Patch(p)) => mesh
                .patches
                .get(p as usize)
                .map(|pa| tokens.role_color(pa.role)),
            (ColorBy::CellType, Item::Kind(k)) => Some(set.unwrap_or_else(|| tokens.kind_color(k))),
            (ColorBy::Zone, Item::Zone(z)) => {
                Some(set.unwrap_or_else(|| tokens.patch_color(z as usize)))
            }
            _ => None,
        }
    }

    /// Whether the cells of zone and kind of cell `c` are shown.
    pub fn cell_visible(&self, mesh: &Mesh, c: usize) -> bool {
        if !self.isolated.is_empty() && !self.isolated.contains(&Item::Cell(c as u32)) {
            return false;
        }
        if let (Some(t), Some(v)) = (self.threshold, self.quality_values())
            && v.get(c).is_some_and(|&x| (x as f64) < t)
        {
            return false;
        }
        if self.hidden.is_empty() {
            return true;
        }
        if self.is_hidden(Item::Mesh) || self.is_hidden(Item::Cell(c as u32)) {
            return false;
        }
        let z = mesh.cell_zone.get(c).copied().unwrap_or(0);
        let k = mesh.cell_kind.get(c).copied();
        !self.is_hidden(Item::Zone(z)) && !k.is_some_and(|k| self.is_hidden(Item::Kind(k)))
    }

    /// Whether boundary face `f` is drawn.
    pub fn face_visible(&self, mesh: &Mesh, f: usize) -> bool {
        if self.hidden.is_empty() && !self.thresholding() {
            return true;
        }
        if self.is_hidden(Item::Mesh) {
            return false;
        }
        let p = mesh.boundary_patch.get(f).copied().unwrap_or(0);
        if self.is_hidden(Item::Patch(p)) {
            return false;
        }
        if let Some(&u) = mesh.boundary_face_id.get(f)
            && self.is_hidden(Item::Face(u))
        {
            return false;
        }
        match mesh.boundary_owner.get(f).copied() {
            Some(c) if c != NO_CELL => self.cell_visible(mesh, c as usize),
            _ => true,
        }
    }

    /// Whether boundary face `f` belongs to `item`.
    pub fn face_in(mesh: &Mesh, f: usize, item: Item) -> bool {
        let owner = mesh
            .boundary_owner
            .get(f)
            .copied()
            .filter(|&c| c != NO_CELL)
            .map(|c| c as usize);
        match item {
            Item::Mesh => true,
            Item::Patch(p) => mesh.boundary_patch.get(f) == Some(&p),
            Item::Zone(z) => owner.is_some_and(|c| mesh.cell_zone.get(c) == Some(&z)),
            Item::Kind(k) => owner.is_some_and(|c| mesh.cell_kind.get(c) == Some(&k)),
            Item::Cell(c) => owner == Some(c as usize),
            Item::Face(u) => mesh.boundary_face_id.get(f) == Some(&u),
            Item::Clip(_) | Item::Measure(_) | Item::Probe(_) => false,
        }
    }

    /// Whether boundary face `f` is drawn as selected.
    pub fn face_selected(&self, mesh: &Mesh, f: usize) -> bool {
        self.selection
            .iter()
            .any(|&item| item != Item::Mesh && Self::face_in(mesh, f, item))
    }

    /// The bounds of the drawn faces that belong to `item` (all drawn
    /// faces for `None`).
    pub fn bounds(&self, mesh: &Mesh, item: Option<Item>) -> Option<Bounds> {
        let mut b = Bounds::empty();
        match item {
            Some(Item::Cell(c)) if (c as usize) < mesh.cell_count() => {
                for f in mesh.cell(c as usize) {
                    for &n in f {
                        b.add(mesh.points[n as usize]);
                    }
                }
                return (!b.is_empty()).then_some(b);
            }
            Some(Item::Face(u)) => {
                for &n in mesh.face_nodes(u as usize) {
                    b.add(mesh.points[n as usize]);
                }
                return (!b.is_empty()).then_some(b);
            }
            // The plane's section of the mesh's box.
            Some(Item::Clip(i)) => {
                for p in self.clips.get(i as usize)?.box_section(&mesh.bounds()) {
                    b.add(p);
                }
                return (!b.is_empty()).then_some(b);
            }
            // The points, with room around them (a probe's cell, or a
            // twentieth of the mesh), so a short distance or a lone point
            // still fills part of the view.
            Some(Item::Measure(n)) => {
                for a in &self.measurement(n)?.points {
                    b.add(a.point);
                }
                return Some(padded(b, mesh));
            }
            Some(Item::Probe(n)) => {
                let p = self.probe(n)?;
                b.add(p.at.point);
                if let Some(c) = p.at.cell.filter(|&c| (c as usize) < mesh.cell_count()) {
                    for f in mesh.cell(c as usize) {
                        for &i in f {
                            b.add(mesh.points[i as usize]);
                        }
                    }
                }
                return Some(padded(b, mesh));
            }
            _ => {}
        }
        for (f, face) in mesh.boundary.iter().enumerate() {
            if !self.face_visible(mesh, f) || item.is_some_and(|i| !Self::face_in(mesh, f, i)) {
                continue;
            }
            for &n in face {
                b.add(mesh.points[n as usize]);
            }
        }
        (!b.is_empty()).then_some(b)
    }

    /// Cells and boundary faces shown (the clip plane included).
    pub fn visible_counts(&self, mesh: &Mesh) -> (usize, usize) {
        if self.hidden.is_empty()
            && self.isolated.is_empty()
            && self.active_clips().next().is_none()
            && !self.thresholding()
        {
            return (mesh.cell_count(), mesh.boundary.len());
        }
        let shown = self.shown_cells(mesh);
        let cells = shown.iter().filter(|&&s| s).count();
        let faces = (0..mesh.boundary.len())
            .filter(|&f| {
                self.face_visible(mesh, f)
                    && match mesh.boundary_owner.get(f).copied() {
                        Some(c) if c != NO_CELL => shown[c as usize],
                        _ => self.active_clips().all(|cl| {
                            mesh.boundary
                                .get(f)
                                .iter()
                                .any(|&i| cl.keeps(mesh.points[i as usize]))
                        }),
                    }
            })
            .count();
        (cells, faces)
    }
}

/// `b` grown on every side by a twentieth of the radius of `mesh`'s box,
/// at least.
fn padded(mut b: Bounds, mesh: &Mesh) -> Bounds {
    let pad = (0.05 * mesh.bounds().radius()).max(b.radius() * 0.1);
    for k in 0..3 {
        b.min[k] -= pad;
        b.max[k] += pad;
    }
    b
}

/// Every tree item of `mesh` below the root, in tree order: zones, the
/// cell types it has, patches.
pub fn all_items(mesh: &Mesh) -> Vec<Item> {
    let mut v: Vec<Item> = (0..mesh.zones.len() as u32).map(Item::Zone).collect();
    for k in CellKind::ALL {
        if mesh.cell_kind.contains(&k) {
            v.push(Item::Kind(k));
        }
    }
    v.extend((0..mesh.patches.len() as u32).map(Item::Patch));
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

    /// Invariant: ids round-trip, and unknown names are refused.
    #[test]
    fn item_ids_round_trip() {
        let m = channel();
        for i in std::iter::once(Item::Mesh).chain(all_items(&m)) {
            assert_eq!(Item::parse(&i.id(&m), &m), Some(i));
        }
        assert_eq!(Item::parse("patch:nope", &m), None);
        assert_eq!(
            Item::parse("kind:hexahedron", &m),
            Some(Item::Kind(CellKind::Hexahedron))
        );
        assert_eq!(Item::parse("walls", &m), None);
    }

    /// Invariant: isolating a patch hides every other patch and nothing
    /// else, so the visible faces are that patch's (walls: 256 of 426);
    /// show all brings back all 426.
    #[test]
    fn isolate_and_show_all() {
        let m = channel();
        let mut d = Display::default();
        let walls = Item::parse("patch:walls", &m).unwrap();
        d.set_visibility(Visibility::Isolate, &[walls], &m);
        assert_eq!(d.hidden.len(), 2);
        assert_eq!(d.visible_counts(&m), (776, 256));
        d.set_visibility(Visibility::Hide, &[Item::Kind(CellKind::Prism)], &m);
        let (cells, _) = d.visible_counts(&m);
        assert_eq!(cells, 8);
        d.set_visibility(Visibility::ShowAll, &[], &m);
        assert_eq!(d.visible_counts(&m), (776, 426));
    }

    /// Invariant: solid is the default mode and gives every face the solid
    /// token; a patch colour wins over the mesh colour, which wins over the
    /// token; a cell-type colour shows only in cell-type mode.
    #[test]
    fn item_colours_layer_in_order() {
        let m = channel();
        let t = Tokens::builtin().unwrap();
        let mut d = Display::default();
        assert_eq!(d.color_by, ColorBy::Solid);
        assert!((0..m.boundary.len()).all(|f| d.face_color(&m, &t, f) == t.mesh.solid));
        let (red, blue) = (Rgb([200, 0, 0]), Rgb([0, 0, 200]));
        let inlet = Item::parse("patch:inlet", &m).unwrap();
        d.colors.insert(Item::Mesh, blue);
        d.colors.insert(inlet, red);
        d.colors.insert(Item::Kind(CellKind::Prism), red);
        let mut n = [0usize; 2];
        for f in 0..m.boundary.len() {
            let c = d.face_color(&m, &t, f);
            if Display::face_in(&m, f, inlet) {
                assert_eq!(c, red);
                n[0] += 1;
            } else {
                assert_eq!(c, blue);
                n[1] += 1;
            }
        }
        assert_eq!(n, [85, 341]);
        d.color_by = ColorBy::Patch;
        assert_eq!(d.item_color(&m, &t, inlet), Some(red));
        assert_eq!(d.item_color(&m, &t, Item::Mesh), None);
        d.color_by = ColorBy::CellType;
        assert_eq!(d.item_color(&m, &t, Item::Kind(CellKind::Prism)), Some(red));
    }
}
