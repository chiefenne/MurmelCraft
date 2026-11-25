File ▸ Open is for the files of our three tools: MurmelCraft sessions, KoflCraft
meshes and FoehnCraft results. Its dialog lists only `.murmel`, `.vtu` and
`.vtkhdf`, with no "All files", so other files are greyed out. Files of other
tools come in through the new File ▸ Import: OpenFOAM polyMesh… (a case,
`constant` or `polyMesh` folder) and VTK file…; neither has a default key, so
Ctrl+Shift+O is free (a key you gave the old "Open OpenFOAM case folder" stays
on the import). The title bar names the tool that wrote the open file, a drop
of another tool's file opens at once with "Imported …" in the status bar, and
each entry of Open recent carries a tag (KoflCraft, FoehnCraft, Session,
OpenFOAM, imported). Load session… leaves the File menu, since Open… loads
sessions; it stays in the palette. New help page: Import from other tools.
