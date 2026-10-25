# Copyright (C) 2026 Toit contributors.
#
# The race probe ceiling (docs/placer.md, "The race's route probe (2026-10-25)"):
# per dense board the KiCad unconnected over three layout seeds, and which run
# the kept placement's race probe, the first route and the crossings would
# have picked. Run from the repository root: python3 -I experiments/race-probe/ceiling.py
import json, os, statistics, sys
B = 'build/'
short = {'OpenDrone-hw__OpenESC-20x20__4in1-mini':'OpenESC 4in1-mini','OpenDrone-hw__OpenESC-30x30__4in1':'OpenESC 4in1',
 'ohdsp__DSP-ADAU1452__DSP-ADAU1452':'DSP-ADAU1452','thpoll83__PolyKybd__poly_kybd_split72_left':'split72_left',
 'Huaqiu-Electronics__ecad-viewer__video':'video','GlasgowEmbedded__glasgow__glasgow__revC3':'Glasgow',
 'doudar__SmartSpin2k__SmartSpin2k_Panelized':'SmartSpin2k','0xCB-dev__0xCB-1337__panel':'0xCB panel',
 'CRImier__MyKiCad__protoesp':'protoesp','stonedDiscord__MegaDrive__MegaDrive':'MegaDrive','stonedDiscord__nonSNES__SNSP-CPU-01':'SNSP-CPU-01'}
order = list(short)
def load(d):
    out = {}
    for x in json.load(open(B+d+'/results.json')):
        n = x['name']; bl = B+d+'/'+n+'/layout/board-layout.json'
        try: j = json.load(open(bl))
        except Exception: j = {}
        cr = None
        out[n] = dict(unc=x.get('native',{}).get('unconnected'), first=x.get('first_unconnected_terminals'),
                      race=j.get('placement_race'), probed=(j.get('congestion') or {}).get('probed'))
    return out
variants = {'main x1818': ['book1-dense-s%d-off'%s for s in (1,101,201)],
            'booking x14343': ['book1-dense-s%d-on'%s for s in (1,101,201)],
            'anneal x14312': ['cross2-dense-s%d-on'%s for s in (1,101,201)]}
data = {v:[load(d) for d in ds] for v,ds in variants.items()}
# crossings of the placed boards
cross = {}
for v, ds in variants.items():
    cross[v] = []
    for d in ds:
        p = B+d+'/cross2-crossings.json'
        if not os.path.exists(p): p = B+d.replace('book1','cross2').replace('-on','-off')+'/cross2-crossings.json' if 'off' in d else None
        cross[v].append(json.load(open(p)) if p and os.path.exists(p) else None)
for v in data:
    print('##', v, [c is not None for c in cross[v]])
    print('| Board | s1 | s101 | s201 | median | min | max | max-min | probe pick | first-route pick | crossings pick |')
    sums = dict(med=0, mn=0, mx=0, s=[0,0,0]); picks = dict(probe=0, first=0, cross=0); pickreg = dict(probe=0, first=0, cross=0); nd=0
    for n in order:
        u = [data[v][i][n]['unc'] for i in range(3)]
        med, mn, mx = statistics.median(u), min(u), max(u)
        for i in range(3): sums['s'][i] += u[i]
        sums['med'] += med; sums['mn'] += mn; sums['mx'] += mx
        pr = [min(data[v][i][n]['race']) if data[v][i][n]['race'] else None for i in range(3)]
        fr = [data[v][i][n]['first'] for i in range(3)]
        cr = [cross[v][i][n]['ours'] if cross[v][i] else None for i in range(3)]
        def pick(vals):
            if any(x is None for x in vals): return None
            k = min(range(3), key=lambda i: (vals[i], i))
            return k
        cells = []
        for key, vals in (('probe', pr), ('first', fr), ('cross', cr)):
            k = pick(vals)
            if k is None: cells.append('-'); continue
            hit = u[k] == mn
            picks[key] += hit; pickreg[key] += u[k] - mn
            cells.append('%s (%s, %+d)' % ('yes' if hit else 'no', ['s1','s101','s201'][k], u[k]-mn))
        print('| %s | %d | %d | %d | %g | %d | %d | %d | %s |' % (short[n], *u, med, mn, mx, mx-mn, ' | '.join(cells)))
        print('|   (probe %s, first %s, cross %s)' % (pr, fr, cr), file=sys.stderr)
    print('| sum | %d | %d | %d | %g | %d | %d | | probe %d/11 regret %d | first %d/11 regret %d | cross %d/11 regret %d |' % (*sums['s'], sums['med'], sums['mn'], sums['mx'], picks['probe'], pickreg['probe'], picks['first'], pickreg['first'], picks['cross'], pickreg['cross']))
# union oracle across all nine
print('## union of the nine runs per board')
tot = 0
for n in order:
    u = [data[v][i][n]['unc'] for v in data for i in range(3)]
    tot += min(u)
print('sum of minima over nine', tot)
