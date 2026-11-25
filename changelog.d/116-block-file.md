### Changed

Slices, iso-surfaces, probes, streamlines and regions on an overview read
far less of the file. Each block used to read every boundary patch whole
(about 550 MB for the 100 M cell test file); it now reads only the rows of
its own boundary faces, and the file is opened once. Exact slices also read
the next blocks while one is being cut. A slice through a 10 M cell file
reads 3.7 times fewer bytes and takes 13 s instead of 23 s; over a network
share, where every byte counts, the gain is larger.
