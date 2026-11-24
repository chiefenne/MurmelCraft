# Session files

A session file keeps what the viewer shows, so a view of a mesh can be
saved, shared and opened again (UI spec, viewer tools: `session.save`,
`session.load`). It is JSON with the extension `.murmel`.

| Where | How |
|---|---|
| Window | Ctrl + S saves (it asks for a file the first time, then saves there again); Ctrl + Shift + S saves under a new name. Ctrl + O opens session files as well as meshes; the palette has "Load session…". Dropping a `.murmel` file on the window, or `murmelcraft view.murmel`, loads it. |
| CLI | `murmel call session.save --input '{"path": "view.murmel"}'`, `murmel call session.load --input '{"path": "view.murmel"}'` |
| MCP | the same tools; the mesh inside a session file must lie within the server's roots ([mcp.md](mcp.md)). |

## What it holds

```json
{
  "format": "murmelcraft.session",
  "format_version": "1.0",
  "mesh": {
    "path": "foehncraft.vtu",
    "absolute": "/cases/channel/foehncraft.vtu",
    "sha256": "…"
  },
  "camera": {"target": [2, 1, 0.5], "distance": 6.1, "yaw_deg": -12, "pitch_deg": 22.8,
             "projection": "perspective_ortho_faces", "fov_deg": 30, "snapped": false, "roll_deg": 0},
  "display": {
    "color_by": "quality", "render_mode": "shaded", "metric": "skewness", "threshold": 0.2,
    "pick": "cell",
    "hidden": ["patch:walls"], "isolated": [], "selection": ["patch:inlet", "cell:7"],
    "colors": {"patch:outlet": "#123456"},
    "clip": {"origin": [2.01, 1, 0.5], "normal": [1, 0, 0], "mode": "flat", "enabled": true}
  },
  "colors": {"solid": "#336699"},
  "view_cube": {"placement": "bottom_left", "auto_hide": false, "distance_px": 120, "position": null}
}
```

- **Mesh.** `path` is relative to the session file's folder, so a case
  folder moved as a whole still opens; `absolute` is tried when the
  relative path does not exist (a session file moved on its own). A
  multi-file mesh (an OpenFOAM `polyMesh`) has the digest of all its files,
  as `mesh.info` shows it. A session without a mesh closes the open one.
- **Items** are named by the ids of the tree and the tools (`patch:walls`,
  `zone:core`, `kind:prism`, `cell:7`, `face:12`), never by index, so a
  mesh written again with the same names keeps its view.
- **Colours and the view cube** (`colors`, `view_cube`) are the user's
  settings when the session was saved. Loading applies them to this
  session only; `settings.toml` is not written (decided 2026-11-24). A
  later `settings.*` call writes the session's settings, those included.
- **Not kept:** the theme and the mouse bindings (the person's, not the
  view's), the undo journal (a loaded session starts a new one) and the
  quality values (computed again when colour mode quality needs them).

## Loading

`session.load` checks the whole file first (the format, a 1.x version,
finite numbers, a positive camera distance, a non-zero clip normal,
colours the tokens take) and fails with `SESS001` without changing
anything. Then it opens the mesh, unless the same mesh with the same
SHA-256 is already open, and restores the rest. It still finishes, with a
warning in `warnings`, when:

- `SESS003`: the mesh's SHA-256 differs from the saved one (the mesh was
  written again);
- `SESS004`: some ids are not in the mesh; they are skipped and listed in
  `skipped`.

The window shows the first warning in the status bar, in amber.
`session.save` fails with `SESS002` when the file cannot be written.
