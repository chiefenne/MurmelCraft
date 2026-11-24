### Changed

- Linking the cell faces of a mesh (matching each face with its twin) sorts and copies the faces faster: the `.vtu` reader notes each face's smallest point as it reads it, and the distinct faces are sized on all threads. At 4 M polyhedra on 4 threads, linking takes 1.2 s instead of 1.65 s and the open about 0.35 s less; the mesh is the same bit for bit.
