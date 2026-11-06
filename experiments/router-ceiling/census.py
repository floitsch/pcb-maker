# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.

"""The router's ceiling (docs/router-ceiling.md): what keeps each board of
the matched designer tier (route mode on the designer's own placement)
from the strict pass, per open connection and per finding, grouped into
classes.

    python3 census.py pads <run.log> [<run dir>] --output pads.json
    python3 census.py tier <run.log> [<run dir>] [--pads pads.json]
        [--inspect DIR] [--designer-drc DIR] [--why <run dir>]
        [--budget <scale>=<run.log>[:<run dir>] ...] [--json out.json] [--table]

<run.log> is a run.py log of the designer tier (the nightly's
designer.log), <run dir> its directory (default: the log's name without
.log beside it; the nightly's designer/). Only rows that fail the strict
rule are read.

- `pads` (needs KiCad's pcbnew module) writes, per board, the features of
  every pad KiCad names in an unconnected item of the result: its
  footprint, its pad count, its size and its pitch (the centre distance
  to the nearest pad of another net of the same part on a shared copper
  layer). `tier` reads them with --pads; without, no pad is fine-pitch.
- --inspect: `pcb-maker inspect-routing-run <run>/route --reference
  <designer's project> --output <DIR>/<task>.json` per board (whether the
  designer's route joins each open net on this placement).
- --designer-drc: `pcb-maker drc-kicad-board <designer's project>
  <board id> <DIR>/<task>.json` per board: KiCad's DRC of the designer's
  own routed board. An open pair the designer's board leaves too is the
  input's.
- --why: a route of the same boards at the tier's budget with
  PCB_ROUTER_TROUBLE_WHY=1 (and PCB_ROUTER_CORRIDOR_CENSUS=1): the
  crossing census of every open fine-pitch pad, read from each board's
  route.log.
- --budget: the same boards at another budget scale (the budget test).

Each open pair (KiCad's unconnected item on our result) gets one cause,
the first that holds:

1. input: the designer's own board has an unconnected item on the same
   net between the same pads (or, for a pour's pieces, within 1 mm);
2. pour island: an item is a zone (two pieces of a pour, or a pad and a
   pour that KiCad's fill does not join);
3. via in pad: a pad of the pair holds a via of its net on the
   designer's board (its way out, an enclosed pad's only one) and the
   router may put none there: the part is not admitted (route mode
   admits the parts whose small SMD pads hold a designer's via), or no
   via size the project's rules allow fits;
4. walled pad: a pad of the pair is among the router's dead pads (no
   legal lattice node of its own and no escape: board-router.json's
   diagnostics.dead_pads), or the corridor census of the WHY route finds
   its escape pocket closed on the kept attempt's lattice with no
   corridor stub out, or its net is `unreachable`; the detail says
   what the router found near it (a footprint's copper graphic, a mask
   opening, the board's edge, its own part's pads);
5. fine-pitch row: a pad of the pair has a pad of another net of its part
   (of four pads or more) closer than 0.65 mm (router/fanout.rs
   FINE_PITCH and MIN_PART_PADS) and the crossing census of the WHY
   route's kept attempt says the row closes it; the detail is the
   crossing census of the WHY route (a crossing pair, every straight way
   closed, no straight way out);
6. congestion: the rest (the negotiation left it open between ordinary
   pads, or at a fine-pitch pad whose straight way out the census finds
   open); the detail adds the trouble repair's refusal for the net.

A board's class is the input's where any open pair is, else the cause
of most of its open pairs, else (nothing open) the finding type of most
of its strict findings; its second class is the next. With --budget, a
board of ours that passes the strict rule at a tested scale is the
budget's (the census's class becomes its second). --notes overrides a
class by hand (marked * in the table)."""
import argparse, collections, glob, json, math, os, re, sys

FINE_PITCH_MM = 0.65
# A part with fewer pads has no row (router/fanout.rs MIN_PART_PADS).
MIN_PART_PADS = 4
SAME_PLACE_MM = 1.0

INPUT = "input's"
POUR = 'pour island'
VIA_IN_PAD = 'via in pad'
WALLED = 'walled pad'
FINE = 'fine-pitch row'
CONGESTION = 'congestion'
BUDGET = 'the budget'
# The crossing census's verdicts that say the row closes the pad.
CLOSED = ('a crossing pair', 'every straight way closed', 'no straight way out')
PAIR_CAUSES = (INPUT, POUR, VIA_IN_PAD, WALLED, FINE, CONGESTION)

# Strict findings by type, the class they count for.
FINDING_CLASSES = {
    'starved_thermal': 'starved thermal',
    'connection_width': 'neck',
    'solder_mask_bridge': 'mask bridge',
    'clearance': 'clearance',
    'shorting_items': 'clearance',
    'items_not_allowed': 'keepout',
    'track_dangling': 'dangling track',
    'skew_out_of_range': 'length rule',
    'length_out_of_range': 'length rule',
    'diff_pair_uncoupled_length_too_long': 'length rule',
    'hole_clearance': 'clearance',
    'courtyards_overlap': 'courtyard',
    'annular_width': 'annular ring',
}

PAD = re.compile(r'^(?:PTH |NPTH |SMD )?[Pp]ad (\S*) \[(.*)\] of (\S+)')
NET = re.compile(r'\[([^\]]*)\]')
CROSSING = re.compile(r'crossing census [^:]*: (\S+) \((.*)\) layer (\d+): (.*)$')
REFUSAL = re.compile(r'^trouble exchange (.+?): (.*)$')
CORRIDOR = re.compile(r'corridor census: pitch (\S+) origin \[(\S+), (\S+)\] pad (.+?) net (.+?) layer (\d+) (open|closed .*)$')


def rows_of(log):
    rows = []
    for line in open(log, errors='replace'):
        line = line.strip()
        if line.startswith('{'):
            try:
                rows.append(json.loads(line))
            except ValueError:
                pass
    return rows


def failing(rows):
    return [row for row in rows if row.get('name') and not row.get('pass_strict')]


def run_dir_of(log, given=None):
    return given or (log[:-4] if log.endswith('.log') else log)


def plain_net(name):
    """KiCad's DRC writes hierarchical nets with a leading '/', the router's
    files without, and '{slash}' for a slash in a name."""
    return (name or '').replace('{slash}', '/').lstrip('/')


def item_of(item):
    """(kind, label, net, position) of an item of a KiCad DRC entry: kind
    'pad', 'zone', 'track', 'via' or 'other'; label REF.NUMBER for a pad."""
    description = item.get('description', '')
    position = item.get('pos') or {}
    at = (position.get('x', 0.0), position.get('y', 0.0))
    match = PAD.match(description)
    if match:
        number, net, ref = match.groups()
        return 'pad', f'{ref}.{number}', plain_net(net), at
    net = NET.search(description)
    net = plain_net(net.group(1)) if net else ''
    for kind in ('zone', 'track', 'via', 'arc'):
        if description.lower().startswith(kind):
            return ('track' if kind == 'arc' else kind), None, net, at
    return 'other', None, net, at


def pairs_of(drc):
    """KiCad's unconnected items: each a dict of its net and items."""
    pairs = []
    for entry in drc.get('unconnected_items', []):
        items = [item_of(item) for item in entry.get('items', [])]
        net = next((net for _, _, net, _ in items if net), '')
        pairs.append({'net': net, 'items': items})
    return pairs


def pads_of(pair):
    return sorted(label for kind, label, _, _ in pair['items'] if kind == 'pad')


def same_item(item, other):
    """Two items of KiCad's reports are one: the same pad, or within
    SAME_PLACE_MM (a track or zone KiCad names at a pad's place)."""
    if item[1] and other[1] and item[1] == other[1]:
        return True
    return math.dist(item[3], other[3]) <= SAME_PLACE_MM


def same_pair(pair, other):
    """The designer's unconnected item `other` is the same as `pair`: the
    same net, and its items the same as ours (in either order). Where both
    join only zones (a pour's pieces, which KiCad names at the zone's
    place, its partner of its own choosing) one zone at the same place is
    enough."""
    if pair['net'] != other['net']:
        return False
    ours, theirs = pair['items'], other['items']
    if all(kind == 'zone' for kind, _, _, _ in ours + theirs):
        return any(math.dist(a[3], b[3]) <= SAME_PLACE_MM for a in ours for b in theirs)
    if len(ours) != len(theirs):
        return False
    return any(all(same_item(a, b) for a, b in zip(ours, order)) for order in (theirs, theirs[::-1]))


def walled_detail(near):
    """What the router found near a dead pad, as a short reason."""
    if not near:
        return 'nothing near (no lattice node in the pad)'
    if any(label.startswith('copper') or ' copper ' in label for label in near):
        return 'a copper graphic (antenna, ring)'
    if any(label in ('board cutout', 'board edge') for label in near):
        return 'the board edge'
    parts = set()
    for label in near:
        match = re.match(r'solder mask opening \(pad (\S+)\.\)$', label)
        parts.add(match.group(1) if match else label.split('.')[0] if '.' in label else label)
    if len(parts) == 1:
        return "its own part's pads" + (' and mask openings' if any('mask opening' in label for label in near) else '')
    if any('mask opening' in label for label in near):
        return 'mask openings'
    if any('plated hole' in label for label in near):
        return 'a plated hole'
    return 'other pads: ' + ', '.join(near[:3])


def crossing_status(text):
    """The crossing census's verdict on an open fine-pitch pad."""
    if text.startswith('no straight way out'):
        return 'no straight way out'
    match = re.match(r'(\d+) of (\d+) ways closed', text)
    if not match:
        return 'unknown'
    if 'a crossing pair' in text:
        return 'a crossing pair'
    closed, ways = int(match.group(1)), int(match.group(2))
    return 'every straight way closed' if closed == ways else 'a straight way open'


def attempt_blocks(route_log):
    """The route.log's lines cut into the ladder's attempts: each block
    ends with the attempt's own line ("attempt <name>: N open, ..."), in
    the order of board-router.json's attempts (the probes' lines fall into
    the first block)."""
    blocks, block = [], []
    if not route_log or not os.path.exists(route_log):
        return blocks
    for line in open(route_log, errors='replace'):
        line = line.rstrip('\n')
        block.append(line)
        if line.startswith('attempt '):
            blocks.append(block)
            block = []
    if block:
        blocks.append(block)
    return blocks


def read_why(route_log, selected=None):
    """From a route.log of a run with PCB_ROUTER_TROUBLE_WHY, for the
    attempt `selected` (its index in the ladder; None: every attempt): per
    pad label the crossing census's verdicts (a Counter) and per net the
    trouble repair's refusals (a Counter of their reasons)."""
    verdicts = collections.defaultdict(collections.Counter)
    refusals = collections.defaultdict(collections.Counter)
    blocks = attempt_blocks(route_log)
    if selected is not None:
        blocks = blocks[selected:selected + 1]
    for block in blocks:
        for line in block:
            match = CROSSING.search(line.strip())
            if match:
                verdicts[match.group(1)][crossing_status(match.group(4))] += 1
                continue
            match = REFUSAL.match(line.strip())
            if match:
                reason = re.sub(r' \(.*$', '', match.group(2))
                reason = re.sub(r'^refused: .*', 'refused (the open terminals would not drop)', reason)
                refusals[plain_net(match.group(1))][reason] += 1
    return verdicts, refusals


def read_crossings(route_log):
    """Per pad label the crossing census's verdicts over every attempt."""
    return read_why(route_log)[0]


def read_corridors(route_log, pitch=None):
    """The pads whose escape nodes' pocket the corridor census found closed
    with no corridor stub out (PCB_ROUTER_CORRIDOR_CENSUS), on the lattice
    of `pitch` (mm; None: any): label -> the pocket's node count."""
    closed = {}
    if not route_log or not os.path.exists(route_log):
        return closed
    for line in open(route_log, errors='replace'):
        match = CORRIDOR.search(line.strip())
        if not match:
            continue
        status = match.group(7).split()
        if pitch is not None and abs(float(match.group(1)) - pitch) > 1e-6:
            continue
        if status[0] == 'closed' and status[-1] == 'none':
            closed[match.group(4).strip()] = int(status[1])
    return closed


def is_fine(features):
    """A pad of a fine-pitch row: a part of MIN_PART_PADS pads or more with
    a pad of another net closer than FINE_PITCH_MM."""
    return (features is not None and features.get('pitch_mm') is not None
            and features['pitch_mm'] < FINE_PITCH_MM and features.get('part_pads', 0) >= MIN_PART_PADS)


def classify_pair(pair, context):
    """(cause, detail) of one open pair; `context` holds the board's dead
    pads (label -> near), unreachable nets, pad features (label -> dict),
    crossing verdicts (label -> Counter) and the designer's pairs."""
    # Each of the designer's pairs answers for one of ours (a panel's copies
    # share their references).
    designer = context.get('designer_pairs', [])
    for index, other in enumerate(designer):
        if same_pair(pair, other):
            designer.pop(index)
            return INPUT, "the designer's board leaves it open too"
    kinds = [kind for kind, _, _, _ in pair['items']]
    if 'zone' in kinds:
        return POUR, 'KiCad joins no copper between ' + ' and '.join(kinds)
    pads = pads_of(pair)
    features = context.get('pads', {})
    admitted = context.get('via_in_pad') or []
    for label in pads:
        if (features.get(label) or {}).get('designer_via_in_pad') and not features[label].get('exposed'):
            part = label.split('.')[0]
            if part not in admitted:
                return VIA_IN_PAD, f"{label}: the designer's via in the pad; the part is not admitted"
            if label in context.get('dead_pads', {}):
                return VIA_IN_PAD, f"{label}: admitted, no via in it (a size the project's rules allow does not fit)"
    dead = context.get('dead_pads', {})
    for label in pads:
        if label in dead:
            return WALLED, f'{label}: ' + walled_detail(dead[label])
    pockets = context.get('pockets', {})
    for label in pads:
        if label in pockets:
            return WALLED, f'{label}: its escape pocket closed on the lattice, no way out off it (an exact fit)'
    if pair['net'] in context.get('unreachable', set()):
        return WALLED, 'the net is unreachable (a pad no search reaches)'
    fine = [label for label in pads if is_fine(features.get(label))]
    refusals = context.get('refusals', {}).get(pair['net'])
    refused = f"; the repair: {refusals.most_common(1)[0][0]}" if refusals else ''
    if fine:
        crossings = context.get('crossings')
        verdicts = collections.Counter()
        for label in fine:
            verdicts.update((crossings or {}).get(label, {}))
        footprint = (features.get(fine[0]) or {}).get('footprint', '?')
        detail = f'{fine[0]} ({footprint.split(":")[-1]}, {features[fine[0]]["pitch_mm"]:.2f} mm)'
        closed = [verdict for verdict in verdicts if verdict in CLOSED]
        if crossings is None:
            return FINE, detail + ' (no crossing census)'
        if closed:
            return FINE, detail + ': ' + max(closed, key=lambda verdict: verdicts[verdict]) + refused
        verdict = verdicts.most_common(1)[0][0] if verdicts else 'not in the census'
        return CONGESTION, detail + f': a fine-pitch pad, {verdict}' + refused
    return CONGESTION, (', '.join(pads) or ', '.join(kinds)) + refused


def ladder_of(report):
    """The ladder's ledger in a line: the selected attempt and its stop, and
    what the budget left out."""
    attempts = report.get('attempts') or []
    selected = next((attempt for attempt in attempts if attempt.get('selected')), None)
    text = ''
    if selected:
        name = selected['name'].replace('the continuation of the probed rung ', 'continued ')
        name = name.replace('the rung ', '').replace(' pitch=regular', '')
        text = f"{name}: {selected.get('negotiation_stop')}, {round(selected.get('work_seconds', 0))} s"
    short = []
    for line in report.get('skipped') or []:
        if 'finer lattice' in line:
            short.append('finer lattice')
        elif 'did not run: too little work' in line or 'budget of' in line or 'work bound' in line:
            short.append('steps' if 'step' in line else 'rungs')
    return text, sorted(set(short))


def budget_bound(report):
    """Whether the ledger says the budget stopped the board: the selected
    attempt stopped at a work limit, or rungs or steps did not run for
    want of work."""
    text, short = ladder_of(report)
    stops = ('negotiation_work_limit', 'work_deadline', 'probe_work_limit')
    return any(stop in text for stop in stops) or 'rungs' in short or 'steps' in short


def board_census(row, run_dir, pads=None, inspect=None, designer_drc=None, why=None):
    """One failing row: its open pairs with their causes, its strict
    findings, its classes."""
    name = row['name']
    route = os.path.join(run_dir, name, 'route')
    drc = load(os.path.join(route, 'drc.json')) or {}
    report = load(os.path.join(route, 'board-router.json')) or {}
    diagnostics = report.get('diagnostics') or {}
    dead = {pad['pad']: pad.get('near', []) for pad in diagnostics.get('dead_pads', [])}
    unreachable = {plain_net(net['connection']) for net in report.get('nets', []) if net.get('status') == 'unreachable'}
    designer = load(os.path.join(designer_drc, name + '.json')) if designer_drc else None
    crossings, refusals, pockets = None, {}, {}
    why_log = os.path.join(why, name, 'route.log') if why else None
    if why_log and os.path.exists(why_log):
        attempts = report.get('attempts') or []
        selected = next((index for index, attempt in enumerate(attempts) if attempt.get('selected')), None)
        crossings, refusals = read_why(why_log, selected)
        pitch = attempts[selected].get('grid_pitch_mm') if selected is not None else None
        pockets = read_corridors(why_log, pitch)
    context = {
        'dead_pads': dead,
        'unreachable': unreachable,
        'pads': (pads or {}).get(name, {}),
        'crossings': crossings,
        'refusals': refusals,
        'pockets': pockets,
        'designer_pairs': pairs_of(designer) if designer else [],
        'via_in_pad': row.get('via_in_pad') or [],
    }
    designer_open = len(context['designer_pairs']) if designer else None
    pairs = []
    for pair in pairs_of(drc):
        cause, detail = classify_pair(pair, context)
        pairs.append({'net': pair['net'], 'pads': pads_of(pair), 'cause': cause, 'detail': detail})
    reference = {}
    inspected = load(os.path.join(inspect, name + '.json')) if inspect else None
    for net in (inspected or {}).get('nets', []):
        reference[plain_net(net['net'])] = (net.get('reference') or {}).get('connected')
    for pair in pairs:
        pair['designer_connects'] = reference.get(pair['net'])
    findings = row.get('new_findings_strict') or {}
    classes = collections.Counter(pair['cause'] for pair in pairs)
    finding_classes = collections.Counter()
    for kind, count in findings.items():
        finding_classes[FINDING_CLASSES.get(kind, kind)] += count
    ranked = rank_classes(classes, finding_classes)
    text, short = ladder_of(report)
    return {
        'board': name,
        'open': len(pairs),
        'router_open': row.get('open'),
        'designer_open': designer_open,
        'pairs': pairs,
        'findings': findings,
        'classes': dict(classes),
        'finding_classes': dict(finding_classes),
        'class': ranked[0] if ranked else None,
        'second': ranked[1] if len(ranked) > 1 else None,
        'owner': "input's" if ranked and ranked[0] == INPUT else 'ours',
        'ladder': text,
        'ladder_short': short,
        'budget_bound': budget_bound(report),
    }


def rank_classes(classes, finding_classes):
    """The board's classes, deciding first: the input's where any open pair
    is the input's, then the open pairs' causes by count, then the
    findings' classes by count (ties in PAIR_CAUSES order, then by name)."""
    order = {cause: index for index, cause in enumerate(PAIR_CAUSES)}
    ranked = []
    if classes.get(INPUT):
        ranked.append(INPUT)
    ranked += [cause for cause, _ in sorted(classes.items(), key=lambda item: (-item[1], order[item[0]]))
               if cause != INPUT]
    ranked += [cause for cause, _ in sorted(finding_classes.items(), key=lambda item: (-item[1], item[0]))]
    return ranked


def load(path):
    try:
        return json.load(open(path))
    except (OSError, ValueError):
        return None


def budget_rows(specs):
    """--budget scale=log[:dir] -> {scale: {name: row}}."""
    tests = {}
    for spec in specs or []:
        scale, _, log = spec.partition('=')
        log = log.split(':')[0]
        # A scale given twice (one run split in two) joins its logs.
        rows = rows_of(log) if os.path.exists(log) else []
        tests.setdefault(scale, {}).update({row['name']: row for row in rows if row.get('name')})
    return tests


def apply_budget(board, rows):
    """A board of ours that passes the strict rule with more work is the
    budget's: its class becomes BUDGET, the census's class its second."""
    if board['owner'] == 'ours' and any(row and row.get('pass_strict') for row in rows):
        board['second'], board['class'] = board['class'], BUDGET
        board['owner'] = 'the budget'


def budget_cell(row):
    if row is None:
        return '-'
    unconnected = (row.get('native') or {}).get('unconnected')
    findings = sum((row.get('new_findings_strict') or {}).values())
    return ('**pass**' if row.get('pass_strict') else f'{unconnected} open' + (f', {findings} f.' if findings else ''))


def ranking(boards):
    """Per class: the boards it is the class of, those where it is the only
    class (its fix alone passes them), and those where it is the second."""
    table = collections.defaultdict(lambda: {'first': [], 'only': [], 'second': []})
    for board in boards:
        if board['class']:
            table[board['class']]['first'].append(board['board'])
            if board['second'] is None:
                table[board['class']]['only'].append(board['board'])
        if board['second']:
            table[board['second']]['second'].append(board['board'])
    return sorted(table.items(), key=lambda item: (-len(item[1]['first']), -len(item[1]['only']), item[0]))


def short_name(name):
    return name.split('__')[-1]


def print_table(boards, tests):
    scales = sorted(tests)
    head = ['Board', 'Open (ours / designer)', 'Open by cause', 'Strict findings', 'Class', 'Second',
            "Input's or ours", 'Ladder'] + [f'Budget scale {scale}' for scale in scales] + ['Note']
    print('| ' + ' | '.join(head) + ' |')
    print('|' + ' --- |' * len(head))
    for board in sorted(boards, key=lambda board: (board['open'], board['board'].lower())):
        causes = ', '.join(f'{cause} {count}' for cause, count in sorted(board['classes'].items(), key=lambda item: -item[1]))
        findings = ', '.join(f'{kind} {count}' for kind, count in sorted(board['findings'].items()))
        designer = '-' if board['designer_open'] is None else board['designer_open']
        ladder = board['ladder'] + (f"; short of work: {', '.join(board['ladder_short'])}" if board['ladder_short'] else '')
        cells = [short_name(board['board']), f"{board['open']} / {designer}", causes or '-', findings or '-',
                 (board['class'] or '-') + ('*' if board.get('class_overridden') else ''), board['second'] or '-',
                 board['owner'], ladder]
        cells += [budget_cell(tests[scale].get(board['board'])) for scale in scales]
        cells.append(board.get('note', ''))
        print('| ' + ' | '.join(str(cell) for cell in cells) + ' |')


def load_board(path):
    """A board read by pcbnew, its chatter on stderr silenced."""
    import pcbnew
    saved = os.dup(2)
    null = os.open(os.devnull, os.O_WRONLY)
    os.dup2(null, 2)
    try:
        return pcbnew.LoadBoard(path)
    finally:
        os.dup2(saved, 2)
        os.close(null)
        os.close(saved)


def pad_features(board_path, labels, designer_path=None):
    """Features of the pads `labels` (REF.NUMBER) on a board, by pcbnew;
    with the designer's board, whether the designer put a via of the pad's
    net inside the pad (its way out where the pad has no other)."""
    import pcbnew
    board = load_board(board_path)
    wanted = collections.defaultdict(set)
    for label in labels:
        ref, _, number = label.partition('.')
        wanted[ref].add(number)
    features = {}
    for footprint in board.GetFootprints():
        ref = footprint.GetReference()
        if ref not in wanted:
            continue
        # Copper pads only (KiCad's 0201 has two paste-only pads).
        pads = [pad for pad in footprint.Pads() if pad.GetLayerSet().CuStack()]
        for pad in pads:
            if pad.GetNumber() not in wanted[ref]:
                continue
            here = pad.GetPosition()
            layers = set(pad.GetLayerSet().CuStack())
            pitch = None
            for other in pads:
                if other is pad or other.GetNetname() == pad.GetNetname() or not other.GetNetname():
                    continue
                if not layers & set(other.GetLayerSet().CuStack()):
                    continue
                there = other.GetPosition()
                distance = math.hypot(here.x - there.x, here.y - there.y) / 1e6
                pitch = distance if pitch is None else min(pitch, distance)
            size = pad.GetSize()
            area = size.x * size.y
            others = [other.GetSize().x * other.GetSize().y for other in pads
                      if other is not pad and other.GetAttribute() == pcbnew.PAD_ATTRIB_SMD]
            features[f'{ref}.{pad.GetNumber()}'] = {
                # A thermal pad: the router lets vias into it (docs/router.md,
                # "Vias in pads").
                'exposed': bool(others) and all(area >= 2 * other for other in others) and len(others) >= 3,
                'footprint': footprint.GetFPIDAsString(),
                'part_pads': len(pads),
                'size_mm': [round(size.x / 1e6, 3), round(size.y / 1e6, 3)],
                'pitch_mm': None if pitch is None else round(pitch, 3),
                'smd': pad.GetAttribute() == pcbnew.PAD_ATTRIB_SMD,
                'at': [round(here.x / 1e6, 4), round(here.y / 1e6, 4)],
            }
    designer = load_board(designer_path) if designer_path and features else None
    if designer is not None:
        vias = [(track.GetPosition(), track.GetNetname()) for track in designer.GetTracks()
                if track.GetClass() == 'PCB_VIA']
        for footprint in designer.GetFootprints():
            ref = footprint.GetReference()
            for pad in footprint.Pads():
                label = f'{ref}.{pad.GetNumber()}'
                if label not in features or not features[label]['smd']:
                    continue
                inside = [at for at, net in vias if net == pad.GetNetname()
                          and pad.HitTest(at)]
                features[label]['designer_via_in_pad'] = bool(inside)
    return features


def designer_boards(tasks_file):
    """Task name -> the designer's board file, from a tasks file."""
    boards = {}
    tasks = load(tasks_file) or {}
    for task in tasks.get('tasks', []):
        boards[task['name']] = os.path.join(task['directory'], task['board_id'] + '.kicad_pcb')
    return boards


def command_pads(arguments):
    run_dir = run_dir_of(arguments.log, arguments.run_dir)
    rows = failing(rows_of(arguments.log))
    tasks = arguments.tasks or next((row.get('tasks') for row in rows if row.get('tasks')), None)
    designer = designer_boards(tasks) if tasks else {}
    result = {}
    for row in rows:
        route = os.path.join(run_dir, row['name'], 'route')
        drc = load(os.path.join(route, 'drc.json')) or {}
        labels = {label for pair in pairs_of(drc) for label in pads_of(pair)}
        boards = [path for path in glob.glob(os.path.join(route, '*.kicad_pcb'))]
        if labels and boards:
            reference = designer.get(row['name'])
            result[row['name']] = pad_features(boards[0], labels, reference if reference and os.path.exists(reference) else None)
    json.dump(result, open(arguments.output, 'w'), indent=1, sort_keys=True)
    print(f'{len(result)} boards, {sum(len(pads) for pads in result.values())} pads -> {arguments.output}')


def command_tier(arguments):
    run_dir = run_dir_of(arguments.log, arguments.run_dir)
    pads = load(arguments.pads) if arguments.pads else None
    boards = [board_census(row, run_dir, pads, arguments.inspect, arguments.designer_drc, arguments.why)
              for row in failing(rows_of(arguments.log))]
    notes = load(arguments.notes) if arguments.notes else {}
    for board in boards:
        note = (notes or {}).get(board['board']) or {}
        board['note'] = note.get('note', '')
        if note.get('class') and note['class'] != board['class']:
            board['census_class'], board['class'] = board['class'], note['class']
            board['class_overridden'] = True
        if 'second' in note:
            board['second'] = note['second']
    tests = budget_rows(arguments.budget)
    for board in boards:
        board['budget'] = {scale: budget_cell(rows.get(board['board'])) for scale, rows in tests.items()}
        apply_budget(board, [rows.get(board['board']) for rows in tests.values()])
    if arguments.json:
        json.dump(boards, open(arguments.json, 'w'), indent=1)
    if arguments.table:
        print_table(boards, tests)
        print()
    pairs = [pair for board in boards for pair in board['pairs']]
    print(f'{len(boards)} boards fail the strict rule; {len(pairs)} open pairs (KiCad), '
          f'{sum(not board["pairs"] for board in boards)} boards with nothing open')
    for cause, count in collections.Counter(pair['cause'] for pair in pairs).most_common():
        chosen = [pair for pair in pairs if pair['cause'] == cause]
        joined = sum(pair['designer_connects'] is True for pair in chosen)
        print(f'  {cause:<14} {count:4d} pairs on {len({b["board"] for b in boards for p in b["pairs"] if p["cause"] == cause}):2d} boards'
              f' (the designer joins the net on this placement: {joined})')
    print()
    print('classes by the boards they decide (first; only class; second):')
    for cause, entry in ranking(boards):
        print(f"  {cause:<16} {len(entry['first']):3d} {len(entry['only']):3d} {len(entry['second']):3d}  "
              + ', '.join(short_name(name) for name in entry['first']))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    commands = parser.add_subparsers(dest='command', required=True)
    pads = commands.add_parser('pads')
    pads.add_argument('log')
    pads.add_argument('run_dir', nargs='?')
    pads.add_argument('--output', required=True)
    pads.add_argument('--tasks', help="the tasks file (default: the rows' own)")
    tier = commands.add_parser('tier')
    tier.add_argument('log')
    tier.add_argument('run_dir', nargs='?')
    tier.add_argument('--pads')
    tier.add_argument('--inspect')
    tier.add_argument('--designer-drc')
    tier.add_argument('--why')
    tier.add_argument('--budget', action='append')
    tier.add_argument('--notes', help='notes.json: per board a note and a class that overrides the rule')
    tier.add_argument('--json')
    tier.add_argument('--table', action='store_true')
    arguments = parser.parse_args(argv)
    {'pads': command_pads, 'tier': command_tier}[arguments.command](arguments)


if __name__ == '__main__':
    main()
