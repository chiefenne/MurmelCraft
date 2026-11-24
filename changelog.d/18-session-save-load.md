### Added

- Session files: Cmd + S or Ctrl + S saves what you see as a `.murmel` file. That covers the mesh, camera, colouring, quality threshold, hidden and isolated items, selection, item colours, clip plane, mesh colours and view cube placement. Cmd + Shift + S or Ctrl + Shift + S saves it under a new name.
- Opening a `.murmel` file (Cmd + O or Ctrl + O, the palette's "Load session…", drag and drop, or `murmelcraft view.murmel`) brings the view back.
- The mesh is found relative to the session file first, so a case folder can move.
- A mesh written again since the save, or items it no longer has, give an amber warning in the status bar, and the rest still loads.
- The saved palette and view cube apply to that session only; your settings are not changed.
- The same works from the CLI and MCP (`session.save`, `session.load`). Over MCP, the mesh a session file names must lie within the server's roots. See docs/sessions.md.
