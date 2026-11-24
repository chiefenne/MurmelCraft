# The README slideshow

`assets/readme/tour.webp` (the slideshow near the top of the README) and
its stills (assets/readme/tour/) are screenshots of
the real window, made on Linux without a screen. To make them again, or
to make the same for another project:

1. **The data.** KoflCraft (`cargo build --release -p mesh-cli`):

   ```sh
   c=test-fixtures/manifold
   target/release/mesh-cli generate-duct $c/manifold.stl $c/patches.json $c/partition.json polyhedral manifold-mesh
   ```

   FoehnCraft (`cargo build --release -p foehncraft-cli`) on
   `manifold-mesh/foehncraft.vtu` with [manifold.toml](manifold.toml):
   `foehn flow manifold.toml --out run` (about 3 minutes on 4 cores) writes
   `run/fields.vtkhdf`.

2. **The tools.** `apt install xvfb xdotool imagemagick mesa-vulkan-drivers
   libxkbcommon-x11-0`, Python 3 with Pillow (WebP support), and
   `cargo build --release -p murmel-app -p murmel-cli`.

3. **The shots.** `tools/readme-tour/make-tour.sh run/fields.vtkhdf`
   writes `assets/readme/tour.webp` and `assets/readme/tour/*.png` (about
   4 minutes).

How it works:

- [make-tour.sh](make-tour.sh) starts Xvfb (3200 x 2000) and the window
  with `WINIT_X11_SCALE_FACTOR=2`, so the 1440 x 900 window renders at
  2880 x 1800; the theme comes from a `settings.toml` in a fresh
  `MURMEL_CONFIG_DIR`, one window per theme.
- [shots.py](shots.py) sets up each shot through the live MCP bridge
  (`murmel mcp --live`, JSON-RPC on stdin and stdout) with the same tools
  as the window's controls, then disconnects; `import -window` captures
  the window after the pointer has moved off it.
- [make_webp.py](make_webp.py) scales the captures to 1760 x 1100 (twice
  the 880 pixels the README shows, so Retina screens draw text sharp) and
  writes an animated WebP with Pillow: each slide holds 4 s, then five
  blended frames of 60 ms crossfade to the next (smoothstep), quality 82,
  method 6, looping; about 2.4 MB for five slides. The stills are the
  2880 x 1800 captures as lossless PNG.
- Only the slides named in [approved.txt](approved.txt) go into the
  slideshow and the stills: the user approves every slide before it is
  public. The other captures stay in the script's work folder for review.
- The README shows it with `<img src="assets/readme/tour.webp" width="880">`.

What bit us:

- Text looked soft in the first version: its frames were 1280 wide and
  JPEG and WebP at quality 70-88, so a Retina screen scaled them up. Keep
  the frames at twice the shown width and the stills lossless.
- Every slide needs the maintainer's approval before it goes into the
  README.

- An agent cannot save settings (`SET001`, shown in the status bar until
  the next message); the theme is set through `settings.toml` instead.
- A failed tool call leaves its diagnostic in the status bar: restart the
  window before the final captures.
- The "Agent connected" badge shows while the bridge is connected, and the
  hover tooltip shows where the pointer rests: capture after the
  disconnect, with the pointer outside the window.
- Slices, arrows and region cuts finish on worker threads: wait (12 s on
  lavapipe) before capturing.
- GIF would cut the colour maps to 256 colours; GitHub plays animated
  WebP in an `<img>`, as FoehnCraft's README does.
