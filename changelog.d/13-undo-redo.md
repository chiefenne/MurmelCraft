### Added

- Undo and redo of the camera and the display (UI spec: "Undo is the tool journal"): Ctrl+Z and Ctrl+Shift+Z (Cmd on macOS), and Undo and Redo in the command palette. Every call of a tool that changes the camera or the display (`view.*` moves, `display.*`, `clip.*`) is a step; one mouse gesture (all calls while a button is held) and one run of wheel or pinch steps make one step, so undo steps are gestures, not frames. Undoing a camera step flies back in 250 ms.
- The selection, the pick filter and the settings are not undone; opening or closing a mesh starts a new journal; 200 steps are kept.
- New tools `journal.undo`, `journal.redo`, `journal.group` (calls inside one group undo as one step, for scripts and the window) and `journal.list`.
