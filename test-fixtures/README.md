# Test fixtures

KoflCraft hand-off meshes (`foehncraft.vtu`, KoflCraft `docs/foehncraft.md`),
made with KoflCraft (MIT, github.com/chiefenne/KoflCraft) at commit `9da116f`
on 2026-11-23. Nothing here was edited by hand.

| Folder | Made with | What it checks |
|---|---|---|
| `box/` | `kofl generate test-fixtures/box-4.json box.mesh.json`, then `kofl export box.mesh.json foehncraft foehncraft.vtu` | 4 Voronoi polyhedra, 6 named patches with role `unspecified`, the digests as field data |
| `channel/` | `kofl generate-channel test-fixtures/straight-channel.json channel` (`foehncraft.vtu` and `mesh-evidence.json` of the bundle) | 776 cells with prism layers, patches `inlet`, `outlet`, `walls` with roles, the evidence beside the mesh |
| `blocks/tetrahedron.vtu` | KoflCraft at commit `193b555` (2026-11-27): `kofl call block.template` with `{"config": {"template": "tetrahedron", "cells": [6, 6, 6, 6]}}`, then `kofl call block.grid`; the `volume.vtu` of the output | a regular tetrahedron as four hexahedral blocks (864 cells), cell fields `BlockId`, `I`, `J`, `K`: colour by category and the exploded view |

## FoehnCraft results (`vtkhdf/`)

Written by `python3 -I test-fixtures/vtkhdf/make_vtkhdf.py` with h5py (BSD)
on the HDF5 library, so MurmelCraft's own HDF5 and VTKHDF readers are
checked against an independent writer. The meshes are `box/` and
`channel/` above; the layout follows FoehnCraft's `fields.vtkhdf` (MIT,
github.com/chiefenne/Foehncraft, `docs/interfaces.md`, "Files of a run").
The fields are analytic functions of the cell and face centres, so the
tests know every value.

| File | What it checks |
|---|---|
| `vtkhdf/channel_fields.vtkhdf` | a PartitionedDataSetCollection with the cells in `internal` and one PolyData block per patch; cell fields velocity (vector), speed, pressure, k, omega, nu_t with units; boundary fields wall_shear_stress and y_plus; the run's root attributes (iteration 250) |
| `vtkhdf/box_chunked.vtkhdf` | the 4 Voronoi polyhedra of `box/` with 14 cell fields (two chunks each, deflate with the shuffle filter), units in one attribute, step and time |
| `vtkhdf/box_latest.vtkhdf` | the same written with the newest HDF5 file format (`libver="latest"`): compact datasets, a single-chunk dataset, a big-endian dataset and many root attributes |

## Blocked VTKHDF from KoflCraft (`vtkhdf/koflcraft/`)

Written by KoflCraft's own writer of the blocked layout
(`docs/blocked-vtkhdf.md`, layout 1), KoflCraft PR #38 (MIT, branch
`claude/project-thread-3olzu6`, 2026-11-24), and copied unchanged, so the
blocked reader is checked against a writer that is not MurmelCraft's.
KoflCraft checked them with h5py 3.16 and VTK 9.7.2.

| File | What it checks |
|---|---|
| `vtkhdf/koflcraft/channel.vtkhdf` | the mesh of `channel/foehncraft.vtu` (776 cells) in one block, patches with their `role` attributes |
| `vtkhdf/koflcraft/box4.vtkhdf` | the mesh of `box/foehncraft.vtu` (4 Voronoi cells), 6 patches, the `koflcraft_*` digests as attributes of `/VTKHDF` |

## OpenFOAM polyMesh cases (`openfoam/`)

| Folder | Made with | What it checks |
|---|---|---|
| `openfoam/box/constant/polyMesh` | KoflCraft (MIT) at commit `5b283ff` on 2026-11-23: `kofl generate test-fixtures/box-4.json box.mesh.json`, then `kofl export box.mesh.json openfoam box-of`; the files of `box-of` copied unchanged | KoflCraft's ascii polyMesh with `cellZones` (zone `fluid`) and `mesh-evidence.json` with a digest per file: the same mesh as `box/foehncraft.vtu`, and the green badge |
| `openfoam/channel_ascii` | Copied unchanged from FoehnCraft (MIT, github.com/chiefenne/Foehncraft, `crates/foehncraft-mesh/tests/data/openfoam`, commit `d4f1878`); made there with OpenFOAM's `blockMesh`, `topoSet` and `setsToZones` (FoehnCraft `tools/mesh/openfoam_test_meshes.md`) | 16 hexahedra, patches `inlet`, `outlet`, `walls` (type wall), `sides` (type empty), cell zones `right` and `left` |
| `openfoam/channel_binary` | As above, after `foamFormatConvert` with `writeFormat binary` | binary lists and `faceCompactList`, `arch "LSB;label=32;scalar=64"` |
| `openfoam/dual_polyhedra` | As above: a Gmsh tetrahedral cube, `gmshToFoam`, `polyDualMesh 80 -overwrite` | 81 true polyhedra, no `cellZones` |

The files are data written by the OpenFOAM utilities, not OpenFOAM source;
no OpenFOAM code was read. The gzip case of the reader tests compresses
`channel_binary` at run time.

A larger mesh for trying the window (not in the repository, 1 MB):
`kofl generate-duct test-fixtures/elbow/elbow.stl test-fixtures/elbow/patches.json test-fixtures/elbow/partition.json polyhedral elbow`
writes `elbow/foehncraft.vtu` (1,082 polyhedral cells).

## Tables of a spray run (`spray-run/`)

Written by FoehnCraft (MIT, github.com/chiefenne/Foehncraft) at the head of
pull request #265 (`8c1efc5`, stage SP1b, 2026-11-24): `foehn flow
examples/cases/flow/spray_datums.toml --out <dir>`, a 3-second run of
the gallery case `spray_datums` (one injector `nozzle`, six surface
datums and one box). The CSV files are copied unchanged (three of the six
histograms); `result.json` keeps only the first solve's `datums` and
`spray` (the end-of-run statistics), cut by a Python script, no number
changed. Layouts: FoehnCraft `docs/interfaces.md`, "Files of a run".

| File | What it checks |
|---|---|
| `spray-run/datums.csv` | format `foehncraft-datums 1`: 7 datums (6 surfaces with liquid columns, the volume `nozzle_box`), a row every 10 steps |
| `spray-run/spray.csv` | format `foehncraft-spray 1`: the mass book every step and injector `nozzle` with its penetration |
| `spray-run/spray_hist_{outlet,floor,x06}.csv` | format `foehncraft-spray-histogram 1`: 40 log bins from 1 µm to 1 mm plus the under and over rows, per interval |
| `spray-run/result.json` | the end-of-run statistics of the datums and the spray |
