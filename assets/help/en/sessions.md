+++
covers = ["window.session_save", "window.session_save_as", "window.session_load", "session.*"]
related = ["open", "saved-views"]
chapter = "getting-started"
+++
A session file (`.murmel`) keeps what you set up on a mesh: the camera, colours, clip planes, slices, arrows, streamlines, measurements, probes and saved views. Loading it opens the mesh again and puts everything back.

<!-- more -->

## Sessions

- **Save session** writes to the file last saved or loaded, and asks for a name the first time.
- **Save session as…** always asks.
- **Load session…** opens a `.murmel` file. Open mesh… and dropping a file take session files too.

The mesh is kept by its path relative to the session file, by its absolute path, and by its digest. If the mesh changed since the session was saved, the status bar warns (SESS003). Items that no longer fit the mesh are skipped, with a warning (SESS004).

Loading applies the session's colours and view cube placement for this session only. Your settings file is not changed.
