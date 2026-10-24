#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""The weight of ratsnest crossings against wirelength (docs/placer.md,
"Ratsnest crossings (2026-10-24)"), calibrated on the placement
ablation's data: for each weight w (mm of wirelength per crossing), how
well wirelength + w * crossings orders

- the ablation's variants by their routed open connections (Spearman over
  the variants, and how often a variant's change against its base has
  the sign of its open count's change beyond the noise),
- our six layouts and the designer's placement per board (the
  ablation's cross-placement table), and whether the designer's
  placement is the cheapest.

    experiments/crossings/calibrate.py [build/abl1]
"""

import sys
import json
import math
from pathlib import Path
A = Path(sys.argv[1] if len(sys.argv) > 1 else 'build/abl1')

def rank(xs):
    order = sorted(range(len(xs)), key=lambda i: xs[i])
    r = [0.0]*len(xs); i = 0
    while i < len(order):
        j = i
        while j+1 < len(order) and xs[order[j+1]] == xs[order[i]]: j += 1
        for k in range(i, j+1): r[order[k]] = (i+j)/2
        i = j+1
    return r

def spearman(a, b):
    ra, rb = rank(a), rank(b)
    ma, mb = sum(ra)/len(ra), sum(rb)/len(rb)
    num = sum((x-ma)*(y-mb) for x, y in zip(ra, rb))
    den = math.sqrt(sum((x-ma)**2 for x in ra)*sum((y-mb)**2 for y in rb))
    return num/den if den else float('nan')

def variants(board, sig):
    opens = {}
    for line in open(A/board/'routes.log'):
        if line.startswith('{'):
            r = json.loads(line)
            u = r.get('native', {}).get('unconnected')
            if u is not None:
                opens[r['name'].split('__')[-1]] = u
    out = []
    for v in json.load(open(A/sig/'ablation.json'))['variants']:
        if v['name'] in opens:
            s = v['signals']
            out.append((v['name'], s['wirelength_mm'], s['ratsnest_crossings'], opens[v['name']]))
    return out

ws = [0, 0.05, 0.1, 0.2, 0.3, 0.5, 0.75, 1, 1.5, 2, 3, 5, 10, 1e9]
for board, sig in [('megadrive', 'megadrive-sig'), ('glasgow', 'glasgow-sig')]:
    vs = variants(board, sig)
    print(board, len(vs), 'variants')
    # pairs: d-x against designer, o-x against ours (sign agreement over pairs where |dopen| > noise)
    base = {n: (wl, x, o) for n, wl, x, o in vs}
    noise = 15 if board == 'megadrive' else 5
    for w in ws:
        s = spearman([wl + w*x for _, wl, x, _ in vs], [o for *_, o in vs])
        agree = total = 0
        for n, wl, x, o in vs:
            if n.startswith('d-') and 'null' not in n: ref = base['designer']
            elif n.startswith('o-') and 'null' not in n: ref = base['ours']
            else: continue
            do = o - ref[2]
            if abs(do) <= noise: continue
            dc = (wl - ref[0]) + w*(x - ref[1])
            total += 1; agree += (dc > 0) == (do > 0)
        print(f'  w={w:<6} spearman {s:5.2f} sign {agree}/{total}')
# cross-table placements
rows = {}
for line in open(A/'cross-table.txt'):
    p = [c.strip() for c in line.strip().strip('|').split('|')]
    if len(p) < 7 or not p[2].isdigit(): continue
    rows.setdefault(p[0], []).append((p[1], float(p[3]), float(p[6]), int(p[2])))
for board, rs in rows.items():
    print(board)
    for w in ws:
        ours = [r for r in rs if not r[0].startswith('designer')]
        s_all = spearman([wl + w*x for _, wl, x, _ in rs], [o for *_, o in rs])
        s_ours = spearman([wl + w*x for _, wl, x, _ in ours], [o for *_, o in ours])
        d = [r for r in rs if r[0].startswith('designer')][0]
        best = all(d[1] + w*d[2] < r[1] + w*r[2] for r in ours)
        print(f'  w={w:<6} spearman all {s_all:5.2f} ours {s_ours:5.2f} designer best {best}')
