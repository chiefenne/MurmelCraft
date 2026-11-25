# Blocked VTKHDF (layout 1)

The file layout that lets MurmelCraft read part of a large mesh and its
fields: a mesh too large for memory opens as an overview, and only the
blocks under a region of interest are read (decided by the maintainer on
2026-11-24: overview plus regions beyond the memory budget, 100 M cells
on the iMac, blocked VTKHDF everywhere). KoflCraft's binary hand-off and
FoehnCraft's `fields.vtkhdf` are written this way; `murmel blocks make`
converts a `.vtu` or a `polyMesh` once, and `murmel bench make --formats
blocked` writes a synthetic mesh of polyhedra of any size up to 684 M
cells straight into one, without the mesh in memory (the 100 M cell
test file).

A blocked file is an ordinary VTKHDF 2.4 file. VTK 9.6 and later
(`vtkHDFReader`) open it as a partitioned data set; nothing below needs a
MurmelCraft-specific reader except the optional speed-ups. Checked on
2026-11-25 with the VTK wheels 9.4.2, 9.5.2, 9.6.2 and 9.7.2: 9.4 and 9.5
crash on the polyhedra (type 42), so ParaView 6.0 (built on VTK 9.5) and
earlier open only a file of linear cells, and 9.4 only since the text is
marked ASCII (§ 8). Written from
the VTK documentation ("VTK File Formats", section "VTKHDF File Format",
version 2.4) and the HDF5 File Format Specification 3.0.

## 1. Version and root

- `/VTKHDF` is a group with the attributes `Version` = `[2, 4]` (two
  64-bit integers) and `Type` = `"PartitionedDataSetCollection"`, and an
  `Assembly` group as the VTKHDF documentation describes.
- Block `internal` (attribute `Index` = 0, `Type` =
  `"UnstructuredGrid"`): the volume cells, one **partition per block**.
- One block per boundary patch (`Index` = 1, 2, … in patch order, `Type` =
  `"PolyData"`), named after the patch, one partition each.
- Root attributes of `/VTKHDF`, all optional except the first:
  - `murmelcraft_blocks` = 1 (64-bit integer): the layout version. A
    file without it is read as an ordinary VTKHDF file (whole).
  - `block_order` = `"hilbert"`. Another order is allowed; regions then
    touch more blocks.
  - Digests and versions (`koflcraft_*`, § 8). MurmelCraft keeps every
    string attribute of `/VTKHDF` as mesh metadata.
- The attributes of a run (`foehncraft_*`, `case`, `units`, `step`,
  `time`, `iteration`) stay where FoehnCraft puts them, on the file's root
  group `/`. MurmelCraft reads them there, and keeps the `foehncraft_*`
  and `koflcraft_*` strings it finds there as mesh metadata too.

## 2. Blocks

- **Order.** The owned cells are sorted by the 3-D Hilbert index of their
  centroid, the mean of the cell's points.
  - Each axis of the bounding box of all mesh points is scaled on its own
    to the integers 0 to 2²¹ − 1 as ⌊(x − min) / (max − min) × (2²¹ − 1)⌋,
    so the index is 63 bits. An axis of zero extent maps to 0.
  - The index comes from Skilling's transpose algorithm ("Programming the
    Hilbert curve", AIP Conf. Proc. 707, 2004): `AxestoTranspose` on
    (x, y, z), then the bits read from the top bit plane down, x, y, z
    within each plane.
  - Ties are broken by the writer's own cell number.
- **Size.** Consecutive runs of exactly 65,536 owned cells; the last block
  holds the rest. Writers may pick another fixed size from 16,384 to
  1,048,576 and must write it as the `internal` attribute `block_cells`
  (64-bit integer). MurmelCraft reads any size.
- **Cell ids.** A cell's *file cell id* is its position among the owned
  cells, block by block: block 0's owned cells are 0 … n₀ − 1, then
  block 1's, and so on. Fields, `OwnerCell` and ghost references use it.
- **Boundary faces** are not in `internal`. They live in the patch blocks
  (§ 7), each face with the file cell id of its owner cell.

## 3. Points and ids

- Each partition lists the points its cells use, owned cells and ghosts
  alike, with local ids 0 … n − 1. `Connectivity` and `FaceConnectivity`
  use these local ids. A point shared by several blocks is repeated in
  each of them.
- `PointData/GlobalPointIds` (64-bit integer, one per partition point):
  the point's id in the whole mesh, any numbering where each point has
  one id. It joins the blocks and the patches without matching
  coordinates.
- `CellData/GlobalCellIds` (64-bit integer, one per partition cell): the
  file cell id. For an owned cell this is its own id; for a ghost cell,
  the id of the owned cell it copies.
- `CellData/OriginalCellIds` (64-bit integer, optional): the writer's own
  cell number, for tracing a cell back to KoflCraft's or FoehnCraft's
  order.

## 4. Ghost layer

- Each partition also holds one layer of **point neighbours**: every cell
  owned by another block that shares at least one point with an owned
  cell of this block.
- In each partition the owned cells come first, then the ghosts.
  `NumberOfCells` counts both, as VTK requires.
- `CellData/vtkGhostType` (unsigned 8-bit): 0 for an owned cell, 1
  (`DUPLICATECELL`) for a ghost.
- `PointData/vtkGhostType` (unsigned 8-bit): 0 for a point this block
  owns, 1 (`DUPLICATEPOINT`) for a point another block owns. A point is
  owned by the lowest-numbered block whose owned cells use it.
- Ghost cells and duplicate points carry the same field values as their
  originals. A region then reads only the blocks its cells lie in, and
  the halo (decided on card 6) comes with them.

## 5. Cells and polyhedra

All integers are 64-bit signed little-endian unless stated otherwise.
Writers have one choice: the ids that count within a partition
(`Offsets`, `Connectivity`, `FaceOffsets`, `FaceConnectivity`,
`PolyhedronOffsets` and `PolyhedronToFaces`) may be 32-bit signed
instead, all six alike, as a partition of at most 1,048,576 owned cells
and their ghosts stays far below 2³¹ of them (decided by the maintainer
on 2026-11-25 for the 100 M cell test file: a file of polyhedra takes
about 940 bytes a cell instead of 1,500). Readers take either, VTK's
`vtkHDFReader` too. The counts (`NumberOf…`) and the ids of the whole
mesh (`GlobalPointIds`, `GlobalCellIds`, `OriginalCellIds`, `OwnerCell`)
stay 64-bit.

- The `internal` block holds the standard UnstructuredGrid datasets.
  Each partition's rows follow the previous one's.
  - `NumberOfPoints`, `NumberOfCells` and `NumberOfConnectivityIds` hold
    one value per partition.
  - `Points` is 64-bit float, rows × 3.
  - `Types` is unsigned 8-bit.
  - `Offsets` holds `NumberOfCells + 1` values per partition, starting at
    0.
  - `Connectivity` holds local point ids.
- Linear cells keep their own VTK types: tetra 10, hexahedron 12, wedge
  13, pyramid 14, with their points in VTK 9's order: the base (0, 1, 2)
  or (0, 1, 2, 3) turns towards the rest of the cell, as VTK 9's face
  tables have it, so that VTK finds positive volumes. A cell whose faces
  are not exactly those of a linear cell (warped into another shape, a
  collapsed point) is a polyhedron. Polyhedra are type 42 in the 2.4
  layout:
  - `NumberOfFaces`, `NumberOfPolyhedronToFaceIds` and
    `NumberOfFaceConnectivityIds` hold one value per partition.
  - `FaceConnectivity` holds local point ids of each face, oriented with
    its normal out of the cell.
  - `FaceOffsets` holds `NumberOfFaces + 1` values per partition.
  - `PolyhedronToFaces` holds local face ids.
  - `Connectivity` of a polyhedron holds its distinct points, in the
    order its faces first use them.
  - `PolyhedronOffsets` holds `NumberOfCells + 1` values per partition;
    a non-polyhedral cell has an empty range.

## 6. Block index and ranges (`internal/FieldData`)

The `nblocks` below is the number of partitions.

| Dataset | Type | Shape | Holds |
|---|---|---|---|
| `BlockOwnedCells` | 64-bit integer | `nblocks` | owned cells of each block (the ghosts follow them) |
| `BlockBounds` | 64-bit float | `nblocks × 6` | x, y, z minimum then maximum of the points of the block's owned cells |
| `BlockRange_<field>` | 64-bit float | `nblocks × 2` | minimum and maximum of a cell or point field over the block's owned cells or owned points; the magnitude for a vector |

The block bounds pick the blocks a region needs. The ranges colour the
overview and skip blocks an iso-surface value cannot cross, the span-space
idea of Livnat, Shen and Johnson (1996). A file without ranges still
works; MurmelCraft finds them once and keeps them in its cache folder.

## 7. Boundary patches

- Each patch block is a VTKHDF PolyData with one partition:
  - `NumberOfPoints` and `Points` hold the patch's points, local.
  - `PointData/GlobalPointIds` is required in a blocked file.
  - The `Polygons` group holds `NumberOfCells`, `NumberOfConnectivityIds`,
    `Offsets` and `Connectivity`.
  - The `Vertices`, `Lines` and `Strips` groups are empty, with
    `NumberOfCells` = 0.
- Each face is oriented with its normal out of the domain.
- `CellData/OwnerCell` (64-bit integer, one per face): the file cell id
  of the face's owner. Its block is found from `BlockOwnedCells`, so a
  boundary face picked in the overview leads to its block. The overview
  reads each cell field at these rows to colour the boundary, so a
  file of 100 M cells opens without reading its cells
  (`vtkhdf::read_overview`; docs/performance.md, "The overview of a
  mesh beyond the memory budget").
- The patch's role is an optional string attribute `role` on the patch
  group: `wall`, `inlet`, `outlet`, `symmetry`, `farfield`, `periodic`,
  … When it is missing, MurmelCraft guesses the role from the name, as it
  does today.
- Boundary fields (wall shear stress, y⁺) are cell datasets of the patch,
  one value per face.

## 8. Zones, names and metadata

- `internal/CellData/zone` (unsigned 32-bit, one per partition cell): the
  zone of each cell. KoflCraft's `.vtu` calls this array `region`; in
  VTKHDF it is `zone`, as the reader already expects.
- `internal/FieldData/zone_names`: a 1-D string dataset of `id=name`
  entries, the same form as the `.vtu` field data (`0=fluid`, `1=rotor`).
- Patch names are the patch block names. No `boundary_names` is needed.
  If a writer also keeps `boundary_names` and `boundary_roles` (`id=…`,
  ids counted from 0 in patch order), MurmelCraft takes the roles from
  them when a patch has no `role` attribute.
- Digests and versions are root string attributes of `/VTKHDF`
  (`koflcraft_version`, `koflcraft_mesh_sha256`, …), not field data.
- **Strings:** fixed-length (null-terminated or null-padded) or
  variable-length (global heap), ASCII or UTF-8. Fixed-length is
  preferred. Text that is ASCII is marked ASCII: VTK 9.4 refuses a
  `Type` attribute marked UTF-8.

## 9. HDF5 features MurmelCraft reads

- **Superblock** 0, 2 or 3. **Object headers** version 1 or 2. **Links**
  compact or dense. **Attributes** compact or dense.
- **Groups track the creation order of their links** (and index it):
  VTK's `vtkHDFReader` lists a group's links in creation order, and the
  HDF5 library refuses that order for a group that does not track it.
- **Contiguous is required** for every topology and id dataset of
  `internal`:
  - `Points`, `Types`, `Offsets` and `Connectivity`
  - the polyhedron arrays
  - `GlobalPointIds`, `GlobalCellIds`, both `vtkGhostType` arrays and
    `zone`

  These datasets must be uncompressed, so a block is one positional read
  per dataset.
- **Fields** may be contiguous, or chunked with at least 65,536 rows per
  chunk. Filters deflate (level 6 or less), shuffle and Fletcher-32 are
  allowed. The chunk index may be a version 1 B-tree, a single chunk,
  implicit, or an unpaged fixed array.
- **Not allowed:**
  - unlimited (resizable) dimensions: their extensible-array and version
    2 B-tree chunk indices are not read, and h5py chooses them for any
    `maxshape=None`
  - szip, n-bit and scale-offset filters
  - virtual datasets and external files
- **Byte order:** little-endian throughout.

## Readers in MurmelCraft

- `read_vtkhdf` reads a blocked file whole, as any VTKHDF file: the
  ghost cells are left out and the blocks join by `GlobalPointIds`.
- `vtkhdf::read_overview` reads the patches and the block index alone,
  with each cell field at the rows of the cells behind the boundary
  faces (§ 7), for a file beyond the memory budget.
- `vtkhdf::read_blocks` reads the cells of some blocks: each block's
  partition alone, its owned cells and, with the halo, its ghosts (§ 4)
  that no block read owns, with the patch faces their cells own. It
  needs `GlobalPointIds` (§ 3), and `GlobalCellIds` for the halo; a
  block number the file does not have is BLOCK005. An overview reads
  its blocks through one `vtkhdf::BlockFile`: the file opened once, the
  `OwnerCell` of every patch face read once, and each block's patch
  faces read by their rows (with their `GlobalPointIds` and boundary
  fields), instead of every patch whole for every block.
- `vtkhdf::read_preview` reads the preview grid of the overview: each
  cell field of the volume on a regular grid of about 128 cells along
  the longest side of the blocks' box, a grid cell showing the cell its
  centre lies in, from the points and point lists of the owned cells
  and the fields at the rows found (docs/performance.md, "The preview
  grid of an overview").
- Slices, iso-surfaces and arrows on an overview draw on its preview
  grid (the engine's `preview.rs`), read the first time one shows, until
  a region reads the cells of its blocks. The grid is kept in the cache
  folder, so the file opened again unchanged is not sampled again
  (docs/performance.md, "The preview grid kept").
- The exact faces of an overview's slices and iso-surfaces are then cut
  through the blocks of the file, one block with its halo at a time, and
  take the grid's place (the engine's `cut_blocks.rs`): a slice cuts the
  blocks whose `BlockBounds` its planes cross, an iso-surface those whose
  `BlockRange_<field>`, with the ranges of the blocks whose bounds touch
  theirs, holds its value (§ 6); a field without ranges cuts every block
  (docs/performance.md, "Exact slices and iso-surfaces on an
  overview").
- The eye of "Block boxes" in the model tree (the item `blocks`) draws
  each block's `BlockBounds` as a box (the engine's `block_boxes.rs`);
  the blocks a region reads take a colour of their own, so you see what
  a region will read. Blocks of a Hilbert order have boxes that overlap,
  so a region touches more of them than the blocks its cells lie in
  alone.
- A box, sphere or slab region drawn on an overview reads, with
  `read_blocks`, the blocks whose `BlockBounds` it touches, with their
  halo (the engine's `blocks.rs`); beyond the memory budget it is
  refused with BLOCK006.
- A probe outside the cells shown reads the block that holds its point,
  with its halo: the blocks whose `BlockBounds` hold the point, until one
  of their owned cells holds it (the engine's `probe_blocks.rs`). The
  last four blocks read stay for the next probes (docs/performance.md,
  "Probes on an overview").
- Streamlines on an overview are traced through the blocks of the file
  (the engine's `stream_blocks.rs`): a seed is looked for in the blocks
  whose `BlockBounds` hold it, and a line walking into a halo cell goes
  on from the same cell in the block that owns it, read then with its
  halo. A trace reads at most as many blocks as a quarter of the memory
  budget holds; a line that needs another ends there (`unread`, with
  BLOCK006). The blocks gone through stay for the next trace
  (docs/performance.md, "Streamlines on an overview").

## Writers

- `murmel blocks make MESH --out FILE [--block-cells N]` (the tool
  `blocks.make`) writes a `.vtu`, an OpenFOAM `polyMesh` or a VTKHDF file
  with its fields in this layout. It holds the mesh in memory while it
  writes; a converter for meshes beyond the memory budget comes later.
  Its HDF5 writer is MurmelCraft's own (`crates/murmel-mesh/src/h5write.rs`,
  from the HDF5 File Format Specification 3.0): superblock 2, version 2
  object headers, compact links and attributes, contiguous datasets.
- Opening a `.vtu`, an OpenFOAM `polyMesh` or an unblocked VTKHDF file
  whose whole read needs more than the memory budget (estimated from the
  head of its files: the `Piece`'s `NumberOfCells`, the `nCells` OpenFOAM
  notes in `owner`, or the VTKHDF structure) asks first (card 4): convert
  it once to `NAME.blocked.vtkhdf` beside it (a case's in its folder,
  named after the case), which then opens as an overview, or open it
  whole. Converting runs as `blocks.make` does, on a thread of its own
  with Cancel or Esc, into a `.part` file renamed when whole; a copy
  newer than every file of the mesh opens at once next time. "Convert
  large meshes without asking" (in the question or Settings, or
  `settings.set_memory` with `convert_large`) skips the question. The
  tools open such a mesh whole, with the warning BLOCK007 naming the
  copy.
- KoflCraft writes its binary hand-off this way, FoehnCraft its
  `fields.vtkhdf`.
- Choices the layout leaves to the writer, which a reader must not
  depend on: the order of a block's points (MurmelCraft sorts them by
  global id, KoflCraft takes them in order of first use) and whether a
  patch without faces has a block.

## Example

A 200,000-cell hexahedral mesh with one zone and the patches `inlet`,
`outlet` and `walls` has four blocks. Their owned cells are 65,536,
65,536, 65,536 and 3,392. Each block adds about 10 % ghost cells.

```
/VTKHDF                Version=[2,4] Type="PartitionedDataSetCollection"
                       murmelcraft_blocks=1 block_order="hilbert"
  /Assembly
  /internal            Index=0 Type="UnstructuredGrid" block_cells=65536
    NumberOfPoints[4]  NumberOfCells[4]  NumberOfConnectivityIds[4]
    Points[Σp,3]  Types[Σc]  Offsets[Σc+4]  Connectivity[Σk]
    CellData/  zone[Σc] GlobalCellIds[Σc] vtkGhostType[Σc] pressure[Σc] …
    PointData/ GlobalPointIds[Σp] vtkGhostType[Σp]
    FieldData/ BlockOwnedCells[4] BlockBounds[4,6] BlockRange_pressure[4,2]
               zone_names["0=fluid"]
  /inlet               Index=1 Type="PolyData" role="inlet"
    NumberOfPoints[1] Points[…,3] PointData/GlobalPointIds
    Polygons/{NumberOfCells,NumberOfConnectivityIds,Offsets,Connectivity}
    Vertices/ Lines/ Strips/ (empty)
    CellData/OwnerCell
  /outlet …  /walls …
```
