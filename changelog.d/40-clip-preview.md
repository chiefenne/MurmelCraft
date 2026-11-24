### Changed

- Dragging a clip plane on a large mesh no longer waits for the cut to be built: the GPU cuts the surface at once (the cut faces look hollow for a moment) and the exact cut follows about a second after the plane stops (it took 1.1 s per step at 4 M cells). Screenshots, exports, picks and agents always see the exact cut. `murmel bench` reports `clip.step` and `clip.exact`.
