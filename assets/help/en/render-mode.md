+++
covers = ["display.render_mode*"]
related = ["color-by", "mesh-look"]
chapter = "display"
+++
How faces are drawn: shaded with edges, shaded, wireframe, or feature edges only. Feature edges show the outline of the geometry without the mesh lines.

<!-- more -->

## Render mode

- **Shaded with edges**: faces and the mesh's edges, to judge the cells.
- **Shaded**: faces only, for pictures of the shape or of a field.
- **Wireframe**: edges only.
- **Feature edges**: only the edges where the surface folds sharply, and its borders.

While you orbit a large mesh, edges are left out if a frame would take longer than 33 ms.
