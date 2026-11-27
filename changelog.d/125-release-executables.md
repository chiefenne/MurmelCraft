- **Release downloads**: the new workflow Executables builds MurmelCraft
  for macOS (one universal build for Apple silicon and Intel:
  `MurmelCraft.app` in a `.dmg`, and the programs in a `.tar.gz`), Linux
  x86-64 and Windows x86-64, and attaches them with `SHA256SUMS.txt` to the
  release; the Release workflow starts it after each release, and it can be
  run by hand for an existing one (docs/release.md). The README's
  "Download a release" says which file to take and how to open the
  unsigned Mac app the first time.
- The Windows release build of `murmelcraft` is a window program: started
  from the Explorer it opens no console window beside it.
