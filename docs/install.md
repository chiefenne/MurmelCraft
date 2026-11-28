# Install and update

MurmelCraft is built from source; release downloads are planned. This page
lists every tool to install on macOS, Linux and Windows, with the commands,
so that a fresh machine gets to a working `murmelcraft` window in one pass.

Contents: [What you need](#what-you-need) · [macOS](#macos) ·
[Linux](#linux) · [Windows](#windows) · [Build](#build) ·
[Optional: ffmpeg and Foehncraft](#optional-ffmpeg-and-foehncraft) ·
[Update](#update)

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
lists them, for example `git checkout v0.6.1`).

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
