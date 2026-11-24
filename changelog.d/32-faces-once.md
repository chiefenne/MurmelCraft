### Changed

- The mesh model stores each face once instead of once per cell: a mesh takes about 27% less memory (476 MiB per million polyhedral cells instead of 655), and an OpenFOAM polyMesh opens faster with a lower peak.
