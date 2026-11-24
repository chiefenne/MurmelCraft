### Added

- Regions of interest (`region.set`, `region.clear`, `region.list`, Alt + R): a box, a sphere, a slab, a cell zone, cell layers along a patch, or picked cells grown by rings of neighbours limit what the viewer shows and what slices, iso-surfaces and arrows compute. A cell belongs when its bounding box touches the shape; several regions on show their union.
- The model tree has a Regions group (eye, select, double-click to fit, Fit and Remove in the context menu) with buttons that add a box, a sphere, a slab or a region of the picked cells; the palette has a command for every shape. Properties show the shape, the cells it takes, the halo of neighbours that a mesh read in pieces will load, and the memory both need.
- Session files (format 1.7) keep the regions; a region whose zone, patch or cells the mesh lacks is skipped as `region:<n>`. New codes REGION001 (no such region), REGION002 (broken shape) and REGION003 (more than 16 regions).
