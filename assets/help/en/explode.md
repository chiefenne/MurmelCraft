+++
covers = ["display.explode*"]
related = ["color-by", "clip", "visibility"]
chapter = "display"
+++
Draw the blocks of a block-structured mesh, or the cell zones, apart from each other, each as a closed piece. Good for seeing how a block topology fits together, in 2-D and 3-D.

<!-- more -->

## Explode

- **Explode** in the toolbar turns it on; the number beside it sets how far the pieces move (0.3 is a good start, up to 4).
- The pieces come from a cell field of whole numbers, such as **BlockId** in KoflCraft's block meshes (`volume.vtu`), or from the cell zones when there is no such field.
- Each piece moves away from the centre of the mesh. A 2-D mesh one cell deep stays in its plane.
- Combine it with **Colour by: category** to give every block its own colour, and with a clip plane to look inside.
- Press **Explode** again to put the pieces back.
