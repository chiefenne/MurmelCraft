+++
covers = ["region.*", "ui.region", "ui.blocks"]
related = ["clip", "open"]
chapter = "display"
picture = "regions.png"
shot = "shots/regions.json"
+++
A region of interest reads and shows only the cells inside a box, sphere or slab, or a picked zone or patch. On a mesh too large for memory it reads just the blocks it needs.

<!-- more -->

## Regions of interest

- **Show or hide**; a box is added if there is none.
- A **box**, a **sphere** or a **slab** between two planes.
- **The selected cells** and one ring around them, **the selected cell zone**, or three cell layers along **the selected patch**.
- **Cut flat along its border, or whole cells**.
- **Remove**, **Remove all**.

On an overview of a blocked VTKHDF file, the blocks the region needs are read on threads while the overview stays.
