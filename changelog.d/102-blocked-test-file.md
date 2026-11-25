### Added

- `murmel bench make --formats blocked` writes the synthetic honeycomb of polyhedra straight into a blocked VTKHDF file, block by block, without the mesh in memory, with two cell fields (`velocity`, a vortex array in a uniform stream, and its `pressure`): a test file of up to 684 M cells. 100 M cells take 94 GB on disk and about 3 to 4 GB of memory while written (docs/performance.md, "A 100 M cell test file"). The command line shows each step as it goes.

### Changed

- The blocked layout (docs/blocked-vtkhdf.md § 5) allows 32-bit ids within a block (`Offsets`, `Connectivity` and the four polyhedron arrays, all six alike); the ids of the whole mesh stay 64-bit. A file of polyhedra then takes about 940 bytes a cell instead of 1,500. MurmelCraft reads either; the synthetic file uses them.
