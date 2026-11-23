### Added

- Colour by quality (V0.5): each cell's worst face non-orthogonality (degrees) or skewness (after Jasak 1996 and Ferziger and Perić), its largest volume ratio to a neighbour, or its face count, on a sequential ramp of our own (pale yellow, good, to deep red, bad; token `mesh.quality`, configurable through `settings.set_colors`). `display.color_by` (version 2) takes `metric`.
- Quality legend in the viewport: the metric, a histogram of the cells over the metric's range on the ramp, the range, a threshold slider that hides the cells better than it (`display.threshold`) and a "Worst 10 %" button. The surface is rebuilt when a slider drag ends.
- `inspect.quality`: min, max, mean and a histogram of a metric over all cells, and the number of cells at or above a value.
- Palette: colour by each quality metric, "show only the worst 10 % of cells" and "show all cells again".
