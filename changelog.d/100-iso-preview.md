### Added

- A new iso value (clicked on the colour bar or typed), or a new iso-surface, shows at once in the window: the GPU casts rays through a grid of the field, and the exact iso-surface takes its place a moment after the value rests (`murmel bench run` times it as `iso.preview`).

### Changed

- The preview grids of slices and iso-surfaces are built on threads of their own as soon as a slice or iso-surface exists, so the first drag on a large mesh no longer stalls the window; the smooth values of a dragged slice now match the exact slice (linear on each cell's tetrahedra).
