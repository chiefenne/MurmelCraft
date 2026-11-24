### Changed

- Slices and iso-surfaces are cut on all threads and test only the cells near them: the cells sit in bins of a regular grid (made once per mesh), a slice tests the cells of the bins it passes and an iso value those of the bins whose values can reach it. At 1 M cells a step of a slice drag takes 0.42 s instead of 1.06 s, and a step of an iso value 0.97 s instead of 2.9 s. What is drawn is the same, bit for bit.
- Colouring by a field the first time, and arrows the first time, take about half and a third of the time: the field's topology is made on all threads, a vector's component is made alone (not with the other two), its magnitude in one pass with them, and the arrows read their largest magnitude from the cells.
