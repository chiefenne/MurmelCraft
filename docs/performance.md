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
| `clip.step.built` | one step that builds the exact cut before it returns (the command line, agents; every step before the preview) |
| `pick.9` | nine picks over the iso view (the hover tooltip, a click, snapping) |
| `render.first`, `render.next` | an offscreen frame with the upload, and a later one |
| `render.next.faces` | a later frame without the edges (the window while the view moves, when frames with edges are slow) |

and reports the peak resident memory (Linux and macOS, from
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
