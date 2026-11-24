### Changed

- The model tree and the legend no longer count the cells on every frame (49 ms a frame at 4 M cells), and picking tests boxes around runs of triangles before the triangles: a pick at 4 M cells takes 0.3 ms instead of 37 ms, so hover and measure snapping keep up on large meshes. `murmel bench` reports `tree.frame` and `surface.boxes`.
