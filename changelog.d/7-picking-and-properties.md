### Added

- Picking (`select.pick`): a click in the viewport picks the cell under the cursor, or the face with the toolbar's Faces filter (`select.filter`); Shift + click adds to the selection or takes an item out again. A click on a clip section picks its cell. The selection is drawn as an outline on top of the mesh.
- Cells and faces as items (`cell:N`, `face:N`): they can be selected, hidden, isolated and framed like zones and patches (context menu: Hide, Isolate, Fit selection, Show all).
- Cell properties: type, zone, faces, vertices, volume, centroid, neighbours, maximum non-orthogonality and skewness. Face properties: owner, neighbour, patch, role, area, normal, centre, non-orthogonality and skewness (after Jasak 1996 and Ferziger and Perić).
- Hover: resting the cursor for 0.4 s shows the cell or face under it (`inspect.at`); the status bar shows the cursor's coordinates on the mesh.

### Changed

- `select.item` (version 2) takes `add`; the selection is a list, and `display.*` results carry it with the isolated items and the pick filter.
