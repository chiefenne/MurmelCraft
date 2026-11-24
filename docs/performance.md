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
`polymesh-binary` (OpenFOAM, `faceCompactList`).

`bench run` times, in this order:

| Stage | What it measures |
|---|---|
| `read` | the reader: file to mesh model, face topology included |
| `digest` | SHA-256 of the mesh files (the validity badge needs it) |
| `open` | `mesh.open` as the window does it: read, evidence, first surface (the files are hashed afterwards) |
| `open.digest` | the wait, after `open`, until the SHA-256 of the files is in place (the validity badge) |
| `surface` | the surface of the default display (boundary faces), with its phases: `surface.polygons` (which faces to draw), `surface.points` (corners and normals), `surface.buffers` (vertices, triangles, edges), `surface.extras` (slices, iso-surfaces, arrows) |
| `surface.crinkle`, `surface.flat` | the surface with one clip plane through the centre |
| `surface.hidden_zone` | one zone hidden, so the faces between zones lie open |
| `surface.cell_type` | colour by cell type |
| `surface.feature_edges` | render mode 4 (feature edges found on the surface) |
| `quality` | the cell quality metrics (colour by quality) |
| `tree.frame` | what the model tree and the legend ask of the mesh on each frame (counts, items), median of 5 |
| `clip.step` | one step of a clip drag as the window does it (the plane moves, the GPU cuts), median of 5 |
| `clip.exact` | from the last step until the exact crinkle cut is in place (built on its own thread once the plane rests) |
| `clip.step.built` | one step that builds the exact cut before it returns (the command line, agents; every step before the preview) |
| `pick.9` | nine picks over the iso view (the hover tooltip, a click, snapping) |
| `render.first`, `render.next` | an offscreen frame with the upload, and a later one |
| `render.next.faces` | a later frame without the edges (the window while the view moves, when frames with edges are slow) |

and reports the peak resident memory (Linux only; the `read` stage also
reports its own peak, `peak_rss_mib`) and the size of the mesh model. `render.*` on a machine without a GPU runs on Mesa's software
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
