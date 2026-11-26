# Performance of large meshes

MurmelCraft should stay responsive on meshes of millions of polyhedral
cells. Every change made for speed is measured before and after with
`murmel bench`, and the numbers are kept here.

## The benchmark

```sh
cargo build --release -p murmel-cli
# A synthetic polyhedral mesh of about 1 M cells in every format the viewer reads
target/release/murmel bench make --cells 1000000 --out bench-1m
# Time every stage of one of them (add --json for the full report)
target/release/murmel bench run bench-1m/vtu/foehncraft.vtu
target/release/murmel bench run bench-1m/polymesh-binary/constant/polyMesh
```

`bench make` writes the bitruncated cubic honeycomb (`murmel_mesh::synth`):
the Voronoi diagram of the body-centred cubic lattice, every cell a
truncated octahedron with 14 faces (6 squares, 8 hexagons) and every point
shared by four cells, which is what a KoflCraft Voronoi core looks like to
the viewer. The outside is left jagged, cut into six patches by the nearest
side of the box, and the cells into two zones. Formats: `vtu` (ascii, as
KoflCraft writes `foehncraft.vtu`), `vtu-binary` (base64), `polymesh` and
`polymesh-binary` (OpenFOAM, `faceCompactList`), and `blocked` (a blocked
VTKHDF file with two cell fields, written without the mesh in memory; see
"A 100 M cell test file").

`bench run` times, in this order:

| Stage | What it measures |
|---|---|
| `read` | the reader: file to mesh model, face topology included; its phases name the reader's steps (`read.stream`, `read.cells`, `read.topology`, ...), and `topology.*` splits linking the cell faces of a `.vtu` or VTKHDF file: `buckets` (faces sorted by their smallest point), `twins` (each face finds its twin), `number` (distinct faces, owner and neighbour), `flip` (which cell faces run backwards), `distinct` (the distinct faces copied out), `boundary` (the boundary faces found), `rest` |
| `digest` | BLAKE3 hash of the mesh files (the validity badge needs it) |
| `open` | `mesh.open` as the window does it: read, evidence, first surface (the files are hashed afterwards) |
| `open.digest` | the wait, after `open`, until the digests of the files are in place (the validity badge) |
| `surface` | the surface of the default display (boundary faces), with its phases: `surface.polygons` (which faces to draw), `surface.points` (corners and normals), `surface.buffers` (vertices, triangles, edges), `surface.extras` (slices, iso-surfaces, arrows) |
| `surface.crinkle`, `surface.flat` | the surface with one clip plane through the centre |
| `surface.hidden_zone` | one zone hidden, so the faces between zones lie open |
| `surface.cell_type` | colour by cell type |
| `surface.feature_edges` | render mode 4 (feature edges found on the surface) |
| `quality` | the cell quality metrics (colour by quality) |
| `tree.frame` | what the model tree and the legend ask of the mesh on each frame (counts, items), median of 5 |
| `clip.step` | one step of a clip drag as the window does it (the plane moves, the GPU cuts), median of 5 |
| `clip.exact` | from the last step until the exact crinkle cut is in place (built on its own thread once the plane rests) |
| `clip.layer.first`, `clip.layer`, `clip.layer.flat` | the thin layer a drag step draws (`Surface::clip_layer`): the first one, which also bins the cells; then a crinkle step (median of 5, phases `layer.cells`, `layer.crossed`, `layer.polygons`, then `surface.points` and `surface.buffers` of the layer); then a flat one |
| `clip.step.built` | one step that builds the exact cut before it returns (the command line, agents; every step before the preview) |
| `pick.9` | nine picks over the iso view (the hover tooltip, a click, snapping) |
| `render.first`, `render.next` | an offscreen frame with the upload, and a later one |
| `render.next.faces` | a later frame without the edges (the window while the view moves, when frames with edges are slow) |
| `field.make` | two cell fields made for the bench (a scalar `p` and a swirling vector `U`, functions of each cell's centroid), so every mesh has results |
| `field.color` | colouring by `p` the first time (`display.color_by`): the field's node values, with their phases `field.topology` (which nodes and cells meet, made once per mesh), `field.gradients` (each cell's least-squares gradient), `field.nodes` (the node values), then the surface |
| `field.switch` | colouring by `U` the first time (a vector: its three components and their magnitude) |
| `field.color.again` | colouring by a field whose values are made already: the surface alone |
| `field.style` | a change of the colour range (`field.style`), which draws again without building anything |
| `slice.set`, `slice.step` | a slice through the middle, then one step of a slice drag (`slice.set` with a new origin), with the phases `extract.slice` (the section's faces), `extract.outline` (the outline of each cell's section) |
| `iso.set`, `iso.step` | an iso-surface of `p`, then one step of its value (`extract.iso`) |
| `arrows.set`, `slice.step.arrows` | arrows of `U` on the slice, the first time (`extract.vector`: the vector's node values), then one step of the slice with them |

`--no-fields` leaves out the field stages. The bench reports the peak resident memory (Linux and macOS, from
`getrusage`; the `read` stage also reports its own peak, `peak_rss_mib`) and the size of the mesh model. `render.*` on a machine without a GPU runs on Mesa's software
rasteriser and says nothing about a real GPU; run it on the Mac for those.

## Baseline (2026-11-24, before any optimisation)

Linux container, 4 threads, release build, Mesa llvmpipe (no GPU), main at
3a0a88b. Honeycomb n = 80: 1,005,039 cells, 6,220,800 points, 7,149,040
faces, 227,534 boundary faces; the mesh model takes 655 MiB.

| Stage (ms) | vtu (ascii, 1.18 GB) | vtu-binary (1.44 GB) | polymesh (ascii, 0.78 GB) | polymesh-binary (0.39 GB) |
|---|---:|---:|---:|---:|
| read | 8,660 | 12,724 | 3,245 | 1,520 |
| digest | 2,724 | 1,838 | 1,035 | 492 |
| open | 12,782 | 16,692 | 4,682 | 2,169 |
| surface (1.21 M triangles) | 355 | 341 | 384 | 340 |
| surface.crinkle | 390 | 323 | 393 | 329 |
| surface.flat | 435 | 393 | 471 | 409 |
| surface.hidden_zone | 231 | 224 | 261 | 225 |
| surface.cell_type | 374 | 323 | 404 | 361 |
| surface.feature_edges | 629 | 596 | 698 | 600 |
| quality | 1,771 | 1,801 | 1,820 | 1,815 |
| pick.9 (one pick ≈ 1/9) | 67 | 66 | 94 | 105 |
| render.first (llvmpipe) | 448 | 742 | 471 | 710 |
| render.next (llvmpipe) | 209 | 423 | 224 | 227 |
| peak memory (MiB) | 4,006 | 4,993 | 2,396 | 2,266 |

What it shows:

- Reading is single threaded and the slowest stage; the ascii `.vtu`
  KoflCraft writes takes 8.7 s and peaks at 6 times the mesh model's size
  (the whole file as text, every array as 64-bit integers, then the model).
  The binary `.vtu` is slower than ascii: it copies the base64 text, decodes
  it and converts each value by matching its type name.
- `open` reads the mesh files a second time to hash them (`digest`).
- Building the surface again (every colour, visibility or clip change, and
  every step of a clip drag) takes a third of a second at 1 M cells.
- Picking walks every triangle: 7 to 12 ms per pick at 1.2 M triangles.
- The window opens a mesh on the thread that draws it, so it freezes for
  the whole `open`.

## Loading (2026-11-24)

What changed, in the readers (`murmel-mesh`) and the window:

- **The file is read once.** Its SHA-256 is taken from the bytes already
  in memory, on another thread while the text is parsed, instead of
  reading the file a second time (`open` no longer pays `digest`).
- **Numbers are parsed in parallel.** The text of an array is cut into one
  piece per thread at a separator, the tokens of each piece are counted,
  and each piece is parsed in place into its part of the result
  (`numbers.rs`); integers by hand, floating-point numbers by the standard
  library (Eisel and Lemire's method). Base64 is decoded in pieces of whole
  four-character groups, and the type of a binary array is decided once
  per array, not once per value. The work is split over scoped threads of
  the standard library (`par.rs`, no thread pool, no new dependency).
- **Arrays are read straight into the mesh model** (32-bit ids, no
  intermediate 64-bit vectors), and the text is dropped before the face
  topology is built, which lowers the peak.
- **The face topology** (which cell faces are the same face) groups the
  faces by their smallest point with a counting sort and compares only
  the faces of a group, instead of a hash map keyed by each face's sorted
  points (`boundary.rs`); OpenFOAM's per-cell faces are assembled in
  parallel from `owner` and `neighbour`.
- **The window reads a mesh of 32 MiB or more on a thread of its own**
  (`opening.rs`): it keeps drawing, the status bar (and the empty
  viewport) shows the phase and the seconds so far, and Cancel or Esc
  stops the reader at its next phase (`MESH019`, not shown as an error).
  The read mesh, and its surface built for the display of the time, opens
  through `mesh.open` (`session.load` for a session file) like any other,
  so undo, the agents and the tools see the same open as before. A mesh
  whose files changed since, or a display changed meanwhile, is read or
  built again.

Same machine and meshes as the baseline (n = 80, 1,005,039 cells), and
the honeycomb n = 127 (4,048,759 cells, 2.6 GB mesh model); `--no-render`.

| Stage (ms) | vtu | vtu-binary | polymesh | polymesh-binary | vtu, 4 M (4.99 GB) | polymesh-binary, 4 M (1.54 GB) |
|---|---:|---:|---:|---:|---:|---:|
| read (was) | 2,505 (8,660) | 3,635 (12,724) | 1,939 (3,245) | 1,155 (1,520) | 11,492 | 4,044 |
| open (was) | 3,347 (12,782) | 4,106 (16,692) | 2,294 (4,682) | 1,473 (2,169) | 11,393 | 5,041 |
| peak memory, MiB (was) | 2,952 (4,006) | 3,657 (4,993) | 2,344 (2,396) | 2,048 (2,266) | 11,762 | 7,980 |
| surface | 437 | 474 | 362 | 368 | 1,296 | 1,037 |
| quality | 1,794 | 1,853 | 1,882 | 2,018 | 7,029 | 7,503 |
| pick.9 | 106 | 126 | 119 | 74 | 274 | 284 |

The phases of `open` (`bench run --json` lists them; ms):

| Phase | vtu | polymesh-binary | vtu, 4 M | polymesh-binary, 4 M |
|---|---:|---:|---:|---:|
| file (read from disk) | 600 | 162 | 2,364 | 653 |
| xml / points | 320 | 195 | 773 | 607 |
| numbers / faces | 940 | 187 | 4,038 | 651 |
| cells | 279 | 365 | 945 | 1,380 |
| topology / owner and neighbour | 449 | 68 | 1,666 | 313 |
| surface | 491 | 465 | 955 | 1,329 |
| SHA-256 (beside the parsing) | 1,129 | in `file` | 4,251 | in `file` |

What it shows, towards the target of 10 M cells:

- Opening is now 2 to 4 times faster and the window stays usable while it
  runs; the ascii `.vtu` of 4 M cells (5 GB) opens in 11 s, where the old
  reader would have needed, going by its 1 M numbers, about 50 s and 16 GB
  (more than this machine has).
- **Memory is the limit now.** The mesh model stores every interior face
  twice (once per cell) and takes 650 MiB per million cells; at 10 M
  cells it would be 6.5 GB, and the peak of an open about three times
  that. Storing each face once is the next step for memory.
- The surface (1 to 1.3 s at 4 M), the quality (7 s at 4 M) and picking
  (30 ms per pick at 4 M) are single threaded or walk everything; they
  come next.

## Each face stored once (2026-11-24)

The mesh model kept every face of every cell, so each interior face was
stored twice. It now keeps each distinct face once, as its owner lists it
(`Mesh::distinct`), and per cell face only the distinct face and whether
the cell turns it the other way (`cell_face_id`, `cell_face_flip`);
`Mesh::cell(c)` gives the cell's faces in its own turn. OpenFOAM stores
faces this way already, so a polyMesh is no longer copied face by face
into cells.

| | 1 M: vtu | 1 M: polymesh | 1 M: polymesh-binary | 4 M: vtu | 4 M: polymesh-binary |
|---|---:|---:|---:|---:|---:|
| mesh model, MiB (was) | 476 (655) | 476 (655) | 476 (655) | 1,891 (2,615) | 1,891 (2,615) |
| peak memory, MiB (was) | 2,768 (2,952) | 1,829 (2,344) | 1,620 (2,048) | 11,050 (11,762) | 5,879 (7,980) |
| open, ms (was) | 3,167 (3,347) | 1,988 (2,294) | 1,347 (1,473) | 11,350 (11,393) | 4,433 (5,041) |

At 10 M cells the model takes about 4.7 GB instead of 6.5 GB. The ascii
`.vtu` still peaks at about six times its model: the whole text is in
memory while it is parsed (5 GB at 4 M cells, 12 GB at 10 M).

## Streaming the `.vtu` (2026-11-24)

The `.vtu` reader no longer holds the file's text. One thread reads the
file ahead in chunks of 32 MiB, a second takes its SHA-256, and the parser
takes each array as it streams past: the XML markup goes through the XML
parser, and the text of each array is parsed in parallel, chunk by chunk,
straight into the type its use needs (a number or base64 group cut by the
end of a chunk waits for the next). Arrays the reader does not use are
passed over. The points are grouped by three as soon as they are read, and
when the cells are polyhedra and polygons only (as KoflCraft writes them),
the faces' points are written over the face stream they come from instead
of being copied.

Same machine and meshes as above, `--no-render`; the reading stage.

| | 1 M: vtu (1.18 GB) | 4 M: vtu (4.99 GB) |
|---|---:|---:|
| peak memory while reading, MiB (was) | 1,056 (2,228) | 3,954 (9,154) |
| read, ms (was) | 2,226 (2,648) | 10,001 (11,630) |
| open, ms (was) | 2,692 (2,895) | 10,181 (11,518) |
| peak of the whole benchmark, MiB (was) | 1,712 (2,776) | 6,737 (11,049) |

The phases at 4 M cells: streaming 7,267 ms (the SHA-256 beside it,
7,145 ms), cells 683 ms (was 1,019, the faces are no longer copied),
topology 2,030 ms.

What it shows:

- Reading now peaks at about twice the mesh model (4.0 GB for a 1.9 GB
  model at 4 M cells), not five times. At 10 M cells that is about 10 GB
  (estimated from the 4 M numbers), well inside the iMac's 32 GB.
- **The SHA-256 sets the pace now.** The arrays parse as fast as the file
  streams in, and the stream ends when the hash does. This machine hashes
  with the CPU's SHA instructions (1.1 GB/s); the iMac's Core i9 (2019)
  has none, and the portable code hashes 0.38 GB/s here. On the iMac the
  hash of a 10 M cell ascii file (12 GB) would take longer than the
  parsing, roughly 15 to 30 s (estimate). Taking it after the mesh is
  open is the next step for opening speed.

## Hashing after the mesh opens (2026-11-24)

The maintainer's choice: the window opens a mesh as soon as it is read and
takes the SHA-256 of its files afterwards, on a thread of its own
(`Session::hash_after_open`). The validity badge says *Checking* until the
digest is in place, then turns green, amber or red as before; counts the
evidence contradicts turn it red at once. The command line and agents on
a headless session still hash while reading, so `mesh.open` returns the
digest there.

`open` is the time until the mesh shows; `open.digest` the time from
then until the badge is final. Same machine and meshes, `--no-render`.
The second pair of columns is a build with the portable SHA-256
(`sha2`'s `force-soft`), as on a CPU without SHA instructions such as
the iMac's Core i9.

| | 1 M: vtu | 4 M: vtu | 1 M: vtu, portable SHA | 4 M: vtu, portable SHA |
|---|---:|---:|---:|---:|
| open, ms (was) | 2,311 (2,692) | 9,596 (10,181) | 2,503 (4,021) | 9,350 (16,583) |
| open.digest, ms | 371 | 2,676 | 2,798 | 12,566 |

Without SHA instructions opening was bound by the hash (13 s of the
16.6 s at 4 M cells); now the mesh shows in 9.4 s and the badge follows
12.6 s later. With the instructions, the parsing sets the pace either
way. A 10 M cell ascii `.vtu` on the iMac should show in about the time
its parsing takes, with the badge some 15 to 30 s later (estimate).

## Quality and surface on all threads (2026-11-24)

- **Cell quality** (colour by quality) was one loop on one thread that
  collected the points of every face into a new list, about five times
  per face. Now the centroid and volume of each cell, and the values of
  each distinct face, are computed on all threads (`murmel_mesh::par`),
  and the face and cell geometry helpers walk a face's points without
  collecting them. Faces go in blocks of 2^20; each block gives its
  values to the cells in face order, so the result is the same for any
  number of threads (a test compares 1, 2, 3, 5 and 8 threads and blocks
  of 7 faces).
- **Surface.** Filling the buffers was three quarters of a surface build,
  and most of that was the set of edges already drawn, hashed with the
  standard library's SipHash. The edge sets now use FxHash (the
  multiply-rotate word hash of Firefox and rustc), and the buffers are
  sized before they are filled. The bench reports the build's phases.

polyMesh binary, same machine (4 threads). "was" is main at 204e509.

| Stage (ms) | 1 M (was) | 4 M (was) |
|---|---:|---:|
| quality | 528 (2,118) | 2,050 (8,001) |
| surface | 200 (443) | 562 (994) |
| surface.crinkle | 245 (374) | 808 (1,245) |
| surface.flat | 312 (458) | 1,081 (1,337) |
| surface.hidden_zone | 162 (259) | 508 (699) |
| surface.cell_type | 205 (433) | 691 (924) |
| surface.feature_edges | 361 (756) | 1,189 (1,790) |

At 4 M cells the default surface now spends about 110 ms choosing faces,
60 ms on corners and normals and 340 ms filling buffers, 200 ms of it on
the edge set. With 8 cores (the iMac) quality should take well under a
second at 4 M cells (estimate). Picking (30 ms a pick at 4 M cells) and
the clip modes' cutting are next.

## Nothing per frame walks the mesh; picking with boxes (2026-11-24)

- **Model tree and legend.** Each frame they asked the mesh for its
  counts (`Mesh::summary`: cells by kind, zone and patch, and the bounds)
  and for its items (`all_items`, which looked through every cell for
  each cell type). At 4 M cells that was 49 ms a frame, so the window
  could not draw faster than about 20 frames a second while the tree
  showed. The counts and bounds are now taken once and kept in the mesh;
  the items come from the counts.
- **Picking** walked every triangle of the surface (Möller and
  Trumbore) for each pick, and the measure tool snaps with a pick on
  every pointer move. The surface now keeps a box around each run of 128
  triangles (`Surface::tri_boxes`, filled on all threads at the end of a
  build, 12 ms at 4 M cells); the ray is tested against the boxes first
  (slab test, Kay and Kajiya 1986) and only against the triangles of the
  boxes it passes through nearer than the best hit so far. The hit is the
  same as before (a test compares both over a grid of rays, views and
  clip modes). The triangles come in face order, so a run covers a small
  patch of the surface; a hierarchy over the boxes (a BVH) would help
  further only if picks again show up in a frame.

polyMesh binary, same machine (4 threads). "was" is main at 6e49af0.

| | 1 M (was) | 4 M (was) |
|---|---:|---:|
| tree.frame, ms per frame | 0.0 (10.2) | 0.0 (49.3) |
| pick.9, ms for nine picks | 1.1 (72.3) | 2.5 (330.6) |
| surface.boxes, ms (new) | 7 | 12 |

## Clip drag: the GPU cuts first, the exact cut follows (2026-11-24)

The maintainer's choice for the clip drag: an instant preview, with the
exact cut following. Every step of a drag (the handle's arrow or arcs,
the toolbar slider, Alt and the arrow keys) used to build the whole
surface before the next frame: 1.1 s a step at 4 M cells.

- The window sets `Session::defer_clips`. A change of the display that
  only moves or turns clip planes leaves the surface as it is and starts
  a `ClipPreview`: the renderer draws the surface built without the
  moving planes and cuts it in the shader (`View::clip_planes`, a
  fragment on the cut-away side is discarded), so the cut follows the
  pointer at once. The cut faces show hollow (the inside of the far
  walls) until the exact cut is in place.
- The surface without the moving planes is built on a thread of its own
  when the drag starts; until it is there, the surface cut at the old
  place is cut again by the shader. Once the planes rest for 120 ms the
  exact cut is built on another thread and put in place when it is done,
  if the planes have not moved since (else the next one starts).
- The open mesh and its fields are shared (`Arc`), so the builds need no
  copy. Tools call `Session::settle` before they run (the clip tools
  excepted), so a screenshot, an export, a pick or an agent's call sees
  the exact cut; the command line and agents on a headless session never
  defer.

polyMesh binary, crinkle cut, same machine (4 threads), `murmel bench`:

| | 1 M | 4 M |
|---|---:|---:|
| clip.step, ms (was clip.step.built) | 0.0 (289) | 0.0 (1,104) |
| clip.exact, ms after the last step | 453 | 1,165 |

## Orbit: the edges hide only when a frame is slow (2026-11-24)

The maintainer's choice for moving the view on a large mesh: hide the
edges while the view moves, and only when a frame takes longer than 33
ms; nothing else is simplified, and nothing changes on a mesh that draws
fast enough.

- The window times the frames it draws with edges while the camera moves
  (an orbit, a pan, a zoom, a roll, a flight to a view): start to start,
  with the next frame asked for at once, so the gap is the frame's own
  cost (the GPU holds a frame back until it has drawn the ones before).
  Gaps after an idle pause are not timed.
- Once the median of the last three gaps is over 1/30 s (35 ms with the
  timer's jitter, so a frame that keeps 30 a second, two refreshes of a
  60 Hz screen, is within it), the edges are left out while the view
  moves and come back 150 ms after it rests. The verdict holds for the
  surface and the viewport's size it was timed on; a new surface (a
  clip, a colour, a field) or a new size is timed again.
- Edges drawn without the mesh's faces (wireframe, feature edges, the
  mesh as an outline) never hide: they are all there is to see.
  Pictures, movies and agents' screenshots always draw the edges.
- `murmel bench run` reports `render.next.faces`, the frame without the
  edges, next to `render.next`, so the edges' share of a frame can be
  read on the Mac.

polyMesh binary, iso view at 1280 x 720, same machine, Mesa llvmpipe (a
software rasteriser: the Radeon's numbers will differ, the share less
so), `murmel bench`:

| | 1 M | 4 M |
|---|---:|---:|
| render.next, ms (with edges) | 473 | 513 |
| render.next.faces, ms | 307 | 350 |

## Surface buffers on all threads (2026-11-24)

Filling the buffers was most of a surface build: every display change
(a colour, a hidden zone, a selection, the exact cut after a clip drag)
waited for it.

- The colours of each polygon are worked out on all threads first, and
  the place of each polygon's vertices and indices is counted before
  anything is written. Each thread then fills its run of polygons, in
  draw order, at the place the counts give: the buffers are the same,
  bit for bit, as before and for any number of threads.
- The edges are found on all threads too. Each edge goes to a shard by
  its smaller end; each shard walks its edges in draw order and keeps
  the first of each, so an edge keeps the colour of the first polygon
  that has it (the selection's, when selected), as before. The kept
  edges are then written run by run.
- The sRGB to linear conversion of the colours is a table of the 256
  values instead of a power per channel.
- `murmel bench` reports the phases of every surface stage.

polyMesh binary, same machine (4 threads), `--repeat 3`, old and new run
one after the other:

| Stage (ms) | 1 M (was) | 4 M (was) |
|---|---:|---:|
| surface | 106 (241) | 305 (586) |
| surface.buffers (phase) | 39 (172) | 100 (460) |
| surface.crinkle | 175 (268) | 701 (811) |
| surface.flat | 300 (362) | 1,195 (1,362) |
| surface.hidden_zone | 97 (145) | 355 (469) |
| surface.cell_type | 108 (193) | 341 (531) |
| surface.feature_edges | 232 (306) | 653 (848) |
| clip.exact | 352 (443) | 929 (1,277) |

What is left at 4 M cells: choosing and cutting the polygons is now
most of a clipped surface (500 ms of the crinkle cut, 980 ms of the
flat one, on one thread), and the feature edges spend 315 ms finding
creases. Those are next.

## Clipped surfaces on all threads (2026-11-24)

With the buffers on all threads, choosing and cutting the polygons was
most of a clipped surface, on one thread: the distance of every node
to the plane, the cells the clip keeps, the faces between cells, and
for a flat cut the cells the plane crosses.

- The distances to each clip plane and the cells it keeps are found on
  all threads (`Cutter::new`, `Display::shown_cells`, `kept_cells`).
- The faces between cells are walked on all threads, in face order,
  unless something cuts flat: the cut points of a flat cut are numbered
  as they are found, so that walk stays on one thread.
- A flat cut finds the cells the plane crosses on all threads first
  (`Sides::crosses`, a thin layer of the cells) and builds the sections
  of those only, instead of testing every cell on one thread. Polygons
  the plane keeps whole are no longer copied through the clipper.
- The buffers are the same, bit for bit, as before.

polyMesh binary, same machine (4 threads), `--repeat 3`, old (main at
e8bc81d) and new run one after the other; in brackets the polygon
phase:

| Stage (ms) | 1 M (was) | 4 M (was) |
|---|---:|---:|
| surface.crinkle | 108 (211) | 417 (733) |
| — polygons | 58 (134) | 223 (547) |
| surface.flat | 170 (353) | 659 (1,292) |
| — polygons | 126 (296) | 503 (1,147) |
| surface.hidden_zone | 75 (110) | 276 (330) |
| clip.step.built | 118 (226) | 447 (702) |
| clip.exact | 275 (366) | 859 (1,116) |

A clip step built at once (the command line, agents) now takes 0.45 s
at 4 M cells, and the exact cut after a drag follows in 0.86 s. Left
in a flat cut: the sections of the crossed cells and the faces they
clip, on one thread (cut points are named as found), about 0.3 s at 4 M.

## Feature edges on all threads (2026-11-24)

Finding the feature edges (render mode 4, and the faint outline of the
mesh while slices or iso-surfaces show) walked every polygon edge into
one map on one thread: 316 ms of a 650 ms surface at 4 M cells.

- The edges go to shards by their smaller end (as the drawn edges do),
  and each shard is walked on a thread of its own, in polygon order, so
  each later face is judged against the same first face as before. The
  shards stay as they are (`Features`) and the drawn edges look an edge
  up in its shard; no set is built from them.

Same machine (4 threads), `--repeat 3`, main at e8bc81d against this
branch; in brackets the points phase, where the feature edges are found:

| Stage (ms) | 1 M (was) | 4 M (was) |
|---|---:|---:|
| surface.feature_edges | 147 (237) | 442 (651) |
| — points phase | 38 (127) | 93 (316) |

## Parsing ascii arrays: room filled by the threads (2026-11-24)

Timing the parts of the `.vtu` stream at 1 M cells (4 threads) showed
where the time went: counting the numbers 360 ms, parsing them 455 ms,
and 254 ms growing the arrays on one thread (`Vec::resize` writes zeros
over the new part, so every page of the result is first touched there,
and each growth copies what was read before).

- **Room, not resizing** (`numbers::Filling`). An array is parsed into a
  buffer the allocator hands over zeroed: fresh pages from the system,
  which no thread writes before the parsing threads do, each its own
  part. The buffer is sized from the counts the file declares (points,
  cells and their data); the connectivity and face streams, which have no
  count, get room for what the rest of the file would hold at the density
  of their first megabyte, and too much room is given back at the end
  (pages never written cost no memory). A buffer that is still too small
  doubles, copied over on all threads.
- **Points straight into points.** The coordinates parse into `[x, y, z]`
  directly (the `.vtu` and the ascii polyMesh reader), no longer into a
  flat array copied into points afterwards (about 150 MB at 1 M cells).
- **The token count compares, it does not look up.** Counting tests each
  byte against the space and control bytes (and parentheses for
  OpenFOAM) instead of a table, so the compiler compares sixteen bytes at
  a time: 3× faster on one thread. A control byte that is not a separator
  is still an error wherever it sits (`stray_control_bytes_are_errors`).

Same machine (4 threads), `--no-render`, two runs each alternating, main
at f55f80f against this branch:

| | 1 M: vtu (1.18 GB) | 4 M: vtu (4.99 GB) |
|---|---:|---:|
| open, ms (was) | 1,779 (2,148) | 7,503 (9,255) |
| — streaming the arrays | 1,026 (1,348) | 4,353 (5,988) |
| read with the SHA-256 beside it, ms (was) | 1,870 (2,448) | 8,169 (10,076) |
| peak memory while reading, MiB (was) | 1,067 (1,059) | 3,945 (3,961) |

What it shows:

- Opening a 4 M cell ascii `.vtu` takes 1.8 s less (19 %), the
  streaming of its arrays 27 % less, with the same peak memory.
- With the arrays' growth gone, what is left of the stream is spread
  over the threads (parsing the numbers is most of it), so the iMac's
  16 threads gain more from it than this machine's 4.
- The hash beside the reading finishes sooner too: the parse leaves it
  more of the CPU.

## Face topology: the one-thread steps on all threads (2026-11-24)

Linking the faces of the cells (`boundary::link`: which cell faces are the
same face, and its owner and neighbour) took 2.2 s of a 7.5 s open at 4 M
cells. Matching the faces already ran on all threads; timing its steps at
1 M cells showed three that did not, together 200 of 470 ms on 4 threads
and the same on 1, so on the iMac's 16 threads they would be most of it:

- **Sorting the faces by their smallest point** (a counting sort). Each
  run of faces now deals its faces out to parts, consecutive ranges of
  points, in face order; each part then sorts its own faces into its
  groups. Every group lists its faces in face order as before.
- **Writing each face's twin** back in face order, one face at a time:
  the threads that match a group now write its twins themselves (each
  face is written once, through an atomic view of the array).
- **Numbering the distinct faces** in the order their first cell face
  comes. Each run of cells counts its first faces, numbers them after
  those of the runs before it, and then every cell face takes the number
  of its first twin. The neighbour is the smallest other cell that uses
  a face (the first, as cells come in order), kept with an atomic
  minimum, so the result does not depend on the threads
  (`buckets_match_the_hash_map` compares 1 and 3 threads with a hash map).

Same machine (4 threads), `--no-render`, two runs each alternating, main
at e878652 against this branch:

| | 1 M: vtu (1.18 GB) | 4 M: vtu (4.99 GB) |
|---|---:|---:|
| topology while opening, ms (was) | 363 (509) | 1,442 (2,155) |
| open, ms (was) | 1,794 (1,885) | 6,869 (7,545) |
| peak memory while reading, MiB (was) | 1,046 (1,062) | 4,009 (3,945) |

What it shows:

- The topology takes a third less at 4 M cells on 4 threads, and the
  same on one thread (860 ms at 1 M, as before), so it is no slower on
  a small machine. On the iMac's 16 threads all of it now divides.
- The sort deals the faces out into a second array of one number per
  cell face: 64 MiB more at the peak at 4 M cells (1.6 %).

## Flat clip cut on all threads (2026-11-24)

The flat cut was the last part of the exact cut on one thread: the cut
points are numbered as they are found, and those numbers decide where
each section's loop starts, so finding them in another order would draw
the same cut with other triangles. The numbering stays on one thread,
in the same order; the rest moves to all threads:

- **The faces.** The threads walk the faces as for every other surface
  and sort each polygon by what the flat planes do to it (`Sides::fate`):
  whole, dropped, or crossed. Only the crossed ones, a thin layer, go
  through the cutter afterwards, in face order, so their cut points are
  numbered as before.
- **The sections.** One thread names, cell by cell, where the plane
  crosses the edges of each cell one plane slices, in the order its
  section asks for them (`Cutter::cut_edges`); a cell more planes slice
  is cut there whole, as before. Then the sections of the other cells
  are built on all threads, reading the numbered points (`View` over a
  read-only `Store`).

The surfaces are the same bit for bit as before (all buffers hashed for
one, two and three flat planes, a crinkle and flat mix, and 1 and 3
threads), and `threads_give_the_same_surface` now also covers two and
three flat planes and a region cutting flat.

Same machine (4 threads), binary polyMesh, `--repeat 3` (1 M:
`--repeat 5`), main at 9b18ec9 against this branch:

| Stage (ms) | 1 M (was) | 4 M (was) |
|---|---:|---:|
| surface.flat | 156 (181) | 512 (612) |
| — polygons | 110 (134) | 380 (454) |

What is left of a flat cut at 4 M: numbering the cut points (about 50
ms, one thread), finding the cells shown and crossed (two walks over
every cell's points, about 190 ms on all threads, which one walk could
do), and the clip's distances, worked out twice (once for the cells
shown and once for the cutter).

## Remembering checked files (2026-11-24)

On the iMac, the SHA-256 of a 4 M cell ascii `.vtu` (5 GB) takes 17 s
after the mesh shows: its Core i9 has no SHA instructions, and SHA-256
cannot be split over threads. The user's choice for it: remember the
files already checked now, and hash with BLAKE3 in all three projects
later.

The window and the command line remember the digest of every mesh file
they hash in `checked-files.json` in the settings folder
(`murmel_engine::checked`). A file opened again with the same size and
modification time, and on Unix the same change time and file id
(device and inode), is not hashed again: its digest comes from the list
and is checked against the evidence as before. Writing to a file
changes its times, replacing it changes its id, and setting its
modification time back by hand changes its change time, which programs
cannot set. A file written while it was hashed is not remembered, nor
one modified less than 3 s before: written again within the same tick
of its file system's clock (2 s on FAT) at the same size, it would look
unchanged. The list keeps the 500 files checked last; it is a cache,
so a list that cannot be read or written means only that files are
hashed again. The MCP server's headless sessions and `murmel bench` do
not use it.

- **The window** (hash after the open): for a file opened again, the
  badge turns from *Checking* to its colour as soon as the mesh shows,
  instead of 17 s later at 4 M cells on the iMac, and saving a session
  no longer waits.
- **The command line** (`murmel call mesh.open`, hash while reading): a
  `.vtu` or `.vtkhdf` file is read without hashing, so the open is the
  parsing alone. The files of a polyMesh folder are still hashed while
  reading there; the window remembers them one by one.

Here (4 threads, SHA instructions, 1 GB/s), `murmel call mesh.open` of
the 4 M cell `.vtu` takes 8.2 s the first time and 7.2 s again. On the
iMac the hash sets the pace instead: its benchmark read the same mesh in
20.2 s while hashing and opened it in 5.3 s without, so opening it again
from the command line should take about 5 s instead of 20 s, and in the
window the badge should be final when the mesh shows instead of 17 s
later (estimates from the iMac's benchmark of 2026-11-24).

## Building the cells on all threads (2026-11-24)

The first numbers from the iMac (4 M cells, the ascii `.vtu` still in
the file cache, main at d91bb3a): the mesh showed after 5.3 s, of which
2.2 s went to parsing the text, 2.0 s to linking the faces and 0.74 s to
building the cells. Building the cells took as long there as on 4
threads here, because it still ran on one thread. It turns each VTK
cell into its faces (a polyhedron's from the `faces` stream, a standard
cell's from its face table) or into a boundary polygon.

It now runs on all threads (`vtu/cells.rs`), in two passes over one
range of cells per thread. Both passes walk the cells with the same
code:

1. Each range checks its cells and counts its faces, their points, its
   cells and its polygons. The counts give each range its place in the
   arrays. A range starts reading the `faces` stream where the last
   polyhedron before it ends.
2. Each range writes its cells at that place.

The first bad cell gives the error, as before, even when a later range
holds another.

The faces' points no longer move. One thread used to move them down
over the stream they came from, dropping each face's point count; a
first version of this change copied them into a new array on all
threads instead, but writing 1.2 GB of fresh memory at 4 M cells left
it at 0.55 s. A file of polyhedra and polygons only, as KoflCraft
writes, now leaves its faces in the stream: the reader notes where each
face's count is (`InStream`), and linking the faces reads them there
(it takes any `FaceList`). The stream is freed after the linking. Files
with standard cells (tetrahedra, hexahedra, prisms, pyramids) still
copy their faces out, as their faces are not in the stream.

The meshes are the same bit for bit: every array the reader gives was
hashed on the fixtures and on 1 M cells, at 1 and 4 threads, against
main. `threads_give_the_cells_of_one_walk` compares 1 to 7 threads on
all cell types mixed and on polyhedra and polygons only,
`threads_report_the_first_bad_cell` compares the errors, and
`polyhedra_leave_their_faces_in_the_stream` checks the faces left in
the stream.

Same machine, 4 M cells ascii `.vtu` (4.99 GB), `--no-render`, two runs
each alternating on 4 threads and one on 1 thread, main at d91bb3a
against this branch:

| | 4 threads | 1 thread |
|---|---:|---:|
| building the cells while opening, ms (was) | 306 (674) | 848 (689) |
| open, ms (was) | 6,553 (6,854) | 14,283 (14,647) |
| peak memory while reading, MiB (was) | 3,999 (4,011) | 3,904 (3,917) |

What it shows:

- Building the cells takes less than half as long on 4 threads, and
  should take about a fifth of the 0.74 s on the iMac's 16 (estimate).
- On one thread it takes a quarter longer: it walks the cells twice.
- Linking the faces reads them from the stream as fast as from their
  own array (1.5 to 1.7 s in both, within the noise of these runs), and
  the peak memory falls slightly: the stream holds each face's count
  besides its points, but the connectivity and the face offsets are now
  freed before the linking.

## The steps of linking the faces, and peak memory on macOS (2026-11-24)

`bench run` now splits `read.topology` into its steps (`topology.*`, see
the stages above) and reports the peak memory on macOS too: it asks
`getrusage` on Linux and macOS (the `nix` crate, MIT) instead of reading
Linux's `/proc/self/status`. On Linux both give the same peak (3,999.6
and 3,998.8 MiB for the read of the 4 M mesh below; 5,915 and 5,914 MiB
over the whole run).

The 4 M cell `.vtu` of polyhedra here (`bench make --cells 4000000`),
4 threads, two runs of `read` and `open` (ms):

| Step | read | open |
|---|---:|---:|
| buckets (faces sorted by their smallest point) | 660, 603 | 635, 589 |
| twins (each face finds its twin) | 376, 322 | 331, 368 |
| number (distinct faces, owner and neighbour) | 224, 184 | 243, 197 |
| flip (cell faces that run backwards) | 139, 142 | 141, 144 |
| distinct (the distinct faces copied out) | 448, 424 | 457, 370 |
| boundary | 31, 31 | 28, 31 |
| rest | 55, 55 | 64, 54 |
| all of `read.topology` | 1,995, 1,839 | 1,962, 1,805 |

Sorting the faces into buckets and copying the distinct faces out take
more than half of it. Both write large fresh arrays, which is where the
iMac (2.0 s for this step at 16 threads) is limited by memory rather
than by its cores; its own split will show whether that holds there.

## Sorting and copying the faces faster (2026-11-24)

The split above showed two steps doing more than they had to:

- **Sorting the faces into buckets** first found each face's smallest
  point (reading all 1.4 GB of the polyhedra's faces again) and joined
  the threads' pieces on one thread, then counted where the faces go in
  another pass. The `.vtu` reader now notes each face's smallest point
  while it reads the face (`Cells::least`), and linking takes it from
  there: one pass counts, in place. Readers that do not note it (VTKHDF,
  the synthetic meshes) have it found in that same pass.
- **Copying the distinct faces out** added up their sizes on one thread
  first. Each range of faces now adds up its own, and copies after the
  ranges before it.

The mesh is the same bit for bit (every array hashed, old against new,
for the fixtures, the 1 M bench mesh and a synthetic honeycomb, at 1, 3
and 4 threads).

The 4 M cell `.vtu` here, two runs of `read` and `open` each (ms; main in
brackets):

| 4 threads | read | open |
|---|---:|---:|
| buckets | 238, 245 (536, 546) | 223, 248 (536, 571) |
| distinct | 212, 209 (351, 345) | 244, 234 (357, 359) |
| all of `read.topology` | 1,211, 1,219 (1,638, 1,688) | 1,259, 1,232 (1,652, 1,696) |
| `read.cells` | 379, 379 (288, 280) | 396, 387 (286, 309) |

Linking takes 0.44 s less, and the cells 0.1 s more: writing the
smallest points (227 MB at 4 M cells) touches fresh memory. Together
0.35 s less at 4 threads; the peak memory is unchanged (3,999 MiB). On
one thread, linking went from 4.9 s to 3.4 s.

## BLAKE3 file checks (2026-11-24)

The second half of that choice: MurmelCraft, KoflCraft and FoehnCraft
hash files with BLAKE3. The user's card, "BLAKE3 only, still check old":
new files record only BLAKE3, named as such, and what was recorded with
SHA-256 before is still checked with SHA-256 (`murmel_mesh::hash`).

- The readers take the BLAKE3 hash beside the parsing; the window takes
  it after the mesh opens, the file read in pieces of 16 MiB on one
  thread and each piece hashed on every core (`update_rayon`).
- KoflCraft's `mesh-evidence.json` 1.1 names its hash in `file_hash`
  (`blake3`); 1.0 has none and holds SHA-256. A mesh is checked with the
  hash its evidence names, BLAKE3 without evidence (`OpenMesh::hash`).
- `checked-files.json` 1.1 keeps `blake3` and, once needed, `sha256` of a
  file side by side; a 1.0 list is still read.
- Session files 1.15 save `mesh.blake3` (`mesh.sha256` for a mesh with
  evidence 1.0); an older session's SHA-256 is compared as SHA-256,
  taken once and remembered.

A 2 GB file in the page cache, on the 4-core cloud machine (which has
SHA instructions, unlike the iMac): SHA-256 1.4 s, BLAKE3 on one thread
0.7 s, BLAKE3 on four threads 0.14 s. Without SHA instructions SHA-256
runs at about 280 MB/s (17 s for the iMac's 5 GB); BLAKE3 on its eight
cores should take the disk's time, about 1 to 3 s, to be measured there
with `murmel bench run` (stage `digest`).

## Blocked VTKHDF: read where it lies, on all threads (2026-11-24)

A blocked VTKHDF file (`murmel blocks make`,
[blocked-vtkhdf.md](blocked-vtkhdf.md)) of the 1 M cell mesh opened in
5.3 to 6.2 s with a peak of 3.4 GB, where the ascii `.vtu` of the same
mesh opens in 1.6 s with 1.0 GB; the 4 M cell file (6 GB) ran out of
memory in the 15 GB container here. Three things took the time: every
dataset was copied into a list of 64-bit integers (2.2 s), the points of
the blocks were joined by their global ids in a hash map (1.2 s), and
the cells were built on one thread (1.1 s).

Now:

- **The datasets are read where they lie.** `H5::values` gives a view of
  a dataset in the file's bytes (`hdf5::Values`) that converts each value
  as it is used, as `read_i64` and `read_f64` convert them; a chunked
  dataset is decoded once.
- **The points are joined in a list by id** when the ids are small
  numbers, as the blocked writers number them, on all threads: each id's
  first row is found by an atomic minimum, and the new points are
  numbered in row order. Other ids still go in a map.
- **The cells are built on all threads** (`vtkhdf/cells.rs`), in the two
  passes of the `.vtu` cells: each range counts, then writes at its
  place. Each partition is checked when the walk comes to it, and the
  first bad cell or partition in file order gives the error.
- **The fields are gathered on all threads**, and the file is freed
  before the faces are linked.

The meshes and fields are the same bit for bit: every array and field
was hashed for 15 VTKHDF files (the fixtures, KoflCraft's hand-offs,
the `.vtu` and polyMesh fixtures converted, 100 k cells in two blocks
with three fields, and the 1 M blocked file) at 1, 3 and 4 threads,
against main. `threads_give_the_cells_of_one_walk` and
`threads_report_the_first_bad_cell_or_partition` (in `vtkhdf/cells.rs`)
compare 1 to 7 threads, `points_join_alike_by_list_or_map` the two ways
of joining points, and `values_convert_as_the_copies_do` every integer
and float storage.

Same machine, 4 threads, `--no-render`, two runs each alternating, main
at d5cc7c3 against this branch:

| 1 M cells, blocked (1.47 GB) | main | now |
|---|---:|---:|
| open, ms | 6,162, 6,223 | 1,601, 1,860 |
| reading the file, ms | 644, 629 | 585, 716 |
| the datasets, points and cells, ms | 5,278, 5,383 (linking included) | 465, 494 (cells 330, 340) |
| linking the faces, ms | | 330, 400 |
| peak memory while reading, MiB | 3,423 | 2,035 |
| peak memory of the run, MiB | 3,930 | 2,560 |

The 4 M cell file (6.04 GB) now opens: in 7.0 s (reading the file
2.9 s, the datasets, points and cells 1.9 s of which the cells 1.3 s,
linking 1.4 s, the surface 0.56 s), with a peak of 8.3 GB while reading
and 10.2 GB over the run.

What it shows:

- The blocked file opens 3.5 times as fast, about as fast as the `.vtu`,
  and needs 40 % less memory.
- It still needs twice the memory of the `.vtu`, because the reader holds
  the whole file (`H5::open` reads it into memory; 6 GB of the 8.3 GB at
  4 M cells). A 10 M cell file would need about 20 GB, more than the
  iMac should spend. The next step reads the datasets from the file in
  pieces, block by block, which the 100 M cell overview needs too.

## HDF5 read from the file, not held (2026-11-24)

`H5::open` no longer reads the whole file. It reads the file's
structure: the superblock and the first 64 KiB, then it walks every
object from the root (headers, links, attributes, dataset descriptions,
chunk indices), notes what the walk needed and had not read, reads that
(a page of 64 KiB at least, or twice the run it continues) and walks
again, until nothing is missing. A dataset's data is read from the file
when it is asked for, with positional reads, in pieces of 8 MiB on all
threads. The datasets the mesh does not need (`GlobalCellIds`,
`OriginalCellIds`, fields until they are gathered) are never read, and
the points and their ids are freed once the blocks are joined. The
file's hash now reads the file on its own thread
(`FileHash::of_file`), as for the `.vtu`.

The meshes and fields are the same bit for bit (the 15 VTKHDF files of
the section above, at 1 and 4 threads), and
`opening_a_path_reads_what_the_bytes_read` reads every fixture both
ways, with pages of 64 KiB and of 512 bytes.

Same machine, 4 threads, `--no-render`, against main after #73:

| | #73 | now |
|---|---:|---:|
| 1 M blocked: open, ms | 1,639, 1,789 | 1,333, 1,338 |
| 1 M blocked: reading the file's structure, ms | 604, 722 (all of it) | 1, 2 |
| 1 M blocked: peak memory while reading, MiB | 2,036 | 1,775, 1,759 |
| 4 M blocked: open, ms | 7,873 | 5,097 |
| 4 M blocked: peak memory while reading, MiB | 8,267 | 7,170 |
| 4 M blocked: peak memory of the run, MiB | 10,216 | 9,108 |

The open takes a fifth to a third less and the peak an eighth less:
the datasets of the cells are still read whole before the cells are
built, and they are most of the file. Reading them block by block, a
few blocks at a time, is the next step.

## VTKHDF cells read in windows, unit by unit (2026-11-24)

The cells of a VTKHDF grid are built in units of 32,768 cells, a run of
units per thread, and each unit reads only its window of each dataset:
its rows of `Types`, ghosts and zones, its offsets and the one after,
the rows of `Connectivity` those offsets reach, and for polyhedra the
rows of `PolyhedronOffsets`, `PolyhedronToFaces`, `FaceOffsets` and
`FaceConnectivity` its cells use, found from the window read before
(`Window::of`). A contiguous dataset's rows are read from the file
(`H5::values_in`); a chunked one is still read whole, once. The reader
now holds the outputs and the units in flight, not the datasets.

Both passes read their windows: the counting pass reads all but
`Connectivity` and `FaceConnectivity` (the counts do not depend on the
point ids), and the pass that writes checks the ids. When the counting
pass meets a bad cell, a pass that reads the ids looks for a bad one
before it, so the error is still the first in file order.

The meshes and fields are the same bit for bit (the 15 VTKHDF files of
the sections above, at 1 and 4 threads).
`threads_give_the_cells_of_one_walk` reads in units of 1 to 1,000 cells
on 1 to 5 threads against one walk over the whole datasets.

Same machine, 4 threads, `--no-render`, against main after #74:

| | #74 | now |
|---|---:|---:|
| 1 M blocked: open, ms | 1,155, 1,287 | 1,074, 1,094 |
| 1 M blocked: building the cells, ms | 271 to 313 | 390 to 479 |
| 1 M blocked: peak memory while reading, MiB | 1,759, 1,774 | 1,116, 1,116 |
| 1 M blocked: peak memory of the run, MiB | 2,310, 2,326 | 1,895, 1,920 |
| 4 M blocked: open, ms | 4,764 | 4,201 |
| 4 M blocked: peak memory while reading, MiB | 7,154 | 3,983 |
| 4 M blocked: peak memory of the run, MiB | 9,103 | 5,937 |

The peak while reading falls by 37 % (1 M) and 44 % (4 M), the peak of
the run by a fifth to a third; the open is about as fast or a little
faster. Building the cells takes longer because it now includes reading
their datasets, which the open read before it, and each unit reads its
windows twice (from the page cache the second time). For a 10 M mesh
this saves about 8 GB at the reading peak.

## The overview of a mesh beyond the memory budget (2026-11-24)

A mesh opens whole up to the memory budget: half the computer's memory
unless `[memory] budget_gib` in settings.toml sets another (Settings →
Memory, or the tool `settings.set_memory`; 16 GiB when the computer's
memory cannot be found). A blocked VTKHDF file whose whole read needs
more opens as an **overview** (the maintainer's cards of 2026-11-24:
overview plus regions beyond the budget, 100 M cells on the iMac). What
a whole read needs is estimated from the file's structure alone
(`vtkhdf::size_of`): 1,500 bytes per cell plus 8 bytes per field
value, about the peak of the run measured above for the 1 M and 4 M
blocked files (1,475 to 1,540 bytes per cell). Any other file opens whole, as before.

The overview reads no volume cells. It reads:

- the patches, with their points joined by `GlobalPointIds` (each point
  once) and their own fields;
- the block index: `BlockOwnedCells`, `BlockBounds` and each
  `BlockRange_<field>`;
- each cell field at the rows of the cells behind the boundary faces
  (`OwnerCell`), read in runs of nearby rows, so it colours the
  boundary as the cell behind each face. Point fields are not shown
  (they are named in `fields_not_shown`).

A field's colour range on an overview spans the whole volume, from the
block ranges (`FieldSet::volume_range`), so the boundary is coloured on
the scale a whole read would give; range mode "shown" still takes the
values drawn.

`mesh.open` takes `mode`: `fit` (the default, as above), `whole` or
`overview`; a file without a block index refuses an overview with
BLOCK004. The window's model tree says when a mesh is an overview, with
its cells, blocks and the memory they need, and offers "Read the cells
anyway". The badge checks the file's cells against the evidence.

`read_overview` alone, same machine as above, 4 threads, three runs:

| | read whole | overview |
|---|---:|---:|
| 1 M blocked (227,534 boundary faces): time, ms | 1,074 to 1,094 (open) | 63 to 110 |
| 1 M blocked: peak memory, MiB | 1,116 | 41 to 51 |
| 4 M blocked (576,086 boundary faces): time, ms | 4,201 (open) | 243 to 264 |
| 4 M blocked: peak memory, MiB | 3,983 | 110 to 134 |

`murmel call mesh.open` on the 4 M file, which also takes the BLAKE3
digest of the 6 GB file and builds the surface: 1.1 s and 659 MiB as an
overview, 4.9 s and 3,978 MiB whole. The boundary of a mesh grows as
its cells to the power 2/3, so the overview of a 100 M mesh of this
shape (about 4.9 M boundary faces) should read in 2 to 3 s within about
1 GB; its whole read would need some 150 GB.

Not yet: the block boxes and the preview grid. Regions that read the
blocks they touch follow below.

## Reading some blocks of a blocked file (2026-11-24)

`vtkhdf::read_blocks(path, blocks, halo)` reads the cells of some blocks
of a blocked VTKHDF file, the base for regions of a mesh beyond the
memory budget (the maintainer's cards of 2026-11-24: overview plus
regions, one hidden halo layer). Each block's partition is read apart
from the others:

- its rows of `Points` and `GlobalPointIds`, joined to the points of
  the blocks before;
- its cells, by the windowed reader over its rows alone
  (`cells::read_rows`, `PointMap::From`);
- with the halo, the ghost cells it holds (one layer of point
  neighbours), each once: a ghost that a block read owns, or that a
  block before already gave, is left out (`CellData/GlobalCellIds`);
- the cell and point fields at its rows, read in runs;
- the patch faces whose `OwnerCell` is a cell read.

`Subset` gives each cell's file cell id and whether it is of the halo.
Read with every block and no halo, the mesh and its fields are the whole
read's, bit for bit: checked on the 14 VTKHDF files of the sections
above, the KoflCraft blocked fixtures and the 1 M blocked file (16
blocks). The whole read is unchanged, bit for bit at 1 and 4 threads.

Same machine, 4 M blocked file (62 blocks of 65,536 cells), file in the
page cache:

| blocks | cells (halo) | boundary faces | time |
|---|---:|---:|---:|
| 1, no halo | 65,536 (0) | 5,984 | 0.20 s |
| 1, halo | 76,670 (11,134) | 6,842 | 0.18 to 0.21 s |
| 4, halo | 289,310 (27,166) | 40,055 | 0.70 to 0.79 s |

From this machine's disk, not cached, one block took 2.1 to 5.3 s: the
reads are many and small, so a fast SSD (the iMac's) matters more than
it does for a whole read. The patches are still read whole to find
their faces of the blocks (5 M boundary faces on a 100 M mesh).

## Regions on an overview read their blocks (2026-11-24)

A box, sphere or slab region drawn on an overview reads the blocks whose
bounds it touches, with their halo, and shows on those cells what it
shows on the mesh read whole (the maintainer's card 1 of 2026-11-24,
overview plus regions; engine `blocks.rs`):

- A block's bounds hold the points of its owned cells, so a cell whose
  box touches the region lies in a block the region touches: the region
  takes the same cells, by their number in the file, and draws as many
  faces as on the whole mesh (checked on a blocked copy of the channel).
- The halo is read for the values at the border and never taken by a
  region (`RegionCells::of_on`), so it never draws.
- A region made smaller keeps the blocks read; one that needs a block
  not read reads exactly the blocks it needs; with none enabled the
  overview shows again, without reading.
- The memory a read needs is estimated before it starts: the whole
  read's share of the blocks' cells and a quarter more for the halo.
  Beyond the budget the region is refused (BLOCK006) and stays as it
  was.
- On an overview a region is a box, a sphere or a slab; a zone, a patch
  or picked cells need the cells read whole (REGION004).
- The window reads on a thread of its own (`Session::defer_blocks`): the
  overview shows meanwhile, the status bar says how many blocks and
  cells are read, and Cancel or Esc stops the read. The tools read
  before they return; `region.list` and `mesh.info` tell the blocks
  read, the cells and halo read, a read running and a read refused.
- Picked cells and faces are dropped when other blocks show, the cell of
  each probe is found again from its point, and Properties gives a
  cell's number in the file.

4 M blocked file (62 blocks of 65,536 cells), 4 threads, file in the
page cache; the time of the read alone and of the whole redraw (the
read, the region's cells and the surface):

| region | blocks | cells read (halo) | region cells | read | redraw |
|---|---:|---:|---:|---:|---:|
| box 0.1 at a corner | 1 | 72,215 (6,679) | 4,941 | 0.18 s | 0.25 s |
| sphere, radius 0.15, in the middle | 14 | 1,013,934 (96,430) | 63,783 | 2.47 s | 2.65 s |
| box 0.4 in the middle | 14 | 1,013,934 (96,430) | 273,259 | 2.66 s | 2.86 s |
| slab 0.02 across x | 33 | 2,268,537 (105,849) | 111,891 | 6.12 s | 7.03 s |

`region.clear` back to the overview: 0.25 to 0.3 s (the overview's
surface built again). The read is nine tenths of the time and reads the
blocks one after the other, about 0.18 s each; they are read on all
threads since (see the next section). A box whose sides lie on the block borders touches
the blocks on both sides (a box of 0.5 in the middle of this file
touches all 62 blocks, as the Hilbert blocks are cubes of a quarter).

## Reading a region's blocks on all threads (2026-11-24)

The blocks of a region were read one after the other. They are read on
all threads now (`murmel_mesh::vtkhdf::read_blocks`, `subset.rs`):

- The points and the point ids of each block are read on threads of
  their own, a block to a thread at a time.
- The points shared by blocks are joined by their id in shards: each id
  goes to a shard by a hash of it, each shard on a thread keeps the
  first place an id is seen, and the points are numbered in the order
  of their first place, so the numbering is the one read in a row. An id
  beyond `u32` joins the way it did.
- The cells of each block are read on threads of their own, the
  threads shared between the blocks, and put together in block order.
- Reading the whole file is untouched: the 14 files of the digest check
  read bit for bit as before, on one thread and on four.

4 M blocked file (62 blocks of 65,536 cells), file in the page cache:

| blocks | read before | read now, 4 threads | read now, 1 thread |
|---:|---:|---:|---:|
| 1 | 0.18–0.20 s | 0.16–0.18 s | |
| 14 | 2.6 s | 1.5 s | 3.1–3.3 s |
| 33 | 6.1 s | 3.1–3.3 s | 7.9–9.3 s |

Of the 33 blocks' 3.2 s, the join takes 0.69 s, the cells 1.05 s and the
face topology 0.73 s. With one thread (`MURMEL_THREADS=1`) the read
takes longer than it did before on four; the window and the tools use
all threads.

The regions of the last section, read and whole redraw, before and now
(4 threads):

| region | blocks | read now | redraw before | redraw now |
|---|---:|---:|---:|---:|
| box 0.1 at a corner | 1 | 0.14–0.16 s | 0.25 s | 0.18–0.22 s |
| sphere, radius 0.15, in the middle | 14 | 1.33 s | 2.65 s | 1.65 s |
| box 0.4 in the middle | 14 | 1.33–1.37 s | 2.86 s | 1.77–1.79 s |
| slab 0.02 across x | 33 | 3.21–3.24 s | 7.03 s | 3.74–3.81 s |

The iMac's 8 cores (16 threads) should take a larger share off. What is
left: each block read reads the patches whole (0.15 s on this file,
about 1.5 s on 100 M cells) where it could keep the overview's, and the
face topology of the blocks read.

## The preview grid of an overview (2026-11-24)

The overview of a mesh beyond the memory budget is the surface, the
block ranges and a preview grid (the maintainer's card 7 of 2026-11-24).
`vtkhdf::read_preview(path, longest)` reads the preview grid: the
volume's cell fields on a regular grid of `longest` cells along the
longest side of the blocks' box (`PREVIEW_CELLS` = 128, the proposal's
"about 128³"), the cells as near cubes as whole numbers allow, as a mesh
of hexahedra, so what draws on a mesh read whole can draw on it.

- Each grid cell shows the file cell its centre lies in: the cell whose
  centroid is nearest the centre among the cells whose box holds it. In
  a Voronoi mesh that is the cell holding the centre; elsewhere it is
  that cell or one next to it. A centre in no cell's box is outside the
  volume and has no grid cell.
- The blocks are read on all threads, a block to a thread at a time,
  and only their points and the point lists of their owned cells
  (`Points`, `Offsets`, `Connectivity`); each grid cell keeps the
  nearest centroid so far in one atomic word, the distance's bits above
  the cell's row, so the blocks may finish in any order. Each cell field
  and the zones are then read at the rows found, in runs of nearby rows.
- A read in the background stops between batches of 64 blocks.

Same machine, 4 threads, files in the page cache, two runs (the
honeycomb fills its box, so every grid cell is inside: the most a grid
of 128 can hold):

| file | grid | grid cells | time | peak memory |
|---|---|---:|---:|---:|
| 1 M blocked, no fields | 128³ | 2,097,152 | 0.78 to 0.92 s | 0.76 to 0.86 GB |
| 1 M blocked, 3 cell fields (5 values a cell) | 128³ | 2,097,152 | 1.09 to 1.14 s | 0.84 to 0.95 GB |
| 4 M blocked, no fields | 128³ | 2,097,152 | 1.13 to 1.34 s | 0.86 GB |
| 4 M blocked, no fields | 96³ | 884,736 | 0.78 s | 0.45 GB |
| 4 M blocked, no fields | 64³ | 262,144 | 0.57 s | 0.22 GB |

Most of the memory is the hexahedra of the grid: 162 bytes a grid cell
once their faces are linked (339 MB for 128³), about twice that while
they are linked. The sampling itself holds 8 bytes a grid cell and one
block's points and point lists a thread. A domain that is not a cube
fills fewer grid cells: a channel four times as long as it is wide has
128 × 32 × 32. The sampling reads the points and point lists of every
block (about a quarter of these files) and each cell field nearly
whole, so on 100 M cells it reads tens of GB; it is meant to run once,
in the background.

## Slices and iso-surfaces on an overview (2026-11-24)

An overview has no volume cells, so slices, iso-surfaces and arrows draw
on its preview grid (the engine's `preview.rs`), as on a mesh read
whole: the field's colours, the clip planes and hidden zones apply, and
the colour range is the volume's, as on the boundary.

- The grid is read the first time a slice, an iso-surface or the arrows
  show on the overview, not when it opens: on 100 M cells the sampling
  reads tens of GB. The window reads it on a thread of its own (the
  status bar says so, with Cancel or Esc), the tools before they return.
- It stays with the overview's fields: while a region's blocks show,
  slices draw on their cells, and the grid comes back with the overview,
  not read again. A stopped read starts again once one more slice,
  iso-surface or the arrows show; opening the mesh again forgets it.
- The triangles name no cell, so a click on them picks nothing of the
  overview's mesh. Fields made on the overview (derived, calculated)
  do not draw on the grid; they need a region's cells. Streamlines are
  traced through the blocks instead (see "Streamlines on an overview").
- `mesh.info` tells the grid under `overview.preview`.

1 M blocked file with three cell fields, 4 threads, file in the page
cache; the grid is 128³ (the honeycomb fills its box: the most a grid
holds), against the same file read whole:

| step | overview, on the grid | read whole |
|---|---:|---:|
| first slice, coloured by `p` | 3.2 to 3.3 s (the grid 1.1 s, the field's node values on it 1.6 s, the slice 0.45 s) | 0.8 s (the node values were made with the colouring, 2.0 s) |
| slice moved | 0.55 to 0.69 s | 0.86 to 0.93 s |
| iso-surface of `p` | 1.7 s (1.07 M triangles) | 2.4 s (1.31 M triangles) |

The grid holds at most 2 M cells whatever the size of the file, so on
100 M cells moving a slice costs what it costs here; only reading the
grid grows with the file. The grid is kept in the cache folder, so a
second open does not sample again (next section).

## The preview grid kept (2026-11-24)

Sampling the preview grid reads every point and point list of the
volume: about 1.1 s for the 1 M and 4 M files from the page cache, and
it grows with the file (on 100 M cells, tens of GB read). The grid it
gives holds at most 2 M cells whatever the file, so it is kept
(the engine's `preview_cache.rs`) in `preview/` of the cache folder
(`MURMEL_CACHE_DIR`, else `~/Library/Caches/MurmelCraft` on macOS,
`%LOCALAPPDATA%\MurmelCraft\cache` on Windows, `~/.cache/murmelcraft`
elsewhere) and read from there while the file is unchanged (its size,
times and file id, as for the files checked).

| file, 128³ grid, 4 threads | sampled | kept grid read | kept file |
|---|---:|---:|---:|
| 1 M cells, fields `p`, `U`, `T` | 1.07 s | 0.63 to 0.67 s | 117 MB |
| 4 M cells, no fields | 1.09 s | 0.53 to 0.59 s | 34 MB |

- Reading the kept grid is mostly building its hexahedra and their
  links again (0.50 to 0.59 s of it); the file is 16 bytes a grid cell
  and 8 a field component.
- The 16 grids used last stay, and no more than 2 GiB in all.
- A file modified less than 3 s before it was sampled is not kept, and
  a kept grid whose length is not what its header declares, or whose
  voxels leave the grid, is sampled again.
- The command line and the window keep grids; the MCP server, confined
  to its folders, does not.
- On 100 M cells the saving is the whole sampling: the kept grid reads in
  the same half second.

## Fields on a large mesh: where the time goes (2026-11-24)

The field stages of `murmel bench` (above) time colouring by a field,
slices, iso-surfaces and arrows as the tools draw them. Before any work
on them, 1 M honeycomb (1,005,039 cells), polyMesh binary, 4 threads,
`--no-render`:

| Stage (ms) | total | where |
|---|---:|---|
| field.color (`p`) | 2,256 | topology 1,617 (one thread), gradients 227, node values 112 |
| field.switch (`U`) | 1,892 | gradients 1,187 (each component and the magnitude on its own), node values 320 |
| field.color.again | 155 | the mesh surface |
| field.style | 0 | |
| slice.set / slice.step | 1,008 / 1,059 | section 586 to 653 (one thread, every cell tested), outlines 218 to 226 (one thread), the mesh surface built again 200 |
| iso.set / iso.step | 2,876 / 2,909 | iso-surface 1,645 to 1,802 (one thread), the slice built again with it |
| arrows.set | 4,972 | the vector's node values 3,699 (each of x, y, z and the magnitude made all three components again) |
| slice.step.arrows | 1,270 | section 929, outlines 314 |

At 10 M cells each of these is about ten times longer: seconds for one
step of a slice drag. What follows takes them down one at a time.

## Slices and iso-surfaces on all threads, near the cells they cross (2026-11-24)

A slice or an iso-surface tested every cell shown, on one thread, before
cutting the few it crosses; a slice's outlines did it again with the
distance of every node to the plane.

- **The cells in bins** (`bins.rs`): each cell goes to the bin of a
  regular grid holding the mean of its nodes, about 8 cells to a bin,
  made once per mesh (46 ms at 1 M cells). A bin keeps the sphere about
  its centre that holds every node of its cells. A plane farther from a
  bin's centre than that crosses none of its cells, so a slice tests
  only the cells of the bins it passes (a few per cent of the mesh). For
  an iso-surface each bin keeps, per field, the smallest and largest
  value the test of its cells can meet (made the first time an
  iso-surface of that field is drawn), so a new iso value tests only the
  bins whose span holds it.
- **The crossed cells are cut on all threads**, in runs of cells; each
  run names the points it makes by the segment of the tetrahedron they
  lie on. The runs are joined in cell order: a point two runs made is
  the first run's, and each triangle's side and normal are found from
  the points kept, as one thread did. A point on a segment from a
  cell's centroid lies in that cell alone and is not looked up.
- The outlines of the cells' sections are found on all threads, only
  for the cells near the plane, with each node's distance worked out
  where it is needed.
- What is drawn is the same, bit for bit, as before, for any number of
  threads (a test compares one thread testing every cell with five
  threads and the bins).

1 M honeycomb, polyMesh binary, 4 threads, `--no-render`; in brackets
before:

| Stage (ms) | now (was) | inside |
|---|---:|---|
| slice.set | 340 (1,008) | bins made 46, section 135, outlines 52 |
| slice.step | 416 (1,059) | cells tested 6 (650 with the section), section 145, outlines 51 |
| iso.set | 1,330 (2,876) | iso-surface 912 (cut 289, join 444) |
| iso.step | 972 (2,909) | iso-surface 530 (cut 231, join 231) |
| slice.step.arrows | 505 (1,270) | section 220, outlines 47 |

What is left in a slice step is the mesh's own surface, built again
with it (about 220 ms), and joining the iso-surface's runs on one
thread. Those come next, with the field's node values (`field.color`,
`arrows.set`).

## A field's node values: one pass, only what is needed (2026-11-24)

Colouring by a field the first time makes its node values from the
cells' least-squares gradients. That cost more than it needed:

- **The topology** (each cell's nodes, the cells of each node, the
  centroids) was made on one thread: now each cell's nodes and the
  centroids are made on all threads (1,617 to 706 ms at 1 M cells).
- **A vector made every component for each one asked**: colouring by
  `U.x` made the gradients and node values of x, y and z, and kept x.
  Now a component is made alone, and the magnitude makes the three
  components' and its own gradient in one pass over the cells around
  each cell (the stencil found once, each input fitted as before).
- **The arrows** asked for x, y, z and the magnitude, each making all
  three components again (twelve gradient passes); now three, and the
  magnitude's largest value is read from the cells without its node
  values (`FieldSet::range`).
- The node values of a vector's magnitude are put together on all
  threads, without a list per node.

The values are the same, bit for bit (a test compares a component made
alone with it made with all three).

1 M honeycomb, polyMesh binary, 4 threads, `--no-render`; in brackets
before this and the section above:

| Stage (ms) | now (was) | inside |
|---|---:|---|
| field.color | 1,157 (2,256) | topology 706, gradients 180, node values 110 |
| field.switch | 882 (1,892) | gradients 322, node values 356 |
| arrows.set | 1,420 (4,972) | the vector's node values 900 |

Left: the topology's centroids and its node to cell lists (about 470
ms of the 706), made once per mesh.

## Probes on an overview (2026-11-24)

An overview shows no volume cells, so a probe there (or outside the
blocks a region read) reads the block of the file that holds its point
(the engine's `probe_blocks.rs`, card 8: a block cache for probes). The
blocks whose boxes hold the point are read with their halo, the deepest
inside first, until one of their owned cells holds it. On the honeycombs
a point lies in one to four boxes (median two, mean 1.7 to 1.8, for 120
cells sampled in 16 and 62 blocks), so a probe reads one or two blocks.

| eight probes at random points, 4 threads | first read of its block | its block kept |
|---|---:|---:|
| 1 M cells, 16 blocks of 65,536 | 0.21 to 0.37 s | 0.01 to 0.04 s |
| 4 M cells, 62 blocks of 65,536 | 0.26 to 0.48 s | 0.02 s |

- The last four blocks read stay (fewer when they would take more than
  a quarter of the memory budget), and each probe's finding stays with
  its point, so drawing again reads nothing; moving a probe, or making a
  field, finds it again.
- The values are the whole mesh's: the halo holds every cell sharing a
  node with an owned cell, so the cell's value and gradient read the
  same cells. Derived and calculated fields are made again on the block;
  next to the halo their gradient can differ a little, as on the blocks
  a region reads.
- A probe in a halo cell of a region's blocks now reads the block owning
  it rather than the halo cell, whose neighbours are partly missing.
- In the window each probe's blocks are read on a thread of their own;
  its label says "reading its block" meanwhile.
- A block is 65,536 cells whatever the file, so on 100 M cells a probe
  reads in the same quarter to half second.

## Slices, iso-surfaces and arrows built again alone (2026-11-24)

Moving a slice built the whole surface again: the mesh's own faces,
their field colours and feature edges first, then the slice. On a
mesh of 1 M cells that was about 220 ms of each slice step, for faces
that had not changed.

Now the surface remembers where its extras start (`Surface.extras_at`:
the slices, iso-surfaces, arrows, streamlines, parcels and block
boxes, appended after the mesh's faces). When only extras changed
(`surface::extras_only`: the same mesh items, clip, selection of mesh
items and look), `Session::redraw` cuts the surface back to that point
and appends the extras again (`Surface::rebuild_extras`); the pick
boxes are made again only for the runs from there on. The renderer
keeps two sets of buffers, the mesh's and the extras', and uploads the
mesh's only when the surface was built anew (`Surface.key`), so a
slice step also uploads only the slice.

The surface is the same, bit for bit, as when built whole (a test
steps through colour, slices, iso-surfaces, arrows, streamlines, a
time series, selection and clips, comparing each with a whole build,
and checks which steps keep the mesh's buffers).

1 M honeycomb, polyMesh binary, 4 threads, `--no-render`; in brackets
before this:

| Stage (ms) | now (was) |
|---|---:|
| slice.step | 230 (421) |
| iso.step | 813 (1,023) |
| slice.step.arrows | 337 (493) |

The slice itself is now most of a step (extract.slice, about 170 ms).

## A slice dragged: drawn from the field's grid at once (2026-11-25)

Building a slice through a large mesh takes a while (about 0.25 s at
1 M cells, more at 10 M), too long for each step of a drag. So in the
window a slice that only moves is drawn at once from a regular grid of
the field on the GPU, and its exact faces follow on a thread of their
own once it rests for 120 ms (the maintainer's card of 2026-11-24:
"grid preview, exact slice follows"; texture-based slicing, Cabral,
Cam and Foran 1994; Engel et al., "Real-Time Volume Graphics", 2006).

- **The grid** (`slice_preview.rs`): about 2 M cells over the mesh's
  box (16 MB on the GPU, an `Rg32Float` 3D texture). Each grid cell
  shows the mesh cell whose centroid is nearest its centre among the
  cells whose box holds that centre (made once per mesh, on all
  threads, an atomic minimum per grid cell, so the same on any number
  of threads); grid cells outside every cell's box show nothing. It
  holds the cell's value (flat colours) and the value smooth colours
  show at the grid cell's centre (smooth and banded: linear on the
  cell's tetrahedra, as the exact slice cuts them), made once per field
  shown, on a thread of its own (below).
- **The drag** (`Session::redraw` with `defer_clips`): a change that
  only moves slices keeps the surface and starts a `SlicePreview`.
  `Session::slice_drawn` gives the renderer the planes inside the
  mesh's box, the grid, and which of the surface's triangles and
  outline vertices to leave out (the moving slices where they were,
  `Surface.slice_parts`, and the arrows); the renderer draws the
  extras in the ranges left and the planes with `fs_slice`, which reads
  the grid cell (flat) or mixes the eight nearest (smooth), cuts them
  with the clip planes, and colours them as the faces are.
- **The exact slice** is built on a thread from the surface's header
  (`Surface::header`, `Surface::extras_part`) and put in place with
  `Surface::splice`, the mesh's faces kept; any other change, or a
  tool (`crate::call` settles), ends the preview with the exact slice.

1 M honeycomb, polyMesh binary, 4 threads, `--no-render`:

| Stage (ms) | now |
|---|---:|
| slice.preview (first: the grid's cells 49, its values 62; now built on a thread, below) | 111 |
| slice.preview.step | 0.0 |
| slice.step (the exact slice, as the tools build it) | 230 to 275 |

Near a curved or concave boundary a grid cell inside a cell's box but
outside the mesh may show that cell until the exact slice is in place.

## An iso value changed: ray cast from the field's grid (2026-11-25)

A new iso value takes about a second at 1 M cells (extract.iso about
0.9 s), more at 10 M. So in the window an iso-surface whose value
changes, or one just added, is drawn at once by casting rays through a
grid of its field on the GPU, and its exact triangles follow on a
thread once the value rests for 120 ms (the maintainer's iso card of
2026-11-25, option 1; GPU ray casting, Krüger and Westermann 2003;
Engel et al., "Real-Time Volume Graphics", 2006).

- **The grid** (`IsoGrid`): the same 2 M grid cells as the slice's,
  each holding the value smooth colours show at its centre (linear on
  the mesh cell's tetrahedra, as the exact iso-surface cuts them; a
  grid cell just outside its cell takes the tetrahedron it is nearest,
  its weights clamped, so no value overshoots), and whether it is
  inside the mesh. Made once per field.
- **The texels** (`IsoTexels`, an `Rg16Float` 3D texture, 8 MB): per
  value, each grid cell's `(v - value) / span` as a half float, so the
  precision is finest at the surface (a half float of the plain value
  speckled the surface), and whether it is inside. 4 to 5 ms per value
  at 1 M cells.
- **The ray cast** (`fs_iso`): the box's back faces are drawn; each
  fragment walks from the eye (or the box's near face) towards them in
  half a grid cell's steps through the trilinear texels, finds the sign
  change, refines it with two secant steps, shades it with the
  gradient (central differences over 1.5 grid cells), cuts it with the
  clip planes, colours it solid or by the field shown (the slice grid),
  and writes its depth, so it sits right among the faces.
- **The drag** (`Session::redraw` with `defer_clips`): a change of one
  iso value (or one new iso-surface) keeps the surface and starts the
  preview; `Session::iso_drawn` gives the renderer the grid, the
  texels, the colour and the iso-surface's old triangles to leave out
  (`Surface.iso_parts`). Two values at once build at once.
- **Grids on threads** (both previews): a grid is built on a thread of
  its own (`GridSlot`), started as soon as a slice or an iso-surface
  exists in the window, so the first drag seldom waits. A change that
  comes before its grid is done leaves the old faces as they were
  until the grid comes in (or the exact faces, whichever is first).

1 M honeycomb, polyMesh binary, 4 threads, `--no-render` (the bench
builds the grids on its own thread to time them):

| Stage (ms) | now |
|---|---:|
| iso.preview (first: the grid, on a thread in the window) | 1,160 |
| iso.preview.step (the texels for the value) | 4.5 |
| iso.step (the exact surface, as the tools build it) | 1,100 |
| slice.preview (first: the grid with tetrahedra, on a thread in the window) | 990 |

The grid's values cost about as much as one exact iso-surface; they are
paid once per field, off the window's thread. Where the mesh is finer
than the grid (2 M cells over its box) the preview is coarser than the
exact surface, which replaces it 120 ms after the value rests.

## Streamlines on an overview (2026-11-25)

An overview shows no volume cells, so its streamlines are traced through
the blocks of the file (the engine's `stream_blocks.rs`, card 8: a block
cache for streamlines). A seed is looked for in the blocks whose boxes
hold it, as a probe is; a line reads a block, with its halo, the first
time it reaches it. When a walk enters a halo cell it goes on from the
same cell in the block that owns it, so the field is always read in an
owned cell, whose neighbours the halo holds. The tracer itself is the
one the whole mesh uses, over "parts" (the whole mesh is one part, each
block another).

1 M cells with three cell fields, 16 blocks of 65,536, 4 threads,
`murmel run` (the envelope's seconds), in brackets the same file read
whole:

| stream.set, field U | blocks | first trace | blocks kept |
|---|---:|---:|---:|
| line, 50 seeds both ways | 11 | 2.8 s (2.5 s) | 0.10 s (0.09 s) |
| sphere, 100 seeds | 12 | 3.6 s | 0.20 s (0.20 s) |
| plane, 81 seeds, after the line and sphere | 15 | 0.85 s (0.17 s) | |

- A block takes 0.25 to 0.3 s to read and make its values; once read it
  traces as fast as the whole mesh. On the mesh read whole the first
  trace pays for the whole mesh's node values instead.
- The lines are the whole mesh's where the field is: with a velocity
  linear in space (exact gradients in every cell) the points and colour
  values agree to rounding, across blocks of 100 cells. Otherwise node
  values on a block's border take the halo cells' gradients, so lines
  there differ a little (here 8 points of 46,500 on the sphere set; a
  magnitude's node values in the sixth digit).
- A trace reads at most as many blocks as a quarter of the memory
  budget holds (on the iMac, 16 GB, about 33 blocks); a line that needs
  another ends there (`ended.unread`, BLOCK006), and the set is traced
  again once the budget changes. The blocks gone through stay for the
  next trace, so moving or changing a set reads only new blocks.
- On 100 M cells (about 1,500 blocks) a set of lines sweeping across the
  whole volume can reach more blocks than that; dropping blocks during a
  trace and reading them again as lines come back is the next step if
  it bites.
- In the window the lines are traced on a thread of their own; the
  status bar counts the blocks read, with Cancel or Esc (the lines then
  end at the blocks read so far). Properties says how many blocks the
  lines read; a set being traced says so.

## Exact slices and iso-surfaces on an overview (2026-11-25)

An overview's slices and iso-surfaces showed on its preview grid only
(about 128 cells along the longest side), however close one looked.
Now the grid shows at once and their exact faces are then cut through
the blocks of the file (the engine's `cut_blocks.rs`, card 8: stream
slices and iso-surfaces), taking the grid's place.

- **Which blocks.** A slice cuts the blocks whose box one of its planes
  crosses. An iso-surface cuts the blocks whose values, from the
  smallest to the largest of their own `BlockRange` and those of the
  blocks whose boxes touch theirs, hold its value (span space, Livnat,
  Shen and Johnson 1996): a node's value lies between the values of the
  cells around it, owned by the block or by one beside it. A block's own
  range is not enough: a large cell whose nodes reach small cells beside
  it carries values its block does not hold (the channel fixture in
  blocks of 20 cells has one). A field made on the overview, or one
  without ranges, cuts every block; a block every clip plane takes away
  is not read.
- **Each block** is read with its halo and cut as the mesh read whole
  is (`extract::append`), owned cells shown and the halo hidden, so every
  face is cut once. With a velocity linear in space the triangles are the
  whole mesh's to rounding (corners, values and colours, coloured by the
  field or by the mesh's colour, on blocks of 100 cells); otherwise node
  values next to a block's border take the halo cells' gradients, as for
  regions.
- **Kept.** The faces stay until the item, the field shown, the colours,
  the clip planes or the hidden zones change. Selecting an item colours
  its faces again without cutting (the selection colour mixed in as the
  whole mesh does it). The last blocks gone through stay, as many as an
  eighth of the memory budget holds, so a slice moved a little cuts the
  same blocks again without reading them. The faces may take a quarter
  of the budget (BLOCK006 beyond); the item then stays on the grid.
- **In the window** the cut starts once the items have rested 0.3 s, on a
  thread of its own; the status bar counts the blocks with Cancel or Esc
  (the item then stays on the grid until it or the budget changes).
  Properties tells how many blocks a cut went through, or that it is
  being cut. The arrows stay on the grid. The tools cut before they
  return.

1 M cells with three cell fields, 16 blocks of 65,536, 4 threads,
`murmel run` (the envelope's seconds; the preview grid in the cache
folder), in brackets the same calls on the file read whole:

| call | blocks | seconds |
|---|---:|---:|
| first slice (x, middle) | 7 | 2.8 to 4.0 (0.38) |
| the slice moved by 0.1, its blocks kept | 7 | 0.24 (0.23) |
| the slice moved by 0.4 | 7 | 1.2 to 1.4 |
| a second slice (z) | 13 | 1.6 to 1.8 (0.39) |
| the second slice as a series of 5 | 16 | 2.5 to 2.6 (1.2) |
| iso-surface of p (middle) | 16 | 2.7 (2.0) |
| iso-surface beyond every range | 0 | 0.8 (the grid alone) |
| selecting the slice | 0 | 0.3 (1.5) |

- A block takes 0.3 to 0.4 s to read, make its values and cut; once
  kept, a cut costs what it costs on the mesh read whole.
- On 100 M cells (about 1,500 blocks) a plane crosses a few hundred
  blocks, so the exact slice follows the grid's after a minute or so on
  the iMac; reading several blocks at once, or keeping an index of the
  cells near each plane, is the next step if that is too slow.

## A large mesh that is not blocked (2026-11-25)

A `.vtu`, an OpenFOAM `polyMesh` or an unblocked VTKHDF file opened
whole whatever the budget, so a mesh beyond the computer's memory could
not open at all. Now its size is estimated before anything is read (the
engine's `convert.rs`, `murmel_mesh::estimate`: the cell count in the
head of the files times the 1,500 bytes a cell a whole read peaks at),
and beyond the budget the window asks to convert it once to a blocked
copy beside it (card 4), which opens as an overview.

- The estimate reads at most the first MiB of a `.vtu` or of `owner`
  (decompressed for `owner.gz`), or the VTKHDF structure: well under a
  millisecond on the fixtures. A `.vtu`'s boundary faces stored as cells
  count as cells, so it errs high.
- Converting reads the mesh whole once, as `murmel blocks make` does;
  1 M cells with three fields took 24 s here (most of it the read). The
  copy is about 1,500 bytes a cell on disk for these polyhedra. When the
  read needs more than the computer's memory, the question says so: a
  blocked file written by KoflCraft or FoehnCraft avoids the whole read.
- The copy is written to a `.part` file and renamed when whole, so a
  conversion stopped (Cancel, Esc, checked between blocks) or failed
  leaves nothing behind.

## A 100 M cell test file (2026-11-25)

Card 2 asks for 100 M cells to open on the iMac, but no mesh of that
size could be made: `bench make` builds the honeycomb in memory first
(476 MiB a M cells, so 47 GB), and `blocks make` reads the mesh whole.
`bench make --formats blocked` now writes the honeycomb straight into a
blocked VTKHDF file (`murmel_mesh::synth::blocked`) with two cell fields,
a Taylor–Green vortex array in a uniform stream (`velocity`) and its
`pressure`:

- Every cell, point and face of a block follows from the lattice, so
  only the cells' Hilbert order is held (16 bytes a cell while it is
  sorted, 8 after), and one block per thread.
- A first pass counts each block's ghost cells and points; the datasets
  are then set aside at their sizes (`H5Writer::reserve`) and every
  thread writes its blocks' rows where they go (`H5Writer::rows`).
- The ids within a block are 32-bit (card of 2026-11-25:
  `blocked-vtkhdf.md` § 5 now allows them), so the file takes 940
  bytes a cell instead of 1,500: 94 GB at 100 M cells instead of 150 GB,
  the iMac's whole free space.
- On a small honeycomb the file holds what `blocks make` writes for the
  mesh in memory: the same blocks, ghosts, cells, faces, field rows and
  block index; only the points' numbers differ. VTK 9.7's `vtkHDFReader`
  opens it with every cell, field and patch, and 9.6 reads its cells.

Here (4 threads, 15 GB, the file written into the page cache):

| `bench make --formats blocked` | 1 M | 3 M | 10 M |
|---|---:|---:|---:|
| cells | 1,005,039 | 3,002,419 | 10,088,659 |
| blocks | 16 | 46 | 154 |
| file | 0.92 GB | 2.79 GB | 9.45 GB |
| bytes a cell | 920 | 929 | 937 |
| time | 1.9 s | 5.7 s | 14.5 s |
| peak memory | 492 MiB | 520 MiB | 605 MiB |

At 10 M cells the order took 0.8 s, counting 2.2 s, the rows 8.7 s and
the patches 0.4 s, with 26 s of CPU time over the 4 threads.

The files opened as overviews (`murmel call mesh.open`, mode overview,
from the page cache):

| | 1 M | 3 M | 10 M |
|---|---:|---:|---:|
| open | 0.3 s | 0.8 s | 1.2 s (3.5 s just after writing it) |
| peak memory | 272 MiB | 564 MiB | 1,243 MiB |

Both grow with the boundary (n², 4.6 times from 10 M to 100 M).

On the 10 M overview, through the tools (which cut before they answer;
the window shows the preview grid's faces meanwhile):

| step | time | blocks |
|---|---:|---:|
| slice across x through the middle, coloured by `pressure` | 24 s | 63 cut |
| the slice moved by 0.05 | 9 to 11 s | 32 cut, the others kept |
| a box region a fifth of the mesh wide | 2.8 s | 12 read (786,432 cells and the halo) |
| iso-surface of `pressure` at its middle value | 42 s, then left on the grid | its faces would take over a quarter of the budget (BLOCK006) |

The peak over that run was 6.3 GB.

What it shows, and what to expect at 100 M cells (n = 369, 100,079,441
cells, 1,528 blocks), estimated from these numbers:

- Writing is bound by the disk: about 260 s of CPU time (half a minute
  or so on the iMac's 8 cores) for 94 GB, so about a minute on an SSD
  and over ten minutes on the spinning part of a Fusion Drive. It holds
  about 3 to 4 GB: the cell order (1.6 GB while sorted) and about
  100 MB a thread.
- The overview opens in about 6 s from the page cache and holds about
  5.7 GB; longer the first time, as the boundary's owner rows are read
  from the disk.
- A box region of 12 blocks takes about 3 s whatever the file.
- A plane crosses about 4.6 times as many blocks as at 10 M, so the
  exact slice through the middle follows the grid's after about two
  minutes (0.38 s a block). Reading several blocks at once is the next
  step; this file is what it can be measured on.
- An iso-surface whose value runs through the whole volume finds only
  after reading the blocks that its faces exceed the budget; estimating
  the faces from the block ranges first would save that time.
- The preview grid reads the points and point lists of every block once
  (about 28 GB at 100 M with 32-bit ids) and is then kept.

## Slices of the 100 M cell file over the network (2026-11-25)

The 100 M cell test file (94 GB, n = 369, 1,528 blocks) on the user's
NAS (gigabit, about 100 MB/s), timed on the iMac with `murmel run
--script` (main at d6fdfb1):

| step | iMac, file on the NAS |
|---|---:|
| `mesh.open`, mode overview, first time (with the BLAKE3 of 94 GB) | 926 s |
| `mesh.open` again (the digest remembered) | 16.0 s |
| colour by `pressure` | 6.0 s |
| slice across x through the middle (and the preview grid, sampled once) | 1,135 s |
| the slice moved by 0.05 | 361 s |
| peak memory | 12.1 GB |

The window ended on opening it: the boundary's face vertices took one
GPU buffer of 1.6 GB, over the 256 MiB a buffer may hold (#115 draws
the surface in pieces).

The slice was slow for a reason the timings from the page cache here
had hidden: `read_blocks` read every patch whole for every block, to
keep the faces the block's cells own. On the 10 M file a slice
through the middle read 10.9 GB for its 63 blocks, the last 119 MB of
the file (the patches) each time; at 100 M the patches are about
550 MB, a block's own rows about 20 MB.

Now an overview reads its blocks through one `vtkhdf::BlockFile`: the
file opened once (its structure read once, not for every block), the
owner of each patch face read once, and each block's faces read by
their rows. The cut reads the next blocks on a thread of its own while
it cuts one.

On the 10 M file here (4 threads, the file in the page cache):

| | before | after |
|---|---:|---:|
| bytes read, open + slice + slice moved | 13.5 GB | 3.7 GB |
| slice through the middle (63 blocks) | 23.0 s | 12.5 to 13.1 s |
| the slice moved by 0.05 | 9.3 to 10.1 s | 5.1 to 5.3 s |

Of the 16 s the slice took before reading ahead, reading the blocks
took about 9 s (cells 4 s, topology 3 s) and cutting them 6.6 s, so
with the reading on its own thread the cut is now about as long as
the reading. At 100 M over the NAS each block reads about 20 MB
instead of 570 MB, so the slice through the middle should take a
minute or two (its blocks' rows, about 6 GB, at the network's speed,
with the cutting alongside) instead of 19 minutes.

### Three blocks read at once (2026-11-26)

The iMac again, the file on the NAS, after #115 and #116 (main at
a2b4653):

| step | d6fdfb1 | a2b4653 |
|---|---:|---:|
| `mesh.open` again | 16.0 s | 18.3 s, 17.1 s |
| colour by `pressure` | 6.0 s | 5.4 s |
| slice across x through the middle | 1,135 s | 274 s |
| the slice moved by 0.05 | 361 s | 109 s |
| peak memory | 12.1 GB | 10.7 GB |

The window opens the file now. The slice is still bound by the
network: its blocks, about 290 of some 40 MB, about 12 GB (an
estimate from the 10 M file), take about two minutes at gigabit, and
one reader got about 44 MB/s out of the link, waiting for each answer
before asking for the next. So a cut now reads three blocks at once
while it cuts one (`READERS`, fewer when the eighth of the budget the
kept blocks take holds fewer blocks), and cuts them in the order of
the blocks, whatever order they come in, so the faces are the same as
with one reader.

On the 10 M file here (4 threads), against the same code with one
reader; "over a link" puts a stand-in for the NAS in front of the
file, a shim that makes every read of it wait as a 100 MB/s link with
0.5 ms latency would (2 ms gave the same within 3 s):

| | one reader | three readers |
|---|---:|---:|
| slice through the middle, from the page cache | 13.0 to 13.8 s | 11.5 to 11.7 s |
| the slice moved by 0.05, from the page cache | 4.8 to 5.0 s | 4.6 to 4.9 s |
| slice through the middle, over a link | 55.8 s | 47.5 s |
| the slice moved by 0.05, over a link | 21.1 s | 18.1 s |
| peak memory | 3.9 to 4.0 GB | 4.2 to 4.4 GB |
| bytes read, open + slice + slice moved | 4.3 GB | 3.9 GB |

Over the stand-in link three readers keep it busy all the time (the
script took as long as its bytes at the link's speed), one about
85 % of it. How much the NAS gains depends on how long SMB waits
between reads, which the stand-in does not know: the iMac's one
reader used less than half the link.

## Clip drag: a live cut in a thin layer (2026-11-25)

The maintainer's report: at about 2 M polyhedral cells the clip drag was
slow and awkward. While dragging, the GPU only cut the surface built
without the moving plane, so the cut looked hollow (the inside of the far
walls). Once the plane rested for 120 ms, two whole surfaces were built
on threads, the one without the plane and the exact cut, and the cells
appeared in one go about half a second later. The maintainer's cards
asked for the exact cut to follow the pointer ("live exact cut, thin
layer") and, when a step is slow, for the cut to lag a little and stay
exact rather than switch to a simpler picture.

- A plane cuts away whole cells (crinkle) or slices them (flat). The
  cells it leaves wholly on one side are cut exactly by the shader: their
  faces lie wholly on one side too. Only the cells the plane crosses
  need faces of their own: the faces a crinkle plane keeps whole, or the
  sections a flat plane cuts. `Surface::clip_layer` builds just those.
  It finds the cells near the plane in the cell bins (`crate::bins`),
  works out the distances for their nodes alone (a lazy `Cutter`), and
  walks only their faces through the same face rules as the whole build
  (`face_polys`, `sections`). A step costs what the layer holds (about
  N^(2/3) cells), not what the mesh holds.
- In the window, each step of a drag starts a layer build on a thread
  of its own. One runs at a time and the latest display wins. The
  renderer draws the surface without the moving planes, cut at the
  layer's planes, and the layer over it uncut (drawn as instance 1, which
  the shader's `uncut` flag exempts from the cut). The planes are those
  the layer was built for, so the two always fit: on a slow step the cut
  lags a step behind the pointer and is never wrong.
- The exact surface is still built once the plane rests (picking, counts,
  feature edges); it shows the same picture, so nothing pops.
- Hovering a clip handle bins the cells and builds the surface without
  that plane ahead of the drag (`Session::warm_clip`). After a drag, that
  surface is kept for the next drag of the same plane.
- Tested: the layer holds exactly the triangles of the whole build
  that belong to the crossed cells (crinkle), or exactly its sections
  (flat), with a second plane and a hidden zone, and every other
  triangle lies on the right side of the plane
  (`clip_layer_completes_the_cut_surface`). A render test checks that
  the live picture matches the exact cut and that the hollow one does not.

Honeycomb of 2.09 M cells (14 faces per cell, heavier than a KoflCraft
mesh of the same size), polyMesh binary, Linux container, 4 threads,
`murmel bench run --repeat 3 --no-fields --no-render`:

| Stage | ms |
|---|---:|
| clip.exact (the whole cut, after a 120 ms rest; as before) | 474 |
| clip.step.built (one whole cut) | 213 |
| clip.layer.first (bins the cells once per mesh; done on hover) | 1,427 |
| clip.layer (crinkle, 340,236 triangles) | 45 |
| — layer.cells, layer.crossed | 3.5, 4.9 |
| — layer.polygons | 12.6 |
| — surface.points, surface.buffers | 6.8, 10.6 |
| clip.layer.flat (123,224 triangles) | 84 |

The cut now follows the pointer at about 20 crinkle steps a second on
four threads (the iMac's sixteen should do better), against one whole
cut after the pointer stops before. Left: the cut points of a flat
layer are numbered on one thread (about 60 ms of its 84). A flat layer
draws only sections, so each cell could name its own points on any
thread.
