### Added

- The time series player's speed and loop can be your defaults: right-click the speed or the Loop box in the player bar and save it; series opened later start with it (paused). Settings lists them under "Time series player" and forgets each; `settings.set_default` with kind `player` (keys `rate`, `loop`) does the same from the command line.

### Changed

- A playing time series reads its next step ahead on a thread of its own, with its derived fields and, while the view is unchanged, its surface, so the window stays free and steps follow each other at the speed set. A step that takes longer to read than the speed allows keeps the step shown until it is ready, instead of holding the window. A step is read ahead only when two steps fit the memory budget.
