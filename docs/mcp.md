# The MCP server

`murmel mcp` (or the program `murmel-mcp`) lets an agent drive MurmelCraft
through the [Model Context Protocol](https://modelcontextprotocol.io/specification/2025-11-25).
It offers every tool of the registry ([tools.md](tools.md)) by its dotted id,
the same ids the window's clicks, keys and menus call and `murmel call` runs.
There is no second set of actions: a new tool in `murmel-tools` is an MCP tool
the moment it is registered.

The server is headless: it keeps one viewer session for the whole
connection, so the open mesh, the camera, the display, the clip plane, the
selection and the undo journal carry from one call to the next.
`view.screenshot` renders that session offscreen (wgpu) and returns the PNG
as an MCP image, so the agent sees what it set up. With `--live` the same
server drives the person's open window instead
([Drive the open window](#drive-the-open-window)).

## Start it

```sh
cargo build --release -p murmel-cli
target/release/murmel mcp --read-root ~/meshes --write-root ~/murmel-out
target/release/murmel mcp --root ~/project     # one folder for both
```

The server speaks JSON-RPC 2.0 on standard input and output, one message per
line (MCP revision 2025-11-25; it also answers clients that ask for
2025-06-18). Messages to the person go to standard error.

### Claude Code

```sh
claude mcp add murmelcraft -- /path/to/murmel mcp --read-root /path/to/meshes --write-root /path/to/out
```

### Claude Desktop and other clients

```json
{
  "mcpServers": {
    "murmelcraft": {
      "command": "/path/to/murmel",
      "args": ["mcp", "--read-root", "/path/to/meshes", "--write-root", "/path/to/out"]
    }
  }
}
```

## Drive the open window

```sh
target/release/murmel mcp --live --root ~/project
claude mcp add murmelcraft-live -- /path/to/murmel mcp --live --root /path/to/project
```

With `--live` the server has no session of its own: each call, once its
paths are checked against the roots, runs in the open MurmelCraft window,
on the window's session, exactly as a click or a key does there. The person
watches the agent orbit, clip and colour; the agent's steps are undo steps
of the window's journal (Cmd + Z or Ctrl + Z undoes them); a failed call
shows in the status bar like the person's own.

- **Allowing agents.** The window listens while Settings > Agents > *Let
  agents drive this window* is on (`settings.set_agents`, saved in
  `settings.toml` as `[agents] enabled`; on by default). While an agent is
  connected the status bar says *Agent connected* with a **Stop** button,
  which switches agents off (and so closes every connection) until the
  person switches them on again.
- **The socket.** The window listens on a Unix domain socket that only the
  person's account can open: `$XDG_RUNTIME_DIR/murmelcraft/live.sock` on
  Linux, `$TMPDIR/murmelcraft/live.sock` on macOS (a folder of mode 700, a
  socket of mode 600; a folder others can open is refused, `LIVE003`).
  `MURMEL_LIVE_SOCKET` names another path, for the window and the server
  alike. One window listens; a second one says `LIVE003` in Settings.
  Windows has no bridge yet.
- **No window.** The server connects when a call comes, so the window may
  open after the agent started. Without a window a call fails with
  `LIVE001` and nothing runs headless instead; if the window goes away or
  the person presses Stop while a call waits, it fails with `LIVE002`.
- **Screenshots** are rendered from the window's session; without a
  `width` and `height` they come at the size of the viewport, so the agent
  sees what the person sees (the 3D view, without the panels). The PNG is
  written below a write root and returned as an image, as above.
- **Settings** tools change the window for this run and never write the
  person's files (`SET001`), as in the headless server.

The wire between the server and the window is one JSON message per line:
a handshake (`{"hello": "murmelcraft-live", "version": 1}`), then
`{"id", "tool", "input"}` requests answered by `{"id", "envelope"}`
(`crates/murmel-tools/src/live.rs`). Only the server speaks it; the roots
are checked in the server, which the person starts with the roots they
choose.

## Roots

The server refuses to start without at least one `--read-root` and one
`--write-root` (both must be existing folders). Before a call runs, every
input field that names a path is checked against them; each tool declares
those fields (`reads` and `outputs` in the registry, shown in
`system.commands`), and a test fails when a field named like a path
(`path`, `mesh`, `file`, `folder`, `dir`, `output`, or ending in one of them
after `_`) is not declared.

- A path the call reads (`mesh.open` `path`, `view.screenshot` `mesh`) must
  exist below a read root or a write root.
- A path the call writes (`view.screenshot` `path`) must have its folder
  below a write root; an existing link at that place is refused.
- Links are followed before the check, so a link inside a root that points
  out is refused, and `..` cannot climb out.
- A relative path starts at the **first write root**, not at the folder the
  client was started in.
- A refusal is `MCP001`, a path that does not resolve `MCP002`; the call does
  not run.

The roots confine the paths the tools are given. Files a reader opens next to
the mesh it is given (`mesh-evidence.json`, the files of a `polyMesh` folder)
are found beside that mesh. The mesh a session file names
(`session.load`, [sessions.md](sessions.md)) is a path inside a file, not
in the input, so the server reads the session file before the call and
refuses it with `MCP001` when that mesh lies outside the read roots; this
holds for the live server too, before the call reaches the window.

## Screenshots

`view.screenshot` over MCP may leave out `path`: the server then writes
`screenshots/shot-0001.png`, `shot-0002.png`, … below the first write root.
The result has the envelope as text and as structured content, then the PNG
as an `image` block (`image/png`, base64). A PNG over 4 MiB is only written,
and a text block says where.

## Settings

The session starts from the person's settings (theme, mesh colours, view
cube, mouse bindings, from `MURMEL_CONFIG_DIR` or the platform's config
folder), as the CLI does. The settings tools (`settings.set_*`) change this
session only and never write the person's config files; they report
`SET001` ("applied for this session, not saved").

## Results

Each `tools/call` returns:

- `structuredContent`: the registry's envelope (`murmelcraft.tool-result`:
  tool, version, status, exit code, result, diagnostics, seconds);
- `content`: the same envelope as JSON text, and for a screenshot the image;
- `isError`: true when the tool failed or was refused. The diagnostic codes
  are in [tools.md](tools.md) and in `diag.codes`.

An unknown tool or a call without a name is a JSON-RPC error (`-32602`), as
the specification asks. `tools/list` gives each tool its title (the English
label of the window), its input schema, an output schema of the envelope and
the hints `readOnlyHint` and `idempotentHint` for the tools that only read
(`NOT_JOURNALED` reason "reads only"); `_meta` carries the tool version,
action, path fields and whether its calls are undo steps.

## Limits

- One message is at most 256 KiB; a longer line is answered with `-32600`
  and skipped.
- One call at a time, in order; there is no cancellation.
- With `--live` a call waits at most two minutes for the window
  (`LIVE002`); a minimised window may not draw, and so not answer, until
  it is shown again.
- Screenshots are at most 8192 × 8192 pixels (`RENDER003`).
