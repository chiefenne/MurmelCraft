+++
covers = ["display.color_by:quality*", "display.threshold*", "inspect.quality", "ui.quality"]
related = ["color-by", "clip"]
chapter = "display"
+++
Colour each cell by its worst value of a mesh quality metric: non-orthogonality, skewness, volume ratio or face count. A threshold hides the good cells, so only the worst remain.

<!-- more -->

## Mesh quality

- **Non-orthogonality**: the angle between a face's normal and the line joining the cells on both sides.
- **Skewness**: how far the face centre lies from that line.
- **Volume ratio**: the largest volume ratio to a neighbour.
- **Face count**: the faces of each cell.

The legend shows the range and a histogram. **Show only the worst 10 % of cells** sets the threshold there; **Show all cells again** clears it. With a clip plane, the bad cells inside the mesh show too.
