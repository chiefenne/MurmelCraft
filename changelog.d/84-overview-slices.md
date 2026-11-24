### Added

- Slices, iso-surfaces and arrows on a mesh opened as an overview (beyond the memory budget) now draw across the whole volume on its preview grid: the cell fields on a grid of about 128 cells along the longest side, each grid cell with the values of the cell at its centre. The grid is read the first time one of them shows (the window reads it in the background, the status bar says so, Cancel or Esc stops it); a region still reads the exact cells of its blocks. The model tree's note and `mesh.info` tell the grid.
