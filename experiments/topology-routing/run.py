#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Synthetic cut-capacity/topology exploration; no KiCad or production claims."""
import argparse
import collections
import hashlib
import heapq
import itertools
import json
import math
import random
import statistics
from pathlib import Path

EPS = 1e-9
COLORS = ['#e66a35', '#00998f', '#7854b5', '#cc4181', '#3683c5', '#7b8e23']


def point_segment(p, a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    t = max(0, min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) /
                   (dx * dx + dy * dy))) if dx or dy else 0
    return math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy)


def segment_distance(a, b, c, d):
    def cross(p, q, r):
        return (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    if (max(min(a[0], b[0]), min(c[0], d[0])) <= min(max(a[0], b[0]), max(c[0], d[0])) + EPS
            and max(min(a[1], b[1]), min(c[1], d[1])) <= min(max(a[1], b[1]), max(c[1], d[1])) + EPS
            and cross(a, b, c) * cross(a, b, d) <= 0
            and cross(c, d, a) * cross(c, d, b) <= 0):
        return 0.0
    return min(point_segment(a, c, d), point_segment(b, c, d),
               point_segment(c, a, b), point_segment(d, a, b))


def obstacles(case):
    cursor = 0
    out = []
    for center, width in case['corridors']:
        out.append([50, cursor, 60, center - width / 2])
        cursor = center + width / 2
    out.append([50, cursor, 60, 20])
    return out


def topology_conflicts(case, assignment):
    """Cheap graph checks; demand includes both walls and pair spacing."""
    conflicts = []
    for corridor, (_, capacity) in enumerate(case['corridors']):
        nets = [i for i, c in enumerate(assignment) if c == corridor]
        demand = sum(case['widths'][i] for i in nets) + (len(nets) + 1) * case['clearance'] if nets else 0
        if demand > capacity + EPS:
            conflicts.append(dict(kind='capacity', corridor=corridor, nets=nets,
                                  demand_mm=demand, capacity_mm=capacity, deficit_mm=demand - capacity))
    for i in range(len(assignment) - 1):
        if assignment[i] > assignment[i + 1]:
            conflicts.append(dict(kind='order', nets=[i, i + 1]))
    return conflicts


def realize(case, assignment):
    """One constructive geometry per signature; failure is not infeasibility."""
    ys = {}
    for corridor, (center, gap) in enumerate(case['corridors']):
        nets = [i for i, c in enumerate(assignment) if c == corridor]
        if not nets:
            continue
        # Equal free space, including the walls. Negative slack is deliberately
        # retained: the independent validator must catch overpacked candidates.
        free = (gap - sum(case['widths'][i] for i in nets)) / (len(nets) + 1)
        y = center - gap / 2 + free
        for i in nets:
            y += case['widths'][i] / 2
            ys[i] = y
            y += case['widths'][i] / 2 + free
    terminals = case.get('terminal_ys', [2 + 16 * i / max(1, len(assignment) - 1) for i in range(len(assignment))])
    return [dict(net=i, width_mm=width, points=[[1, terminals[i]],
                [45, ys[i]], [65, ys[i]], [109, terminals[i]]])
            for i, width in enumerate(case['widths'])]


def validate(case, routes):
    """Independent continuous capsule distances; never consult graph capacities."""
    violations = []
    minimum = float('inf')
    for route in routes:
        i, radius = route['net'], route['width_mm'] / 2
        expected_y = case.get('terminal_ys', [2 + 16 * j / max(1, len(routes) - 1) for j in range(len(routes))])[i]
        assert route['points'][0] == [1, expected_y] and route['points'][-1] == [109, expected_y]
        for a, b in itertools.pairwise(route['points']):
            for k, (x0, y0, x1, y1) in enumerate(obstacles(case)):
                corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
                inside = any(x0 <= p[0] <= x1 and y0 <= p[1] <= y1 for p in [a, b])
                distance = 0 if inside else min(segment_distance(a, b, c, d) for c, d in itertools.pairwise(corners))
                actual = distance - radius
                minimum = min(minimum, actual)
                if actual + EPS < case['clearance']:
                    violations.append(dict(kind='obstacle_clearance', net=i, obstacle=k, clearance_mm=actual))
            actual = min(min(p[0], 110 - p[0], p[1], 20 - p[1]) for p in [a, b]) - radius
            minimum = min(minimum, actual)
            if actual + EPS < case['clearance']:
                violations.append(dict(kind='board_edge_clearance', net=i, clearance_mm=actual))
    for first, second in itertools.combinations(routes, 2):
        distance = min(segment_distance(a, b, c, d)
                       for a, b in itertools.pairwise(first['points'])
                       for c, d in itertools.pairwise(second['points']))
        actual = distance - (first['width_mm'] + second['width_mm']) / 2
        minimum = min(minimum, actual)
        if actual + EPS < case['clearance']:
            violations.append(dict(kind='trace_clearance', nets=[first['net'], second['net']], clearance_mm=actual))
    return dict(valid=not violations, minimum_clearance_mm=minimum, violations=violations)


def signature(assignment):
    # In this restricted single-barrier model, the corridor names encode which
    # obstacles each path passes, and each corridor's order is fixed by net ID.
    return {'corridor_per_net': list(assignment), 'lane_order': [
        [i for i, c in enumerate(assignment) if c == corridor] for corridor in range(max(assignment) + 1)]}


def svg(case, record):
    parts = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="-1 -3 112 27" width="1120" height="270">',
             '<rect x="0" y="0" width="110" height="20" fill="#faf8ee" stroke="#777" stroke-width=".1"/>']
    for x0, y0, x1, y1 in obstacles(case):
        parts.append(f'<rect x="{x0}" y="{y0}" width="{x1-x0}" height="{y1-y0}" fill="#424b59"/>')
    for corridor, (center, gap) in enumerate(case['corridors']):
        parts.append(f'<text x="61" y="{center-.8}" font-size=".8" fill="#111">C{corridor}: {gap:g} mm</text>')
    for route in record['routes']:
        color = COLORS[route['net'] % len(COLORS)]
        points = ' '.join(f'{x},{y}' for x, y in route['points'])
        parts.append(f'<polyline points="{points}" fill="none" stroke="{color}" stroke-width="{route["width_mm"]}" stroke-linecap="round" stroke-linejoin="round"/>')
        for x, y in [route['points'][0], route['points'][-1]]:
            parts.append(f'<circle cx="{x}" cy="{y}" r=".3" fill="{color}"/>')
    status = 'CLEARANCE VALID' if record['geometry']['valid'] else 'REJECTED'
    parts.append(f'<text x="1" y="-1" font-size="1.1" fill="#111">{case["id"]}: {record["assignment"]} — {status}</text></svg>')
    return ''.join(parts)


def evaluate(case, assignment, directory, index):
    routes = realize(case, assignment)
    record = dict(attempt=index, assignment=list(assignment), signature=signature(assignment),
                  graph_conflicts=topology_conflicts(case, assignment), routes=routes, geometry=validate(case, routes))
    (directory / f'attempt-{index:04}.svg').write_text(svg(case, record))
    return record


def run_search(case, policy, directory, ordering=None):
    directory.mkdir(parents=True, exist_ok=True)
    start = tuple(case['initial'])
    choices = list(itertools.product(range(len(case['corridors'])), repeat=len(start)))
    attempts = []
    seen = set()
    priority_evaluations = 0
    ordering = ordering or dict(net_priority=list(range(len(start))), corridor_priority=list(range(len(case['corridors']))))
    corridor_rank = {value: rank for rank, value in enumerate(ordering['corridor_priority'])}
    def order_key(a):
        return tuple(corridor_rank[a[i]] for i in ordering['net_priority'])
    def priority(a):
        nonlocal priority_evaluations
        priority_evaluations += 1
        conflicts = topology_conflicts(case, a)
        return (sum(c.get('deficit_mm', 1) for c in conflicts), sum(x != y for x, y in zip(a, start)), order_key(a), a)
    if policy == 'blind':
        choices.sort(key=lambda a: (sum(x != y for x, y in zip(a, start)), order_key(a)))
        queue = collections.deque(choices)
    else:
        queue = [priority(start)]
    while queue:
        assignment = queue.popleft() if policy == 'blind' else heapq.heappop(queue)[-1]
        if assignment in seen:
            continue
        seen.add(assignment)
        record = evaluate(case, assignment, directory, len(attempts))
        attempts.append(record)
        if record['geometry']['valid']:
            break
        if policy == 'blind':
            continue
        children = set()
        for conflict in record['graph_conflicts']:
            if conflict['kind'] == 'order':
                i, j = conflict['nets']
                child = list(assignment)
                child[i], child[j] = child[j], child[i]
                children.add(tuple(child))
            else:
                # Only traces consuming an overloaded cut are moved. Every
                # alternate corridor is allowed: do not trap the policy locally.
                for i in conflict['nets']:
                    for corridor in range(len(case['corridors'])):
                        child = list(assignment)
                        child[i] = corridor
                        children.add(tuple(child))
        # Capacity is only a necessary check. Geometric-only failures retain a
        # bounded complete fallback instead of declaring the topology impossible.
        if not record['graph_conflicts']:
            children.update(choices)
        for child in children - seen:
            heapq.heappush(queue, priority(child))
    result = dict(policy=policy, ordering=ordering, attempts=len(attempts), feasible=attempts[-1]['geometry']['valid'],
                  selected=attempts[-1]['assignment'] if attempts[-1]['geometry']['valid'] else None,
                  search_space=len(choices), graph_rejections=sum(bool(a['graph_conflicts']) for a in attempts),
                  realization_calls=len(attempts), priority_evaluations=priority_evaluations,
                  graph_check_calls=len(attempts) + priority_evaluations,
                  geometry_only_rejections=sum(not a['graph_conflicts'] and not a['geometry']['valid'] for a in attempts),
                  records=attempts)
    (directory / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


HTML = '''<!doctype html><meta charset="utf-8"><title>Topology routing experiment</title>
<style>body{font:16px system-ui;margin:24px;background:#e9edf2;color:#142237}button,select,input{font:inherit}svg{width:100%;height:auto;background:white}pre{white-space:pre-wrap} .cols{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:20px}</style>
<h1>Capacity-aware topology exploration</h1><p>Each frame is an actual retained candidate. Widths are physical, obstacles are solid. Synthetic single-layer corridor experiment; no KiCad verification.</p>
<select id="case"></select> <button id="play">Play</button> <input id="frame" type="range" min="0" value="0"> <span id="position"></span> <label><input type="checkbox" id="focus">Focus passages</label>
<div class="cols"><div><h2>Blind enumeration</h2><div id="blind"></div><pre id="blind-info"></pre></div><div><h2>Conflict-guided</h2><div id="guided"></div><pre id="guided-info"></pre></div></div>
<p>Playback advances one evaluated topology per frame, not equal CPU time. The guided priority computes all proposed children's cut deficits; that work is counted separately. A capacity-safe signature can still fail continuous clearance. This is an existence test within one realization family, not a production router.</p>
<script>const data=__DATA__;const s=document.getElementById('case'),f=document.getElementById('frame');let timer;
for(const name of Object.keys(data)){let o=document.createElement('option');o.textContent=name;s.append(o)}
function draw(){const d=data[s.value];f.max=Math.max(d.blind.records.length,d.guided.records.length)-1;document.getElementById('position').textContent='step '+f.value;
for(const p of ['blind','guided']){const r=d[p].records[Math.min(+f.value,d[p].records.length-1)];document.getElementById(p).innerHTML=r.svg;if(document.getElementById('focus').checked)document.querySelector('#'+p+' svg').setAttribute('viewBox','43 0 28 20');document.getElementById(p+'-info').textContent=JSON.stringify({attempt:r.attempt+1,total_attempts:d[p].attempts,assignment:r.assignment,graph_conflicts:r.graph_conflicts,geometry_valid:r.geometry.valid,minimum_clearance_mm:r.geometry.minimum_clearance_mm,first_violations:r.geometry.violations.slice(0,3)},null,2)}}
s.onchange=()=>{f.value=0;draw()};f.oninput=draw;document.getElementById('focus').onchange=draw;document.getElementById('play').onclick=()=>{if(timer){clearInterval(timer);timer=null;return}timer=setInterval(()=>{if(+f.value>=+f.max){clearInterval(timer);timer=null}else{f.value=+f.value+1;draw()}},250)};draw();</script>'''


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, default=Path('build/algorithm-exploration-2026-09-07/topology'))
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    cases = [dict(id='two_traces_037_gap', widths=[.15, .15], corridors=[[6, .37], [14, .65]], initial=[0, 0]),
             dict(id='six_net_bottleneck', widths=[.15] * 6, corridors=[[4, .37], [10, .65], [16, .94]], initial=[1] * 6),
             dict(id='capacity_safe_wrong_order', widths=[.15] * 4, corridors=[[6, .65], [14, .65]], initial=[0, 1, 0, 1]),
             dict(id='capacity_safe_realizer_shortfall', widths=[.15, .15], terminal_ys=[2, 4], corridors=[[6, .60], [14, .65]], initial=[0, 0]),
             dict(id='mixed_widths', widths=[.15, .45, .15, .15], corridors=[[6, .65], [14, 1.25]], initial=[0, 0, 0, 0]),
             dict(id='insufficient_capacity_in_family', widths=[.15] * 3, corridors=[[6, .37], [14, .37]], initial=[0, 0, 0])]
    payload = {}
    summary = []
    for case in cases:
        case['clearance'] = .1
        payload[case['id']] = {}
        for policy in ['blind', 'guided']:
            result = run_search(case, policy, args.output / case['id'] / policy)
            payload[case['id']][policy] = result
            for record in result['records']:
                record['svg'] = svg(case, record)
            summary.append(dict(case=case['id'], **{k: v for k, v in result.items() if k != 'records'}))
        blind, guided = payload[case['id']]['blind'], payload[case['id']]['guided']
        assert blind['feasible'] == guided['feasible']
        assert blind['feasible'] == (case['id'] != 'insufficient_capacity_in_family')
        assert all(len({json.dumps(r['signature'], sort_keys=True) for r in p['records']}) == p['attempts'] for p in [blind, guided])
    # Geometry checker sanity controls are independent of graph/realizer logic.
    assert segment_distance((0, 0), (1, 1), (0, 1), (1, 0)) == 0
    assert abs(segment_distance((0, 0), (1, 0), (0, .25), (1, .25)) - .25) < EPS
    assert abs(segment_distance((0, 0), (1, 0), (2, 0), (3, 0)) - 1) < EPS
    zero_case = dict(cases[0], clearance=0)
    zero_routes = [dict(route, width_mm=0) for route in realize(cases[0], cases[0]['initial'])]
    zero_valid = validate(zero_case, zero_routes)['valid']
    assert zero_valid and not validate(cases[0], realize(cases[0], cases[0]['initial']))['valid']
    shortfall = payload['capacity_safe_realizer_shortfall']['guided']['records'][0]
    assert not shortfall['graph_conflicts'] and not shortfall['geometry']['valid']
    (args.output / 'cases.json').write_text(json.dumps(cases, indent=2) + '\n')
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    (args.output / 'viewer.html').write_text(HTML.replace('__DATA__', json.dumps(payload)))
    # Keep the board fixed while perturbing initial proposals and the ordering
    # induced by arbitrary net/corridor labels. Never tune permutations to wins.
    sensitivity = []
    sensitivity_viewer = {}
    for initial_id, initial in enumerate([[1] * 6, [0] * 6, [2] * 6, [0, 2, 0, 1, 2, 1]]):
        for seed in [11, 29, 73]:
            rng = random.Random(seed)
            ordering = dict(net_priority=rng.sample(range(6), 6), corridor_priority=rng.sample(range(3), 3))
            case = dict(cases[1], id=f'initial-{initial_id}-labels-{seed}', initial=initial)
            pair = {}
            for policy in ['blind', 'guided']:
                result = run_search(case, policy, args.output / 'sensitivity' / case['id'] / policy, ordering)
                assert result['feasible'] and result['selected'] == [0, 1, 1, 2, 2, 2]
                pair[policy] = result
                for record in result['records']:
                    record['svg'] = svg(case, record)
                sensitivity.append(dict(case=case['id'], initial=initial, **{k: v for k, v in result.items() if k != 'records'}))
            sensitivity_viewer[case['id']] = pair
    aggregate = {}
    for policy in ['blind', 'guided']:
        rows = [r for r in sensitivity if r['policy'] == policy]
        aggregate[policy] = dict(runs=len(rows), attempts_total=sum(r['attempts'] for r in rows),
                                attempts_min=min(r['attempts'] for r in rows), attempts_max=max(r['attempts'] for r in rows),
                                attempts_median=statistics.median(r['attempts'] for r in rows),
                                graph_checks_total=sum(r['graph_check_calls'] for r in rows))
    (args.output / 'sensitivity.json').write_text(json.dumps(dict(runs=sensitivity, aggregate=aggregate), indent=2) + '\n')
    (args.output / 'sensitivity.html').write_text(HTML.replace('__DATA__', json.dumps(sensitivity_viewer)))
    (args.output / 'provenance.json').write_text(json.dumps(dict(script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        validation='independent continuous segment/capsule clearance; not native KiCad', deterministic=True,
        zero_width_initial_geometry_valid=zero_valid, capacity_safe_geometry_rejection_checked=True,
        no_capacity_control=dict(single_trace_capacity_per_corridor=[1, 1], required_traces=3,
                                 proof_scope='Only these two corridor cuts, one layer, fixed obstacles.')), indent=2) + '\n')
    print(json.dumps(summary, indent=2))
    print(json.dumps(aggregate, indent=2))


if __name__ == '__main__':
    main()
