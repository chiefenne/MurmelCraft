### Added

- Several clip planes (up to 8): "Add clip plane" in the tree's Clip planes group, Alt + C or the palette adds one. Every plane that is on cuts; flat planes slice in the order they were added, so two flat planes take a corner out of the mesh with both sections shown.
- Each plane has its own row ("Clip 1: X · Crinkle"): the eye turns it on or off, a click makes it the current plane (the toolbar, the drag handles and the clip keys act on it), double-click fits it, and the context menu flips it, switches crinkle or flat, aligns it to the view or removes it. The Properties panel shows its cut, origin, normal and offset.
- Palette: "Next clip plane", "Remove the current clip plane", "Remove all clip planes".
- Session files keep every plane (format 1.2, `clips`); 1.0 and 1.1 files load their single plane as `clip:1`.
- The same from the CLI and MCP: `clip.set` with `plane` or `new`, `clip.clear` with `plane` or `all`, `clip.rotate` with `plane`, items `clip:1`, `clip:2` in `select.item`, `display.visibility`, `view.fit` and `inspect.item`.
