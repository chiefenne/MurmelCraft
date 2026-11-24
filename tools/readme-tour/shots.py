"""The eight README shots: each sets up the open MurmelCraft window through
the live MCP bridge (`murmel mcp --live`), then disconnects, so the
"Agent connected" badge is gone when the window is captured.

    python3 -I shots.py MURMEL SHOT

MURMEL is the `murmel` binary, SHOT one of the names in SHOTS. The window
must have FoehnCraft's fields.vtkhdf of the Manifold case open (see
README.md). Coordinates are metres (the case scales KoflCraft's mm by 0.001).
"""
import json
import subprocess
import sys


class Mcp:
    def __init__(self, murmel):
        self.p = subprocess.Popen([murmel, "mcp", "--live", "--root", "/"], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, text=True, bufsize=1)
        self.n = 0
        self.rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                "clientInfo": {"name": "readme-tour", "version": "1"}})
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, msg):
        self.p.stdin.write(json.dumps(msg) + "\n")
        self.p.stdin.flush()

    def rpc(self, method, params):
        self.n += 1
        self.send({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params})
        while True:
            m = json.loads(self.p.stdout.readline())
            if m.get("id") == self.n:
                if "error" in m:
                    raise RuntimeError(f"{method}: {m['error']}")
                return m["result"]

    def __call__(self, tool, args=None):
        r = self.rpc("tools/call", {"name": tool, "arguments": args or {}})
        if r.get("isError"):
            raise RuntimeError(f"{tool} {args}: {r['content'][0]['text'][:500]}")

    def close(self):
        self.p.stdin.close()
        self.p.wait()


def reset(m):
    """Back to the plain mesh; never calls a settings tool (agents cannot
    save settings, SET001 would show in the status bar)."""
    for tool, args in [("slice.clear", {"all": True}), ("iso.clear", {"all": True}), ("arrows.clear", {}),
                       ("clip.clear", {"all": True}), ("region.clear", {"all": True}),
                       ("measure.remove", {"all": True}), ("select.clear", {}),
                       ("display.visibility", {"action": "show_all"}),
                       ("display.render_mode", {"mode": "shaded_edges"}), ("display.color_by", {"by": "solid"})]:
        m(tool, args)


def fit(m, yaw, pitch, zoom=1.0, ndc=(0.0, 0.0)):
    m("view.set_camera", {"yaw_deg": yaw, "pitch_deg": pitch, "roll_deg": 0})
    m("view.fit", {})
    if zoom != 1.0:
        m("view.zoom", {"factor": zoom, "cursor_ndc": list(ndc)})


def cam(m, yaw, pitch, distance, target):
    m("view.set_camera", {"yaw_deg": yaw, "pitch_deg": pitch, "roll_deg": 0, "distance": distance,
                          "target": target})


def field(m, name, **style):
    m("display.color_by", {"by": "field", "field": name})
    m("field.style", {"field": name, "mode": "smooth", "lines": False, **style})


def mesh(m):  # dark: the mesh as it opens, shaded with edges
    fit(m, -50, 30, 0.8)


def cell_types_clip(m):  # dark: cell types, flat clip with its handles
    m("display.color_by", {"by": "cell_type"})
    m("clip.set", {"new": True, "axis": "z", "origin": [0.13, 0.2, 0.04], "mode": "flat"})
    fit(m, -35, 50, 0.6, (0.1, 0.15))


def jet_slice(m):
    m("display.render_mode", {"mode": "shaded"})
    m("slice.set", {"new": True, "axis": "z", "origin": [0.13, 0.2, 0.0]})


def vorticity_slice(m):  # dark: vorticity on a slice, mesh as outline
    field(m, "vorticity_magnitude", map="inferno", range="fixed", min=0.0, max=12.0)
    jet_slice(m)
    fit(m, -20, 62, 0.85, (0.05, 0.05))


def speed_arrows(m):  # dark: speed on the slice with in-plane arrows
    field(m, "speed", map="viridis")
    jet_slice(m)
    m("arrows.set", {"field": "velocity", "density": 24, "in_plane": True, "color": "solid", "scale": 3.0})
    m("display.set_color", {"item": "arrows", "color": "#F4F4F4"})
    m("select.clear", {})
    fit(m, -20, 62, 0.85, (0.05, 0.05))
    m("view.zoom", {"factor": 0.5, "cursor_ndc": [0.45, -0.4]})


def slices_iso(m):  # dark: a series of four slices and an iso-surface
    field(m, "speed", map="viridis")
    m("display.render_mode", {"mode": "shaded"})
    m("slice.set", {"new": True, "axis": "x", "origin": [0.25, 0.2, 0.02], "count": 4, "spacing": 0.05})
    m("iso.set", {"new": True, "field": "speed", "value": 0.05, "color": "solid"})
    m("select.clear", {})
    cam(m, -60, 30, 0.95, [0.24, 0.22, 0.02])


def region(m):  # dark: a sphere region of interest, cut flat
    field(m, "speed", map="turbo", mode="flat")
    m("region.set", {"new": True, "shape": "sphere", "center": [0.27, 0.3, 0.0], "radius": 0.045,
                     "cut": "flat"})
    m("select.clear", {})
    cam(m, -40, 28, 0.36, [0.27, 0.3, 0.0])


def measure_probe(m):  # dark: wall shear stress, two distances, a probe
    field(m, "wall_shear_stress_magnitude", map="plasma", range="fixed", min=0.0, max=0.0015)
    m("display.render_mode", {"mode": "shaded"})
    m("measure.add", {"kind": "distance", "name": "Inlet diameter",
                      "points": [{"point": [-0.025, 0.0, 0.0]}, {"point": [0.025, 0.0, 0.0]}]})
    m("measure.add", {"kind": "distance", "name": "Inlet to outlet",
                      "points": [{"point": [0.0, 0.0, 0.0]}, {"point": [-0.12, 0.36, 0.04]}]})
    m("probe.add", {"at": {"point": [0.0, 0.08, 0.0]}, "name": "Inlet core"})
    m("select.item", {"item": "probe:1"})
    fit(m, -50, 30, 0.8)
    m("view.pan", {"dx_px": -45, "dy_px": 10})


def light_pressure(m):  # light: banded pressure with contour lines
    field(m, "pressure", mode="banded", bands=14, lines=True, line_count=13, map="cool_warm",
          range="fixed", min=0.0, max=0.014)
    m("display.render_mode", {"mode": "shaded"})
    m("select.clear", {})
    fit(m, -50, 38, 0.82)
    m("view.pan", {"dx_px": -85, "dy_px": 10})


SHOTS = {f.__name__: f for f in [mesh, cell_types_clip, vorticity_slice, speed_arrows, slices_iso, region,
                                 measure_probe, light_pressure]}

if __name__ == "__main__":
    m = Mcp(sys.argv[1])
    reset(m)
    SHOTS[sys.argv[2]](m)
    m.close()
