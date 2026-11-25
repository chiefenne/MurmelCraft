### Changed

- The fast CI run takes a fraction of the time: cargo-nextest runs every test binary at once, our crates build with `opt-level = 1` and no debug info for the tests, and the Mesa packages come from a cache. It still runs the same checks and tests.
