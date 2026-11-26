### Changed

Exact slices and iso-surfaces on an overview read three blocks at once while
one is being cut, and still cut them in the order of the blocks, so the
faces are the same as before. A file on a network share is read with
several requests on the way instead of one, which keeps the link busy: over
a stand-in for a gigabit NAS a slice through a 10 M cell file takes 48 s
instead of 56 s, and the link is full. It holds up to two blocks more than
before (about 0.4 GB at 10 M).
