### Changed

- A playing time series reads its next step ahead on a thread of its own, with its derived fields and, while the view is unchanged, its surface, so the window stays free and steps follow each other at the speed set. A step that takes longer to read than the speed allows keeps the step shown until it is ready, instead of holding the window. A step is read ahead only when two steps fit the memory budget.
