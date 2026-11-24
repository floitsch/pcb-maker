# Copyright (C) 2026 Florian Loitsch.
"""why-open: per KiCad unconnected item, the evidence for why it is open.

A lab prototype for docs/experiments/evaluation-representations.md. It reads
a routed board with KiCad 10's pcbnew and KiCad's DRC report, cuts a window
around each unconnected item, rasterises the copper there and answers, by a
shortest-path search on the raster:

- is there a legal path for the item's net alone, with every other net's
  copper kept (a single-net miss)?
- if not, which nets' routed copper does the cheapest path cross (walled by
  routed copper)?
- if no path exists even with every track and via of the other nets removed,
  which fixed objects (pads, footprint copper graphics, keepouts, holes, the
  board edge) close the item's ends in (walled at the pad)?

From the same raster it writes three representations of one item:

- a dossier (JSON): the classification with its evidence;
- a picture (PNG): one panel per copper layer, the item's net highlighted;
- a text map (ASCII): the same window as characters.

The raster is an approximation (the net class's width and clearance, no
custom rules, no per-pad clearances, through vias only); it is evidence for
a diagnosis, not a DRC.

Usage:
  whyopen.py BOARD.kicad_pcb --drc drc.json [--router board-router.json]
             --out DIR [--item N] [--formats dossier,png,ascii]
             [--without-net NET]... [--margin MM] [--plain]
"""

import argparse
import json
import math
import os
import re
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from scipy import ndimage
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import dijkstra

import pcbnew

LONG_MM = 20.0
NECK_MAX_MM = 2.0  # the router lets a neck run this far from a pad
PENALTY = 1000.0  # mm per pixel of another net's routed copper crossed
VIA_COST = 1.0  # mm
MAX_NODES = 700_000
FONT = "/usr/share/fonts/TTF/DejaVuSans.ttf"
FONT_MONO = "/usr/share/fonts/TTF/DejaVuSansMono.ttf"


def mm(v):
    return v / 1e6


def nm(v):
    return int(round(v * 1e6))


class Window:
    """The copper of a board inside a box, as rasters per copper layer."""

    def __init__(self, board, box, px, net, without=()):
        self.board = board
        self.box = box  # x0, y0, x1, y1 in mm
        self.px = px
        self.net = net
        self.without = set(without)
        ds = board.GetDesignSettings()
        self.layers = list(ds.GetEnabledLayers().CuStack())
        self.names = [board.GetLayerName(l) for l in self.layers]
        self.w = int(math.ceil((box[2] - box[0]) / px))
        self.h = int(math.ceil((box[3] - box[1]) / px))
        n = len(self.layers)
        shape = (n, self.h, self.w)
        # Objects: index 0 is "nothing". Each is a dict with a name and a kind.
        self.objects = [None]
        self.fixed_id = np.zeros(shape, np.int32)  # pads, graphics, keepouts
        self.routed_id = np.zeros(shape, np.int32)  # tracks and vias of other nets
        self.own = np.zeros(shape, bool)  # the item's net: any copper
        self.own_zone = np.zeros(shape, bool)
        self.other_zone = np.zeros(shape, bool)
        self.hole = np.zeros((self.h, self.w), bool)
        self.own_smd = np.zeros((self.h, self.w), bool)  # the net's pads without a hole
        self.inside = np.zeros((self.h, self.w), bool)
        self.pads = []  # for labels: (ref, number, net, x, y, layers, own)
        self.net_objects = {}
        self._collect()

    # -- rasterising -------------------------------------------------------

    def _xy(self, pts):
        x0, y0 = self.box[0], self.box[1]
        return [((x - x0) / self.px, (y - y0) / self.px) for x, y in pts]

    def _mask(self, polys):
        img = Image.new("L", (self.w, self.h), 0)
        d = ImageDraw.Draw(img)
        for outline, holes in polys:
            if len(outline) >= 3:
                d.polygon(self._xy(outline), fill=1, outline=1)
            for hole in holes:
                if len(hole) >= 3:
                    d.polygon(self._xy(hole), fill=0)
        return np.asarray(img, bool)

    def _hits(self, item):
        bb = item.GetBoundingBox()
        x0, y0, x1, y1 = self.box
        pad = 1.0
        return not (
            mm(bb.GetRight()) < x0 - pad
            or mm(bb.GetLeft()) > x1 + pad
            or mm(bb.GetBottom()) < y0 - pad
            or mm(bb.GetTop()) > y1 + pad
        )

    def _object(self, name, kind, net=""):
        self.objects.append({"name": name, "kind": kind, "net": net})
        return len(self.objects) - 1

    def _net_object(self, net):
        if net not in self.net_objects:
            self.net_objects[net] = self._object(net, "routed", net)
        return self.net_objects[net]

    def _shape_polys(self, item, layer):
        ps = pcbnew.SHAPE_POLY_SET()
        try:
            item.TransformShapeToPolygon(ps, layer, 0, 5000, pcbnew.ERROR_INSIDE)
        except Exception:
            return []
        return polys_of(ps)

    def _clip(self, ps):
        x0, y0, x1, y1 = self.box
        rect = pcbnew.SHAPE_POLY_SET()
        rect.NewOutline()
        for x, y in [(x0 - 1, y0 - 1), (x1 + 1, y0 - 1), (x1 + 1, y1 + 1), (x0 - 1, y1 + 1)]:
            rect.Append(nm(x), nm(y))
        out = pcbnew.SHAPE_POLY_SET(ps)
        out.BooleanIntersection(rect)
        return out

    def _collect(self):
        b = self.board
        outline = pcbnew.SHAPE_POLY_SET()
        if b.GetBoardPolygonOutlines(outline, True) and outline.OutlineCount():
            self.inside = self._mask(polys_of(self._clip(outline)))
        else:
            self.inside[:] = True

        for fp in b.GetFootprints():
            if not self._hits(fp):
                continue
            ref = fp.GetReference()
            for pad in fp.Pads():
                if not self._hits(pad):
                    continue
                net = pad.GetNetname()
                own = net == self.net and net != ""
                name = f"{ref}.{pad.GetNumber()}"
                oid = None
                on = []
                for li, layer in enumerate(self.layers):
                    if not pad.IsOnLayer(layer):
                        continue
                    m = self._mask(self._shape_polys(pad, layer))
                    if not m.any():
                        continue
                    on.append(li)
                    if own:
                        self.own[li] |= m
                        if not pad.HasHole():
                            self.own_smd |= m
                    else:
                        if oid is None:
                            oid = self._object(name, "pad", net)
                        self.fixed_id[li][m] = oid
                if pad.HasHole():
                    ps = pcbnew.SHAPE_POLY_SET()
                    pad.TransformHoleToPolygon(ps, 0, 5000, pcbnew.ERROR_INSIDE)
                    self.hole |= self._mask(polys_of(ps))
                pos = pad.GetPosition()
                self.pads.append((ref, pad.GetNumber(), net, mm(pos.x), mm(pos.y), on, own))
            for g in fp.GraphicalItems():
                for li, layer in enumerate(self.layers):
                    if g.GetLayer() != layer or not self._hits(g):
                        continue
                    m = self._mask(self._shape_polys(g, layer))
                    if m.any():
                        oid = self._object(f"copper graphic of {ref}", "graphic")
                        self.fixed_id[li][m & (self.fixed_id[li] == 0)] = oid
            for z in fp.Zones():
                self._zone(z, f"keepout of {ref}")

        for g in b.Drawings():
            for li, layer in enumerate(self.layers):
                if g.GetLayer() != layer or not self._hits(g):
                    continue
                m = self._mask(self._shape_polys(g, layer))
                if m.any():
                    oid = self._object("board copper graphic", "graphic")
                    self.fixed_id[li][m & (self.fixed_id[li] == 0)] = oid

        for t in b.GetTracks():
            if not self._hits(t):
                continue
            net = t.GetNetname()
            own = net == self.net and net != ""
            is_via = t.GetClass() == "PCB_VIA"
            for li, layer in enumerate(self.layers):
                if not t.IsOnLayer(layer):
                    continue
                m = self._mask(self._shape_polys(t, layer))
                if own:
                    self.own[li] |= m
                elif net not in self.without:
                    self.routed_id[li][m] = self._net_object(net)
            if is_via and (own or net not in self.without):
                pos = t.GetPosition()
                r = mm(t.GetDrillValue()) / 2
                x, y = mm(pos.x), mm(pos.y)
                pts = [
                    (x + r * math.cos(a / 16 * 2 * math.pi), y + r * math.sin(a / 16 * 2 * math.pi))
                    for a in range(16)
                ]
                self.hole |= self._mask([(pts, [])])

        for z in b.Zones():
            self._zone(z, "keepout")

    def _zone(self, z, keepout_name):
        if not self._hits(z):
            return
        if z.GetIsRuleArea():
            if not z.GetDoNotAllowTracks():
                return
            oid = self._object(keepout_name, "keepout")
            for li, layer in enumerate(self.layers):
                if z.IsOnLayer(layer):
                    m = self._mask(polys_of(self._clip(z.Outline())))
                    self.fixed_id[li][m & (self.fixed_id[li] == 0)] = oid
            return
        net = z.GetNetname()
        for li, layer in enumerate(self.layers):
            if not z.IsOnLayer(layer):
                continue
            fill = z.GetFilledPolysList(layer)
            if fill.OutlineCount() == 0:
                continue
            m = self._mask(polys_of(self._clip(fill)))
            if net == self.net:
                self.own[li] |= m
                self.own_zone[li] |= m
            else:
                self.other_zone[li] |= m


def polys_of(ps):
    out = []
    for i in range(ps.OutlineCount()):
        o = ps.Outline(i)
        pts = [(mm(o.CPoint(k).x), mm(o.CPoint(k).y)) for k in range(o.PointCount())]
        holes = []
        for h in range(ps.HoleCount(i)):
            hh = ps.Hole(i, h)
            holes.append([(mm(hh.CPoint(k).x), mm(hh.CPoint(k).y)) for k in range(hh.PointCount())])
        out.append((pts, holes))
    return out


# -- the search -------------------------------------------------------------


def dist_mm(mask, px):
    """Distance in mm of every pixel to the nearest set pixel of mask."""
    if not mask.any():
        return np.full(mask.shape, 1e9)
    return ndimage.distance_transform_edt(~mask) * px


class Search:
    """Free space and shortest paths for one track width in a window."""

    def __init__(self, win, rules, width):
        self.win = win
        self.rules = rules
        self.width = width
        px = win.px
        c = rules["clearance"]
        half = width / 2
        n, h, w = win.fixed_id.shape
        edge = dist_mm(~win.inside, px)
        hole = dist_mm(win.hole, px)
        common = (edge < rules["edge_clearance"] + half) | ~win.inside
        hole_block = hole < rules["hole_clearance"] + half
        self.fixed_block = np.zeros((n, h, w), bool)
        self.routed_block = np.zeros((n, h, w), bool)
        self.nearest_routed = []
        self.nearest_fixed = []
        via_half = rules["via_diameter"] / 2
        via_fixed = common | (hole < rules["hole_to_hole"] + rules["via_drill"] / 2) | (
            edge < rules["edge_clearance"] + via_half
        )
        via_routed = np.zeros((h, w), bool)
        for l in range(n):
            fixed = win.fixed_id[l] > 0
            routed = win.routed_id[l] > 0
            df = dist_mm(fixed, px)
            dr = dist_mm(routed, px)
            # Inside the net's own copper a track adds nothing where it lies
            # wholly within that copper; near its rim it must keep the
            # clearances like any other track.
            inner = dist_mm(~win.own[l], px) >= half - px
            self.fixed_block[l] = ((df < c + half) | common | hole_block) & ~inner
            self.routed_block[l] = (dr < c + half) & ~inner & ~self.fixed_block[l]
            via_fixed |= df < c + via_half
            via_routed |= dr < c + via_half
        # A via on the net's own SMD pad is a via in pad: not offered.
        via_fixed |= dist_mm(win.own_smd, px) < via_half
        self.via_ok = ~via_fixed
        self.via_pen = via_routed & self.via_ok
        self.passable = ~self.fixed_block
        self._graph = None

    def graph(self):
        if self._graph is not None:
            return self._graph
        n, h, w = self.passable.shape
        px = self.win.px
        idx = np.arange(n * h * w).reshape(n, h, w)
        pen = self.routed_block.astype(np.float64) * PENALTY
        rows, cols, vals = [], [], []

        def add(a, b, cost):
            ok = self.passable[a] & self.passable[b]
            rows.append(idx[a][ok])
            cols.append(idx[b][ok])
            vals.append(cost + (pen[a][ok] + pen[b][ok]) / 2)

        s = np.s_
        add(s[:, :, :-1], s[:, :, 1:], px)
        add(s[:, :-1, :], s[:, 1:, :], px)
        add(s[:, :-1, :-1], s[:, 1:, 1:], px * math.sqrt(2))
        add(s[:, :-1, 1:], s[:, 1:, :-1], px * math.sqrt(2))
        if n > 1:
            step = VIA_COST / (n - 1)
            for l in range(n - 1):
                ok = self.via_ok & self.passable[l] & self.passable[l + 1]
                rows.append(idx[l][ok])
                cols.append(idx[l + 1][ok])
                vals.append(step + self.via_pen[ok] * PENALTY * 3)
        size = n * h * w + 1
        self._size = size
        self._edges = (np.concatenate(rows), np.concatenate(cols), np.concatenate(vals))
        self._graph = True
        return True

    def from_seeds(self, seeds):
        """Distances from a seed set (a bool array n x h x w)."""
        self.graph()
        n, h, w = self.passable.shape
        r, c, v = self._edges
        seed_idx = np.flatnonzero(seeds & self.passable)
        src = self._size - 1
        rows = np.concatenate([r, np.full(len(seed_idx), src)])
        cols = np.concatenate([c, seed_idx])
        vals = np.concatenate([v, np.full(len(seed_idx), 1e-9)])
        g = coo_matrix((vals, (rows, cols)), shape=(self._size, self._size)).tocsr()
        dist, pred = dijkstra(g, directed=False, indices=src, return_predecessors=True)
        return dist[:-1].reshape(n, h, w), pred

    def path(self, pred, target_idx):
        n, h, w = self.passable.shape
        src = self._size - 1
        out = []
        i = int(target_idx)
        while i != src and i >= 0:
            out.append(i)
            i = int(pred[i])
        out.reverse()
        return [np.unravel_index(i, (n, h, w)) for i in out]


def nearest_ids(ids):
    """Per pixel the id of the nearest non-zero pixel of ids (one layer)."""
    if not (ids > 0).any():
        return np.zeros_like(ids)
    _, (iy, ix) = ndimage.distance_transform_edt(ids == 0, return_indices=True)
    return ids[iy, ix]


def border_objects(win, search, region, limit=8):
    """The objects that bound a reachable region, by length of contact."""
    k = int(math.ceil((search.rules["clearance"] + search.width / 2) / win.px)) + 2
    counts = {}
    for l in range(region.shape[0]):
        if not region[l].any():
            continue
        ring = ndimage.binary_dilation(region[l], iterations=2) & ~region[l]
        if not ring.any():
            continue
        both = np.where(win.fixed_id[l] > 0, win.fixed_id[l], win.routed_id[l])
        near = nearest_ids(both)
        d = dist_mm(both > 0, win.px)
        sel = ring & (d < (k + 1) * win.px)
        for oid, cnt in zip(*np.unique(near[sel], return_counts=True)):
            if oid:
                key = (int(oid), l)
                counts[key] = counts.get(key, 0) + int(cnt)
        edge = ring & ~win.inside
        hole = ring & (dist_mm(win.hole, win.px) < (k + 1) * win.px) & ~sel
        if edge.sum() > 2:
            counts[("edge", l)] = int(edge.sum())
        if hole.sum() > 2:
            counts[("hole", l)] = int(hole.sum())
    merged = {}
    for (oid, l), cnt in counts.items():
        if oid == "edge":
            name, kind, net = "board edge", "edge", ""
        elif oid == "hole":
            name, kind, net = "drilled hole", "hole", ""
        else:
            o = win.objects[oid]
            name, kind, net = o["name"], o["kind"], o["net"]
        e = merged.setdefault((name, kind, net), {"contact_mm": 0.0, "layers": []})
        e["contact_mm"] += cnt * win.px / 2
        if win.names[l] not in e["layers"]:
            e["layers"].append(win.names[l])
    out = [
        {"object": n, "kind": k_, "net": net, "contact_mm": round(e["contact_mm"], 2), "layers": e["layers"]}
        for (n, k_, net), e in merged.items()
    ]
    out.sort(key=lambda e: -e["contact_mm"])
    return out[:limit]


# -- one item ---------------------------------------------------------------


def net_rules(board, netname):
    ds = board.GetDesignSettings()
    net = board.FindNet(netname)
    nc = net.GetNetClassSlow() if net else None
    rules = {
        "class": nc.GetName() if nc else "Default",
        "track_width": mm(nc.GetTrackWidth()) if nc else mm(ds.m_TrackMinWidth),
        "clearance": max(mm(nc.GetClearance()) if nc else 0.0, mm(ds.m_MinClearance)),
        "via_diameter": mm(nc.GetViaDiameter()) if nc else mm(ds.m_ViasMinSize),
        "via_drill": mm(nc.GetViaDrill()) if nc else 0.3,
        "min_track_width": mm(ds.m_TrackMinWidth),
        "neck_width": mm(ds.m_TrackMinWidth),
        "neck_source": "the board's minimum track width",
        "edge_clearance": mm(ds.m_CopperEdgeClearance),
        "hole_clearance": mm(ds.m_HoleClearance),
        "hole_to_hole": mm(ds.m_HoleToHoleMin),
    }
    return rules


def end_info(desc):
    m = re.match(r"(Pad|Track|Via|Zone|Arc|PTH pad|NPTH pad)\b", desc)
    kind = m.group(1).lower() if m else "item"
    net = re.search(r"\[([^\]]*)\]", desc)
    pad = re.search(r"Pad (\S+) \[[^\]]*\] of (\S+)", desc)
    return {
        "kind": kind,
        "net": net.group(1) if net else "",
        "pad": f"{pad.group(2)}.{pad.group(1)}" if pad else None,
    }


def seeds_for(win, end, pos):
    """The item's own copper piece at an end: bool n x h x w."""
    n, h, w = win.own.shape
    x = int((pos[0] - win.box[0]) / win.px)
    y = int((pos[1] - win.box[1]) / win.px)
    out = np.zeros((n, h, w), bool)
    r = max(2, int(0.12 / win.px))
    best = None
    for l in range(n):
        if end.get("layer_idx") is not None and l not in end["layer_idx"]:
            continue
        lab, cnt = ndimage.label(win.own[l], structure=np.ones((3, 3)))
        y0, y1, x0, x1 = max(0, y - r), min(h, y + r + 1), max(0, x - r), min(w, x + r + 1)
        ids = np.unique(lab[y0:y1, x0:x1])
        ids = ids[ids > 0]
        for i in ids:
            out[l] |= lab == i
            best = l
    if best is None:
        # No own copper at the position: fall back to the nearest own pixel.
        for l in range(n):
            if win.own[l].any():
                d = dist_mm(win.own[l], win.px)
                if 0 <= y < h and 0 <= x < w and d[y, x] < 0.5:
                    lab, _ = ndimage.label(win.own[l], structure=np.ones((3, 3)))
                    _, (iy, ix) = ndimage.distance_transform_edt(~win.own[l], return_indices=True)
                    out[l] |= lab == lab[iy[y, x], ix[y, x]]
    return out


def layers_in(desc, names):
    idx = [i for i, nme in enumerate(names) if re.search(r"\b" + re.escape(nme) + r"\b", desc)]
    if "and" in desc and "more" in desc:
        return None
    return idx or None


def analyse(win, rules, ends, width):
    """One search at a width: the path A to B and each end's reach."""
    search = Search(win, rules, width)
    seeds = [seeds_for(win, e, e["pos"]) for e in ends]
    res = {"width_mm": round(width, 3)}
    if not seeds[0].any() or not seeds[1].any():
        res["status"] = "no_copper_at_end"
        return res, search, seeds, None, None
    dist_a, pred_a = search.from_seeds(seeds[0])
    target = np.where(seeds[1] & search.passable, dist_a, np.inf)
    best = np.argmin(target)
    cost = target.flat[best]
    path = None
    if np.isfinite(cost):
        path = search.path(pred_a, best)
        length = 0.0
        vias = 0
        crossed = {}
        layers = []
        via_at = []
        for a, b_ in zip(path, path[1:]):
            if a[0] != b_[0]:
                if not via_at or via_at[-1] != (a[1], a[2]):
                    via_at.append((a[1], a[2]))
            else:
                length += win.px * math.hypot(a[1] - b_[1], a[2] - b_[2])
                if win.names[a[0]] not in layers and not win.own[a[0], a[1], a[2]]:
                    layers.append(win.names[a[0]])
        vias = len(via_at)
        res["_via_at"] = via_at
        nearest = {}
        for l, y, x in path:
            if search.routed_block[l, y, x]:
                if l not in nearest:
                    nearest[l] = nearest_ids(win.routed_id[l])
                oid = int(nearest[l][y, x])
                name = win.objects[oid]["name"] if oid else "?"
                crossed[name] = crossed.get(name, 0) + 1
        # A via whose barrel or ring is too close to another net's copper.
        via_r = int(math.ceil((rules["clearance"] + rules["via_diameter"] / 2) / win.px))
        for y, x in via_at:
            if not search.via_pen[y, x]:
                continue
            y0, y1, x0, x1 = max(0, y - via_r), y + via_r + 1, max(0, x - via_r), x + via_r + 1
            for l in range(len(win.layers)):
                ids = win.routed_id[l][y0:y1, x0:x1]
                for oid in np.unique(ids[ids > 0]):
                    name = win.objects[int(oid)]["name"]
                    crossed[name] = crossed.get(name, 0) + via_r
        res["path"] = {
            "length_mm": round(length, 2),
            "vias": vias,
            "vias_at_mm": [
                [round(win.box[0] + (x + 0.5) * win.px, 2), round(win.box[1] + (y + 0.5) * win.px, 2)] for y, x in via_at
            ],
            "new_copper_on_layers": layers,
            "crosses_routed_nets": {k: round(v * win.px, 2) for k, v in sorted(crossed.items(), key=lambda kv: -kv[1])},
        }
        res["status"] = "path_with_all_copper_kept" if not crossed else "path_only_across_other_nets"
    else:
        res["status"] = "no_path_even_without_other_tracks"
    # Each end's reach, with all copper kept and with the other tracks gone.
    reach = []
    dist_b, _ = search.from_seeds(seeds[1])
    for name, dist in (("A", dist_a), ("B", dist_b)):
        kept = dist < PENALTY / 2
        bare = np.isfinite(dist)
        entry = {}
        for label, region in (("all_copper_kept", kept), ("other_tracks_removed", bare)):
            beyond = region & ~win.own
            touches = bool(
                region[:, 0, :].any() or region[:, -1, :].any() or region[:, :, 0].any() or region[:, :, -1].any()
            )
            entry[label] = {
                "free_area_mm2": round(float(beyond.sum()) * win.px * win.px, 2),
                "reaches_window_edge": touches,
                "bounded_by": [] if touches else border_objects(win, search, region),
            }
        reach.append(entry)
    res["reach"] = {"A": reach[0], "B": reach[1]}
    return res, search, seeds, path, (dist_a, dist_b)


def gap_report(win, seeds, rules):
    """Room around an end's own copper: the nearest other fixed copper."""
    out = []
    for s in seeds:
        best = None
        for l in range(s.shape[0]):
            if not s[l].any():
                continue
            fixed = win.fixed_id[l] > 0
            if not fixed.any():
                continue
            d = dist_mm(s[l], win.px)
            near = np.where(fixed, d, np.inf)
            i = np.argmin(near)
            if np.isfinite(near.flat[i]) and (best is None or near.flat[i] < best[0]):
                oid = int(win.fixed_id[l].flat[i])
                best = (float(near.flat[i]), win.objects[oid]["name"], win.objects[oid]["net"], win.names[l])
        if best:
            out.append(
                {
                    "nearest_fixed_copper_mm": round(best[0], 3),
                    "object": best[1],
                    "net": best[2],
                    "layer": best[3],
                }
            )
        else:
            out.append(None)
    return out


def classify(item, a_class, a_min):
    ends = item["ends"]
    far = item["distance_mm"] > LONG_MM
    zone_ends = [e for e in ends if e["kind"] == "zone"]
    st = a_class.get("status")
    neck = (a_min or {}).get("status")
    neck_path = (a_min or {}).get("path") or {}
    tight = neck_path.get("of_it_too_tight_for_the_class_width_mm", 0.0)
    if zone_ends:
        base = "an end is a piece of the net's pour (KiCad's fill left it apart from the rest): "
        if far:
            return "pour_island", base + "the ends KiCad names are %.1f mm apart; see each end's reach" % item["distance_mm"]
        if st == "path_with_all_copper_kept":
            p = a_class["path"]
            return "pour_island", base + "a track of %.1f mm with %d via(s) would join the pieces with every other net's copper kept" % (
                p["length_mm"],
                p["vias"],
            )
        if st == "path_only_across_other_nets":
            return "pour_island", base + "the cheapest joining track crosses the routed copper of %s" % ", ".join(
                a_class["path"]["crosses_routed_nets"]
            )
        return "pour_island", base + "no track joins the pieces inside the window"
    if far:
        return "long", "the ends are %.1f mm apart: not a local problem; see each end's reach" % item["distance_mm"]
    if st == "no_copper_at_end":
        return "unknown", "no copper of the net was found at an end (item off the board or outside the model)"
    if st == "path_with_all_copper_kept":
        p = a_class["path"]
        return (
            "single_net_path_exists",
            "a track of the class's width joins the two ends with every other net's copper kept: %.1f mm, %d via(s), on %s"
            % (p["length_mm"], p["vias"], ", ".join(p["new_copper_on_layers"]) or "the net's own copper"),
        )
    if neck == "path_with_all_copper_kept":
        if tight <= NECK_MAX_MM:
            return (
                "single_net_path_exists",
                "with every other net's copper kept a track joins the two ends if it necks down to %.3f mm for %.1f mm of its %.1f mm (%d via(s)); at the class's full width %.3f mm there is no path"
                % (a_min["width_mm"], tight, neck_path["length_mm"], neck_path["vias"], a_class["width_mm"]),
            )
        return (
            "class_width_does_not_fit",
            "the class's %.3f mm track does not fit: a %.3f mm track joins the ends with all copper kept, but %.1f mm of it runs where the class's width has no room"
            % (a_class["width_mm"], a_min["width_mm"], tight),
        )
    if st == "path_only_across_other_nets" or neck == "path_only_across_other_nets":
        src = a_class if st == "path_only_across_other_nets" else a_min
        nets = list(src["path"]["crosses_routed_nets"])
        if st != "path_only_across_other_nets" and tight > NECK_MAX_MM:
            return (
                "class_width_does_not_fit",
                "the class's %.3f mm track does not fit even with every other track removed; a %.3f mm track would also have to cross the routed copper of %s"
                % (a_class["width_mm"], a_min["width_mm"], ", ".join(nets)),
            )
        return (
            "walled_by_routed_copper",
            "no path with the copper kept; the cheapest path (at %.3f mm) crosses the routed copper of %s" % (src["width_mm"], ", ".join(nets)),
        )
    return "walled_by_fixed_objects", "no path even with every other net's tracks and vias removed"


def walls(a):
    """Which end is closed in with the other tracks removed, and by what."""
    out = {}
    for name in ("A", "B"):
        r = a.get("reach", {}).get(name, {}).get("other_tracks_removed")
        if r and not r["reaches_window_edge"]:
            out[name] = {"free_area_mm2": r["free_area_mm2"], "bounded_by": r["bounded_by"]}
    return out


def item_window(ends, margin):
    xs = [e["pos"][0] for e in ends]
    ys = [e["pos"][1] for e in ends]
    x0, x1, y0, y1 = min(xs) - margin, max(xs) + margin, min(ys) - margin, max(ys) + margin
    side = 6.0
    if x1 - x0 < side:
        c = (x0 + x1) / 2
        x0, x1 = c - side / 2, c + side / 2
    if y1 - y0 < side:
        c = (y0 + y1) / 2
        y0, y1 = c - side / 2, c + side / 2
    return [x0, y0, x1, y1]


def pick_px(box, nlayers):
    area = (box[2] - box[0]) * (box[3] - box[1])
    px = math.sqrt(area * nlayers / MAX_NODES)
    return max(0.02, round(px + 0.0049, 2))


def router_context(router, net, ends):
    if not router:
        return None
    diag = router.get("diagnostics", {})
    out = {}
    short = net.split("/")[-1]
    for n in router.get("nets", []):
        if n["connection"] in (net, net.lstrip("/"), short):
            out["net_status"] = {k: n[k] for k in ("status", "terminals", "unconnected_terminals") if k in n}
            break
    out["router_open_connections"] = router.get("unconnected_terminals")
    out["in_conflicted_nets"] = any(c in (net, net.lstrip("/")) for c in diag.get("conflicted_nets", []))
    dead = []
    for d in diag.get("dead_pads", []):
        s = json.dumps(d)
        if any(e["pad"] and (e["pad"].replace(".", " ") in s or e["pad"] in s) for e in ends) or (
            isinstance(d, dict) and d.get("net") in (net, net.lstrip("/"))
        ):
            dead.append(d)
    if dead:
        out["dead_pads"] = dead
    best = None
    for i, hs in enumerate(diag.get("hot_spots", [])):
        d = min(math.hypot(hs["at"][0] - e["pos"][0], hs["at"][1] - e["pos"][1]) for e in ends)
        if best is None or d < best[0]:
            best = (d, i, hs)
    if best:
        out["nearest_hot_spot"] = {
            "index": best[1],
            "distance_mm": round(best[0], 1),
            "layer": best[2].get("layer"),
            "nets": best[2].get("nets"),
        }
    sel = [a for a in router.get("attempts", []) if a.get("selected")]
    if sel:
        out["kept_attempt"] = {k: sel[0].get(k) for k in ("name", "pours", "grid_pitch_mm", "open", "negotiation_stop")}
    return out


def do_item(board, drc_item, index, args, router):
    descs = [it["description"] for it in drc_item["items"]]
    ends = []
    for it in drc_item["items"][:2]:
        e = end_info(it["description"])
        e["description"] = it["description"]
        e["pos"] = [it["pos"]["x"], it["pos"]["y"]]
        ends.append(e)
    net = ends[0]["net"] or ends[1]["net"]
    dist = math.hypot(ends[0]["pos"][0] - ends[1]["pos"][0], ends[0]["pos"][1] - ends[1]["pos"][1])
    rules = net_rules(board, net)
    neck = (router or {}).get("rules", {}).get("neck_width_mm")
    if neck:
        rules["neck_width"] = neck
        rules["neck_source"] = "the router's neck width (board-router.json rules.neck_width_mm)"
    item = {
        "item": index,
        "net": net,
        "kicad": descs,
        "ends": ends,
        "distance_mm": round(dist, 2),
        "rules": rules,
    }
    names = [board.GetLayerName(l) for l in board.GetDesignSettings().GetEnabledLayers().CuStack()]
    for e in ends:
        e["layer_idx"] = layers_in(e["description"], names)
    windows = []
    if dist > LONG_MM:
        for e in ends:
            windows.append(([e, e], item_window([e], 3.0)))
    else:
        windows.append((ends, item_window(ends, args.margin)))
    results = []
    for wends, box in windows:
        px = pick_px(box, len(names))
        win = Window(board, box, px, net, without=args.without_net or ())
        a_class, search, seeds, path, dists = analyse(win, rules, wends, rules["track_width"])
        a_min = None
        if 0 < rules["neck_width"] < rules["track_width"] - 1e-6 and a_class.get("status") not in (
            "path_with_all_copper_kept",
            "no_copper_at_end",
        ):
            a_min, s_min, _, path_min, _ = analyse(win, rules, wends, rules["neck_width"])
            if path_min:
                # The part of the narrow path where the class's width does not fit.
                tight = sum(
                    1 for l, y, x in path_min if (search.fixed_block[l, y, x] or search.routed_block[l, y, x])
                )
                a_min["path"]["of_it_too_tight_for_the_class_width_mm"] = round(tight * win.px, 2)
        else:
            path_min = None
        results.append((win, a_class, a_min, search, seeds, path, path_min, wends))
    win, a_class, a_min, search, seeds, path, path_min, _ = results[0]
    cause, why = classify(item, a_class, a_min)
    dossier = {
        "item": index,
        "net": net,
        "kicad_says": descs,
        "ends": {
            n: {"what": e["description"], "at_mm": e["pos"]} for n, e in zip("AB", ends)
        },
        "ends_apart_mm": round(dist, 2),
        "rules_used": {
            "net_class": rules["class"],
            "track_width_mm": rules["track_width"],
            "clearance_mm": rules["clearance"],
            "via_diameter_mm": rules["via_diameter"],
            "board_minimum_track_mm": rules["min_track_width"],
            "neck_width_mm": rules["neck_width"],
            "neck_width_from": rules["neck_source"],
        },
        "cause": cause,
        "because": why,
    }
    if args.without_net:
        dossier["tracks_removed_for_this_query"] = list(args.without_net)
    if dist > LONG_MM:
        dossier["end_windows"] = []
        for (w_, ac, am, s_, sd, _, _, we), n in zip(results, "AB"):
            r = ac.get("reach", {}).get("A", {})
            dossier["end_windows"].append(
                {
                    "end": n,
                    "window_mm": [round(v, 2) for v in w_.box],
                    "leaves_its_neighbourhood_with_all_copper_kept": r.get("all_copper_kept", {}).get("reaches_window_edge"),
                    "leaves_with_other_tracks_removed": r.get("other_tracks_removed", {}).get("reaches_window_edge"),
                    "bounded_by": r.get("all_copper_kept", {}).get("bounded_by"),
                }
            )
    else:
        dossier["window_mm"] = [round(v, 2) for v in win.box]
        dossier["raster_mm_per_pixel"] = win.px
        dossier["search_at_class_width"] = {k: v for k, v in a_class.items() if not k.startswith("_")}
        if a_min:
            dossier["search_at_neck_width"] = {
                k: v for k, v in a_min.items() if k in ("width_mm", "status", "path")
            }
        w = walls(a_class)
        if w:
            dossier["closed_in_with_other_tracks_removed"] = w
            kinds = {b["kind"] for e in w.values() for b in e["bounded_by"]}
            dossier["wall_kinds"] = sorted(kinds)
        gaps = gap_report(win, seeds, rules)
        need = rules["clearance"] * 2 + rules["track_width"]
        dossier["room_at_ends"] = {
            "a_track_between_two_coppers_needs_mm": round(need, 3),
            "A": gaps[0],
            "B": gaps[1],
        }
        other = [win.names[l] for l in range(len(win.names)) if win.other_zone[l].any()]
        if other:
            dossier["other_nets_pours_in_window"] = {
                "layers": other,
                "note": "not counted as obstacles: KiCad refills a pour around new copper",
            }
    if args.verify and cause == "single_net_path_exists" and dist <= LONG_MM:
        necked = a_class.get("status") != "path_with_all_copper_kept"
        j = verify(args.board, win, rules, ends, rules["neck_width"] if necked else rules["track_width"], args.out, index)
        dossier["kicad_judgement"] = j
        if not j["judged"]:
            dossier["cause"] = "marginal_path"
            dossier["because"] += "; but only just: " + j["why"] + ", so KiCad was not asked"
        elif j["accepted"]:
            dossier["because"] += "; written as copper, KiCad's DRC finds the item closed and no new error"
        else:
            dossier["cause"] = "raster_path_rejected_by_kicad"
            dossier["because"] += "; written as copper, KiCad's DRC %s and reports new errors: %s" % (
                "finds the item closed" if j["kicad_unconnected_after"] < j["kicad_unconnected_before"] else "still finds it open",
                ", ".join(f"{k} x{v}" for k, v in j["new_kicad_errors"].items()) or "none",
            )
    ctx = router_context(router, net, ends)
    if ctx:
        dossier["router_said"] = ctx
    out = {"dossier": dossier}
    os.makedirs(args.out, exist_ok=True)
    stem = os.path.join(args.out, f"item{index}")
    formats = args.formats.split(",")
    if "dossier" in formats:
        with open(stem + ".json", "w") as f:
            json.dump(dossier, f, indent=1)
    for wi, (w_, ac, am, s_, sd, p, pm, we) in enumerate(results):
        suffix = "" if len(results) == 1 else "-end" + "AB"[wi]
        marks = [(n, e["pos"]) for n, e in zip("AB", ends)] if len(results) == 1 else [("AB"[wi], we[0]["pos"])]
        if "png" in formats:
            render(w_, s_, sd, marks, None if args.plain else p, stem + suffix + ".png", item, args.plain, ac)
        if "ascii" in formats:
            with open(stem + suffix + ".txt", "w") as f:
                f.write(ascii_map(w_, sd, marks, item))
    return out


# -- KiCad as the judge -----------------------------------------------------


def simplify(path, ok, px):
    """Greedy line-of-sight simplification of a pixel path per layer.

    Returns runs: (layer index, [(y, x), ...] corner pixels)."""
    runs = []
    cur = None
    for l, y, x in path:
        if cur is None or cur[0] != l:
            cur = (l, [])
            runs.append(cur)
        cur[1].append((y, x))

    def sight(l, a, b):
        n = int(max(abs(a[0] - b[0]), abs(a[1] - b[1])) * 2) + 1
        for k in range(n + 1):
            y = a[0] + (b[0] - a[0]) * k / n
            x = a[1] + (b[1] - a[1]) * k / n
            if not ok[l, int(round(y)), int(round(x))]:
                return False
        return True

    out = []
    for l, pts in runs:
        keep = [pts[0]]
        i = 0
        while i < len(pts) - 1:
            j = len(pts) - 1
            while j > i + 1 and not sight(l, pts[i], pts[j]):
                j -= 1
            keep.append(pts[j])
            i = j
        out.append((l, keep))
    return out


def kicad_drc(board_path, report):
    import subprocess

    subprocess.run(
        ["kicad-cli", "pcb", "drc", "--format", "json", "--severity-error", "--refill-zones", "-o", report, board_path],
        capture_output=True,
        text=True,
    )
    d = json.load(open(report))
    types = {}
    for v in d.get("violations", []):
        types[v["type"]] = types.get(v["type"], 0) + 1
    return len(d.get("unconnected_items", [])), types


def verify(board_path, win, rules, wends, width, out_dir, index, margin=0.03):
    """Writes the single-net path as copper and lets KiCad judge it.

    The path is searched again with the clearance grown by a margin, so the
    raster's rounding does not put the copper a hair too close."""
    import shutil

    for margin in (margin, margin / 2):
        strict = dict(rules)
        strict["clearance"] = rules["clearance"] + margin
        res, search, seeds, path, _ = analyse(win, strict, wends, width)
        if res.get("status") == "path_with_all_copper_kept":
            break
    else:
        return {"judged": False, "why": "no path once the clearance is grown by the raster's margin of %.3f mm" % margin}
    ok = ~(search.fixed_block | search.routed_block)
    runs = simplify(path, ok, win.px)
    src = os.path.dirname(os.path.abspath(board_path))
    stem = os.path.splitext(os.path.basename(board_path))[0]
    base_dir = os.path.join(out_dir, "verify-base")
    cand_dir = os.path.join(out_dir, f"verify-item{index}")
    for d in (base_dir, cand_dir):
        os.makedirs(d, exist_ok=True)
        for ext in (".kicad_pro", ".kicad_dru", ".kicad_prl", ".kicad_pcb"):
            f = os.path.join(src, stem + ext)
            if os.path.exists(f) and not os.path.exists(os.path.join(d, stem + ext)):
                shutil.copy(f, d)
    base_report = os.path.join(base_dir, "drc.json")
    if os.path.exists(base_report + ".counts"):
        base = json.load(open(base_report + ".counts"))
    else:
        base = kicad_drc(os.path.join(base_dir, stem + ".kicad_pcb"), base_report)
        json.dump(base, open(base_report + ".counts", "w"))
    board = pcbnew.LoadBoard(os.path.join(cand_dir, stem + ".kicad_pcb"))
    net = board.FindNet(win.net)

    def at(y, x):
        return pcbnew.VECTOR2I(nm(win.box[0] + (x + 0.5) * win.px), nm(win.box[1] + (y + 0.5) * win.px))

    segments = 0
    vias = 0
    last = None
    reach = width / win.px

    def stretched(a, b):
        """b moved on by a track width, to end well inside the copper it joins."""
        n = math.hypot(b[0] - a[0], b[1] - a[1])
        if n == 0:
            return b
        return (b[0] + (b[0] - a[0]) / n * reach, b[1] + (b[1] - a[1]) / n * reach)

    if len(runs[0][1]) > 1:
        runs[0][1][0] = stretched(runs[0][1][1], runs[0][1][0])
    if len(runs[-1][1]) > 1:
        runs[-1][1][-1] = stretched(runs[-1][1][-2], runs[-1][1][-1])
    for l, pts in runs:
        for a, b in zip(pts, pts[1:]):
            t = pcbnew.PCB_TRACK(board)
            t.SetStart(at(*a))
            t.SetEnd(at(*b))
            t.SetWidth(nm(width))
            t.SetLayer(win.layers[l])
            t.SetNet(net)
            board.Add(t)
            segments += 1
        last = pts[-1]
    for (l0, p0), (l1, p1) in zip(runs, runs[1:]):
        # Consecutive runs on adjacent layers at one pixel are one via.
        if p0[-1] == p1[0]:
            key = p0[-1]
            if vias and key == verify.last_via:
                continue
            v = pcbnew.PCB_VIA(board)
            v.SetPosition(at(*key))
            v.SetWidth(pcbnew.F_Cu, nm(rules["via_diameter"]))
            v.SetDrill(nm(rules["via_drill"]))
            v.SetViaType(pcbnew.VIATYPE_THROUGH)
            v.SetNet(net)
            board.Add(v)
            verify.last_via = key
            vias += 1
    pcbnew.SaveBoard(os.path.join(cand_dir, stem + ".kicad_pcb"), board)
    cand = kicad_drc(os.path.join(cand_dir, stem + ".kicad_pcb"), os.path.join(cand_dir, "drc.json"))
    new_errors = {t: c - base[1].get(t, 0) for t, c in cand[1].items() if c > base[1].get(t, 0)}
    return {
        "judged": True,
        "copper_added": {"segments": segments, "vias": vias, "width_mm": round(width, 3)},
        "kicad_unconnected_before": base[0],
        "kicad_unconnected_after": cand[0],
        "new_kicad_errors": new_errors,
        "accepted": cand[0] < base[0] and not new_errors,
        "board": os.path.join(cand_dir, stem + ".kicad_pcb"),
    }


verify.last_via = None


# -- the picture ------------------------------------------------------------

PALETTE = [
    (66, 165, 245),
    (239, 83, 80),
    (102, 187, 106),
    (171, 71, 188),
    (255, 138, 101),
    (38, 198, 218),
    (236, 64, 122),
    (156, 204, 101),
]


def top_nets(win, limit=8):
    counts = {}
    for l in range(len(win.layers)):
        ids, cnt = np.unique(win.routed_id[l], return_counts=True)
        for i, c in zip(ids, cnt):
            if i:
                counts[int(i)] = counts.get(int(i), 0) + int(c)
    order = sorted(counts, key=lambda i: -counts[i])
    return order[:limit], order[limit:]


def render(win, search, seeds, marks, path, out, item, plain, a_class):
    n = len(win.layers)
    used = [
        l
        for l in range(n)
        if win.own[l].any() or (win.routed_id[l] > 0).any() or l in (0, n - 1)
    ]
    if len(used) > 4:
        # Keep the outer layers and the two busiest inner ones.
        inner = sorted(used[1:-1], key=lambda l: -(win.routed_id[l] > 0).sum() - 5 * win.own[l].sum())[:2]
        used = sorted({used[0], used[-1], *inner})
    side = 760
    scale = side / max(win.w, win.h)
    pw, ph = int(win.w * scale), int(win.h * scale)
    cols = 2 if len(used) > 1 else 1
    rows = (len(used) + cols - 1) // cols
    legend_w = 330
    head = 64
    W = cols * (pw + 12) + legend_w + 12
    H = head + rows * (ph + 34) + 8
    img = Image.new("RGB", (W, H), (16, 20, 24))
    d = ImageDraw.Draw(img)
    f_big = ImageFont.truetype(FONT, 18)
    f = ImageFont.truetype(FONT, 14)
    f_small = ImageFont.truetype(FONT, 12)
    top, rest = top_nets(win)
    colour = {i: PALETTE[k % len(PALETTE)] for k, i in enumerate(top)}
    r = item["rules"]
    d.text(
        (10, 6),
        f"open item {item['item']}  net {item['net']}   A-B {item['distance_mm']} mm   class {r['class']}: track {r['track_width']} mm, clearance {r['clearance']} mm, via {r['via_diameter']} mm",
        font=f_big,
        fill=(235, 235, 235),
    )
    d.text(
        (10, 32),
        f"window x {win.box[0]:.2f}..{win.box[2]:.2f}  y {win.box[1]:.2f}..{win.box[3]:.2f} mm (y grows downwards, as in KiCad); one panel per copper layer, all seen from the top",
        font=f,
        fill=(170, 170, 170),
    )
    for k, l in enumerate(used):
        ox = 6 + (k % cols) * (pw + 12)
        oy = head + (k // cols) * (ph + 34)
        rgb = np.zeros((win.h, win.w, 3), np.uint8)
        rgb[:] = (30, 36, 42)
        rgb[~win.inside] = (0, 0, 0)
        rgb[win.other_zone[l] & win.inside] = (38, 52, 60)
        if not plain:
            blocked = (search.fixed_block[l] | search.routed_block[l]) & win.inside
            rgb[blocked & ~win.other_zone[l]] = (44, 40, 46)
            rgb[blocked & win.other_zone[l]] = (50, 52, 60)
        rid = win.routed_id[l]
        rgb[rid > 0] = (120, 124, 130)
        for i, c in colour.items():
            rgb[rid == i] = c
        fid = win.fixed_id[l]
        kinds = np.array([0] + [{"pad": 1, "graphic": 2, "keepout": 3}.get(o["kind"], 0) for o in win.objects[1:]])
        fk = kinds[fid]
        rgb[fk == 1] = (176, 136, 72)
        rgb[fk == 2] = (214, 60, 214)
        yy, xx = np.indices(fid.shape)
        hatch = ((xx + yy) // max(2, int(0.08 / win.px))) % 2 == 0
        rgb[(fk == 3) & hatch] = (214, 60, 214)
        rgb[win.own_zone[l]] = (150, 135, 20)
        rgb[win.own[l] & ~win.own_zone[l]] = (255, 235, 59)
        rgb[win.hole] = (0, 0, 0)
        panel = Image.fromarray(rgb).resize((pw, ph), Image.NEAREST)
        img.paste(panel, (ox, oy + 22))
        d.text((ox + 2, oy + 2), f"{win.names[l]}", font=f_big, fill=(255, 255, 255))
        d.rectangle([ox, oy + 22, ox + pw - 1, oy + 22 + ph - 1], outline=(90, 90, 90))

        def pt(x, y):
            return (ox + (x - win.box[0]) / win.px * scale, oy + 22 + (y - win.box[1]) / win.px * scale)

        # Pad labels.
        placed = []
        for ref, num, net, x, y, on, own in win.pads:
            if l not in on:
                continue
            if not (win.box[0] <= x <= win.box[2] and win.box[1] <= y <= win.box[3]):
                continue
            label = f"{ref}.{num}"
            px_, py_ = pt(x, y)
            tw = d.textlength(label, font=f_small)
            bx = [px_ - tw / 2, py_ - 7, px_ + tw / 2, py_ + 7]
            if any(not (bx[2] < q[0] or bx[0] > q[2] or bx[3] < q[1] or bx[1] > q[3]) for q in placed):
                continue
            if bx[0] < ox or bx[2] > ox + pw or bx[1] < oy + 22 or bx[3] > oy + 22 + ph:
                continue
            placed.append(bx)
            d.text((bx[0], bx[1]), label, font=f_small, fill=(0, 0, 0) if own else (20, 12, 0), stroke_width=2, stroke_fill=(255, 235, 59) if own else (222, 190, 130))
        # The path found (annotated variant only).
        if path:
            pts = [pt(win.box[0] + (x + 0.5) * win.px, win.box[1] + (y + 0.5) * win.px) for ll, y, x in path if ll == l]
            seg = []
            prev = None
            for (ll, y, x) in path:
                if ll == l:
                    p_ = pt(win.box[0] + (x + 0.5) * win.px, win.box[1] + (y + 0.5) * win.px)
                    if prev is not None:
                        seg.append((prev, p_))
                    prev = p_
                else:
                    prev = None
            for a_, b_ in seg:
                d.line([a_, b_], fill=(0, 255, 255), width=3)
            del pts
            for vy, vx in a_class.get("_via_at", []):
                cx, cy = pt(win.box[0] + (vx + 0.5) * win.px, win.box[1] + (vy + 0.5) * win.px)
                rr = item["rules"]["via_diameter"] / 2 / win.px * scale
                d.ellipse([cx - rr, cy - rr, cx + rr, cy + rr], outline=(0, 255, 255), width=3)
        # Ends.
        for mi, (name, pos) in enumerate(marks):
            x, y = pt(pos[0], pos[1])
            here = seeds[mi if len(marks) == 2 else 0][l].any()
            if here:
                d.ellipse([x - 9, y - 9, x + 9, y + 9], outline=(255, 255, 255), width=3)
                d.text((x + 11, y - 22), name, font=f_big, fill=(255, 255, 255), stroke_width=3, stroke_fill=(200, 0, 0))
            else:
                d.line([x - 6, y, x + 6, y], fill=(200, 200, 200), width=1)
                d.line([x, y - 6, x, y + 6], fill=(200, 200, 200), width=1)
        if len(marks) == 2:
            a_, b_ = pt(*marks[0][1]), pt(*marks[1][1])
            steps = max(2, int(math.hypot(a_[0] - b_[0], a_[1] - b_[1]) / 10))
            for s in range(0, steps, 2):
                p0 = (a_[0] + (b_[0] - a_[0]) * s / steps, a_[1] + (b_[1] - a_[1]) * s / steps)
                p1 = (a_[0] + (b_[0] - a_[0]) * (s + 1) / steps, a_[1] + (b_[1] - a_[1]) * (s + 1) / steps)
                d.line([p0, p1], fill=(255, 255, 255), width=2)
        # A 1 mm scale bar.
        bar = 1.0 / win.px * scale
        d.line([ox + 10, oy + 22 + ph - 12, ox + 10 + bar, oy + 22 + ph - 12], fill=(255, 255, 255), width=3)
        d.text((ox + 14 + bar, oy + 22 + ph - 20), "1 mm", font=f_small, fill=(255, 255, 255))
    # Legend.
    lx = cols * (pw + 12) + 12
    ly = head + 4
    def row(col, text, hatch=False):
        nonlocal ly
        d.rectangle([lx, ly + 2, lx + 22, ly + 14], fill=col, outline=(200, 200, 200))
        d.text((lx + 30, ly), text[:38], font=f, fill=(230, 230, 230))
        ly += 20
    row((255, 235, 59), f"{item['net']} (the open net)")
    if win.own_zone.any():
        row((150, 135, 20), f"{item['net']} pour (filled)")
    row((176, 136, 72), "pads of other nets")
    if any(o and o["kind"] == "graphic" for o in win.objects[1:]):
        row((214, 60, 214), "copper graphic (no net)")
    if any(o and o["kind"] == "keepout" for o in win.objects[1:]):
        row((214, 60, 214), "keepout (hatched)")
    if win.other_zone.any():
        row((38, 52, 60), "other nets' pours (refill)")
    if not plain:
        row((44, 40, 46), "too close for this net's track")
        if path:
            row((0, 255, 255), "cheapest path (ring: a new via)")
    row((0, 0, 0), "hole / off the board")
    ly += 6
    d.text((lx, ly), "tracks and vias of other nets:", font=f, fill=(200, 200, 200))
    ly += 20
    for i in top:
        row(colour[i], win.objects[i]["name"])
    if rest:
        row((120, 124, 130), f"{len(rest)} more nets")
    ly += 6
    for text in (
        "white rings A, B: the item's ends,",
        "on the layers where that end has",
        "copper (a small cross elsewhere);",
        "dashed: the missing connection",
    ):
        d.text((lx, ly), text, font=f, fill=(200, 200, 200))
        ly += 18
    img.save(out)


# -- the text map -----------------------------------------------------------


def ascii_map(win, seeds, marks, item, cell=0.1):
    """The window as characters, one block per copper layer."""
    step = max(1, int(round(cell / win.px)))
    cell = step * win.px
    top, rest = top_nets(win, 20)
    letters = {i: chr(ord("a") + k) for k, i in enumerate(top)}
    r = item["rules"]
    lines = [
        f"open item {item['item']} net {item['net']}: A-B {item['distance_mm']} mm apart; class {r['class']}: track {r['track_width']} mm, clearance {r['clearance']} mm, via {r['via_diameter']} mm",
        f"window x {win.box[0]:.2f}..{win.box[2]:.2f} y {win.box[1]:.2f}..{win.box[3]:.2f} mm; one character = {cell:.2f} mm; y grows downwards; all layers seen from the top",
        "legend: = the open net's copper   ~ the open net's pour   A B the item's ends   # pad of another net",
        "        X copper graphic or keepout (no net)   o hole   ' ' off the board   . free   : another net's pour (refills)",
        "        letters: tracks and vias of other nets: "
        + ", ".join(f"{letters[i]}={win.objects[i]['name']}" for i in top)
        + (f", ?={len(rest)} more nets" if rest else ""),
        "",
    ]
    hh, ww = win.h // step, win.w // step
    kinds = np.array([0] + [{"pad": 1, "graphic": 2, "keepout": 2}.get(o["kind"], 0) for o in win.objects[1:]])
    for l in range(len(win.layers)):
        if not (win.own[l].any() or (win.routed_id[l] > 0).any() or (win.fixed_id[l] > 0).any()):
            continue
        grid = [["." for _ in range(ww)] for _ in range(hh)]
        for gy in range(hh):
            for gx in range(ww):
                sl = np.s_[gy * step : (gy + 1) * step, gx * step : (gx + 1) * step]
                if win.own[l][sl].any():
                    ch = "~" if win.own_zone[l][sl].all() and not (win.own[l][sl] & ~win.own_zone[l][sl]).any() else "="
                elif (kinds[win.fixed_id[l][sl]] == 1).any():
                    ch = "#"
                elif (kinds[win.fixed_id[l][sl]] == 2).any():
                    ch = "X"
                elif (win.routed_id[l][sl] > 0).any():
                    ids = win.routed_id[l][sl]
                    i = int(np.bincount(ids[ids > 0]).argmax())
                    ch = letters.get(i, "?")
                elif win.hole[sl].any():
                    ch = "o"
                elif not win.inside[sl].any():
                    ch = " "
                elif win.other_zone[l][sl].any():
                    ch = ":"
                else:
                    ch = "."
                grid[gy][gx] = ch
        for name, pos in marks:
            gx = int((pos[0] - win.box[0]) / cell)
            gy = int((pos[1] - win.box[1]) / cell)
            if 0 <= gx < ww and 0 <= gy < hh:
                grid[gy][gx] = name
        lines.append(f"--- {win.names[l]} ---")
        lines.append("      " + "".join(str((k // 10) % 10) if k % 10 == 0 else " " for k in range(ww)))
        for gy in range(hh):
            lines.append(f"{win.box[1] + gy * cell:6.2f} " + "".join(grid[gy]))
        lines.append("")
    lines.append("pads in the window (x, y in mm; column, row in the map):")
    for ref, num, net, x, y, on, own in sorted(win.pads, key=lambda p: (p[4], p[3])):
        if not on or not (win.box[0] <= x <= win.box[2] and win.box[1] <= y <= win.box[3]):
            continue
        lines.append(
            f"  {ref}.{num:<4} net {net or '-':<24} at {x:.2f}, {y:.2f}  col {int((x - win.box[0]) / cell)} row {int((y - win.box[1]) / cell)}  on {','.join(win.names[l] for l in on)}"
        )
    return "\n".join(lines) + "\n"


# -- main -------------------------------------------------------------------


def load_board(path, refill=True):
    board = pcbnew.LoadBoard(path)
    if refill:
        pcbnew.ZONE_FILLER(board).Fill(board.Zones())
    return board


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("board")
    ap.add_argument("--drc", required=True, help="KiCad's DRC report (drc.json) of that board")
    ap.add_argument("--router", help="board-router.json of the run, for the router's own diagnosis")
    ap.add_argument("--out", required=True)
    ap.add_argument("--item", type=int, action="append", help="index into the report's unconnected_items")
    ap.add_argument("--formats", default="dossier,png,ascii")
    ap.add_argument("--without-net", action="append", help="treat this net's tracks and vias as removed")
    ap.add_argument("--margin", type=float, default=2.5)
    ap.add_argument("--plain", action="store_true", help="the picture without the search's overlays")
    ap.add_argument("--no-refill", action="store_true")
    ap.add_argument("--verify", action="store_true", help="write a single-net path as copper and have KiCad's DRC judge it")
    args = ap.parse_args()
    drc = json.load(open(args.drc))
    router = json.load(open(args.router)) if args.router else None
    board = load_board(args.board, not args.no_refill)
    items = drc.get("unconnected_items", [])
    summary = []
    for i, it in enumerate(items):
        if args.item and i not in args.item:
            continue
        out = do_item(board, it, i, args, router)
        d = out["dossier"]
        row = {"item": i, "net": d["net"], "ends_apart_mm": d["ends_apart_mm"], "cause": d["cause"], "because": d["because"]}
        if "kicad_judgement" in d:
            row["kicad_judgement"] = d["kicad_judgement"]
        summary.append(row)
    json.dump(summary, sys.stdout, indent=1)
    print()


if __name__ == "__main__":
    main()
