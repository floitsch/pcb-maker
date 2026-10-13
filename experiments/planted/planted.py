# Copyright (C) 2026 Toit contributors.
"""A board under construction with a planted routing: real footprints
placed with courtyard and copper checks, nets, routes planted on the
lattice of grid.py (clearances by construction), netless blockers, and the
writer (KiCad board through pcbnew, plus the project with the net class).

Coordinates are mm, board origin at ORIGIN so that KiCad's page is happy.
Grid layers: 0 = F.Cu, 1.. = inner layers, last = B.Cu; even layers
prefer horizontal steps, odd vertical (as the router)."""

import json
import math
import random
from pathlib import Path

import numpy as np

import grid as G

ORIGIN = (100.0, 100.0)
ROOT = Path(__file__).resolve().parents[2]
LIBRARY = ROOT / "build/planted/fplib.kicad_pcb"


class Rules:
    def __init__(self, track=0.2, clearance=0.2, via_diameter=0.6, via_drill=0.3, edge=0.3,
                 hole_clearance=0.25, hole_to_hole=0.25):
        self.track = track
        self.clearance = clearance
        self.via_diameter = via_diameter
        self.via_drill = via_drill
        self.edge = edge
        self.hole_clearance = hole_clearance
        self.hole_to_hole = hole_to_hole

    def as_dict(self):
        return dict(self.__dict__)


def nm(value):
    return int(round(value * 1e6))


def mm(value):
    return value / 1e6


class Pad:
    def __init__(self, part, pad, number, shape, layers, hole, centre, copper):
        self.part = part
        self.kicad = pad
        self.number = number
        self.shape = shape
        self.layers = layers
        self.hole = hole  # drill radius or None
        self.centre = centre
        self.copper = copper  # False for an NPTH hole without copper
        self.net = None


class Part:
    def __init__(self, footprint, name, category, reference):
        self.fp = footprint
        self.name = name
        self.category = category
        self.reference = reference
        self.pads = []
        self.courtyard = []

    def groups(self):
        """Pads by number: pads sharing a number are one terminal (an
        exposed pad's pieces)."""
        out = {}
        for pad in self.pads:
            out.setdefault(pad.number, []).append(pad)
        return out


class Library:
    def __init__(self, path=LIBRARY):
        import pcbnew
        self.board = pcbnew.LoadBoard(str(path))
        self.index = json.loads(Path(path).with_suffix(".json").read_text())
        self.footprints = {}
        for fp in self.board.GetFootprints():
            self.footprints[str(fp.GetValue())] = fp

    def names(self, category):
        return [n for n, v in self.index.items() if v["category"] == category]


class PlantBoard:
    def __init__(self, library, width, height, layers, rules, pitch=0.1, seed=1, corner=0.0):
        import pcbnew
        self.pcbnew = pcbnew
        self.library = library
        self.width = width
        self.height = height
        self.layer_count = layers
        self.rules = rules
        self.rng = random.Random(seed)
        self.board = pcbnew.BOARD()
        self.board.SetCopperLayerCount(layers)
        x0, y0 = ORIGIN
        self.grid = G.Grid(x0, y0, x0 + width, y0 + height, pitch, layers)
        p = pitch
        # How far a chord between two legal nodes can dip into a stamp.
        r0 = rules.via_diameter / 2 + rules.clearance + rules.via_diameter / 2
        self.delta = r0 - math.sqrt(r0 * r0 - (p / math.sqrt(2)) ** 2) + 0.002
        self.parts = []
        self.nets = []  # names; id = index
        self.copper = {}  # net id -> {"edges": set((layer, a, b)), "vias": set(node)}
        self.connections = []  # planted connection records
        self.trees = {}  # net -> states of its copper and reached pads
        self.planes = {}  # grid layer -> net of a plane pour covering the board
        self.pad_states = {}  # (net, state) -> pad whose terminal the state is
        self.stubs = set()  # (net, state, id(pad)) from a path's end to its pad's centre
        self.stub_pads = {}
        self.blockers = []
        self._stamp_outline()

    # Layers ----------------------------------------------------------------
    def kicad_layer(self, index):
        pcbnew = self.pcbnew
        if index == 0:
            return pcbnew.F_Cu
        if index == self.layer_count - 1:
            return pcbnew.B_Cu
        return [pcbnew.In1_Cu, pcbnew.In2_Cu, pcbnew.In3_Cu, pcbnew.In4_Cu][index - 1]

    def grid_layers(self, kicad_layers):
        out = []
        for i in range(self.layer_count):
            if self.kicad_layer(i) in kicad_layers:
                out.append(i)
        return out

    # Outline ---------------------------------------------------------------
    def _stamp_outline(self):
        g = self.grid
        r = self.rules
        X, Y = np.meshgrid(g.x0 + np.arange(g.nx) * g.pitch, g.y0 + np.arange(g.ny) * g.pitch)
        inner = np.minimum.reduce([X - g.x0, g.x0 + self.width - X, Y - g.y0, g.y0 + self.height - Y])
        track_block = inner < r.edge + r.track / 2 + self.delta
        via_block = inner < r.edge + r.via_diameter / 2 + self.delta
        for layer in range(self.layer_count):
            g.trk[layer][track_block] = G.BLOCK
        g.via[via_block] = G.BLOCK
        self.edge_distance = inner

    # Parts -----------------------------------------------------------------
    def _pad_info(self, part, pad):
        pcbnew = self.pcbnew
        layers = self.grid_layers(list(pad.GetLayerSet().CuStack()))
        attribute = pad.GetAttribute()
        hole = None
        if attribute in (pcbnew.PAD_ATTRIB_PTH, pcbnew.PAD_ATTRIB_NPTH):
            d = pad.GetDrillSize()
            hole = mm(max(d.x, d.y)) / 2
        copper = True
        if attribute == pcbnew.PAD_ATTRIB_NPTH:
            size = pad.GetSize(pcbnew.F_Cu)
            copper = mm(min(size.x, size.y)) / 2 > (hole or 0) + 0.01
            if not copper:
                layers = list(range(self.layer_count))
        if attribute == pcbnew.PAD_ATTRIB_PTH:
            layers = list(range(self.layer_count))
        probe = pcbnew.F_Cu if not layers or 0 in layers else self.kicad_layer(layers[0])
        poly = pcbnew.SHAPE_POLY_SET()
        pad.TransformShapeToPolygon(poly, probe, 0, nm(0.002), pcbnew.ERROR_OUTSIDE)
        line = poly.Outline(0)
        points = [(mm(line.CPoint(i).x), mm(line.CPoint(i).y)) for i in range(line.PointCount())]
        shape = G.Shape("poly", points=points)
        position = pad.GetPosition()
        return Pad(part, pad, str(pad.GetNumber()), shape, layers, hole, (mm(position.x), mm(position.y)), copper)

    def courtyard_polygons(self, fp):
        pcbnew = self.pcbnew
        fp.BuildCourtyardCaches()
        polys = []
        for side in (pcbnew.F_CrtYd, pcbnew.B_CrtYd):
            c = fp.GetCourtyard(side)
            for i in range(c.OutlineCount()):
                line = c.Outline(i)
                polys.append([(mm(line.CPoint(k).x), mm(line.CPoint(k).y)) for k in range(line.PointCount())])
        if not polys:
            box = fp.GetBoundingBox(False)
            x0, y0, x1, y1 = mm(box.GetLeft()), mm(box.GetTop()), mm(box.GetRight()), mm(box.GetBottom())
            polys.append([(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
        return polys

    def try_place(self, name, x, y, rotation, nets=None, margin=0.0, reference=None):
        """Places footprint `name` at (x, y) when its courtyard is free, it
        is inside the outline, and its copper keeps the clearance (plus
        `margin`) from all copper already there. `nets` maps pad numbers to
        net ids (others netless or assigned later). Returns the Part."""
        pcbnew = self.pcbnew
        g = self.grid
        source = self.library.footprints[name]
        fp = pcbnew.FOOTPRINT(source)
        fp.SetPosition(pcbnew.VECTOR2I(nm(x), nm(y)))
        fp.SetOrientationDegrees(rotation)
        courtyards = self.courtyard_polygons(fp)
        # Courtyard inside the board (with the edge clearance) and free.
        masks = []
        for poly in courtyards:
            shape = G.Shape("poly", points=poly)
            b = shape.box
            if b[0] < g.x0 + 0.2 or b[1] < g.y0 + 0.2 or b[2] > g.x0 + self.width - 0.2 or b[3] > g.y0 + self.height - 0.2:
                return None
            index, m = g.mask(shape, 0.06)
            if index is None or (g.court[0][index] & m).any():
                return None
            masks.append((index, m))
        part = Part(fp, name, self.library.index[name]["category"], reference or f"P{len(self.parts) + 1}")
        part.courtyard = courtyards
        for pad in fp.Pads():
            part.pads.append(self._pad_info(part, pad))
        r = self.rules
        nets = nets or {}
        for pad in part.pads:
            net = nets.get(pad.number)
            if not self._pad_fits(pad, net, margin):
                return None
        for index, m in masks:
            g.court[0][index] |= m
        fp.SetReference(part.reference)
        self.board.Add(fp)
        self.parts.append(part)
        for pad in part.pads:
            pad.net = nets.get(pad.number)
        return part

    def _pad_fits(self, pad, net, margin):
        """The pad's copper keeps the clearance from every other copper on
        its layers, and its hole the hole rules."""
        g = self.grid
        r = self.rules
        grow = r.clearance + margin + g.pitch * 0.75
        index, m = g.mask(pad.shape if pad.copper else G.Shape("disc", centre=pad.centre, radius=pad.hole),
                          grow + (r.hole_clearance - r.clearance if not pad.copper else 0.0))
        if index is None:
            return False
        for layer in pad.layers:
            if (g.copper[layer][index] & m).any():
                # Own net's copper may touch: compare owners.
                if net is None:
                    return False
                owners = g.trk[layer][index][m & g.copper[layer][index]]
                if ((owners != net)).any():
                    return False
        # Outline: copper edge clearance.
        if pad.shape.box[0] < g.x0 + r.edge + 0.05 or pad.shape.box[1] < g.y0 + r.edge + 0.05 or \
                pad.shape.box[2] > g.x0 + self.width - r.edge - 0.05 or pad.shape.box[3] > g.y0 + self.height - r.edge - 0.05:
            return False
        if pad.hole:
            # Hole to hole against vias already planted.
            hole = G.Shape("disc", centre=pad.centre, radius=pad.hole)
            index, m = g.mask(hole, r.hole_to_hole + r.via_drill / 2 + r.via_diameter / 2 + r.clearance)
            for vias in (self.copper[n]["vias"] for n in self.copper):
                for (ix, iy) in vias:
                    if index[0].start <= iy < index[0].stop and index[1].start <= ix < index[1].stop:
                        if m[iy - index[0].start, ix - index[1].start]:
                            return False
        return True

    def stamp_part(self, part):
        """Stamps a part's pads into the maps once their nets are known."""
        for pad in part.pads:
            self.stamp_pad(pad)

    def stamp_pad(self, pad):
        g = self.grid
        r = self.rules
        d = self.delta
        owner = G.BLOCK if pad.net is None else pad.net
        if pad.copper:
            g.stamp_layers(pad.layers, pad.shape, r.clearance + r.track / 2 + d, owner)
            g.stamp(g.via, pad.shape, r.clearance + r.via_diameter / 2 + d, owner)
            lane = self.escape_lane(pad)
            if lane is not None and pad.net is not None:
                # Reserved as if the net's track were there: no other net
                # comes close enough to close the pad's way out.
                g.stamp_layers(pad.layers, lane, r.clearance + r.track / 2 + d, pad.net)
                g.stamp(g.via, lane, r.clearance + r.via_diameter / 2 + d, pad.net)
            for layer in pad.layers:
                g.mark(g.copper[layer], pad.shape, 0.75 * g.pitch)
                index, m = g.mask(pad.shape, 0.0)
                # The copper map records the owner through trk; a netless
                # pad's nodes are BLOCK already.
        if pad.hole:
            hole = G.Shape("disc", centre=pad.centre, radius=pad.hole)
            if not pad.copper:
                g.stamp_layers(range(self.layer_count), hole, r.hole_clearance + r.track / 2 + d, G.BLOCK)
                g.stamp(g.via, hole, r.hole_clearance + r.via_diameter / 2 + d, G.BLOCK)
                for layer in range(self.layer_count):
                    g.mark(g.copper[layer], hole, r.hole_clearance - r.clearance + 0.75 * g.pitch)
            g.stamp(g.via, hole, r.hole_to_hole + r.via_drill / 2 + d, G.BLOCK)

    def escape_lane(self, pad, beyond=0.6):
        """A straight lane out of an elongated surface pad of a part with
        six or more pads (fine-pitch rows), away from the part's centre."""
        if pad.hole or len(pad.part.pads) < 6:
            return None
        x0, y0, x1, y1 = pad.shape.box
        w, h = x1 - x0, y1 - y0
        if max(w, h) < 1.5 * min(w, h) or min(w, h) > 0.8:
            return None
        position = pad.part.fp.GetPosition()
        cx, cy = mm(position.x), mm(position.y)
        px, py = pad.centre
        if w > h:
            sign = 1 if px > cx else -1
            end = (x1 + beyond if sign > 0 else x0 - beyond, py)
        else:
            sign = 1 if py > cy else -1
            end = (px, y1 + beyond if sign > 0 else y0 - beyond)
        return G.Shape("capsule", a=(px, py), b=end, half=self.rules.track / 2)

    # Nets ------------------------------------------------------------------
    def add_net(self, name):
        self.nets.append(name)
        net = len(self.nets) - 1
        self.copper[net] = {"edges": set(), "vias": set()}
        return net

    def terminals(self, pad, net, layers=None):
        out = []
        for layer in pad.layers:
            if layers is not None and layer not in layers:
                continue
            out.extend(self.grid.inside_nodes(pad.shape, layer, net))
        return out

    def route_net(self, net, pads, wind=None, layer_cost=None, allowed_layers=None, via_cost=12.0,
                  max_expansions=300000):
        """Plants `net` over `pads` (Prim order from the first). `wind`:
        a function (A, B) -> waypoint (x, y) or None. Returns the pads
        connected (the first always) and the pads that could not be."""
        tree = self.trees.setdefault(net, set())
        connected = [pads[0]]
        self.tree_add_pad(net, tree, pads[0])
        if not tree:
            return [pads[0]], list(pads[1:])
        remaining = list(pads[1:])
        failed = []
        while remaining:
            best = None
            for pad in remaining:
                d = min(math.dist(pad.centre, c.centre) for c in connected)
                if best is None or d < best[0]:
                    best = (d, pad)
            pad = best[1]
            remaining.remove(pad)
            if self.connect(net, tree, pad, wind, layer_cost, via_cost, allowed_layers, max_expansions) is None:
                failed.append(pad)
            else:
                connected.append(pad)
        return connected, failed

    def connect(self, net, tree, pad, wind=None, layer_cost=None, via_cost=12.0, allowed_layers=None,
                max_expansions=300000):
        """Plants one connection from the net's `tree` (states, updated) to
        `pad`. Returns the connection record or None."""
        g = self.grid
        allowed_layers = allowed_layers or list(range(self.layer_count))
        layer_cost = layer_cost or [1.0] * self.layer_count
        every = self.signal_layers()
        all_targets = {t for t in self.terminals(pad, net) if t // g.layer_size in every}
        targets = {t for t in all_targets if t // g.layer_size in allowed_layers}
        if not all_targets or not tree:
            return None
        path = None
        if wind is not None and targets:
            start = min(tree, key=lambda s: self._dist2(s, pad.centre))
            waypoint = wind(self.state_point(start), pad.centre)
            if waypoint is not None:
                path = self._via_waypoint(net, tree, targets, pad.centre, waypoint, layer_cost, via_cost,
                                          allowed_layers, max_expansions)
        if path is not None and not self._vias_apart(path):
            path = None
        exhausted = False
        if path is None and targets:
            path = g.search(net, tree, targets, pad.centre, layer_cost, via_cost, allowed_layers,
                            max_expansions=max_expansions)
            exhausted = path is None and g.exhausted
            if path is not None and not self._vias_apart(path):
                path = None
        if path is None and not (exhausted and len(allowed_layers) == len(every)):
            # Every layer, more patience, dearer vias, sharp turns allowed.
            path = g.search(net, tree, all_targets, pad.centre, [1.0] * self.layer_count,
                            via_cost * 2, every, max_expansions=max_expansions * 6, sharp=True)
            if path is not None:
                targets = all_targets
            if path is not None and not self._vias_apart(path):
                path = None
        if path is None:
            return None
        record = self.commit_path(net, path)
        record.update(pad_a=self.state_point(path[0]), pad_b=pad.centre)
        self.connections.append(record)
        tree.update(path)
        self.tree_add_pad(net, tree, pad, all_targets)
        self.stub(net, path[0])
        self.stub(net, path[-1])
        return record

    def tree_add_pad(self, net, tree, pad, states=None):
        states = self.terminals(pad, net) if states is None else states
        for s in states:
            self.pad_states.setdefault((net, s), pad)
        tree.update(states)

    def stub(self, net, state):
        """A path ending on a pad's terminal node is drawn on to the pad's
        centre, as KiCad and our router draw it (inside the pad)."""
        pad = self.pad_states.get((net, state))
        if pad is not None:
            self.stubs.add((net, state, id(pad)))
            self.stub_pads[id(pad)] = pad

    def signal_layers(self):
        return [l for l in range(self.layer_count) if l not in self.planes]

    def connect_to_plane(self, net, pad, via_cost, max_expansions=200000):
        """A pad of a plane net reaches its plane by a short stub and a via
        (a through-hole pad already sits in it)."""
        g = self.grid
        layers = [l for l, n in self.planes.items() if n == net]
        if not layers:
            return None
        plane = layers[0]
        if plane in pad.layers:
            return {"net": net, "length_mm": 0.0, "vias": 0, "nodes": 0, "added": ([], [])}
        sources = self.terminals(pad, net)
        if not sources:
            return None

        class OnPlane:
            def __contains__(self, s):
                return s // g.layer_size == plane

        allowed = sorted(set(pad.layers) | {plane})
        cost = [1.0] * self.layer_count
        path = g.search(net, sources, OnPlane(), pad.centre, cost, via_cost, allowed,
                        max_expansions=max_expansions, weight=0.0)
        if path is None or not self._vias_apart(path):
            return None
        record = self.commit_path(net, path)
        record.update(pad_a=pad.centre, pad_b=pad.centre, plane=True)
        self.connections.append(record)
        for st in sources:
            self.pad_states.setdefault((net, st), pad)
        self.stub(net, path[0])
        return record

    def _vias_apart(self, path):
        """The vias a path places keep the hole-to-hole distance from each
        other (the search does not see its own vias)."""
        g = self.grid
        r = self.rules
        vias = []
        for a, b in zip(path, path[1:]):
            la, ax, ay = g.state_xy(a)
            lb, bx, by = g.state_xy(b)
            if la != lb and (ax, ay) not in vias:
                vias.append((ax, ay))
        limit = (r.hole_to_hole + r.via_drill + 0.01) / g.pitch
        for i in range(len(vias)):
            for j in range(i + 1, len(vias)):
                if math.dist(vias[i], vias[j]) < limit:
                    return False
        return True

    def _dist2(self, s, point):
        layer, ix, iy = self.grid.state_xy(s)
        return (self.grid.x(ix) - point[0]) ** 2 + (self.grid.y(iy) - point[1]) ** 2

    def state_point(self, s):
        layer, ix, iy = self.grid.state_xy(s)
        return (self.grid.x(ix), self.grid.y(iy))

    def _via_waypoint(self, net, tree, targets, goal, waypoint, layer_cost, via_cost, allowed_layers, max_expansions):
        g = self.grid
        ix, iy = g.node(*waypoint)
        if not (0 <= ix < g.nx and 0 <= iy < g.ny):
            return None
        mid = set()
        for layer in allowed_layers:
            s = layer * g.layer_size + iy * g.nx + ix
            v = g.trk.reshape(-1)[s]
            if v == G.FREE or v == net:
                mid.add(s)
        if not mid:
            return None
        first = g.search(net, tree, mid, waypoint, layer_cost, via_cost, allowed_layers, max_expansions=max_expansions)
        if first is None:
            return None
        forbid = set(first[:-1])
        second = g.search(net, [first[-1]], targets, goal, layer_cost, via_cost, allowed_layers,
                          forbid=forbid, max_expansions=max_expansions)
        if second is None:
            return None
        return first + second[1:]

    def commit_path(self, net, path):
        """Adds a path's copper to the net and stamps it."""
        g = self.grid
        r = self.rules
        d = self.delta
        copper = self.copper[net]
        length = 0.0
        vias = 0
        added_edges = []
        added_vias = []
        runs = []  # (layer, [nodes])
        current = None
        for a, b in zip(path, path[1:]):
            la, ax, ay = g.state_xy(a)
            lb, bx, by = g.state_xy(b)
            if la != lb:
                if (ax, ay) not in copper["vias"]:
                    copper["vias"].add((ax, ay))
                    added_vias.append((ax, ay))
                    vias += 1
                    disc = G.Shape("disc", centre=(g.x(ax), g.y(ay)), radius=r.via_diameter / 2)
                    g.stamp_layers(range(self.layer_count), disc, r.clearance + r.track / 2 + d, net)
                    g.stamp(g.via, disc, r.clearance + r.via_diameter / 2 + d, net)
                    g.stamp(g.via, G.Shape("disc", centre=disc.centre, radius=r.via_drill / 2),
                            r.hole_to_hole + r.via_drill / 2 + d, G.BLOCK)
                    for layer in range(self.layer_count):
                        g.mark(g.copper[layer], disc, 0.75 * g.pitch)
                current = None
                continue
            edge = (la, (ax, ay), (bx, by)) if (ax, ay) <= (bx, by) else (la, (bx, by), (ax, ay))
            if edge not in copper["edges"]:
                copper["edges"].add(edge)
                added_edges.append(edge)
            length += math.hypot(bx - ax, by - ay) * g.pitch
            if current is None or current[0] != la:
                current = (la, [(ax, ay)])
                runs.append(current)
            current[1].append((bx, by))
        for layer, nodes in runs:
            for (ax, ay), (bx, by) in self._straight(nodes):
                capsule = G.Shape("capsule", a=(g.x(ax), g.y(ay)), b=(g.x(bx), g.y(by)), half=r.track / 2)
                g.stamp_layers([layer], capsule, r.clearance + r.track / 2 + d, net)
                g.stamp(g.via, capsule, r.clearance + r.via_diameter / 2 + d, net)
                g.mark(g.copper[layer], capsule, 0.75 * g.pitch)
        return {"net": net, "length_mm": round(length, 3), "vias": vias, "nodes": len(path),
                "added": (added_edges, added_vias)}

    @staticmethod
    def _straight(nodes):
        """Maximal straight runs of a node chain."""
        out = []
        start = 0
        for i in range(1, len(nodes) - 1):
            d1 = (nodes[i][0] - nodes[i - 1][0], nodes[i][1] - nodes[i - 1][1])
            d2 = (nodes[i + 1][0] - nodes[i][0], nodes[i + 1][1] - nodes[i][1])
            if d1 != d2:
                out.append((nodes[start], nodes[i]))
                start = i
        if len(nodes) >= 2:
            out.append((nodes[start], nodes[-1]))
        return out

    # Blockers --------------------------------------------------------------
    def try_blocker(self, name, x, y, rotation, margin):
        """A netless part whose copper keeps `margin` beyond the clearance
        from every copper."""
        part = self.try_place(name, x, y, rotation, nets={}, margin=margin, reference=f"BLK{len(self.blockers) + 1}")
        if part is None:
            return None
        self.stamp_part(part)
        self.blockers.append(part)
        return part

    # Output ----------------------------------------------------------------
    def segments(self, net):
        """The net's edges merged into straight segments, split at every
        junction (a node of degree other than 2) and via."""
        g = self.grid
        copper = self.copper[net]
        out = []
        by_layer = {}
        for layer, a, b in copper["edges"]:
            by_layer.setdefault(layer, set()).add((a, b))
        for layer, edges in by_layer.items():
            neighbours = {}
            for a, b in edges:
                neighbours.setdefault(a, []).append(b)
                neighbours.setdefault(b, []).append(a)
            used = set()

            def breaks(n):
                return len(neighbours[n]) != 2 or n in copper["vias"]

            for a, b in sorted(edges):
                if (a, b) in used:
                    continue
                # Grow the chain both ways while straight and unbroken.
                direction = (b[0] - a[0], b[1] - a[1])
                start, end = a, b
                used.add((a, b))
                while not breaks(end):
                    nxt = (end[0] + direction[0], end[1] + direction[1])
                    key = (end, nxt) if end <= nxt else (nxt, end)
                    if nxt not in neighbours[end] or key in used:
                        break
                    used.add(key)
                    end = nxt
                while not breaks(start):
                    prv = (start[0] - direction[0], start[1] - direction[1])
                    key = (prv, start) if prv <= start else (start, prv)
                    if prv not in neighbours[start] or key in used:
                        break
                    used.add(key)
                    start = prv
                out.append((layer, (g.x(start[0]), g.y(start[1])), (g.x(end[0]), g.y(end[1]))))
        return out

    def write(self, directory, board_id, extra=None):
        pcbnew = self.pcbnew
        r = self.rules
        directory = Path(directory)
        directory.mkdir(parents=True, exist_ok=True)
        board = self.board
        # A net with a single terminal (a pin nothing joined) is written
        # without a net, as KiCad leaves an unconnected pin.
        terminals = {}
        for part in self.parts:
            for pad in part.pads:
                if pad.net is not None:
                    terminals.setdefault(pad.net, set()).add((id(part), pad.number))
        infos = {}
        for net, name in enumerate(self.nets):
            if len(terminals.get(net, ())) < 2 and net not in self.planes.values():
                continue
            info = pcbnew.NETINFO_ITEM(board, name)
            board.Add(info)
            infos[net] = info
        for part in self.parts:
            for pad in part.pads:
                if pad.net is not None and pad.net in infos:
                    pad.kicad.SetNet(infos[pad.net])
                else:
                    pad.kicad.SetNetCode(0)
        x0, y0 = ORIGIN
        corners = [(x0, y0), (x0 + self.width, y0), (x0 + self.width, y0 + self.height), (x0, y0 + self.height)]
        for (ax, ay), (bx, by) in zip(corners, corners[1:] + corners[:1]):
            line = pcbnew.PCB_SHAPE(board)
            line.SetShape(pcbnew.SHAPE_T_SEGMENT)
            line.SetStart(pcbnew.VECTOR2I(nm(ax), nm(ay)))
            line.SetEnd(pcbnew.VECTOR2I(nm(bx), nm(by)))
            line.SetLayer(pcbnew.Edge_Cuts)
            line.SetWidth(nm(0.05))
            board.Add(line)
        total_length = 0.0
        total_vias = 0
        for net in range(len(self.nets)):
            if net not in infos:
                continue
            for layer, a, b in self.segments(net):
                track = pcbnew.PCB_TRACK(board)
                track.SetStart(pcbnew.VECTOR2I(nm(a[0]), nm(a[1])))
                track.SetEnd(pcbnew.VECTOR2I(nm(b[0]), nm(b[1])))
                track.SetWidth(nm(r.track))
                track.SetLayer(self.kicad_layer(layer))
                track.SetNet(infos[net])
                board.Add(track)
                total_length += math.dist(a, b)
            for stub_net, state, key in sorted(self.stubs):
                if stub_net != net:
                    continue
                layer, ix, iy = self.grid.state_xy(state)
                pad = self.stub_pads[key]
                if layer not in pad.layers or not pad.copper:
                    continue
                a = (self.grid.x(ix), self.grid.y(iy))
                b = pad.centre
                if math.dist(a, b) < 0.001:
                    continue
                track = pcbnew.PCB_TRACK(board)
                track.SetStart(pcbnew.VECTOR2I(nm(a[0]), nm(a[1])))
                track.SetEnd(pcbnew.VECTOR2I(nm(b[0]), nm(b[1])))
                track.SetWidth(nm(r.track))
                track.SetLayer(self.kicad_layer(layer))
                track.SetNet(infos[net])
                board.Add(track)
                total_length += math.dist(a, b)
            for ix, iy in self.copper[net]["vias"]:
                via = pcbnew.PCB_VIA(board)
                via.SetViaType(pcbnew.VIATYPE_THROUGH)
                via.SetPosition(pcbnew.VECTOR2I(nm(self.grid.x(ix)), nm(self.grid.y(iy))))
                via.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
                via.SetWidth(nm(r.via_diameter))
                via.SetDrill(nm(r.via_drill))
                via.SetNet(infos[net])
                board.Add(via)
                total_vias += 1
        for layer, net in self.planes.items():
            zone = pcbnew.ZONE(board)
            zone.SetLayer(self.kicad_layer(layer))
            zone.SetNet(infos[net])
            inset = r.edge
            outline = zone.Outline()
            outline.NewOutline()
            for cx, cy in [(x0 + inset, y0 + inset), (x0 + self.width - inset, y0 + inset),
                           (x0 + self.width - inset, y0 + self.height - inset), (x0 + inset, y0 + self.height - inset)]:
                outline.Append(nm(cx), nm(cy))
            zone.SetLocalClearance(nm(r.clearance))
            zone.SetMinThickness(nm(0.2))
            zone.SetThermalReliefGap(nm(0.3))
            zone.SetThermalReliefSpokeWidth(nm(0.4))
            zone.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
            board.Add(zone)
        path = directory / f"{board_id}.kicad_pcb"
        pcbnew.SaveBoard(str(path), board)
        (directory / f"{board_id}.kicad_pro").write_text(json.dumps(self.project(board_id), indent=2))
        prl = directory / f"{board_id}.kicad_prl"
        if prl.exists():
            prl.unlink()
        return {"length_mm": round(total_length, 3), "vias": total_vias}

    def project(self, board_id):
        r = self.rules
        return {
            "board": {"design_settings": {
                "defaults": {},
                "rules": {
                    "min_clearance": r.clearance, "min_track_width": r.track,
                    "min_copper_edge_clearance": r.edge, "min_hole_clearance": r.hole_clearance,
                    "min_hole_to_hole": r.hole_to_hole, "min_via_diameter": r.via_diameter,
                    "min_through_hole_diameter": r.via_drill, "min_via_annular_width": 0.1,
                    "min_resolved_spokes": 2, "allow_blind_buried_vias": False, "allow_microvias": False,
                    "max_error": 0.005, "min_connection": 0.0, "min_silk_clearance": 0.0,
                    "solder_mask_clearance": 0.0, "solder_mask_min_width": 0.0,
                    "solder_mask_to_copper_clearance": 0.0},
                "rule_severities": {},
                "track_widths": [], "via_dimensions": []}},
            "meta": {"filename": f"{board_id}.kicad_pro", "version": 1},
            "net_settings": {
                "classes": [{"name": "Default", "clearance": r.clearance, "track_width": r.track,
                             "via_diameter": r.via_diameter, "via_drill": r.via_drill,
                             "microvia_diameter": 0.3, "microvia_drill": 0.1, "diff_pair_gap": 0.25,
                             "diff_pair_via_gap": 0.25, "diff_pair_width": 0.2, "wire_width": 6,
                             "bus_width": 12, "line_style": 0, "priority": 2147483647,
                             "pcb_color": "rgba(0, 0, 0, 0.000)", "schematic_color": "rgba(0, 0, 0, 0.000)"}],
                "meta": {"version": 4}, "net_colors": None, "netclass_assignments": None,
                "netclass_patterns": []},
        }
