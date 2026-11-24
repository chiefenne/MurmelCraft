# Exporting pictures and movies

Pictures of the view for reports, slides and posters, and movies: the
view rendered offscreen at the size you choose, with the colour legend
and the axis triad drawn over it. Decided by the maintainer on cards
(2026-11-24): the legend and the axes are each optional, the background
can be transparent, lines, legend and text grow with the scale so a 2x
picture looks like the window ("same look as on screen"), PNG frames and
GIF are written by MurmelCraft itself and MP4 by the system `ffmpeg`, and
a flight through saved views eases into each view and can hold there.
A movie of a time series (motion `series`, D268) plays the steps of the
series opened with `series.open`, the camera still.

| Where | How |
|---|---|
| Window | File, "Export image…" (Ctrl + Shift + E, Cmd + Shift + E on macOS): size, resolution, legend, axes, transparent background, then Export… asks for the file. "Export movie…" (Ctrl + Shift + M): motion, length, frame rate, format, size and overlays; the movie is made in the background with a progress bar and Cancel. |
| CLI | `murmel call export.image --input '{"path": "view.png", "scale": 2}'` after opening a mesh in the same session (`murmel run --script`), see below; `export.movie` the same way. |
| MCP | the same tools; the files must lie below a write root ([mcp.md](mcp.md)). Over the live bridge a movie is made in the window's background and must end within two minutes (`LIVE002`); the headless server has no such limit. |

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

## `export.movie`

Every frame is rendered as `export.image` renders a picture, with the same
`width`, `height`, `scale`, `legend`, `axes`, `transparent`, `line_width`
and `theme`. The camera, the clip planes and the theme go back as they
were; the movie is no undo step.

| Field | Default | Meaning |
|---|---|---|
| `path` | (required) | A `.mp4` or `.gif` file, or a folder for PNG frames (`frame-00001.png`, …), made if missing. |
| `format` | from the path | `mp4`, `gif` or `png`. |
| `motion` | (required) | `orbit`: the camera turns about the vertical axis through its target. `clip`: a clip plane sweeps through the mesh. `views`: the camera flies through saved views. `series`: the steps of the open time series one after the other (`series.open`), the camera still; the step shown before the movie is shown again after it. |
| `fps` | 30 | Frames a second, 1 to 60. A GIF times frames in hundredths of a second, so it plays at most 50 (the result says the rate it plays at). |
| `seconds` | 6 | `orbit` and `clip`: the length. At most 36000 frames (20 minutes at 30). |
| `degrees` | 360 | `orbit`: the angle. A whole turn leaves out the last frame, which equals the first, so a looping movie does not stutter. |
| `plane` | the current one | `clip`: the plane, counted from 1; it is switched on for the movie. |
| `from`, `to` | the ends of the plane's slider | `clip`: offsets along the plane's normal from the centre of the mesh's bounds, from where the plane cuts nothing to where it cuts everything. `series`: the first and the last step (default the series' first and last; `to` before `from` plays backwards); a step the series lacks is refused (`SERIES003`). |
| `views` | all, in the order they were saved | `views`: the names of two saved views or more. |
| `hold` | 0.5 | `views`: seconds at each view (at least one frame). `series`: seconds each step shows, default one step at the player's rate (4 steps a second: 0.25 s). |
| `travel` | 2 | `views`: seconds of each flight; it eases out of one view and into the next (smoothstep in time, shortest-arc rotation, as the window's view flights). |

The fields of another motion are refused (`TOOL002`), so a mistyped call
is not silently another movie.

| Format | Written by | Notes |
|---|---|---|
| PNG frames | MurmelCraft (`png`, MIT OR Apache-2.0) | Lossless; for editing or your own encoder. Transparency kept. |
| GIF | MurmelCraft (`gif`, MIT OR Apache-2.0, with `color_quant`, MIT) | Loops; up to 256 colours a frame (NeuQuant when a frame has more); transparency is one bit (soft edges become solid). |
| MP4 | the `ffmpeg` installed on the machine | H.264 (x264 at CRF 18, else Apple's VideoToolbox, else ffmpeg's MPEG-4 part 2), yuv420p tagged BT.709, the index at the front so it plays while it loads. No transparency; an odd side loses one pixel (H.264 needs even sides). |

MurmelCraft neither links nor ships ffmpeg; it runs the program and pipes
the frames to it. It looks for `MURMEL_FFMPEG` (the path of the program),
then `ffmpeg` on the PATH, in `/opt/homebrew/bin` and in `/usr/local/bin`
(a Mac app started from the Finder does not see the shell's PATH). Without
one the call fails with `EXPORT002`; on macOS `brew install ffmpeg`.

A turn, a flight and a time series from the command line:

```sh
cat > movie.json <<'JSON'
[
  {"tool": "session.load", "input": {"path": "rotor.murmel"}},
  {"tool": "export.movie", "input": {"path": "rotor-turn.mp4", "motion": "orbit", "seconds": 8, "width": 1920, "height": 1080}},
  {"tool": "export.movie", "input": {"path": "rotor-tour.gif", "motion": "views", "hold": 1, "travel": 2, "fps": 25, "scale": 1}},
  {"tool": "series.open", "input": {"path": "output/spray_duct"}},
  {"tool": "export.movie", "input": {"path": "spray.mp4", "motion": "series", "hold": 0.1, "fps": 30}}
]
JSON
murmel run --script movie.json
```

| Code | Meaning |
|---|---|
| `EXPORT001` | A setting is out of range: frame rate, length, frame count, a transparent MP4. |
| `EXPORT002` | MP4 without ffmpeg. |
| `EXPORT003` | ffmpeg failed; the message ends with what it said. A half-written MP4 is removed. |
| `EXPORT004` | A movie file or frame could not be written. |
| `EXPORT005` | Nothing to move: no clip plane, fewer than two saved views, or an unknown view name. |
| `EXPORT006` | Cancelled. A half-written MP4 or GIF is removed; PNG frames written stay, and the message says how many. |
| `EXPORT007` | The window makes one movie at a time. |

Time steps of field results come later, when MurmelCraft reads them.

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
the window's legend reads too. A movie renders all its frames on one
offscreen renderer and writes each as it is made, so memory stays at one
frame whatever the length.
