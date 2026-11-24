# Pitfalls

Lessons from earlier work, so nobody pays for them twice. Add one whenever a
problem took more than a few minutes to understand.

## Rendering

- **egui samples gamma-encoded textures.** Showing the renderer's
  `Rgba8UnormSrgb` view in egui decodes the sRGB twice and the whole
  viewport turns dark. `MeshRenderer::egui_view` is a `Rgba8Unorm` view of
  the same texels; use it for egui and the sRGB view for screenshots.
- **Edges and faces fight in depth.** Faces are pushed back with a depth
  bias (`constant 2, slope 1.5`) and edges use `LessEqual`; without the bias
  edges flicker on coplanar faces.
- **Large coordinates lose precision in `f32`.** The surface is built
  relative to the centre of the bounds (`Surface::origin`) and the camera
  matrices are built relative to the same origin in `f64`.
- **The top view of a turntable.** At pitch ±90 degrees `to_eye` is parallel
  to the up axis; the screen right vector comes from the yaw alone, so the
  basis stays defined.
- **Headless Linux has no GPU.** wgpu then finds no adapter (`RENDER001`).
  Install Mesa lavapipe (`mesa-vulkan-drivers`); CI does. The window under
  Xvfb also needs `libxkbcommon-x11-0`.
- **Render tests across drivers.** Rasterisers differ in a few edge pixels,
  so the comparison allows 1 % of pixels to differ by more than 24 levels.
  Bless an intended change with `MURMEL_BLESS=1`; do not raise the tolerance.

## Window input

- **A clicked button keeps the keyboard focus in egui.** After any click,
  `Context::egui_wants_keyboard_input()` is true and the viewport keys
  (F, H, I, Esc) stop working. Check `Context::text_edit_focused()` instead:
  only a text field should keep the keys.
- **`consume_key` matches modifiers logically.** `consume_key(NONE, H)` also
  takes Alt + H. Consume the combination with more modifiers first.
- **egui cannot tell the numeric keypad from the top row.** egui-winit
  maps `Numpad1` and `Digit1` to the same `Key::Num1`, so the keypad views
  of the spec (1, 3, 7) clash with the render modes 1 to 4. The views take
  Alt (Ctrl + Alt for the opposite view), decided 2026-11-23.
- **Option + a digit types a character on a Mac.** winit reports the
  character as the logical key (Option + 7 is `|` on a German layout, so
  egui's key is `Pipe`), and `consume_shortcut` matches the logical key
  only, so Alt + 7 never fired there. The window matches shortcuts with
  Alt by the physical key too (`commands::consume`). Strings that name
  keys take `{ $ctrl }` and `{ $alt }`, which say Cmd and Option on
  macOS.
- **A colour button inside a context menu.** egui's
  `color_edit_button_srgb` opens its picker as a popup of the menu; the
  click into the picker lands outside the menu, the menu closes and the
  picker with it, so the colour entry seemed to do nothing. The tree's
  Colour entry closes the menu and opens a window of its own.
- **`Color32::gamma_multiply` scales alpha too.** `gamma_multiply(0.8)
  .to_opaque()` turns a colour dark and dull, not dimmer; mix two token
  colours instead (the view cube's `lerp`).
- **egui moves the focus with the arrow keys and drops it on Esc.** Both
  happen when the frame begins, before the window reads its keys, so
  `memory().focused()` is already `None` after Esc; the window keeps the
  focused widget of the last frame (`Focus::last`). The viewport keeps the
  arrows for orbiting with `set_focus_lock_filter`, or the first Left
  moves the focus out of it.
- **Tab follows the order widgets are created.** A `right_to_left` layout
  creates its rightmost widget first, so Tab ran backwards through the
  title bar's buttons and visited the toolbar's panel toggles before Open.
  Both bars now draw from left to right after a spacer as wide as the
  buttons were last frame.
- **A label's text runs carry its glyphs.** Setting a widget's name does
  not change the `TextRun` children AccessKit gets from the text, so an
  icon glyph inside a label's text still reaches the screen reader. Paint
  the glyph with `Painter::text` beside the label instead.
- **`get_by_label` skips labels.** kittest's queries leave out `Label`
  nodes unless asked (`include_labels`); the window test looks names up
  over all nodes.
- **Keys under Xvfb.** Without a window manager the window has no keyboard
  focus; send keys with `xdotool key --window <id>` after `windowfocus`.

## Display

- **A rolled turntable.** `view.rotate` can give any orientation; the
  camera keeps it as yaw, pitch and roll (recovered from the rotated
  frame), so mouse orbiting stays a turntable. An orbit drag must be
  turned back by the roll first (`orbit_by_drag`), or on a screen rolled
  by 180 degrees the scene moves against the pointer.
- **The flight is the window's, the camera is the session's.** A tool
  sets the final camera at once (scripts and screenshots see the end
  state); the window only draws `Camera::lerp` towards it for 250 ms.

- **Boundary polygons carry no owner cell.** KoflCraft writes the boundary as
  separate polygons (type 7). Their owner cells, which colour by cell type and
  hiding a zone or a cell type need, come from matching each polygon to a cell
  face by its sorted point ids (`murmel-mesh` `boundary::link`).
- **A closed patch has no mean normal.** The normals of a tube or a box
  cancel, and normalising the round-off gives a random direction. The patch
  properties show no mean normal when the summed vector area is below 1e-6
  of the area.

- **Name a cut point by its edge.** A flat clip makes new points where
  edges cross the plane. Naming each by the edge's two node ids (not by
  its coordinates) makes the cuts of neighbouring faces and cells meet,
  so sections close. A node exactly on the plane counts as kept, which
  gives zero-length edges but never a gap (`clip.rs`).
- **The edge colour goes to the first face drawn.** Faces are drawn
  selected first, then boundary faces in file order, then open faces; a
  shared edge takes the first face's colour. Changing this order moves
  patch-border colours in the render tests.
- **Several flat planes cut in turn.** With more than one flat clip a
  cut point is named by the plane and the two corners of the edge it
  cuts, and corners may be cut points of earlier planes. Faces are cut by
  the planes in the order they were added, and a cell's section by plane
  k is taken from the cell as the planes before k left it (their sections
  included); then the corner where two planes meet a face has the same
  name in the face and in both sections, and the cut stays closed. A
  section taken from the whole, uncut cell would name those corners
  differently and leave holes.
- **A flat cut numbers its cut points on one thread, in order**
  (`polygons` in `surface.rs`). The threads only sort the faces (whole,
  dropped, or crossed by a plane: `Sides::fate`) and build the sections
  of cells one plane slices, reading points already numbered (`View`
  over a `Store`). The numbers decide the start of each section's loop,
  so a change that makes a cut point anywhere else (a new kind of cut, a
  section that asks for a point `Cutter::cut_edges` did not name) adds it
  in that serial pass, or the surface changes with the threads; the
  reading store answers `None` for a point it lacks, and that cell is
  cut on one thread instead.
- **Cut towards the camera.** `clip.set` by axis points the normal at the
  eye, so the half nearer the viewer goes and the cut is in sight; a fixed
  +axis normal hid the cut behind the mesh from the iso view.

## Library versions (2026-11)

- **wgpu 30**: vertex buffer layouts and bind group layouts are `Option`s,
  `RequestAdapterOptions` has `apply_limit_buckets`, pipelines and render
  passes take `multiview_mask`, `BufferSlice::get_mapped_range` returns a
  `Result`, `PollType::wait_indefinitely()`.
- **glam 0.34** has no `DMat4::look_at_rh` or `perspective_rh` any more (a
  camera module replaced them); `camera.rs` builds its own matrices (depth
  0..1, right-handed).
- **egui 0.37 / eframe 0.37**: `App::ui(&mut self, ui, frame)` replaces
  `update`; panels are `egui::Panel::top(..)` and show inside a `Ui`;
  `WidgetType` became `Role` (AccessKit); `Ui::id` became `Ui::scope_id`;
  a dropped file's path is `DroppedFile::path()`.
- **quick-xml 0.42**: `local_name().as_ref()` is a `&str`;
  `Attribute::unescape_value` is deprecated for `normalized_value(version)`.
- **Mesa 26 lavapipe (CI image ubuntu-26.04)** crashed the render-test
  process (SIGSEGV, about two runs in three) when each test created and
  dropped its own Vulkan instance on its own thread; Mesa 25 never did.
  Offscreen renders share one device (`Gpu::shared`) and take turns.

## Mesh model

- **`Polygons::default()` is not `Polygons::new()`.** The derived default
  has no leading 0 in `start`, so pushing into it shifts every polygon by
  one. Build a `Mesh` with `boundary: Polygons::new()`, and per-cell
  faces for `boundary::link` with `Polygons::new()`.
- **Each face is stored once** (`Mesh::distinct`, as its owner lists it).
  `Mesh::cell(c)` yields `CellFace`s: `nodes` is the stored order, which
  is the neighbour's face turned the other way. Code where the turn
  matters (volumes, winding numbers, face normals for quality, sections)
  reads `CellFace::oriented()` or `CellFace::at(i)`, which give the
  cell's own order; code that only needs the points (bounds, kept cells,
  centroids) reads `nodes` or `Mesh::cell_points`.

## Large meshes

- **Measure first, with `murmel bench`** ([performance.md](performance.md)).
  The binary `.vtu` was slower than the ascii one, and hashing the files a
  second time cost as much as reading them; neither showed without the
  numbers.
- **A test of a parallel path runs it on several threads.** `par::ranges`
  hands small inputs to one thread, so the test fixtures never split;
  wrap the check in `par::with_threads(4, …)`, which splits any input.
- **Pieces of a text are cut at a separator.** A piece cut inside a number
  reads one number as two; `numbers::pieces` moves each cut forward to the
  next separator, and base64 is cut at whole four-character groups.
- **The window reads large meshes on their own thread** (`opening.rs`) and
  opens the result through `mesh.open` from `Session::preloaded`. The
  preloaded mesh is used only for the same path with its files unchanged,
  and its surface only for the same display and tokens; anything else is
  read or built again. Meshes under 32 MiB open at once, which keeps the
  window tests synchronous; a test of the background path sets
  `background_bytes` to 0 and holds the reader with `opening_gate`.
- **The `.vtu` reader streams; it never holds the file's text.** Each
  array is parsed as it streams past, into the type its use needs, and
  `vtu::kind_of` decides which arrays are parsed at all: an array it does
  not name is passed over unread. A new array the reader uses goes into
  `kind_of` first. A chunk can end anywhere (inside a number, a base64
  group, a tag), so a change to the streaming path is tested with the
  file handed over a few bytes at a time (`streams_in_any_pieces`).
- **Parsed numbers are filled into room, not pushed or resized**
  (`numbers::Filling`). The room comes zeroed from the allocator, sized
  from the counts the file declares (or a guess from the density of the
  first megabyte), and the parsing threads are the first to write it;
  `Vec::resize` would write every page on one thread first. The token
  count tests bytes with compares instead of the separator table
  (`numbers::count_tokens`), so a separator stays a control byte, a space
  or a parenthesis: with any other, the count and the parse disagree and
  every array that uses it fails to read.
- **The face topology numbers faces on all threads** (`boundary::number`):
  a distinct face's number is its place among the first faces before it,
  and its neighbour the smallest other cell, so the result is the same
  for any split. A change that keeps more per face (a third cell, say)
  keeps it in a form whose result does not depend on which thread came
  first, such as a minimum, or the meshes differ from run to run.
- **The window hashes a mesh after it opens** (`Session::hash_after_open`):
  until the digests are taken on their own thread, `OpenMesh::digest` and the
  files' digests are empty and the badge is `Validity::Checking`. A tool
  that needs the digest (`session.save` writes it, `session.load` compares
  it) calls `Session::wait_digest` first. The command line and agents on a
  headless session hash while reading, as before, so `mesh.open` returns
  the digest. A window test that tells meshes apart compares paths, not
  digests.
- **Nothing per frame walks the mesh.** The model tree and the legend ask
  for `Mesh::summary` and `all_items` on every frame; the counts and the
  bounds are taken the first time and kept in the mesh (a `OnceLock`), as
  a mesh is not changed once read. A method that changes a read mesh
  resets them (`apply_evidence_roles` does); changing the public fields of
  a mesh after asking for its summary leaves stale counts. A frame that
  needs a count over the cells takes it from the summary or the surface
  (`Surface::cells`), never from a loop of its own.
- **The window defers clip cuts** (`Session::defer_clips`): after a tool
  moves a clip plane, `OpenMesh::surface` is still the old cut until the
  preview ends (`Session::poll_surface`), and the renderer draws
  `Session::drawn`, not `surface`. Code that reads the surface outside a
  tool calls `Session::settle` first (tools do it for you, except
  `clip.set` and `clip.rotate`, which keep a drag going). A window test
  that moves a plane and then looks at the surface settles too.
- **The window may leave the edges out while the view moves**
  (`edges::MovingEdges`, when frames with edges took over 33 ms). The
  viewport's render key holds the edges, so a frame at rest draws them
  again; a window test that looks at a frame while the camera moves on a
  slow machine may see none. Offscreen pictures always draw them.
- **The surface's buffers are filled on all threads** (`Fill::chunk`,
  `Lines::draw` in `surface.rs`). Each run of polygons writes at the
  place the counts before it give, so a change to how a polygon is
  triangulated changes the counts (`vstart`, `istart`) with it, or the
  runs overlap. The edges keep the first polygon's colour in draw order
  however many threads there are; `threads_give_the_same_surface`
  checks both.
- **Sessions that remember checked files write to the settings folder**
  (`Session::remember_checked`, `checked-files.json`; the window and the
  command line turn it on). A test that opens meshes with it on sets
  `config_dir` to a folder of its own. A file modified less than
  `checked::SETTLED` (3 s) ago is not remembered, so a test that writes
  a mesh file and expects it remembered sets its modification time back
  first.
- **The `.vtu` cells are read in ranges, in two passes** (`vtu/cells.rs`).
  Both passes walk the cells with the same `walk`, so a new check goes
  there and holds in both. A range starts in the polyhedra's `faces`
  stream where the last polyhedron before it ends (the face offsets of
  other cells are -1), and its first error counts only when no range
  before it has one.
- **A `.vtu` of polyhedra and polygons leaves its cell faces in the
  stream** (`InStream`: where each face's point count is), so the faces
  that `boundary::link` reads are a `FaceList`, not always `Polygons`.
  Code that needs the cell faces after reading takes them from the
  mesh's distinct faces, which are copied out as before.
- **A span names what the window shows while a mesh opens.**
  `profile::span` also sets the phase that the opening overlay shows
  (`opening.rs`, `phase_message`), so a new span inside a reader needs
  its name mapped there, or the window says "reading the file" while it
  runs. The `topology.*` steps map to "matching the faces".
- **Linking trusts the smallest points a reader hands it.**
  `boundary::link` sorts the faces by the smallest point that the `.vtu`
  reader noted for each (`Cells::least`). A reader that changes a face's
  points after noting them hands `None` instead, or the face lands in the
  wrong group and loses its twin. Turning a face round keeps its points
  and its smallest one.
- **An HDF5 file opened by its path holds only its structure**
  (`H5::open`): the object headers, groups, attributes and chunk indices
  are read in pages when the file opens, by walking every object until
  nothing the walk needs is missing (`load_structure`), and the data of
  a dataset is read from the file each time it is asked for. Read a
  dataset once and keep the values (`H5::values`, `hdf5::Values`), not
  once per use. A new kind of structure the reader follows (a new chunk
  index, say) goes into `H5::walk` too, or it is missing when read
  (`MESH016`); `opening_a_path_reads_what_the_bytes_read` compares every
  fixture read both ways. `H5::from_bytes` holds the whole file, as
  before.
- **The VTKHDF cells are read in units, in two passes**
  (`vtkhdf/cells.rs`), as the `.vtu` cells are in ranges. A unit checks
  each partition when it comes to it; a partition without cells is
  checked by the unit at whose first row it lies, and the last unit
  checks those after the last cell, so every partition is checked once,
  in file order. A new check of a partition goes into `enter`, which
  both passes call.
- **A unit of VTKHDF cells holds only its window of each dataset**
  (`Window::of`): the rows its cells use, found from the offsets it read
  before. `hdf5::Values::int` of a row outside the window panics, so a
  new dataset the walk reads, or a new row of one (a neighbour's offset,
  say), goes into `Window::of` first; `threads_give_the_cells_of_one_walk`
  reads in units of 1 to 1,000 cells against one walk over the whole
  datasets. The counting pass does not read `Connectivity` and
  `FaceConnectivity` (`Points::Skip`): a check that needs the point ids
  belongs in the walk that writes, and the counting pass's error is
  reported only after a pass that reads the ids finds nothing before it.
- **Picking tests boxes first.** `Surface::tri_boxes` holds a box per run
  of `PICK_RUN` triangles and is filled at the end of `build_with`; code
  that adds triangles to a surface adds them before that (as the slices,
  iso-surfaces and arrows do), or picking falls back to walking every
  triangle (the box count no longer matches).

## Serde

- **`flatten` and `deny_unknown_fields` do not combine.** A flattened
  struct with `deny_unknown_fields` refuses the outer struct's fields;
  `settings.set_colors` takes `reset` out of the JSON before parsing the
  colours instead.
- **serde_json parses floats fast, not exactly.** Without its
  `float_roundtrip` feature a number written by serde_json can come back
  one unit in the last place off (a camera pitch of …658 read as …654),
  so a saved session did not load to the same camera. The workspace turns
  the feature on.

## Formats

- **VTK polyhedra (type 42)**: `faces` holds, per polyhedron, the face count
  and then each face as its point count and point ids; `faceoffsets` holds
  the *end* of each cell's block and `-1` for cells that are not polyhedra.
- **KoflCraft field data**: `zone_names`, `boundary_names` and
  `boundary_roles` are VTK `String` arrays written as byte values with a
  zero after each string; each string is `id=name`. `region` is the zone of
  a volume cell and the patch of a boundary polygon (type 7).
- **OpenFOAM faces turn from the owner to the neighbour.** A face is
  outward for its owner only; the neighbour's copy is reversed (first node
  kept) before it joins the neighbour's cell. The reader's tests check that
  every cell closes (Newell's area vectors sum to zero) and has a positive
  volume.
- **OpenFOAM binary lists.** The count, `(`, then the raw values, then
  `)`; no comment or white space is skipped inside. Widths and byte order
  come from the header's `arch` (`LSB;label=32;scalar=64` when absent).
  `faceCompactList` is two label lists: N + 1 offsets, then the nodes.
- **OpenFOAM evidence lives in the polyMesh folder.** KoflCraft's
  `export … openfoam DIR` writes `mesh-evidence.json` among the polyMesh
  files and lists the digest of each; the validity badge checks them all
  (a `.gz` file is looked up by its plain name).
- **Prism layers are polyhedra.** KoflCraft writes its layer cells as type
  42; the cell type comes from the faces (2 triangles and 3 quads: prism).

## Input

- **Apple mouse.** `cmd` in a binding is the Command key (`mac_cmd` in
  egui); on Linux and Windows it never fires, so the Apple preset is a macOS
  preset. The Magic Mouse's right side is the secondary button.
- **A handle must not cover the viewport.** An egui widget's rect takes
  the pointer from the viewport below it, so a handle that registered its
  whole bounding box blocked orbiting in a large square round a diagonal
  arrow. The clip handles register a small square at the nearest point of
  their centre line only while the pointer is within reach, and the whole
  viewport only while their own drag lasts (`clip_handle.rs`).
- **A tilt needs a fixed axis and pivot for the whole drag.** Recomputing
  the plane's axes and centre every frame let the plane drift and the
  arcs jump; the drag keeps the pivot, the axis and which side each arc
  sits on from the press to the release (`ClipDrag`).
- **A click is not a drag.** egui starts a drag only after its movement
  threshold, so a press without movement stays a click (pick), as the spec
  asks.

## MCP server

- **Relative paths start at the first write root.** The MCP server never
  resolves a path from its own working folder (the client chose it, not
  the person), so `test-fixtures/x.vtu` is looked for in the write root and
  fails with `MCP002`; pass absolute paths or paths below the write root.
- **A new path field must be declared.** The server confines only the input
  fields a tool lists in `reads` or `outputs`; a test catches fields named
  like paths (`path`, `mesh`, `file`, `folder`, `dir`, `output`), so name
  path fields that way.
- **Clippy in integration tests.** `allow-unwrap-in-tests` covers `#[test]`
  functions and `#[cfg(test)]` modules, not helpers in a `tests/*.rs` file;
  those files allow `clippy::unwrap_used` at the top (as `render.rs` does).
- **The live bridge runs calls on the window's thread.** Agents' calls
  queue in the listener and run at the start of the next frame
  (`MurmelApp::serve_agents`); the connection thread only wakes the window
  (`request_repaint`). A window that does not draw (minimised on macOS)
  does not answer until it draws again, so the client waits at most two
  minutes (`LIVE002`).
- **The window's session has a config folder.** Over the bridge the
  settings tools must not write the person's files, so the window takes
  `config_dir` away for the call and puts it back (as the headless server
  drops it); a tool that reads the config folder sees none there.
- **A window test listens on its own socket.** `MurmelApp::new` sets the
  default socket path; window tests set `bridge_path` to `None` (or to a
  scratch path), or two test runs, or a test and a real window, fight over
  one socket (`LIVE003`).
