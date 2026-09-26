#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Two checks for the GPU survey (docs/reviews/2026-09-26-gpu-survey.md).

1. Screened diffusion: the 2026-09-07 resistor field detoured (42 cells
   against 28 on the wall fixture) because a Laplace potential spreads flow.
   With a leak to ground, -eps*log(u) approaches the geodesic distance
   (Varadhan), so descending it should give shortest paths.
2. Min-plus sweeps: the shortest-path field computed only by directional
   prefix-min scans (the GPU-friendly GAMER-style primitive) must equal
   Dijkstra; the number of rounds is what a GPU pays.

Run: python3 experiments/field-routing/screened_and_sweeps.py
"""
import heapq
import math

import numpy as np
import scipy.sparse as sp
import scipy.sparse.linalg as spla

BIG = 1e9


def wall_fixture():
    w, h = 21, 15
    free = np.ones((h, w), bool)
    free[2:, 10] = False
    return free, (7, 2), (7, 18)


def pad_fixture(size, walls, seed):
    """PCB-like: rows of rectangular pads with channels, plus long walls
    (connector rows) with a few openings, so paths need several bends."""
    rng = np.random.default_rng(seed)
    free = np.ones((size, size), bool)
    for r in range(4, size - 4, 6):
        for c in range(4, size - 4, 5):
            if rng.random() < 0.7:
                free[r:r + 3, c:c + 3] = False
    for k in range(walls):
        if k % 2 == 0:
            row = rng.integers(8, size - 8)
            free[row:row + 2, 1:size - 1] = False
            for gap in rng.integers(2, size - 4, 2):
                free[row:row + 2, gap:gap + 2] = True
        else:
            col = rng.integers(8, size - 8)
            free[1:size - 1, col:col + 2] = False
            for gap in rng.integers(2, size - 4, 2):
                free[gap:gap + 2, col:col + 2] = True
    source, target = (1, 1), (size - 2, size - 2)
    free[source] = free[target] = True
    return free, source, target


def offsets(directions):
    base = [(0, 1), (0, -1), (1, 0), (-1, 0)]
    if directions == 8:
        base += [(1, 1), (1, -1), (-1, 1), (-1, -1)]
    return base


def step_ok(free, r, c, dr, dc):
    h, w = free.shape
    rr, cc = r + dr, c + dc
    if not (0 <= rr < h and 0 <= cc < w) or not free[rr, cc]:
        return False
    if dr and dc and not (free[r + dr, c] and free[r, c + dc]):
        return False
    return True


def dijkstra(free, source, directions):
    dist = np.full(free.shape, np.inf)
    dist[source] = 0
    queue = [(0.0, source)]
    while queue:
        d, (r, c) = heapq.heappop(queue)
        if d > dist[r, c]:
            continue
        for dr, dc in offsets(directions):
            if step_ok(free, r, c, dr, dc):
                nd = d + math.hypot(dr, dc)
                if nd < dist[r + dr, c + dc]:
                    dist[r + dr, c + dc] = nd
                    heapq.heappush(queue, (nd, (r + dr, c + dc)))
    return dist


def descend(free, field, target, source, directions, better):
    """Greedy descent on a field from target to source; returns path length."""
    r, c = target
    length, seen = 0.0, {target}
    while (r, c) != source:
        options = [(field[r + dr, c + dc], dr, dc) for dr, dc in offsets(directions)
                   if step_ok(free, r, c, dr, dc) and (r + dr, c + dc) not in seen]
        if not options:
            return None
        value, dr, dc = better(options)
        r, c = r + dr, c + dc
        seen.add((r, c))
        length += math.hypot(dr, dc)
    return length


def resistor(free, source, target, directions, leak):
    """Solves the (screened) resistor network exactly; source held at 1."""
    h, w = free.shape
    index = -np.ones(free.shape, int)
    cells = np.argwhere(free)
    index[free] = np.arange(len(cells))
    rows, cols, vals = [], [], []
    diag = np.full(len(cells), leak)
    for i, (r, c) in enumerate(cells):
        for dr, dc in offsets(directions):
            if step_ok(free, r, c, dr, dc):
                g = 1 / math.hypot(dr, dc)
                rows.append(i); cols.append(index[r + dr, c + dc]); vals.append(-g)
                diag[i] += g
    a = sp.csr_matrix((vals, (rows, cols)), shape=(len(cells),) * 2) + sp.diags(diag)
    a = a.tolil()
    b = np.zeros(len(cells))
    s = index[source]
    a[s, :] = 0; a[s, s] = 1; b[s] = 1
    if leak == 0:  # plain Laplace: sink held at 0
        t = index[target]
        a[t, :] = 0; a[t, t] = 1; b[t] = 0
    u = spla.spsolve(a.tocsr(), b)
    field = np.zeros(free.shape)
    field[free] = u
    return field


def sweep_field(free, source, directions, cost=None):
    """Shortest-path field from prefix-min scans only.

    Along a line with step costs w_i (from cell i-1 into i) and prefix sums S,
    one directional relaxation is d_new = S + cummin(d - S); that is exactly
    min over j<=i of d_j + (S_i - S_j). A GPU does each line as one scan.
    """
    h, w = free.shape
    cost = np.ones(free.shape) if cost is None else cost
    d = np.full(free.shape, BIG)
    d[source] = 0
    rounds = 0
    lines = []
    # Axis lines (both directions) and, for 8 neighbours, diagonal lines.
    for r in range(h):
        lines.append([(r, c) for c in range(w)])
    for c in range(w):
        lines.append([(r, c) for r in range(h)])
    if directions == 8:
        for k in range(-h + 1, w):
            lines.append([(r, r + k) for r in range(h) if 0 <= r + k < w])
            lines.append([(r, w - 1 - r - k) for r in range(h) if 0 <= w - 1 - r - k < w])
    prepared = []
    for line in lines:
        line = np.array(line)
        for seq in (line, line[::-1]):
            if len(seq) < 2:
                continue
            rr, cc = seq[:, 0], seq[:, 1]
            step = np.hypot(np.diff(rr), np.diff(cc))
            ok = free[rr[1:], cc[1:]] & free[rr[:-1], cc[:-1]]
            diag = (np.diff(rr) != 0) & (np.diff(cc) != 0)
            ok &= ~diag | (free[rr[1:], cc[:-1]] & free[rr[:-1], cc[1:]])
            wts = np.where(ok, step * cost[rr[1:], cc[1:]], BIG)
            prepared.append((rr, cc, np.concatenate([[0.0], np.cumsum(wts)])))
    while True:
        rounds += 1
        before = d.copy()
        for rr, cc, s in prepared:
            vals = d[rr, cc]
            d[rr, cc] = np.minimum(vals, s + np.minimum.accumulate(vals - s))
        if np.allclose(before, d):
            break
    d[d > BIG / 2] = np.inf
    return d, rounds - 1


def bends(free, dist, target, source, directions):
    """Direction changes of the Dijkstra path (for the rounds comparison)."""
    r, c = target
    last, count = None, 0
    while (r, c) != source:
        best = None
        for dr, dc in offsets(directions):
            if step_ok(free, r, c, dr, dc):
                nd = dist[r + dr, c + dc] + math.hypot(dr, dc)
                if abs(nd - dist[r, c]) < 1e-9:
                    best = (dr, dc)
                    break
        if best is None:
            return None
        if last is not None and best != last:
            count += 1
        last = best
        r, c = r + best[0], c + best[1]
    return count


def main():
    print('## 1. Screened diffusion on the wall fixture (shortest: 28 / 20.97)')
    free, source, target = wall_fixture()
    for directions in (4, 8):
        dist = dijkstra(free, source, directions)
        plain = resistor(free, source, target, directions, 0.0)
        # Current-following in the original experiment; here the plain
        # potential is descended (steepest drop), which is similar.
        plain_len = descend(free, -plain, target, source, directions, min)
        row = [f'{directions}-neighbour: shortest {dist[target]:.2f}, plain Laplace {plain_len:.2f}']
        for leak in (1e-1, 1e-2, 1e-3):
            field = resistor(free, source, target, directions, leak)
            length = descend(free, -np.log(np.maximum(field, 1e-300)), target, source,
                             directions, min)
            row.append(f'leak {leak:g}: {length:.2f}' if length else f'leak {leak:g}: stuck')
        print('- ' + '; '.join(row))
    print()
    print('## 2. Prefix-min sweeps against Dijkstra')
    cases = [('wall', *wall_fixture())] + [
        (f'pads {n}x{n}, {k} walls', *pad_fixture(n, k, seed))
        for n, k, seed in [(64, 2, 1), (128, 4, 2), (256, 6, 3), (256, 10, 4), (512, 12, 5)]]
    for name, free, source, target in cases:
        for directions in (4, 8):
            exact = dijkstra(free, source, directions)
            swept, rounds = sweep_field(free, source, directions)
            finite = np.isfinite(exact)
            same = np.array_equal(finite, np.isfinite(swept)) and np.allclose(
                exact[finite], swept[finite])
            b = bends(free, exact, target, source, directions) if np.isfinite(exact[target]) else None
            print(f'- {name}, {directions}-nbr: equal to Dijkstra: {same}; '
                  f'rounds {rounds}; target distance {exact[target]:.1f}; bends on path {b}')


if __name__ == '__main__':
    main()
