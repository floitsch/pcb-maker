#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Stage-0 cut census of a placed board (docs/experiments/cut-census.md,
Proposal B of the fundamental-approaches note): the room between parts in
track quanta, and each many-pin part's simultaneous escape as a max-flow.

    cuts.py measure --list UNITS.tsv --out DIR [--ring 2.5] [--jobs 4]
    cuts.py one BOARD.kicad_pcb [--ring 2.5]

A unit line is `<board>\\t<label>\\t<path to .kicad_pcb>[\\t<json of extras>]`.
Per unit the board is read with KiCad's pcbnew (cached as JSON under
`<out>/extract/`) and measured; `<out>/units.json` holds every unit's
measures.

The rules are the board's own default net class: track width `w`,
clearance `s`, via diameter, copper-to-edge clearance. A gap `g` between two
pieces of copper holds `k = floor((g - s) / (w + s))` tracks; the quanta
are `Q_k = k w + (k + 1) s` (at 0.2 / 0.2: 0.6, 1.0, 1.4 mm).

**Gaps (cuts).** On each outer copper layer, every pair of pads of two
different footprints whose copper lies within `Q_3 + w + s` (at most 2 mm)
and whose shortest connecting segment crosses no third pad (a visible pair:
nothing between them). Per cut: the gap, its track count `k` and its phase
`(g - s) / (w + s) - k` (0: just at a quantum, near 1: just below the
next). `wasted_01` is the brief's measure, the share of cuts within 0.1 mm
below a quantum `Q_k`, k >= 1; `wasted_q` the share in the top quarter of
the period (phase >= 0.75), comparable across rules; `tight_q` the share in
the bottom quarter (phase < 0.25: a gap just at a quantum).

**Escape flow.** Per footprint with at least ten copper pads (not a frame
part, whose pads' box covers a quarter of the board's): the pins
that must escape are its pads on a signal net (a net with a pad on another
footprint, not carried by a zone). Free space around the part (its pads'
box grown by `ring` + 1 mm) is triangulated on the copper outlines (pads of
every footprint, non-plated holes, the board edge) sampled every 0.05 mm;
the triangles inside copper or off the board are removed; two adjacent
free triangles are joined by an arc whose capacity is the tracks their
shared edge holds between the two pieces of copper it spans (the formula
above; at the edge the copper-to-edge clearance); the sink is every free
triangle outside the part's box grown by `ring`. `flow_planar` is the
max-flow from one unit per pin on the part's own layers (both outer layers
for a through-hole part); `flow_vias` adds, as a sink of one unit, each
free triangle inside the ring whose empty circle holds a via with its
clearance and whose centre is clear on the other outer layer. The
deficit is pins less flow. Triangle edges overstate a triangle's own
capacity (the diagonal capacity of Yan and Wong): the flow is an upper
bound and the deficit a lower bound.
"""

import argparse
import concurrent.futures
import json
import math
import os
import sys
from pathlib import Path

import numpy as np
from scipy.sparse import csr_matrix
from scipy.sparse.csgraph import maximum_flow
from scipy.spatial import Delaunay

NM = 1e-6
SAMPLE = 0.05
OUTER = ("F.Cu", "B.Cu")
MANY_PINS = 10
BIG = 1000


# --- extraction (pcbnew) -----------------------------------------------------------


def poly_set(polygons):
    """A SHAPE_POLY_SET's outlines as lists of [x, y] (mm)."""
    result = []
    for index in range(polygons.OutlineCount()):
        outline = polygons.Outline(index)
        result.append([[outline.CPoint(i).x * NM, outline.CPoint(i).y * NM] for i in range(outline.PointCount())])
    return result


def extract(path, overrides=None):
    """The board's pads per outer layer, holes, outline, zone nets and
    rules as a plain dict (mm). `overrides`: reference -> [x, y, angle]
    poses set before reading (a layout's kept moves undone)."""
    import pcbnew  # noqa: PLC0415  (only the extraction needs KiCad)

    board = pcbnew.LoadBoard(str(path))
    settings = board.GetDesignSettings()
    net_settings = settings.m_NetSettings
    default = net_settings.GetDefaultNetclass()
    classes = {"Default": [default.GetTrackWidth() * NM, default.GetClearance() * NM]}
    for name, netclass in net_settings.GetNetclasses().items():
        width = netclass.GetTrackWidth() * NM if netclass.HasTrackWidth() else classes["Default"][0]
        clearance = netclass.GetClearance() * NM if netclass.HasClearance() else classes["Default"][1]
        classes[str(name)] = [width, clearance]
    rules = {
        "track": default.GetTrackWidth() * NM,
        "clearance": default.GetClearance() * NM,
        "via": default.GetViaDiameter() * NM,
        "edge": settings.m_CopperEdgeClearance * NM,
        "classes": classes,
    }
    outlines = pcbnew.SHAPE_POLY_SET()
    board.GetBoardPolygonOutlines(outlines, True)
    rings = []
    for index in range(outlines.OutlineCount()):
        outline = outlines.Outline(index)
        rings.append([[outline.CPoint(i).x * NM, outline.CPoint(i).y * NM] for i in range(outline.PointCount())])
        for hole in range(outlines.HoleCount(index)):
            ring = outlines.Hole(index, hole)
            rings.append([[ring.CPoint(i).x * NM, ring.CPoint(i).y * NM] for i in range(ring.PointCount())])
    zone_nets = sorted({zone.GetNetname() for zone in board.Zones()
                        if not zone.GetIsRuleArea() and zone.GetNetname() and zone.IsOnCopperLayer()})
    layers = {"F.Cu": pcbnew.F_Cu, "B.Cu": pcbnew.B_Cu}
    overrides = overrides or {}
    footprints = []
    for footprint in board.GetFootprints():
        reference = footprint.GetReference()
        if reference in overrides:
            x, y, angle = overrides[reference]
            footprint.SetPosition(pcbnew.VECTOR2I(int(round(x / NM)), int(round(y / NM))))
            footprint.SetOrientationDegrees(angle)
        pads = []
        for pad in footprint.Pads():
            attribute = pad.GetAttribute()
            entry = {"number": pad.GetNumber(), "net": pad.GetNetname() or None,
                     "class": pad.GetNetClassName() if pad.GetNetname() else None,
                     "npth": attribute == pcbnew.PAD_ATTRIB_NPTH, "layers": {}}
            for name, layer in layers.items():
                if pad.IsOnLayer(layer) and attribute != pcbnew.PAD_ATTRIB_NPTH:
                    entry["layers"][name] = poly_set(pad.GetEffectivePolygon(layer, pcbnew.ERROR_INSIDE))
            if attribute == pcbnew.PAD_ATTRIB_NPTH or pad.HasHole():
                drill = pad.GetDrillSize()
                position = pad.GetPosition()
                entry["hole"] = [position.x * NM, position.y * NM, max(drill.x, drill.y) * NM / 2]
            pads.append(entry)
        position = footprint.GetPosition()
        footprints.append({"reference": reference, "back": footprint.IsFlipped(),
                           "at": [position.x * NM, position.y * NM, footprint.GetOrientationDegrees() % 360.0],
                           "pads": pads})
    return {"rules": rules, "outline": rings, "zone_nets": zone_nets,
            "copper_layers": board.GetCopperLayerCount(), "footprints": footprints}


def extracted(path, cache, overrides=None):
    cache = Path(cache)
    if cache.exists() and cache.stat().st_mtime >= Path(path).stat().st_mtime:
        return json.loads(cache.read_text())
    data = extract(path, overrides)
    cache.parent.mkdir(parents=True, exist_ok=True)
    cache.write_text(json.dumps(data))
    return data


# --- geometry ----------------------------------------------------------------------


def circle(x, y, r, n=24):
    angles = np.linspace(0, 2 * math.pi, n, endpoint=False)
    return np.stack([x + r * np.cos(angles), y + r * np.sin(angles)], axis=1)


def point_segments(points, a, b):
    """Distances (n, m) of n points from m segments a -> b, and the
    closest points on the segments (n, m, 2)."""
    ab = b - a
    length2 = np.maximum((ab * ab).sum(axis=1), 1e-18)
    t = ((points[:, None, :] - a[None, :, :]) * ab[None, :, :]).sum(axis=2) / length2[None, :]
    t = np.clip(t, 0.0, 1.0)
    closest = a[None, :, :] + t[:, :, None] * ab[None, :, :]
    return np.linalg.norm(points[:, None, :] - closest, axis=2), closest


def polygon_distance(pa, pb):
    """The distance between two polygons' outlines (they do not overlap)
    and the closest points on each."""
    ea, eb = np.roll(pa, -1, axis=0), np.roll(pb, -1, axis=0)
    d1, c1 = point_segments(pa, pb, eb)
    d2, c2 = point_segments(pb, pa, ea)
    i1 = np.unravel_index(np.argmin(d1), d1.shape)
    i2 = np.unravel_index(np.argmin(d2), d2.shape)
    if d1[i1] <= d2[i2]:
        return float(d1[i1]), pa[i1[0]], c1[i1]
    return float(d2[i2]), c2[i2], pb[i2[0]]


def segments_cross(p, q, a, b):
    """Whether segment p -> q properly crosses any of the segments a -> b."""
    def orient(u, v, w):
        return (v[..., 0] - u[..., 0]) * (w[..., 1] - u[..., 1]) - (v[..., 1] - u[..., 1]) * (w[..., 0] - u[..., 0])
    o1 = orient(p, q, a)
    o2 = orient(p, q, b)
    o3 = orient(a, b, p[None, :])
    o4 = orient(a, b, q[None, :])
    return bool(np.any((o1 * o2 < 0) & (o3 * o4 < 0)))


def inside(points, polygon):
    """Even-odd test of points (n, 2) against one ring (m, 2)."""
    x, y = points[:, 0:1], points[:, 1:2]
    a = polygon
    b = np.roll(polygon, -1, axis=0)
    ya, yb = a[None, :, 1], b[None, :, 1]
    straddle = (ya > y) != (yb > y)
    with np.errstate(divide="ignore", invalid="ignore"):
        cross_x = a[None, :, 0] + (y - ya) * (b[None, :, 0] - a[None, :, 0]) / (yb - ya)
    return (np.sum(straddle & (x < cross_x), axis=1) % 2) == 1


def inside_rings(points, rings):
    result = np.zeros(len(points), dtype=bool)
    for ring in rings:
        result ^= inside(points, ring)
    return result


def resample(polygon, step, closed=True):
    """Points along a polyline every `step` at most, its vertices kept."""
    points = []
    count = len(polygon)
    last = count if closed else count - 1
    for i in range(last):
        a, b = polygon[i], polygon[(i + 1) % count]
        n = max(1, int(math.ceil(np.linalg.norm(b - a) / step)))
        for j in range(n):
            points.append(a + (b - a) * (j / n))
    if not closed:
        points.append(polygon[-1])
    return np.array(points)


# --- the board as layers of copper ---------------------------------------------------


class Board:
    def __init__(self, data):
        self.data = data
        rules = data["rules"]
        self.w, self.s = rules["track"], rules["clearance"]
        self.via, self.edge = rules["via"], rules["edge"]
        self.pitch = self.w + self.s
        self.outline = [np.array(ring) for ring in data["outline"] if len(ring) >= 3]
        zone_nets = set(data["zone_nets"])
        footprints_of_net = {}
        for index, footprint in enumerate(data["footprints"]):
            for pad in footprint["pads"]:
                if pad["net"]:
                    footprints_of_net.setdefault(pad["net"], set()).add(index)
        # Per layer: (footprint, pad index, polygon) for every copper
        # outline and every hole (footprint -1 for a non-plated hole).
        self.layers = {name: [] for name in OUTER}
        for index, footprint in enumerate(data["footprints"]):
            for number, pad in enumerate(footprint["pads"]):
                for name, polygons in pad["layers"].items():
                    for polygon in polygons:
                        if len(polygon) >= 3:
                            self.layers[name].append((index, number, np.array(polygon), pad["net"]))
                if pad["npth"] and "hole" in pad:
                    x, y, r = pad["hole"]
                    for name in OUTER:
                        self.layers[name].append((index, number, circle(x, y, r), None))
        self.boxes = {name: np.array([[p[:, 0].min(), p[:, 1].min(), p[:, 0].max(), p[:, 1].max()]
                                      for _, _, p, _ in items]) if items else np.zeros((0, 4))
                      for name, items in self.layers.items()}
        self.signal = {net for net, owners in footprints_of_net.items()
                       if len(owners) > 1 and net not in zone_nets}

    def quantum(self, k):
        return k * self.w + (k + 1) * self.s

    def tracks(self, gap, clearance_a=None, clearance_b=None):
        a = self.s if clearance_a is None else clearance_a
        b = self.s if clearance_b is None else clearance_b
        return max(0, int(math.floor((gap - a - b + self.s) / self.pitch + 1e-9)))


# --- gaps --------------------------------------------------------------------------


def cuts(board):
    """The visible pad pairs of different footprints per outer layer, as
    (layer, footprint a, footprint b, gap)."""
    reach = min(board.quantum(3) + board.pitch, 2.0)
    result = []
    for layer, items in board.layers.items():
        boxes = board.boxes[layer]
        count = len(items)
        for i in range(count):
            fa = items[i][0]
            if fa < 0:
                continue
            bi = boxes[i]
            near = np.nonzero((boxes[:, 0] <= bi[2] + reach) & (boxes[:, 2] >= bi[0] - reach)
                              & (boxes[:, 1] <= bi[3] + reach) & (boxes[:, 3] >= bi[1] - reach))[0]
            for j in near:
                if j <= i or items[j][0] == fa:
                    continue
                gap, pa, pb = polygon_distance(items[i][2], items[j][2])
                if gap > reach or gap <= 0.0:
                    continue
                box = [min(pa[0], pb[0]), min(pa[1], pb[1]), max(pa[0], pb[0]), max(pa[1], pb[1])]
                blockers = np.nonzero((boxes[:, 0] <= box[2]) & (boxes[:, 2] >= box[0])
                                      & (boxes[:, 1] <= box[3]) & (boxes[:, 3] >= box[1]))[0]
                blocked = False
                for k in blockers:
                    if k == i or k == j:
                        continue
                    polygon = items[k][2]
                    if segments_cross(pa, pb, polygon, np.roll(polygon, -1, axis=0)) or \
                            inside(((pa + pb) / 2)[None, :], polygon)[0]:
                        blocked = True
                        break
                if not blocked:
                    result.append((layer, fa, items[j][0], gap))
    return result


def gap_stats(board, cut_list):
    """The gaps against the track quanta: counts, shares and the phase
    histogram (eight bins over a period) of the gaps at least the clearance
    and below Q_4."""
    s, pitch = board.s, board.pitch
    gaps = np.array([c[3] for c in cut_list])
    usable = gaps[(gaps >= s - 1e-6) & (gaps < board.quantum(4))]
    x = (usable - s) / pitch
    k = np.floor(x + 1e-9)
    phase = x - k
    quanta = [board.quantum(q) for q in range(1, 5)]
    wasted_01 = np.zeros(len(usable), dtype=bool)
    for q in quanta:
        wasted_01 |= (usable >= q - 0.1) & (usable < q)
    # The share a uniform spread over [s, Q_4) would put within 0.1 mm
    # below a quantum (the base rate of `wasted_01`).
    span = board.quantum(4) - s
    base_01 = sum(min(0.1, q - s) for q in quanta) / span
    carrying = k >= 1
    histogram = np.histogram(phase, bins=8, range=(0.0, 1.0))[0].tolist()
    return {
        "cuts": int(len(gaps)), "usable": int(len(usable)),
        "below_clearance": int(np.sum(gaps < s - 1e-6)),
        "by_tracks": [int(np.sum(k == q)) for q in range(4)],
        "wasted_01": float(np.mean(wasted_01)) if len(usable) else None,
        "wasted_01_base": base_01,
        "wasted_q": float(np.mean(phase >= 0.75)) if len(usable) else None,
        "tight_q": float(np.mean(phase < 0.25)) if len(usable) else None,
        "wasted_q_carrying": float(np.mean(phase[carrying] >= 0.75)) if carrying.any() else None,
        "tight_q_carrying": float(np.mean(phase[carrying] < 0.25)) if carrying.any() else None,
        "phase_mean": float(np.mean(phase)) if len(usable) else None,
        "phase_histogram": histogram,
        "median_gap": float(np.median(usable)) if len(usable) else None,
        "wasted_mm": float(np.sum(phase * pitch)),
    }


# --- escape flow -------------------------------------------------------------------


def escape(board, index, ring):
    """The escape flow of footprint `index`: pins, planar flow, flow with
    via sites, or None for a part of fewer than MANY_PINS copper pads."""
    footprint = board.data["footprints"][index]
    copper = [n for n, pad in enumerate(footprint["pads"]) if pad["layers"]]
    if len(copper) < MANY_PINS:
        return None
    pins = [n for n in copper if footprint["pads"][n]["net"] in board.signal]
    own_layers = sorted({name for n in copper for name in footprint["pads"][n]["layers"]})
    own_box = None
    for name in own_layers:
        for item, box in zip(board.layers[name], board.boxes[name]):
            if item[0] == index:
                own_box = box.copy() if own_box is None else np.array(
                    [min(own_box[0], box[0]), min(own_box[1], box[1]), max(own_box[2], box[2]), max(own_box[3], box[3])])
    if own_box is not None and board.outline:
        outer = np.concatenate(board.outline)
        area = (outer[:, 0].max() - outer[:, 0].min()) * (outer[:, 1].max() - outer[:, 1].min())
        if (own_box[2] - own_box[0]) * (own_box[3] - own_box[1]) > 0.25 * area:
            # A frame part (mounting holes and pads around the board).
            return None
    if not pins or own_box is None:
        return {"reference": footprint["reference"], "pads": len(copper), "pins": len(pins),
                "flow_planar": 0, "flow_vias": 0}
    ring_box = own_box + np.array([-ring, -ring, ring, ring])
    region = ring_box + np.array([-1.0, -1.0, 1.0, 1.0])
    pin_set = set(pins)
    # Nodes: 0 source, 1 sink, then one per pin, then triangles per layer.
    pin_node = {n: 2 + i for i, n in enumerate(pins)}
    next_node = 2 + len(pins)
    rows, cols, caps = [], [], []

    def arc(u, v, c, both=True):
        if c <= 0:
            return
        rows.append(u)
        cols.append(v)
        caps.append(c)
        if both:
            rows.append(v)
            cols.append(u)
            caps.append(c)

    for n in pins:
        arc(0, pin_node[n], 1, both=False)
    via_arcs = []
    for layer in own_layers:
        items = board.layers[layer]
        boxes = board.boxes[layer]
        near = np.nonzero((boxes[:, 0] <= region[2]) & (boxes[:, 2] >= region[0])
                          & (boxes[:, 1] <= region[3]) & (boxes[:, 3] >= region[1]))[0]
        points, owners, clearances = [], [], []
        polygons = {}
        for k in near:
            polygon = items[k][2]
            polygons[k] = polygon
            sampled = resample(polygon, SAMPLE)
            points.append(sampled)
            owners.append(np.full(len(sampled), k))
            clearances.append(np.full(len(sampled), board.s))
        for ring_points in board.outline:
            sampled = resample(ring_points, SAMPLE)
            keep = ((sampled[:, 0] >= region[0]) & (sampled[:, 0] <= region[2])
                    & (sampled[:, 1] >= region[1]) & (sampled[:, 1] <= region[3]))
            if keep.any():
                points.append(sampled[keep])
                owners.append(np.full(int(keep.sum()), -2))
                clearances.append(np.full(int(keep.sum()), board.edge))
        frame = np.array([[region[0], region[1]], [region[2], region[1]], [region[2], region[3]], [region[0], region[3]]])
        sampled = resample(frame, 0.5)
        points.append(sampled)
        owners.append(np.full(len(sampled), -1))
        clearances.append(np.zeros(len(sampled)))
        points = np.concatenate(points)
        owners = np.concatenate(owners)
        clearances = np.concatenate(clearances)
        points, unique = np.unique(np.round(points, 6), axis=0, return_index=True)
        owners, clearances = owners[unique], clearances[unique]
        mesh = Delaunay(points)
        triangles = mesh.simplices
        centroids = points[triangles].mean(axis=1)
        free = np.ones(len(triangles), dtype=bool)
        if board.outline:
            free &= inside_rings(centroids, board.outline)
        tri_owners = owners[triangles]
        same = (tri_owners[:, 0] == tri_owners[:, 1]) & (tri_owners[:, 1] == tri_owners[:, 2]) & (tri_owners[:, 0] >= 0)
        for k in np.unique(tri_owners[same, 0]):
            selected = np.nonzero(same & (tri_owners[:, 0] == k))[0]
            free[selected[inside(centroids[selected], polygons[k])]] = False
        base = next_node
        next_node += len(triangles)
        sink = free & ~((centroids[:, 0] >= ring_box[0]) & (centroids[:, 0] <= ring_box[2])
                        & (centroids[:, 1] >= ring_box[1]) & (centroids[:, 1] <= ring_box[3]))
        for t in np.nonzero(sink)[0]:
            arc(base + t, 1, BIG, both=False)
        # Adjacent free triangles.
        neighbours = mesh.neighbors
        for t in np.nonzero(free)[0]:
            for e in range(3):
                u = neighbours[t, e]
                if u < 0 or u < t or not free[u]:
                    continue
                a, b = triangles[t, (e + 1) % 3], triangles[t, (e + 2) % 3]
                oa, ob = owners[a], owners[b]
                if oa == -1 or ob == -1:
                    c = BIG
                elif oa == ob:
                    c = 0
                else:
                    c = board.tracks(float(np.linalg.norm(points[a] - points[b])), clearances[a], clearances[b])
                arc(base + t, base + u, c)
        # Each pin to the free triangles at its copper.
        pin_owner = {k: items[k][1] for k in near if items[k][0] == index and items[k][1] in pin_set}
        for t in np.nonzero(free)[0]:
            touched = set()
            for v in triangles[t]:
                k = owners[v]
                if k in pin_owner and pin_owner[k] not in touched:
                    touched.add(pin_owner[k])
                    arc(pin_node[pin_owner[k]], base + t, BIG, both=False)
        # Via sites: free triangles inside the ring whose empty circle
        # holds a via and its clearance, clear on the other outer layer.
        need = board.via / 2 + board.s
        corners = points[triangles]
        ax, ay = corners[:, 0, 0], corners[:, 0, 1]
        bx, by = corners[:, 1, 0], corners[:, 1, 1]
        cx, cy = corners[:, 2, 0], corners[:, 2, 1]
        d = 2 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by))
        with np.errstate(divide="ignore", invalid="ignore"):
            ux = ((ax * ax + ay * ay) * (by - cy) + (bx * bx + by * by) * (cy - ay) + (cx * cx + cy * cy) * (ay - by)) / d
            uy = ((ax * ax + ay * ay) * (cx - bx) + (bx * bx + by * by) * (ax - cx) + (cx * cx + cy * cy) * (bx - ax)) / d
        radius = np.hypot(ax - ux, ay - uy)
        candidate = free & ~sink & np.isfinite(radius) & (radius >= need)
        centres = np.stack([ux, uy], axis=1)
        if candidate.any():
            # The centre lies in free space on this layer (a triangle of
            # the mesh that is free holds it) and on the board.
            located = mesh.find_simplex(centres[candidate])
            ok = (located >= 0)
            ok[ok] = free[located[ok]]
            if board.outline:
                ok &= inside_rings(centres[candidate], board.outline)
            selected = np.nonzero(candidate)[0][ok]
            other = [name for name in OUTER if name != layer]
            for t in selected:
                if all(clear(board, name, centres[t], need) for name in other):
                    via_arcs.append(base + t)
    n_nodes = next_node
    planar = flow(rows, cols, caps, n_nodes)
    for t in via_arcs:
        rows.append(t)
        cols.append(1)
        caps.append(1)
    with_vias = flow(rows, cols, caps, n_nodes)
    return {"reference": footprint["reference"], "pads": len(copper), "pins": len(pins),
            "flow_planar": planar, "flow_vias": with_vias, "via_sites": len(via_arcs)}


def clear(board, layer, point, need):
    """Whether `point` lies at least `need` from every piece of copper on
    `layer` (and outside them)."""
    boxes = board.boxes[layer]
    near = np.nonzero((boxes[:, 0] <= point[0] + need) & (boxes[:, 2] >= point[0] - need)
                      & (boxes[:, 1] <= point[1] + need) & (boxes[:, 3] >= point[1] - need))[0]
    for k in near:
        polygon = board.layers[layer][k][2]
        if inside(point[None, :], polygon)[0]:
            return False
        distance, _ = point_segments(point[None, :], polygon, np.roll(polygon, -1, axis=0))
        if distance.min() < need:
            return False
    return True


def flow(rows, cols, caps, n_nodes):
    matrix = csr_matrix((np.array(caps, dtype=np.int32), (np.array(rows), np.array(cols))), shape=(n_nodes, n_nodes))
    matrix.sum_duplicates()
    return int(maximum_flow(matrix, 0, 1).flow_value)


# --- units -------------------------------------------------------------------------


def measure(data, ring):
    board = Board(data)
    cut_list = cuts(board)
    parts = []
    for index in range(len(data["footprints"])):
        result = escape(board, index, ring)
        if result is not None:
            parts.append(result)
    pins = sum(p["pins"] for p in parts)
    return {
        "rules": {"track": board.w, "clearance": board.s, "via": board.via, "edge": board.edge,
                  "quanta": [board.quantum(k) for k in range(4)], "classes": data["rules"]["classes"]},
        "gaps": gap_stats(board, cut_list),
        "parts": parts,
        "pins": pins,
        "deficit_planar": sum(p["pins"] - p["flow_planar"] for p in parts),
        "deficit_vias": sum(p["pins"] - p["flow_vias"] for p in parts),
        "parts_short_planar": sum(1 for p in parts if p["flow_planar"] < p["pins"]),
        "parts_short_vias": sum(1 for p in parts if p["flow_vias"] < p["pins"]),
        "ring": ring,
    }


def run_unit(arguments):
    board, label, path, extras, out, ring = arguments
    overrides = extras.get("overrides")
    cache = Path(out) / "extract" / board / (label.replace("/", "_") + ".json")
    data = extracted(path, cache, overrides)
    result = measure(data, ring)
    result.update({"board": board, "label": label, "path": path, **{k: v for k, v in extras.items() if k != "overrides"}})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    m = sub.add_parser("measure")
    m.add_argument("--list", required=True)
    m.add_argument("--out", required=True)
    m.add_argument("--ring", type=float, default=2.5)
    m.add_argument("--jobs", type=int, default=4)
    o = sub.add_parser("one")
    o.add_argument("board")
    o.add_argument("--ring", type=float, default=2.5)
    arguments = parser.parse_args()
    if arguments.command == "one":
        result = measure(extract(arguments.board), arguments.ring)
        print(json.dumps(result, indent=1))
        return
    units = []
    for line in Path(arguments.list).read_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split("\t")
        extras = json.loads(fields[3]) if len(fields) > 3 else {}
        units.append((fields[0], fields[1], fields[2], extras, arguments.out, arguments.ring))
    out = Path(arguments.out)
    out.mkdir(parents=True, exist_ok=True)
    target = out / f"units-r{arguments.ring:g}.json"
    done = {}
    if target.exists():
        for row in json.loads(target.read_text()):
            done[(row["board"], row["label"])] = row
    todo = [u for u in units if (u[0], u[1]) not in done]
    with concurrent.futures.ProcessPoolExecutor(max_workers=arguments.jobs) as pool:
        for result in pool.map(run_unit, todo):
            done[(result["board"], result["label"])] = result
            print(f"{result['board']} {result['label']}: deficit planar {result['deficit_planar']} "
                  f"vias {result['deficit_vias']} of {result['pins']} pins; wasted_q {result['gaps']['wasted_q']}",
                  flush=True)
            target.write_text(json.dumps(list(done.values())))
    target.write_text(json.dumps(list(done.values())))


if __name__ == "__main__":
    sys.exit(main())
