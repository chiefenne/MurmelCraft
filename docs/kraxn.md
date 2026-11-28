# Kraxn, the installer

Kraxn installs MurmelCraft, KoflCraft and FoehnCraft on macOS, Linux and
Windows, for you alone or for every user of the computer, from the GitHub
releases or from a folder of downloaded files, with several versions side
by side. *Kraxn* is Tyrolean for the carrying frame on your back: it
carries the three up the hill.

This page describes the core, the window and the text mode. Source
builds, extras (menu entries, file types, Python and Gmsh, ffmpeg,
examples) and the AI assistant set-up follow in the next pull requests.

## Start it

```sh
cargo run --release -p kraxn            # the window (the text mode without a desktop)
cargo run --release -p kraxn -- text    # the questions in this terminal
cargo run --release -p kraxn -- apply plan.toml
cargo run --release -p kraxn -- call installed.list --input '{"scope": "user"}'
```

The window walks through the same questions as the text mode, one page
each: for you or for every user (and the folder), the GitHub releases or a
folder of downloads, which programs and versions (a program with no file
for this computer says so), the PATH, then every step with those that
need administrator rights marked. Install runs them on a thread and shows
each step's result, the SHA-256 check and the start check. "Save as plan
file" writes the answers for `kraxn apply`. "Manage what is installed"
lists the versions for you or for every user, makes one the default and
removes one version or all of a program. The window follows the system's
dark or light theme (the sun and moon button switches it) and uses
MurmelCraft's design tokens; every control has a name for screen readers.

The text mode asks: what to do (install, change the default version, remove,
list), for whom, where from, which programs and versions, and whether to
put them on the PATH. It then shows every step, marks those that need
administrator rights, offers to save the answers as a plan file, and runs
on a yes. After the install it starts each product's check
(`murmel help`, `kofl commands`, `foehn info`) and says what came back.

## For whom, and where

| System | For you | For every user |
|---|---|---|
| macOS, Linux | `~/.local/opt/<Title>/<version>`, links in `~/.local/bin` | `/opt/<Title>/<version>`, links in `/usr/local/bin` |
| Windows | `%LOCALAPPDATA%\Programs\Kraxn\<Title>\<version>`, links in `...\Kraxn\bin` | `%ProgramFiles%\Kraxn\<Title>\<version>`, links in `...\Kraxn\bin` |

`<Title>` is `MurmelCraft`, `KoflCraft` or `Foehncraft`. Another root
(`prefix` in a plan, or the folder question) puts everything below it,
with the links in `<prefix>/bin`.

Each version folder holds `bin/` with the programs, `README.md`,
`LICENSE` and `kraxn.toml` (what Kraxn installed: the release file, its
SHA-256 and whether `SHA256SUMS.txt` confirmed it). On macOS and Linux it
also holds `env.sh` and `env.csh`, the product folder holds `current` (the
default version), `use.sh` (`<name>-use VERSION` switches a shell) and
module files for Environment Modules and Lmod: the files of
`tools/install-shared.sh`, so Kraxn and that script see each other's
versions. On Windows the default is in `default.txt` and the links are
hard links (no administrator rights or developer mode needed).

The links on the PATH point through `current`, so a new default needs no
new links. Kraxn puts the PATH folder on the PATH of new terminals when it
is not there yet: a marked block in `~/.profile` (and `~/.bashrc`,
`~/.zshrc` if you have them) on Linux, in `~/.zprofile` (and
`~/.bash_profile`) on macOS, the user's or the machine's `Path` on
Windows. Removing the last product takes the block away again.

## Administrator rights

Kraxn runs as you. Only the steps that write into system folders run with
administrator rights: Kraxn writes them to a file in a folder only you can
read and starts `kraxn admin-run` with the system's own prompt (`sudo` in
a terminal, `pkexec` on a Linux desktop, the macOS password dialog,
Windows UAC). Downloads, checks and unpacking never run elevated.

## Checks

Every file is checked against the release's `SHA256SUMS.txt` before
anything is copied. A file that does not match stops the install
(`KRX004`); a file no `SHA256SUMS.txt` lists stops it too (`KRX005`),
unless the plan says `require_checksums = false`, and then the report says
"not checked". Archives may hold only plain files below their one top
folder; a link or a path outside it is refused (`KRX006`).

## Without internet

Put the release archives of your system and their `SHA256SUMS.txt` in a
folder (or one folder per product or release, each with its own
`SHA256SUMS.txt`) and choose "A folder of downloaded release files", or
set `offline_dir` in the plan. Online, Kraxn uses the system's
certificates (company proxies with their own root certificate work) and
the proxy of `HTTPS_PROXY` or `ALL_PROXY`; `GITHUB_TOKEN` lifts GitHub's
limit of 60 requests an hour.

## Plan files

```toml
format = "kraxn.plan"
format_version = "1.0"
scope = "system"            # user (default) or system
# prefix = "/apps"          # another install root
# offline_dir = "/srv/kraxn"
path = true                 # put the programs on the PATH (default)
require_checksums = true    # default

[[install]]
product = "foehncraft"
version = "1.3.0"           # default: latest
make_default = true         # default
# replace = true            # install the same version again

[[install]]
product = "koflcraft"
```

`kraxn apply plan.toml` shows the steps and asks; `--yes` runs without the
question (for scripts), `--dry-run` only shows them. An unknown field
stops the plan, so a typo never changes an install silently.

## Tools

Every action is a tool of the registry, JSON in and out, the same for the
text mode, the window and `kraxn call`: [kraxn-tools.md](kraxn-tools.md).
