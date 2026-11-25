//! The outside of the cells shown, as triangles and edge lines ready for
//! the GPU. Built once per mesh and display (colouring, hidden items,
//! clip plane, selection); orbiting never re-triangulates.
//!
//! What is drawn: the boundary faces of the cells shown (in their patch's
//! colour), the faces between a cell shown and one that is not (a hidden
//! zone or cell type, or the clip plane, lays them open; they take the
//! cell's colour), and with a flat clip the section of every cell the
//! plane crosses. So hiding the prism layers shows the polyhedral core,
//! and a crinkle clip shows the cells themselves. A region that cuts flat
//! clips the cells it cuts to its borders in the same way.
//!
//! Each polygon is a fan about its node average (exact for the
//! star-shaped faces KoflCraft writes, including merged faces, and for
//! the convex sections), with one flat normal from Newell's method.
//! Positions are relative to `origin` (the centre of the bounds) so they
//! keep their precision in `f32`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use murmel_mesh::fasthash::{FastMap, FastSet, set_with_capacity};

use murmel_mesh::{Bounds, Mesh, NO_CELL};

use crate::clip::ClipMode;
use crate::display::{Display, Item, RenderMode};
use crate::field::FieldSet;
use crate::region::{Bound, NOT_CUT, RegionCells};

/// Feature edges (render mode 4) bend by more than this angle between
/// their two faces (UI spec, Display).
pub const FEATURE_ANGLE_DEG: f64 = 30.0;
use crate::theme::{Rgb, Tokens};

/// The feature-edge group of a face that is on no patch (laid open by the
/// clip or a hidden item), and of a section.
const GROUP_OPEN: u32 = u32::MAX - 1;
const GROUP_SECTION: u32 = u32::MAX - 2;

/// Triangles per pick box ([`Surface::tri_boxes`]).
pub const PICK_RUN: usize = 128;

/// Which build a surface's own faces come from: a new key each time
/// [`Surface::build_with`] builds them, kept when only the slices,
/// iso-surfaces, arrows, streamlines, parcels and block boxes are built
/// again ([`Surface::rebuild_extras`]), so a renderer uploads only those.
/// It names a build, not what was built, so it never makes two surfaces
/// differ.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildKey(pub u64);

impl PartialEq for BuildKey {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl BuildKey {
    fn next() -> BuildKey {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        BuildKey(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

/// Where the slices, iso-surfaces, arrows, streamline tubes, parcels and
/// block boxes start in a surface's buffers (everything before is the
/// mesh's own), and what the mesh's own faces gave of the bounds and the
/// field range before them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExtrasAt {
    pub vertices: usize,
    pub indices: usize,
    pub edges: usize,
    visible_bounds: Option<Bounds>,
    field_range: Option<(f64, f64)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Surface {
    /// The build of the mesh's own faces ([`BuildKey`]).
    pub key: BuildKey,
    /// Where the extras start ([`ExtrasAt`]).
    pub extras_at: ExtrasAt,
    pub origin: [f64; 3],
    pub radius: f64,
    pub bounds: Option<Bounds>,
    /// Bounds of the faces drawn (hidden items left out).
    pub visible_bounds: Option<Bounds>,
    /// Boundary faces drawn, faces laid open (by the clip plane or hidden
    /// items, sections included), and the cells shown (counted once per
    /// build so the status bar does not walk the mesh every frame).
    pub faces: usize,
    pub open_faces: usize,
    pub cells: usize,
    /// Triangle vertices: position, normal, linear colour.
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 3]>,
    /// Per vertex, for colour mode field: the cell value (colour mode
    /// flat), the node value (smooth and banded), both relative to
    /// `value_origin`, the share of the selection colour to mix in, and
    /// 1 when the face has a value (0: it keeps `colors`).
    pub values: Vec<[f32; 4]>,
    pub value_origin: f64,
    /// The smallest and largest field value drawn (range mode shown).
    pub field_range: Option<(f64, f64)>,
    pub indices: Vec<u32>,
    /// Per triangle: the cell it shows (the owner of a boundary face, the
    /// cell shown beside an open face, the cell of a section;
    /// [`NO_CELL`] when unknown) and the distinct face (`face:<n>`, or
    /// [`NO_CELL`] for a section). Picking reads these.
    pub tri_cell: Vec<u32>,
    pub tri_face: Vec<u32>,
    /// The box (smallest and largest corner, relative to `origin`) of
    /// each run of [`PICK_RUN`] triangles in `indices`, so a pick tests
    /// only the runs its ray passes through ([`crate::pick`]). Triangles
    /// come in face order, so a run covers a small patch of the surface.
    pub tri_boxes: Vec<[[f32; 3]; 2]>,
    /// Edge line vertices, two per edge, with their linear colour.
    pub edge_positions: Vec<[f32; 3]>,
    pub edge_colors: Vec<[f32; 3]>,
    /// The outline of the picked cells and faces (line pairs), drawn over
    /// everything in `outline_color` (linear).
    pub outline: Vec<[f32; 3]>,
    pub outline_color: [f32; 3],
    /// Vector arrows drawn, and whether more were wanted than drawn
    /// ([`crate::glyph::MAX_ARROWS`]).
    pub arrows: usize,
    pub arrows_capped: bool,
    /// The magnitude an arrow 0.9 of a spacing long stands for (times
    /// the scale); 0 without arrows.
    pub arrow_longest: f64,
    /// The smallest and largest magnitude of the arrows drawn (the
    /// colour range of arrows coloured by their magnitude, range mode
    /// shown).
    pub arrow_range: Option<(f64, f64)>,
    /// Lines drawn in every render mode, hidden behind faces (line
    /// pairs, relative to `origin`), with their linear colours: the
    /// streamlines, then the boxes of an overview's blocks.
    pub stream_positions: Vec<[f32; 3]>,
    pub stream_colors: Vec<[f32; 3]>,
    /// The boxes of an overview's blocks drawn ([`crate::block_boxes`]).
    pub block_boxes: usize,
    /// Spray parcels drawn, and whether more passed the filters than
    /// were drawn ([`crate::parcel::MAX_DRAWN`]).
    pub parcels: usize,
    pub parcels_capped: bool,
    /// The smallest and largest colour value of the parcels drawn.
    pub parcel_range: Option<(f64, f64)>,
}

/// A corner of a drawn polygon: a mesh node, or a point where a clip
/// plane crosses an edge (`Cutter::cut`, numbered as found).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum V {
    Node(u32),
    Cut(u32),
}

/// One polygon to draw.
struct Poly {
    verts: Vec<V>,
    cell: u32,
    face: u32,
    boundary: u32,
    color: Rgb,
    group: u32,
}

/// A border that cuts: a clip plane, with the side and distance of every
/// node, or a border of a region cutting flat, whose distances are found
/// as needed (it cuts only the cells along it).
struct Plane {
    bound: Bound,
    /// Whether a clip plane cuts flat (region borders always do).
    flat: bool,
    nodes: Option<(Vec<bool>, Vec<f64>)>,
}

/// The planes of a [`Cutter`] and the sides of the mesh nodes, without
/// the cut points it numbers as it finds them: shared by threads.
#[derive(Clone, Copy)]
struct Sides<'a> {
    points: &'a [[f64; 3]],
    planes: &'a [Plane],
    flat: &'a [usize],
    regions: Option<&'a (&'a RegionCells, Vec<std::ops::Range<usize>>)>,
}

impl<'a> Sides<'a> {
    /// The planes that slice cell `c` (`NO_CELL`: the clip planes only),
    /// in order.
    fn flat_of(self, c: u32) -> impl Iterator<Item = usize> + 'a {
        let region = self.regions.and_then(|(rc, ranges)| {
            let k = *rc.cut_by.as_ref()?.get(c as usize)?;
            (k != NOT_CUT).then(|| ranges[k as usize].clone())
        });
        self.flat
            .iter()
            .copied()
            .chain(region.into_iter().flatten())
    }

    /// Whether plane `p` keeps node `i` (a node on the plane counts as
    /// kept).
    fn keeps_node(self, p: usize, i: u32) -> bool {
        let plane = &self.planes[p];
        match &plane.nodes {
            Some((keep, _)) => keep[i as usize],
            None => plane.bound.distance(self.points[i as usize]) <= 0.0,
        }
    }

    /// What the planes slicing cell `c` do to a polygon of it with the
    /// mesh nodes `verts`, as [`Cutter::clip_flat`] would: leave it whole
    /// (every plane keeps every node), drop it (the first plane that does
    /// not keep them all keeps none), or cut it (that plane keeps some),
    /// which only the cutter can do.
    fn fate(self, c: u32, verts: &[V]) -> Fate {
        for p in self.flat_of(c) {
            let kept = verts
                .iter()
                .filter(|&&v| match v {
                    V::Node(i) => self.keeps_node(p, i),
                    V::Cut(_) => false,
                })
                .count();
            if kept == 0 {
                return Fate::Drop;
            }
            if kept < verts.len() || verts.iter().any(|v| matches!(v, V::Cut(_))) {
                return Fate::Cut;
            }
        }
        Fate::Whole
    }

    /// Whether a plane that slices cell `c` cuts away a node of it: the
    /// cells with a section.
    fn crosses(self, mesh: &Mesh, c: usize) -> bool {
        self.flat_of(c as u32)
            .any(|p| mesh.cell_points(c).any(|i| !self.keeps_node(p, i)))
    }
}

/// What the flat planes do to a polygon (see [`Sides::fate`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fate {
    Whole,
    Drop,
    Cut,
}

/// The cut points found so far: where each is, which plane made it, and
/// its name (plane, edge).
#[derive(Default)]
struct Cuts {
    pos: Vec<[f64; 3]>,
    plane: Vec<usize>,
    ids: FastMap<(usize, V, V), u32>,
    /// The edge each cut point lies on and where along it (0 at the
    /// first corner), so values interpolate like positions.
    along: Vec<(V, V, f64)>,
}

/// The clip planes that cut, and the borders of regions that cut flat.
/// A point where plane `p` crosses the edge between corners `a` and `b`
/// is named by `(p, a, b)`, so the cuts of neighbouring faces and cells
/// meet at the same named points and the sections close. Flat planes
/// slice in the order they were added, each one the polygons the ones
/// before it left, so a corner where two planes meet a face has one name
/// too; a cell a region cuts is then sliced by that region's borders.
struct Cutter<'a> {
    points: &'a [[f64; 3]],
    planes: Vec<Plane>,
    /// The clip planes are `planes[..clips]`.
    clips: usize,
    /// The flat clip planes, by index into `planes`, in order.
    flat: Vec<usize>,
    /// Which region cuts each cell, and each region's borders, by index
    /// into `planes`.
    regions: Option<(&'a RegionCells, Vec<std::ops::Range<usize>>)>,
    cuts: RefCell<Cuts>,
}

impl<'a> Cutter<'a> {
    /// The planes of `display` that cut, or `None` when none does.
    fn new(mesh: &'a Mesh, display: &'a Display) -> Option<Cutter<'a>> {
        let mut planes: Vec<Plane> = display
            .active_clips()
            .map(|clip| {
                let points = &mesh.points;
                let mut dist = vec![0.0; points.len()];
                murmel_mesh::par::chunks_mut(&mut dist, 1 << 16, |at, part| {
                    for (d, &p) in part.iter_mut().zip(&points[at..]) {
                        *d = clip.distance(p);
                    }
                });
                let mut keeps = vec![false; points.len()];
                murmel_mesh::par::chunks_mut(&mut keeps, 1 << 16, |at, part| {
                    for (k, &d) in part.iter_mut().zip(&dist[at..]) {
                        *k = d <= 0.0;
                    }
                });
                Plane {
                    bound: Bound::from(clip),
                    flat: clip.mode == ClipMode::Flat,
                    nodes: Some((keeps, dist)),
                }
            })
            .collect();
        let clips = planes.len();
        let flat = (0..clips).filter(|&p| planes[p].flat).collect();
        let regions = display
            .region_cells
            .as_deref()
            .filter(|rc| rc.cuts_flat() && rc.regions == display.regions)
            .map(|rc| {
                let ranges = rc
                    .borders
                    .iter()
                    .map(|bs| {
                        let start = planes.len();
                        planes.extend(bs.iter().map(|&bound| Plane {
                            bound,
                            flat: true,
                            nodes: None,
                        }));
                        start..planes.len()
                    })
                    .collect();
                (rc, ranges)
            });
        if planes.is_empty() {
            return None;
        }
        Some(Cutter {
            points: &mesh.points,
            planes,
            clips,
            flat,
            regions,
            cuts: RefCell::default(),
        })
    }

    /// Whether anything cuts flat.
    fn slices(&self) -> bool {
        !self.flat.is_empty() || self.regions.is_some()
    }

    /// The planes that slice cell `c` (`NO_CELL`: the clip planes only),
    /// in order.
    fn flat_of(&self, c: u32) -> impl Iterator<Item = usize> + '_ {
        self.sides().flat_of(c)
    }

    /// What the cutter knows before it cuts anything, for the threads.
    fn sides(&self) -> Sides<'_> {
        Sides {
            points: self.points,
            planes: &self.planes,
            flat: &self.flat,
            regions: self.regions.as_ref(),
        }
    }

    /// Whether every clip plane keeps a node of `face`.
    fn keeps_some(&self, face: &[u32]) -> bool {
        self.planes[..self.clips].iter().all(|pl| {
            pl.nodes
                .as_ref()
                .is_none_or(|(keep, _)| face.iter().any(|&i| keep[i as usize]))
        })
    }

    /// The cutting with the cut points numbered as they are found.
    fn view(&self) -> View<'_, RefCell<Cuts>> {
        View {
            sides: self.sides(),
            store: &self.cuts,
        }
    }

    fn pos(&self, v: V) -> [f64; 3] {
        self.view().pos(v)
    }

    fn value(&self, v: V, node: &[f64]) -> f64 {
        self.view().value(v, node)
    }

    fn clip_flat(&self, cell: u32, verts: Vec<V>) -> Vec<V> {
        self.view().clip_flat(cell, verts).unwrap_or_default()
    }

    fn sections(&self, mesh: &Mesh, c: usize) -> Vec<Vec<V>> {
        self.view().sections(mesh, c).unwrap_or_default()
    }

    /// The only plane that slices cell `c`, if one alone does.
    fn only_plane(&self, c: u32) -> Option<usize> {
        let mut planes = self.flat_of(c);
        match (planes.next(), planes.next()) {
            (Some(p), None) => Some(p),
            _ => None,
        }
    }

    /// Number the points where plane `p`, the only one slicing cell `c`,
    /// crosses the edges of its faces, in the order its section finds
    /// them (face by face, edge by edge; see [`View::ring`]), so the
    /// section can then be built on any thread, reading them.
    fn cut_edges(&self, mesh: &Mesh, p: usize, c: usize) {
        let view = self.view();
        for f in mesh.cell(c) {
            let face: Vec<V> = f.oriented().map(V::Node).collect();
            let n = face.len();
            for i in 0..n {
                let (a, b) = (face[i], face[(i + 1) % n]);
                if view.keeps(p, a) != view.keeps(p, b) {
                    view.cut(p, a, b);
                }
            }
        }
    }
}

/// Where the cut points are kept: numbered as they are found (on one
/// thread, the cutter's own), or, once all are found, only read (on all
/// threads).
trait Store {
    fn id(&self, key: &(usize, V, V)) -> Option<u32>;
    fn pos(&self, k: u32) -> [f64; 3];
    fn plane(&self, k: u32) -> usize;
    fn along(&self, k: u32) -> Option<(V, V, f64)>;
    /// A new cut point, numbered next; `None` from a store that only
    /// reads.
    fn add(&self, key: (usize, V, V), at: [f64; 3], along: (V, V, f64)) -> Option<u32>;
}

impl Store for RefCell<Cuts> {
    fn id(&self, key: &(usize, V, V)) -> Option<u32> {
        self.borrow().ids.get(key).copied()
    }

    fn pos(&self, k: u32) -> [f64; 3] {
        self.borrow().pos[k as usize]
    }

    fn plane(&self, k: u32) -> usize {
        self.borrow().plane[k as usize]
    }

    fn along(&self, k: u32) -> Option<(V, V, f64)> {
        self.borrow().along.get(k as usize).copied()
    }

    fn add(&self, key: (usize, V, V), at: [f64; 3], along: (V, V, f64)) -> Option<u32> {
        let mut c = self.borrow_mut();
        let k = c.pos.len() as u32;
        c.pos.push(at);
        c.plane.push(key.0);
        c.ids.insert(key, k);
        c.along.push(along);
        Some(k)
    }
}

impl Store for Cuts {
    fn id(&self, key: &(usize, V, V)) -> Option<u32> {
        self.ids.get(key).copied()
    }

    fn pos(&self, k: u32) -> [f64; 3] {
        self.pos[k as usize]
    }

    fn plane(&self, k: u32) -> usize {
        self.plane[k as usize]
    }

    fn along(&self, k: u32) -> Option<(V, V, f64)> {
        self.along.get(k as usize).copied()
    }

    fn add(&self, _: (usize, V, V), _: [f64; 3], _: (V, V, f64)) -> Option<u32> {
        None
    }
}

/// The cutting of polygons and cells by the planes of `sides`, with the
/// cut points in `store`.
struct View<'a, S> {
    sides: Sides<'a>,
    store: &'a S,
}

impl<S> Clone for View<'_, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for View<'_, S> {}

impl<'a, S: Store> View<'a, S> {
    fn flat_of(self, c: u32) -> impl Iterator<Item = usize> + 'a {
        self.sides.flat_of(c)
    }

    fn pos(self, v: V) -> [f64; 3] {
        match v {
            V::Node(i) => self.sides.points[i as usize],
            V::Cut(k) => self.store.pos(k),
        }
    }

    fn dist(self, p: usize, v: V) -> f64 {
        let plane = &self.sides.planes[p];
        match (v, &plane.nodes) {
            (V::Node(i), Some((_, dist))) => dist[i as usize],
            _ => plane.bound.distance(self.pos(v)),
        }
    }

    /// Whether plane `p` keeps corner `v` (a point on the plane counts as
    /// kept, and so does every cut point the plane made).
    fn keeps(self, p: usize, v: V) -> bool {
        match (v, &self.sides.planes[p].nodes) {
            (V::Node(i), Some((keep, _))) => keep[i as usize],
            (V::Node(_), None) => self.dist(p, v) <= 0.0,
            (V::Cut(k), _) => self.store.plane(k) == p || self.dist(p, v) <= 0.0,
        }
    }

    /// The point where plane `p` crosses the edge from `a` to `b`;
    /// `None` when it is not numbered yet and the store only reads.
    fn cut(self, p: usize, a: V, b: V) -> Option<V> {
        let key = (p, a.min(b), a.max(b));
        if let Some(k) = self.store.id(&key) {
            return Some(V::Cut(k));
        }
        let (a, b) = (key.1, key.2);
        let (da, db) = (self.dist(p, a), self.dist(p, b));
        let t = if da == db { 0.0 } else { da / (da - db) };
        let (pa, pb) = (self.pos(a), self.pos(b));
        let at = [0, 1, 2].map(|k| pa[k] + t * (pb[k] - pa[k]));
        self.store.add(key, at, (a, b, t)).map(V::Cut)
    }

    /// The value at corner `v` of a field with `node` values: a node's
    /// own, a cut point's mixed from the ends of its edge.
    fn value(self, v: V, node: &[f64]) -> f64 {
        match v {
            V::Node(i) => node.get(i as usize).copied().unwrap_or(f64::NAN),
            V::Cut(k) => match self.store.along(k) {
                Some((a, b, t)) => {
                    let (va, vb) = (self.value(a, node), self.value(b, node));
                    va + t * (vb - va)
                }
                None => f64::NAN,
            },
        }
    }

    /// The part of `poly` that plane `p` keeps (Sutherland and Hodgman).
    fn clip_poly(self, p: usize, verts: &[V]) -> Option<Vec<V>> {
        let mut out = Vec::with_capacity(verts.len() + 2);
        for i in 0..verts.len() {
            let (a, b) = (verts[i], verts[(i + 1) % verts.len()]);
            let (ka, kb) = (self.keeps(p, a), self.keeps(p, b));
            if ka {
                out.push(a);
            }
            if ka != kb {
                out.push(self.cut(p, a, b)?);
            }
        }
        Some(out)
    }

    /// The part of `poly`, a polygon of cell `cell`, that every plane
    /// slicing that cell keeps, cut in their order.
    fn clip_flat(self, cell: u32, verts: Vec<V>) -> Option<Vec<V>> {
        let mut v = verts;
        for p in self.flat_of(cell) {
            if v.len() < 3 {
                break;
            }
            // Most polygons lie wholly on the kept side: left as they are.
            if v.iter().all(|&x| self.keeps(p, x)) {
                continue;
            }
            v = self.clip_poly(p, &v)?;
        }
        Some(v)
    }

    /// The section of the closed polygons `polys` by plane `p`: closed
    /// loops of cut points. In each polygon the kept part runs from the
    /// cut where it leaves the kept side to the next cut where it comes
    /// back; those segments, joined at the shared cut points, close
    /// around the cell.
    fn ring(self, p: usize, polys: &[Vec<V>]) -> Option<Vec<Vec<V>>> {
        let mut next: HashMap<V, Vec<V>> = HashMap::new();
        for face in polys {
            let n = face.len();
            let mut cuts: Vec<(V, bool)> = Vec::new();
            for i in 0..n {
                let (a, b) = (face[i], face[(i + 1) % n]);
                let (ka, kb) = (self.keeps(p, a), self.keeps(p, b));
                if ka != kb {
                    cuts.push((self.cut(p, a, b)?, ka));
                }
            }
            for (i, &(v, leaving)) in cuts.iter().enumerate() {
                if !leaving {
                    continue;
                }
                if let Some(&(w, _)) = (1..cuts.len())
                    .map(|j| &cuts[(i + j) % cuts.len()])
                    .find(|(_, l)| !l)
                {
                    next.entry(v).or_default().push(w);
                    next.entry(w).or_default().push(v);
                }
            }
        }
        let mut loops = Vec::new();
        let mut used: HashSet<V> = HashSet::new();
        let mut starts: Vec<V> = next.keys().copied().collect();
        starts.sort_unstable();
        for start in starts {
            if used.contains(&start) {
                continue;
            }
            let mut ring = vec![start];
            used.insert(start);
            let mut cur = start;
            while let Some(&n) = next
                .get(&cur)
                .and_then(|ns| ns.iter().find(|n| !used.contains(n)))
            {
                ring.push(n);
                used.insert(n);
                cur = n;
            }
            if ring.len() >= 3 {
                loops.push(ring);
            }
        }
        Some(loops)
    }

    /// The sections of cell `c` by the flat planes: the cell's faces are
    /// cut by each plane in turn, and each plane's section joins the
    /// polygons the next one cuts (convex clipping of a polyhedron by
    /// half-spaces, one plane at a time).
    fn sections(self, mesh: &Mesh, c: usize) -> Option<Vec<Vec<V>>> {
        if !self.sides.crosses(mesh, c) {
            return Some(Vec::new());
        }
        let planes: Vec<usize> = self.flat_of(c as u32).collect();
        let mut set: Vec<(Vec<V>, bool)> = mesh
            .cell(c)
            .map(|f| (f.oriented().map(V::Node).collect(), false))
            .collect();
        for p in planes {
            let polys: Vec<Vec<V>> = set.iter().map(|(v, _)| v.clone()).collect();
            if !polys.iter().flatten().any(|&v| !self.keeps(p, v)) {
                continue;
            }
            let rings = self.ring(p, &polys)?;
            let mut cut = Vec::with_capacity(set.len() + rings.len());
            for (v, s) in set {
                let v = self.clip_poly(p, &v)?;
                if v.len() >= 3 {
                    cut.push((v, s));
                }
            }
            cut.extend(rings.into_iter().map(|r| (r, true)));
            set = cut;
        }
        Some(
            set.into_iter()
                .filter(|(_, s)| *s)
                .map(|(v, _)| v)
                .collect(),
        )
    }
}

impl Surface {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn edges(&self) -> usize {
        self.edge_positions.len() / 2
    }

    /// Triangulate the outside of the cells of `mesh` that `display`
    /// shows, in the colours it asks for, with its slices (iso-surfaces
    /// need the fields: [`Surface::build_with`]).
    pub fn build(mesh: &Mesh, tokens: &Tokens, display: &Display) -> Surface {
        Self::build_with(mesh, None, tokens, display)
    }

    /// [`Surface::build`] with the slices and iso-surfaces of `display`
    /// ([`crate::extract`]) drawn into the same buffers.
    pub fn build_with(
        mesh: &Mesh,
        fields: Option<&FieldSet>,
        tokens: &Tokens,
        display: &Display,
    ) -> Surface {
        let bounds = mesh.bounds();
        let origin = bounds.center();
        let mut s = Surface {
            origin,
            radius: bounds.radius(),
            bounds: (!bounds.is_empty()).then_some(bounds),
            ..Surface::default()
        };
        let phase = murmel_mesh::profile::span("surface.polygons");
        let cutter = Cutter::new(mesh, display);
        let shown = display.shown_cells(mesh);
        s.cells = shown.iter().filter(|&&v| v).count();
        let polys = polygons(mesh, tokens, display, &shown, cutter.as_ref());
        drop(phase);
        let phase = murmel_mesh::profile::span("surface.points");

        let pos = |v: V| -> [f64; 3] {
            let p = match (v, &cutter) {
                (V::Node(i), _) => mesh.points[i as usize],
                (_, Some(c)) => c.pos(v),
                (V::Cut(_), None) => [0.0; 3],
            };
            [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]]
        };
        let darken = 1.0 - tokens.mesh.edge_darken;
        let sel = tokens.mesh.selection;
        let mix = tokens.mesh.selection_mix.clamp(0.0, 1.0);
        // Selected faces first, so the edges they share with others take
        // the selection colour.
        // Then boundary faces in file order, then open faces: an edge two
        // faces share takes the colour of the first.
        let mut order: Vec<(bool, u32, usize)> = polys
            .iter()
            .enumerate()
            .map(|(i, p)| (!selected(mesh, display, p), p.boundary, i))
            .collect();
        order.sort_unstable();
        s.faces = polys.iter().filter(|p| p.boundary != NO_CELL).count();
        s.open_faces = polys.len() - s.faces;
        let pts: Vec<Vec<[f64; 3]>> = polys
            .iter()
            .map(|p| p.verts.iter().map(|&v| pos(v)).collect())
            .collect();
        let normals: Vec<[f32; 3]> = pts.iter().map(|p| newell(p)).collect();
        // While slices or iso-surfaces show, the mesh is a faint outline
        // of its feature edges (or, in a render mode without faces, its
        // lines): its triangles are left out, so a click picks what is
        // inside.
        let faint = display.outline_mesh();
        // Without its faces, the mesh gives no triangles (the renderer
        // draws faces for slices, iso-surfaces and arrows).
        let extras = display.extracts_shown() || display.arrows_shown();
        let mesh_triangles = !(extras && !display.mesh_faces());
        let features = (display.render_mode == RenderMode::FeatureEdges || faint).then(|| {
            let runs = murmel_mesh::par::ranges(polys.len(), 1 << 12);
            feature_edges(
                &polys,
                &normals,
                &runs,
                FEATURE_ANGLE_DEG.to_radians().cos(),
            )
        });
        // Colour mode field: the value of each polygon (colour mode flat)
        // and of each corner (smooth, banded).
        let field = display.field_values();
        s.value_origin = field.map_or(0.0, |f| f.origin);
        let corner_values: Vec<Vec<f64>> = match field {
            Some(f) => polys
                .iter()
                .map(|p| {
                    p.verts
                        .iter()
                        .map(|&v| match (v, &cutter) {
                            (V::Node(i), _) => f.node.get(i as usize).copied().unwrap_or(f64::NAN),
                            (_, Some(c)) => c.value(v, &f.node),
                            (V::Cut(_), None) => f64::NAN,
                        })
                        .collect()
                })
                .collect(),
            None => Vec::new(),
        };
        drop(phase);
        let phase = murmel_mesh::profile::span("surface.buffers");
        // Lines alone are drawn in the face colour; over faces, darker.
        let lines_only = !display.render_mode.faces();
        // The colours of each polygon in draw order, and its flat value
        // when it has a value at every corner.
        let looks: Vec<Look> = murmel_mesh::par::collect(order.len(), 1 << 12, |r| {
            r.map(|o| {
                let (unselected, _, i) = order[o];
                let poly = &polys[i];
                let rgb = poly.color;
                let flat = field.and_then(|f| {
                    let corners = &corner_values[i];
                    if !corners.iter().all(|x| x.is_finite()) {
                        return None;
                    }
                    let mut flat = f.flat(poly.cell, poly.boundary);
                    if !flat.is_finite() {
                        // A face of no cell (a surface mesh) with a point
                        // field: the mean of its corners.
                        flat = corners.iter().sum::<f64>() / corners.len().max(1) as f64;
                    }
                    flat.is_finite().then_some(flat)
                });
                let sel_mix = if unselected { 0.0 } else { mix };
                let (color, edge) = if faint {
                    let ec = if unselected {
                        tokens.mesh.field.outline
                    } else {
                        sel
                    };
                    (rgb.linear(), ec.linear())
                } else if field.is_some() {
                    let face = if flat.is_some() {
                        rgb
                    } else {
                        tokens.mesh.field.none
                    };
                    let face = if unselected {
                        face
                    } else {
                        Rgb(std::array::from_fn(|k| {
                            (face.0[k] as f32 * (1.0 - mix) + sel.0[k] as f32 * mix).round() as u8
                        }))
                    };
                    let edge = if !unselected {
                        sel
                    } else if lines_only {
                        tokens.mesh.field.none
                    } else {
                        tokens.mesh.field.edge
                    };
                    (face.linear(), edge.linear())
                } else if unselected {
                    (
                        rgb.linear(),
                        if lines_only {
                            rgb.linear()
                        } else {
                            Rgb(rgb.0.map(|v| (v as f32 * darken).round() as u8)).linear()
                        },
                    )
                } else {
                    let m = Rgb(std::array::from_fn(|k| {
                        (rgb.0[k] as f32 * (1.0 - mix) + sel.0[k] as f32 * mix).round() as u8
                    }));
                    (m.linear(), sel.linear())
                };
                Look {
                    color,
                    edge,
                    sel_mix,
                    flat,
                }
            })
            .collect()
        });
        // Where each polygon's vertices and indices start: a triangle is
        // drawn as is, a larger polygon as a fan about its centre.
        let (mut vstart, mut istart) = (vec![0usize], vec![0usize]);
        vstart.reserve(order.len());
        istart.reserve(order.len());
        let (mut nv, mut ni) = (0, 0);
        for &(_, _, i) in &order {
            let k = polys[i].verts.len();
            if mesh_triangles {
                nv += if k == 3 { 3 } else { k + 1 };
                ni += if k == 3 { 3 } else { 3 * k };
            }
            vstart.push(nv);
            istart.push(ni);
        }
        s.positions = vec![[0.0; 3]; nv];
        s.normals = vec![[0.0; 3]; nv];
        s.colors = vec![[0.0; 3]; nv];
        s.values = vec![[0.0; 4]; nv];
        s.indices = vec![0; ni];
        s.tri_cell = vec![0; ni / 3];
        s.tri_face = vec![0; ni / 3];
        let ranges = murmel_mesh::par::ranges(order.len(), 1 << 12);
        let cuts = |at: &[usize], per: usize| -> Vec<usize> {
            ranges
                .iter()
                .map(|r| at[r.start] / per)
                .chain([at[order.len()] / per])
                .collect()
        };
        let (vcuts, icuts, tcuts) = (cuts(&vstart, 1), cuts(&istart, 1), cuts(&istart, 3));
        let mut outs = Vec::with_capacity(ranges.len());
        let mut positions = murmel_mesh::par::split_mut(&mut s.positions, &vcuts).into_iter();
        let mut normals_out = murmel_mesh::par::split_mut(&mut s.normals, &vcuts).into_iter();
        let mut colors = murmel_mesh::par::split_mut(&mut s.colors, &vcuts).into_iter();
        let mut values = murmel_mesh::par::split_mut(&mut s.values, &vcuts).into_iter();
        let mut indices = murmel_mesh::par::split_mut(&mut s.indices, &icuts).into_iter();
        let mut tri_cell = murmel_mesh::par::split_mut(&mut s.tri_cell, &tcuts).into_iter();
        let mut tri_face = murmel_mesh::par::split_mut(&mut s.tri_face, &tcuts).into_iter();
        for r in &ranges {
            if let (Some(p), Some(n), Some(c), Some(v), Some(x), Some(tc), Some(tf)) = (
                positions.next(),
                normals_out.next(),
                colors.next(),
                values.next(),
                indices.next(),
                tri_cell.next(),
                tri_face.next(),
            ) {
                outs.push((
                    r.clone(),
                    Out {
                        positions: p,
                        normals: n,
                        colors: c,
                        values: v,
                        indices: x,
                        tri_cell: tc,
                        tri_face: tf,
                    },
                ));
            }
        }
        let fill = Fill {
            order: &order,
            polys: &polys,
            pts: &pts,
            normals: &normals,
            corner_values: &corner_values,
            looks: &looks,
            vstart: &vstart,
            istart: &istart,
            origin,
            value_origin: s.value_origin,
            mesh_triangles,
        };
        let parts = on_threads(outs, |(r, mut out)| fill.chunk(r, &mut out));
        // The edges, each once: an edge two polygons share takes the
        // colour of the first in draw order.
        let lines = Lines {
            order: &order,
            polys: &polys,
            pts: &pts,
            looks: &looks,
            features: features.as_ref(),
        };
        let edges = lines.draw(&ranges);
        (s.edge_positions, s.edge_colors) = edges;
        let mut vb = Bounds::empty();
        let mut drawn = Bounds1::default();
        for (b, d) in parts {
            if !b.is_empty() {
                vb.add(b.min);
                vb.add(b.max);
            }
            drawn.merge(&d);
        }
        drop(phase);
        s.visible_bounds = (!vb.is_empty()).then_some(vb);
        s.field_range = drawn.range();
        s.key = BuildKey::next();
        s.extras_at = ExtrasAt {
            vertices: s.positions.len(),
            indices: s.indices.len(),
            edges: s.edge_positions.len(),
            visible_bounds: s.visible_bounds,
            field_range: s.field_range,
        };
        let _phase = murmel_mesh::profile::span("surface.extras");
        if extras {
            s.append_extras(mesh, fields, tokens, display, &shown);
        }
        s.outline_color = sel.linear();
        for (a, b) in outline(mesh, display) {
            for i in [a, b] {
                let p = mesh.points[i as usize];
                s.outline.push([0, 1, 2].map(|k| (p[k] - origin[k]) as f32));
            }
        }
        drop(_phase);
        let _phase = murmel_mesh::profile::span("surface.boxes");
        s.tri_boxes = tri_boxes(&s.positions, &s.indices);
        s
    }

    /// The slices, iso-surfaces, arrows, streamlines, parcels and block
    /// boxes of `display`, appended.
    fn append_extras(
        &mut self,
        mesh: &Mesh,
        fields: Option<&FieldSet>,
        tokens: &Tokens,
        display: &Display,
        shown: &[bool],
    ) {
        // An overview has no cells: what cuts through the volume draws
        // on its preview grid.
        match fields
            .and_then(FieldSet::preview)
            .filter(|_| mesh.cell_count() == 0)
        {
            Some(p) => crate::preview::append(self, p, tokens, display),
            None => crate::extract::append(self, mesh, fields, tokens, display, shown),
        }
        // (an overview's lines are traced through its blocks)
        crate::stream::append(self, mesh, fields, tokens, display);
        crate::parcel::append(self, mesh, tokens, display);
        crate::block_boxes::append(self, tokens, display);
    }

    /// Build the extras (slices, iso-surfaces, arrows, streamlines,
    /// parcels, block boxes) of `display` again, keeping the mesh's own
    /// faces: the same surface, bit for bit, as [`Surface::build_with`]
    /// of `display`, when it differs from the display this surface was
    /// built for only where [`extras_only`] allows. The key stays, so a
    /// renderer uploads only the extras.
    pub fn rebuild_extras(
        &mut self,
        mesh: &Mesh,
        fields: Option<&FieldSet>,
        tokens: &Tokens,
        display: &Display,
    ) {
        let at = self.extras_at;
        self.positions.truncate(at.vertices);
        self.normals.truncate(at.vertices);
        self.colors.truncate(at.vertices);
        self.values.truncate(at.vertices);
        self.indices.truncate(at.indices);
        self.tri_cell.truncate(at.indices / 3);
        self.tri_face.truncate(at.indices / 3);
        self.edge_positions.truncate(at.edges);
        self.edge_colors.truncate(at.edges);
        self.stream_positions.clear();
        self.stream_colors.clear();
        self.visible_bounds = at.visible_bounds;
        self.field_range = at.field_range;
        self.arrows = 0;
        self.arrows_capped = false;
        self.arrow_longest = 0.0;
        self.arrow_range = None;
        self.block_boxes = 0;
        self.parcels = 0;
        self.parcels_capped = false;
        self.parcel_range = None;
        let _phase = murmel_mesh::profile::span("surface.extras");
        if display.extracts_shown() || display.arrows_shown() {
            let shown = display.shown_cells(mesh);
            self.append_extras(mesh, fields, tokens, display, &shown);
        }
        drop(_phase);
        let _phase = murmel_mesh::profile::span("surface.boxes");
        // The pick boxes of the runs from the first the extras touch.
        let first = at.indices / 3 / PICK_RUN;
        self.tri_boxes.truncate(first);
        let from = first * PICK_RUN * 3;
        let rest = tri_boxes(&self.positions, &self.indices[from..]);
        self.tri_boxes.extend(rest);
    }
}

/// Whether a surface built for display `old` becomes the surface of
/// `new` by building its extras again ([`Surface::rebuild_extras`]): the
/// two differ only in their slices, iso-surfaces, arrows, streamlines,
/// parcels and block boxes (and in which of those are selected), and
/// whether the mesh is drawn as its outline, with faces, or without its
/// triangles is the same.
pub fn extras_only(old: &Display, new: &Display) -> bool {
    let extra = |i: &Item| {
        matches!(
            i,
            Item::Slice(_)
                | Item::Iso(_)
                | Item::Arrows
                | Item::Stream(_)
                | Item::Parcels
                | Item::Blocks
        )
    };
    let mesh_items =
        |d: &Display| -> Vec<Item> { d.selection.iter().copied().filter(|i| !extra(i)).collect() };
    let any = |d: &Display| d.extracts_shown() || d.arrows_shown();
    if any(old) != any(new)
        || old.outline_mesh() != new.outline_mesh()
        || old.mesh_faces() != new.mesh_faces()
        || mesh_items(old) != mesh_items(new)
    {
        return false;
    }
    let mut d = Display {
        slices: new.slices.clone(),
        isos: new.isos.clone(),
        arrows: new.arrows.clone(),
        streams: new.streams.clone(),
        stream_traces: new.stream_traces.clone(),
        parcels: new.parcels.clone(),
        parcel_data: new.parcel_data.clone(),
        show_blocks: new.show_blocks,
        block_boxes: new.block_boxes.clone(),
        selection: new.selection.clone(),
        ..old.clone()
    };
    // The pick target follows a click on a slice and changes nothing drawn.
    d.pick = new.pick;
    crate::session::same_display(&d, new)
}

/// The box of each run of [`PICK_RUN`] triangles, runs split over the
/// threads.
fn tri_boxes(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[[f32; 3]; 2]> {
    let tris = indices.as_chunks::<3>().0;
    let runs = tris.len().div_ceil(PICK_RUN);
    murmel_mesh::par::collect(runs, 1 << 10, |r| {
        r.map(|k| {
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            let end = ((k + 1) * PICK_RUN).min(tris.len());
            for tri in &tris[k * PICK_RUN..end] {
                for &i in tri {
                    if let Some(p) = positions.get(i as usize) {
                        for d in 0..3 {
                            lo[d] = lo[d].min(p[d]);
                            hi[d] = hi[d].max(p[d]);
                        }
                    }
                }
            }
            [lo, hi]
        })
        .collect()
    })
}

/// The smallest and largest of the values added.
#[derive(Default)]
struct Bounds1(Option<(f64, f64)>);

impl Bounds1 {
    fn add(&mut self, x: f64) {
        if x.is_finite() {
            self.0 = Some(match self.0 {
                None => (x, x),
                Some((a, b)) => (a.min(x), b.max(x)),
            });
        }
    }

    fn range(&self) -> Option<(f64, f64)> {
        self.0
    }

    fn merge(&mut self, other: &Bounds1) {
        if let Some((a, b)) = other.0 {
            self.add(a);
            self.add(b);
        }
    }
}

/// The colours of a polygon (its faces and its edges), the share of the
/// selection colour mixed into its values, and its flat value when it
/// has a value at every corner.
struct Look {
    color: [f32; 3],
    edge: [f32; 3],
    sel_mix: f32,
    flat: Option<f64>,
}

/// The part of the triangle buffers one run of polygons fills.
struct Out<'a> {
    positions: &'a mut [[f32; 3]],
    normals: &'a mut [[f32; 3]],
    colors: &'a mut [[f32; 3]],
    values: &'a mut [[f32; 4]],
    indices: &'a mut [u32],
    tri_cell: &'a mut [u32],
    tri_face: &'a mut [u32],
}

/// What filling the triangle buffers reads: the polygons in draw order
/// (`order`), their corners relative to the origin, normals, corner
/// values and looks, and where each one's vertices and indices start.
struct Fill<'a> {
    order: &'a [(bool, u32, usize)],
    polys: &'a [Poly],
    pts: &'a [Vec<[f64; 3]>],
    normals: &'a [[f32; 3]],
    corner_values: &'a [Vec<f64>],
    looks: &'a [Look],
    vstart: &'a [usize],
    istart: &'a [usize],
    origin: [f64; 3],
    value_origin: f64,
    mesh_triangles: bool,
}

impl Fill<'_> {
    /// Fill `out` with the polygons `run` (positions in draw order) and
    /// give the bounds of their corners and of the field values drawn.
    fn chunk(&self, run: std::ops::Range<usize>, out: &mut Out<'_>) -> (Bounds, Bounds1) {
        let (v0, i0) = (self.vstart[run.start], self.istart[run.start]);
        let mut vb = Bounds::empty();
        let mut drawn = Bounds1::default();
        let o3 = self.origin;
        for o in run {
            let (_, _, i) = self.order[o];
            let (poly, pts, look) = (&self.polys[i], &self.pts[i], &self.looks[o]);
            for p in pts {
                vb.add([p[0] + o3[0], p[1] + o3[1], p[2] + o3[2]]);
            }
            if !self.mesh_triangles {
                // Edges only.
                continue;
            }
            let corners = look.flat.map(|_| self.corner_values[i].as_slice());
            if let (Some(flat), Some(c)) = (look.flat, corners) {
                drawn.add(flat);
                c.iter().for_each(|&x| drawn.add(x));
            }
            let origin_v = self.value_origin;
            let value = |node: f64| -> [f32; 4] {
                match look.flat {
                    Some(flat) => [
                        (flat - origin_v) as f32,
                        (node - origin_v) as f32,
                        look.sel_mix,
                        1.0,
                    ],
                    None => [0.0, 0.0, 0.0, 0.0],
                }
            };
            let corner =
                |j: usize| -> f64 { corners.map_or(0.0, |c| c.get(j).copied().unwrap_or(0.0)) };
            let n = self.normals[i];
            let (lv, li) = (self.vstart[o] - v0, self.istart[o] - i0);
            let mut put = |slot: usize, p: [f64; 3], v: [f32; 4]| {
                out.positions[lv + slot] = p.map(|x| x as f32);
                out.normals[lv + slot] = n;
                out.colors[lv + slot] = look.color;
                out.values[lv + slot] = v;
            };
            let base = self.vstart[o] as u32;
            let tris = if pts.len() == 3 {
                for (j, &p) in pts.iter().enumerate() {
                    put(j, p, value(corner(j)));
                }
                out.indices[li..li + 3].copy_from_slice(&[base, base + 1, base + 2]);
                1
            } else {
                let k = pts.len() as f64;
                let c = [0, 1, 2].map(|d| pts.iter().map(|p| p[d]).sum::<f64>() / k);
                let mean = (0..pts.len()).map(corner).sum::<f64>() / k;
                put(0, c, value(mean));
                for (j, &p) in pts.iter().enumerate() {
                    put(1 + j, p, value(corner(j)));
                }
                let m = pts.len() as u32;
                for t in 0..m {
                    let at = li + 3 * t as usize;
                    out.indices[at..at + 3].copy_from_slice(&[
                        base,
                        base + 1 + t,
                        base + 1 + (t + 1) % m,
                    ]);
                }
                pts.len()
            };
            let lt = li / 3;
            out.tri_cell[lt..lt + tris].fill(poly.cell);
            out.tri_face[lt..lt + tris].fill(poly.face);
        }
        (vb, drawn)
    }
}

/// Run `f` on each of `items`, each on a thread of its own (the first on
/// this one), and give the results in order.
fn on_threads<T: Send, R: Send>(items: Vec<T>, f: impl Fn(T) -> R + Sync) -> Vec<R> {
    let f = &f;
    std::thread::scope(|sc| {
        let mut items = items.into_iter();
        let here = items.next();
        let others: Vec<_> = items.map(|t| sc.spawn(move || f(t))).collect();
        let mut out = Vec::with_capacity(others.len() + 1);
        out.extend(here.map(f));
        for h in others {
            match h.join() {
                Ok(r) => out.push(r),
                Err(p) => std::panic::resume_unwind(p),
            }
        }
        out
    })
}

/// What drawing the edges reads: the polygons in draw order, their
/// corners and looks, and the feature edges when only those are drawn.
struct Lines<'a> {
    order: &'a [(bool, u32, usize)],
    polys: &'a [Poly],
    pts: &'a [Vec<[f64; 3]>],
    looks: &'a [Look],
    features: Option<&'a Features>,
}

impl Lines<'_> {
    /// The edge from corner `j` of the polygon at `o` in draw order, its
    /// ends in increasing order.
    fn key(&self, o: usize, j: usize) -> (V, V) {
        let vs = &self.polys[self.order[o].2].verts;
        let (a, b) = (vs[j], vs[(j + 1) % vs.len()]);
        (a.min(b), a.max(b))
    }

    /// The edge lines, two vertices per edge, each edge once, in the
    /// colour of the first polygon in draw order that has it, in the
    /// order of their first polygons. The polygons are split into `runs`
    /// (in draw order) and the edges into as many shards by their
    /// smaller end, so finding the first of each edge is split over the
    /// threads too: each shard walks its edges in draw order, run by run.
    fn draw(&self, runs: &[std::ops::Range<usize>]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let shards = runs.len().max(1);
        let shard = |k: (V, V)| shard_of(k, shards);
        // The edges of each run that are drawn at all, by shard, as the
        // polygon (its place in draw order) and the corner they start at.
        let by_run: Vec<Vec<Vec<(u32, u32)>>> = on_threads(runs.to_vec(), |r| {
            let mut out = vec![Vec::new(); shards];
            for o in r {
                let vs = &self.polys[self.order[o].2].verts;
                for j in 0..vs.len() {
                    let key = self.key(o, j);
                    if key.0 == key.1 || self.features.is_some_and(|fe| !fe.contains(&key)) {
                        continue;
                    }
                    out[shard(key)].push((o as u32, j as u32));
                }
            }
            out
        });
        // The first of each edge in draw order, shard by shard.
        let firsts: Vec<Vec<(u32, u32)>> = on_threads((0..shards).collect(), |k| {
            let n: usize = by_run.iter().map(|r| r[k].len()).sum();
            let mut seen: FastSet<(V, V)> = set_with_capacity(n / 2 + 16);
            let mut first = Vec::with_capacity(n / 2 + 16);
            for run in &by_run {
                for &(o, j) in &run[k] {
                    if seen.insert(self.key(o as usize, j as usize)) {
                        first.push((o, j));
                    }
                }
            }
            first
        });
        drop(by_run);
        // Mark them per polygon corner, then write the lines run by run.
        let mut corner_at = Vec::with_capacity(self.order.len() + 1);
        corner_at.push(0usize);
        for &(_, _, i) in self.order {
            let last = corner_at.last().copied().unwrap_or(0);
            corner_at.push(last + self.polys[i].verts.len());
        }
        let mut drawn = vec![false; corner_at.last().copied().unwrap_or(0)];
        for (o, j) in firsts.into_iter().flatten() {
            drawn[corner_at[o as usize] + j as usize] = true;
        }
        let counts: Vec<usize> = runs
            .iter()
            .map(|r| {
                drawn[corner_at[r.start]..corner_at[r.end]]
                    .iter()
                    .filter(|&&d| d)
                    .count()
            })
            .collect();
        let total: usize = counts.iter().sum();
        let mut cuts = vec![0usize];
        for c in &counts {
            let last = cuts.last().copied().unwrap_or(0);
            cuts.push(last + 2 * c);
        }
        let mut positions = vec![[0.0f32; 3]; 2 * total];
        let mut colors = vec![[0.0f32; 3]; 2 * total];
        let pieces = murmel_mesh::par::split_mut(&mut positions, &cuts)
            .into_iter()
            .zip(murmel_mesh::par::split_mut(&mut colors, &cuts));
        let work: Vec<_> = runs.iter().cloned().zip(pieces).collect();
        let (drawn, corner_at) = (&drawn, &corner_at);
        on_threads(work, |(r, (pos, col))| {
            let mut at = 0;
            for o in r {
                let (_, _, i) = self.order[o];
                let (pts, ec) = (&self.pts[i], self.looks[o].edge);
                let n = pts.len();
                for j in 0..n {
                    if drawn[corner_at[o] + j] {
                        pos[at] = pts[j].map(|v| v as f32);
                        pos[at + 1] = pts[(j + 1) % n].map(|v| v as f32);
                        col[at] = ec;
                        col[at + 1] = ec;
                        at += 2;
                    }
                }
            }
        });
        (positions, colors)
    }
}

/// The polygons to draw, before the selection orders them.
fn polygons(
    mesh: &Mesh,
    tokens: &Tokens,
    display: &Display,
    shown: &[bool],
    cutter: Option<&Cutter<'_>>,
) -> Vec<Poly> {
    let flat = cutter.filter(|c| c.slices());
    let mut polys: Vec<Poly> = Vec::new();
    let add = |polys: &mut Vec<Poly>,
               verts: Vec<V>,
               cell: u32,
               face: u32,
               boundary: u32,
               color: Rgb,
               group: u32| {
        let verts = match flat {
            Some(c) => c.clip_flat(cell, verts),
            None => verts,
        };
        if verts.len() >= 3 {
            polys.push(Poly {
                verts,
                cell,
                face,
                boundary,
                color,
                group,
            });
        }
    };
    let nodes = |f: &[u32]| f.iter().map(|&i| V::Node(i)).collect::<Vec<_>>();
    let patch = |b: usize| mesh.boundary_patch.get(b).copied().unwrap_or(0);
    // Boundary polygons that match no cell face (or a mesh with no cells):
    // drawn by their patch, and by the clip when it keeps a node.
    for (b, &u) in mesh.boundary_face_id.iter().enumerate() {
        if u != NO_CELL && mesh.cell_count() > 0 {
            continue;
        }
        let face = mesh.boundary.get(b);
        if !display.face_visible(mesh, b) || cutter.is_some_and(|c| !c.keeps_some(face)) {
            continue;
        }
        add(
            &mut polys,
            nodes(face),
            NO_CELL,
            u,
            b as u32,
            display.face_color(mesh, tokens, b),
            patch(b),
        );
    }
    // The faces between cells, and the boundary faces of the cells: on
    // all threads, in face order.
    let regions_cut = flat
        .and_then(|c| c.regions.as_ref())
        .and_then(|(rc, _)| rc.cut_by.as_deref());
    let slicing = flat.is_some();
    let cut_by = |c: u32| {
        regions_cut
            .and_then(|k| k.get(c as usize).copied())
            .unwrap_or(NOT_CUT)
    };
    let between = |u: usize, add: &mut dyn FnMut(Vec<V>, u32, u32, u32, Rgb, u32)| {
        let (o, n) = (mesh.face_owner[u], mesh.face_neighbour[u]);
        if o == NO_CELL {
            return;
        }
        let so = shown[o as usize];
        if n == NO_CELL {
            if !so {
                return;
            }
            match mesh.face_boundary.get(u).copied() {
                Some(b) if b != NO_CELL => {
                    let b = b as usize;
                    if !display.face_visible(mesh, b) {
                        return;
                    }
                    add(
                        nodes(mesh.boundary.get(b)),
                        o,
                        u as u32,
                        b as u32,
                        display.face_color(mesh, tokens, b),
                        patch(b),
                    );
                }
                _ => add(
                    nodes(mesh.face_nodes(u)),
                    o,
                    u as u32,
                    NO_CELL,
                    display.cell_color(mesh, tokens, o as usize),
                    GROUP_OPEN,
                ),
            }
            return;
        }
        let sn = shown[n as usize];
        if so && sn {
            // Two cells cut differently (one by a flat region, the other
            // whole or by another region): each side draws the face as
            // its own clip keeps it, so where one side is cut away the
            // other's face closes the surface (the rest lies inside).
            if slicing && cut_by(o) != cut_by(n) {
                for c in [o, n] {
                    add(
                        nodes(mesh.face_nodes(u)),
                        c,
                        u as u32,
                        NO_CELL,
                        display.cell_color(mesh, tokens, c as usize),
                        GROUP_OPEN,
                    );
                }
            }
            return;
        }
        if !so && !sn {
            return;
        }
        let c = if so { o } else { n };
        add(
            nodes(mesh.face_nodes(u)),
            c,
            u as u32,
            NO_CELL,
            display.cell_color(mesh, tokens, c as usize),
            GROUP_OPEN,
        );
    };
    let faces = mesh.face_owner.len().min(mesh.face_neighbour.len());
    // When something cuts flat, the threads also tell which polygons a
    // plane crosses: only those go through the cutter, afterwards and in
    // face order, so the cut points are numbered as before; the others
    // are kept whole or dropped where they are found.
    let sides = flat.map(Cutter::sides);
    let found = murmel_mesh::par::collect(faces, 1 << 14, |r| {
        let mut out = Vec::new();
        let mut keep = |verts: Vec<V>, cell, face, boundary, color, group| {
            let fate = sides.map_or(Fate::Whole, |s| s.fate(cell, &verts));
            if verts.len() >= 3 && fate != Fate::Drop {
                let poly = Poly {
                    verts,
                    cell,
                    face,
                    boundary,
                    color,
                    group,
                };
                out.push((poly, fate == Fate::Cut));
            }
        };
        for u in r {
            between(u, &mut keep);
        }
        out
    });
    polys.reserve(found.len());
    for (p, cut) in found {
        if cut {
            let Poly {
                verts,
                cell,
                face,
                boundary,
                color,
                group,
            } = p;
            add(&mut polys, verts, cell, face, boundary, color, group);
        } else {
            polys.push(p);
        }
    }
    // Faces isolated on their own (I on picked faces), unless a cell
    // shown beside them already draws them.
    for &item in &display.isolated {
        let Item::Face(u) = item else { continue };
        let u = u as usize;
        let side = |c: Option<&u32>| c.is_some_and(|&c| c != NO_CELL && shown[c as usize]);
        if u >= mesh.unique_faces
            || side(mesh.face_owner.get(u))
            || side(mesh.face_neighbour.get(u))
        {
            continue;
        }
        let owner = mesh.face_owner.get(u).copied().unwrap_or(NO_CELL);
        match mesh.face_boundary.get(u).copied() {
            Some(b) if b != NO_CELL => add(
                &mut polys,
                nodes(mesh.boundary.get(b as usize)),
                owner,
                u as u32,
                b,
                display.face_color(mesh, tokens, b as usize),
                patch(b as usize),
            ),
            _ => add(
                &mut polys,
                nodes(mesh.face_nodes(u)),
                owner,
                u as u32,
                NO_CELL,
                if owner == NO_CELL {
                    tokens.mesh.solid
                } else {
                    display.cell_color(mesh, tokens, owner as usize)
                },
                GROUP_OPEN,
            ),
        }
    }
    if let Some(cutter) = flat {
        // The cells a plane crosses, found on all threads: a thin layer
        // of the cells shown.
        let sides = cutter.sides();
        let crossed: Vec<usize> = murmel_mesh::par::collect(shown.len(), 1 << 12, |r| {
            r.filter(|&c| shown[c] && sides.crosses(mesh, c)).collect()
        });
        // The cut points first, numbered in cell order as one thread
        // finds them: a cell one plane slices names where that plane
        // crosses the edges of its faces; a cell more planes slice is cut
        // here, whole.
        let mut rings: Vec<Option<Vec<Vec<V>>>> = Vec::with_capacity(crossed.len());
        for &c in &crossed {
            match cutter.only_plane(c as u32) {
                Some(p) => {
                    cutter.cut_edges(mesh, p, c);
                    rings.push(None);
                }
                None => rings.push(Some(cutter.sections(mesh, c))),
            }
        }
        // Then the sections of the cells one plane slices, on all
        // threads, reading the cut points (a point not found, which the
        // numbering above rules out, sends the cell back to the cutter).
        let built: Vec<Option<Vec<Vec<V>>>> = {
            let cuts = cutter.cuts.borrow();
            let view = View {
                sides,
                store: &*cuts,
            };
            let (crossed, rings) = (&crossed, &rings);
            murmel_mesh::par::collect(crossed.len(), 1 << 6, |r| {
                r.map(|i| match rings[i] {
                    Some(_) => Some(Vec::new()),
                    None => view.sections(mesh, crossed[i]),
                })
                .collect()
            })
        };
        for ((&c, serial), parallel) in crossed.iter().zip(rings).zip(built) {
            let rings = match (serial, parallel) {
                (Some(r), _) | (None, Some(r)) => r,
                (None, None) => cutter.sections(mesh, c),
            };
            if rings.is_empty() {
                continue;
            }
            let color = display.cell_color(mesh, tokens, c);
            for ring in rings {
                polys.push(Poly {
                    verts: ring,
                    cell: c as u32,
                    face: NO_CELL,
                    boundary: NO_CELL,
                    color,
                    group: GROUP_SECTION,
                });
            }
        }
    }
    polys
}

/// Whether `poly` belongs to a selected item.
fn selected(mesh: &Mesh, display: &Display, poly: &Poly) -> bool {
    let cell = (poly.cell != NO_CELL).then_some(poly.cell as usize);
    display.selection.iter().any(|&item| match item {
        Item::Mesh => false,
        Item::Patch(p) => {
            poly.boundary != NO_CELL && mesh.boundary_patch.get(poly.boundary as usize) == Some(&p)
        }
        Item::Zone(z) => cell.is_some_and(|c| mesh.cell_zone.get(c) == Some(&z)),
        Item::Kind(k) => cell.is_some_and(|c| mesh.cell_kind.get(c) == Some(&k)),
        Item::Cell(c) => poly.cell == c,
        Item::Face(u) => poly.face == u,
        Item::Clip(_)
        | Item::Measure(_)
        | Item::Probe(_)
        | Item::Slice(_)
        | Item::Iso(_)
        | Item::Arrows
        | Item::Parcels
        | Item::Blocks
        | Item::Region(_)
        | Item::Stream(_) => false,
    })
}

/// The outline of the picked cells and faces: every edge of a selected
/// cell (or of the cell holding a selected probe), every edge of a
/// selected face, each once, as line pairs of point ids. Drawn over everything (UI spec: "an accent outline"), so a picked
/// cell stays visible behind others.
fn outline(mesh: &Mesh, display: &Display) -> Vec<(u32, u32)> {
    let mut seen: HashSet<(u32, u32)> = HashSet::new();
    let mut out = Vec::new();
    let mut add_face = |f: &[u32], out: &mut Vec<(u32, u32)>| {
        for i in 0..f.len() {
            let (a, b) = (f[i], f[(i + 1) % f.len()]);
            if seen.insert((a.min(b), a.max(b))) {
                out.push((a, b));
            }
        }
    };
    for &item in &display.selection {
        let item = match item {
            Item::Probe(n) => match display.probe(n).and_then(|p| p.at.cell) {
                Some(c) => Item::Cell(c),
                None => continue,
            },
            other => other,
        };
        match item {
            Item::Cell(c) if (c as usize) < mesh.cell_count() => {
                for f in mesh.cell(c as usize) {
                    add_face(f.nodes, &mut out);
                }
            }
            Item::Face(u) => add_face(mesh.face_nodes(u as usize), &mut out),
            _ => {}
        }
    }
    out
}

/// The shard of edge `key` among `shards`, by its smaller end, so each
/// thread can take the edges of one shard.
fn shard_of(key: (V, V), shards: usize) -> usize {
    let id = match key.0 {
        V::Node(i) => u64::from(i),
        V::Cut(i) => u64::from(i) | 1 << 32,
    };
    let mixed = id.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 32;
    ((mixed * shards as u64) >> 32) as usize
}

/// How the faces seen so far meet at an edge: the normal and group of
/// the first, how many there are, and whether a later one makes a crease
/// or a border of groups with the first.
struct Seen {
    normal: [f32; 3],
    group: u32,
    count: u32,
    feature: bool,
}

/// The feature edges of a surface, in shards by [`shard_of`].
struct Features {
    shards: Vec<FastMap<(V, V), Seen>>,
}

impl Features {
    fn contains(&self, key: &(V, V)) -> bool {
        self.shards[shard_of(*key, self.shards.len())]
            .get(key)
            .is_some_and(|e| e.feature || e.count == 1)
    }
}

/// The feature edges among `polys` (with their `normals`): an edge whose
/// two faces meet at a crease (the cosine of the angle between their
/// normals below `cos_max`), whose faces lie in different groups
/// (patches, open faces, sections), or that has only one face drawn (an
/// open border, also where hidden items were cut away). The normals'
/// orientation is not trusted, so a crease is judged on the absolute
/// cosine. Each later face is judged against the first, in the order of
/// `polys`; the polygons are split into `runs` and the edges into as many
/// shards, each walked in that order on a thread of its own.
fn feature_edges(
    polys: &[Poly],
    normals: &[[f32; 3]],
    runs: &[std::ops::Range<usize>],
    cos_max: f64,
) -> Features {
    let shards = runs.len().max(1);
    let by_run: Vec<Vec<Vec<(u32, u32)>>> = on_threads(runs.to_vec(), |r| {
        let mut out = vec![Vec::new(); shards];
        for i in r {
            let face = &polys[i].verts;
            for j in 0..face.len() {
                let (a, b) = (face[j], face[(j + 1) % face.len()]);
                out[shard_of((a.min(b), a.max(b)), shards)].push((i as u32, j as u32));
            }
        }
        out
    });
    let shards = on_threads((0..shards).collect(), |k| {
        let n: usize = by_run.iter().map(|r| r[k].len()).sum();
        let mut edges: FastMap<(V, V), Seen> = FastMap::default();
        edges.reserve(n / 2 + 16);
        for run in &by_run {
            for &(i, j) in &run[k] {
                let (i, j) = (i as usize, j as usize);
                let (face, group, n) = (&polys[i].verts, polys[i].group, normals[i]);
                let (a, b) = (face[j], face[(j + 1) % face.len()]);
                let e = edges.entry((a.min(b), a.max(b))).or_insert(Seen {
                    normal: n,
                    group,
                    count: 0,
                    feature: false,
                });
                if e.count > 0 {
                    let dot: f32 = (0..3).map(|k| e.normal[k] * n[k]).sum();
                    e.feature |= e.group != group || (dot.abs() as f64) < cos_max;
                }
                e.count += 1;
            }
        }
        edges
    });
    Features { shards }
}

/// The unit normal of a polygon by Newell's method.
fn newell(p: &[[f64; 3]]) -> [f32; 3] {
    let mut n = [0.0f64; 3];
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l > 0.0 {
        n.map(|v| (v / l) as f32)
    } else {
        [0.0, 0.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{Clip, ClipMode};

    /// Invariant: on the channel (a box of walls, inlet and outlet) the
    /// feature edges are the box's 12 creases and nothing inside a flat
    /// side: all lie on the bounding box's edges, the flat interior edges
    /// are dropped, and every box edge is covered.
    #[test]
    fn channel_feature_edges_are_the_box_edges() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        let mesh = murmel_mesh::read(&p).unwrap();
        let t = Tokens::builtin().unwrap();
        let all = Surface::build(&mesh, &t, &Display::default());
        let d = Display {
            render_mode: RenderMode::FeatureEdges,
            ..Display::default()
        };
        let s = Surface::build(&mesh, &t, &d);
        assert!(
            s.edges() > 0 && s.edges() < all.edges() / 4,
            "{} of {}",
            s.edges(),
            all.edges()
        );
        let b = s.bounds.unwrap();
        let half: Vec<f64> = (0..3).map(|k| 0.5 * (b.max[k] - b.min[k])).collect();
        let mut length = 0.0;
        for e in s.edge_positions.chunks(2) {
            let on_box_edge = |p: [f32; 3]| {
                (0..3)
                    .filter(|&k| ((p[k].abs() as f64) - half[k]).abs() < 1e-6 * (1.0 + half[k]))
                    .count()
                    >= 2
            };
            assert!(on_box_edge(e[0]) && on_box_edge(e[1]), "{e:?}");
            length += (0..3)
                .map(|k| ((e[0][k] - e[1][k]) as f64).powi(2))
                .sum::<f64>()
                .sqrt();
        }
        let want = 4.0 * 2.0 * (half[0] + half[1] + half[2]);
        assert!((length - want).abs() < 1e-6 * want, "{length} vs {want}");
    }

    fn channel() -> Mesh {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/channel/foehncraft.vtu");
        murmel_mesh::read(&p).unwrap()
    }

    fn clip_y(mesh: &Mesh, mode: ClipMode) -> Display {
        let b = mesh.bounds();
        Display {
            clips: vec![Clip {
                origin: b.center(),
                normal: [0.0, 1.0, 0.0],
                mode,
                enabled: true,
            }],
            ..Display::default()
        }
    }

    /// The y plane of `clip_y` and more planes through the centre: x with
    /// mode `b`, and for three planes an oblique one, flat.
    fn clips(mesh: &Mesh, a: ClipMode, b: ClipMode, three: bool) -> Display {
        let mut d = clip_y(mesh, a);
        let c = mesh.bounds().center();
        d.clips.push(Clip {
            origin: c,
            normal: [1.0, 0.0, 0.0],
            mode: b,
            enabled: true,
        });
        if three {
            let n = 1.0 / 3f64.sqrt();
            d.clips.push(Clip {
                origin: [c[0] - 0.1, c[1], c[2]],
                normal: [-n, n, n],
                mode: ClipMode::Flat,
                enabled: true,
            });
        }
        d
    }

    /// Every polygon edge of a display, counted by use (zero-length edges
    /// at nodes on the plane left out).
    fn edge_uses(mesh: &Mesh, d: &Display) -> HashMap<(V, V), usize> {
        let t = Tokens::builtin().unwrap();
        let shown = d.shown_cells(mesh);
        let cutter = Cutter::new(mesh, d);
        let mut uses = HashMap::new();
        for p in polygons(mesh, &t, d, &shown, cutter.as_ref()) {
            for i in 0..p.verts.len() {
                let (a, b) = (p.verts[i], p.verts[(i + 1) % p.verts.len()]);
                if a != b {
                    *uses.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                }
            }
        }
        uses
    }

    /// The mean of cell `c`'s points.
    fn cell_mean(mesh: &Mesh, c: usize) -> [f64; 3] {
        let pts: Vec<[f64; 3]> = mesh
            .cell_points(c)
            .map(|i| mesh.points[i as usize])
            .collect();
        [0, 1, 2].map(|k| pts.iter().map(|p| p[k]).sum::<f64>() / pts.len() as f64)
    }

    /// A display of the channel with `regions` (their cells found).
    fn with_regions(mesh: &Mesh, regions: Vec<crate::Region>, clips: Vec<Clip>) -> Display {
        let mut d = Display {
            regions,
            clips,
            ..Display::default()
        };
        crate::region::ensure(&mut d, mesh);
        d
    }

    /// Invariant (large-mesh stage 1b, the maintainer's card 6): a box,
    /// a slab or a sphere region cutting flat draws a closed surface,
    /// every edge shared by exactly two polygons, and nothing outside
    /// the shape: a box's and a slab's corners lie inside every face's
    /// half-space, a sphere's within the sphere (a cut point on an edge
    /// is inside, the distance being convex). A slab thinner than the
    /// channel's cells still draws its cells' sections. Breaks when a
    /// cell's cut does not close, when a polygon escapes its region's
    /// clip, or when a thin slab draws nothing.
    #[test]
    fn flat_regions_are_closed_and_stay_inside() {
        use crate::region::{Region, Shape};
        let m = channel();
        let b = m.bounds();
        // Inside the fluid (the channel's middle is its obstacle).
        let c = cell_mean(&m, m.cell_count() / 3);
        let r = 0.3 * b.radius();
        let thin = 1e-3 * b.radius();
        let flat = |shape: Shape| Region {
            cut: ClipMode::Flat,
            ..Region::new(shape)
        };
        for shape in [
            Shape::default_box(&b),
            Shape::Box {
                center: c,
                size: [r, r, 0.5 * thin],
            },
            Shape::Sphere {
                center: c,
                radius: r,
            },
            Shape::Slab {
                origin: c,
                normal: [0.6, 0.0, 0.8],
                thickness: 0.5 * r,
            },
            Shape::Slab {
                origin: c,
                normal: [0.0, 1.0, 0.0],
                thickness: thin,
            },
        ] {
            let d = with_regions(&m, vec![flat(shape.clone())], Vec::new());
            let t = Tokens::builtin().unwrap();
            let shown = d.shown_cells(&m);
            let cutter = Cutter::new(&m, &d).expect("a flat region cuts");
            let polys = polygons(&m, &t, &d, &shown, Some(&cutter));
            assert!(
                polys.iter().any(|p| p.group == GROUP_SECTION),
                "{shape:?} draws sections"
            );
            let borders = shape.borders();
            let tol = 1e-9 * b.radius();
            for p in &polys {
                for &v in &p.verts {
                    let at = cutter.pos(v);
                    for bd in &borders {
                        assert!(bd.distance(at) <= tol, "{shape:?}: {at:?} outside");
                    }
                }
            }
            let uses = edge_uses(&m, &d);
            let open: Vec<_> = uses.iter().filter(|(_, n)| **n != 2).take(3).collect();
            assert!(open.is_empty(), "{shape:?}: {open:?}");
        }
    }

    /// Invariant: regions cutting differently side by side still draw a
    /// closed surface: a flat box beside a whole-cell sphere, two flat
    /// regions overlapping, and a flat region under a flat clip plane;
    /// every edge is used an even number of times (each way of cutting
    /// closes its own cells, and the face between two cells cut
    /// differently is drawn by both). Breaks when the face between a
    /// clipped cell and a whole one is left out, which opens a hole into
    /// the whole cell.
    #[test]
    fn mixed_regions_are_closed() {
        use crate::region::{Region, Shape};
        let m = channel();
        let b = m.bounds();
        let c = cell_mean(&m, m.cell_count() / 3);
        let r = 0.3 * b.radius();
        let flat = |shape: Shape| Region {
            cut: ClipMode::Flat,
            ..Region::new(shape)
        };
        let sphere = Shape::Sphere {
            center: [c[0] + 0.5 * r, c[1], c[2]],
            radius: r,
        };
        let cases = [
            (
                "flat box, whole sphere",
                vec![flat(Shape::default_box(&b)), Region::new(sphere.clone())],
                Vec::new(),
            ),
            (
                "flat box, flat sphere",
                vec![flat(Shape::default_box(&b)), flat(sphere.clone())],
                Vec::new(),
            ),
            (
                "flat sphere, flat clip",
                vec![flat(sphere.clone())],
                vec![Clip {
                    origin: c,
                    normal: [0.0, 1.0, 0.0],
                    mode: ClipMode::Flat,
                    enabled: true,
                }],
            ),
        ];
        for (name, regions, clips) in cases {
            let d = with_regions(&m, regions, clips);
            let uses = edge_uses(&m, &d);
            assert!(!uses.is_empty(), "{name}");
            let odd: Vec<_> = uses.iter().filter(|(_, n)| **n % 2 != 0).take(3).collect();
            assert!(odd.is_empty(), "{name}: {odd:?}");
        }
    }

    /// Invariant: what the viewport draws is a closed surface, every edge
    /// shared by exactly two polygons, for the whole channel, with the
    /// prism layers hidden (the core's faces laid open), and with a
    /// crinkle or a flat clip (the sections close the cut), and with two
    /// or three planes at once (a corner where two flat planes meet a face
    /// has one name in the face and in both sections). A wrong neighbour
    /// in the face table, a section that does not close, or a cut point
    /// named differently by two faces would leave a hole.
    #[test]
    fn drawn_surfaces_are_closed() {
        let m = channel();
        let mut no_prisms = Display::default();
        no_prisms
            .hidden
            .insert(Item::Kind(murmel_mesh::CellKind::Prism));
        for (name, d) in [
            ("all", Display::default()),
            ("no prisms", no_prisms),
            ("crinkle", clip_y(&m, ClipMode::Crinkle)),
            ("flat", clip_y(&m, ClipMode::Flat)),
            (
                "flat + flat",
                clips(&m, ClipMode::Flat, ClipMode::Flat, false),
            ),
            (
                "crinkle + flat",
                clips(&m, ClipMode::Crinkle, ClipMode::Flat, false),
            ),
            (
                "three flat",
                clips(&m, ClipMode::Flat, ClipMode::Flat, true),
            ),
        ] {
            let uses = edge_uses(&m, &d);
            assert!(!uses.is_empty(), "{name}");
            let open: Vec<_> = uses.iter().filter(|(_, n)| **n != 2).take(3).collect();
            assert!(open.is_empty(), "{name}: {open:?}");
        }
    }

    /// Invariant: the buffers the renderer uploads are well formed for
    /// every display the render tests draw: one normal and colour per
    /// position, whole triangles and segments, every index in range, all
    /// numbers finite, unit normals. A software rasteriser may read out of
    /// bounds (or crash) on a bad index or a NaN instead of failing
    /// validation.
    #[test]
    fn buffers_are_well_formed() {
        let m = channel();
        let t = Tokens::builtin().unwrap();
        let mut no_prisms = Display::default();
        no_prisms
            .hidden
            .insert(Item::Kind(murmel_mesh::CellKind::Prism));
        for (name, d) in [
            ("all", Display::default()),
            ("no prisms", no_prisms),
            ("crinkle", clip_y(&m, ClipMode::Crinkle)),
            ("flat", clip_y(&m, ClipMode::Flat)),
            (
                "flat + flat",
                clips(&m, ClipMode::Flat, ClipMode::Flat, false),
            ),
            (
                "crinkle + flat",
                clips(&m, ClipMode::Crinkle, ClipMode::Flat, false),
            ),
            (
                "three flat",
                clips(&m, ClipMode::Flat, ClipMode::Flat, true),
            ),
        ] {
            let s = Surface::build(&m, &t, &d);
            let n = s.positions.len();
            assert_eq!(s.normals.len(), n, "{name}");
            assert_eq!(s.colors.len(), n, "{name}");
            assert_eq!(s.indices.len() % 3, 0, "{name}");
            assert!(s.indices.iter().all(|&i| (i as usize) < n), "{name}");
            assert_eq!(s.edge_positions.len() % 2, 0, "{name}");
            assert_eq!(s.edge_colors.len(), s.edge_positions.len(), "{name}");
            let finite = |v: &[[f32; 3]]| v.iter().flatten().all(|x| x.is_finite());
            assert!(finite(&s.positions) && finite(&s.colors), "{name}");
            assert!(
                finite(&s.edge_positions) && finite(&s.edge_colors),
                "{name}"
            );
            let bad: Vec<_> = s
                .normals
                .iter()
                .filter(|v| {
                    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                    (l - 1.0).abs() >= 1e-3
                })
                .take(3)
                .collect();
            assert!(bad.is_empty(), "{name}: {bad:?}");
        }
    }

    /// Invariant: the buffers are the same however many threads fill
    /// them (each fills a run of polygons in draw order at the place the
    /// counts before it give; the edges are drawn once, in draw order;
    /// a feature edge is judged against its first face in polygon order,
    /// shard by shard; the cut points are numbered on one thread in cell
    /// order before the sections are built on all), for faces with
    /// edges, a cut, sections by one, two and three flat planes and by a
    /// region, lines only, feature edges and a selection, on a mesh that
    /// is all creases and one that is mostly flat. Breaks when a run
    /// starts at the wrong place, the edges or faces are taken in another
    /// order, or a cut point is numbered in another order.
    #[test]
    fn threads_give_the_same_surface() {
        // The honeycomb's boundary is all creases; the channel's flat
        // sides are not, so its feature edges are a few of its edges.
        for m in [murmel_mesh::synth::honeycomb(6), channel()] {
            same_surface_on_threads(&m);
        }
    }

    fn same_surface_on_threads(m: &Mesh) {
        let t = Tokens::builtin().unwrap();
        let wire = Display {
            render_mode: RenderMode::Wireframe,
            ..Display::default()
        };
        let feature = Display {
            render_mode: RenderMode::FeatureEdges,
            ..Display::default()
        };
        let mut sel = Display::default();
        sel.selection.extend([Item::Cell(7), Item::Cell(70)]);
        let key = |s: &Surface| {
            (
                s.positions.clone(),
                s.normals.clone(),
                s.colors.clone(),
                s.values.clone(),
                s.indices.clone(),
                s.tri_cell.clone(),
                s.tri_face.clone(),
                (s.edge_positions.clone(), s.edge_colors.clone()),
                s.visible_bounds.map(|b| (b.min, b.max)),
                s.field_range,
            )
        };
        let b = m.bounds();
        let slab = crate::region::Region {
            cut: ClipMode::Flat,
            ..crate::region::Region::new(crate::region::Shape::Slab {
                origin: b.center(),
                normal: [0.6, 0.0, 0.8],
                thickness: 0.3 * b.radius(),
            })
        };
        for (name, d) in [
            ("all", Display::default()),
            ("crinkle", clip_y(m, ClipMode::Crinkle)),
            ("flat", clip_y(m, ClipMode::Flat)),
            ("two flat", clips(m, ClipMode::Flat, ClipMode::Flat, false)),
            ("three flat", clips(m, ClipMode::Flat, ClipMode::Flat, true)),
            ("flat slab", with_regions(m, vec![slab], Vec::new())),
            ("wireframe", wire),
            ("feature edges", feature),
            ("selection", sel),
        ] {
            let one = murmel_mesh::par::with_threads(1, || key(&Surface::build(m, &t, &d)));
            assert!(!one.0.is_empty(), "{name}");
            for n in [2, 3, 7] {
                let many = murmel_mesh::par::with_threads(n, || key(&Surface::build(m, &t, &d)));
                assert!(one == many, "{name}: {n} threads");
            }
        }
    }

    /// Invariant: hiding the prism layers keeps the 8 polyhedra of the
    /// core and draws their faces, and the crinkle clip keeps whole cells:
    /// fewer than all, more than half (the cells the plane crosses stay).
    #[test]
    fn hidden_types_and_crinkle_keep_whole_cells() {
        let m = channel();
        let t = Tokens::builtin().unwrap();
        let mut d = Display::default();
        d.hidden.insert(Item::Kind(murmel_mesh::CellKind::Prism));
        let s = Surface::build(&m, &t, &d);
        assert_eq!(s.cells, 8);
        assert!(s.open_faces > 0);
        let s = Surface::build(&m, &t, &clip_y(&m, ClipMode::Crinkle));
        assert!(s.cells > m.cell_count() / 2 && s.cells < m.cell_count());
        assert_eq!(s.tri_cell.len(), s.triangles());
        assert!(s.tri_cell.iter().all(|&c| (c as usize) < m.cell_count()));
    }

    /// Invariant: the flat cut's sections tile the channel's cross-section
    /// at y = centre exactly (width times height of the box), so the
    /// section of every crossed cell is there once and none overlaps.
    #[test]
    fn flat_sections_tile_the_cross_section() {
        let m = channel();
        let t = Tokens::builtin().unwrap();
        let d = clip_y(&m, ClipMode::Flat);
        let s = Surface::build(&m, &t, &d);
        let b = m.bounds();
        let want = (b.max[0] - b.min[0]) * (b.max[2] - b.min[2]);
        let mut area = 0.0;
        for (i, tri) in s.indices.chunks(3).enumerate() {
            if s.tri_face[i] != NO_CELL {
                continue;
            }
            let p: [[f64; 3]; 3] =
                std::array::from_fn(|j| s.positions[tri[j] as usize].map(f64::from));
            let e1 = [0, 1, 2].map(|k| p[1][k] - p[0][k]);
            let e2 = [0, 1, 2].map(|k| p[2][k] - p[0][k]);
            area += 0.5 * crate::clip::cross(e1, e2)[1].abs();
        }
        assert!((area - want).abs() < 1e-4 * want, "{area} vs {want}");
        // Everything drawn is on the kept side (y <= centre, to f32).
        assert!(s.positions.iter().all(|p| p[1] <= 1e-5));
    }

    /// Invariant: a selected probe outlines the cell that holds it, the
    /// same lines as selecting that cell, and a probe outside the mesh
    /// outlines nothing. Breaks if `outline` stops mapping probes to their
    /// cell; the window draws the probe marker with egui, so no render
    /// test would notice the missing outline.
    #[test]
    fn a_selected_probe_outlines_its_cell() {
        let m = channel();
        let t = Tokens::builtin().unwrap();
        let mut cell = Display::default();
        cell.selection.push(Item::Cell(7));
        let want = Surface::build(&m, &t, &cell).outline;
        assert!(!want.is_empty());
        let mut d = Display::default();
        let at = |c: Option<u32>| crate::measure::Anchor {
            cell: c,
            ..crate::measure::Anchor::typed([0.0; 3])
        };
        d.probes.push(crate::measure::Probe {
            n: 1,
            name: None,
            at: at(Some(7)),
            visible: true,
        });
        d.probes.push(crate::measure::Probe {
            n: 2,
            name: None,
            at: at(None),
            visible: true,
        });
        d.selection.push(Item::Probe(1));
        assert_eq!(Surface::build(&m, &t, &d).outline, want);
        d.selection = vec![Item::Probe(2)];
        assert!(Surface::build(&m, &t, &d).outline.is_empty());
    }

    /// Invariant: every boundary polygon becomes a closed fan (n triangles
    /// for n > 3 nodes, one for a triangle) and every polygon edge is drawn
    /// once, so a closed surface has as many edge lines as unique edges.
    #[test]
    fn tetra_surface() {
        let mesh = murmel_mesh::read_vtu_str(
            r#"<VTKFile type="UnstructuredGrid"><UnstructuredGrid><Piece NumberOfPoints="4" NumberOfCells="1">
<Points><DataArray type="Float64" NumberOfComponents="3" format="ascii">0 0 0 1 0 0 0 1 0 0 0 1</DataArray></Points>
<Cells><DataArray type="Int64" Name="connectivity">0 1 2 3</DataArray>
<DataArray type="Int64" Name="offsets">4</DataArray><DataArray type="UInt8" Name="types">10</DataArray></Cells>
</Piece></UnstructuredGrid></VTKFile>"#,
        )
        .unwrap();
        let t = Tokens::builtin().unwrap();
        let s = Surface::build(&mesh, &t, &Display::default());
        assert_eq!(s.triangles(), 4);
        assert_eq!(s.edges(), 6);
        assert_eq!(s.origin, [0.5; 3]);
        assert_eq!(s.faces, 4);
        // Every edge of a tetrahedron is a crease (70.5 degrees).
        let feat = Display {
            render_mode: RenderMode::FeatureEdges,
            ..Display::default()
        };
        assert_eq!(Surface::build(&mesh, &t, &feat).edges(), 6);
        for n in &s.normals {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((l - 1.0).abs() < 1e-5);
        }
    }
}
