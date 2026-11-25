+++
covers = ["window.open", "window.open_folder", "mesh.close", "mesh.open", "recent.*"]
related = ["sessions", "series"]
chapter = "getting-started"
+++
Open a KoflCraft mesh (`foehncraft.vtu`), a FoehnCraft result (`.vtkhdf`) or an OpenFOAM case. The mesh-evidence.json beside the mesh is read with it, and the view fits the mesh.

A mesh larger than the memory budget opens as an overview: its boundary only, with each field on it. Regions of interest read the cells you look at.

<!-- more -->

## Opening

- **Open mesh…** takes a `.vtu`, a `.vtkhdf`, a `.foam` file or a polyMesh file.
- **Open OpenFOAM case folder…** takes a case, `constant` or `polyMesh` folder.
- You can also drop a file on the window, or start `murmelcraft FILE`.
- **File ▸ Open recent** lists the files opened lately, the latest first. A file that is gone is greyed out. **Clear the list** forgets them.

While a large mesh is read, the status bar shows its progress, and Esc stops it. The files are checked with BLAKE3 after the mesh opens. A file that is unchanged since it was last checked is not hashed again.

**Close mesh** drops the mesh and everything shown on it. The undo steps go with it.

Scripts and agents call `mesh.open` with the same paths.
