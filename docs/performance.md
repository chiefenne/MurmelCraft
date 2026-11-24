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
| `surface` | the surface of the default display (boundary faces) |
| `surface.crinkle`, `surface.flat` | the surface with one clip plane through the centre |
| `surface.hidden_zone` | one zone hidden, so the faces between zones lie open |
| `surface.cell_type` | colour by cell type |
| `surface.feature_edges` | render mode 4 (feature edges found on the surface) |
| `quality` | the cell quality metrics (colour by quality) |
| `pick.9` | nine picks over the iso view (the hover tooltip, a click, snapping) |
| `render.first`, `render.next` | an offscreen frame with the upload, and a later one |

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
