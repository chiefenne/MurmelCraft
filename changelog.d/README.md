# Changelog fragments

Each pull request adds its changelog entry here as a file of its own,
`changelog.d/<PR number>-<short-name>.md`, instead of editing
`CHANGELOG.md`, so that parallel pull requests never conflict (AGENTS.md).

- Content: the bullets as they will stand in `CHANGELOG.md` (plain
  sentences, a bold lead where it helps). No heading.
- A release pull request moves every fragment into a new version section
  of `CHANGELOG.md`, grouped by topic under a short summary, adds its row to
  the summary table, and deletes the fragments. The Release workflow then
  tags `main` (see `docs/release.md`).
