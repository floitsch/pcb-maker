# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.

"""What the routes look like next to a designer's (docs/router.md, "What
the routes look like (2026-11-05)"): measures a designer recognises,
computed from a routed .kicad_pcb, the same way on the designer's board
and on ours.

    python3 census.py board <board.kicad_pcb> [--json]
    python3 census.py pair <designer.kicad_pcb> <ours.kicad_pcb> [--json]
    python3 census.py tier <run.log> [<run dir>] [--json out.json] [--table]

`tier` takes a run.py log (route mode on the designer's placement: the
nightly's designer.log or expansion.log, or one of yours) and its run
directory (default: the log's name without .log beside it), keeps the rows
that route completely (nothing open by the router, no unconnected item by
KiCad) and pairs each routed board (<run dir>/<task>/route/*.kicad_pcb)
with the designer's (the task's directory/<board_id>.kicad_pcb, the tasks
file named in the log).

The measures (per connection: per net of two pads or more, its pads less
one, the same for any routing of one netlist):

- bends: score-kicad-board's rule (crates/pcb-kicad/src/aesthetics.rs):
  paths are a net's segments on one layer between terminals (a pad of the
  net, a via, a junction, a free end); a direction change of 5 degrees or
  more is a turn, turns of one sense less than 1 mm apart summing to at
  most 90 degrees are one bend.
- sharp bends: bends of more than 95 degrees (an acute corner).
- returns: score-kicad-board's stair-steps (two bends within 2 mm that
  return to the first direction), kept to check against it; split into
  jogs (a chain of exactly two such bends: a step sideways) and staircases
  (three or more alternating bends, each within 2 mm of the next: a
  diagonal drawn in steps).
- pin-row crossings: score-kicad-board's rule (another net's track across
  the line between two adjacent SMD pads of a part); split by the pads'
  pitch at 1.5 mm (the router's pin_row_cost charges the narrower).
- vias, and layer changes (a via or a through-hole pad where the net's
  tracks meet on two or more layers: stitching vias and vias into a pour
  are no layer change).
- detour: a net's track length over the Euclidean minimum spanning tree of
  its pads, over the nets without a zone that have tracks (the sums'
  ratio, and the median per net). Paired, only the nets eligible on both
  boards count.
- parallel runs: straight segments of two nets on one layer, parallel,
  overlapping by 0.5 mm or more with an edge gap of at most 0.6 mm; per
  net pair and layer the length-weighted spread (standard deviation) of
  the gap; the board's spread is the length-weighted mean over the pairs
  with 2 mm of parallel run or more. Bundling: the parallel run over the
  board's track length (a designer runs buses side by side; less is what
  counts as worse)."""
import argparse, collections, glob, json, math, os, statistics, sys

TURN_DEGREES = 5.0
MERGE_MM = 1.0
STAIR_MM = 2.0
ADJACENT = 1.25
SHARP_DEGREES = 90.0 + TURN_DEGREES
PIN_ROW_PITCH_MM = 1.5
PARALLEL_GAP_MM = 0.6
PARALLEL_OVERLAP_MM = 0.5
PARALLEL_PAIR_MM = 2.0
PARALLEL_SIN = math.sin(math.radians(1.0))

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))


# ---------------------------------------------------------------- reading

def read_board(path):
    """The board as plain data: tracks (start, end, mid, layer, net, width,
    length), vias (at, net, radius), pads (center, half, net, layer or None
    for a through-hole pad, part, smd) and the nets with a copper zone."""
    import pcbnew
    # wx's image handlers and font substitutions talk on stderr.
    saved = os.dup(2)
    null = os.open(os.devnull, os.O_WRONLY)
    os.dup2(null, 2)
    try:
        board = pcbnew.LoadBoard(path)
    finally:
        os.dup2(saved, 2)
        os.close(saved)
        os.close(null)
    mm = 1.0e-6

    def point(p):
        return (p.x * mm, p.y * mm)

    tracks, vias = [], []
    for item in board.GetTracks():
        kind = item.GetClass()
        net = item.GetNetname() or None
        if kind == 'PCB_VIA':
            vias.append({'at': point(item.GetPosition()), 'net': net,
                         'radius': item.GetWidth(pcbnew.F_Cu) * mm / 2})
            continue
        track = {'start': point(item.GetStart()), 'end': point(item.GetEnd()), 'mid': None,
                 'layer': item.GetLayerName(), 'net': net, 'width': item.GetWidth() * mm,
                 'length': item.GetLength() * mm}
        if kind == 'PCB_ARC':
            track['mid'] = point(item.GetMid())
        tracks.append(track)
    pads = []
    for index, footprint in enumerate(board.GetFootprints()):
        back = footprint.IsFlipped()
        for pad in footprint.Pads():
            if not pad.IsOnCopperLayer():
                continue
            box = pad.GetBoundingBox()
            smd = pad.GetAttribute() == pcbnew.PAD_ATTRIB_SMD
            layer = None
            if smd:
                # The copper layer it is on (a reversible footprint's
                # pads lie on the side the part is not), else the part's.
                front, rear = pad.IsOnLayer(pcbnew.F_Cu), pad.IsOnLayer(pcbnew.B_Cu)
                layer = 'B.Cu' if rear and not front else 'F.Cu' if front and not rear else \
                    ('B.Cu' if back else 'F.Cu')
            pads.append({'center': point(pad.GetPosition()),
                         'half': (box.GetWidth() * mm / 2, box.GetHeight() * mm / 2),
                         'net': pad.GetNetname() or None, 'layer': layer,
                         'smd': smd, 'part': index, 'name': footprint.GetReference() + '.' + pad.GetNumber()})
    zone_nets = {zone.GetNetname() for zone in board.Zones()
                 if zone.IsOnCopperLayer() and not zone.GetIsRuleArea() and zone.GetNetname()}
    return {'tracks': tracks, 'vias': vias, 'pads': pads, 'zone_nets': zone_nets}


# --------------------------------------------------------------- geometry

class Buckets:
    def __init__(self, cell):
        self.cell = cell
        self.map = collections.defaultdict(list)

    def keys(self, low, high):
        c = self.cell
        for x in range(math.floor(low[0] / c), math.floor(high[0] / c) + 1):
            for y in range(math.floor(low[1] / c), math.floor(high[1] / c) + 1):
                yield (x, y)

    def insert(self, index, low, high):
        for key in self.keys(low, high):
            self.map[key].append(index)

    def query(self, low, high):
        found = set()
        for key in self.keys(low, high):
            found.update(self.map.get(key, ()))
        return found


def key(p):
    return (round(p[0] * 1000), round(p[1] * 1000))


def dist(a, b):
    return math.hypot(a[0] - b[0], a[1] - b[1])


def turn(a, b):
    return math.degrees(math.atan2(a[0] * b[1] - a[1] * b[0], a[0] * b[0] + a[1] * b[1]))


def bends_of(points):
    """The bends along a polyline: [(angle, where it starts, where it
    ends)], score-kicad-board's rule."""
    clean = []
    for p in points:
        if not clean or dist(clean[-1], p) >= 1.0e-6:
            clean.append(p)
    bends, along = [], 0.0
    for i in range(1, len(clean) - 1):
        along += dist(clean[i - 1], clean[i])
        a = (clean[i][0] - clean[i - 1][0], clean[i][1] - clean[i - 1][1])
        b = (clean[i + 1][0] - clean[i][0], clean[i + 1][1] - clean[i][1])
        angle = turn(a, b)
        if abs(angle) < TURN_DEGREES:
            continue
        if bends and (bends[-1][0] > 0) == (angle > 0) and along - bends[-1][2] < MERGE_MM \
                and abs(bends[-1][0] + angle) <= 90.0 + TURN_DEGREES:
            last = bends[-1]
            bends[-1] = (last[0] + angle, last[1], along)
        else:
            bends.append((angle, along, along))
    return bends


def returns_of(bends):
    """score-kicad-board's stair-steps: pairs of bends within STAIR_MM that
    return to the first direction, without overlap."""
    count, i = 0, 1
    while i < len(bends):
        first, second = bends[i - 1], bends[i]
        if (first[0] > 0) != (second[0] > 0) and abs(first[0] + second[0]) < TURN_DEGREES \
                and second[1] - first[2] < STAIR_MM:
            count += 1
            i += 2
        else:
            i += 1
    return count


def chains_of(bends):
    """Maximal chains of alternating bends, each within STAIR_MM of the
    next and returning the direction the one before it left: (jogs,
    staircases, bends in staircases). A chain of two is a jog, of three or
    more a staircase."""
    jogs = stairs = stair_bends = 0
    run = 1
    for i in range(1, len(bends) + 1):
        linked = False
        if i < len(bends):
            first, second = bends[i - 1], bends[i]
            linked = (first[0] > 0) != (second[0] > 0) and abs(first[0] + second[0]) < TURN_DEGREES \
                and second[1] - first[2] < STAIR_MM
        if linked:
            run += 1
            continue
        if run == 2:
            jogs += 1
        elif run >= 3:
            stairs += 1
            stair_bends += run
        run = 1
    return jogs, stairs, stair_bends


def crosses(a, b, c, d):
    def side(p, q, r):
        return (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    d1, d2 = side(a, b, c), side(a, b, d)
    d3, d4 = side(c, d, a), side(c, d, b)
    return d1 * d2 <= 0 and d3 * d4 <= 0 and (d1 != 0 or d2 != 0)


def inside(pad, p, margin=1.0e-6):
    return abs(p[0] - pad['center'][0]) <= pad['half'][0] + margin \
        and abs(p[1] - pad['center'][1]) <= pad['half'][1] + margin


def on_layer(pad, layer):
    return pad['layer'] is None or pad['layer'] == layer


def mst_length(points):
    """The Euclidean minimum spanning tree's length (Prim)."""
    if len(points) < 2:
        return 0.0
    best = [math.inf] * len(points)
    done = [False] * len(points)
    best[0], total = 0.0, 0.0
    for _ in range(len(points)):
        i = min((j for j in range(len(points)) if not done[j]), key=lambda j: best[j])
        done[i] = True
        total += best[i]
        for j in range(len(points)):
            if not done[j]:
                d = dist(points[i], points[j])
                if d < best[j]:
                    best[j] = d
    return total


# --------------------------------------------------------------- measures

def paths_of(board):
    """The track paths: (net, layer, points), score-kicad-board's chains."""
    pads, tracks = board['pads'], board['tracks']
    pad_buckets = Buckets(2.0)
    for i, pad in enumerate(pads):
        c, h = pad['center'], pad['half']
        pad_buckets.insert(i, (c[0] - h[0], c[1] - h[1]), (c[0] + h[0], c[1] + h[1]))
    via_keys = {key(via['at']) for via in board['vias']}
    groups = collections.defaultdict(list)
    for track in tracks:
        if track['net'] is None:
            continue
        if track['mid'] is None and dist(track['start'], track['end']) < 1.0e-6:
            continue
        line = [track['start'], track['mid'], track['end']] if track['mid'] else [track['start'], track['end']]
        groups[(track['net'], track['layer'])].append(line)
    out = []
    for (net, layer), pieces in sorted(groups.items()):
        ends = collections.defaultdict(list)
        for i, piece in enumerate(pieces):
            ends[key(piece[0])].append((i, False))
            ends[key(piece[-1])].append((i, True))

        def terminal(p):
            at = key(p)
            if len(ends.get(at, ())) != 2 or at in via_keys:
                return True
            return any(pads[i]['net'] == net and on_layer(pads[i], layer) and inside(pads[i], p)
                       for i in pad_buckets.query(p, p))
        used = [False] * len(pieces)

        def walk(first, backwards):
            points, index = [], first
            while True:
                used[index] = True
                piece = pieces[index][::-1] if backwards else pieces[index]
                points.extend(piece if not points else piece[1:])
                end = points[-1]
                if terminal(end):
                    break
                nxt = next(((o, e) for o, e in ends[key(end)] if not used[o]), None)
                if nxt is None:
                    break
                index, backwards = nxt
            return points
        for i in range(len(pieces)):
            for at_end in (False, True):
                if used[i]:
                    continue
                p = pieces[i][-1] if at_end else pieces[i][0]
                if terminal(p):
                    out.append((net, layer, walk(i, at_end)))
        for i in range(len(pieces)):
            if not used[i]:
                out.append((net, layer, walk(i, False)))
    return out


def connections_of(board):
    count = collections.Counter(pad['net'] for pad in board['pads'] if pad['net'])
    return sum(n - 1 for n in count.values()), count


def pin_row_crossings(board):
    """(charged, wider): crossings of pad pairs at most PIN_ROW_PITCH_MM
    apart and of wider ones."""
    found = pin_row_crossing_list(board)
    charged = sum(1 for gap in found.values() if gap <= PIN_ROW_PITCH_MM + 1.0e-6)
    return charged, len(found) - charged


def pin_row_crossing_list(board):
    """{(pad index, pad index, net): the pads' pitch} of every crossing."""
    pads = board['pads']
    segments = []
    for track in board['tracks']:
        if track['net'] is None:
            continue
        if track['mid']:
            segments += [(track['start'], track['mid'], track['net'], track['layer']),
                         (track['mid'], track['end'], track['net'], track['layer'])]
        else:
            segments.append((track['start'], track['end'], track['net'], track['layer']))
    buckets = Buckets(2.0)
    for i, (a, b, _, _) in enumerate(segments):
        buckets.insert(i, (min(a[0], b[0]), min(a[1], b[1])), (max(a[0], b[0]), max(a[1], b[1])))
    by_part = collections.defaultdict(list)
    for i, pad in enumerate(pads):
        if pad['layer'] is not None:
            by_part[pad['part']].append(i)
    found = {}
    for members in by_part.values():
        if len(members) < 3:
            continue
        nearest = []
        for a in members:
            ds = [dist(pads[a]['center'], pads[b]['center']) for b in members
                  if b != a and pads[b]['layer'] == pads[a]['layer']]
            ds = [d for d in ds if d > 1.0e-6]
            nearest.append(min(ds) if ds else math.inf)
        for i, a in enumerate(members):
            for j in range(i + 1, len(members)):
                b = members[j]
                pa, pb = pads[a], pads[b]
                if pa['layer'] != pb['layer']:
                    continue
                gap = dist(pa['center'], pb['center'])
                if gap < 1.0e-6 or gap > ADJACENT * min(nearest[i], nearest[j]):
                    continue
                c, d = pa['center'], pb['center']
                for k in buckets.query((min(c[0], d[0]), min(c[1], d[1])), (max(c[0], d[0]), max(c[1], d[1]))):
                    s, e, net, layer = segments[k]
                    if layer != pa['layer'] or net in (pa['net'], pb['net']):
                        continue
                    if crosses(c, d, s, e):
                        found[(a, b, net)] = gap
    return found


def layer_changes(board):
    """Vias and through-hole pads where the net's tracks meet on two or
    more layers."""
    buckets = Buckets(1.0)
    ends = []
    for track in board['tracks']:
        if track['net'] is None:
            continue
        for p in (track['start'], track['end']):
            buckets.insert(len(ends), p, p)
            ends.append((p, track['net'], track['layer']))

    def layers_at(center, net, test):
        r = 3.0
        found = set()
        for i in buckets.query((center[0] - r, center[1] - r), (center[0] + r, center[1] + r)):
            p, n, layer = ends[i]
            if n == net and test(p):
                found.add(layer)
        return found
    changes = 0
    for via in board['vias']:
        at, radius = via['at'], max(via['radius'], 0.05)
        if len(layers_at(at, via['net'], lambda p: dist(p, at) <= radius + 1.0e-6)) >= 2:
            changes += 1
    for pad in board['pads']:
        if pad['layer'] is None and pad['net'] and \
                len(layers_at(pad['center'], pad['net'], lambda p: inside(pad, p, 1.0e-3))) >= 2:
            changes += 1
    return changes


def net_lengths(board):
    """Per net with tracks and no zone, of two pads or more: (track length,
    the pads' spanning tree)."""
    length = collections.defaultdict(float)
    for track in board['tracks']:
        if track['net']:
            length[track['net']] += track['length']
    centers = collections.defaultdict(list)
    for pad in board['pads']:
        if pad['net']:
            centers[pad['net']].append(pad['center'])
    out = {}
    for net, total in length.items():
        if net in board['zone_nets'] or len(centers[net]) < 2 or len(centers[net]) > 400:
            continue
        tree = mst_length(centers[net])
        if tree > 0.05:
            out[net] = (total, tree)
    return out


def parallel_runs(board):
    """(parallel mm, median gap, spread): see the module's doc."""
    segments = []
    for track in board['tracks']:
        if track['net'] is None or track['mid'] is not None:
            continue
        a, b = track['start'], track['end']
        n = dist(a, b)
        if n < PARALLEL_OVERLAP_MM:
            continue
        segments.append((a, b, ((b[0] - a[0]) / n, (b[1] - a[1]) / n), n, track['width'],
                         track['net'], track['layer']))
    buckets = Buckets(1.0)
    reach = PARALLEL_GAP_MM + 0.5
    for i, (a, b, *_rest) in enumerate(segments):
        buckets.insert(i, (min(a[0], b[0]) - reach, min(a[1], b[1]) - reach),
                       (max(a[0], b[0]) + reach, max(a[1], b[1]) + reach))
    groups = collections.defaultdict(list)
    seen = set()
    for i, (a, b, u, n, w, net, layer) in enumerate(segments):
        for j in buckets.query((min(a[0], b[0]), min(a[1], b[1])), (max(a[0], b[0]), max(a[1], b[1]))):
            if j <= i or (i, j) in seen:
                continue
            seen.add((i, j))
            c, d, v, m, w2, net2, layer2 = segments[j]
            if layer2 != layer or net2 == net:
                continue
            if abs(u[0] * v[1] - u[1] * v[0]) > PARALLEL_SIN:
                continue
            t0 = (c[0] - a[0]) * u[0] + (c[1] - a[1]) * u[1]
            t1 = (d[0] - a[0]) * u[0] + (d[1] - a[1]) * u[1]
            overlap = min(n, max(t0, t1)) - max(0.0, min(t0, t1))
            if overlap < PARALLEL_OVERLAP_MM:
                continue
            offset = abs((c[0] - a[0]) * u[1] - (c[1] - a[1]) * u[0])
            gap = offset - (w + w2) / 2
            if gap <= 1.0e-3 or gap > PARALLEL_GAP_MM:
                continue
            groups[(min(net, net2), max(net, net2), layer)].append((gap, overlap))
    total = sum(o for pieces in groups.values() for _, o in pieces)
    gaps = sorted((g, o) for pieces in groups.values() for g, o in pieces)
    median = None
    if gaps:
        half, run = total / 2, 0.0
        for g, o in gaps:
            run += o
            if run >= half:
                median = g
                break
    weighted, weight = 0.0, 0.0
    for pieces in groups.values():
        length = sum(o for _, o in pieces)
        if length < PARALLEL_PAIR_MM:
            continue
        mean = sum(g * o for g, o in pieces) / length
        spread = math.sqrt(sum(o * (g - mean) ** 2 for g, o in pieces) / length)
        weighted += spread * length
        weight += length
    return total, median, (weighted / weight if weight else None)


def measure(board, detour_nets=None):
    """The board's measures; detour_nets limits the detour to those nets."""
    connections, _ = connections_of(board)
    bends = sharp = returns = jogs = stairs = stair_bends = 0
    paths = paths_of(board)
    for _net, _layer, points in paths:
        found = bends_of(points)
        bends += len(found)
        sharp += sum(1 for b in found if abs(b[0]) > SHARP_DEGREES)
        returns += returns_of(found)
        j, s, sb = chains_of(found)
        jogs, stairs, stair_bends = jogs + j, stairs + s, stair_bends + sb
    charged, wider = pin_row_crossings(board)
    changes = layer_changes(board)
    lengths = net_lengths(board)
    if detour_nets is not None:
        lengths = {net: v for net, v in lengths.items() if net in detour_nets}
    parallel, gap, spread = parallel_runs(board)
    track_mm = sum(t['length'] for t in board['tracks'])
    per = (lambda x: round(x / connections, 4)) if connections else (lambda x: None)
    ratios = sorted(t / m for t, m in lengths.values())
    return {
        'connections': connections,
        'paths': len(paths),
        'bends': bends, 'bends_pc': per(bends),
        'sharp': sharp, 'sharp_pc': per(sharp),
        'returns': returns,
        'jogs': jogs, 'jogs_pc': per(jogs),
        'staircases': stairs, 'staircases_pc': per(stairs), 'staircase_bends': stair_bends,
        'crossings': charged + wider, 'crossings_charged': charged, 'crossings_wider': wider,
        'crossings_pc': per(charged + wider),
        'vias': len(board['vias']), 'vias_pc': per(len(board['vias'])),
        'layer_changes': changes, 'layer_changes_pc': per(changes),
        'detour_nets': len(lengths),
        'detour': round(sum(t for t, _ in lengths.values()) / sum(m for _, m in lengths.values()), 4)
        if lengths else None,
        'detour_median': round(statistics.median(ratios), 4) if ratios else None,
        'parallel_mm': round(parallel, 1), 'parallel_gap_mm': round(gap, 3) if gap is not None else None,
        'parallel_share': round(parallel / track_mm, 4) if track_mm else None,
        'parallel_spread_mm': round(spread, 4) if spread is not None else None,
        'track_mm': round(track_mm, 1),
    }


def measure_pair(designer, ours):
    """Both boards' measures, the detour over the nets eligible on both."""
    nets = set(net_lengths(designer)) & set(net_lengths(ours))
    return measure(designer, nets), measure(ours, nets)


# ------------------------------------------------------------------- tiers

# The measures compared: title, per-board key, the count summed for the
# pooled rate (None: no pooling), and whether less than the designer's is
# what counts as worse (bundling: a designer runs buses side by side).
COMPARED = [
    ('bends per connection', 'bends_pc', 'bends', False),
    ('sharp bends per connection', 'sharp_pc', 'sharp', False),
    ('jogs per connection', 'jogs_pc', 'jogs', False),
    ('staircases per connection', 'staircases_pc', 'staircases', False),
    ('pin-row crossings per connection', 'crossings_pc', 'crossings', False),
    ('vias per connection', 'vias_pc', 'vias', False),
    ('layer changes per connection', 'layer_changes_pc', 'layer_changes', False),
    ('detour (length over spanning tree)', 'detour', None, False),
    ('parallel-run spacing spread (mm)', 'parallel_spread_mm', None, False),
    ('bundling (parallel-run share of the track length)', 'parallel_share', None, True),
]


def tier_pairs(log, run_dir):
    rows = [json.loads(line) for line in open(log) if line.startswith('{')]
    pairs = []
    tasks_cache = {}
    for row in rows:
        if row.get('open') != 0 or (row.get('native') or {}).get('unconnected') != 0:
            continue
        tasks_file = row.get('tasks')
        if tasks_file not in tasks_cache:
            path = tasks_file if os.path.isabs(tasks_file) else os.path.join(ROOT, tasks_file)
            tasks_cache[tasks_file] = {t['name']: t for t in json.load(open(path))['tasks']}
        task = tasks_cache[tasks_file].get(row['name'])
        if not task:
            continue
        directory = task['directory']
        if not os.path.isabs(directory):
            directory = os.path.join(ROOT, directory)
        designer = os.path.join(directory, task['board_id'] + '.kicad_pcb')
        ours = glob.glob(os.path.join(run_dir, row['name'], 'route', '*.kicad_pcb'))
        if not os.path.exists(designer) or len(ours) != 1:
            print(f'skipped {row["name"]}: designer {os.path.exists(designer)}, ours {ours}', file=sys.stderr)
            continue
        pairs.append((row['name'], designer, ours[0]))
    return pairs


def ratio(ours, designer):
    if ours is None or designer is None:
        return None
    if designer == 0:
        return 1.0 if ours == 0 else None
    return ours / designer


def summarize(results):
    """Per measure: boards, the designer's median, ours, ours over the
    designer's (of the medians, of the pooled rates: the counts summed over
    the boards over their connections summed, and the median per board
    where the designer's is not 0), worse / same / better. Ranked by how
    far we are: the boards worse less the boards better, over the boards."""
    lines = []
    for title, field, count, less_is_worse in COMPARED:
        rows = [r for r in results if r['designer'].get(field) is not None and r['ours'].get(field) is not None]
        if not rows:
            continue
        values = [(r['designer'][field], r['ours'][field]) for r in rows]
        ratios = [o / d for d, o in values if d > 0]
        worse = sum(1 for d, o in values if o > d + 1.0e-9)
        better = sum(1 for d, o in values if o < d - 1.0e-9)
        if less_is_worse:
            worse, better = better, worse
        designer, ours = statistics.median(d for d, _ in values), statistics.median(o for _, o in values)
        pooled = None
        if count:
            sums = [sum(r[side][count] for r in rows) for side in ('designer', 'ours')]
            connections = sum(r['ours']['connections'] for r in rows)
            pooled = (sums[0] / connections, sums[1] / connections)
        lines.append({
            'measure': title, 'key': field, 'boards': len(values),
            'designer': designer, 'ours': ours,
            'of_medians': ours / designer if designer else None,
            'pooled': pooled, 'pooled_ratio': pooled[1] / pooled[0] if pooled and pooled[0] else None,
            'per_board': statistics.median(ratios) if ratios else None, 'per_board_boards': len(ratios),
            'worse': worse, 'same': len(values) - worse - better, 'better': better,
        })
    # How far: the share of boards where ours is worse less the share
    # where it is better, then the ratio of the medians.
    lines.sort(key=lambda line: (-(line['worse'] - line['better']) / line['boards'], -(line['of_medians'] or 0)))
    return lines


def print_tables(results, out=sys.stdout):
    summary = summarize(results)
    x = lambda v: '-' if v is None else f'x{v:.2f}'
    print('| measure | boards | designer (median) | ours (median) | pooled: designer / ours | ours over designer: pooled | of the medians | median per board (boards) | ours worse / same / better | (worse - better) / boards |', file=out)
    print('|---|---|---|---|---|---|---|---|---|---|', file=out)
    for line in summary:
        pooled = '-' if line['pooled'] is None else f'{line["pooled"][0]:.3f} / {line["pooled"][1]:.3f}'
        print(f'| {line["measure"]} | {line["boards"]} | {line["designer"]:.3f} | {line["ours"]:.3f} | {pooled} '
              f'| {x(line["pooled_ratio"])} | {x(line["of_medians"])} | {x(line["per_board"])} ({line["per_board_boards"]}) '
              f'| {line["worse"]} / {line["same"]} / {line["better"]} '
              f'| {(line["worse"] - line["better"]) / line["boards"]:+.2f} |', file=out)
    print(file=out)
    keys = [('bends_pc', 'bends'), ('sharp_pc', 'sharp'), ('jogs_pc', 'jogs'), ('staircases_pc', 'stairs'),
            ('crossings_pc', 'cross'), ('vias_pc', 'vias'), ('layer_changes_pc', 'changes'),
            ('detour', 'detour'), ('parallel_spread_mm', 'spread'), ('parallel_share', 'bundled')]
    print('| board | conn | ' + ' | '.join(k for _, k in keys) + ' |', file=out)
    print('|---|---|' + '---|' * len(keys), file=out)

    def cell(d, o):
        if d is None or o is None:
            return '-'
        return f'{o:.2f} ({d:.2f})'
    for r in sorted(results, key=lambda r: r['name'].lower()):
        name = '/'.join(r['name'].split('__')[-2:])
        print(f'| {name} | {r["ours"]["connections"]} | '
              + ' | '.join(cell(r['designer'][k], r['ours'][k]) for k, _ in keys) + ' |', file=out)


def tables_from_json(path):
    results = json.load(open(path))
    for r in results:
        for side in ('designer', 'ours'):
            m = r[side]
            if 'parallel_share' not in m:
                m['parallel_share'] = round(m['parallel_mm'] / m['track_mm'], 4) if m['track_mm'] else None
    print_tables(results)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    one = sub.add_parser('board')
    one.add_argument('board')
    one.add_argument('--json', action='store_true')
    two = sub.add_parser('pair')
    two.add_argument('designer')
    two.add_argument('ours')
    two.add_argument('--json', action='store_true')
    tier = sub.add_parser('tier')
    tier.add_argument('log')
    tier.add_argument('run_dir', nargs='?')
    tier.add_argument('--json', help='write the per-board measures here')
    tier.add_argument('--only', action='append', help='only these tasks (repeatable)')
    table = sub.add_parser('table', help="print a tier's tables again from its --json")
    table.add_argument('json')
    arguments = parser.parse_args()
    if arguments.command == 'table':
        tables_from_json(arguments.json)
        return
    if arguments.command == 'board':
        result = measure(read_board(arguments.board))
        print(json.dumps(result, indent=1 if arguments.json else None))
    elif arguments.command == 'pair':
        designer, ours = measure_pair(read_board(arguments.designer), read_board(arguments.ours))
        print(json.dumps({'designer': designer, 'ours': ours}, indent=1))
    else:
        run_dir = arguments.run_dir or arguments.log[:-len('.log')]
        results = []
        for name, designer, ours in tier_pairs(arguments.log, run_dir):
            if arguments.only and name not in arguments.only:
                continue
            d, o = measure_pair(read_board(designer), read_board(ours))
            results.append({'name': name, 'designer': d, 'ours': o})
            print(f'{name}: bends {o["bends_pc"]} ({d["bends_pc"]})', file=sys.stderr)
        if arguments.json:
            json.dump(results, open(arguments.json, 'w'), indent=1)
        print_tables(results)


if __name__ == '__main__':
    main()
