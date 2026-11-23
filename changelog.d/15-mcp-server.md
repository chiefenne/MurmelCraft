### Added

- An MCP server for agents (UI spec V1.0): `murmel mcp --read-root DIR --write-root DIR` (or `--root DIR`, or the program `murmel-mcp`) offers every tool of the registry by its dotted id over stdio (MCP 2025-11-25, also 2025-06-18), the same tools the window and `murmel call` use. One headless session lasts the whole connection, so the mesh, camera, display, clip plane, selection and undo journal carry from call to call. See docs/mcp.md for the setup in Claude Code and Claude Desktop.
- `view.screenshot` over MCP returns the PNG as an image the agent sees; `path` may be left out, and the server writes `screenshots/shot-NNNN.png` below the first write root.
- Paths in tool inputs are confined: reads below a read or write root, writes below a write root, links followed, relative paths from the first write root. New diagnostic codes MCP001 (outside the roots) and MCP002 (path does not resolve). The settings tools change the MCP session only and never write the person's config files.
- The tool catalogue (`system.commands`) lists the input fields each tool reads as paths (`reads`), beside the files it writes (`outputs`).
