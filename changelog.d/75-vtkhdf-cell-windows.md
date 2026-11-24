### Changed

- VTKHDF cells are built from their datasets a few thousand cells at a time, each reading only the rows it needs, instead of from whole datasets: a 4 M cell blocked file peaks at 4.0 GB while reading instead of 7.2 GB (5.9 GB instead of 9.1 GB for the whole open), and opens in 4.2 s instead of 4.8 s; the mesh and fields are the same bit for bit.
