# Gallery

The slides of the README's slideshow as still images. Every picture is a
screenshot of the MurmelCraft window (1440 x 900, rendered at twice that
and scaled down), set up through the live MCP bridge (`murmel mcp --live`)
by [tools/readme-tour](../tools/readme-tour/README.md).

The data: KoflCraft's Manifold fixture (a pipe with a bend, a box, a
rounded junction and an outlet pipe; 50 mm pipes) meshed by
`kofl generate-duct … polyhedral` into 229,615 cells, a Voronoi
polyhedral core with prism layers at the walls. FoehnCraft solved laminar
air flow through it (inlet 0.05 m/s, Re = 167, steady SIMPLEC, converged
in 146 iterations) and wrote `fields.vtkhdf`, which MurmelCraft opens with
its own HDF5 reader.

## The mesh

![The MurmelCraft window in the dark theme: the Manifold mesh shaded with edges, the model tree on the left, the view cube top right and the mesh properties on the right](../assets/readme/tour/01-mesh.jpg)

The mesh as it opens: shaded with edges, the model tree with cell zones,
cell types and patches, the view cube and the properties.

## Cell types under a clip plane

![The mesh coloured by cell type and cut by a flat clip plane through the outlet pipe and the box: polyhedra in teal, prisms in orange along the walls, the clip plane's slide arrow and tilt arcs in the middle](../assets/readme/tour/02-cell-types-clip.jpg)

Polyhedra and prisms by colour, cut flat through the outlet pipe and the
box; the plane's handles slide and tilt it.

## Vorticity on a slice

![A slice through the inlet pipe and the box coloured by vorticity magnitude on the inferno colour map, with the shear layers of the jet bright at the pipe wall and around the jet in the box](../assets/readme/tour/03-vorticity-slice.jpg)

The mesh draws as a faint outline while a slice shows; the colour bar
holds a fixed range.

## Speed with vector arrows

![A close view of the slice where the jet leaves the bend into the box, coloured by speed, with white arrows of the in-plane velocity showing the jet and the slow recirculation beside it](../assets/readme/tour/04-speed-arrows.jpg)

Shaded 3D arrows of the velocity in the plane of the slice, on an even
grid.

## Slices and an iso-surface

![Four parallel slices across the box coloured by speed and an orange iso-surface of speed 0.05 m/s around the cores of both pipes and the jet in the box](../assets/readme/tour/05-slices-iso-surface.jpg)

A series of four slices and the iso-surface at 0.05 m/s, made by marching
tetrahedra in every cell.

## A region of interest

![Only the cells inside a sphere around the jet in the box, cut flat to the sphere and coloured cell by cell by speed on the turbo colour map, so the polyhedra show](../assets/readme/tour/06-region-of-interest.jpg)

A sphere region cut flat: only its 2,263 cells are shown and computed,
each in its own colour (flat field style).

## Measurements and a probe

![The surface coloured by wall shear stress, a distance across the inlet (0.05 m) and from the inlet to the outlet (0.38 m), and a probe in the inlet pipe whose field values fill the properties panel](../assets/readme/tour/07-measure-probe.jpg)

Two measured distances and a probe; the selected probe shows the values
of every field at its point.

## The light theme

![The light theme: the surface coloured by pressure in fourteen bands of the cool-warm map with contour lines, the pressure falling along the inlet pipe and the outlet pipe](../assets/readme/tour/08-light-theme-pressure.jpg)

Banded pressure with contour lines in the light theme.
