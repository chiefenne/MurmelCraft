//! The boxes of an overview's blocks in the view (large-mesh
//! post-processing, after the preview grid): on a mesh open as an
//! overview, the eye of `blocks` in the model tree draws the box of each
//! block, so it shows how the file divides the volume and which blocks a
//! region reads before it reads them.
//!
//! - **What.** The twelve edges of the box of each block's owned points
//!   (the block index of the file, [`murmel_mesh::vtkhdf::Block`]), in
//!   the lines that draw in every render mode and hide behind faces, as
//!   streamlines do; blocks without cells have no box.
//! - **Colour.** The boxes take `mesh.field.block` (or the colour set on
//!   `blocks`); the blocks the enabled box, sphere and slab regions touch
//!   ([`crate::blocks::wanted`]) take `mesh.field.block_read` and are
//!   drawn last, so a box edge two blocks share shows the read one.
//! - **The mesh.** While the boxes show, the mesh draws as its faint
//!   outline (the boxes are inside it) unless `display.mesh_look` asks
//!   for its faces, as with slices.
//! - **Not cut.** Clip planes and hidden zones leave the boxes whole: a
//!   region reads every block it touches, whatever is clipped away.
//! - They show over the overview and over the blocks a region read alike,
//!   and leave with the overview.

use std::sync::Arc;

use murmel_mesh::Bounds;
use murmel_mesh::vtkhdf::Overview;

use crate::display::{Display, Item};
use crate::region::Region;
use crate::surface::Surface;
use crate::theme::Tokens;

/// The boxes of the blocks of an overview, and which of them the regions
/// read.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockBoxes {
    /// The box of each block's owned points, in file order (empty for a
    /// block without cells).
    pub bounds: Vec<Bounds>,
    /// The blocks the enabled box, sphere and slab regions touch,
    /// ascending.
    pub read: Vec<usize>,
}

impl BlockBoxes {
    /// The boxes of `overview` and the blocks `regions` read of it.
    pub fn of(overview: &Overview, regions: &[Region]) -> BlockBoxes {
        BlockBoxes {
            bounds: overview.blocks.iter().map(|b| b.bounds).collect(),
            read: crate::blocks::wanted(overview, regions),
        }
    }

    /// The blocks that have a box.
    pub fn count(&self) -> usize {
        self.bounds.iter().filter(|b| !b.is_empty()).count()
    }
}

/// Put the boxes of `overview` in `d` (none when the open mesh is not an
/// overview), made again only when they changed.
pub(crate) fn ensure(d: &mut Display, overview: Option<&Overview>) {
    let want = overview.map(|o| BlockBoxes::of(o, &d.regions));
    if d.block_boxes.as_deref() != want.as_ref() {
        d.block_boxes = want.map(Arc::new);
    }
    if d.block_boxes.is_none() {
        d.selection.retain(|&i| i != Item::Blocks);
    }
}

/// The corners of a box joined by its twelve edges: corner `i` takes the
/// largest x when bit 0 is set, y for bit 1, z for bit 2.
const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

/// Draw the boxes of `display`'s blocks into the lines of `s`, when they
/// show: the others first, then those the regions read.
pub(crate) fn append(s: &mut Surface, tokens: &Tokens, display: &Display) {
    let Some(boxes) = display
        .block_boxes
        .as_deref()
        .filter(|_| display.show_blocks)
    else {
        return;
    };
    let plain = display
        .colors
        .get(&Item::Blocks)
        .copied()
        .unwrap_or(tokens.mesh.field.block)
        .linear();
    let read = tokens.mesh.field.block_read.linear();
    let o = s.origin;
    let is_read = |k: usize| boxes.read.binary_search(&k).is_ok();
    let order = (0..boxes.bounds.len())
        .filter(|&k| !is_read(k))
        .chain(boxes.read.iter().copied());
    for k in order {
        let Some(b) = boxes.bounds.get(k).filter(|b| !b.is_empty()) else {
            continue;
        };
        let color = if is_read(k) { read } else { plain };
        let corner = |i: usize| -> [f32; 3] {
            [0, 1, 2].map(|a| {
                let x = if i & (1 << a) == 0 {
                    b.min[a]
                } else {
                    b.max[a]
                };
                (x - o[a]) as f32
            })
        };
        for (i, j) in EDGES {
            s.stream_positions.extend([corner(i), corner(j)]);
            s.stream_colors.extend([color, color]);
        }
        s.block_boxes += 1;
    }
}
