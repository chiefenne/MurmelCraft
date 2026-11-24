### Changed

- Ascii `.vtu` files open faster: the numbers are parsed into memory the parsing threads fill, sized from the counts the file declares, and counted with vector compares. At 4 M cells opening takes 7.5 s instead of 9.3 s on 4 threads, with the same peak memory. Points are read straight into points (`.vtu` and ascii polyMesh), without a copy.
