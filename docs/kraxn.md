# Kraxn, the installer

Kraxn installs MurmelCraft, KoflCraft and FoehnCraft on macOS, Linux and
Windows, for you alone or for every user of the computer, from the GitHub
releases or from a folder of downloaded files, with several versions side
by side. *Kraxn* is Tyrolean for the carrying frame on your back: it
carries the three up the hill.

This page describes the core, the window, the text mode and source
builds. Extras (menu entries, file types, Python and Gmsh, ffmpeg,
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

## Build from the source

When a release has no file for your computer (an older Linux, a system
the releases do not cover yet), or you want FoehnCraft's MPI or GPU
backend, Kraxn builds the release from its source: choose "Build from
the source" for a program in the window, answer `s` in the text mode, or
write `from = "source"` in a plan.

Kraxn first checks the tools the build needs and lists them on the steps
page:

| Tool | Needed by | When it is missing, Kraxn… |
|---|---|---|
| Rust (at least the product's `min_rust`) | all three | updates it with `rustup`, or downloads `rustup-init` (checked against its `.sha256`) and installs Rust into your `~/.cargo` |
| C compiler and linker (and C++ for FoehnCraft) | all three | Linux: `apt-get install build-essential` or `dnf install gcc gcc-c++ make` (administrator); Windows: the Visual Studio C++ Build Tools with `winget`; macOS: asks you to run `xcode-select --install` |
| CMake 3.10 or newer | FoehnCraft (HDF5 and METIS) | `apt`/`dnf`, `brew install cmake`, `winget install Kitware.CMake` |
| Open MPI and libclang | FoehnCraft with `mpi` | `apt`/`dnf` (`libopenmpi-dev openmpi-bin libclang-dev` or `openmpi-devel clang-devel`), `brew install open-mpi`; not on Windows |

The steps that install tools show before anything runs, like every other
step; a tool only you can install (Apple's Command Line Tools, or a Linux
without `apt` or `dnf`) stops the plan with `KRX015` and a note what to
do, and "Check again" looks once more. Then Kraxn downloads GitHub's
archive of the release's tag, runs `cargo install --locked` for each of
the product's programs (with `--features`, and `-C target-cpu=native`
when you ask for a build tuned for this computer), shows cargo's output
as it comes and puts the programs into place like a release's. The
build runs as you; only the copy into system folders asks for
administrator rights. The receipt (`kraxn.toml`) says `source = "build"`
with the tag, the features and `rustc --version`. A build needs the
internet (cargo downloads the crates), so it does not combine with a
folder of downloads. The build folder (several GB for FoehnCraft) is in
Kraxn's temporary work folder and is removed when Kraxn ends.

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
# from = "source"           # build here instead of the release file
# features = ["mpi"]        # with from = "source": mpi, cuda
# native = true             # with from = "source": for this CPU only

[[install]]
product = "koflcraft"
```

`kraxn apply plan.toml` shows the steps and asks; `--yes` runs without the
question (for scripts), `--dry-run` only shows them. An unknown field
stops the plan, so a typo never changes an install silently.

## Tools

Every action is a tool of the registry, JSON in and out, the same for the
text mode, the window and `kraxn call`: [kraxn-tools.md](kraxn-tools.md).
