<p align="center"><img src="assets/logo/murmelcraft-256.png" width="128" alt="MurmelCraft logo: an engraved marmot on a magenta rounded square"></p>

# MurmelCraft

The shared desktop viewer of [KoflCraft](https://github.com/chiefenne/KoflCraft)
(polyhedral CFD mesher) and [FoehnCraft](https://github.com/chiefenne/Foehncraft)
(CFD solver), named after the Murmeltier, the Alpine lookout. Rust, egui and
eframe on wgpu; one engine with thin front ends, every action a registered
tool that the window, the command line and the MCP server share.

## Status: V0.1

- Opens KoflCraft's `foehncraft.vtu` (VTK polyhedra, boundary polygons, zone
  and patch names and roles) and the `mesh-evidence.json` beside it.
- Opens OpenFOAM meshes: a case, `constant` or `polyMesh` folder, or a
  `.foam` file in the case (ascii, binary and gzip files, `cellZones`),
  such as KoflCraft's `export … openfoam`; the evidence in the polyMesh
  folder is checked file by file.
- Shaded with edges, patch colours, orbit, pan, zoom to the cursor, fit,
  home, the three projection modes (default: perspective with orthographic
  faces), dark and light themes switched live.
- Mouse presets (three-button, Apple, Fusion, ParaView), every binding
  editable through `settings.set_bindings` and saved in `navigation.toml`.
- Title bar, toolbar, viewport with view note and axis triad, status bar.
- Headless screenshots with `murmel call view.screenshot`, checked by
  render tests in CI.
- An MCP server for agents (`murmel mcp`, [docs/mcp.md](docs/mcp.md)): every
  tool of the registry on one headless session, paths confined to read and
  write roots, screenshots returned as images.
- Keyboard and screen-reader use ([docs/accessibility.md](docs/accessibility.md)):
  F6 between the regions, a visible focus ring, AccessKit names for every
  pane and control, palettes checked for WCAG AA contrast.

Not yet (see the milestones of the UI spec): model tree, properties, colour
by role or cell type (V0.2), view cube and command palette (V0.3), clip
plane, picking, orbit about the point under the cursor and cursor
coordinates (V0.4).

## Build and run

```sh
cargo run --release -p murmel-app -- test-fixtures/channel/foehncraft.vtu
cargo run --release -p murmel-app -- test-fixtures/openfoam/dual_polyhedra
cargo run --release -p murmel-cli -- call view.screenshot \
  --input '{"mesh": "test-fixtures/channel/foehncraft.vtu", "view": "iso", "path": "shot.png"}'
cargo run -p murmel-cli -- commands --markdown   # docs/tools.md
cargo run --release -p murmel-cli -- mcp --root .  # the MCP server on stdio
```

## Let Claude drive MurmelCraft (macOS, copy and paste)

`cargo build --release` builds three programs into `target/release/`:
`murmelcraft` (the window), `murmel` (the command line; `murmel mcp` is
the MCP server) and `murmel-mcp` (the same server on its own). Claude
Code needs the server's full path, so run these lines **in the
MurmelCraft folder** (`$PWD` fills the path in for you):

```sh
cargo build --release
mkdir -p ~/murmel-work                       # the folder the agent may read and write
claude mcp add --scope user murmelcraft-live -- \
  "$PWD/target/release/murmel" mcp --live --root ~/murmel-work
claude mcp list                              # murmelcraft-live: ... ✓ Connected
```

Then start the window (`target/release/murmelcraft`, or with a mesh:
`target/release/murmelcraft test-fixtures/channel/foehncraft.vtu`),
start `claude` in any folder and ask, for example, *"Use murmelcraft-live:
clip the mesh along x and show me a screenshot"*. Meshes the agent opens
must lie in `~/murmel-work` (or give it more folders with
`--read-root DIR`). For a session of its own without a window, add the
server without `--live` under another name:

```sh
claude mcp add --scope user murmelcraft -- \
  "$PWD/target/release/murmel" mcp --root ~/murmel-work
```

If it does not work:

- *`claude: command not found`*: Claude Code is not installed in this
  shell; install it, or open a new terminal after installing.
- *`MCP server murmelcraft-live already exists`*: remove the old entry
  first, `claude mcp remove murmelcraft-live --scope user` (or
  `--scope local` if it was added without `--scope`), and add it again.
- *Not listed in another folder*: without `--scope user` the server is
  added for the current folder only.
- *`✗ Failed to connect`*: the path is wrong or the program is not
  built; `ls "$PWD/target/release/murmel"` must show the file, and
  `target/release/murmel mcp --live --root ~/murmel-work` must print
  nothing and wait (Ctrl + C ends it). A root that does not exist stops
  it with a message.
- *`LIVE001`* in a call: no MurmelCraft window is open; *`LIVE003`* in
  Settings: a second window is open. Settings > Agents must be on.

More in [docs/mcp.md](docs/mcp.md).

Rules for contributors and agents: [AGENTS.md](AGENTS.md). Tools:
[docs/tools.md](docs/tools.md). MCP server: [docs/mcp.md](docs/mcp.md). Lessons: [docs/pitfalls.md](docs/pitfalls.md).

MIT licence. Fonts Inter and JetBrains Mono (SIL OFL 1.1), icons Lucide
(ISC); see [assets/PROVENANCE.md](assets/PROVENANCE.md).
