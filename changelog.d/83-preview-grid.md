### Added

- The VTKHDF reader can read the preview grid of a blocked file (`vtkhdf::read_preview`): each cell field of the volume on a regular grid of about 128 cells along the longest side, each grid cell showing the cell its centre lies in, read from the cells' points alone and the fields at those cells. A 4 M cell file gives its 128³ grid in about 1.2 s. This is the base for slices and iso-surfaces across the whole volume of a mesh opened as an overview.
