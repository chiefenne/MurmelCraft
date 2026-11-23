### Added

- F6 and Shift + F6 move the keyboard focus between the regions of the window: title bar, toolbar, model tree, viewport, properties and status bar (hidden panels are skipped). Tab then moves from control to control, left to right in the title bar and the toolbar.
- A 2 px focus ring in the accent colour shows which control has the focus after keyboard use; a mouse press hides it.
- Screen readers (AccessKit): the panes have names, drop-downs and toggles name their setting and value, tree rows read with their count and whether they are hidden, property values with their label, and the status bar is a status node. No name holds an icon glyph any more (the Open button, the filter and the status bar's alert had one). See docs/accessibility.md.

### Changed

- The arrow keys and Esc belong to a control the keyboard reached (tree rows, buttons, sliders); in the viewport and after a mouse click they orbit and clear the selection as before.
- The light theme's error colour is a little darker (#C03D2D, was #C7402F), so the status bar's error text reaches WCAG AA contrast (4.66:1).
