# Copyright (C) 2026 Toit contributors.
"""The planting lattice: per-layer ownership maps, exact stamping of copper
clearance zones, and an A* search over (layer, node).

A node of a layer's track map holds FREE, BLOCK, or the one net whose
centreline may pass there (a node inside another net's clearance zone is
BLOCK for everyone but that net). Stamps are distance tests against the
real shape (polygon, capsule, disc), grown by `delta`: on a lattice with
pitch p a chord between two nodes that are both outside a union of discs of
radius R dips into it by at most R - sqrt(R^2 - (p/sqrt 2)^2), so stamping
R + delta keeps every segment between two legal nodes legal. The via map
is one 2-D map (through vias): a via may sit where every layer allows a
via's ring."""

import heapq
import math

import numpy as np

FREE = -1
BLOCK = -2


def segment_distance(X, Y, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    length2 = dx * dx + dy * dy
    if length2 == 0:
        return np.hypot(X - ax, Y - ay)
    t = np.clip(((X - ax) * dx + (Y - ay) * dy) / length2, 0.0, 1.0)
    return np.hypot(X - (ax + t * dx), Y - (ay + t * dy))


def polygon_distance(X, Y, points):
    """Distance to a closed polygon (0 inside)."""
    distance = np.full(X.shape, np.inf)
    inside = np.zeros(X.shape, dtype=bool)
    count = len(points)
    for index in range(count):
        ax, ay = points[index]
        bx, by = points[(index + 1) % count]
        distance = np.minimum(distance, segment_distance(X, Y, ax, ay, bx, by))
        crosses = ((ay > Y) != (by > Y))
        with np.errstate(divide="ignore", invalid="ignore"):
            xcross = ax + (Y - ay) * (bx - ax) / (by - ay)
        inside ^= crosses & (X < xcross)
    distance[inside] = 0.0
    return distance


class Shape:
    """A copper or keep-out shape in mm: kind 'poly' (points), 'capsule'
    (a, b, half width) or 'disc' (centre, radius)."""

    def __init__(self, kind, **data):
        self.kind = kind
        self.__dict__.update(data)
        if kind == "poly":
            xs = [p[0] for p in self.points]
            ys = [p[1] for p in self.points]
            self.box = (min(xs), min(ys), max(xs), max(ys))
        elif kind == "capsule":
            (ax, ay), (bx, by) = self.a, self.b
            r = self.half
            self.box = (min(ax, bx) - r, min(ay, by) - r, max(ax, bx) + r, max(ay, by) + r)
        else:
            (cx, cy), r = self.centre, self.radius
            self.box = (cx - r, cy - r, cx + r, cy + r)

    def distance(self, X, Y):
        if self.kind == "poly":
            return polygon_distance(X, Y, self.points)
        if self.kind == "capsule":
            (ax, ay), (bx, by) = self.a, self.b
            return np.maximum(segment_distance(X, Y, ax, ay, bx, by) - self.half, 0.0)
        (cx, cy), r = self.centre, self.radius
        return np.maximum(np.hypot(X - cx, Y - cy) - r, 0.0)


class Grid:
    def __init__(self, x0, y0, x1, y1, pitch, layers):
        self.pitch = pitch
        self.x0, self.y0 = x0, y0
        self.nx = int(math.floor((x1 - x0) / pitch)) + 1
        self.ny = int(math.floor((y1 - y0) / pitch)) + 1
        self.layers = layers
        self.trk = np.full((layers, self.ny, self.nx), FREE, dtype=np.int32)
        self.via = np.full((self.ny, self.nx), FREE, dtype=np.int32)
        # Placement maps: courtyards per side (0 front, 1 back) and the
        # copper keep-away used when blockers are dropped in.
        self.court = np.zeros((2, self.ny, self.nx), dtype=bool)
        self.copper = np.zeros((layers, self.ny, self.nx), dtype=bool)
        self.layer_size = self.ny * self.nx

    # Coordinates -------------------------------------------------------
    def x(self, ix):
        return self.x0 + ix * self.pitch

    def y(self, iy):
        return self.y0 + iy * self.pitch

    def node(self, x, y):
        return int(round((x - self.x0) / self.pitch)), int(round((y - self.y0) / self.pitch))

    def window(self, box, grow):
        ix0 = max(0, int(math.floor((box[0] - grow - self.x0) / self.pitch)))
        iy0 = max(0, int(math.floor((box[1] - grow - self.y0) / self.pitch)))
        ix1 = min(self.nx, int(math.ceil((box[2] + grow - self.x0) / self.pitch)) + 1)
        iy1 = min(self.ny, int(math.ceil((box[3] + grow - self.y0) / self.pitch)) + 1)
        if ix0 >= ix1 or iy0 >= iy1:
            return None
        X, Y = np.meshgrid(self.x0 + np.arange(ix0, ix1) * self.pitch,
                           self.y0 + np.arange(iy0, iy1) * self.pitch)
        return (slice(iy0, iy1), slice(ix0, ix1)), X, Y

    def mask(self, shape, radius):
        win = self.window(shape.box, radius)
        if win is None:
            return None, None
        index, X, Y = win
        return index, shape.distance(X, Y) <= radius

    # Stamping ------------------------------------------------------------
    @staticmethod
    def _apply(sub, m, net):
        if net == BLOCK:
            sub[m] = BLOCK
            return
        current = sub[m]
        sub[m] = np.where((current == FREE) | (current == net), net, BLOCK)

    def stamp(self, array, shape, radius, net):
        index, m = self.mask(shape, radius)
        if index is not None:
            self._apply(array[index], m, net)

    def stamp_layers(self, layers, shape, radius, net):
        index, m = self.mask(shape, radius)
        if index is None:
            return
        for layer in layers:
            self._apply(self.trk[layer][index], m, net)

    def mark(self, array, shape, radius):
        index, m = self.mask(shape, radius)
        if index is not None:
            array[index] |= m

    def inside_nodes(self, shape, layer, net):
        """Nodes of `layer` inside `shape` that `net` may use."""
        index, m = self.mask(shape, 0.0)
        if index is None:
            return []
        sub = self.trk[layer][index]
        ok = m & ((sub == FREE) | (sub == net))
        iy, ix = np.nonzero(ok)
        iy = iy + index[0].start
        ix = ix + index[1].start
        base = layer * self.layer_size
        return [base + int(a) * self.nx + int(b) for a, b in zip(iy, ix)]

    # Search --------------------------------------------------------------
    def state_xy(self, s):
        layer, rest = divmod(s, self.layer_size)
        iy, ix = divmod(rest, self.nx)
        return layer, ix, iy

    def search(self, net, sources, targets, goal_xy, layer_cost, via_cost, allowed_layers,
               forbid=None, max_expansions=400000, axis_penalty=0.15, weight=1.3, bend=0.6, sharp=False):
        """A* from `sources` (states) to any of `targets` (a set of states).
        `layer_cost[l]` multiplies a step's length on layer l; a step
        across the layer's preferred axis (even layers horizontal, odd
        vertical) pays `axis_penalty` more; a 45-degree turn costs `bend`
        (in pitches), a 90-degree turn twice that, sharper turns are not
        taken. The turn is judged against the direction of the node's best
        arrival (states are nodes, not node and direction: approximate,
        nine times cheaper). Returns the state path or None; `exhausted`
        says whether the search ran out of states (unreachable) rather
        than out of budget."""
        nx, ny, size = self.nx, self.ny, self.layer_size
        trk = self.trk.reshape(-1)
        via = self.via.reshape(-1)
        gx, gy = goal_xy
        gix = (gx - self.x0) / self.pitch
        giy = (gy - self.y0) / self.pitch
        cmin = min(layer_cost[l] for l in allowed_layers)
        allowed = set(allowed_layers)
        forbid = forbid or set()
        diag = math.sqrt(2.0)
        moves = [(1, 0, 1.0), (1, 1, diag), (0, 1, 1.0), (-1, 1, diag),
                 (-1, 0, 1.0), (-1, -1, diag), (0, -1, 1.0), (1, -1, diag)]
        turn = []
        for d in range(9):
            row = []
            for e in range(8):
                if d == 8:
                    row.append(0.0)
                    continue
                k = min((e - d) % 8, (d - e) % 8)
                row.append(0.0 if k == 0 else bend if k == 1 else 2 * bend if k == 2 else (4 * bend if sharp else None))
            turn.append(row)

        def h(ix, iy):
            dx = abs(ix - gix)
            dy = abs(iy - giy)
            return weight * cmin * (max(dx, dy) + (diag - 1.0) * min(dx, dy))

        g = {}
        parent = {}
        arrive = {}
        heap = []
        for s in sources:
            layer, rest = divmod(s, size)
            if layer not in allowed:
                continue
            iy, ix = divmod(rest, nx)
            g[s] = 0.0
            parent[s] = -1
            arrive[s] = 8
            heapq.heappush(heap, (h(ix, iy), 0.0, s))
        expansions = 0
        closed = set()
        self.exhausted = False
        while heap:
            f, gs, s = heapq.heappop(heap)
            if s in closed:
                continue
            closed.add(s)
            if s in targets:
                path = [s]
                while parent[path[-1]] != -1:
                    path.append(parent[path[-1]])
                path.reverse()
                return path
            expansions += 1
            if expansions > max_expansions:
                return None
            layer, rest = divmod(s, size)
            iy, ix = divmod(rest, nx)
            lc = layer_cost[layer]
            horizontal = layer % 2 == 0
            row = turn[arrive[s]]
            for e in range(8):
                tc = row[e]
                if tc is None:
                    continue
                dx, dy, length = moves[e]
                jx, jy = ix + dx, iy + dy
                if jx < 0 or jy < 0 or jx >= nx or jy >= ny:
                    continue
                t = s + dy * nx + dx
                if t in closed or t in forbid:
                    continue
                v = trk[t]
                if v != FREE and v != net:
                    continue
                step = length * lc + tc
                if (horizontal and dx == 0) or (not horizontal and dy == 0):
                    step += axis_penalty * lc
                ng = gs + step
                if ng < g.get(t, 1e30):
                    g[t] = ng
                    parent[t] = s
                    arrive[t] = e
                    heapq.heappush(heap, (ng + h(jx, jy), ng, t))
            # Through via: every layer at this node.
            v = via[rest]
            if (v == FREE or v == net) and len(allowed) > 1:
                for other in allowed:
                    if other == layer:
                        continue
                    t = other * size + rest
                    if t in closed or t in forbid:
                        continue
                    tv = trk[t]
                    if tv != FREE and tv != net:
                        continue
                    ng = gs + via_cost
                    if ng < g.get(t, 1e30):
                        g[t] = ng
                        parent[t] = s
                        arrive[t] = 8
                        heapq.heappush(heap, (ng + h(ix, iy), ng, t))
        self.exhausted = True
        return None
