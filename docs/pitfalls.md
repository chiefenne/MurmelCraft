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
- **Keys under Xvfb.** Without a window manager the window has no keyboard
  focus; send keys with `xdotool key --window <id>` after `windowfocus`.

## Display

- **Boundary polygons carry no owner cell.** KoflCraft writes the boundary as
  separate polygons (type 7). Their owner cells, which colour by cell type and
  hiding a zone or a cell type need, come from matching each polygon to a cell
  face by its sorted point ids (`murmel-mesh` `boundary::link`).
- **A closed patch has no mean normal.** The normals of a tube or a box
  cancel, and normalising the round-off gives a random direction. The patch
  properties show no mean normal when the summed vector area is below 1e-6
  of the area.

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

## Formats

- **VTK polyhedra (type 42)**: `faces` holds, per polyhedron, the face count
  and then each face as its point count and point ids; `faceoffsets` holds
  the *end* of each cell's block and `-1` for cells that are not polyhedra.
- **KoflCraft field data**: `zone_names`, `boundary_names` and
  `boundary_roles` are VTK `String` arrays written as byte values with a
  zero after each string; each string is `id=name`. `region` is the zone of
  a volume cell and the patch of a boundary polygon (type 7).
- **Prism layers are polyhedra.** KoflCraft writes its layer cells as type
  42; the cell type comes from the faces (2 triangles and 3 quads: prism).

## Input

- **Apple mouse.** `cmd` in a binding is the Command key (`mac_cmd` in
  egui); on Linux and Windows it never fires, so the Apple preset is a macOS
  preset. The Magic Mouse's right side is the secondary button.
- **A click is not a drag.** egui starts a drag only after its movement
  threshold, so a press without movement stays a click (pick), as the spec
  asks.
