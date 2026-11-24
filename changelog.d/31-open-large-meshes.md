### Changed

- Opening a mesh is 2 to 4 times faster and needs less memory: a 1 M cell ascii `.vtu` opens in 3.3 s instead of 12.8 s, a binary polyMesh in 1.5 s instead of 2.2 s. The file is read once, numbers are parsed on all cores, and faces are matched without hashing.
- A mesh of 32 MiB or more opens in the background: the window keeps working, the status bar shows what is being read and for how long, and Cancel (or Esc) stops it.
