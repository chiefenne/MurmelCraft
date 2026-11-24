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
| `open` | `mesh.open` as the window does it: read, digest, evidence, first surface |
| `surface` | the surface of the default display (boundary faces) |
| `surface.crinkle`, `surface.flat` | the surface with one clip plane through the centre |
| `surface.hidden_zone` | one zone hidden, so the faces between zones lie open |
| `surface.cell_type` | colour by cell type |
| `surface.feature_edges` | render mode 4 (feature edges found on the surface) |
| `quality` | the cell quality metrics (colour by quality) |
| `pick.9` | nine picks over the iso view (the hover tooltip, a click, snapping) |
| `render.first`, `render.next` | an offscreen frame with the upload, and a later one |

and reports the peak resident memory (Linux only) and the size of the mesh
model. `render.*` on a machine without a GPU runs on Mesa's software
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
