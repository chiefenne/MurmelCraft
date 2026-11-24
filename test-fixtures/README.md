# Test fixtures

KoflCraft hand-off meshes (`foehncraft.vtu`, KoflCraft `docs/foehncraft.md`),
made with KoflCraft (MIT, github.com/chiefenne/KoflCraft) at commit `9da116f`
on 2026-11-23. Nothing here was edited by hand.

| Folder | Made with | What it checks |
|---|---|---|
| `box/` | `kofl generate test-fixtures/box-4.json box.mesh.json`, then `kofl export box.mesh.json foehncraft foehncraft.vtu` | 4 Voronoi polyhedra, 6 named patches with role `unspecified`, the digests as field data |
| `channel/` | `kofl generate-channel test-fixtures/straight-channel.json channel` (`foehncraft.vtu` and `mesh-evidence.json` of the bundle) | 776 cells with prism layers, patches `inlet`, `outlet`, `walls` with roles, the evidence beside the mesh |

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
