# Post-processing guide

How to look at the results of a FoehnCraft run in MurmelCraft: colour the
mesh by a field, cut it with slices, draw iso-surfaces, vector arrows and
streamlines, derive new quantities, write your own formulas, read values
with probes, play a time series, show spray parcels and charts, keep to a
region of a large mesh, and export pictures and movies.

Everything in the window is a tool of the registry, so each step below
also runs from the command line (`murmel call`, `murmel run --script`) and
from an agent (`murmel mcp`). The window column of each table says where
to click; the tool column names what it calls. The exact inputs of every
tool are in [tools.md](tools.md), every key in [keys.md](keys.md).
MurmelCraft's post-processing is its own Rust code (no VTK or ParaView);
the methods are cited in the code and in this guide.

Contents:

1. [Open a result](#1-open-a-result)
2. [Colour the mesh by a field](#2-colour-the-mesh-by-a-field)
3. [Clip planes and the mesh's look](#3-clip-planes-and-the-meshs-look)
4. [Slices](#4-slices)
5. [Iso-surfaces](#5-iso-surfaces)
6. [Vector arrows](#6-vector-arrows)
7. [Streamlines](#7-streamlines)
8. [Derived fields](#8-derived-fields)
9. [The field calculator](#9-the-field-calculator)
10. [Probes and measurements](#10-probes-and-measurements)
11. [Time series](#11-time-series)
12. [Spray parcels and charts](#12-spray-parcels-and-charts)
13. [Regions of interest and large meshes](#13-regions-of-interest-and-large-meshes)
14. [Pictures and movies](#14-pictures-and-movies)
15. [Keep your work: sessions and defaults](#15-keep-your-work-sessions-and-defaults)
16. [The same from a script](#16-the-same-from-a-script)
17. [What the numbers are](#17-what-the-numbers-are)

## 1. Open a result

| What | Window | Tool |
|---|---|---|
| A FoehnCraft result (`fields.vtkhdf`, `fields_<step>.vtkhdf`, `derived.vtkhdf`) | File ▸ Open (Ctrl + O, Cmd + O on macOS), File ▸ Open recent, or drop the file on the window | `mesh.open` |
| A KoflCraft mesh with its cell and point data (`foehncraft.vtu`) | the same | `mesh.open` |
| A `.vtu` or `.vtkhdf` of another tool, an OpenFOAM case | File ▸ Import (VTK file…, OpenFOAM polyMesh…); the title bar then says "imported" | `mesh.open` |
| The fields of the open result | Colour by (toolbar), or the Properties of the mesh | `field.list` |

A `.vtkhdf` result holds the mesh and the fields in one HDF5 file, read by
MurmelCraft's own HDF5 reader. Cell fields (one value per cell, the
solver's own), point fields and boundary fields (wall shear stress, y+ on
the boundary patches) are all read; `velocity_x`, `velocity_y` and
`velocity_z` are read as one vector `velocity`. The file's units, its
time, step or iteration show in `field.list` and in the Properties.

An OpenFOAM `polyMesh` opens as a mesh; its time folders are not read as
fields yet.

## 2. Colour the mesh by a field

| What | Window | Tool |
|---|---|---|
| Colour by a field | Toolbar, Colour by, "Field: <name>"; or the palette (Ctrl + K), "Colour by: field" | `display.color_by` `{"by": "field", "field": "pressure"}` |
| A vector's magnitude or one component | the colour bar's title, Component | `display.color_by` `component` |
| Flat, smooth or banded | the colour bar's title, Colouring; B cycles | `field.style` `mode` |
| Contour lines | the colour bar's title, Contour lines; L | `field.style` `lines`, `line_count` |
| Colour map, reversed | the colour bar's title, Colour map, Reverse | `field.style` `map`, `reverse` |
| Range | the colour bar's title, Range | `field.style` `range` |
| Back to the default style | the colour bar's title, Default style | `field.style` `{"reset": true}` |

The colour bar sits at the right edge of the viewport, below the view
cube. Its title is a button: it opens the field's settings, and the menu
stays open while you try one setting after another.

- **Flat** gives each cell one colour, the solver's own value. Use it to
  see what the solver computed, cell by cell.
- **Smooth** (the default) blends node values across the faces. The node
  values are rebuilt from least-squares gradients of the cells, so a
  peak inside a coarse cell layer is not averaged away.
- **Banded** splits the range into even steps of colour (2 to 64 bands,
  12 by default).
- **Contour lines** are drawn where the field crosses `line_count + 1`
  even steps of the range (12 lines by default).

Colour maps: with the map left automatic, a field is drawn in Rainbow
desaturated, and a range centred on zero (such as the symmetric range)
in Rainbow uniform.
Inferno, Cool to warm, Viridis, Plasma, Magma, Turbo, Rainbow (jet) and
Grey are there too.

Ranges:

| Range | Meaning |
|---|---|
| Whole field (default) | the field's smallest to largest value |
| Shown cells | what is drawn now (after clips, hiding and regions) |
| Symmetric about zero | −max\|v\| to +max\|v\| |
| Fixed | your min and max; the palette's "Field range: fix as shown now" freezes the range shown |

Each field keeps its own style: switching from pressure to speed and back
brings back the pressure's style. Right-click a setting, or use the
palette's "Defaults: colour new fields like the field shown", to keep the
style as your default for fields shown for the first time
([section 15](#15-keep-your-work-sessions-and-defaults)).

The colour bar is also how iso-surfaces are placed: a click on it adds an
iso-surface at that value ([section 5](#5-iso-surfaces)).

## 3. Clip planes and the mesh's look

Clip planes cut the mesh open so the inside shows; they cut slices,
iso-surfaces and spray parcels too.

| What | Window | Tool |
|---|---|---|
| Clip normal to X, Y or Z | X, Y, Z | `clip.set` `axis` |
| Add a plane (up to 8) | Alt + C, or the tree's Clip planes, "Add clip plane" | `clip.set` `{"new": true}` |
| Move it | drag the slide arrow; Alt + Shift + Up and Down; the toolbar's slider | `clip.set` `offset` |
| Tilt it | drag a tilt arc; Alt + arrow keys | `clip.rotate` |
| Crinkle (whole cells) or flat (cut cells) | the toolbar's Clip mode | `clip.set` `mode` |
| Flip the side cut away | Shift + C, the toolbar's Flip | `clip.set` `flip` |

While slices, iso-surfaces, streamlines or parcels show, the mesh draws
as a faint outline (its feature edges) so what is inside stands out. Tick
"Show the mesh faces" in the tree, or use the palette's "Mesh beside
slices: faces", to see the faces (`display.mesh_look`).

## 4. Slices

A slice is a plane that cuts nothing away and draws the section of the
cells it crosses in the mesh's colours: the field shown, in its style.

| What | Window | Tool |
|---|---|---|
| Slice normal to X, Y or Z | Shift + X, Shift + Y, Shift + Z | `slice.set` `axis` |
| Add a slice (up to 8) | Alt + S, or the tree's "Add slice" | `slice.set` `{"new": true}` |
| Along the view direction | the palette, "Slice along the view direction" | `slice.set` `{"axis": "view"}` |
| Move or tilt it | select its row in the tree, then drag the slide arrow or a tilt arc | `slice.set` `offset`, `slice.rotate` |
| A series of planes | the palette, "Slice: a series of 5 through the mesh"; or `count` and `spacing` | `slice.set` `count`, `spacing` |
| Show, hide, remove | the row's eye and context menu | `slice.set` `enabled`, `slice.clear` |

A series of `count` planes (1 to 64) lies `spacing` apart along the
normal, the middle one at the origin; a count without a spacing spreads
the planes evenly through the mesh.

Slices are cut through the same tetrahedra as the iso-surfaces: each
cell is split into tetrahedra from its centroid (the cell's value), its
face centres and its nodes, and the plane's section is found in each of
them. So a slice shows the solver's peak values even on a coarse mesh.

## 5. Iso-surfaces

| What | Window | Tool |
|---|---|---|
| An iso-surface of the field shown | Alt + I (at the middle of its range), or click the colour bar (at that value) | `iso.set` `{"new": true, "value": …}` |
| Move it to another value | select its row, then click the colour bar | `iso.set` `value` |
| Of another field, or a vector's component | (tool or script) | `iso.set` `field`, `component` |
| Solid or coloured by the field shown | the palette, "Iso-surface: solid colour" or "… coloured by the field shown" | `iso.set` `color` |
| Show, hide, remove | the row's eye and context menu | `iso.set` `enabled`, `iso.clear` |

Up to 8 iso-surfaces, each of one value. They are made by marching
tetrahedra (the tetrahedra of [section 4](#4-slices)) in every cell
shown, so they close across cells; clip planes cut them. A solid
iso-surface takes the patch hues in turn; `display.set_color` on
`iso:<n>` gives it a colour of your own.

For vortex cores, derive Q or λ₂ first ([section 8](#8-derived-fields)),
then add an iso-surface of `q_criterion` above zero or of `lambda2` below
zero, coloured by the speed shown on the mesh.

## 6. Vector arrows

| What | Window | Tool |
|---|---|---|
| Show or hide arrows | Alt + V, or the tree's "Add vector arrows" | `arrows.set` |
| How many | the row's Across box (4 to 200, default 25); palette "more" or "fewer" | `arrows.set` `density` |
| How long | the row's Scale box; palette "longer" or "shorter" | `arrows.set` `scale` |
| Length rule | palette: as long as the magnitude; at least a quarter of the longest; all the same length | `arrows.set` `length` |
| On the surface too | the row's context menu, or the palette | `arrows.set` `surface` |
| Only the part in the slice's plane | the row's context menu, or the palette | `arrows.set` `in_plane` |
| Colour | the row's context menu, or the title of their colour bar | `arrows.set` `color` |
| Remove | the context menu | `arrows.clear` |

Arrows stand on every slice shown, on an even square grid anchored at the
mesh's centre, and with `surface` on the mesh surface shown (one boundary
face per cube of the grid). With neither a slice nor the surface they
have nowhere to stand; the tree says so ("Add a slice, or put them on the
surface, to see them").

At scale 1 the largest magnitude is 0.9 of a grid spacing long. With
`in_plane`, only the part of each vector in the slice's plane is drawn,
which shows the swirl in a cross-section.

Colours: by their own magnitude (the default, with a colour bar of their
own left of the mesh's), by the field on the mesh's colour bar, or one
solid colour. The title of their colour bar opens their colouring and
their field's style (map, range, bands), apart from the mesh's field.

At most 10 000 arrows are drawn; the result of `arrows.set` says when the
cap was hit.

## 7. Streamlines

| What | Window | Tool |
|---|---|---|
| Add a set (up to 8) | Alt + L, or the tree's "Add streamlines": from a line of seeds, a ball, a patch or a plane | `stream.set` `{"new": true, "seed": …}` |
| Move the seeds | select the set, then drag the seed line's end dots or its middle | `stream.set` `from`, `to` |
| How many seeds | the row's Seeds box (1 to 1000, default 50); palette "more" or "fewer" | `stream.set` `count` |
| Direction | palette: downstream only, upstream only, both ways | `stream.set` `direction` |
| Tubes or thin lines, arrowheads | the row's context menu, or the palette | `stream.set` `look`, `arrowheads`, `tube_radius` |
| Colour | the row's context menu, or the title of their colour bar | `stream.set` `color`, `color_field` |
| Remove | the context menu | `stream.clear` |

Seeds:

| Seed | Where the seeds are |
|---|---|
| line | `count` points evenly from `from` to `to`, ends included; a new line lies across the mesh near the low end of its longest side |
| sphere | the centre and Halton points through the ball |
| patch | a boundary patch's faces, thinned evenly to about `count`; a new set takes the first inlet |
| plane | an even grid of about `count` points inside the mesh |

The lines follow the vector field through the cells' tetrahedra (the
field the slices show) by fourth-order Runge-Kutta in arc length, a
quarter of a cell per step. A line stops where it leaves the mesh (or the
regions shown), where the speed falls below 0.1 % of the field's largest,
at the length limit (three mesh diagonals unless set) or after 10 000
steps. The set's Properties count how many lines ended each way, and how
many seeds fell outside the mesh.

Streamlines are coloured by the magnitude of the traced field by default,
with a colour bar of their own; while the mesh shows the same field, they
share the mesh's bar.

## 8. Derived fields

Quantities computed from the result's fields, then used like any field of
the file: colour by them, slice them, iso-surfaces, arrows and
streamlines of them.

| What | Window | Tool |
|---|---|---|
| The catalogue | palette "Derived fields…", or "Derived fields…" at the end of Colour by | `field.catalogue` |
| Compute one | its Compute button; or palette "Derive vorticity" and the others | `field.derive` `quantity` |
| Show it | its Show button | `display.color_by` |
| Remove it | its Remove button | `field.derive` `{"remove": true}` |

| Quantity | Field | Formula | Unit |
|---|---|---|---|
| Vorticity | `vorticity` (vector) | ω = ∇ × u | 1/s |
| Q criterion | `q_criterion` | Q = ½ (‖Ω‖² − ‖S‖²), vortices where Q > 0 | 1/s² |
| λ₂ criterion | `lambda2` | the middle eigenvalue of S² + Ω², vortices where λ₂ < 0 | 1/s² |
| Total pressure | `total_pressure` | p + ½ ρ u·u | Pa |
| Pressure coefficient | `pressure_coefficient` | (p − p_ref) / (½ ρ_ref U_ref²) | 1 |
| Helicity | `helicity` | H = u·ω | m/s² |
| Normalised helicity | `normalized_helicity` | u·ω / (\|u\| \|ω\|), ±1 along a vortex core | 1 |
| Swirling strength | `swirling_strength` | λ_ci, the imaginary part of the complex eigenvalues of ∇u | 1/s |
| Turbulence intensity | `turbulence_intensity` | √(2k/3) / U_ref | 1 |
| Local turbulence intensity | `local_turbulence_intensity` | √(2k/3) / \|u\| (no value where the flow stands) | 1 |

S and Ω are the symmetric and antisymmetric parts of the velocity
gradient, taken in each cell by least squares from the cells around it;
the vortex quantities are cell fields. The pressure quantities are taken
wherever the pressure is (cells, nodes, boundary faces). The inputs are
found by name (velocity, U, u; pressure, p, static_pressure); the window
greys out a quantity whose input is missing and says which.

A pressure in m²/s² is kinematic and takes no density; a density field
(density, rho) wins over the reference density. The reference values of
the pressure quantities come, in this order, from:

1. FoehnCraft's `case.resolved.toml` beside the result,
2. your defaults (right-click a reference value in the window),
3. air at sea level: ρ_ref 1.225 kg/m³, p_ref 0, U_ref the largest speed.

The window says where each value came from, and you can type your own.
A session file keeps how each field was made, and makes it again when it
is loaded.

Turbomachinery, turbulence and wall quantities come from FoehnCraft
(`foehn post derive`); the window says so below the catalogue.

FoehnCraft's own wall heat fields (FoehnCraft 2026-11-27 or later, a
case with `[energy]`, a restart written by that version) are rows under
Wall that run `foehn post derive`: `wall_heat_flux` (W/m², positive into
the fluid) with `wall_temperature`, `heat_transfer_coefficient`
q_w / (T_w − T_P), and `nusselt_number` h L / k(T_w), which also needs
`[output.reference] length` in the case. They are on the wall faces.

The Statistics group works on FoehnCraft's `statistics.vtkhdf`, which a
transient run writes with `[output.statistics] fields = true`: the RMS
`rms_velocity_x` … (read as the vector `rms_velocity`), the covariances
`covariance_velocity_xy` … and `covariance_velocity_x_temperature` …,
and `mean_k`. Open that file, then derive:

| Quantity | Field | Formula | Unit |
|---|---|---|---|
| Resolved kinetic energy | `resolved_kinetic_energy` | ½ (u′² + v′² + w′²) | m²/s² |
| Total kinetic energy | `total_kinetic_energy` | resolved plus the mean modelled k | m²/s² |
| Resolved Reynolds stress xx … xz | `resolved_reynolds_stress_xx` … | ⟨u′u′⟩ = u′_rms², …, ⟨u′v′⟩, … | m²/s² |
| Anisotropy η, ξ | `anisotropy_eta`, `anisotropy_xi` | the Lumley triangle of b = ⟨u′u′⟩/(2k) − ⅓ I (Pope 2000, §11.3) | 1 |
| Turbulent heat flux | `turbulent_heat_flux` (vector) | ⟨u′T′⟩ | K m/s |

Isotropic turbulence sits at η = ξ = 0, one-component turbulence at
(⅓, ⅓) and axisymmetric two-component turbulence at (⅙, −⅙).

**Mean flow.** On a result with time means (`mean_velocity`,
`mean_pressure`, `mean_k`), the window shows a switch "Of the
instantaneous flow | mean flow". On the mean flow, every quantity of the
velocity, the pressure and k is taken of the means and named
`mean_<field>` (`mean_vorticity`, `mean_q_criterion`,
`mean_pressure_coefficient`, …), so both can sit side by side. A file
with only the means (`statistics.vtkhdf`) starts on the mean flow;
right-click the switch to keep your choice as the default. The tool takes
`"mean": true`; quantities of fields the statistics do not average (ω, ε,
ν_t, the wall distance) are refused (FIELD021).

## 9. The field calculator

| What | Window | Tool |
|---|---|---|
| Compute a formula | palette "Field calculator…" (the Your formulas tab of Derived fields): name, formula, unit, Compute | `field.calculate` `name`, `formula`, `unit` |
| Keep it for every result | Keep | `settings.keep_formula` |
| Edit, forget | the formula's row | `field.calculate`, `settings.forget_formula` |
| Remove the field | (tool or script) | `field.calculate` `{"name": …, "remove": true}` |

A formula reads the result's fields by name and gives a scalar or a
vector field:

```text
mag(velocity)                 speed from the velocity
p - 101325                    gauge pressure
0.5 * 1.225 * dot(U, U)       dynamic pressure
dot(U, curl(U))               helicity density
if(x > 0, p, 0)               the pressure downstream of x = 0 only
[U.x, U.y, 0]                 the velocity in the x-y plane
"p rgh" / 1000                a field whose name has a space, in kPa
```

- Numbers, `pi`, the coordinates `x`, `y`, `z` and the position vector
  `pos` (of each cell centroid, node or face centre).
- `+ - * / ^`, and the comparisons `< <= > >=` (1 or 0).
- Components `U.x`, `U.y`, `U.z`; vectors `[a, b, c]`.
- Functions `sqrt abs exp log log10 sin cos tan asin acos atan atan2
  sinh cosh tanh sign floor ceil min max if mag dot cross grad curl div`
  (the window lists each with its arguments).
- `grad`, `curl` and `div` are least-squares gradients, exact for linear
  fields; a formula that takes one is a cell field.

The result sits wherever every field it reads is (cells, nodes, boundary
faces). Values that are not finite count as no value (`no_value` in the
result). An error names the column where the formula went wrong.

A field made from another is computed again when that one changes
(`made_again` in the result); a formula that reads its own field, or a
field made from it, is refused. Kept formulas live in `settings.toml` in
the config folder and show for every result.

## 10. Probes and measurements

| What | Window | Tool |
|---|---|---|
| Place a probe | Shift + P, then click a point of the mesh; or the tree's "Place a probe" | `probe.add` |
| What a probe would read, without keeping it | | `probe.at` |
| Measure a distance | M, then click two points | `measure.add` |
| Measure an angle | Shift + M, then point, vertex, point | `measure.add` |
| Rename, delete | the row's context menu; Delete or Backspace | `measure.rename`, `measure.remove` |
| A probe's values over the run | the Probes group's "History…", or palette "Probe history" | `probe.history` |
| Export them as CSV | the history window's "Export CSV…" | `probe.export` |

Clicks snap to nodes, edge midpoints and face centres (hold Alt, Option
on macOS, for no snapping); Esc stops. Points can also be typed
coordinates, a node, the midpoint of two nodes, or a face or cell centre
(see `measure.add` in [tools.md](tools.md)).

A probe reads the cell holding its point: its type, zone, centroid and
the distance to it, its volume and quality, and every field's value at
the point, derived and calculated fields included. A cell field's value
at the point is the cell's own value plus its least-squares gradient
times the way from the centroid, as a finite-volume solver reconstructs
it; a node field's is the cell's linear fit to its nodes. The probe's
tree row shows the value of the field colouring the mesh; the Properties
show all of them.

The Probe history window reads every step of the open time series on a
thread of its own (Cancel stops it), plots the chosen field at each probe
over the run, and writes a CSV with one line per step and one column per
probe, field and component. Without a time series it shows the step
shown.

Probes and measurements are tree items, kept in session files; undo
removes a new one.

## 11. Time series

| What | Window | Tool |
|---|---|---|
| Open a series | palette "Open time series…": a `series.json` or an output folder | `series.open` |
| Play or pause | Space, or the player bar | `series.play` |
| Next and previous step | Period and Comma, or the player bar | `series.go` `by` |
| First and last step | the player bar, or the palette | `series.go` `to` |
| Any step | the player bar's slider | `series.go` `index`, `step`, `time` |
| Speed and loop | the player bar's Speed (0.1 to 60 steps a second, default 4) and Loop | `series.play` `rate`, `loop` |

A series is FoehnCraft's `series.json` (format `foehncraft.series` 1) or
an output folder: its `series.json`, else its `fields_<step>.vtkhdf` and
`spray/parcels_<step>.vtkhdf` matched by step. Each step opens its fields
file as the next time step of the same mesh: the display, the derived and
calculated fields (made again from the new values), the camera and the
undo history stay; only the values change.

The player bar shows below the viewport while a series is open; the
status bar shows the step and its time. Steps are read on the window's
thread, so a large mesh plays as fast as it reads. A movie of the steps is
`export.movie` with motion `series` ([section 14](#14-pictures-and-movies)).

## 12. Spray parcels and charts

| What | Window | Tool |
|---|---|---|
| Show the parcels of a file | the tree's "Open spray parcels…", or the palette | `parcels.open` |
| Colour, size, filters, colour map, range | the parcels' row: right-click; or the title of their colour bar | `parcels.set` |
| Size factor | the row's Size box (1: 1/400 of the mesh diagonal) | `parcels.set` `scale` |
| Show or hide | the row's eye | `parcels.set` `enabled` |
| Remove | the context menu | `parcels.clear` |
| Open the charts of a run | palette "Open the charts of a run…" (a series opens its charts too) | `charts.open` |
| Choose a chart | the chart panel's Chart box, or the palette | `charts.show` |
| Save a chart as PNG or SVG | the chart panel's "Save…" | `charts.export` |
| The statistics at a step | the chart panel's Statistics | `stats.at` |

Parcels come from FoehnCraft's `spray/parcels_<step>.vtkhdf` (points
with velocity, diameter, droplets, age, parcel id; time and step) or any
VTKHDF PolyData of points. Each parcel is a small shaded sphere, all the
same size or sized by diameter (the largest droplet the fixed size, the
others in proportion). The first file is coloured by diameter with a
colour bar of its own; the next file of a run keeps the settings. The
diameter filter shows only the parcels within a range (typed in
micrometres in the window). Parcels on the cut-away side of a clip plane
are left out. At most 200 000 are drawn, thinned evenly beyond; the row
says "thinned" when that happens.

The chart panel shows above the bottom of the viewport: the liquid mass
flow through the datums, the liquid through them so far, the spray
penetration per injector, the Sauter mean diameter D32, the droplet
sizes of one datum, or columns of FoehnCraft's tables. The time series'
step is the chart's cursor. The statistics give the mass book (injected,
in the flow, on walls, left the domain, lost), D32 and the penetration
at the step, and per datum the flow through it and its D10, D32 and
Dv50.

## 13. Regions of interest and large meshes

| What | Window | Tool |
|---|---|---|
| Add a box region, or show and hide | Alt + R, or the tree's Regions of interest | `region.set` |
| A sphere, a slab, the picked cells, a cell zone, layers along a patch | the tree's buttons, or the palette | `region.set` `shape` |
| Cut flat along the border, or whole cells | Alt + Shift + R, or the row's context menu | `region.set` `cut` |
| Remove | the context menu | `region.clear` |

While a region is enabled only its cells show, and slices, iso-surfaces,
arrows, streamlines and quality are computed only there; several enabled
regions show their union (up to 16). A cell belongs to a box, sphere or
slab when its bounding box touches the shape. Regions keep the work and
the picture small on a large mesh.

A blocked VTKHDF file (`murmel blocks make`,
[blocked-vtkhdf.md](blocked-vtkhdf.md)) that needs more memory than the
budget (Settings, Memory; half the computer's memory by default) opens as
an overview: its boundary with each cell field shown as the value of the
cell behind each face. On an overview a box, sphere or slab region reads
the cells of the blocks it touches (on a thread of its own, Esc cancels);
the post-processing of sections 4 to 10 then works on those cells. Zone,
patch and picked-cell regions are refused on an overview.

Before a region reads any cells, slices, iso-surfaces and arrows on an
overview draw on its preview grid: a coarse grid of the cell fields read
the first time one of them shows (on a thread of its own, Esc cancels).
A click on them picks nothing; derived and calculated fields and
streamlines need a region's cells. More in
[performance.md](performance.md).

## 14. Pictures and movies

| What | Window | Tool |
|---|---|---|
| A picture | "Export image…" (Ctrl + Shift + E) | `export.image` |
| A movie | "Export movie…" (Ctrl + Shift + M): turn around, sweep a clip plane, fly through saved views, or play the time series | `export.movie` |
| A chart | the chart panel's "Save…" | `charts.export` |
| A printed page or a PDF page | File ▸ Print… (Ctrl + P), File ▸ Export ▸ Export PDF page…; paper, margins and header in File ▸ Page setup… | `export.pdf`, `settings.set_page` |

Pictures and movies have the look of the window: lines, the legend and
text grow with the scale. The legend and the axis triad are each
optional; the background can be transparent. While a time series is open,
the "Time and step" box adds the time and step at the bottom right
(`time` in `export.image` and `export.movie`, on by default for series
movies). MP4 needs the `ffmpeg`
installed on the computer; GIF and PNG frames are written by MurmelCraft.
Details in [export.md](export.md).

## 15. Keep your work: sessions and defaults

A session file (`.murmel`, Ctrl + S) keeps the mesh, the camera, the
display, clip planes, slices, iso-surfaces, arrows, streamlines, spray
parcels, the time series and its step, regions, measurements and probes,
each field's style, and how each derived and calculated field was made
([sessions.md](sessions.md)).

Your defaults are kept in `settings.toml`: right-click a value or an item
in the window (arrows, slices, iso-surfaces, streamlines, clip planes,
regions, parcels, the field colouring, the render mode and projection,
the reference values of derived fields) and save it as your default.
New items start from it; items already open keep theirs. Settings lists
each default with a Forget button (`settings.set_default`,
`settings.reset_default`). Only an item's look, density, count and modes
can be defaults, never where it is or which field it shows.

## 16. The same from a script

`murmel run --script FILE` runs a list of tool calls in one headless
session and stops at the first failure; each call prints its result. This
script opens the channel result of the test fixtures, derives the
vorticity, draws a slice coloured by the speed with arrows on it, adds a
Q iso-surface and streamlines, places a probe and writes a picture
(`export.image` renders with wgpu, so it needs a GPU, or Mesa lavapipe on
a headless Linux machine). A test runs this script
(`post_processing_guide_script_runs` in `crates/murmel-tools`), so it
stays in step with the tools:

```json
[
  {"tool": "mesh.open", "input": {"path": "test-fixtures/vtkhdf/channel_fields.vtkhdf"}},
  {"tool": "field.derive", "input": {"quantity": "q-criterion"}},
  {"tool": "display.color_by", "input": {"by": "field", "field": "speed"}},
  {"tool": "field.style", "input": {"mode": "banded", "lines": true}},
  {"tool": "slice.set", "input": {"axis": "z"}},
  {"tool": "arrows.set", "input": {"density": 40, "in_plane": true}},
  {"tool": "iso.set", "input": {"field": "q_criterion", "value": 0.0}},
  {"tool": "stream.set", "input": {"seed": "line", "count": 20, "look": "tubes"}},
  {"tool": "probe.add", "input": {"at": {"cell": 3}}},
  {"tool": "view.standard", "input": {"view": "iso"}},
  {"tool": "export.image", "input": {"path": "channel.png", "scale": 2}}
]
```

The same calls run one at a time with `murmel call TOOL --input JSON`
(each in a session of its own, so `mesh` and `view` inputs set up a
one-shot screenshot), and from an agent over MCP: `murmel mcp --root DIR`
for a headless session, `murmel mcp --live` to drive the open window
([mcp.md](mcp.md)).

## 17. What the numbers are

MurmelCraft shows what the file has and says how anything else was made:

| Shown | How it is made |
|---|---|
| Flat colours, probe's cell value | the solver's cell values, as read |
| Smooth colours | node values rebuilt from the cells' least-squares gradients |
| Slices, iso-surfaces | the cells' tetrahedra: centroid (cell value), face centres (linear between the two cells), nodes |
| Arrows | the field at the slice's grid points, or the boundary face value (else the cell value) on the surface |
| Streamlines | fourth-order Runge-Kutta in the cells' tetrahedra |
| Derived and calculated fields with derivatives | least-squares cell gradients |
| Probes | cell value plus gradient times the way from the centroid |

Limits that change what is drawn are reported, never hidden: arrows
capped at 10 000 (`capped`), parcels thinned beyond 200 000, streamlines
that hit the length or step limit (counted in their Properties), seeds
outside the mesh, values that are not finite (`no_value`).
