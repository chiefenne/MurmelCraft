# Accessibility

What MurmelCraft does for keyboard and screen-reader use, and how it is
checked. The rules for new widgets are in [AGENTS.md](../AGENTS.md).

## Keyboard

- **F6 and Shift + F6** move the focus between the regions of the window,
  in this order: title bar, toolbar, model tree, viewport, properties,
  status bar. A hidden panel is skipped, and the order wraps round. With
  nothing focused, F6 starts at the title bar and Shift + F6 at the
  status bar. The focus lands on the region's first control (the theme
  button, Open, the tree filter, the viewport, the first property row, the
  status bar's message).
- **Tab and Shift + Tab** move from control to control. The title bar and
  the toolbar are drawn from left to right, so Tab follows what is shown.
- **Arrow keys** move between tree rows and buttons once the keyboard has
  reached them, and step sliders. In the viewport, or after a mouse click
  on a button, they orbit (Shift pans) as before. **Esc** leaves a focused
  control; in the viewport it clears the selection.
- **Enter and Space** press the focused button.
- Every command has a key or a palette entry (Ctrl + K); the shortcut
  sheet (F1) and [keys.md](keys.md) list them. The view cube's views are
  also on keys (Alt + 1, 3, 7) and in the palette.

## Focus ring

The focused control gets a 2 px ring in the accent colour, outside the
control (inside the viewport, which fills its pane). It shows only after
keyboard use (Tab, F6) and hides on the next mouse press, like CSS
`:focus-visible`.

## Screen readers (AccessKit)

eframe builds the AccessKit tree, which reaches VoiceOver (macOS),
Narrator and NVDA (Windows) and Orca (Linux, AT-SPI).

- The six panes have names ("Title bar", "Toolbar", "Model tree",
  "Viewport", "Properties", "Status bar").
- Icon buttons carry a name, never their icon glyph (Lucide glyphs are
  private-use characters a reader cannot say). Painted glyphs beside text
  (the filter's search icon, the status bar's alert) are not in the tree.
- Drop-downs name their setting and value ("Render mode: Shaded with
  edges", "Clip mode: Crinkle"); toggles name their state ("Theme: Dark",
  "Projection: Perspective").
- Tree rows read with their count and state ("inlet, 85 faces, hidden");
  property values read with their label ("Cells: 776").
- The viewport is a canvas named by its view, counts and colouring; the
  status bar is a status node with its message and the mouse hint.

## Contrast

The theme test (`murmel-engine`, `contrast_is_aa`) checks both palettes
against WCAG 2.2 AA: text at least 4.5:1 on every background it sits on,
the status bar's error text 4.5:1, the accent (focus ring, selection
outline) and the status dots at least 3:1, the focus ring also on both
ends of the viewport gradient.

## How it is checked

- `crates/murmel-app/src/a11y_tests.rs` drives the real window headless
  with egui_kittest (MIT or Apache-2.0, a test-only dependency): every
  control of the main window, the settings window, the shortcut sheet and
  the palette has a name without glyphs, every pane is named, F6 visits
  the regions in order, and the arrows go to the view or to the focused
  control as above.
- Not checked by a machine: a session with a real screen reader. Try
  VoiceOver (Cmd + F5) on macOS and report what reads badly.

## Not yet

A high-contrast theme, translations other than English, and keyboard
editing of the clip plane's gizmo (the clip slider and keys cover it).
