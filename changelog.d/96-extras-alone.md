### Changed

- Moving a slice, changing an iso value or the arrows now builds only those again, not the mesh's own faces, and the renderer uploads only them (a slice step on 1 M cells: 421 to 230 ms).
