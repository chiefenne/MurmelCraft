# Exporting pictures and movies

Pictures of the view for reports, slides and posters, and (next) movies:
the view rendered offscreen at the size you choose, with the colour legend
and the axis triad drawn over it. Decided by the maintainer on cards
(2026-11-24): the legend and the axes are each optional, the background
can be transparent, lines, legend and text grow with the scale so a 2x
picture looks like the window ("same look as on screen"), PNG frames and
GIF are written by MurmelCraft itself and MP4 by the system `ffmpeg`.

| Where | How |
|---|---|
| Window | File, "Export image…" (Ctrl + Shift + E, Cmd + Shift + E on macOS): size, resolution, legend, axes, transparent background, then Export… asks for the file. |
| CLI | `murmel call export.image --input '{"path": "view.png", "scale": 2}'` after opening a mesh in the same session (`murmel run --script`), see below. |
| MCP | the same tool; the PNG must lie below a write root ([mcp.md](mcp.md)). |

## `export.image`

| Field | Default | Meaning |
|---|---|---|
| `path` | (required) | The PNG to write. |
| `scale` | 1, or the height over the viewport's height | Pixels per point of the window. Without `width` and `height` the picture is the viewport times the scale. Lines, legend and text are drawn this many times their size in the window. 0.25 to 8. |
| `width`, `height` | the viewport times the scale | Pixels, up to 8192 a side. One side alone keeps the viewport's aspect. The camera frames the view by its height, so a wider picture shows more at the sides. |
| `legend` | true | The colour legend, top left: the swatches of patches, roles, cell types or zones, or the colour bar of quality (later field values) with nice ticks (Heckbert, Graphics Gems 1990). |
| `axes` | true | The axis triad, bottom left, X red, Y green, Z blue. |
| `transparent` | false | No background: alpha 0 where nothing is drawn; the silhouette's soft edge keeps the faces' colour. |
| `line_width` | the scale (at least 1) | Edge width in pixels, 1 to 32. |
| `dpi` | none | Resolution stored in the PNG (pHYs), so a 3000-pixel picture at 300 dpi prints 10 inches wide. |
| `theme` | the session's | `dark`, `light` or `system` for this picture only. |

The view, the colouring, the clip planes and the hidden items are the
session's; the export changes nothing (it is no undo step).

A 4K picture of a session from the command line:

```sh
cat > shot.json <<'JSON'
[
  {"tool": "session.load", "input": {"path": "rotor.murmel"}},
  {"tool": "export.image", "input": {"path": "rotor-4k.png", "width": 3840, "height": 2160, "dpi": 300}}
]
JSON
murmel run --script shot.json
```

## How it is drawn

The mesh is drawn by the same wgpu renderer as the window, into an
offscreen texture of the picture's size (4x multisampling). Lines wider
than one pixel are drawn as screen-aligned quads (`vs_wide` in
`crates/murmel-render/src/mesh.wgsl`); one-pixel lines keep the line list,
so the window is unchanged. With a transparent background the GPU's edge
pixels are premultiplied in linear light; they are divided by alpha before
the PNG is written. The legend and the triad are drawn on the CPU with
vello_cpu (Apache-2.0 OR MIT, the renderer egui itself uses) in Inter
(SIL OFL), from the engine's legend model (`murmel_engine::legend`), which
the window's legend reads too.
