# Making a release

A release is the annotated Git tag `v<version>` on `main` plus a GitHub
release whose notes are that version's section of
[CHANGELOG.md](../CHANGELOG.md). Versions follow semantic versioning; before
1.0 a minor step (0.1 to 0.2) may change file formats and tool inputs.

The minor version follows the milestone of the UI spec that is done
(0.6.0 after V0.6), and 1.0.0 is the V1.0 milestone. 0.7.0 is the
maintainer's exception: a minor step released together with Foehncraft
1.3.0 and KoflCraft 0.3.0 while V1.0 is in progress.

1. In a pull request (branch `release-v<version>-changelog`):
   - set `version` in `[workspace.package]` of `Cargo.toml` and run
     `cargo build` so `Cargo.lock` follows;
   - move every fragment of `changelog.d/` into a new section
     `## <version> (YYYY-MM-DD) – <title>` of `CHANGELOG.md`: a paragraph
     "In one paragraph", the highlights, the known limits and all changes
     grouped by topic; delete the fragments (keep `changelog.d/README.md`);
   - add the version's row at the top of the `## Summary` table;
   - update the status block of the README.
2. Merge it when CI is green.
3. Run the **Release** workflow on `main` (Actions ▸ Release ▸ Run workflow,
   or through the GitHub API) with `version` = the number without the `v`,
   first with *dry run* checked (it prints the notes and the commit it would
   tag), then without. It refuses a version whose changelog section is
   missing or that differs from `Cargo.toml`.

To publish at a set time, give *publish_at* a UTC time up to 8.5 hours
ahead, for example `2026-11-28T09:00:00Z` (10:00 in Vienna in winter). The
workflow then creates the release as a draft at the current `main`, the
Executables job builds that commit and attaches the programs to the draft,
and the job *publish* waits until that second, pushes the annotated tag and
publishes the release. Cancel the run before then to call the release off;
the draft stays and can be deleted by hand. A second run waits behind a
running one, so cancel the first before starting it again.

Tags cannot be pushed from the cloud sessions; the workflow creates them.
Locally, after the release: `git checkout main && git pull --tags`.

## Executables

After it creates the release, the Release workflow starts **Executables**
(`.github/workflows/executables.yml`), which builds the programs and
attaches them to the release in about 15 minutes:

| File | Contents |
|---|---|
| `murmelcraft-<version>-macos-universal.dmg` | `MurmelCraft.app` (Apple silicon and Intel) with `murmel` and `murmel-mcp` inside it |
| `murmelcraft-<version>-macos-universal.tar.gz` | `murmelcraft`, `murmel`, `murmel-mcp` |
| `murmelcraft-<version>-linux-x86_64.tar.gz` | the same three programs |
| `murmelcraft-<version>-windows-x86_64.zip` | the same three programs (`.exe`) |
| `SHA256SUMS.txt` | the SHA-256 of each file |

To attach them to a release that exists already, run Executables by hand
with `tag` = `v<version>` and *upload* checked (unchecked, it only builds,
and the files are an artifact of the run). A pull request that changes
`executables.yml` builds everything without uploading. The Mac builds are
signed ad hoc only, not with an Apple Developer ID, so macOS asks once
before the first start (README, "Download a release").
