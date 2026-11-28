# Rules for agents and contributors

The standing rules of MurmelCraft, the shared desktop viewer of
[KoflCraft](https://github.com/chiefenne/KoflCraft) (mesher) and
[FoehnCraft](https://github.com/chiefenne/Foehncraft) (solver). Written for
coding agents first; they apply to every contributor. **MUST** rules are
never broken; **SHOULD** rules are followed unless the pull request says why
not. Lessons from earlier work are in [docs/pitfalls.md](docs/pitfalls.md).
The method is the ArtCraft Craft Anything method of KoflCraft and FoehnCraft:
**Study → Plan → Build → Check → Approve**.

## Scope and approval

- **MUST** put the plan of a feature on a decision for the maintainer before
  building it; every choice that changes the goal is the maintainer's.
- **MUST** ask before any step a new commit cannot undo: publishing,
  releasing, deleting a branch or tag, a full CI run.
- **MUST** keep every function reachable without a window: scriptable through
  `murmel call`, JSON in and out, no interactive-only paths (headless first).

## One engine, thin front ends

- **MUST** add every action a front end performs as a tool of the registry
  (`crates/murmel-tools`, ids `area.verb`). A click, a key, a menu item, the
  CLI and the MCP server (`murmel mcp`, [docs/mcp.md](docs/mcp.md)) all call
  the same tool id; panels read the session and never change it directly.
- **MUST** keep the layers (a test checks them): `murmel-mesh` (readers, no
  GPU) → `murmel-engine` (session, camera, bindings, tokens, no window, no
  GPU) → `murmel-render` (wgpu, no egui) → `murmel-tools` (registry) →
  front ends `murmel-cli`, `murmel-mcp` and `murmel-app` (egui + eframe).
- **MUST** name model items by the ids the tree and the tools share:
  `mesh`, `zone:<name>`, `kind:<cell type>`, `patch:<name>`
  (`murmel_engine::Item`). A change of display state (colour, visibility,
  selection) goes through `Session::redraw`, which rebuilds the surface.
- **MUST** mark a tool that writes files `write-files` and name what it
  writes (`outputs`), so an approval table can judge a call before it runs,
  and name the input fields that are paths it reads (`reads`); the MCP
  server confines exactly these fields to its roots (a test checks fields
  named like paths).
- **MUST** give a tool that changes nothing the `NOT_JOURNALED` reason
  `READS_ONLY` and no other tool that reason: MCP clients are told such a
  tool is read-only and idempotent (a test checks it changes nothing).
- **MUST** regenerate `docs/tools.md` with
  `cargo run -p murmel-cli -- commands --markdown > docs/tools.md` (a test
  fails when it is out of date).
- **MUST** give every new tool a command in `crates/murmel-tools/src/commands.rs`
  (a key or a palette entry), or list it in `NOT_IN_PALETTE` with the reason,
  and regenerate `docs/keys.md` with
  `cargo run -p murmel-cli -- keys --markdown > docs/keys.md` (tests check
  both, and that no key is bound twice).
- **SHOULD** put a new command that people use often in a menu
  (`crates/murmel-tools/src/menus.rs`, a curated choice: every command
  stays in the palette) and regenerate `docs/menus.md` with
  `cargo run -p murmel-cli -- menus --markdown > docs/menus.md` (a test
  checks that it is up to date and that every entry names a command once).
- **MUST** give every new command or tool a help page: a Markdown file in
  `assets/help/en/` whose `covers` names its id (a prefix ending in `*`
  is fine), listed in `pages!` (`crates/murmel-engine/src/help.rs`), with
  `help-<page>-title` and `help-<page>-line` in the Fluent file (tests
  check all three). A new control takes its tip from there with
  `self.tip(resp, id)` or `self.tip_as(…)` (`crates/murmel-app/src/tips.rs`)
  instead of `on_hover_text`.
- **SHOULD** give a page whose topic is easier seen than read a picture:
  `picture = "<page>.png"` and `shot = "shots/<page>.json"` in its header,
  the recipe a list of tool calls ending in `export.image` without a path,
  then `cargo run -p murmel-cli -- help shots <page>` (needs a GPU or
  lavapipe) and the file in `pictures!` (`tips.rs`). Never draw or paste a
  picture by hand; tests check the recipe's digest and the list.
- **MUST** decide for every new tool whether its calls are undo steps:
  list it in `JOURNALED` (it changes the camera or the display) or in
  `NOT_JOURNALED` with the reason (`crates/murmel-tools/src/tools.rs`; a
  test checks). The selection, settings and files are never undone.
- **MUST** take every visible string from a translation key in
  `assets/i18n/en/murmel.ftl` (a test checks the keys the window uses).
- **MUST** give every command a key or a palette entry, and every widget an
  accessible name (AccessKit): icon-only buttons set `widget_info`, and no
  name or label text holds an icon glyph (paint glyphs beside text). A new
  region offers its first control to F6 (`Focus::offer`); the window test
  (`crates/murmel-app/src/a11y_tests.rs`) checks names and F6
  ([docs/accessibility.md](docs/accessibility.md)).
- **MUST** take every colour from `assets/tokens.toml`; no colour literal in
  code.

## Correctness

- **MUST NOT** present a partial or failed result as success in the CLI, the
  window or the docs. Say what was checked and how.
- **MUST NOT** add `unsafe` (forbidden in the workspace). **SHOULD NOT** add
  `unwrap`, `expect` or `panic!` outside tests; return a diagnostic with a
  registered code instead (`MESH001`, `VIEW001`, …, listed by `diag.codes`).
- **MUST** check counts a file declares against the file size before
  allocating (KoflCraft SW-17), and add the input of every reader failure or
  crash to the broken-input corpus (`murmel-mesh` tests).

## Clean room and licences

- **MUST NOT** use, read or imitate SolidCraft or FieldCraft code, UI kit,
  theme, icons or look (licence-restricted). They were analysed only for
  their backends (egui/eframe on wgpu). Our look comes from our own tokens.
- **MUST NOT** use Qt (licence).
- **MUST** use only open-licence fonts and icons (Inter and JetBrains Mono,
  SIL OFL; Lucide, ISC) and record every asset's source and licence in
  [assets/PROVENANCE.md](assets/PROVENANCE.md), every fixture's in
  [test-fixtures/README.md](test-fixtures/README.md).
- **MUST** build algorithms from papers and documented formats, preferring
  the computer graphics literature (Crane, Lévy, Kobbelt), and cite them in
  the code or `docs/`.
- **MUST NOT** read copyleft (GPL, LGPL, AGPL) or weak-copyleft (MPL, EPL,
  CDDL) source; **MAY** read permissively licensed source but **MUST NOT**
  copy, port or translate it. **MUST** review the licence of every new crate
  (no GPL, AGPL or commercial runtime dependency).

## Tests

- **MUST** follow the keep rule: each test names its invariant, a change that
  breaks it, and why no other test catches it.
- **MUST** add a failing test with every bug fix.
- **MUST NOT** delete, weaken, ignore or skip a test to get green.
- **MUST** check a change to what the viewport shows with a render test
  (`crates/murmel-render/tests/render.rs`). To accept an intended change of
  the picture, run `MURMEL_BLESS=1 cargo test -p murmel-render --test render`
  and say why in the pull request; never raise the tolerance to get green.

## Pull requests

- **MUST** keep one issue per pull request; moving code and changing
  behaviour go in separate pull requests.
- **MUST** fill in "What ran and what did not, and why"; a check that did not
  run is never reported as passed.
- **MUST** write a changelog fragment `changelog.d/<PR number>-<short-name>.md`.
- **SHOULD** squash-merge when CI is green; merged branches are deleted.

## Checks to run before a pull request

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --locked --workspace --all-targets
```

The render tests need a GPU or a software driver; on a headless Linux
machine install Mesa lavapipe (`mesa-vulkan-drivers`).

CI (`.github/workflows/ci.yml`) makes the fast run on every pull request and
push to main: format, clippy and the tests, with the Rust cache and
cancel-in-progress. It runs the tests with cargo-nextest (every test binary
at once, each test in its own process, settings in `.config/nextest.toml`),
builds without debug info, and takes Mesa from a cache. `cargo test` builds
murmel-mesh and murmel-engine with `opt-level = 1` (Cargo.toml), here as in
CI; `cargo nextest run --workspace --all-targets` runs them faster still if
nextest is installed.
A test must not depend on another test running in the same process. The full run adds tests marked
`#[ignore = "full run: …"]`; it starts by hand (`workflow_dispatch`) or with
the label `full-ci`, only after asking the maintainer. Mark a test as a
full-run test when it takes more than about a minute; keep a fast test of
the same code path in the fast run.

## This repository

| Path | What |
|---|---|
| `crates/murmel-mesh` | `.vtu` reader (VTK polyhedra 42, `region`, zone and boundary names and roles), OpenFOAM `polyMesh` reader (ascii, binary, gzip, `cellZones`), our own HDF5 reader and the VTKHDF reader of FoehnCraft results (`hdf5.rs`, `vtkhdf.rs`), field data (`field.rs`), `mesh-evidence.json`, mesh model and summary, synthetic meshes of any size for benchmarks (`synth.rs`), work split over threads (`par.rs`, scoped threads of the standard library), parallel parsing of number arrays and base64 (`numbers.rs`), the phases of an opening for `murmel bench` and the window's progress and Cancel (`profile.rs`, `MESH019`) |
| `crates/murmel-engine` | session, tool journal (undo and redo), camera (turntable with roll, three projection modes, 26 cube views, rotation about screen axes), view cube geometry, navigation bindings and presets, tokens and user colours, settings files, session files (`.murmel`, save and load), triangulated surface and feature edges, legend model, the motions of movies (`animation.rs`), field values, ranges and styles (`field.rs`, node values from least-squares cell gradients, `gradient.rs`), derived fields (`derive.rs`: vorticity, Q, λ₂, total pressure, pressure coefficient), the field calculator (`calc.rs`: your formulas, parsed by recursive descent, evaluated on all threads) and colour maps (`colormap.rs`, `assets/colormaps.toml`), slices and iso-surfaces (`extract.rs`, marching tetrahedra), vector arrows (`glyph.rs`), streamlines (`stream.rs`, traced through the cells' tetrahedra of `tets.rs`) |
| `crates/murmel-render` | wgpu renderer for the window and offscreen PNGs (transparent background, wide lines), overlays of exported pictures (legend, axis triad; vello_cpu), render tests |
| `crates/murmel-tools` | tool registry, result envelope, diagnostic codes, `docs/tools.md`; command table (keys and palette), `docs/keys.md`; picture and movie export (`export.rs`, `movie.rs`: PNG frames, GIF, MP4 through the system ffmpeg; [docs/export.md](docs/export.md)); the benchmark of large meshes (`bench.rs`, `murmel bench`, [docs/performance.md](docs/performance.md)) |
| `crates/murmel-cli` | `murmel`, the command line of the registry; `murmel mcp` starts the MCP server |
| `crates/murmel-mcp` | the MCP server (stdio, MCP 2025-11-25): the registry for agents on one headless session, paths confined to read and write roots, screenshots returned as images ([docs/mcp.md](docs/mcp.md)) |
| `crates/murmel-app` | `murmelcraft`, the egui + eframe window: panels, viewport, view cube, command palette, shortcut sheet, keyboard focus (F6 regions, focus ring), large meshes read on their own thread with progress and Cancel (`opening.rs`), window tests with egui_kittest |
| `crates/kraxn-core` | Kraxn, the installer of MurmelCraft, KoflCraft and FoehnCraft, without a window: catalogue (`assets/kraxn/catalogue.toml`), folders per system and scope, release lists (GitHub or a folder), downloads with the SHA-256 check, unpacking, install steps, versions side by side, removal, its tool registry ([docs/kraxn.md](docs/kraxn.md), [docs/kraxn-tools.md](docs/kraxn-tools.md)); none of the viewer's crates (a test checks) |
| `crates/kraxn` | `kraxn`: the text mode, plan files, `kraxn call`, and the steps that need administrator rights (`kraxn admin-run`) |
| `assets/` | tokens, fonts, icons, translations, with provenance |
| `test-fixtures/` | KoflCraft meshes and OpenFOAM polyMesh cases, with provenance |

The UI spec is the maintainer's document "MurmelCraft mesh viewer UI spec";
its milestones (V0.1 … V2) order the work.
