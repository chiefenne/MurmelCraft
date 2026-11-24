### Added

- Probes show the value of the field colouring the mesh on their label in the view and on their row in the model tree (`|velocity| 12.3 m/s`, `velocity_x …`, `pressure … Pa`), following the field and component you switch to. Properties still lists every field, derived and formula fields included, and a probe on a wall also lists the boundary fields there (y+, wall shear). `probe.add`, `probe.at` and `measure.list` give it as `shown`.
- The Probe history window (the Probes group's "History…" button, or the palette): every probe's value of a field over all time steps of the open series, read in the background with progress and Cancel, derived and calculated fields made again for each step. The step shown is marked, hovering lists a step's values, and "Export CSV…" writes them. The tools `probe.history` and `probe.export` do the same from the command line and for agents (PROBE001 to PROBE003).

### Changed

- A probe reads a cell field as the cell's value plus its least-squares gradient to the probe's point, as a finite-volume solver reconstructs it, and a node field through the cell's linear fit to its nodes. Both are exact for fields linear in space and keep a cell's peak, which the inverse-distance mean of node values flattened.
