+++
covers = ["view.projection*", "view.set_camera"]
related = ["views"]
chapter = "navigation"
+++
Three projections: perspective, orthographic, and perspective that switches to orthographic when you look straight at a face (the default). Orthographic keeps parallel edges parallel, which helps to compare sizes.

<!-- more -->

## Projection

- **Perspective with orthographic faces** (the default): perspective while you orbit, orthographic on the six standard faces.
- **Perspective**: always.
- **Orthographic**: always; distances across the view compare directly.

**Next projection mode** cycles through the three.
