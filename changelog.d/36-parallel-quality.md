### Changed

- Colour by quality computes the cell quality on all threads: 4 M cells take 2.1 s instead of 8.0 s on 4 threads. Surface builds (every hide, colour or clip change) are 30 to 55 % faster, and `murmel bench` reports the surface's phases.
