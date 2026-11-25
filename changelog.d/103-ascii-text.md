### Changed

- Blocked VTKHDF files mark their text ASCII when it is (other text stays UTF-8): VTK 9.4's reader refused every blocked file MurmelCraft wrote, because their `Type` attribute was marked UTF-8. The layout doc, `blocks.make` and the blocks module said ParaView 5.13 opens these files; they now say VTK 9.6 and later, as checked with VTK 9.4 to 9.7: 9.4 and 9.5 (ParaView 6.0 and earlier) crash on their polyhedra and open only a file of linear cells.
