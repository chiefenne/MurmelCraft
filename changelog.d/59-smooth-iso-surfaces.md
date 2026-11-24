### Changed

- Iso-surfaces are smooth. Values between cells are now rebuilt from each cell's least-squares gradient (fitted to the cells that share a node with it), instead of the inverse-distance mean of the cell values, which waved even on a straight-line field. A cell predicts the value at a node or a face centre from its own value and gradient, the predictions are averaged, and the result stays within the values of the cells around it, so no new extremes appear. A field linear in space is now exact: its iso-surface is a flat plane (it had 2.3 % too much area). On uneven cells the shading normals of a round iso-surface are about 9 times closer to the true ones (1.0° instead of 9.2° on average).
- Iso-surface normals follow the field's slope, interpolated from the cell gradients, so the shading is smooth across the facets.
- Smooth colouring, slices, streamlines and probes use the same rebuilt values, so every view still shows the same field. Pictures of results change slightly: banded colours of a linear field are now straight.

### Fixed

- An iso-surface could have holes where a face centre crossed the value but the cell's own corners did not. That cell is no longer passed over.
- An iso-surface could cap a tube at a large inlet or outlet face, because that face's centre took the mean of its rim. A boundary face's centre now follows its cell.
