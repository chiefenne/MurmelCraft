### Changed

- Vector arrows are denser and smoother: 40 across each slice's own extent by default (was 25 across the whole mesh, so a cross-section of a long duct got only a few), with twelve sides instead of eight. The density counts across each slice, so every slice gets a full grid of arrows whatever its size.
- Arrows have a floor on their length by default: as long as the magnitude, but at least a quarter of the longest, so the direction still shows where the flow is slow.
- Arrows are coloured by their own magnitude by default, with a colour bar of their own beside the mesh's legend (in the window and in exported pictures), in their field's style; its title opens their colouring, colour map and range. Solid and "by the field shown" remain.

### Added

- `arrows.set` takes `length`: `magnitude` (strictly as long as the magnitude), `floor` (the default) or `equal` (all the same length, the magnitude in the colour only), and `color` `magnitude`; the result reports each slice's spacing and the colour range. The arrows' context menu in the model tree has Length and Colour submenus, the palette has a command for each, and the Properties panel shows the length mode. Session files keep the length (format 1.10); files before 1.10 open with the new defaults.
