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
  one. Build a `Mesh` with `faces: Polygons::new()` and
  `boundary: Polygons::new()`.

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
