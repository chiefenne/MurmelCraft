### Added

- The preview grid of a mesh opened as an overview is kept in the cache folder (`MURMEL_CACHE_DIR`, else `~/Library/Caches/MurmelCraft` on macOS, `%LOCALAPPDATA%\MurmelCraft\cache` on Windows, `~/.cache/murmelcraft` elsewhere), so the file opened again unchanged shows its slices, iso-surfaces and arrows without sampling its volume again: about half a second whatever the size of the file. The 16 grids used last stay, within 2 GiB.
