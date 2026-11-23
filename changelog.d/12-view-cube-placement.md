### Added

- View cube placement (`settings.set_view_cube`, saved in settings.toml): any corner of the viewport (top right by default), a floating panel that you drag anywhere over the viewport by its grip, or hidden. With auto-hide on, the cube fades out while the pointer is farther from it than a set distance (120 px by default, 0 to 2000 px) and comes back as the pointer nears; it stays while you drag it or a menu of it is open.
- The placement is set from a new pull-down beside the cube's Home and projection buttons, from the View cube section of the Settings window, from the viewport's context menu (so a hidden cube can come back) and from the command palette.
- A cube in a left corner moves the view note and legend below it (top left) or the axis triad beside it (bottom left), so nothing is covered.
