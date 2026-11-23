### Changed

- Orbiting in the viewport turns about the point of the mesh under the cursor where the drag starts, or about the centre of the shown faces when the drag starts over empty space (UI spec, Viewport), so the part you grab stays under the pointer. `view.orbit` (version 2) takes `pivot`, a world point; without it the camera turns about its target as before (the view cube's drag and the arrow keys).
