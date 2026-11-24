"""Build the README slideshow from the window captures.

    python3 -I make_webp.py SHOTS_DIR OUT_DIR

Reads SHOTS_DIR/01-*.png ... (2x window captures, in name order) and writes
OUT_DIR/tour.webp (1760 x 1100, 4 s per slide, a 0.3 s crossfade, looping)
and OUT_DIR/tour/NN-name.png (the 2880 x 1800 captures, lossless).

The README shows the slideshow 880 pixels wide, so 1760 pixels are what a
Retina screen draws there: smaller frames are scaled up by the browser and
the window's text turns soft (the first version was 1280 wide).
Needs Pillow with WebP support.
"""
import pathlib
import sys

from PIL import Image

HOLD_MS, FADE_FRAMES, FADE_MS, QUALITY = 4000, 5, 60, 82
SLIDE = (1760, 1100)

src, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
(out / "tour").mkdir(parents=True, exist_ok=True)
shots = sorted(src.glob("[0-9][0-9]-*.png"))
caps = [Image.open(p).convert("RGB") for p in shots]
for p, im in zip(shots, caps):
    im.save(out / "tour" / (p.stem + ".png"), optimize=True)
slides = [im.resize(SLIDE, Image.LANCZOS) for im in caps]
frames, durations = [], []
for i, im in enumerate(slides):
    nxt = slides[(i + 1) % len(slides)]
    frames.append(im)
    durations.append(HOLD_MS)
    for k in range(1, FADE_FRAMES + 1):
        a = k / (FADE_FRAMES + 1)
        frames.append(Image.blend(im, nxt, a * a * (3 - 2 * a)))
        durations.append(FADE_MS)
frames[0].save(out / "tour.webp", save_all=True, append_images=frames[1:], duration=durations, loop=0,
               quality=QUALITY, method=6)
print(f"{out / 'tour.webp'}: {len(slides)} slides, {(out / 'tour.webp').stat().st_size / 1e6:.2f} MB")
