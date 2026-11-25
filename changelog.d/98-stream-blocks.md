### Added

- On a mesh opened as an overview, streamlines are traced through the blocks of the file: each block is read with its halo the first time a line reaches it, and the lines are the ones the whole mesh gives. The blocks gone through stay, so moving or changing a set reads only new blocks; a trace reads at most as many blocks as a quarter of the memory budget holds, and a line that needs another ends there (Properties: "Reached a block not read", BLOCK006). In the window the lines are traced in the background; the status bar counts the blocks read, with Cancel or Esc. `stream.set` traces before it answers and gives `traced.blocks`.

### Changed

- The streamlines of an overview draw beside the slices on its preview grid (they did not draw on an overview before).
