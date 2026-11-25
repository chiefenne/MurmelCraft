`mesh.open` (version 6) says where a file comes from: `origin` names the tool
that wrote it (koflcraft, foehncraft, murmelcraft, openfoam, vtk), read from
the marks in the file, and `imported` is true for an OpenFOAM polyMesh and for
a VTK file none of our tools wrote. Properties names the file the same way
("KoflCraft mesh (.vtu)", "OpenFOAM polyMesh (binary), imported"), and
`recent.list` (version 2) keeps the origin of each mesh for the tags in
File ▸ Open recent. The MCP server introduces MurmelCraft as the viewer for
KoflCraft meshes and FoehnCraft results that also imports OpenFOAM and VTK.
