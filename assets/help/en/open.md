+++
covers = ["window.open", "mesh.close", "mesh.open", "recent.*"]
related = ["import", "sessions", "series"]
chapter = "getting-started"
+++
Open a KoflCraft mesh (`foehncraft.vtu`, `foehncraft.vtkhdf`), a FoehnCraft result (`.vtkhdf`) or a MurmelCraft session (`.murmel`). The mesh-evidence.json beside the mesh is read with it, and the view fits the mesh. Files of other tools come in through File, Import.

A mesh larger than the memory budget opens as an overview: its boundary only, with each field on it. Regions of interest read the cells you look at.

<!-- more -->

## Opening

- **Open…** lists the files of our three tools: MurmelCraft sessions, KoflCraft meshes and FoehnCraft results. Other files are greyed out (on Linux and Windows the dialog hides them).
- The title bar and Properties name the tool that wrote the file: KoflCraft mesh, FoehnCraft result, MurmelCraft blocked mesh. A `.vtu` or `.vtkhdf` that none of our tools wrote still opens, tagged *VTK, imported*.
- You can also drop a file on the window, or start `murmelcraft FILE`. A file of another tool opens as an import.
- **File ▸ Open recent** lists the files opened lately, the latest first, with a tag saying where each came from. A file that is gone is greyed out. **Clear the list** forgets them.

While a large mesh is read, the status bar shows its progress, and Esc stops it. The files are checked with BLAKE3 after the mesh opens. A file that is unchanged since it was last checked is not hashed again.

**Close mesh** drops the mesh and everything shown on it. The undo steps go with it.

Scripts and agents call `mesh.open` with the same paths; its result says `origin` and `imported`.
