"""Build the README slideshow from the window captures.

    python3 -I make_webp.py SHOTS_DIR OUT_DIR

Reads SHOTS_DIR/01-*.png ... (2x window captures, in name order) and writes
OUT_DIR/tour.webp (1280 x 800, 4 s per slide, a 0.3 s crossfade, looping)
and OUT_DIR/tour/NN-name.jpg (1440 x 900 stills).
Needs Pillow with WebP support.
"""
import pathlib
import sys

from PIL import Image

HOLD_MS, FADE_FRAMES, FADE_MS, QUALITY = 4000, 5, 60, 70

src, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
(out / "tour").mkdir(parents=True, exist_ok=True)
shots = sorted(src.glob("[0-9][0-9]-*.png"))
caps = [Image.open(p).convert("RGB") for p in shots]
for p, im in zip(shots, caps):
    im.resize((1440, 900), Image.LANCZOS).save(out / "tour" / (p.stem + ".jpg"), quality=88, optimize=True,
                                               progressive=True, subsampling=0)
slides = [im.resize((1280, 800), Image.LANCZOS) for im in caps]
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
