### Added

- Opening a `.vtu`, an OpenFOAM mesh or an unblocked VTKHDF file that needs more than the memory budget now asks first: convert it once to a blocked copy beside it (`NAME.blocked.vtkhdf`), which opens as an overview, or open it whole. The conversion runs in the background with Cancel or Esc, and next time the up-to-date copy opens at once. "Convert large meshes without asking" (in the question, in Settings, or `settings.set_memory` with `convert_large`) skips the question.
- `mesh.open` warns (BLOCK007) when it opens such a mesh whole, and names its blocked copy.
