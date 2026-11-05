# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.

"""Via room census on pad boxes (docs/placer.md, "A far-side pad under fine
pitch, and what it was (2026-11-05)").

    python3 census.py <out.json> <run dir>...   (a run dir holds layout/result)
    python3 census.py --summary <out.json>

Per SMD pad whose net has copper on another layer, the free via-centre area within reach R = via_d + 2c
(and 2R) of the pad's box, all pads as obstacles (`all`) and the other
side's SMD pads left out (`own`); whether the pad's box meets a
fine-pitch (<= 0.5 mm, 8+ SMD pads) pin field on the other side; whether
KiCad lists the pad among the unconnected items."""
import pcbnew, sys, json, glob, os, re
import numpy as np

STEP = 0.04
def mm(v): return v / 1e6

def box_dist(pts, boxes):
    """(N,) x (M,4) -> (N,M) distance from points to boxes (0 inside)."""
    dx = np.maximum(np.maximum(boxes[None, :, 0] - pts[:, None, 0], 0), pts[:, None, 0] - boxes[None, :, 2])
    dy = np.maximum(np.maximum(boxes[None, :, 1] - pts[:, None, 1], 0), pts[:, None, 1] - boxes[None, :, 3])
    return np.sqrt(dx * dx + dy * dy)

def classes(pro):
    try: d = json.load(open(pro))
    except Exception: return {}
    return {c['name']: (c.get('via_diameter', 0.6), c.get('clearance', 0.2)) for c in d.get('net_settings', {}).get('classes', [])}

def board(path, drc):
    b = pcbnew.LoadBoard(path)
    cls = classes(path[:-len('.kicad_pcb')] + '.kicad_pro')
    default = cls.get('Default', (0.6, 0.2))
    edge_c = mm(b.GetDesignSettings().m_CopperEdgeClearance)
    bb = b.GetBoardEdgesBoundingBox()
    board_box = (mm(bb.GetLeft()), mm(bb.GetTop()), mm(bb.GetRight()), mm(bb.GetBottom()))
    open_pads = set()
    for u in json.load(open(drc)).get('unconnected_items', []):
        for it in u['items']:
            m = re.match(r'Pad (\S+) \[.*\] of (\S+) on', it['description'])
            if m: open_pads.add((m.group(2), m.group(1)))
    zl = {}
    for z in b.Zones():
        if z.GetNetname():
            zl.setdefault(z.GetNetname(), set()).update(b.GetLayerName(l) for l in z.GetLayerSet().Seq())
    P = []; fields = {'F': [], 'B': []}
    for f in b.GetFootprints():
        smd = []
        for p in f.Pads():
            fc, bc = p.IsOnLayer(pcbnew.F_Cu), p.IsOnLayer(pcbnew.B_Cu)
            th = p.GetAttribute() in (pcbnew.PAD_ATTRIB_PTH, pcbnew.PAD_ATTRIB_NPTH)
            if not (fc or bc or th): continue
            side = 'A' if th or (fc and bc) else ('F' if fc else 'B')
            q = p.GetBoundingBox()
            P.append((f.GetReference(), p.GetNumber(), p.GetNetname(), side, (mm(q.GetLeft()), mm(q.GetTop()), mm(q.GetRight()), mm(q.GetBottom())),
                      p.GetNetClassName(), mm(p.GetOwnClearance(pcbnew.F_Cu if fc else pcbnew.B_Cu))))
            if side in 'FB': smd.append((mm(p.GetPosition().x), mm(p.GetPosition().y), P[-1][4]))
        if len(smd) >= 8:
            cs = np.array([(x, y) for x, y, _ in smd])
            dd = np.sqrt(((cs[:, None] - cs[None]) ** 2).sum(-1)); np.fill_diagonal(dd, 1e9)
            if np.median(dd.min(1)) <= 0.5 + 1e-6:
                bx = np.array([q for _, _, q in smd])
                side = 'F' if f.GetLayerName() == 'F.Cu' else 'B'
                fields[side].append((f.GetReference(), (bx[:, 0].min(), bx[:, 1].min(), bx[:, 2].max(), bx[:, 3].max())))
    boxes = np.array([p[4] for p in P]); sides = np.array([p[3] for p in P]); nets = np.array([p[2] for p in P])
    clear = np.array([p[6] for p in P])
    netsides = {}
    for p in P:
        if p[2]: netsides.setdefault(p[2], set()).add(p[3])
    count = {}
    for p in P:
        if p[2]: count[p[2]] = count.get(p[2], 0) + 1
    rows = []
    for i, (ref, num, net, side, bx, cn, cl) in enumerate(P):
        if side == 'A' or not net or count[net] < 2: continue
        other = 'B' if side == 'F' else 'F'
        zone_other = any(l != side + '.Cu' for l in zl.get(net, ()))
        if not (other in netsides[net] or zone_other): continue
        via_d, c = cls.get(cn.split(',')[0] if cn else 'Default', default)
        c = max(c, cl); vr = via_d / 2; R1 = via_d + 2 * c; R2 = 2 * R1
        x0, y0, x1, y1 = bx
        xs = np.arange(x0 - R2, x1 + R2 + 1e-9, STEP); ys = np.arange(y0 - R2, y1 + R2 + 1e-9, STEP)
        gx, gy = np.meshgrid(xs, ys); pts = np.stack([gx.ravel(), gy.ravel()], 1)
        dpad = box_dist(pts, np.array([bx]))[:, 0]
        keep = (dpad >= vr) & (dpad <= R2)
        pts, dpad = pts[keep], dpad[keep]
        g = vr + max(c, 0) + 1e-6
        near = np.where((boxes[:, 0] < x1 + R2 + g) & (boxes[:, 2] > x0 - R2 - g) & (boxes[:, 1] < y1 + R2 + g) & (boxes[:, 3] > y0 - R2 - g))[0]
        near = near[near != i]
        same = nets[near] == net
        # same-net pads: no via inside an SMD pad; other nets: the clearance
        gap = np.where(same, np.where(sides[near] == 'A', -1.0, vr), vr + np.maximum(c, clear[near]))
        d = box_dist(pts, boxes[near])
        hit = d < gap[None]
        oth = sides[near] == other
        blk_other = hit[:, oth].any(1); blk_own = hit[:, ~oth].any(1)
        # the board edge (its box: a cheap stand-in for the outline)
        e = np.minimum.reduce([pts[:, 0] - board_box[0], board_box[2] - pts[:, 0], pts[:, 1] - board_box[1], board_box[3] - pts[:, 1]])
        blk_own |= e < vr + edge_c
        culprits = sorted(set(P[j][0] for j in near[oth][hit[:, oth].any(0)]))
        r1 = dpad <= R1
        a = STEP * STEP
        fl = [r for r, fb in fields[other] if fb[0] < x1 and fb[2] > x0 and fb[1] < y1 and fb[3] > y0]
        rows.append(dict(ref=ref, pad=num, net=net, side=side,
                         all1=round(((~(blk_own | blk_other)) & r1).sum() * a, 4), own1=round(((~blk_own) & r1).sum() * a, 4),
                         all2=round((~(blk_own | blk_other)).sum() * a, 4), own2=round((~blk_own).sum() * a, 4),
                         strict=not ((nets == net) & ((sides == side) | (sides == 'A'))).sum() > 1, zone_other=zone_other, field=fl, culprits=culprits,
                         open=(ref, num) in open_pads))
    return rows, len(open_pads)

def summary(path):
    res = json.load(open(path))
    rows = [dict(r, b=b) for b, v in res.items() for r in v['rows']]

    def line(name, chosen):
        opened = sum(r['open'] for r in chosen)
        print('%-62s %6d pads %5d open (%5.2f %%) %3d boards' % (
            name, len(chosen), opened, 100.0 * opened / max(len(chosen), 1), len({r['b'] for r in chosen})))
    line('pads that need a via (net copper on another layer)', rows)
    line('  the net has no other pad on the pad\'s side', [r for r in rows if r['strict']])
    line('  under an other-side fine-pitch pin field', [r for r in rows if r['field']])
    line('  a via site within R', [r for r in rows if r['all1'] > 0])
    line('  no via site within R', [r for r in rows if r['all1'] == 0])
    line('  no site within R, the own side alone leaves one', [r for r in rows if r['all1'] == 0 and r['own1'] > 0])
    line('    ... under a fine-pitch field', [r for r in rows if r['all1'] == 0 and r['own1'] > 0 and r['field']])
    line('  no via site within 2R', [r for r in rows if r['all2'] == 0])
    line('  no site within 2R, the own side alone leaves one', [r for r in rows if r['all2'] == 0 and r['own2'] > 0])
    line('    ... under a fine-pitch field', [r for r in rows if r['all2'] == 0 and r['own2'] > 0 and r['field']])
    print('KiCad unconnected pads (every kind):', sum(v['open_pads'] for v in res.values()))

if __name__ == '__main__':
    if sys.argv[1] == '--summary':
        summary(sys.argv[2]); sys.exit(0)
    out = sys.argv[1]; res = {}
    for d in sys.argv[2:]:
        pcb = glob.glob(d + '/layout/result/*.kicad_pcb') or glob.glob(d + '/*/layout/result/*.kicad_pcb')
        drc = os.path.join(os.path.dirname(pcb[0]), 'drc.json') if pcb else ''
        if not pcb or not os.path.exists(drc): continue
        try:
            rows, n = board(pcb[0], drc)
        except Exception as ex:
            print('error', d, ex, file=sys.stderr); continue
        res[os.path.basename(d)] = dict(rows=rows, open_pads=n)
    json.dump(res, open(out, 'w'))
