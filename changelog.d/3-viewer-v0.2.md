### Added

- Model tree on the left (T): the mesh with a validity badge from its mesh-evidence.json, then zones, cell types and patches with counts, colour swatches and eye toggles. Click selects, double click fits, the context menu isolates, shows all, fits or copies the name; Ctrl + F filters by name.
- Properties on the right (P): read-only rows for the mesh (file, KoflCraft version, cells, interior and boundary faces, volume, relative volume error, inverted faces, SHA-256) and for a patch, zone or cell type (role, FoehnCraft type, faces, area, mean normal and spread, bounds). Click a value to copy it.
- Colour by patch, patch role or cell type, from the toolbar, with a legend in the viewport. The role and cell-type palettes stay apart under simulated protanopia, deuteranopia and tritanopia.
- Keys: H hides the selection, I isolates it, Alt + H shows all, Esc clears it.
- Tools `display.color_by`, `display.visibility`, `select.item`, `select.clear` and `inspect.item`; `view.fit` takes an item.

### Changed

- The patch palette is reordered so the first patches differ most, also for colour-blind viewers.
- Keys work again after clicking a toolbar button.
