### Added

- View cube, top right of the viewport: faces, edges and corners fly to all 26 views in 250 ms, a drag orbits, the face or edge looked along is tinted. Above it Home and the projection menu (Fusion's three modes), beside it two arrows that roll the view by 90 degrees, below it rotation by any angle about the screen X, Y or Z axis.
- `view.standard` takes all 26 cube views (`front`, `front_top`, `front_right_top`, …); `view.rotate` turns the scene about a screen axis; the camera has a roll (`roll_deg`).
- Render modes (keys 1 to 4, toolbar): shaded with edges, shaded, wireframe, feature edges (creases over 30 degrees, patch borders, open borders); tool `display.render_mode`.
- Command palette (Ctrl + K, or the search button in the title bar): every command by name with its key, fuzzy search, arrows and Enter. C opens it on Colour by.
- Keyboard map from one command table (`docs/keys.md`, `murmel keys`): Alt + 1, 3, 7 front, right and top (Ctrl + Alt for the opposite views), 5 cycles the projection, F1 shows the shortcut sheet, Shift + arrows pan, Ctrl + left drag rolls (three-button preset).
- "Save a screenshot…" in the palette writes the current view as a PNG through `view.screenshot`.
