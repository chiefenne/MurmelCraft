### Added

- Clip plane (`clip.set`, `clip.clear`): X, Y and Z clip normal to an axis through the centre, cutting away the half towards the camera; the palette also clips along the view direction. Crinkle cut keeps whole cells, flat cut slices them and shows each cell's section. Shift + C flips the side. The toolbar's Clip group has the axis buttons, the cut mode, Flip and a position slider; the model tree has a Clip planes row (eye, flip, cut, remove); a dashed outline and an arrow in the viewport show the plane, and dragging the arrow slides it.
- The mesh's face topology (owner and neighbour of every face). The viewport now draws the outside of the cells shown, so hiding a zone or a cell type lays open the cells behind it: hide the prisms to see the polyhedral core, isolate them to see the layers.

### Changed

- `display.*` results carry the clip plane (`clip`: mode, origin, normal, offset, range).
