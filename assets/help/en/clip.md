+++
covers = ["clip.*", "select.item:next_clip", "ui.clip_handle", "ui.clip"]
related = ["regions", "slices", "quality"]
chapter = "display"
picture = "clip.png"
shot = "shots/clip.json"
+++
Cut the mesh open with clip planes to see inside. Clip along X, Y or Z, along the view direction, or anywhere with the handles: drag the arrow to slide the plane and the arcs to tilt it.

<!-- more -->

## Clip planes

- **Clip normal to X, Y or Z**, or **along the view direction**.
- **Add a clip plane** adds another; several planes cut together.
- **Flip the clip side** swaps the half that is cut away.
- **Crinkle cut** shows whole cells along the cut, so cell shapes stay visible. **Flat cut** cuts the cells in the plane.
- Drag the **arrow** to slide the plane, the **arcs** to tilt it. Alt with the arrow keys tilts it in 5° steps.
- **Turn the clip plane on or off**, **Remove the current clip plane**, **Remove all clip planes**.

While you drag, the GPU shows the cut at once, and the exact cut follows.
