### Added

- On a mesh opened as an overview, a probe reads the block of the file that holds its point and shows the cell, its quality and the field values the whole mesh shows there (Properties give the block and the cell's number in the file). The last four blocks read stay, so the next probes nearby read at once; in the window the block is read in the background and the probe's label says "reading its block" meanwhile. `probe.add` and `probe.at` do the same.

### Fixed

- A probe in a halo cell of the blocks a region read on an overview took that cell, whose neighbours are partly missing, so its gradient and values were off; it now reads the block that owns the cell.
