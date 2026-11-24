"""Write assets/colormaps.toml, the colour maps of field colouring.

Each map is a list of sRGB colours from the low end (0) to the high end
(1), at even steps unless the map gives their positions (`at`); the
viewer mixes neighbouring stops in sRGB, as the sources of the data do.
Sources and licences (assets/PROVENANCE.md):

- rainbow_desaturated (the default, maintainer's decision 2026-11-24,
  shared with FoehnCraft and KoflCraft): the eight RGB control points of
  ParaView's "Rainbow Desaturated" preset (Remoting/Views/ColorMaps.json,
  colour space RGB), Copyright (c) 2000 Kitware Inc., BSD 3-Clause
  (Rainbow-Desaturated-LICENSE.txt). The points are written as they are,
  with their positions; nothing is sampled.
- viridis, magma, inferno, plasma: the data of Nathaniel J. Smith,
  Stefan van der Walt and Eric Firing (github.com/BIDS/colormap), CC0 1.0,
  sampled from the 256-entry tables that matplotlib ships.
- turbo: Anton Mikhailov, Google LLC, 2019, Apache License 2.0, sampled
  from the 256-entry table that matplotlib ships.
- cool_warm: computed here from K. Moreland, "Diverging Color Maps for
  Scientific Visualization", ISVC 2009 (interpolation in Msh, the polar
  form of CIELAB, with white inserted at the middle), with the paper's
  end colours.
- rainbow: the classic piecewise-linear "jet" ramp (blue, cyan, yellow,
  red), r = 1.5 - |4t - 3| and so on, clamped.
- grey: black to white.

    python3 -I assets/colormaps.py
"""

import math
import os

import numpy as np
from matplotlib import colormaps as mpl

HERE = os.path.dirname(os.path.abspath(__file__))
STOPS = 86


def hexes(rgb):
    return ['"#%02X%02X%02X"' % tuple(int(round(min(max(c, 0.0), 1.0) * 255)) for c in p[:3])
            for p in rgb]


def sampled(name):
    # every third of the 256 table entries (0, 3, …, 255)
    table = mpl[name](np.arange(256))[:, :3]
    idx = np.arange(0, 256, 255 // (STOPS - 1))
    t = idx / 255
    stops = table[idx]
    # the largest error of mixing the stops against the full table
    x = np.arange(256) / 255
    mixed = np.stack([np.interp(x, t, stops[:, k]) for k in range(3)], axis=1)
    err = np.abs(mixed - table).max() * 255
    assert err < 1.0, (name, err)
    return stops


# --- Moreland's diverging map ---------------------------------------------

def to_linear(c):
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def to_srgb(c):
    return 12.92 * c if c <= 0.0031308 else 1.055 * c ** (1 / 2.4) - 0.055


M = np.array([[0.4124, 0.3576, 0.1805], [0.2126, 0.7152, 0.0722], [0.0193, 0.1192, 0.9505]])
WHITE = M @ np.ones(3)


def lab(rgb):
    xyz = M @ np.array([to_linear(c) for c in rgb])
    f = [(v ** (1 / 3) if v > 0.008856 else 7.787 * v + 16 / 116) for v in xyz / WHITE]
    return np.array([116 * f[1] - 16, 500 * (f[0] - f[1]), 200 * (f[1] - f[2])])


def rgb_of_lab(c):
    fy = (c[0] + 16) / 116
    fx, fz = fy + c[1] / 500, fy - c[2] / 200
    inv = lambda f: f ** 3 if f ** 3 > 0.008856 else (f - 16 / 116) / 7.787
    xyz = np.array([inv(fx), inv(fy), inv(fz)]) * WHITE
    return [to_srgb(min(max(v, 0.0), 1.0)) for v in np.linalg.solve(M, xyz)]


def msh(c):
    m = math.sqrt(sum(v * v for v in c))
    return np.array([m, math.acos(c[0] / m), math.atan2(c[2], c[1])])


def lab_of_msh(c):
    m, s, h = c
    return np.array([m * math.cos(s), m * math.sin(s) * math.cos(h), m * math.sin(s) * math.sin(h)])


def adjust_hue(sat, unsat_m):
    m, s, h = sat
    if m >= unsat_m - 0.1:
        return h
    spin = s * math.sqrt(unsat_m * unsat_m - m * m) / (m * math.sin(s))
    return h + spin if h > -math.pi / 3 else h - spin


def diverging(c1, c2, t):
    a, b = msh(lab(c1)), msh(lab(c2))
    if a[1] > 0.05 and b[1] > 0.05 and abs(a[2] - b[2]) > math.pi / 3:
        mid = max(a[0], b[0], 88.0)
        if t < 0.5:
            b, t = np.array([mid, 0.0, 0.0]), 2 * t
        else:
            a, t = np.array([mid, 0.0, 0.0]), 2 * t - 1
    if a[1] < 0.05 and b[1] > 0.05:
        a[2] = adjust_hue(b, a[0])
    elif b[1] < 0.05 and a[1] > 0.05:
        b[2] = adjust_hue(a, b[0])
    return rgb_of_lab(lab_of_msh((1 - t) * a + t * b))


def cool_warm():
    lo, hi = (0.230, 0.299, 0.754), (0.706, 0.016, 0.150)
    return [diverging(lo, hi, t) for t in np.linspace(0, 1, STOPS)]


# ParaView's "Rainbow Desaturated": (position, colour), interpolated in
# RGB. Every value of the source is a whole number of 255ths, so the
# colours below are exact.
RAINBOW_DESATURATED = [
    (0.0, "#4747DB"),
    (0.143, "#00005C"),
    (0.285, "#00FFFF"),
    (0.429, "#008000"),
    (0.571, "#FFFF00"),
    (0.714, "#FF6100"),
    (0.857, "#6B0000"),
    (1.0, "#E04D4D"),
]


def rainbow():
    c = lambda v: min(max(v, 0.0), 1.0)
    return [(c(1.5 - abs(4 * t - 3)), c(1.5 - abs(4 * t - 2)), c(1.5 - abs(4 * t - 1)))
            for t in np.linspace(0, 1, STOPS)]


MAPS = [
    ("rainbow_desaturated", "rainbow", RAINBOW_DESATURATED),
    ("inferno", "sequential", lambda: sampled("inferno")),
    ("cool_warm", "diverging", cool_warm),
    ("viridis", "sequential", lambda: sampled("viridis")),
    ("plasma", "sequential", lambda: sampled("plasma")),
    ("magma", "sequential", lambda: sampled("magma")),
    ("turbo", "rainbow", lambda: sampled("turbo")),
    ("rainbow", "rainbow", rainbow),
    ("grey", "sequential", lambda: [(t, t, t) for t in np.linspace(0, 1, STOPS)]),
]


def main():
    out = [
        "# Colour maps of field colouring, written by assets/colormaps.py; do",
        "# not edit by hand. Each map: its kind and its colours from the low",
        "# end to the high end (mixed in sRGB), at even steps unless `at`",
        "# gives their positions. Sources and licences: assets/PROVENANCE.md.",
        "# The first map is the default for one-sided ranges, the same in",
        "# FoehnCraft and KoflCraft; cool_warm for symmetric ranges",
        "# (maintainer's decisions, 2026-11-24).",
        "",
    ]
    for name, kind, make in MAPS:
        out.append(f"[{name}]")
        out.append(f'kind = "{kind}"')
        if isinstance(make, list):
            out.append("at = [" + ", ".join(repr(a) for a, _ in make) + "]")
            cols = [f'"{c}"' for _, c in make]
        else:
            cols = hexes(make())
        out.append("stops = [")
        for i in range(0, len(cols), 6):
            out.append("  " + ", ".join(cols[i:i + 6]) + ",")
        out.append("]")
        out.append("")
    with open(os.path.join(HERE, "colormaps.toml"), "w") as f:
        f.write("\n".join(out))


if __name__ == "__main__":
    main()
