### Fixed

- A wedge (VTK cell type 13) in VTK's node order, its base (0, 1, 2) pointing towards the top (3, 4, 5) as VTK 9, FoehnCraft and KoflCraft write it, opens with its faces outward from a `.vtu` file too; it opened inside out before. `.vtkhdf` files no longer turn linear cells by their volume, so a cell given inside out stays visible to the quality check.
