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
  "format_version": "1.2",
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
    "clips": [
      {"origin": [2.01, 1, 0.5], "normal": [1, 0, 0], "mode": "flat", "enabled": true},
      {"origin": [2, 1, 0.5], "normal": [0, -1, 0], "mode": "crinkle", "enabled": false}
    ]
  },
  "colors": {"solid": "#336699"},
  "view_cube": {"placement": "bottom_left", "auto_hide": false, "distance_px": 120, "position": null},
  "views": [
    {"name": "Inlet close up", "camera": {"target": [0, 1, 0.5], "distance": 2.4, "yaw_deg": -40,
      "pitch_deg": 15, "projection": "perspective_ortho_faces", "fov_deg": 30, "snapped": false, "roll_deg": 0}}
  ]
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
- **Clip planes** (format 1.2) are `display.clips`, in the order they
  were added; `clip:1` in `selection` is the first. A 1.0 or 1.1 file has
  one plane in `display.clip`, which loads as `clip:1`.
- **Fields** (format 1.4) are `display.field`, the field shown when
  `color_by` is `field` (`{"name": "velocity", "component": "magnitude"}`),
  and `display.field_styles`, one style per field: `mode` (`flat`,
  `smooth`, `banded`), `bands` (2 to 64), `lines` and `line_count` (1 to
  64), `map` (a colour map id, or left out for automatic), `reverse` and
  `range` (`field`, `shown`, `symmetric` or `{"fixed": {"min": 0, "max": 2}}`).
  A field the result does not have is skipped as `field:<name>` and the
  colour falls back to solid. The values themselves are read from the
  result file again, never stored.
- **Slices and iso-surfaces** (format 1.5) are `display.slices`
  (`{"origin": [2, 1, 0.5], "normal": [1, 0, 0], "enabled": true}`, plus
  `count` and `spacing` for a series of parallel planes centred on
  `origin`), `display.isos` (`{"field": {"name": "speed", "component":
  "magnitude"}, "value": 1.2, "enabled": true, "color": "solid"}`, with
  `color` `solid` or `field`; an own colour is in `colors` as `iso:1`)
  and `display.mesh_look` (`faces`; left out for the default, the faint
  outline while slices or iso-surfaces show). At most 8 of each and 64
  planes in a series. An iso-surface of a field the result does not have
  is skipped as `field:<name>`. The geometry is computed again on
  loading, never stored.
- **Vector arrows** (format 1.6) are `display.arrows` (`{"field":
  "velocity", "enabled": true}`, plus `surface` and `in_plane` when on,
  `density` (4 to 200, left out for 25), `scale` (above 0 up to 100, left
  out for 1) and `color` (`solid`, the default, or `field`); an own
  colour is in `colors` as `arrows`). Arrows of a field the result does
  not have, or of a scalar, are skipped as `field:<name>`. The arrows are
  computed again on loading, never stored.
- **Regions of interest** (format 1.7) are `display.regions`, in the
  order they were added (`region:1` is the first): `{"shape": {"kind":
  "box", "center": [2, 1, 0.5], "size": [1, 1, 1]}, "enabled": true}`.
  The kinds are `box` (`center`, `size`), `sphere` (`center`, `radius`),
  `slab` (`origin`, unit `normal`, `thickness`), `zone` (`zone`, a cell
  zone's name), `patch` (`patch`, `layers` from 1 to 64) and `cells`
  (`cells`, ids, and `rings` from 0 to 64). At most 16. A region whose
  zone, patch or cells the mesh lacks is skipped as `region:<n>`. The
  cells a region takes are found again on loading, never stored.
- **Saved views** (format 1.1) are the named cameras of the model tree's
  "Saved views" (`views.save`, `views.go`, `views.rename`,
  `views.delete`), in the order they were saved. A 1.0 file has none and
  still loads; a file without views leaves the key out. Names are 1 to 64
  characters, unique, without control characters.
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
colours the tokens take, saved views with valid unique names and valid
cameras) and fails with `SESS001` without changing
anything. Then it opens the mesh, unless the same mesh with the same
SHA-256 is already open, and restores the rest. It still finishes, with a
warning in `warnings`, when:

- `SESS003`: the mesh's SHA-256 differs from the saved one (the mesh was
  written again);
- `SESS004`: some ids are not in the mesh; they are skipped and listed in
  `skipped`.

The window shows the first warning in the status bar, in amber.
`session.save` fails with `SESS002` when the file cannot be written.
