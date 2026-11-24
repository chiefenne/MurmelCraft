### Changed

- Moving the view on a large mesh hides the edges while it moves, only when a frame with edges takes longer than 33 ms; they come back as soon as the view rests, and nothing else is simplified. Meshes that draw fast enough look as before, and pictures and movies always draw the edges. `murmel bench` reports `render.next.faces`, a frame without the edges.
