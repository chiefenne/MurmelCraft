# Provenance of the assets

Everything here is ours or under an open licence that allows shipping it;
no SolidCraft or FieldCraft asset, kit, theme or icon is used (clean room).

| File | Source | Licence | How it was made |
|---|---|---|---|
| `fonts/Inter-Regular.ttf`, `fonts/Inter-SemiBold.ttf` | Inter 4.1, github.com/rsms/inter release `Inter-4.1.zip`, `extras/ttf/` | SIL OFL 1.1 (`fonts/Inter-LICENSE.txt`) | `pyftsubset` to U+0020-007E, U+00A0-017F, U+2000-206F, U+2190-21FF, U+2212, U+00B0, U+00B5, U+2026, U+2248-2265 with `kern,tnum,liga` |
| `fonts/JetBrainsMono-Regular.ttf` | JetBrains Mono 2.304, github.com/JetBrains/JetBrainsMono release, `fonts/ttf/` | SIL OFL 1.1 (`fonts/JetBrainsMono-OFL.txt`) | `pyftsubset`, same ranges, `kern` |
| `icons/lucide-subset.ttf` | `lucide-static` 0.460.0 (npm), `font/lucide.ttf` | ISC (`icons/Lucide-LICENSE.txt`) | `pyftsubset` to the glyphs folder-open, scan, house, box, square, sun, moon, monitor, settings, search, circle-alert, rotate-3d, move, zoom-in, eye, eye-off, layers, info (codepoints in `crates/murmel-app/src/style.rs`) |
| `tokens.toml` | ours (UI spec, design tokens) | MIT | patch palette computed in OKLCH, see the file |
| `i18n/en/murmel.ftl` | ours | MIT | |

The logo is a plain placeholder (a rose square with an M) until the
maintainer supplies the marmot logo.
