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

use murmel_mesh::{Bounds, CellKind, Mesh, NO_CELL};
use serde::{Deserialize, Serialize};

use crate::clip::Clip;
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
}

impl ColorBy {
    pub const ALL: [ColorBy; 4] = [
        ColorBy::Solid,
        ColorBy::Patch,
        ColorBy::Role,
        ColorBy::CellType,
    ];

    pub fn key(self) -> &'static str {
        match self {
            ColorBy::Solid => "solid",
            ColorBy::Patch => "patch",
            ColorBy::Role => "role",
            ColorBy::CellType => "cell_type",
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
    /// The clip plane, if one is set (`clip.set`).
    pub clip: Option<Clip>,
}

impl Display {
    pub fn is_hidden(&self, item: Item) -> bool {
        self.hidden.contains(&item)
    }

    /// Apply a visibility change to `items` of `mesh`.
    pub fn set_visibility(&mut self, v: Visibility, items: &[Item], mesh: &Mesh) {
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
        }
    }

    /// The colour of a face of cell `c` that is not on the boundary (a
    /// face the clip or a hidden item lays open, or a section): the cell
    /// type's colour in cell-type mode, else the zone's, the mesh's or the
    /// solid colour.
    pub fn cell_color(&self, mesh: &Mesh, tokens: &Tokens, c: usize) -> Rgb {
        let set = |i: Item| self.colors.get(&i).copied();
        if self.color_by == ColorBy::CellType
            && let Some(&k) = mesh.cell_kind.get(c)
        {
            return set(Item::Kind(k)).unwrap_or_else(|| tokens.kind_color(k));
        }
        mesh.cell_zone
            .get(c)
            .and_then(|&z| set(Item::Zone(z)))
            .or_else(|| set(Item::Mesh))
            .unwrap_or(tokens.mesh.solid)
    }

    /// The clip plane if it cuts now.
    pub fn active_clip(&self) -> Option<&Clip> {
        self.clip.as_ref().filter(|c| c.enabled)
    }

    /// The cells shown: not hidden (by zone, cell type or the mesh) and
    /// kept by the clip plane.
    pub fn shown_cells(&self, mesh: &Mesh) -> Vec<bool> {
        let mut shown: Vec<bool> = (0..mesh.cell_count())
            .map(|c| self.cell_visible(mesh, c))
            .collect();
        if let Some(clip) = self.active_clip() {
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
            _ => None,
        }
    }

    /// Whether the cells of zone and kind of cell `c` are shown.
    pub fn cell_visible(&self, mesh: &Mesh, c: usize) -> bool {
        if !self.isolated.is_empty() && !self.isolated.contains(&Item::Cell(c as u32)) {
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
        if self.hidden.is_empty() {
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
        if self.hidden.is_empty() && self.isolated.is_empty() && self.active_clip().is_none() {
            return (mesh.cell_count(), mesh.boundary.len());
        }
        let shown = self.shown_cells(mesh);
        let cells = shown.iter().filter(|&&s| s).count();
        let clip = self.active_clip();
        let faces = (0..mesh.boundary.len())
            .filter(|&f| {
                self.face_visible(mesh, f)
                    && match mesh.boundary_owner.get(f).copied() {
                        Some(c) if c != NO_CELL => shown[c as usize],
                        _ => clip.is_none_or(|cl| {
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
