### Changed

- VTKHDF and parcel files are no longer read into memory whole: the reader reads the file's structure when it opens, and each dataset from the file when it needs it, on all threads; datasets it does not need are never read. A 4 M cell blocked file opens in 5.1 s instead of 7.9 s, with a peak of 7.2 GB instead of 8.3 GB while reading; the mesh and fields are the same bit for bit.
