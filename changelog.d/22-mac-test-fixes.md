### Changed

- The clip plane's tilt arcs have their own colour, a little more violet than the slide arrow, in both themes; `settings.set_colors` sets either with `gizmo.dark` and `gizmo.light` (`slide`, `tilt`).
- While a tilt arc is hovered or dragged, its rotation plane shows as a faint disc through the plane's centre with ticks every 15 degrees (longer every 45), the tilt axis as a dashed line and, while dragging, a wedge of the angle turned so far.
- On macOS, tooltips, menus, the shortcut sheet (F1) and the palette name the keys Cmd and Option instead of Ctrl and Alt; `docs/keys.md` says so.

### Fixed

- Option + 1, 3 and 7 (and Cmd + Option for the opposite views) now work on Mac keyboard layouts where Option + digit types a character (Option + 7 is `|` on a German layout): shortcuts with Alt match the key's place on the keyboard too.
- The model tree's right-click "Colour…" opens a colour window (hue, saturation and brightness sliders, hex code, the patch palette, Reset colour, Done); the colour button inside the menu closed with the menu and seemed to do nothing.
- README: a copy-paste setup of the MCP server for Claude Code on macOS (`claude mcp add --scope user …` with the full path to `target/release/murmel`), and what to do when it fails.
