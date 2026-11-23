# Test fixtures

KoflCraft hand-off meshes (`foehncraft.vtu`, KoflCraft `docs/foehncraft.md`),
made with KoflCraft (MIT, github.com/chiefenne/KoflCraft) at commit `9da116f`
on 2026-11-23. Nothing here was edited by hand.

| Folder | Made with | What it checks |
|---|---|---|
| `box/` | `kofl generate test-fixtures/box-4.json box.mesh.json`, then `kofl export box.mesh.json foehncraft foehncraft.vtu` | 4 Voronoi polyhedra, 6 named patches with role `unspecified`, the digests as field data |
| `channel/` | `kofl generate-channel test-fixtures/straight-channel.json channel` (`foehncraft.vtu` and `mesh-evidence.json` of the bundle) | 776 cells with prism layers, patches `inlet`, `outlet`, `walls` with roles, the evidence beside the mesh |

A larger mesh for trying the window (not in the repository, 1 MB):
`kofl generate-duct test-fixtures/elbow/elbow.stl test-fixtures/elbow/patches.json test-fixtures/elbow/partition.json polyhedral elbow`
writes `elbow/foehncraft.vtu` (1,082 polyhedral cells).
