### Added

- Colour mode **Solid colour**, now the default: every boundary face in one green (#6EC64A, sampled from the maintainer's reference screenshot), edges darkened as before.
- Every mesh colour is configurable. `display.set_color` colours one item of the open mesh (mesh, zone, cell type or patch; also from the tree's context menu), and `settings.set_colors` saves the user's default colours (solid, patch palette, role and cell-type colours, edge darkening) in settings.toml (also from Settings, Colours).

### Changed

- `settings.set_theme` keeps the saved colours; settings.toml holds both.
