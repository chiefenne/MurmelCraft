### Changed

- Blocked VTKHDF files open several times faster and with less memory: the reader reads the datasets where they lie in the file instead of copying them, joins the points of the blocks on all threads and builds the cells on all threads. At 1 M cells on 4 threads a blocked file opens in 1.6 to 1.9 s instead of 6.2 s, with a peak of 2.0 GB instead of 3.4 GB; the 4 M cell file, which ran out of memory in 15 GB before, opens in 7 s. The mesh and fields are the same bit for bit.

### Fixed

- A blocked VTKHDF file with ghost cells and a field the reader does not convert (half floats) no longer crashes the reader; the field has no values, as in a file without ghosts.
