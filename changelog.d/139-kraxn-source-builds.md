- **Kraxn builds from the source.** When a release has no file for your
  computer, or for FoehnCraft's MPI and GPU backends, Kraxn builds the
  release from its source: it checks Rust, the compilers, CMake and MPI,
  installs the missing ones (rustup, apt or dnf, Homebrew, winget) as
  steps you see first, downloads the tag's source, runs cargo with its
  output shown and installs the programs like a release's
  ([docs/kraxn.md](docs/kraxn.md), "Build from the source").
