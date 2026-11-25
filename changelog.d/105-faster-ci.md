### Changed

- The fast CI run takes a fraction of the time: cargo-nextest runs every test binary at once, the mesh and engine crates build with `opt-level = 1` for the tests (locally too), CI builds without debug info, and the Mesa packages come from a cache. It still runs the same checks and tests.

### Fixed

- F6 now reaches the player bar while a series shows its first step; it did nothing there, because the disabled "First step" button was the one it tried to focus.
