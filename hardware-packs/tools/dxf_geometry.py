#!/usr/bin/env python3
"""A small reader for the ASCII DXF files that board vendors publish as "dimensions source file" (no third-party library).

It knows only what the board packs need: LWPOLYLINE (with the tiny two-vertex bulge-1 polylines that CAD tools use for circles),
SOLID and TEXT entities with their layers, and from them

* the straight edges of the BOARD_OUTLINE layer (the board outline is their bounding rectangle),
* the through-hole header pads (oblong pads drawn as two circles on PADLAYER_TOP),
* the TEXT labels (silkscreen names, the drawing's own dimension texts),
* the extents of a layer, and
* `quote()`: a short, verifiable excerpt of the raw file for a layer, which the evidence of a pack can quote (hcp-manager reads a `.dxf` as plain
  text, so the excerpt can be found again by `hcp-manager evidence verify`).

Run it on a file to print what it finds:   python3 dxf_geometry.py FILE.dxf
"""
import math
import pathlib
import re
import sys


class Dxf:
    def __init__(self, path):
        self.path = pathlib.Path(path)
        self.raw = self.path.read_text(encoding="latin-1")
        lines = self.raw.split("\n")
        pairs = [(lines[i].strip(), lines[i + 1].rstrip("\r")) for i in range(0, len(lines) - 1, 2)]
        self.entities = []
        sec, cur = None, None
        for code, val in pairs:
            if code == "0" and val == "SECTION":
                sec = "?"
                continue
            if code == "2" and sec == "?":
                sec = val.strip()
                continue
            if code == "0" and val == "ENDSEC":
                sec = None
                continue
            if sec == "ENTITIES":
                if code == "0":
                    cur = {"type": val.strip(), "data": []}
                    self.entities.append(cur)
                elif cur is not None:
                    cur["data"].append((code, val.strip()))

    # -- entities --------------------------------------------------------------------------------------------------------------

    @staticmethod
    def layer(e):
        return next((v for c, v in e["data"] if c == "8"), "")

    @staticmethod
    def verts(e):
        v, x = [], None
        for c, val in e["data"]:
            if c == "10":
                x = float(val)
            elif c == "20" and x is not None:
                v.append((x, float(val)))
                x = None
        return v

    @staticmethod
    def bulges(e):
        return [float(val) for c, val in e["data"] if c == "42"]

    def polylines(self, layer):
        return [e for e in self.entities if e["type"] == "LWPOLYLINE" and self.layer(e) == layer]

    # -- what the packs need ---------------------------------------------------------------------------------------------------

    def outline(self):
        """The bounding rectangle (x0, y0, x1, y1) of the straight edges of BOARD_OUTLINE (the corner arcs and the line-end caps are inside it)."""
        pts = []
        for e in self.polylines("BOARD_OUTLINE"):
            v = self.verts(e)
            if len(v) == 2 and not any(abs(b - 1.0) < 1e-9 for b in self.bulges(e)) and (abs(v[0][0] - v[1][0]) < 1e-9 or abs(v[0][1] - v[1][1]) < 1e-9):
                pts += v
        if not pts:
            raise ValueError("no straight edge on the layer BOARD_OUTLINE")
        xs, ys = [p[0] for p in pts], [p[1] for p in pts]
        return (min(xs), min(ys), max(xs), max(ys))

    def circles(self, layer, d_min=0.0, d_max=1e9):
        """Circles drawn as a two-vertex polyline with bulge 1 on `layer`: (cx, cy, diameter)."""
        out = []
        for e in self.polylines(layer):
            v = self.verts(e)
            b = self.bulges(e)
            if len(v) == 2 and b and all(abs(x - 1.0) < 1e-6 for x in b):
                d = math.dist(v[0], v[1])
                if d_min <= d <= d_max:
                    out.append(((v[0][0] + v[1][0]) / 2, (v[0][1] + v[1][1]) / 2, d))
        return out

    def header_pads(self, layer="PADLAYER_TOP"):
        """Oblong through-hole pads (two equal circles side by side): (cx, cy) of each, rounded to 0.001 mm, without duplicates."""
        circles = [(round(x, 3), round(y, 3), round(d, 3)) for x, y, d in self.circles(layer, 0.5, 2.0)]
        by_y = {}
        for x, y, d in set(circles):
            by_y.setdefault(y, []).append(x)
        pads = []
        for y, xs in by_y.items():
            xs = sorted(xs)
            # circles in pairs: the two ends of one oblong pad
            for a, b in zip(xs[0::2], xs[1::2]):
                pads.append((round((a + b) / 2, 3), y))
        return sorted(pads, key=lambda p: (round(p[0]), -p[1]))

    def texts(self, layer=None):
        out = []
        for e in self.entities:
            if e["type"] != "TEXT" or (layer and self.layer(e) != layer):
                continue
            d = {c: v for c, v in e["data"]}
            out.append({"text": d.get("1", ""), "x": float(d.get("10", "nan")), "y": float(d.get("20", "nan")), "layer": self.layer(e)})
        return out

    def extent(self, layer, x_range=(-1e9, 1e9), y_range=(-1e9, 1e9)):
        pts = [p for e in self.polylines(layer) for p in self.verts(e) if x_range[0] < p[0] < x_range[1] and y_range[0] < p[1] < y_range[1]]
        if not pts:
            return None
        return (min(p[0] for p in pts), min(p[1] for p in pts), max(p[0] for p in pts), max(p[1] for p in pts))

    def rect(self, layer, min_width):
        """The bounding box (x0, y0, x1, y1) of the first polyline on `layer` that is at least `min_width` wide."""
        for e in self.polylines(layer):
            v = self.verts(e)
            if len(v) >= 4:
                xs, ys = [p[0] for p in v], [p[1] for p in v]
                if max(xs) - min(xs) >= min_width:
                    return (min(xs), min(ys), max(xs), max(ys))
        return None

    # -- verifiable excerpts ---------------------------------------------------------------------------------------------------

    def quote(self, layer, nth=0, groups=12):
        """The raw excerpt, whitespace collapsed, that starts at the layer name of the `nth` entity on `layer` and runs for `groups` lines.
        hcp-manager reads a .dxf as text, so this excerpt is a quotation that `evidence verify` can find again."""
        pat = re.compile(r"(?m)^\s*8\s*\r?\n" + re.escape(layer) + r"\s*\r?$")
        ms = list(pat.finditer(self.raw))
        if len(ms) <= nth:
            raise ValueError(f"layer {layer} has no entity number {nth}")
        tail = self.raw[ms[nth].start():].split("\n")[: 2 + groups]
        return " ".join(" ".join(tail).split())

    def text_quote(self, text):
        """The raw excerpt of a TEXT entity whose content is `text`: the group `1` line and its value (e.g. `1 25.40mm`)."""
        m = re.search(r"(?m)^\s*1\s*\r?\n" + re.escape(text) + r"\s*\r?$", self.raw)
        if not m:
            raise ValueError(f"no TEXT entity with the content {text!r}")
        return " ".join(m.group(0).split())


if __name__ == "__main__":
    d = Dxf(sys.argv[1])
    print("outline", d.outline())
    pads = d.header_pads()
    print("header pads", len(pads), pads[:3], "...", pads[-3:])
    print("labels", len(d.texts("LAYNR3")))
    print("antenna outline", d.rect("TOP", 15))
    print("quote", d.quote("BOARD_OUTLINE", 0))
