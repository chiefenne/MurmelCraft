### Changed

- A `.vtu` file's cells are built on all threads, and a file of polyhedra and polygons (as KoflCraft writes) leaves their faces where the file put them instead of moving them. At 4 M polyhedra on 4 threads building the cells takes 306 ms instead of 674 ms; the mesh is the same bit for bit.
