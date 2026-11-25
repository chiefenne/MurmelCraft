+++
covers = ["window.import_openfoam", "window.import_vtk"]
related = ["open", "regions"]
chapter = "getting-started"
+++
Bring in a mesh written by another tool: an OpenFOAM polyMesh, or a VTK file from ParaView, VTK or others. It opens like one of ours, tagged *imported*.

<!-- more -->

## Importing from other tools

- **Import ▸ OpenFOAM polyMesh…** takes a case, `constant` or `polyMesh` folder (ascii, binary and gzip files, cell zones). A `.foam` file in the case works by drop or from the command line. A polyMesh that KoflCraft exported, with its evidence in the folder, is named a KoflCraft mesh and its files are checked one by one.
- **Import ▸ VTK file…** takes a `.vtu`, `.vtkhdf`, `.hdf` or `.h5` file.
- Dropping such a file on the window imports it at once; the status bar says so for a few seconds.

An imported mesh gets everything ours get: fields, clips, slices, probes, regions, and the offer to convert a large mesh to a blocked copy beside it. The copy is a MurmelCraft file and opens with **Open…**.

Neither entry has a key; give them one in **Shortcuts**. Scripts and agents use `mesh.open` for imports too.
