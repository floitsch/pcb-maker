# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.

"""The census of pocketed pads (docs/router.md, "Corridors of pocketed
pads"): pads without a legal lattice node of their own (escape pads) whose
escape nodes lie in a pocket the lattice closes, and whether a corridor
stub finds the way out off the lattice.

    python3 census.py <census run dir> <night's sweep dir> [--json out.json]

The census run is run.py in route mode on the night's placements with
PCB_ROUTER_CORRIDOR_CENSUS=1 (one line per escape pad and lattice in each
board's route.log), PCB_ROUTER_CORRIDOR_ESCAPES=0 (main's routes) and
PCB_SKIP_NATIVE=1 at a small budget. Open pads are KiCad's unconnected
items of the night's result (<sweep>/<task>/layout/result/drc.json)."""
import collections, glob, json, os, re, sys

LINE = re.compile(r'corridor census: pitch (\S+) origin \[(\S+), (\S+)\] pad (.+?) net (.+?) layer (\d+) (open|closed (\d+) (stub (\d+) points width (\S+) margin (\S+)|none))$')


def footprints(board):
    """Reference -> footprint name, from the board file."""
    text = open(board).read()
    names = {}
    for match in re.finditer(r'\(footprint "([^"]+)"', text):
        reference = re.search(r'\(property "Reference" "([^"]+)"', text[match.end():match.end() + 3000])
        if reference:
            names[reference.group(1)] = match.group(1)
    return names


def open_pads(drc):
    pads = set()
    for item in json.load(open(drc)).get('unconnected_items', []):
        for entry in item['items']:
            match = re.match(r'Pad (\S+) \[.*\] of (\S+) on', entry['description'])
            if match:
                pads.add(match.group(2) + '.' + match.group(1))
    return pads


def main():
    census, sweep = sys.argv[1], sys.argv[2]
    rows = []
    for log in sorted(glob.glob(os.path.join(census, '*', 'route.log'))):
        task = os.path.basename(os.path.dirname(log))
        # (pad, net) -> lattice -> the outcome of the first router built
        # on it (a later one, a continuation or a repair, routes beside
        # kept copper, which may close what the placement leaves open).
        pads = collections.defaultdict(lambda: collections.defaultdict(set))
        for line in open(log, errors='replace'):
            match = LINE.search(line.strip())
            if not match:
                continue
            pitch, ox, oy, pad, net, layer, status = match.group(1, 2, 3, 4, 5, 6, 7)
            lattice = (pitch, ox, oy)
            kind = 'open' if status == 'open' else ('stub' if match.group(10) else 'none')
            if not pads[(pad.strip(), net)][lattice]:
                pads[(pad.strip(), net)][lattice].add((kind, match.group(12)))
        result = glob.glob(os.path.join(sweep, task, 'layout', 'result', '*.kicad_pcb'))
        drc = os.path.join(sweep, task, 'layout', 'result', 'drc.json')
        opened = open_pads(drc) if os.path.exists(drc) else set()
        names = footprints(result[0]) if result else {}
        for (pad, net), lattices in pads.items():
            kinds = {kind for outcomes in lattices.values() for kind, _ in outcomes}
            part = re.split(r'[. ]', pad)[0]
            rows.append(dict(board=task, pad=pad, net=net, footprint=names.get(part, '?'),
                             lattices=len(lattices), closed=bool(kinds - {'open'}),
                             stub='stub' in kinds, none='none' in kinds,
                             margins=sorted({m for outcomes in lattices.values() for _, m in outcomes if m}),
                             open=pad in opened, has_result=bool(result)))
    if '--json' in sys.argv:
        json.dump(rows, open(sys.argv[sys.argv.index('--json') + 1], 'w'), indent=1)

    def line(name, chosen):
        print('%-66s %6d pads %4d open (KiCad) %3d boards %3d footprints' % (
            name, len(chosen), sum(r['open'] for r in chosen), len({r['board'] for r in chosen}),
            len({r['footprint'] for r in chosen})))
    print('boards with a census:', len({r['board'] for r in rows}), 'of', len(glob.glob(os.path.join(census, '*', 'route.log'))))
    line('escape pads (no legal node of their own on a lattice built)', rows)
    line('  their pocket closed on some lattice', [r for r in rows if r['closed']])
    line('    ... a corridor stub leaves it (the lattice misses the way out)', [r for r in rows if r['closed'] and r['stub']])
    line('    ... no way out off the lattice either', [r for r in rows if r['closed'] and not r['stub']])
    print()
    print('pocketed pads with a way out off the lattice, by footprint:')
    by = collections.defaultdict(list)
    for r in rows:
        if r['closed'] and r['stub']:
            by[r['footprint']].append(r)
    for footprint, chosen in sorted(by.items(), key=lambda item: -len(item[1])):
        print('  %-50s %3d pads, %2d open, boards: %s' % (
            footprint, len(chosen), sum(r['open'] for r in chosen),
            ', '.join(sorted({r['board'].split('__')[-1] for r in chosen}))))
        for r in chosen:
            print('      %-40s %-12s %-20s margins %s%s' % (r['board'].split('__')[-1], r['pad'], r['net'], r['margins'], ' OPEN' if r['open'] else ''))
    print()
    print('pocketed pads with no way out, by footprint:')
    by = collections.defaultdict(list)
    for r in rows:
        if r['closed'] and not r['stub']:
            by[r['footprint']].append(r)
    for footprint, chosen in sorted(by.items(), key=lambda item: -len(item[1])):
        print('  %-50s %3d pads, %2d open, boards: %s' % (
            footprint, len(chosen), sum(r['open'] for r in chosen),
            ', '.join(sorted({r['board'].split('__')[-1] for r in chosen}))))


if __name__ == '__main__':
    main()
