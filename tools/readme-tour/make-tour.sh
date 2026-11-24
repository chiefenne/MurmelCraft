#!/usr/bin/env bash
# Render the README slideshow (assets/readme/tour.webp and the stills in
# assets/readme/tour/) from a FoehnCraft result, on Linux without a screen.
# Needs Xvfb, xdotool, ImageMagick (import), Mesa lavapipe, Python 3 with
# Pillow, and release builds of murmelcraft and murmel. See README.md.
#
#   tools/readme-tour/make-tour.sh RESULT.vtkhdf [OUT_DIR]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
result=$(realpath "$1")
out=${2:-$repo/assets/readme}
work=$(mktemp -d)
export DISPLAY=:97 XDG_RUNTIME_DIR=$work/run MURMEL_CONFIG_DIR=$work/config WINIT_X11_SCALE_FACTOR=2
mkdir -p -m 700 "$XDG_RUNTIME_DIR" && mkdir -p "$MURMEL_CONFIG_DIR" "$work/shots"
Xvfb $DISPLAY -screen 0 3200x2000x24 -nolisten tcp & xvfb=$!
trap 'kill $xvfb 2>/dev/null; pkill -f "murmelcraft $result" || true' EXIT
sleep 2

shoot() { # theme, then pairs of "NN-file shot"
  local theme=$1; shift
  printf 'format = "murmelcraft.settings"\ntheme = "%s"\n' "$theme" > "$MURMEL_CONFIG_DIR/settings.toml"
  "$repo/target/release/murmelcraft" "$result" > "$work/window-$theme.log" 2>&1 & local app=$!
  sleep 10   # the window opens the result and takes its SHA-256
  local win; win=$(xdotool search --name MurmelCraft | head -1)
  while [ $# -gt 0 ]; do
    python3 -I "$here/shots.py" "$repo/target/release/murmel" "$2"
    xdotool mousemove 3150 1950   # the pointer off the window: no hover tooltip
    sleep 12                      # slices, arrows and cuts finish on worker threads
    import -window "$win" "$work/shots/$1.png"
    shift 2
  done
  kill $app; wait $app 2>/dev/null || true
}

shoot dark 01-mesh mesh 02-cell-types-clip cell_types_clip 03-vorticity-slice vorticity_slice \
  04-speed-arrows speed_arrows 05-slices-iso-surface slices_iso 06-region-of-interest region \
  07-measure-probe measure_probe
shoot light 08-light-theme-pressure light_pressure
python3 -I "$here/make_webp.py" "$work/shots" "$out"
