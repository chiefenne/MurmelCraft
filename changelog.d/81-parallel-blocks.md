### Changed

- The blocks a region reads on an overview are read on all threads: the points and ids of each block, the join of the points the blocks share (in shards) and the cells. On the 4 M blocked file 14 blocks read in 1.5 s rather than 2.6 s and 33 blocks in 3.2 s rather than 6.1 s, on 4 threads. The status bar says "reading the cells of the blocks" through every step of the read.
