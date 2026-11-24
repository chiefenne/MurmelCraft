### Added

- Saved views: Cmd + B or Ctrl + B saves the current camera as a named view ("View 1", "View 2" …) under "Saved views" in the model tree. Click a view to fly there, double-click to rename it; its context menu also updates it to the current camera or deletes it. V and Shift + V step through the saved views.
- Saved views are undo steps and go into session files (`.murmel` format 1.1; 1.0 files still load). Opening another mesh starts with none.
- Colour by cell zone: the toolbar's "Colour by" and the palette have "Cell zone", which gives each OpenFOAM `cellZone` (or VTU region) its own colour; a zone's context menu in the tree has "Colour by cell zone", and a colour set on a zone wins.
- The tree's "Zones" group is now called "Cell zones".
- The same works from the CLI and MCP: `views.save`, `views.go`, `views.rename`, `views.delete`, `views.list`, and `display.color_by` with `by: zone`.
