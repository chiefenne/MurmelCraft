### Fixed

The window no longer ends when it opens a very large mesh: the boundary of a
100 M cell overview (1.6 GB of face vertices, 349 MB at 10 M) is drawn in
pieces of at most 256 MiB, the most one GPU buffer may hold. Any other error
the GPU reports while drawing now shows in the status bar (RENDER006) instead
of ending the window.
