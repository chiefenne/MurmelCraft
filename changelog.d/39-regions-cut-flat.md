### Added

- A box, sphere or slab region can cut flat along its border (`region.set` with `cut: flat`, the tree's context menu, Alt + Shift + R): its border cells are clipped to the shape and closed by their sections, as a flat clip plane cuts, and slices, iso-surfaces and arrows follow the cut. A box's and a slab's cut is exact, even when thinner than a cell; a sphere's follows it to within a cell, and a sphere with no mesh node inside shows whole cells instead. Where a whole-cell region and flat regions meet, each cell keeps one way of cutting and the surface stays closed.
- Session files (format 1.8) keep the cut.
