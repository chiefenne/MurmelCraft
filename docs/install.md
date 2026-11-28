# Install and update

MurmelCraft releases have ready-to-run programs for Linux, macOS and Windows (README, "Download a release"); building from source gives the newest `main`. For an install under `/opt` that all users of a Linux machine share, with several versions side by side, see [Shared install on Linux](#shared-install-on-linux). This page
lists every tool to install on macOS, Linux and Windows, with the commands,
so that a fresh machine gets to a working `murmelcraft` window in one pass.

Contents: [What you need](#what-you-need) · [macOS](#macos) ·
[Linux](#linux) · [Windows](#windows) · [Build](#build) ·
[Optional: ffmpeg and Foehncraft](#optional-ffmpeg-and-foehncraft) ·
[Shared install on Linux](#shared-install-on-linux) · [Update](#update)

## What you need

| Tool | Needed for | Required |
|---|---|---|
| Rust 1.98 or newer (rustup) | building MurmelCraft | yes |
| C compiler and linker | linking the Rust programs | yes |
| Git | cloning the repository | yes |
| A GPU driver: Metal (macOS), Vulkan (Linux), DirectX 12 or Vulkan (Windows) | drawing the window and the images | yes |
| `ffmpeg` on `PATH` | MP4 movies | no |
| Foehncraft's `foehn` | fields only its models give | no |

MurmelCraft needs **no CMake, no HDF5 and no Gmsh**: it reads its result
formats (VTU, VTKHDF and the others) with its own code. Fonts, icons, help pages and translations are built into the
programs.

CI builds and tests on Ubuntu (`ubuntu-latest`, with Mesa's software Vulkan
`mesa-vulkan-drivers`); macOS is tested by hand, and Windows is not
checked yet.

## macOS

```
xcode-select --install                 # C compiler, linker and Git
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Open a new terminal, then `rustc --version`. It must show 1.98 or newer:
the repository's `rust-toolchain.toml` selects the stable channel, but
rustup does not update a stable it already has, so run
`rustup update stable` if it is older (older versions stop with
`use of unstable library feature atomic_from_mut`). macOS draws with Metal
and needs nothing else. Continue with [Build](#build).

## Linux

You need an X11 or Wayland desktop and a Vulkan driver. Most desktops have
both; the Mesa Vulkan drivers cover Intel and AMD GPUs and give a software
renderer (lavapipe) where there is no GPU. With an NVIDIA GPU, the
proprietary driver brings its own Vulkan.

```
# Ubuntu and Debian
sudo apt update
sudo apt install build-essential git curl mesa-vulkan-drivers
# Fedora, RHEL, Rocky and Alma
sudo dnf install gcc make git curl mesa-vulkan-drivers

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustc --version                        # 1.98 or newer; else: rustup update stable
```

To check the Vulkan driver: `sudo apt install vulkan-tools` (Fedora:
`vulkan-tools`), then `vulkaninfo --summary` lists the devices.

These commands install Rust for your own user. To install MurmelCraft once for all users of a machine, as root, see [Shared install on Linux](#shared-install-on-linux).

Continue with [Build](#build).

## Windows

In PowerShell, with `winget` (part of Windows 10 and 11):

```
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Rustlang.Rustup -e
winget install --id Git.Git -e
```

The first command installs the MSVC compiler and linker that Rust needs
(`rustup-init.exe` offers them too). Close PowerShell, open a new one, and
check `rustc --version` (1.98 or newer; else `rustup update stable`). With
[Chocolatey](https://chocolatey.org):
`choco install rustup.install git visualstudio2022buildtools visualstudio2022-workload-vctools`.
The graphics driver of Windows 10 and 11 already has DirectX 12. Windows is
not checked by CI yet; tell us if something fails.

## Build

On all three systems (PowerShell on Windows):

```
git clone https://github.com/chiefenne/MurmelCraft.git
cd MurmelCraft
cargo build --release --locked
target/release/murmelcraft test-fixtures/channel/foehncraft.vtu
```

The first build takes a few minutes. It builds `murmelcraft` (the window),
`murmel` (the command line; `murmel mcp` is the MCP server) and
`murmel-mcp` (the same server on its own) into `target/release/` (`.exe` on
Windows). To call them by name, install them into `~/.cargo/bin`, which
rustup puts on your `PATH`:

```
cargo install --locked --path crates/murmel-app   # murmelcraft
cargo install --locked --path crates/murmel-cli   # murmel
murmelcraft
```

To build a release instead of `main`, check out its tag first (`git tag`
lists them, for example `git checkout v0.7.1`).

## Optional: ffmpeg and Foehncraft

**ffmpeg** for MP4 movies (MurmelCraft uses H.264 when ffmpeg has it and
falls back to MPEG-4 otherwise):

| System | Command |
|---|---|
| macOS | `brew install ffmpeg` |
| Ubuntu, Debian | `sudo apt install ffmpeg` |
| Fedora | `sudo dnf install ffmpeg-free` (or `ffmpeg` from RPM Fusion) |
| Windows | `winget install --id Gyan.FFmpeg -e`, then a new PowerShell |

Check with `ffmpeg -version`.

**Foehncraft** for the fields only its models give: build it as in its
[install guide](https://github.com/chiefenne/Foehncraft/blob/main/docs/install.md)
(it needs CMake in addition) and put `foehn` on `PATH`
(`cargo install --locked --path crates/foehncraft-cli` in the Foehncraft
folder).

## Shared install on Linux

For a machine (or several that mount the same folder) where an
administrator installs MurmelCraft once, as root, under `/opt/MurmelCraft/<version>/`, and
every user runs it. Several versions stay installed side by side; each user
picks one. The layout is the same as for Foehncraft:

```
/opt/MurmelCraft/
  0.6.0/bin/                 murmelcraft, murmel, murmel-mcp
  0.6.0/src/                 the source tree of v0.6.0, optional
  0.6.0/env.sh, env.csh      switch this version on (bash/zsh, csh/tcsh)
  current -> 0.6.0           the default version
  use.sh                   defines the command murmelcraft-use (bash, zsh)
  modulefiles/murmelcraft/        module files (Environment Modules, Lmod)
  rust/                    only for a build from source with --install-rust
```

rustup installs Rust for one user only (into `~/.cargo` of whoever runs
it), so Rust installed as root is not found by other users. The release
programs need no Rust.

### With the install script

[`tools/install-shared.sh`](../tools/install-shared.sh), as root
(`--dry-run` prints the steps without changing anything, `--help` lists
the options):

```
curl -fsSLO https://raw.githubusercontent.com/chiefenne/MurmelCraft/main/tools/install-shared.sh
bash install-shared.sh --dry-run
bash install-shared.sh                        # the latest release
bash install-shared.sh --version 0.6.0          # a given release
```

It downloads `murmelcraft-<version>-linux-<x86_64|aarch64>.tar.gz` (for this machine) and `SHA256SUMS.txt`
from the release, checks the checksum, puts the programs into
`<version>/bin/` (`--tarball FILE` for a file you downloaded), writes
`env.sh`, `env.csh`, the module file and `use.sh`, makes the version the
default (`--no-current` keeps the old one), makes everything readable for
all users and runs `murmel help`. `--with-source` clones the tag into
`<version>/src/`; `--no-system-deps` leaves apt and dnf alone.

MurmelCraft needs no Python (`--python none` is the default). The script
installs the Vulkan driver the window needs (`mesa-vulkan-drivers
libvulkan1`, or `mesa-vulkan-drivers vulkan-loader` with dnf); NVIDIA
cards use the NVIDIA driver instead.

The release programs are built for x86-64 and, from the next release on, aarch64 (64-bit ARM), and need glibc 2.28 or newer
(RHEL, Rocky and Alma 8 and 9, Debian 11 to 13, Ubuntu 20.04 and newer);
v0.6.0 itself still needs a newer glibc (Debian 13, Ubuntu 24.04). On older
systems, or on ARM with an older release, build from source.

### Build from source into /opt

For an older Linux, ARM, or programs tuned for one machine's CPU; the
result has the same layout. Complete for Debian 13 (Ubuntu the same), as
root:

```
apt install build-essential git curl mesa-vulkan-drivers libvulkan1
curl -fsSLO https://raw.githubusercontent.com/chiefenne/MurmelCraft/main/tools/install-shared.sh
bash install-shared.sh --version 0.6.0 --build --install-rust --native
```

Fedora, RHEL, Rocky and Alma: `dnf install gcc make git curl mesa-vulkan-drivers vulkan-loader`, then the same.

- `--build` clones the tag into `<version>/src/` and runs
  `cargo install --locked --root /opt/MurmelCraft/<version>` for each program; the
  build files go to a temporary folder that is removed at the end.
- `--install-rust` installs Rust system-wide into `/opt/MurmelCraft/rust`
  (`RUSTUP_HOME` and `CARGO_HOME` there) when no `cargo` is on `PATH`;
  users never need it.
- `--native` optimises for the build machine's CPU (`-C target-cpu=native`);
  leave it out when different machines share the folder.

### Several versions side by side

Users switch in their shell:

```
source /opt/MurmelCraft/use.sh        # once per shell, or at login
murmelcraft-use                       # list: 0.5.0, 0.6.0 (default) (active)
murmelcraft-use 0.5.0                 # this shell now runs 0.5.0
murmelcraft-use default               # back to the default
```

or with Environment Modules or Lmod: `module use /opt/MurmelCraft/modulefiles`,
`module load murmelcraft/0.5.0`, `module switch murmelcraft/0.6.0`. csh and tcsh:
`source /opt/MurmelCraft/0.5.0/env.csh`.

The administrator, as root:

```
bash install-shared.sh --version 0.6.0 --no-current      # next to the others, default unchanged
bash install-shared.sh --list                           # installed versions
bash install-shared.sh --activate 0.6.0                   # make 0.6.0 the default
ln -s /opt/MurmelCraft/use.sh /etc/profile.d/murmelcraft.sh             # default version on at every login
```

A version is removed by deleting `/opt/MurmelCraft/<version>` and
`/opt/MurmelCraft/modulefiles/murmelcraft/<version>`.

## Update

```
git checkout main && git pull --ff-only
cargo build --release --locked
```

If you installed the programs with `cargo install`, run those commands
again.

## Problems on a first build or start

| Message | Cause and fix |
|---|---|
| `use of unstable library feature atomic_from_mut` | Rust is older than 1.98: `rustup update stable`. |
| `linker 'cc' not found` (Linux), `link.exe not found` (Windows) | No C toolchain: `build-essential` / `gcc`, or the Visual Studio C++ Build Tools. |
| `Install a Vulkan, Metal or DirectX 12 driver` | No usable GPU driver: on Linux `mesa-vulkan-drivers` (also for a machine without a GPU). |
| The window does not open over SSH | MurmelCraft needs a desktop session; `murmel call view.screenshot` (the command line) renders images without a window, with a Vulkan driver such as `mesa-vulkan-drivers`. |
