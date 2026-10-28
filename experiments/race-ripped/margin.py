# Copyright (C) 2026 Toit contributors.
#
# The ripped count's pick margin (docs/placer.md, "The race ranks its probes
# by the ripped count (2026-10-27)"), calibrated on the coarse study's data:
# the trouble study's placements (build/trouble1-study/rows.json: each seed's
# kept placement, its final KiCad unconnected and the run's 75 s race probe)
# with the coarse study's p1-i25 probe at 5 % (build/coarse1-study: the
# race's probe but for an iteration cap of 25, its ripped count and its
# standing). Prints the race of k = 3 with a margin over the first-ranked
# placement (every subset, every member as the first-ranked) and the seed
# pairs by the relative margin of the lower count, for the ripped count and
# for the recorded probe's unfinished nets.
#
#   python3 -I experiments/race-ripped/margin.py [BUILD_DIR]
import itertools, json, os, sys

root = sys.argv[1] if len(sys.argv) > 1 else 'build'
rows = json.load(open(root + '/trouble1-study/rows.json'))
data = {}
for key, row in rows.items():
    board, seed = key.split('|')
    seed = int(seed)
    path = '%s/coarse1-study/%s/%d/p1-i25-5.json' % (root, board, seed)
    if not os.path.exists(path) or row.get('probe') is None:
        continue
    probe = json.load(open(path))
    data.setdefault(board, {})[seed] = dict(
        final=row['unc'], ripped=probe['ripped_open_terminals'],
        standing=(probe['negotiated_open_terminals'], probe['unfinished'], probe['conflicted']),
        unfinished=row['probe'])


def count(seeds, count_name, seed):
    return seeds[seed]['ripped' if count_name == 'ripped' else 'unfinished']


def race(count_name, margin, k=3):
    total = 0
    for seeds in data.values():
        picks = []
        for subset in itertools.combinations(sorted(seeds), min(k, len(seeds))):
            for first in subset:
                order = [first] + [s for s in subset if s != first]
                if count_name == 'ripped':
                    key = lambda s: (seeds[s]['ripped'], seeds[s]['standing'], order.index(s))
                else:
                    key = lambda s: (seeds[s]['unfinished'], order.index(s))
                best = min(order, key=key)
                c_first, c_best = count(seeds, count_name, first), count(seeds, count_name, best)
                relative = (c_first - c_best) / c_first if c_first else 0
                pick = best if best == first or relative >= margin else first
                picks.append(seeds[pick]['final'])
        total += sum(picks) / len(picks)
    return total


print('placements', sum(len(s) for s in data.values()), 'on', len(data), 'boards')
for count_name in ('ripped', 'unfinished'):
    print(count_name, 'race of 3 by margin:',
          ' '.join('%g %%: %.0f' % (m * 100, race(count_name, m)) for m in (0, .01, .02, .03, .05, .075, .1, .15, .2)))
    pairs = []
    for seeds in data.values():
        for a, b in itertools.combinations(sorted(seeds), 2):
            ca, cb = count(seeds, count_name, a), count(seeds, count_name, b)
            if ca == cb:
                continue
            high, low = (a, b) if ca > cb else (b, a)
            pairs.append(((max(ca, cb) - min(ca, cb)) / max(ca, cb), seeds[high]['final'] - seeds[low]['final']))
    print(count_name, 'pairs', len(pairs))
    for low, high in ((0, .02), (.02, .05), (.05, .1), (.1, .2), (.2, .4), (.4, 9)):
        chosen = [p for p in pairs if low <= p[0] < high]
        if chosen:
            print('  margin %g to %g %%: %d pairs, the lower count routed better %d, worse %d, mean gain %+.1f' % (
                low * 100, high * 100, len(chosen), sum(1 for p in chosen if p[1] > 0),
                sum(1 for p in chosen if p[1] < 0), sum(p[1] for p in chosen) / len(chosen)))
