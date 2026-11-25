+++
covers = ["window.settings", "settings.set_theme*", "settings.set_view_cube*", "settings.set_colors*", "settings.set_agents", "settings.set_memory", "settings.get", "ui.view_cube_settings"]
related = ["shortcuts", "defaults"]
chapter = "getting-started"
+++
Settings holds the theme (dark, light or following the system), the mesh colours, where the view cube sits, the memory budget for large meshes, and whether agents may drive the window. Each change is saved to settings.toml at once.

<!-- more -->

## Settings

- **Theme**: dark (the default), light, or follow the system.
- **Colours**: the solid mesh colour, patch palette, role and cell-type colours, the quality ramp, clip handles and measurements, each per theme. *Reset colours* goes back to the built-in ones.
- **View cube**: a corner, floating, hidden, or hidden until the pointer comes near.
- **Memory budget**: meshes that need more open as an overview.
- **Let agents drive the window**: `murmel mcp --live` can then call tools in this window, with a badge and a Stop button while one does.

The settings folder is shown in the window; set `MURMEL_CONFIG_DIR` to use another.
