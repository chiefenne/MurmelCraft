### Changed

- The window and the command line remember the SHA-256 of each mesh file they check (`checked-files.json` in the settings folder). A file opened again unchanged (same size, modification time and, on Unix, change time and file id) is not hashed again: on the iMac a 4 M cell `.vtu` shows its badge at once instead of 17 s later, and `murmel call mesh.open` takes about 5 s instead of 20 s.
