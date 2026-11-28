# Changelog

## Unreleased

Nothing yet.

## Summary

| Version | Milestone | Highlights |
|---|---|---|
| 0.6.1 (2026-11-28) | V0.1 to V0.6 done, V1.0 in progress | Exploded view of the blocks of a KoflCraft mesh with a colour per block; derived statistics, helicity, swirling strength, turbulence intensity and quantities of the time means; FoehnCraft's wall heat fields through `foehn post derive`; slices and iso-surfaces read three blocks at once (a 10 M cell slice over a gigabit share 48 s instead of 56 s); ready-to-run programs for macOS, Linux and Windows on every release; install guide for the three systems |
| 0.6.0 (2026-11-26) | V0.1 to V0.6 of the UI spec done, V1.0 in progress | First release. The desktop viewer of KoflCraft meshes and FoehnCraft results: model tree, picking, properties, view cube, clip planes with handles, regions of interest, cell quality, measurements and probes; fields with slices, smooth iso-surfaces, vector arrows, streamlines, derived fields and a field calculator; spray parcels, time series and run charts; OpenFOAM and VTK import, an own HDF5 reader for VTKHDF, blocked files and overviews of meshes larger than memory; sessions, undo, pictures, movies, print and PDF; one tool registry for the window, the command line and an MCP server that can also drive the open window. **A 1 million cell ascii `.vtu` opens in 3.3 s instead of 12.8 s; at 4 million cells a flat clip cut builds in 0.66 s and the cell quality in 2.1 s on 4 threads.** |

## 0.6.1 (2026-11-28) – exploded blocks, more derived fields, release downloads

**In one paragraph.** MurmelCraft 0.6.1 stays at the V0.6 milestone of the
UI spec and adds to it: the blocks of a KoflCraft block mesh (or the cell
zones) can be drawn apart, each in its own colour; the derived fields gain
the turbulence statistics of FoehnCraft's `statistics.vtkhdf`, helicity,
swirling strength, turbulence intensity and the quantities of the time
means, and the `foehn post derive` runner offers FoehnCraft 1.3's wall heat
fields; slices of large files on a network share are faster. Every release
now carries programs for macOS (an app in a `.dmg`), Linux and Windows,
and the README and `docs/install.md` say how to install on each system.
Sessions of 0.6.0 read unchanged (format now 1.19).

### All changes

### Results and derived fields

- **Derived statistics, vortex and turbulence quantities.** Statistics
  group for FoehnCraft's `statistics.vtkhdf`: resolved and total kinetic
  energy, the resolved Reynolds stresses, the Lumley anisotropy invariants
  η and ξ (Pope 2000) and the turbulent heat flux ⟨u'T'⟩; helicity,
  normalised helicity and the swirling strength λ_ci; the turbulence
  intensity against U_ref and against the local speed. A mean switch takes
  the quantities of velocity, pressure and k from the time means
  (`mean_<field>`, default `[defaults.derived] mean`, diagnostic FIELD021).
  The `foehn post derive` runner lists `solid-fraction`. Session format 1.19.
- **FoehnCraft's wall heat fields.** The `foehn post derive` runner lists
  `wall-heat-flux` (`wall_heat_flux`, `wall_temperature`),
  `heat-transfer-coefficient` and `nusselt-number` of FoehnCraft 1.3 under
  Wall, on the walls.
- **Exploded view and colour by category.** `display.explode` draws the
  blocks of a block mesh (the whole-number cell field `BlockId` that
  KoflCraft writes) or the cell zones apart, each as a closed piece, in
  2-D and 3-D; `display.color_by` `category` gives every block its own
  colour with a legend. Toolbar toggle with the factor, palette entries,
  help page; sessions keep both (format 1.18).

### Speed

- **Three blocks read at once.** Exact slices and iso-surfaces on an overview read three blocks at once while one is being cut, and still cut them in the order of the blocks, so the faces are the same as before. A file on a network share is read with several requests on the way instead of one, which keeps the link busy: over a stand-in for a gigabit NAS a slice through a 10 M cell file takes 48 s instead of 56 s, and the link is full. It holds up to two blocks more than before (about 0.4 GB at 10 M).

### Downloads, install and docs

- **Release downloads**: the new workflow Executables builds MurmelCraft
  for macOS (one universal build for Apple silicon and Intel:
  `MurmelCraft.app` in a `.dmg`, and the programs in a `.tar.gz`), Linux
  x86-64 and Windows x86-64, and attaches them with `SHA256SUMS.txt` to the
  release; the Release workflow starts it after each release, and it can be
  run by hand for an existing one (docs/release.md). The README's
  "Download a release" says which file to take and how to open the
  unsigned Mac app the first time.
- The Windows release build of `murmelcraft` is a window program: started
  from the Explorer it opens no console window beside it.
- **Install section in the README.** How to build MurmelCraft from source
  (prerequisites on macOS and Linux, clone, build, the three programs in
  `target/release/`, `cargo install`, update), with a placeholder for
  release downloads. The minimum Rust in the badge is corrected from 1.85
  to 1.98 (`AtomicU32::from_mut_slice` is stable since 1.98).
- **Install guide for macOS, Linux and Windows.** `docs/install.md` lists
  the tools with the install commands per system (Xcode tools; apt and dnf;
  winget and Chocolatey), the GPU driver each system needs (Metal, Vulkan
  with `mesa-vulkan-drivers`, DirectX 12), ffmpeg per system and how to
  check the Vulkan driver. MurmelCraft needs no CMake, HDF5 or Gmsh. The
  README's install section gives the short version per system, now also
  for Windows.

---

## 0.6.0 (2026-11-26) – first release: the viewer of KoflCraft and FoehnCraft

**In one paragraph.** MurmelCraft 0.6.0 is the first tagged release of the
shared viewer of KoflCraft (meshes) and FoehnCraft (results). The version
follows the milestones of the UI spec: V0.1 to V0.6 are done, and parts of
V1.0 (OpenFOAM meshes, the MCP server, sessions, export, the menu bar) are
already in. Every action of the window is a registered tool that the
command line and the MCP server share, so an agent can do what a user can.

### Highlights

- **Look at a mesh.** KoflCraft's `foehncraft.vtu` and `foehncraft.vtkhdf`
  with their `mesh-evidence.json` (validity badge, BLAKE3 or SHA-256 file
  checks), OpenFOAM `polyMesh` cases and VTK files through File ▸ Import;
  model tree with zones, cell types and patches; picking of cells and faces
  with their properties; colour by patch, role, cell type, zone or cell
  quality with a threshold for the worst cells; up to eight clip planes with
  drag handles, crinkle or flat cuts; regions of interest; view cube, saved
  views, undo and redo.
- **Look at results.** FoehnCraft's `fields.vtkhdf` (read by MurmelCraft's
  own HDF5 reader) and the data of a `.vtu`: flat, smooth or banded colours
  with contour lines and nine colour maps (Rainbow Desaturated by default,
  shared with FoehnCraft and KoflCraft); slices, smooth iso-surfaces,
  vector arrows and streamlines (also as tubes); derived fields (vorticity,
  Q, λ₂, total pressure, c_p, turbomachinery, turbulence and wall
  quantities with FoehnCraft's names) and a field calculator with `grad`,
  `curl` and `div`; FoehnCraft's own wall and turbulence fields (y⁺, wall
  shear stress, u_τ, …) computed by the `foehn` installed on the machine. A
  [post-processing guide](docs/post-processing.md) shows every step.
- **Sprays and transient runs.** Spray parcels as shaded spheres sized and
  coloured by any parcel field, time series with a player bar and movies of
  the steps, and charts and statistics of a run (datum mass flows,
  penetration, D32, size histograms) read without the solver.
- **Large meshes.** A memory budget: a mesh beyond it opens as an overview
  of its boundary, regions, slices, iso-surfaces, probes and streamlines
  read only the blocks of a blocked VTKHDF file they touch, previews come
  from the file's preview grid or from the GPU at once, and a large `.vtu`
  or OpenFOAM mesh is offered for conversion to a blocked copy. Opening,
  topology, quality, cuts and surfaces run on all threads (a 1 M cell ascii
  `.vtu` in 3.3 s instead of 12.8 s, about 27 % less memory per cell).
- **Agents, sessions and export.** `murmel mcp` (read and write roots) and
  `murmel mcp --live`, which drives the open window; session files;
  pictures, movies (GIF, MP4), print and PDF with the time of a series;
  configurable shortcuts, a menu bar, open recent, tips and a help with
  pictures; keyboard focus regions (F6) for accessibility.

### Known limits

- No prebuilt binaries yet: build from source with `cargo build --release`.
- Not yet in the window: the rest of the V1.0 milestone of the UI spec and
  everything of V2.
- Movies through MP4 need the system `ffmpeg`; render tests run on Mesa
  lavapipe in CI, the picture export needs a GPU or lavapipe.

### All changes

### Viewer, navigation and window

- First viewer (V0.1): open KoflCraft's foehncraft.vtu, shaded with edges in
  patch colours, orbit, pan, zoom to the cursor, fit and home, three projection
  modes, dark and light themes, mouse presets (three-button, Apple, Fusion,
  ParaView) saved in navigation.toml, and headless screenshots with
  `murmel call view.screenshot`.
- The window, the title bar and the README show the MurmelCraft logo (an engraved marmot on magenta) instead of the placeholder.
- The accent colour follows the logo's magenta: #AD3285 in the light theme, #DE74B6 in the dark theme (contrast 5.2:1 and 6.2:1 against the app background).
- Model tree on the left (T): the mesh with a validity badge from its mesh-evidence.json, then zones, cell types and patches with counts, colour swatches and eye toggles. Click selects, double click fits, the context menu isolates, shows all, fits or copies the name; Ctrl + F filters by name.
- Properties on the right (P): read-only rows for the mesh (file, KoflCraft version, cells, interior and boundary faces, volume, relative volume error, inverted faces, SHA-256) and for a patch, zone or cell type (role, FoehnCraft type, faces, area, mean normal and spread, bounds). Click a value to copy it.
- Colour by patch, patch role or cell type, from the toolbar, with a legend in the viewport. The role and cell-type palettes stay apart under simulated protanopia, deuteranopia and tritanopia.
- Keys: H hides the selection, I isolates it, Alt + H shows all, Esc clears it.
- Tools `display.color_by`, `display.visibility`, `select.item`, `select.clear` and `inspect.item`; `view.fit` takes an item.
- The patch palette is reordered so the first patches differ most, also for colour-blind viewers.
- Keys work again after clicking a toolbar button.
- Colour mode **Solid colour**, now the default: every boundary face in one green (#6EC64A, sampled from the maintainer's reference screenshot), edges darkened as before.
- Every mesh colour is configurable. `display.set_color` colours one item of the open mesh (mesh, zone, cell type or patch; also from the tree's context menu), and `settings.set_colors` saves the user's default colours (solid, patch palette, role and cell-type colours, edge darkening) in settings.toml (also from Settings, Colours).
- `settings.set_theme` keeps the saved colours; settings.toml holds both.
- View cube, top right of the viewport: faces, edges and corners fly to all 26 views in 250 ms, a drag orbits, the face or edge looked along is tinted. Above it Home and the projection menu (Fusion's three modes), beside it two arrows that roll the view by 90 degrees, below it rotation by any angle about the screen X, Y or Z axis.
- `view.standard` takes all 26 cube views (`front`, `front_top`, `front_right_top`, …); `view.rotate` turns the scene about a screen axis; the camera has a roll (`roll_deg`).
- Render modes (keys 1 to 4, toolbar): shaded with edges, shaded, wireframe, feature edges (creases over 30 degrees, patch borders, open borders); tool `display.render_mode`.
- Command palette (Ctrl + K, or the search button in the title bar): every command by name with its key, fuzzy search, arrows and Enter. C opens it on Colour by.
- Keyboard map from one command table (`docs/keys.md`, `murmel keys`): Alt + 1, 3, 7 front, right and top (Ctrl + Alt for the opposite views), 5 cycles the projection, F1 shows the shortcut sheet, Shift + arrows pan, Ctrl + left drag rolls (three-button preset).
- "Save a screenshot…" in the palette writes the current view as a PNG through `view.screenshot`.
- View cube: the face labels keep a 3 px gap to the grid lines (each label takes the largest size, up to its old one, that fits the face's centre patch with that gap, and none if even 8 px does not), and the cube is 124 px instead of 96 px so the labels stay readable in the iso view.
- View cube: the cube's edges are drawn 2 px wide and the grid lines between a face's patches 0.6 px, so edges and corners read as targets.
- Orbiting in the viewport turns about the point of the mesh under the cursor where the drag starts, or about the centre of the shown faces when the drag starts over empty space (UI spec, Viewport), so the part you grab stays under the pointer. `view.orbit` (version 2) takes `pivot`, a world point; without it the camera turns about its target as before (the view cube's drag and the arrow keys).
- The toolbar no longer hides its last group (Select, Faces) when the window is narrow or a label is long (for example colour by quality at 1280 px): when the full toolbar does not fit, it leaves out the group names and the "Colour by:" prefix (the full name stays in the tooltip and the accessible name).
- The Properties panel keeps its width: labels and values take fixed shares of it and are cut with an ellipsis, with the full text on hover, so a long value (a centroid, a neighbour list) no longer widens the panel.
- View cube placement (`settings.set_view_cube`, saved in settings.toml): any corner of the viewport (top right by default), a floating panel that you drag anywhere over the viewport by its grip, or hidden. With auto-hide on, the cube fades out while the pointer is farther from it than a set distance (120 px by default, 0 to 2000 px) and comes back as the pointer nears; it stays while you drag it or a menu of it is open.
- The placement is set from a new pull-down beside the cube's Home and projection buttons, from the View cube section of the Settings window, from the viewport's context menu (so a hidden cube can come back) and from the command palette.
- A cube in a left corner moves the view note and legend below it (top left) or the axis triad beside it (bottom left), so nothing is covered.
- Undo and redo of the camera and the display (UI spec: "Undo is the tool journal"): Ctrl+Z and Ctrl+Shift+Z (Cmd on macOS), and Undo and Redo in the command palette. Every call of a tool that changes the camera or the display (`view.*` moves, `display.*`, `clip.*`) is a step; one mouse gesture (all calls while a button is held) and one run of wheel or pinch steps make one step, so undo steps are gestures, not frames. Undoing a camera step flies back in 250 ms.
- The selection, the pick filter and the settings are not undone; opening or closing a mesh starts a new journal; 200 steps are kept.
- New tools `journal.undo`, `journal.redo`, `journal.group` (calls inside one group undo as one step, for scripts and the window) and `journal.list`.
- F6 and Shift + F6 move the keyboard focus between the regions of the window: title bar, toolbar, model tree, viewport, properties and status bar (hidden panels are skipped). Tab then moves from control to control, left to right in the title bar and the toolbar.
- A 2 px focus ring in the accent colour shows which control has the focus after keyboard use; a mouse press hides it.
- Screen readers (AccessKit): the panes have names, drop-downs and toggles name their setting and value, tree rows read with their count and whether they are hidden, property values with their label, and the status bar is a status node. No name holds an icon glyph any more (the Open button, the filter and the status bar's alert had one). See docs/accessibility.md.
- The arrow keys and Esc belong to a control the keyboard reached (tree rows, buttons, sliders); in the viewport and after a mouse click they orbit and clear the selection as before.
- The light theme's error colour is a little darker (#C03D2D, was #C7402F), so the status bar's error text reaches WCAG AA contrast (4.66:1).
- A message in the status bar goes away ten seconds after it first shows, or as soon as your next action works (a view or display change, yours or an agent's). Before, it stayed until the next error, so an agent's SET001 stayed red for the whole session.
- Zoom to box (rubber band): press W or the new toolbar button, drag a box over the part you want, and it fills the view. One box per press; Esc cancels. In perspective it zooms at the depth of the mesh inside the box, so that part really fills the view. The same zoom is the tool `view.zoom_box` for `murmel call` and MCP, and `zoom_box` is a new mouse action you can bind to a drag in `navigation.toml`.
- Every keyboard shortcut and mouse binding can be changed in the new Shortcuts window (Settings → Edit shortcuts…, the F1 sheet, or the palette: "Edit keyboard and mouse shortcuts"). On the Keyboard tab, find a command, click Add key and press the keys. On the Mouse tab, pick a preset or change each binding's action, button, gesture and the keys held.
- A key or mouse input that another command already has shows a warning that names it, with Replace (move it here) and Cancel. Nothing changes until you choose.
- Your keys are saved under `[keys]` in settings.toml, holding only the commands you changed, so new built-in keys of later versions still arrive. You can edit them there by hand: `"view.fit" = ["F", "Shift+F"]`. A clash or a mistyped key in the file is reported at start. Mouse bindings stay in navigation.toml.
- The same is the tool `settings.set_keys` for `murmel call` and MCP. It refuses a taken key (SET004) unless `replace` is true.
- Tooltips, menus and tree rows name the keys you set, not the built-in ones.
- `murmel call` and the MCP server kept the "offer to convert large meshes" setting when they rewrote settings.toml.
- A menu bar in the title row (File, Edit, View, Display, Results, Tools, Window, Help) holds the
  common commands; every command stays in the palette and keeps its keys. Items show your keys,
  items that cannot work now are greyed out and say why, and Undo and Redo name the step they walk.
  New: Quit (Ctrl + Q), Close mesh on Ctrl + W, Minimise (Ctrl + M), Full screen (F11), Edit ▸
  Undo history, Copy the view as an image (Ctrl + Shift + C), Help ▸ About and links to the tool
  reference, the agent setup and the issue tracker. Copying values now reaches the system clipboard.
- File ▸ Open recent lists the last ten meshes, sessions, time series, spray parcels and chart
  folders you opened, the latest first, each named by its file and folder; a file that is gone is
  greyed out. The list lives in recent.toml beside settings.toml and only the window adds to it, so
  scripts and agents leave it as it is. New tools: recent.list and recent.clear (also in the menu).
- Tips grow with the rest: after a moment (250 ms) a control's tip names it, says what it does and
  shows your key; after a longer rest (one more second), or at once while you hold Shift, it shows
  the first part of the control's help page. A control the keyboard reached shows the same tip.
  Settings ▸ Tips sets both delays, and Settings scrolls when it is taller than the screen. Behind
  the tips are 34 help pages (assets/help/en), one per topic, so every command has one; agents read
  them with the new help.get tool, and settings.set_help sets the delays.

  The manual opens in a Help panel at the right of the window: its pages by chapter, a search, Back,
  links between pages, and at the end of each page the commands it explains with your keys, each
  running from there. F1 opens the page of the tip you are looking at (or the tip's "More in the
  manual" link does), and F1 with no tip opens or closes the contents. The shortcut sheet moves from
  F1 to Ctrl + / (both keys can be changed in the Shortcuts window). Help ▸ Manual opens the panel.
- Ten help pages have a picture: render mode, colour by, quality, clip planes, slices, iso-surfaces,
  arrows, streamlines, field style and regions. It shows in the rich tip and on the page in the Help
  panel. Each picture is rendered from a recipe of tool calls (`assets/help/shots`) by
  `murmel help shots`, so the pictures show what the program draws and can be made again when it
  changes; a test notices a recipe edited without its picture made again.

### Mesh inspection: clip planes, picking, quality, regions

- Clip plane (`clip.set`, `clip.clear`): X, Y and Z clip normal to an axis through the centre, cutting away the half towards the camera; the palette also clips along the view direction. Crinkle cut keeps whole cells, flat cut slices them and shows each cell's section. Shift + C flips the side. The toolbar's Clip group has the axis buttons, the cut mode, Flip and a position slider; the model tree has a Clip planes row (eye, flip, cut, remove); a dashed outline and an arrow in the viewport show the plane, and dragging the arrow slides it.
- The mesh's face topology (owner and neighbour of every face). The viewport now draws the outside of the cells shown, so hiding a zone or a cell type lays open the cells behind it: hide the prisms to see the polyhedral core, isolate them to see the layers.
- `display.*` results carry the clip plane (`clip`: mode, origin, normal, offset, range).
- Picking (`select.pick`): a click in the viewport picks the cell under the cursor, or the face with the toolbar's Faces filter (`select.filter`); Shift + click adds to the selection or takes an item out again. A click on a clip section picks its cell. The selection is drawn as an outline on top of the mesh.
- Cells and faces as items (`cell:N`, `face:N`): they can be selected, hidden, isolated and framed like zones and patches (context menu: Hide, Isolate, Fit selection, Show all).
- Cell properties: type, zone, faces, vertices, volume, centroid, neighbours, maximum non-orthogonality and skewness. Face properties: owner, neighbour, patch, role, area, normal, centre, non-orthogonality and skewness (after Jasak 1996 and Ferziger and Perić).
- Hover: resting the cursor for 0.4 s shows the cell or face under it (`inspect.at`); the status bar shows the cursor's coordinates on the mesh.
- `select.item` (version 2) takes `add`; the selection is a list, and `display.*` results carry it with the isolated items and the pick filter.
- Colour by quality (V0.5): each cell's worst face non-orthogonality (degrees) or skewness (after Jasak 1996 and Ferziger and Perić), its largest volume ratio to a neighbour, or its face count, on a sequential ramp of our own (pale yellow, good, to deep red, bad; token `mesh.quality`, configurable through `settings.set_colors`). `display.color_by` (version 2) takes `metric`.
- Quality legend in the viewport: the metric, a histogram of the cells over the metric's range on the ramp, the range, a threshold slider that hides the cells better than it (`display.threshold`) and a "Worst 10 %" button. The surface is rebuilt when a slider drag ends.
- `inspect.quality`: min, max, mean and a histogram of a metric over all cells, and the number of cells at or above a value.
- Palette: colour by each quality metric, "show only the worst 10 % of cells" and "show all cells again".
- The clip plane can be moved and tilted in the viewport: drag the shaded
  arrow to slide it along its normal, drag one of the two arcs to tilt it
  about its centre (Shift for fine steps). Alt + arrows tilt it by 5°, and
  Alt + Shift + Up and Down slide it. Each drag is one undo step. Agents
  and scripts tilt it with the new tool `clip.rotate`.
- Several clip planes (up to 8): "Add clip plane" in the tree's Clip planes group, Alt + C or the palette adds one. Every plane that is on cuts; flat planes slice in the order they were added, so two flat planes take a corner out of the mesh with both sections shown.
- Each plane has its own row ("Clip 1: X · Crinkle"): the eye turns it on or off, a click makes it the current plane (the toolbar, the drag handles and the clip keys act on it), double-click fits it, and the context menu flips it, switches crinkle or flat, aligns it to the view or removes it. The Properties panel shows its cut, origin, normal and offset.
- Palette: "Next clip plane", "Remove the current clip plane", "Remove all clip planes".
- Session files keep every plane (format 1.2, `clips`); 1.0 and 1.1 files load their single plane as `clip:1`.
- The same from the CLI and MCP: `clip.set` with `plane` or `new`, `clip.clear` with `plane` or `all`, `clip.rotate` with `plane`, items `clip:1`, `clip:2` in `select.item`, `display.visibility`, `view.fit` and `inspect.item`.
- The clip plane's tilt arcs have their own colour, a little more violet than the slide arrow, in both themes; `settings.set_colors` sets either with `gizmo.dark` and `gizmo.light` (`slide`, `tilt`).
- While a tilt arc is hovered or dragged, its rotation plane shows as a faint disc through the plane's centre with ticks every 15 degrees (longer every 45), the tilt axis as a dashed line and, while dragging, a wedge of the angle turned so far.
- On macOS, tooltips, menus, the shortcut sheet (F1) and the palette name the keys Cmd and Option instead of Ctrl and Alt; `docs/keys.md` says so.
- Option + 1, 3 and 7 (and Cmd + Option for the opposite views) now work on Mac keyboard layouts where Option + digit types a character (Option + 7 is `|` on a German layout): shortcuts with Alt match the key's place on the keyboard too.
- The model tree's right-click "Colour…" opens a colour window (hue, saturation and brightness sliders, hex code, the patch palette, Reset colour, Done); the colour button inside the menu closed with the menu and seemed to do nothing.
- README: a copy-paste setup of the MCP server for Claude Code on macOS (`claude mcp add --scope user …` with the full path to `target/release/murmel`), and what to do when it fails.
- Regions of interest (`region.set`, `region.clear`, `region.list`, Alt + R): a box, a sphere, a slab, a cell zone, cell layers along a patch, or picked cells grown by rings of neighbours limit what the viewer shows and what slices, iso-surfaces and arrows compute. A cell belongs when its bounding box touches the shape; several regions on show their union.
- The model tree has a Regions group (eye, select, double-click to fit, Fit and Remove in the context menu) with buttons that add a box, a sphere, a slab or a region of the picked cells; the palette has a command for every shape. Properties show the shape, the cells it takes, the halo of neighbours that a mesh read in pieces will load, and the memory both need.
- Session files (format 1.7) keep the regions; a region whose zone, patch or cells the mesh lacks is skipped as `region:<n>`. New codes REGION001 (no such region), REGION002 (broken shape) and REGION003 (more than 16 regions).
- A box, sphere or slab region can cut flat along its border (`region.set` with `cut: flat`, the tree's context menu, Alt + Shift + R): its border cells are clipped to the shape and closed by their sections, as a flat clip plane cuts, and slices, iso-surfaces and arrows follow the cut. A box's and a slab's cut is exact, even when thinner than a cell; a sphere's follows it to within a cell, and a sphere with no mesh node inside shows whole cells instead. Where a whole-cell region and flat regions meet, each cell keeps one way of cutting and the surface stays closed.
- Session files (format 1.8) keep the cut.
- A wedge (VTK cell type 13) in VTK's node order, its base (0, 1, 2) pointing towards the top (3, 4, 5) as VTK 9, FoehnCraft and KoflCraft write it, opens with its faces outward from a `.vtu` file too; it opened inside out before. `.vtkhdf` files no longer turn linear cells by their volume, so a cell given inside out stays visible to the quality check.
- Probes show the value of the field colouring the mesh on their label in the view and on their row in the model tree (`|velocity| 12.3 m/s`, `velocity_x …`, `pressure … Pa`), following the field and component you switch to. Properties still lists every field, derived and formula fields included, and a probe on a wall also lists the boundary fields there (y+, wall shear). `probe.add`, `probe.at` and `measure.list` give it as `shown`.
- The Probe history window (the Probes group's "History…" button, or the palette): every probe's value of a field over all time steps of the open series, read in the background with progress and Cancel, derived and calculated fields made again for each step. The step shown is marked, hovering lists a step's values, and "Export CSV…" writes them. The tools `probe.history` and `probe.export` do the same from the command line and for agents (PROBE001 to PROBE003).
- A probe reads a cell field as the cell's value plus its least-squares gradient to the probe's point, as a finite-volume solver reconstructs it, and a node field through the cell's linear fit to its nodes. Both are exact for fields linear in space and keep a cell's peak, which the inverse-distance mean of node values flattened.

### Results: fields, slices, iso-surfaces, arrows, streamlines

- Opens FoehnCraft results (`fields.vtkhdf`, VTKHDF 2.x) with MurmelCraft's own HDF5 reader (superblocks 0 to 3, compact, contiguous and chunked datasets, deflate and shuffle, fractal heaps and B-trees), and the cell and point data of a `.vtu`. `mesh.open` lists the fields; `field.list` gives each field's components, unit, location and ranges, and the colour maps.
- Colour by field (`display.color_by` with `"by": "field"`, a field and for vectors a component or the magnitude). Three looks with `field.style`: smooth (the first look; node values by inverse distance to the cell centres, boundary faces with values of their own keep them), flat (one colour per cell, the solver's values) and banded (12 bands); contour lines on or off with their own count (12), anti-aliased and as wide on HiDPI screens and in scaled exports.
- Eight colour maps (viridis by default, cool-warm when the range is symmetric about zero, plasma, magma, inferno, turbo, rainbow, grey), each reversible; the range spans the whole field, or what is shown now, symmetric about zero, or fixed numbers. Each field keeps its own style; changes are undo steps.
- A colour bar at the right edge below the view cube with the field's name, unit, ticks, bands and line marks; clicking its title opens the field settings (field, component, look, bands, lines, map, range, reset). Exported pictures and movies draw the same bar.
- Properties show the field values of a cell or boundary face; probes show the values interpolated at their point. Session files (format 1.4) keep the field shown and every field's style.
- Slices of the results (`slice.set`, `slice.rotate`, `slice.clear`): up to 8 planes placed and moved like clip planes (the same slide arrow and tilt arcs in the viewport, Shift + X/Y/Z, a typed origin, normal or offset), each with an eye, a context menu and Properties in the model tree's "Slices and iso-surfaces". A slice can be a series of up to 64 parallel planes with a count and spacing; a count alone spreads them through the mesh. Slices use the mesh's field style and its colour bar (or the colour mode's cell colours without a field), and show a cell's own value inside it: they cut the same tetrahedra as iso-surfaces, so the core of a coarse mesh keeps its peak.
- Iso-surfaces (`iso.set`, `iso.clear`): up to 8, each where one field (any component) takes one value, typed, or placed by a click on the colour bar (a click with an iso-surface of that field selected moves it). Solid in a colour of their own (Colour… in the tree) or coloured by the field shown; marks on the colour bar show their values. Built by marching tetrahedra (Doi and Koide 1991) on the cells' tetrahedra, closed across cells.
- While slices or iso-surfaces show, the mesh draws as a faint outline of its feature edges (`display.mesh_look`, token `mesh.field.outline`); "Show the mesh faces" in the tree brings the faces back.
- Session files (format 1.5) keep slices, series, iso-surfaces, their colours and the mesh's look.
- Vector arrows of the results (`arrows.set`, `arrows.clear`, Alt + V): shaded 3D arrows (a shaft and a cone, eight sides, lit like the mesh) of a vector field, the field shown when it is a vector, else the result's first. They sit on every slice shown, on an even square grid anchored at the mesh's centre (density: arrows across the mesh's largest extent, 4 to 200, 25 by default), so they do not crowd in fine boundary layers; the vector there is the one the slice's tetrahedra interpolate. On request they also go on the mesh surface shown, one boundary face per cube of the same grid, outside the mesh (starting at an outlet, ending at an inlet).
- The length follows the magnitude: the field's largest is 0.9 of a grid spacing, times a length factor (above 0, up to 100). "Only the part in the slice" draws the vector's part in each slice's plane, its longest arrow then 0.9 of a spacing. At most 10 000 arrows are drawn; the result says when more were wanted.
- Arrows are solid in a colour of their own (token `mesh.field.arrow`, Colour… in the tree) or coloured by the field on the colour bar. The model tree has a row for them (eye, swatch, context menu) with their density and length boxes below it, and Properties rows; the palette has show or hide, surface, in-plane, more, fewer, longer, shorter, solid, field and remove.
- Session files (format 1.6) keep the arrows and their colour; arrows of a missing or scalar field are skipped as `field:<name>`. New codes ARROW001 (no vector field) and ARROW002 (density or scale out of range).
- Streamlines of the results (`stream.set`, `stream.clear`, Alt + L), stage 4 of field results: up to 8 sets traced through the cells by fourth-order Runge-Kutta from seeds on a line, in a ball, on a patch or on a plane. Seeds are placed by drag handles in the viewport (the line's ends and middle, the ball's centre and rim, the plane's centre) that snap to a node, an edge's middle or a face's centre (Alt moves them freely), or by typed points. Lines run downstream, upstream or both ways (the default) and stop where the flow leaves the mesh, slows below 0.1 % of its top speed or reaches a length limit. They draw as thin lines, or as shaded tubes with a radius setting and arrowheads on request, in a solid colour or coloured by any field with their own colour bar beside the mesh's legend, in the window and in exported pictures. The model tree has a row per set with its seeds, length and radius, and the Properties panel says how many seeds lay inside the mesh and how each line ended. Session files keep the sets (format 1.9).
- Vector arrows are denser and smoother: 40 across each slice's own extent by default (was 25 across the whole mesh, so a cross-section of a long duct got only a few), with twelve sides instead of eight. The density counts across each slice, so every slice gets a full grid of arrows whatever its size.
- Arrows have a floor on their length by default: as long as the magnitude, but at least a quarter of the longest, so the direction still shows where the flow is slow.
- Arrows are coloured by their own magnitude by default, with a colour bar of their own beside the mesh's legend (in the window and in exported pictures), in their field's style; its title opens their colouring, colour map and range. Solid and "by the field shown" remain.
- `arrows.set` takes `length`: `magnitude` (strictly as long as the magnitude), `floor` (the default) or `equal` (all the same length, the magnitude in the colour only), and `color` `magnitude`; the result reports each slice's spacing and the colour range. The arrows' context menu in the model tree has Length and Colour submenus, the palette has a command for each, and the Properties panel shows the length mode. Session files keep the length (format 1.10); files before 1.10 open with the new defaults.
- Field colours default to Rainbow Desaturated (ParaView's eight control points), the colour map now shared by MurmelCraft, FoehnCraft and KoflCraft; Inferno is second in the list and cool-to-warm stays for ranges symmetric about zero. Fields saved with the automatic map open in the new default; a map chosen by name stays as it was.
- Iso-surfaces are smooth. Values between cells are now rebuilt from each cell's least-squares gradient (fitted to the cells that share a node with it), instead of the inverse-distance mean of the cell values, which waved even on a straight-line field. A cell predicts the value at a node or a face centre from its own value and gradient, the predictions are averaged, and the result stays within the values of the cells around it, so no new extremes appear. A field linear in space is now exact: its iso-surface is a flat plane (it had 2.3 % too much area). On uneven cells the shading normals of a round iso-surface are about 9 times closer to the true ones (1.0° instead of 9.2° on average).
- Iso-surface normals follow the field's slope, interpolated from the cell gradients, so the shading is smooth across the facets.
- Smooth colouring, slices, streamlines and probes use the same rebuilt values, so every view still shows the same field. Pictures of results change slightly: banded colours of a linear field are now straight.
- An iso-surface could have holes where a face centre crossed the value but the cell's own corners did not. That cell is no longer passed over.
- An iso-surface could cap a tube at a large inlet or outlet face, because that face's centre took the mean of its rim. A boundary face's centre now follows its cell.
- Derived fields: vorticity, Q criterion, λ₂, total pressure and pressure coefficient, computed from the result's velocity and pressure (`field.derive`; `field.catalogue` lists what can be derived). The velocity gradient of each cell comes from the same least-squares fit as smooth colouring, so a velocity linear in space gives its exact vorticity, Q and λ₂ in every cell. The names and units are FoehnCraft's, and a field the file already has is not computed again. A derived field joins the file's fields for colouring, slices, iso-surfaces, arrows, streamlines and probes; session files (format 1.11) keep how each was made and compute it again on loading.
- The Derived fields window (palette "Derived fields…", the colour-by menu and the field legend's menu): the quantities by group with formula, unit and inputs, the ones whose inputs are missing greyed, and Compute (which also shows the field), Show and Remove.
- Reference values of the pressure quantities (density, pressure, velocity) come from FoehnCraft's `case.resolved.toml` beside the result, else your defaults, else air at sea level, 0 and the largest speed; each says where it came from, and a right-click keeps one as your default (`settings.set_default` kind `reference`).
- A kinematic pressure (m²/s²) takes no density, and a density field of the file wins over the reference density.
- The window draws Greek letters and sub- and superscripts (a subset of Inter of their own); a test checks that every string of the window has its glyphs.
- Ranges centred on zero (the automatic map with a symmetric range) now take Rainbow Uniform, Colin Ware's map from ParaView, the same as FoehnCraft's signed-field plots: blue through green to a bright yellow at zero, then orange, red and magenta. Cool to warm stays in the list.
- The field calculator (`field.calculate`): a field of your own formula of the result's fields, for colouring, slices, iso-surfaces, arrows, streamlines and probes like the file's. Formulas take numbers and `pi`, the coordinates `x`, `y`, `z` and `pos`, field names bare or quoted (`"p rgh"`), `+ - * / ^`, the comparisons `< <= > >=`, components (`U.x`), vectors (`[a, b, c]`) and the functions sqrt, abs, exp, log, log10, the trigonometric and hyperbolic ones, atan2, sign, floor, ceil, min, max, if, mag, dot, cross, grad, curl and div. The derivatives are the least-squares gradients of the derived quantities, exact for linear fields. A result is a scalar or a vector and sits wherever every field it reads is (cells, nodes, boundary faces). Refusals say the column (FIELD011 to FIELD015).
- Your formulas (`settings.keep_formula`, `settings.forget_formula`): kept in settings.toml and offered for every result; `field.calculate` computes one by its name alone.
- The Derived fields window has two tabs, Quantities and Your formulas; the second has the formula editor with the result's fields and the functions to click into it, why a formula was refused, and your formulas and calculated fields with Compute, Show, Remove, Edit, Keep and Forget. The palette's "Field calculator…" opens it there.
- A field made from another (a formula of a derived field, a formula of a formula) is computed again when that one changes, and cannot be removed before it; a formula that would read itself is refused. Session files (format 1.14) keep the formulas beside the derived recipes, in order.
- The Derived fields window computes FoehnCraft's turbomachinery, turbulence and wall quantities itself, with FoehnCraft's names and units: enstrophy and strain rate; relative and frame velocity, the radial, circumferential, axial and meridional velocity, the relative circumferential velocity, the flow angles α and β, the relative total pressure and rothalpy; ε, ω, the turbulence time and length scales, the viscosity ratio ν_t/ν and the six Boussinesq Reynolds stresses; and the exact wall distance.
- Turning zones for the turbomachinery quantities come from FoehnCraft's `case.resolved.toml` beside the result (`[[mrf]]`, `[[sliding]]`, `[output] frame`), else you type them in the window: cell zone, axis point, axis and rpm, several zones, and the frame every cell is seen in. A right-click keeps the speed or the axis as your default (`[defaults.rotation]`).
- The wall distance measures to the case's walls (`type = "wall"`), else to the patches whose role is wall; the window lets you tick others.
- The kinematic viscosity ν and C_μ come from the case too (`[fluid]`, `[turbulence.constants]`), else your default, else air at sea level and 0.09.
- The window shows one group at a time (Vortices, Pressure, Turbomachinery, Turbulence, Wall), the values its quantities take above the rows.
- `field.derive` takes `k`, `dissipation` (ω or ε), `nu_t`, `rotation` and `walls`; `field.catalogue` lists the rotation, the walls, the mesh's zones and patches. New refusals FIELD016 (turning zones) and FIELD017 (walls). Session files are format 1.17.
- Colour range "All time steps" (`field.style` range `steps`): while a time series is open, the mesh's colours span the smallest to the largest value of the field over every step, so a colour means the same value at each step of a playing series and in a movie of it. It is the default while a series is open (fields coloured over their whole range at one step take it; shown, symmetric and fixed ranges stay); "Whole field" gives each step's own range back. The window reads the other steps in the background and the player bar shows how far it is; the legend widens as it reads. `series.range` reads it from the command line and for agents, and pictures and movies read it before their first frame (SERIES004). Session files keep it (format 1.17).
- The Derived fields window lists FoehnCraft's own fields under Wall and Turbulence: y⁺, wall shear stress, the wall treatment's u_τ, y* and blending, the turbulence features, the wall normal and the curvature correction. One the result lacks is computed by the `foehn` installed on your machine (`foehn post derive`, in the background, with Cancel and Esc) and read in as the next step of the same result, so the view and your derived fields stay. The row is greyed with the reason when foehn, the restart file or the case is not found.
- `field.foehn_derive` does the same for scripts and agents (over MCP it needs `output` or `file`). New codes FIELD018 to FIELD020.

### Sprays, time series and charts

- Spray parcels (`parcels.open`, `parcels.set`, `parcels.clear`; D268, first part): the droplets of a FoehnCraft spray (`spray/parcels_<step>.vtkhdf`, or any VTKHDF PolyData of points) show with the mesh and its fields as small shaded spheres, all the same size or sized by droplet diameter, coloured by any parcel field (diameter, a velocity component, age, …) with a colour bar, colour map and range of their own, or in one solid colour. Filters keep a diameter range or one injector, and clip planes cut the parcels away with the cells. The model tree has a row with eye, settings and size factor, the Properties panel shows the file's time, step and counts, and session files keep the parcels (format 1.12).
- Time series (`series.open`, `series.go`, `series.play`; D268, second part): the steps of one run from FoehnCraft's `series.json` or an output folder of `fields_<step>.vtkhdf` and `spray/parcels_<step>.vtkhdf`. Each step shows its fields and parcels with the slices, colouring, parcel settings, derived fields, camera and undo steps kept. A player bar below the view (first, back, play or pause, on, last, a step slider, speed and loop; Space, comma and full stop) and the time in the status bar; `export.movie` with motion `series` makes a movie of the steps; session files keep the series and the step (format 1.13).
- The time series player's speed and loop can be your defaults: right-click the speed or the Loop box in the player bar and save it; series opened later start with it (paused). Settings lists them under "Time series player" and forgets each; `settings.set_default` with kind `player` (keys `rate`, `loop`) does the same from the command line.
- A playing time series reads its next step ahead on a thread of its own, with its derived fields and, while the view is unchanged, its surface, so the window stays free and steps follow each other at the speed set. A step that takes longer to read than the speed allows keeps the step shown until it is ready, instead of holding the window. A step is read ahead only when two steps fit the memory budget.
- Exported pictures and movies can show the time and step of the open time series in a panel at the bottom right (`t = 0.25 s · step 50`, or `Step 50` without a time): `time` in `export.image` and `export.movie`, on by default for series movies, and a "Time and step" box in both export windows while a series is open.
- Charts and statistics of a FoehnCraft run (`charts.open`, `charts.show`, `charts.data`, `charts.export`, `stats.at`; D268, third part): `datums.csv`, `spray.csv` and the size histograms `spray_hist_<datum>.csv` read without the solver. Charts of the liquid mass flow through the datums, the liquid through them so far, the penetration per injector, D32 through each datum and in the flow, any columns of one unit, and the droplet size histogram of a datum (by mass or number, cumulative or per interval). A panel below the view draws the chart with a cursor at the player's step and the statistics at that step beside it; `series.open` opens the charts of its folder; charts export as PNG or SVG; six chart hues of our own in the tokens.

### Large meshes: memory budget, overviews and blocked files

- A memory budget: a mesh opens whole up to half the computer's memory, or the budget set in Settings → Memory (`settings.set_memory`, `[memory] budget_gib` in settings.toml). A blocked VTKHDF file whose whole read needs more opens as an overview: its boundary, with each cell field shown as the value of the cell behind each face, and its block index, without reading the volume cells. A 4 M cell blocked file reads as an overview in 0.25 s within 134 MB instead of 4.2 s and 3.9 GB. The model tree says when a mesh is an overview and offers "Read the cells anyway"; `mesh.open` takes `mode` (`fit`, `whole` or `overview`).
- A mesh opened as an overview colours its boundary on the scale of the whole volume: each field's colour range and legend come from the file's block ranges, so they match what reading the whole mesh would show (range mode "shown" still takes the values drawn). Properties count the file's cells and its blocks.
- The VTKHDF reader can read the cells of some blocks of a blocked file (`vtkhdf::read_blocks`): each block's own rows, with the halo of ghost cells around it when asked, the fields at those cells and the boundary faces they own. One block of 65,536 cells of a 4 M cell file reads in about 0.2 s. This is the base for regions of a mesh beyond the memory budget. A block the file does not have is BLOCK005.
- A box, sphere or slab region drawn on a mesh open as an overview (beyond the memory budget) now reads the cells of the blocks it touches, with a hidden halo for the values at its border, and shows what it shows on the mesh read whole. Turn the regions off and the overview shows again. The window reads the blocks in the background: the status bar says how many blocks and cells, and Cancel or Esc stops the read. The model tree's note and Properties tell the blocks shown, and Properties gives a picked cell's number in the file. A region whose blocks need more memory than the budget is refused (BLOCK006). Zone, patch and picked-cell regions need the cells read whole, so an overview refuses them (REGION004).
- The blocks a region reads on an overview are read on all threads: the points and ids of each block, the join of the points the blocks share (in shards) and the cells. On the 4 M blocked file 14 blocks read in 1.5 s rather than 2.6 s and 33 blocks in 3.2 s rather than 6.1 s, on 4 threads. The status bar says "reading the cells of the blocks" through every step of the read.
- The VTKHDF reader can read the preview grid of a blocked file (`vtkhdf::read_preview`): each cell field of the volume on a regular grid of about 128 cells along the longest side, each grid cell showing the cell its centre lies in, read from the cells' points alone and the fields at those cells. A 4 M cell file gives its 128³ grid in about 1.2 s. This is the base for slices and iso-surfaces across the whole volume of a mesh opened as an overview.
- Slices, iso-surfaces and arrows on a mesh opened as an overview (beyond the memory budget) now draw across the whole volume on its preview grid: the cell fields on a grid of about 128 cells along the longest side, each grid cell with the values of the cell at its centre. The grid is read the first time one of them shows (the window reads it in the background, the status bar says so, Cancel or Esc stops it); a region still reads the exact cells of its blocks. The model tree's note and `mesh.info` tell the grid.
- On a mesh opened as an overview, the model tree has a "Block boxes" row: its eye draws the box of each block of the file, and the blocks your regions read take a colour of their own, so you see what a region will read before it reads it. The mesh draws as its faint outline while they show (tick the mesh's faces to keep them). Right-click for their colour, or to keep them on as your default; session files keep the setting (format 1.16), and `mesh.info` tells the boxes.
- The preview grid of a mesh opened as an overview is kept in the cache folder (`MURMEL_CACHE_DIR`, else `~/Library/Caches/MurmelCraft` on macOS, `%LOCALAPPDATA%\MurmelCraft\cache` on Windows, `~/.cache/murmelcraft` elsewhere), so the file opened again unchanged shows its slices, iso-surfaces and arrows without sampling its volume again: about half a second whatever the size of the file. The 16 grids used last stay, within 2 GiB.
- On a mesh opened as an overview, a probe reads the block of the file that holds its point and shows the cell, its quality and the field values the whole mesh shows there (Properties give the block and the cell's number in the file). The last four blocks read stay, so the next probes nearby read at once; in the window the block is read in the background and the probe's label says "reading its block" meanwhile. `probe.add` and `probe.at` do the same.
- A probe in a halo cell of the blocks a region read on an overview took that cell, whose neighbours are partly missing, so its gradient and values were off; it now reads the block that owns the cell.
- On a mesh opened as an overview, streamlines are traced through the blocks of the file: each block is read with its halo the first time a line reaches it, and the lines are the ones the whole mesh gives. The blocks gone through stay, so moving or changing a set reads only new blocks; a trace reads at most as many blocks as a quarter of the memory budget holds, and a line that needs another ends there (Properties: "Reached a block not read", BLOCK006). In the window the lines are traced in the background; the status bar counts the blocks read, with Cancel or Esc. `stream.set` traces before it answers and gives `traced.blocks`.
- The streamlines of an overview draw beside the slices on its preview grid (they did not draw on an overview before).
- Opening a `.vtu`, an OpenFOAM mesh or an unblocked VTKHDF file that needs more than the memory budget now asks first: convert it once to a blocked copy beside it (`NAME.blocked.vtkhdf`), which opens as an overview, or open it whole. The conversion runs in the background with Cancel or Esc, and next time the up-to-date copy opens at once. "Convert large meshes without asking" (in the question, in Settings, or `settings.set_memory` with `convert_large`) skips the question.
- `mesh.open` warns (BLOCK007) when it opens such a mesh whole, and names its blocked copy.
- A new iso value (clicked on the colour bar or typed), or a new iso-surface, shows at once in the window: the GPU casts rays through a grid of the field, and the exact iso-surface takes its place a moment after the value rests (`murmel bench run` times it as `iso.preview`).
- The preview grids of slices and iso-surfaces are built on threads of their own as soon as a slice or iso-surface exists, so the first drag on a large mesh no longer stalls the window; the smooth values of a dragged slice now match the exact slice (linear on each cell's tetrahedra).
- On a mesh opened as an overview, slices and iso-surfaces show on the preview grid at once, and their exact faces are then cut through the blocks of the file and take its place. A slice reads only the blocks its planes cross; an iso-surface reads only the blocks whose ranges, with those of the blocks beside them, hold its value. Selecting one reads nothing again, and a slice moved a little cuts the blocks kept from the last cut again without reading them. In the window the cut starts once the items have rested and runs in the background; the status bar counts the blocks, with Cancel or Esc. Properties tells how many blocks a cut went through ("Blocks cut through"), or that it is being cut. `slice.set` and `iso.set` cut before they answer. Faces that would take more than a quarter of the memory budget stop with BLOCK006, and the item stays on the grid.
- The window no longer ends when it opens a very large mesh: the boundary of a
  100 M cell overview (1.6 GB of face vertices, 349 MB at 10 M) is drawn in
  pieces of at most 256 MiB, the most one GPU buffer may hold. Any other error
  the GPU reports while drawing now shows in the status bar (RENDER006) instead
  of ending the window.
- Slices, iso-surfaces, probes, streamlines and regions on an overview read
  far less of the file. Each block used to read every boundary patch whole
  (about 550 MB for the 100 M cell test file); it now reads only the rows of
  its own boundary faces, and the file is opened once. Exact slices also read
  the next blocks while one is being cut. A slice through a 10 M cell file
  reads 3.7 times fewer bytes and takes 13 s instead of 23 s; over a network
  share, where every byte counts, the gain is larger.
- Dragging a clip plane on a large mesh no longer waits for the cut to be built: the GPU cuts the surface at once (the cut faces look hollow for a moment) and the exact cut follows about a second after the plane stops (it took 1.1 s per step at 4 M cells). Screenshots, exports, picks and agents always see the exact cut. `murmel bench` reports `clip.step` and `clip.exact`.
- Moving the view on a large mesh hides the edges while it moves, only when a frame with edges takes longer than 33 ms; they come back as soon as the view rests, and nothing else is simplified. Meshes that draw fast enough look as before, and pictures and movies always draw the edges. `murmel bench` reports `render.next.faces`, a frame without the edges.
- Dragging a slice in the window draws it at once from a grid of the field on the GPU, whatever the mesh's size; the exact slice takes its place a moment after the slice stops (`murmel bench run` times it as `slice.preview`).

### Files and formats

- OpenFOAM meshes open like a `.vtu` (UI spec V1.0): `mesh.open` (version 2) takes a case folder, its `constant` folder, the `polyMesh` folder, a `.foam` file in the case or one of the polyMesh files. The reader (`murmel-mesh` `polymesh`, written from the format documentation) reads `points`, `faces` (`faceList` or `faceCompactList`), `owner`, `neighbour`, `boundary` and `cellZones`, in ascii or binary (32 or 64 bit labels and scalars, either byte order) and gzip-compressed (`faces.gz`). Cell and face ids are OpenFOAM's own labels.
- Cells, cell types, patches, zones, the clip plane, picking and quality colouring work on OpenFOAM meshes unchanged. Patch roles come from the OpenFOAM type (`wall`, `symmetry`, `symmetryPlane`), else from the name as KoflCraft names patches; the roles in `mesh-evidence.json` win. Cells in no cell zone join a zone `cells`.
- The validity badge checks every file KoflCraft lists in the evidence of its OpenFOAM export (`kofl export … openfoam`); `mesh.open` and `mesh.info` return the files read with their SHA-256. The mesh properties show the format, the patch properties the OpenFOAM patch type.
- The window opens a case folder with "Open OpenFOAM case folder…" (Ctrl+Shift+O, in the palette) or by dropping it; the Open dialog also offers `.foam` files.
- New diagnostic codes MESH013 (no polyMesh in the folder), MESH014 (malformed OpenFOAM file) and MESH015 (patches do not cover the boundary faces).
- `murmel blocks make MESH --out FILE` (the tool `blocks.make`) writes a mesh and its fields as a blocked VTKHDF file (`docs/blocked-vtkhdf.md`): the cells sorted along a 3-D Hilbert curve and cut into blocks of 65,536 cells, each with its own points and one layer of ghost cells, global ids, the bounds and field ranges of every block, and the patches with their owner cells. It is the layout from which MurmelCraft will read a region of a mesh too large for memory; ParaView 5.13 and later open it as it is. A `.vtu`, an OpenFOAM `polyMesh` or a VTKHDF file converts; 1 million polyhedra take about 11 seconds.
- Blocked VTKHDF files open whole, from MurmelCraft, KoflCraft or FoehnCraft: ghost cells are left out, the blocks join at their shared points, and zone names, patch roles and the file's digests are kept.
- `murmel bench make --formats blocked` writes the synthetic honeycomb of polyhedra straight into a blocked VTKHDF file, block by block, without the mesh in memory, with two cell fields (`velocity`, a vortex array in a uniform stream, and its `pressure`): a test file of up to 684 M cells. 100 M cells take 94 GB on disk and about 3 to 4 GB of memory while written (docs/performance.md, "A 100 M cell test file"). The command line shows each step as it goes.
- The blocked layout (docs/blocked-vtkhdf.md § 5) allows 32-bit ids within a block (`Offsets`, `Connectivity` and the four polyhedron arrays, all six alike); the ids of the whole mesh stay 64-bit. A file of polyhedra then takes about 940 bytes a cell instead of 1,500. MurmelCraft reads either; the synthetic file uses them.
- Blocked VTKHDF files mark their text ASCII when it is (other text stays UTF-8): VTK 9.4's reader refused every blocked file MurmelCraft wrote, because their `Type` attribute was marked UTF-8. The layout doc, `blocks.make` and the blocks module said ParaView 5.13 opens these files; they now say VTK 9.6 and later, as checked with VTK 9.4 to 9.7: 9.4 and 9.5 (ParaView 6.0 and earlier) crash on their polyhedra and open only a file of linear cells.
- `mesh.open` (version 6) says where a file comes from: `origin` names the tool
  that wrote it (koflcraft, foehncraft, murmelcraft, openfoam, vtk), read from
  the marks in the file, and `imported` is true for an OpenFOAM polyMesh and for
  a VTK file none of our tools wrote. Properties names the file the same way
  ("KoflCraft mesh (.vtu)", "OpenFOAM polyMesh (binary), imported"), and
  `recent.list` (version 2) keeps the origin of each mesh for the tags in
  File ▸ Open recent. The MCP server introduces MurmelCraft as the viewer for
  KoflCraft meshes and FoehnCraft results that also imports OpenFOAM and VTK.
- File ▸ Open is for the files of our three tools: MurmelCraft sessions, KoflCraft
  meshes and FoehnCraft results. Its dialog lists only `.murmel`, `.vtu` and
  `.vtkhdf`, with no "All files", so other files are greyed out. Files of other
  tools come in through the new File ▸ Import: OpenFOAM polyMesh… (a case,
  `constant` or `polyMesh` folder) and VTK file…; neither has a default key, so
  Ctrl+Shift+O is free (a key you gave the old "Open OpenFOAM case folder" stays
  on the import). The title bar names the tool that wrote the open file, a drop
  of another tool's file opens at once with "Imported …" in the status bar, and
  each entry of Open recent carries a tag (KoflCraft, FoehnCraft, Session,
  OpenFOAM, imported). Load session… leaves the File menu, since Open… loads
  sessions; it stays in the palette. New help page: Import from other tools.
- The window shows a mesh as soon as it is read and checks the SHA-256 of its files afterwards; the validity badge says Checking until the digest is in place. Without SHA instructions (as on a 2019 Core i9) a 4 M cell ascii `.vtu` shows after 9.4 s instead of 16.6 s. `murmel bench` times the wait for the digest (`open.digest`).
- The window and the command line remember the SHA-256 of each mesh file they check (`checked-files.json` in the settings folder). A file opened again unchanged (same size, modification time and, on Unix, change time and file id) is not hashed again: on the iMac a 4 M cell `.vtu` shows its badge at once instead of 17 s later, and `murmel call mesh.open` takes about 5 s instead of 20 s.
- Mesh files are checked with BLAKE3 instead of SHA-256 (the user's choice, 2026-11-24, "BLAKE3 only, still check old"): many times faster on the iMac, which has no SHA instructions. KoflCraft's evidence 1.1 names its hash (`file_hash`); evidence 1.0, older session files and older `checked-files.json` lists are still checked with SHA-256. Session files are format 1.15 (`mesh.blake3`), and `mesh.open` returns `hash` with each file's digest under it.
- A VTKHDF file of boundary patches only (no volume cells) lost its first face, and the points two patches share appeared twice; every face is now read and shared points are one point.

### Speed

- `murmel bench make --cells N --out DIR` writes a synthetic polyhedral mesh of any size (14-face cells like a Voronoi core) as ascii or binary `.vtu` and OpenFOAM `polyMesh`; `murmel bench run MESH` times reading, opening, the surface of several displays, quality, picking and drawing, with the peak memory. The baseline at 1 M cells is in `docs/performance.md`.
- Opening a mesh is 2 to 4 times faster and needs less memory: a 1 M cell ascii `.vtu` opens in 3.3 s instead of 12.8 s, a binary polyMesh in 1.5 s instead of 2.2 s. The file is read once, numbers are parsed on all cores, and faces are matched without hashing.
- A mesh of 32 MiB or more opens in the background: the window keeps working, the status bar shows what is being read and for how long, and Cancel (or Esc) stops it.
- The mesh model stores each face once instead of once per cell: a mesh takes about 27% less memory (476 MiB per million polyhedral cells instead of 655), and an OpenFOAM polyMesh opens faster with a lower peak.
- The `.vtu` reader streams the file instead of holding its text: reading a 4 M cell ascii `.vtu` peaks at 4.0 GB instead of 9.2 GB and is a little faster. `murmel bench` reports the peak memory of reading.
- Ascii `.vtu` files open faster: the numbers are parsed into memory the parsing threads fill, sized from the counts the file declares, and counted with vector compares. At 4 M cells opening takes 7.5 s instead of 9.3 s on 4 threads, with the same peak memory. Points are read straight into points (`.vtu` and ascii polyMesh), without a copy.
- Opening a mesh links the faces of its cells on all threads (the sort of the faces, their numbering and the matching): at 4 M cells that step takes 1.4 s instead of 2.2 s on 4 threads, and divides further on more. The faces, owners and neighbours found are the same.
- A `.vtu` file's cells are built on all threads, and a file of polyhedra and polygons (as KoflCraft writes) leaves their faces where the file put them instead of moving them. At 4 M polyhedra on 4 threads building the cells takes 306 ms instead of 674 ms; the mesh is the same bit for bit.
- Linking the cell faces of a mesh (matching each face with its twin) sorts and copies the faces faster: the `.vtu` reader notes each face's smallest point as it reads it, and the distinct faces are sized on all threads. At 4 M polyhedra on 4 threads, linking takes 1.2 s instead of 1.65 s and the open about 0.35 s less; the mesh is the same bit for bit.
- Blocked VTKHDF files open several times faster and with less memory: the reader reads the datasets where they lie in the file instead of copying them, joins the points of the blocks on all threads and builds the cells on all threads. At 1 M cells on 4 threads a blocked file opens in 1.6 to 1.9 s instead of 6.2 s, with a peak of 2.0 GB instead of 3.4 GB; the 4 M cell file, which ran out of memory in 15 GB before, opens in 7 s. The mesh and fields are the same bit for bit.
- A blocked VTKHDF file with ghost cells and a field the reader does not convert (half floats) no longer crashes the reader; the field has no values, as in a file without ghosts.
- VTKHDF and parcel files are no longer read into memory whole: the reader reads the file's structure when it opens, and each dataset from the file when it needs it, on all threads; datasets it does not need are never read. A 4 M cell blocked file opens in 5.1 s instead of 7.9 s, with a peak of 7.2 GB instead of 8.3 GB while reading; the mesh and fields are the same bit for bit.
- VTKHDF cells are built from their datasets a few thousand cells at a time, each reading only the rows it needs, instead of from whole datasets: a 4 M cell blocked file peaks at 4.0 GB while reading instead of 7.2 GB (5.9 GB instead of 9.1 GB for the whole open), and opens in 4.2 s instead of 4.8 s; the mesh and fields are the same bit for bit.
- Colour by quality computes the cell quality on all threads: 4 M cells take 2.1 s instead of 8.0 s on 4 threads. Surface builds (every hide, colour or clip change) are 30 to 55 % faster, and `murmel bench` reports the surface's phases.
- The model tree and the legend no longer count the cells on every frame (49 ms a frame at 4 M cells), and picking tests boxes around runs of triangles before the triangles: a pick at 4 M cells takes 0.3 ms instead of 37 ms, so hover and measure snapping keep up on large meshes. `murmel bench` reports `tree.frame` and `surface.boxes`.
- Building the drawn surface fills its buffers and finds its edges on all threads: at 4 M cells the default surface takes 0.3 s instead of 0.6 s, a colour change by cell type 0.34 s instead of 0.53 s, and the exact cut after a clip drag 0.93 s instead of 1.28 s. The picture is the same, bit for bit. `murmel bench` reports the phases of every surface stage.
- Clipped surfaces are built on all threads: at 4 M cells a crinkle cut takes 0.42 s instead of 0.73 s and a flat cut 0.66 s instead of 1.29 s, so the exact cut after a clip drag follows sooner. The picture is the same, bit for bit.
- Feature edges (render mode 4, and the outline of the mesh while slices or iso-surfaces show) are found on all threads: at 4 M cells that surface takes 0.44 s instead of 0.65 s. The edges found are the same.
- A flat clip cut is built on all threads, except the numbering of its cut points: at 4 M cells the cut surface takes 0.51 s instead of 0.61 s on 4 threads, and divides further on more. The cut looks exactly as before.
- `murmel bench run` times the field stages too: colouring by a field the first time and again, a change of the colour range, a slice and a step of its drag, an iso-surface and a step of its value, and arrows on the slice, with the phases inside them (the field's topology, gradients and node values; the section, the outlines, the iso-surface, the vector). Every mesh gets two cell fields for it, a scalar and a swirling vector. `--no-fields` leaves them out.
- Slices and iso-surfaces are cut on all threads and test only the cells near them: the cells sit in bins of a regular grid (made once per mesh), a slice tests the cells of the bins it passes and an iso value those of the bins whose values can reach it. At 1 M cells a step of a slice drag takes 0.42 s instead of 1.06 s, and a step of an iso value 0.97 s instead of 2.9 s. What is drawn is the same, bit for bit.
- Colouring by a field the first time, and arrows the first time, take about half and a third of the time: the field's topology is made on all threads, a vector's component is made alone (not with the other two), its magnitude in one pass with them, and the arrows read their largest magnitude from the cells.
- Moving a slice, changing an iso value or the arrows now builds only those again, not the mesh's own faces, and the renderer uploads only them (a slice step on 1 M cells: 421 to 230 ms).
- `murmel bench run` lists the steps of linking the faces (`topology.buckets`, `topology.twins`, `topology.number`, `topology.flip`, `topology.distinct`, `topology.boundary`, `topology.rest`) and reports the peak memory on macOS as well as Linux.

### Agents, sessions and export

- An MCP server for agents (UI spec V1.0): `murmel mcp --read-root DIR --write-root DIR` (or `--root DIR`, or the program `murmel-mcp`) offers every tool of the registry by its dotted id over stdio (MCP 2025-11-25, also 2025-06-18), the same tools the window and `murmel call` use. One headless session lasts the whole connection, so the mesh, camera, display, clip plane, selection and undo journal carry from call to call. See docs/mcp.md for the setup in Claude Code and Claude Desktop.
- `view.screenshot` over MCP returns the PNG as an image the agent sees; `path` may be left out, and the server writes `screenshots/shot-NNNN.png` below the first write root.
- Paths in tool inputs are confined: reads below a read or write root, writes below a write root, links followed, relative paths from the first write root. New diagnostic codes MCP001 (outside the roots) and MCP002 (path does not resolve). The settings tools change the MCP session only and never write the person's config files.
- The tool catalogue (`system.commands`) lists the input fields each tool reads as paths (`reads`), beside the files it writes (`outputs`).
- Agents can drive the open window: `murmel mcp --live --root DIR` runs the same MCP server with the same roots check, but each call runs in your open MurmelCraft window. You watch the agent work, its steps are undo steps (Cmd + Z or Ctrl + Z), and screenshots come back at the viewport's size. See docs/mcp.md, "Drive the open window".
- While an agent is connected, the status bar says "Agent connected" with a Stop button. Settings > Agents has the switch "Let agents drive this window" (on by default, saved in settings.toml) and shows the socket the window listens on. The window listens on a socket only your user account can open (macOS and Linux).
- Session files: Cmd + S or Ctrl + S saves what you see as a `.murmel` file. That covers the mesh, camera, colouring, quality threshold, hidden and isolated items, selection, item colours, clip plane, mesh colours and view cube placement. Cmd + Shift + S or Ctrl + Shift + S saves it under a new name.
- Opening a `.murmel` file (Cmd + O or Ctrl + O, the palette's "Load session…", drag and drop, or `murmelcraft view.murmel`) brings the view back.
- The mesh is found relative to the session file first, so a case folder can move.
- A mesh written again since the save, or items it no longer has, give an amber warning in the status bar, and the rest still loads.
- The saved palette and view cube apply to that session only; your settings are not changed.
- The same works from the CLI and MCP (`session.save`, `session.load`). Over MCP, the mesh a session file names must lie within the server's roots. See docs/sessions.md.
- Saved views: Cmd + B or Ctrl + B saves the current camera as a named view ("View 1", "View 2" …) under "Saved views" in the model tree. Click a view to fly there, double-click to rename it; its context menu also updates it to the current camera or deletes it. V and Shift + V step through the saved views.
- Saved views are undo steps and go into session files (`.murmel` format 1.1; 1.0 files still load). Opening another mesh starts with none.
- Colour by cell zone: the toolbar's "Colour by" and the palette have "Cell zone", which gives each OpenFOAM `cellZone` (or VTU region) its own colour; a zone's context menu in the tree has "Colour by cell zone", and a colour set on a zone wins.
- The tree's "Zones" group is now called "Cell zones".
- The same works from the CLI and MCP: `views.save`, `views.go`, `views.rename`, `views.delete`, `views.list`, and `display.color_by` with `by: zone`.
- Your defaults: right-click a value in the Properties panel (such as Arrows across), an item in the model tree (arrows, slices, iso-surfaces, streamlines, clip planes, regions), the colour bar's settings or the toolbar's render mode and projection, and choose "Save … as your default". New items of that kind start from it, a field shown for the first time takes your field colouring, and the window starts in your view; items already open keep their values. Settings lists every default with a Forget button, and "Forget them all" goes back to the built-in values. The defaults are kept in `settings.toml` (`[defaults]`).
- `settings.set_default` keeps values as defaults: given directly (`kind` with `values`), taken from an item (`item` with `keys`, or all its values), or from the view or a field's style. `settings.reset_default` forgets one value, a kind or all. `settings.get` lists the defaults and which values each kind can keep. The palette has four Defaults commands: save the selected item's settings, start with this view, colour new fields like the field shown, and forget them all.
- Only an item's look, density, count and modes can be defaults, never where it is or which field it shows; a value saved at its built-in value forgets the default.
- The translations (the Fluent bundle of `assets/i18n/en/murmel.ftl`) now live
  in the engine crate, so tools can translate the legends of exported pictures.
  Nothing changes for users.
- The colour legend now comes from one model in the engine (`murmel_engine::legend`),
  which exported pictures will draw too. Ramp legends get nice tick values
  (1, 2 or 5 times a power of ten). The window's legend looks the same as before.
- Export pictures for reports and posters: File, "Export image…" (Ctrl + Shift + E)
  and the tool `export.image` write a PNG of the view at any size up to 8192 pixels
  or a scale of the viewport, with lines, legend and text scaled to match, the
  colour legend and the axis triad each optional, a transparent background and
  the dpi for print. See docs/export.md.
- Movies: `export.movie` and the "Export movie…" window (Ctrl + Shift + M)
  make the camera turn about its target, sweep a clip plane through the
  mesh, or fly through saved views (easing into each, with an optional
  hold), with the look of exported pictures (size, scale, legend, axes,
  lines). Written as an MP4 (H.264, by the ffmpeg installed on the
  machine), a looping GIF or PNG frames. The window makes the movie in the
  background with a progress bar and Cancel; an agent's movie over the live
  bridge runs the same way. New codes EXPORT002 to EXPORT007.
- File ▸ Print… (Ctrl + P) puts the view on a page and opens the system's print dialog (Preview's
  on macOS); File ▸ Export ▸ Export PDF page… writes the same page as a PDF, and File ▸ Page setup…
  chooses paper (A4, A3, Letter), orientation, margins, resolution, colours (light by default) and
  the header with the title, the date and the file. The page is kept in `[page]` of settings.toml;
  agents use export.pdf and settings.set_page. The PDF writer is MurmelCraft's own.

### README, docs, tests and CI

- The README opens with a slideshow of the window: eight screenshots of a FoehnCraft result on KoflCraft's Manifold mesh that change every four seconds. The same pictures, with captions, are in docs/gallery.md.
- tools/readme-tour makes the slideshow again from any result, on Linux without a screen.
- The README lists what works on main today (model tree, picking, quality colouring, view cube, clip planes with handles, regions, measuring and probes, undo, sessions, large meshes, the live MCP bridge) instead of the old V0.1 status and its "Not yet" list.
- The README slideshow and its gallery page are taken off until each slide is approved; the recipe in tools/readme-tour stays.
- The README slideshow recipe (tools/readme-tour) renders frames at twice the width they are shown (1760 x 1100, WebP q82) and keeps the stills as lossless PNG, so text stays sharp on Retina screens.
- The README slideshow is back near the top with the five approved slides (mesh, cell types under a clip plane, region of interest, measurements and a probe, light-theme pressure), sharp on Retina screens; its stills are lossless PNG. Only slides named in tools/readme-tour/approved.txt are published.
- The README slideshow has two new slides: streamline tubes from the inlet coloured by speed, and three speed slices with a smooth vorticity iso-surface. The arrows shot in tools/readme-tour now draws equal-length arrows on a speed slice, and the vorticity shots use MurmelCraft's own least-squares vorticity, which gives a much smoother iso-surface than the solver's.
- Window tests now click the spray parcels' tree row, eye, size box and
  menus, the arrows' own colour bar menu, and the quality legend's
  threshold, so a control that stops calling its tool fails in CI.
- The fast CI run takes a fraction of the time: cargo-nextest runs every test binary at once, the mesh and engine crates build with `opt-level = 1` for the tests (locally too), CI builds without debug info, and the Mesa packages come from a cache. It still runs the same checks and tests.
- F6 now reaches the player bar while a series shows its first step; it did nothing there, because the disabled "First step" button was the one it tried to focus.
- A post-processing guide ([docs/post-processing.md](docs/post-processing.md)): fields, slices,
  iso-surfaces, arrows, streamlines, derived fields, the calculator,
  probes, time series, spray parcels and charts, regions, export and
  sessions, each step with where to click and the tool it calls. A test
  runs its example script.
