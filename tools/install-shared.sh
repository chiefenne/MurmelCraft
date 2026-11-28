#!/usr/bin/env bash
# Shared (system-wide) install on Linux, for example as root under
# /opt/MurmelCraft/<version>/, so that every user of the machine (or every
# machine that mounts the folder) runs the same release.
#
#   /opt/MurmelCraft/0.6.0/bin/        the programs
#   /opt/MurmelCraft/0.6.0/src/        the source tree (--with-source)
#   /opt/MurmelCraft/0.6.0/env.sh      for bash/zsh users: source it
#   /opt/MurmelCraft/0.6.0/env.csh     for csh/tcsh users: source it
#   /opt/MurmelCraft/modulefiles/murmelcraft/0.6.0   module file (Environment Modules, Lmod)
#   /opt/MurmelCraft/current -> 0.6.0  the default version
#   /opt/MurmelCraft/use.sh            defines murmelcraft-use (bash, zsh)
#
# Several versions live side by side (0.5.0/, 0.6.0/, ...). Users switch in
# their shell with `murmelcraft-use 0.5.0` (after `source /opt/MurmelCraft/use.sh`)
# or `module load murmelcraft/0.5.0`; `murmelcraft-use` alone lists them. The
# admin moves the default with `install-shared.sh --activate 0.5.0`.
#
# Usage (see docs/install.md, "Shared install on Linux"):
#   sudo bash install-shared.sh                       # latest release
#   sudo bash install-shared.sh --version 0.6.0 --prefix /opt/MurmelCraft
#   sudo bash install-shared.sh --build               # build from source (cargo on PATH)
#   sudo bash install-shared.sh --activate 0.5.0      # make 0.5.0 the default
#   bash install-shared.sh --list                     # installed versions
#   bash install-shared.sh --dry-run                  # print the steps only
#
# The same script, with other settings in the block below, is in the
# Foehncraft and KoflCraft repositories.
set -euo pipefail

# ---- per-project settings ---------------------------------------------------
NAME=murmelcraft                     # archive and module name
TITLE=MurmelCraft                    # folder name under /opt
REPO=chiefenne/MurmelCraft
CHECK="murmel help"                  # smoke test after the install
BUILD_PATHS="crates/murmel-app crates/murmel-cli crates/murmel-mcp"  # cargo install --path ... for --build
BUILD_FEATURES=""                    # cargo features with --features
PY_PROJECT=""                        # no Python package
PY_DEFAULT=none                      # MurmelCraft needs no Python (venv | system | none)
PIP_PACKAGES=""
APT_PACKAGES=""
DNF_PACKAGES=""
# Libraries the pip gmsh module loads on Linux (else: OSError libGLU.so.1).
APT_GMSH_LIBS="libglu1-mesa libxcursor1 libxft2 libxinerama1 libgomp1"
DNF_GMSH_LIBS="mesa-libGLU libXcursor libXft libXinerama libgomp"
# Libraries the programs themselves need at run time.
APT_RUNTIME="mesa-vulkan-drivers libvulkan1"   # the GPU (wgpu) through Vulkan
DNF_RUNTIME="mesa-vulkan-drivers vulkan-loader"
# ----------------------------------------------------------------------------

VERSION=latest
PREFIX="/opt/$TITLE"
PYTHON_MODE="$PY_DEFAULT"
PYTHON_EXE=python3
WITH_SOURCE=0
BUILD=0
FEATURES="$BUILD_FEATURES"
TARBALL=""
SET_CURRENT=1
SYSTEM_DEPS=1
DRY=0
FORCE=0
ACTION=install
NATIVE=0
INSTALL_RUST=0

usage() {
  sed -n '2,/^set -euo/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'
  cat <<EOF
Options:
  --version V         release to install, e.g. 1.2.0 or v1.2.0 (default: latest)
  --prefix DIR        install root (default: /opt/$TITLE)
  --python MODE       venv | system | none (default: $PY_DEFAULT)
                        venv:   DIR/<version>/venv with pip: $PIP_PACKAGES
                        system: distribution packages (apt or dnf), never pip
  --python-exe EXE    the Python that makes the venv (default: python3)
  --with-source       clone the release tag into DIR/<version>/src (examples/, tools/)
  --build             build from source with cargo instead of the release archive
                      (needs Rust, a C/C++ compiler and CMake; implies --with-source)
  --features LIST     cargo features for --build, e.g. mpi
  --native            with --build: optimise for this machine's CPU (-C target-cpu=native);
                      the programs may then not run on other CPUs
  --install-rust      with --build: if cargo is missing, install Rust system-wide into
                      DIR/rust (RUSTUP_HOME and CARGO_HOME there, not in root's home)
  --tarball FILE      install from a downloaded release archive (offline)
  --no-current        install next to the others, keep the default (DIR/current) as it is
  --activate V        only make the installed version V the default (DIR/current)
  --list              only list the installed versions
  --no-system-deps    do not run apt-get/dnf (only print what is missing)
  --force             replace an existing DIR/<version>
  --dry-run           print the steps, change nothing
  -h, --help          this text
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:?}"; shift 2 ;;
    --prefix) PREFIX="${2:?}"; shift 2 ;;
    --python) PYTHON_MODE="${2:?}"; shift 2 ;;
    --python-exe) PYTHON_EXE="${2:?}"; shift 2 ;;
    --with-source) WITH_SOURCE=1; shift ;;
    --build) BUILD=1; WITH_SOURCE=1; shift ;;
    --features) FEATURES="${2:?}"; shift 2 ;;
    --native) NATIVE=1; shift ;;
    --install-rust) INSTALL_RUST=1; shift ;;
    --tarball) TARBALL="${2:?}"; shift 2 ;;
    --no-current) SET_CURRENT=0; shift ;;
    --activate) ACTION=activate; VERSION="${2:?}"; shift 2 ;;
    --list) ACTION=list; shift ;;
    --no-system-deps) SYSTEM_DEPS=0; shift ;;
    --force) FORCE=1; shift ;;
    --dry-run) DRY=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "install-shared: unknown option $1 (see --help)" >&2; exit 2 ;;
  esac
done

case "$PYTHON_MODE" in venv|system|none) ;; *)
  echo "install-shared: --python must be venv, system or none" >&2; exit 2 ;;
esac
[ "$(uname -s)" = Linux ] || { echo "install-shared: this script is for Linux" >&2; exit 2; }

say() { printf '==> %s\n' "$*"; }
run() {
  printf '    $'; printf ' %q' "$@"; printf '\n'
  [ "$DRY" = 1 ] || "$@"
}
need() { command -v "$1" >/dev/null 2>&1 || { echo "install-shared: '$1' not found" >&2; exit 1; }; }

umask 022

list_versions() {
  local cur="" d v
  cur="$(readlink "$PREFIX/current" 2>/dev/null || true)"
  for d in "$PREFIX"/*/; do
    v="$(basename "$d")"
    case "$v" in current|modulefiles) continue ;; esac
    [ -f "$d/env.sh" ] || continue
    if [ "$v" = "$cur" ]; then echo "$v (default)"; else echo "$v"; fi
  done
}
write() { # file, content on stdin
  if [ "$DRY" = 1 ]; then echo "    write $1"; cat >/dev/null; else cat >"$1"; fi
}
set_default() { # version
  run mkdir -p "$PREFIX/modulefiles/$NAME"
  run ln -sfn "$1" "$PREFIX/current"
  write "$PREFIX/modulefiles/$NAME/.version" <<EOF
#%Module1.0
set ModulesVersion "$1"
EOF
}

if [ "$ACTION" = list ]; then
  list_versions; exit 0
fi
if [ "$ACTION" = activate ]; then
  VERSION="${VERSION#v}"
  [ -f "$PREFIX/$VERSION/env.sh" ] || {
    echo "install-shared: $TITLE $VERSION is not installed in $PREFIX; installed:" >&2
    list_versions >&2; exit 1; }
  set_default "$VERSION"
  say "default is now $TITLE $VERSION (new shells, 'module load $NAME', '${NAME}-use default')"
  exit 0
fi

need curl; need tar; need sha256sum

# ---- version ----------------------------------------------------------------
if [ -n "$TARBALL" ] && [ "$VERSION" = latest ]; then
  base="$(basename "$TARBALL")"; VERSION="${base#"$NAME"-}"; VERSION="${VERSION%-linux-*}"
fi
if [ "$VERSION" = latest ]; then
  VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)"
  [ -n "$VERSION" ] || { echo "install-shared: could not read the latest release of $REPO" >&2; exit 1; }
fi
VERSION="${VERSION#v}"
TAG="v$VERSION"
DEST="$PREFIX/$VERSION"
say "$TITLE $VERSION -> $DEST (python: $PYTHON_MODE$([ "$BUILD" = 1 ] && echo ', build from source'))"

if [ -e "$DEST/bin" ] && [ "$FORCE" = 0 ]; then
  echo "install-shared: $DEST exists already (use --force to replace it)" >&2; exit 1
fi

arch="$(uname -m)"
if [ "$BUILD" = 0 ] && [ -z "$TARBALL" ] && [ "$arch" != x86_64 ]; then
  echo "install-shared: release archives are built for x86_64 only; use --build on $arch" >&2; exit 1
fi

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
run mkdir -p "$DEST/bin"

# ---- programs ---------------------------------------------------------------
if [ "$WITH_SOURCE" = 1 ]; then
  need git
  say "source tree $TAG"
  [ "$FORCE" = 1 ] && [ -d "$DEST/src" ] && run rm -rf "$DEST/src"
  run git clone --quiet --depth 1 --branch "$TAG" "https://github.com/$REPO.git" "$DEST/src"
fi

if [ "$BUILD" = 1 ]; then
  # A system-wide Rust (from an earlier --install-rust) comes first.
  if [ -x "$PREFIX/rust/cargo/bin/cargo" ]; then
    export RUSTUP_HOME="$PREFIX/rust/rustup" CARGO_HOME="$PREFIX/rust/cargo"
    export PATH="$CARGO_HOME/bin:$PATH"
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    if [ "$INSTALL_RUST" = 1 ]; then
      say "Rust system-wide into $PREFIX/rust"
      export RUSTUP_HOME="$PREFIX/rust/rustup" CARGO_HOME="$PREFIX/rust/cargo"
      run curl -fsSL --proto '=https' --tlsv1.2 -o "$WORK/rustup-init.sh" https://sh.rustup.rs
      run sh "$WORK/rustup-init.sh" -y --no-modify-path --profile minimal --default-toolchain stable
      export PATH="$CARGO_HOME/bin:$PATH"
      run chmod -R a+rX "$PREFIX/rust"
    else
      echo "install-shared: cargo not found. Add --install-rust (Rust into $PREFIX/rust) or" >&2
      echo "  install Rust yourself (docs/install.md, 'Build from source into /opt')." >&2
      exit 1
    fi
  fi
  [ "$DRY" = 1 ] || need cargo
  say "build with $(cargo --version 2>/dev/null || echo cargo)"
  [ "$NATIVE" = 1 ] && export RUSTFLAGS="${RUSTFLAGS:-} -C target-cpu=native"
  feat=(); [ -n "$FEATURES" ] && feat=(--features "$FEATURES")
  for p in $BUILD_PATHS; do
    run env -C "$DEST/src" CARGO_TARGET_DIR="$WORK/target" cargo install --locked --root "$DEST" \
      --path "$DEST/src/$p" "${feat[@]}"
  done
  run rm -f "$DEST/.crates.toml" "$DEST/.crates2.json"
  [ "$DRY" = 1 ] || cp "$DEST/src/README.md" "$DEST/src/LICENSE" "$DEST/" 2>/dev/null || true
else
  file="$NAME-$VERSION-linux-x86_64.tar.gz"
  if [ -n "$TARBALL" ]; then
    archive="$TARBALL"
    sums="$(dirname "$TARBALL")/SHA256SUMS.txt"
  else
    url="https://github.com/$REPO/releases/download/$TAG"
    say "download $url/$file"
    archive="$WORK/$file"; sums="$WORK/SHA256SUMS.txt"
    run curl -fL --proto '=https' --tlsv1.2 -o "$archive" "$url/$file"
    run curl -fsSL --proto '=https' --tlsv1.2 -o "$sums" "$url/SHA256SUMS.txt"
  fi
  if [ "$DRY" = 0 ]; then
    if [ -f "$sums" ]; then
      want="$(awk -v f="$file" '$2 == f || $2 == "*" f {print $1}' "$sums")"
      [ -n "$want" ] || { echo "install-shared: $file is not listed in SHA256SUMS.txt" >&2; exit 1; }
      have="$(sha256sum "$archive" | awk '{print $1}')"
      [ "$want" = "$have" ] || { echo "install-shared: checksum of $file does not match" >&2; exit 1; }
      say "checksum ok"
    else
      say "no SHA256SUMS.txt next to the archive: checksum not checked"
    fi
  fi
  run tar -xzf "$archive" -C "$WORK"
  if [ "$DRY" = 0 ]; then
    top="$WORK/$NAME-$VERSION-linux-x86_64"
    for f in "$top"/*; do
      case "$(basename "$f")" in
        README.md|LICENSE) cp "$f" "$DEST/" ;;
        *) install -m 0755 "$f" "$DEST/bin/" ;;
      esac
    done
  fi
fi

# ---- Python -----------------------------------------------------------------
pkg_tool=""
command -v apt-get >/dev/null 2>&1 && pkg_tool=apt
[ -z "$pkg_tool" ] && command -v dnf >/dev/null 2>&1 && pkg_tool=dnf
sys_install() { # packages...
  [ $# -gt 0 ] || return 0
  if [ "$SYSTEM_DEPS" = 1 ] && [ "$(id -u)" = 0 ] && [ -n "$pkg_tool" ]; then
    case "$pkg_tool" in
      apt) run env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$@" ;;
      dnf) run dnf install -y "$@" ;;
    esac || say "could not install $* (run '$pkg_tool update' first, or install them by hand); going on"
  else
    say "install these system packages (as root): ${pkg_tool:-your package manager} install $*"
  fi
}

# shellcheck disable=SC2086
case "$pkg_tool" in
  apt) sys_install $APT_RUNTIME ;;
  dnf) sys_install $DNF_RUNTIME ;;
esac

case "$PYTHON_MODE" in
  venv)
    need "$PYTHON_EXE"
    say "Python venv $DEST/venv"
    if [ "$pkg_tool" = apt ]; then
      # Debian and Ubuntu ship venv without pip unless python3-venv is installed.
      "$PYTHON_EXE" -m venv "$WORK/probe" >/dev/null 2>&1 || sys_install python3-venv
    fi
    case " $PIP_PACKAGES " in *" gmsh "*)
      [ "$pkg_tool" = apt ] && sys_install $APT_GMSH_LIBS
      [ "$pkg_tool" = dnf ] && sys_install $DNF_GMSH_LIBS ;;
    esac
    [ "$FORCE" = 1 ] && [ -d "$DEST/venv" ] && run rm -rf "$DEST/venv"
    run "$PYTHON_EXE" -m venv "$DEST/venv"
    run "$DEST/venv/bin/python" -m pip install --quiet --upgrade pip
    # shellcheck disable=SC2086
    [ -n "$PIP_PACKAGES" ] && run "$DEST/venv/bin/python" -m pip install --quiet $PIP_PACKAGES
    ;;
  system)
    say "Python from the distribution (no pip)"
    # shellcheck disable=SC2086
    case "$pkg_tool" in
      apt) sys_install $APT_PACKAGES ;;
      dnf) sys_install $DNF_PACKAGES ;;
      *) say "no apt-get or dnf: install the Python packages $PIP_PACKAGES with your package manager" ;;
    esac
    ;;
  none) ;;
esac
if [ "$BUILD" = 1 ] && [ "$PYTHON_MODE" = venv ] && [ -n "$PY_PROJECT" ] \
   && { [ "$DRY" = 1 ] || [ -f "$DEST/src/$PY_PROJECT/pyproject.toml" ]; }; then
  say "Python package from $DEST/src/$PY_PROJECT (builds its Rust module)"
  run env CARGO_TARGET_DIR="$WORK/target-py" "$DEST/venv/bin/python" -m pip install --quiet "$DEST/src/$PY_PROJECT"
fi

# ---- environment files ------------------------------------------------------
venv_sh=""; venv_csh=""; venv_mod=""
if [ "$PYTHON_MODE" = venv ]; then
  venv_sh="\$${TITLE^^}_HOME/venv/bin:"
  venv_csh="\${${TITLE^^}_HOME}/venv/bin:"
  venv_mod="prepend-path PATH \$root/venv/bin"
fi
home_var="${TITLE^^}_HOME"
src_sh=""; src_csh=""; src_mod=""
if [ "$WITH_SOURCE" = 1 ]; then
  src_sh="export ${TITLE^^}_SRC=\"\$$home_var/src\""
  src_csh="setenv ${TITLE^^}_SRC \"\${$home_var}/src\""
  src_mod="setenv ${TITLE^^}_SRC \$root/src"
fi

say "environment files"
write "$DEST/env.sh" <<EOF
# $TITLE $VERSION for bash and zsh: source $DEST/env.sh
# (or $PREFIX/current/env.sh for the version the 'current' link points to)
# Takes every other $TITLE version under $PREFIX off PATH first, so that
# sourcing another version's env.sh switches cleanly.
_fc_p=":\$PATH:"
for _fc_d in "$PREFIX"/*/bin "$PREFIX"/*/venv/bin; do _fc_p="\${_fc_p//:\$_fc_d:/:}"; done
_fc_p="\${_fc_p#:}"; PATH="\${_fc_p%:}"; unset _fc_p _fc_d
export $home_var="$DEST"
export PATH="\$$home_var/bin:$venv_sh\$PATH"
$src_sh
EOF
write "$DEST/env.csh" <<EOF
# $TITLE $VERSION for csh and tcsh: source $DEST/env.csh
setenv $home_var "$DEST"
setenv PATH "\${$home_var}/bin:$venv_csh\${PATH}"
$src_csh
EOF
run mkdir -p "$PREFIX/modulefiles/$NAME"
write "$PREFIX/modulefiles/$NAME/$VERSION" <<EOF
#%Module1.0
## $TITLE $VERSION (Environment Modules and Lmod)
## module use $PREFIX/modulefiles ; module load $NAME/$VERSION
proc ModulesHelp { } { puts stderr "$TITLE $VERSION in $DEST" }
module-whatis "$TITLE $VERSION"
conflict $NAME
set root $DEST
setenv $home_var \$root
prepend-path PATH \$root/bin
$venv_mod
$src_mod
EOF

fn="${NAME}-use"
write "$PREFIX/use.sh" <<EOF
# $TITLE version switch for bash and zsh. Source it once (for everybody:
# ln -s $PREFIX/use.sh /etc/profile.d/$NAME.sh); it switches on the default
# version unless one is on already, and defines:
#   $fn            list the installed versions
#   $fn 1.1.0      switch this shell to 1.1.0
#   $fn default    switch to the default ($PREFIX/current)
$fn() {
  _fc_root="$PREFIX"
  if [ \$# -eq 0 ] || [ "\$1" = list ]; then
    _fc_cur="\$(readlink "\$_fc_root/current" 2>/dev/null)"
    for _fc_d in "\$_fc_root"/*/env.sh; do
      _fc_v="\$(basename "\$(dirname "\$_fc_d")")"
      [ "\$_fc_v" = current ] && continue
      _fc_m=""
      [ "\$_fc_v" = "\$_fc_cur" ] && _fc_m=" (default)"
      [ "\$_fc_root/\$_fc_v" = "\${${home_var}:-}" ] && _fc_m="\$_fc_m (active)"
      echo "\$_fc_v\$_fc_m"
    done
    unset _fc_root _fc_cur _fc_d _fc_v _fc_m
    return 0
  fi
  _fc_v="\${1#v}"; [ "\$_fc_v" = default ] && _fc_v=current
  if [ ! -f "\$_fc_root/\$_fc_v/env.sh" ]; then
    echo "$fn: \$1 is not installed; installed:" >&2; $fn list >&2
    unset _fc_root _fc_v; return 1
  fi
  . "\$_fc_root/\$_fc_v/env.sh"
  unset _fc_root _fc_v
  echo "$TITLE \$(basename "\$${home_var}") (\$${home_var})"
}
if [ -z "\${${home_var}:-}" ] && [ -f "$PREFIX/current/env.sh" ]; then
  . "$PREFIX/current/env.sh"
fi
EOF

if [ "$SET_CURRENT" = 1 ] || [ ! -e "$PREFIX/current" ]; then
  set_default "$VERSION"
fi

run chmod -R a+rX "$DEST" "$PREFIX/modulefiles" "$PREFIX/use.sh"

# ---- check ------------------------------------------------------------------
say "check: $CHECK"
if [ "$DRY" = 0 ]; then
  # shellcheck disable=SC1091
  ( . "$DEST/env.sh"; $CHECK > /dev/null ) && say "check ok" || {
    echo "install-shared: '$CHECK' failed. If the message names GLIBC, this Linux is older than the" >&2
    echo "  release build supports: install with --build instead (docs/install.md)." >&2
    exit 1; }
  gmsh_py=0
  case "$PYTHON_MODE: $PIP_PACKAGES $APT_PACKAGES " in
    "venv:"*" gmsh "*|"system:"*" python3-gmsh "*) gmsh_py=1 ;;
  esac
  if [ "$gmsh_py" = 1 ]; then
    py="$PYTHON_EXE"; [ "$PYTHON_MODE" = venv ] && py="$DEST/venv/bin/python"
    "$py" -c 'import gmsh; gmsh.initialize(); print("gmsh", gmsh.__version__); gmsh.finalize()' \
      || say "Python gmsh does not import yet (see the messages above)"
  fi
fi

cat <<EOF

Done. Installed: $(list_versions | tr '\n' ' ')
Users switch $TITLE on with one of:
  source $PREFIX/use.sh; $fn $VERSION      # bash, zsh ($fn alone lists the versions)
  source $PREFIX/$VERSION/env.csh            # csh, tcsh
  module use $PREFIX/modulefiles; module load $NAME/$VERSION   # Environment Modules, Lmod
For everybody at login: ln -s $PREFIX/use.sh /etc/profile.d/$NAME.sh
Default for new shells: sudo bash install-shared.sh --prefix $PREFIX --activate VERSION
EOF
