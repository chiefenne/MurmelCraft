+++
covers = ["window.color_by", "display.color_by*", "display.set_color", "ui.color_by"]
related = ["quality", "field-style", "render-mode"]
chapter = "display"
picture = "color-by.png"
shot = "shots/color-by.json"
+++
Colour the mesh by one solid colour, by patch, by patch role, by cell type, by cell zone, by a field of the results, or by category (one colour per block). Each item's colour can be changed in the model tree.

<!-- more -->

## Colour by

- **Solid colour**: one colour for the whole mesh (green by default).
- **Patch** and **patch role** (wall, inlet, outlet, symmetry, …) show the boundary conditions.
- **Cell type** shows the polyhedra, prisms, hexahedra and others; useful with a clip plane.
- **Cell zone** shows the zones of the mesh.
- **Field** colours by a result, with a colour bar.
- **Category** gives every whole-number value of a cell field its own colour, such as **BlockId** of KoflCraft's block meshes: one colour per block. See also [Explode](explode).
- **Quality** has a page of its own.

Right-click an item in the tree to give it your own colour.
