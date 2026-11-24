### Changed

- Mesh files are checked with BLAKE3 instead of SHA-256 (the user's choice, 2026-11-24, "BLAKE3 only, still check old"): many times faster on the iMac, which has no SHA instructions. KoflCraft's evidence 1.1 names its hash (`file_hash`); evidence 1.0, older session files and older `checked-files.json` lists are still checked with SHA-256. Session files are format 1.15 (`mesh.blake3`), and `mesh.open` returns `hash` with each file's digest under it.
