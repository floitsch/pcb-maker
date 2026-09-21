#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Rank fixed-center quarter-turn counterfactuals and check them with the placer.

Experiment scope: legacy two-terminal branches with one geometric pad center per
pin. Ranking uses width * tension_weight * squared pad-center distance. This is
an attraction diagnostic, not electrical quality or routed length. Every tested
counterfactual gets a rendering, including rejected rotations.
"""
import argparse
import copy
import hashlib
import html
import json
import math
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'placement-exploration'))
import run as geometry


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('problem', type=Path)
    parser.add_argument('placement', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--limit', type=int, default=12)
    parser.add_argument('--executable', type=Path, default=Path('target/release/pcb-maker'))
    args = parser.parse_args(); out = args.output.resolve(); out.mkdir(parents=True, exist_ok=False)
    assert args.limit > 0
    exe = out/'pcb-maker'; shutil.copy2(args.executable, exe)
    problem = json.loads(args.problem.read_text()); proposal = json.loads(args.placement.read_text())
    assert not problem.get('electrical_nets'), 'This bounded diagnostic supports legacy branches only'
    poses = {p['component']: p for p in proposal['poses']}
    components = {c['id']: c for c in problem['components']}
    offsets = {}
    for c in problem['components']:
        for pin in c['pins']:
            centers = {tuple(geometry.xy(p.get('local_center', pin['offset']))) for p in pin.get('pads', [])}
            assert len(centers) <= 1, 'Multiple pad centers need a layer-aware diagnostic'
            offsets[c['id'], pin['id']] = geometry.point(*next(iter(centers))) if centers else pin['offset']
    def costs(candidate):
        result = {}
        for net in problem['nets']:
            points = []
            for key in ['from', 'to']:
                t = net[key]; points.append(geometry.transform(offsets[t['component'], t['pin']], candidate[t['component']]))
            distance = math.dist(*points)
            result[net['id']] = {'distance_mm': distance, 'weighted_squared_distance': net['width']*net.get('tension_weight', 1)*distance**2}
        return result
    before = costs(poses); ranking = []
    for cid, c in components.items():
        if c.get('constraints', {}).get('rotation', 'free') == 'fixed':
            continue
        incident = [n['id'] for n in problem['nets'] if cid in [n['from']['component'], n['to']['component']]]
        for turn in [90, 180, 270]:
            candidate = copy.deepcopy(poses); candidate[cid]['rotation_degrees'] = (poses[cid]['rotation_degrees']+turn)%360
            after = costs(candidate)
            benefit = sum(before[n]['weighted_squared_distance']-after[n]['weighted_squared_distance'] for n in incident)
            if benefit <= 1e-8:
                continue
            ranking.append({'component': cid, 'rotation_degrees': candidate[cid]['rotation_degrees'],
                            'benefit': benefit, 'incident': {n: {'before': before[n], 'after': after[n]} for n in incident}})
    ranking.sort(key=lambda r: (-r['benefit'], r['component'], r['rotation_degrees']))
    rows = []; cards = []
    config = out/'config.json'; config.write_text(json.dumps({'policy': {'kind': 'declared'}, 'projection_sweeps': 1}))
    def draw(candidate, path, title):
        svg = geometry.svg(problem, list(candidate.values()), title)
        path.write_text(svg)
        subprocess.run(['rsvg-convert', '--width', '1000', '--output', str(path.with_suffix('.png')), str(path)], check=True)
    draw(poses, out/'input.svg', 'Input: final legalized placement')
    for ordinal, row in enumerate(ranking[:args.limit]):
        directory = out/f'{ordinal:02}-{row["component"]}-{row["rotation_degrees"]:g}'; directory.mkdir()
        candidate = copy.deepcopy(poses); candidate[row['component']]['rotation_degrees'] = row['rotation_degrees']
        check = copy.deepcopy(problem)
        for c in check['components']:
            p = candidate[c['id']]; c.update(position=p['position'], rotation_degrees=p['rotation_degrees'])
            c.setdefault('constraints', {}).update(movement='fixed', rotation='fixed')
        (directory/'problem.json').write_text(json.dumps(check, indent=2)+'\n')
        result = subprocess.run([str(exe), 'place', str(directory/'problem.json'), str(config), str(directory/'placement.json')], capture_output=True, text=True)
        (directory/'check.log').write_text(result.stdout+result.stderr)
        row.update(legal=result.returncode == 0, exit_code=result.returncode, directory=directory.name, error=result.stderr.strip() if result.returncode else None)
        if row['legal']:
            actual = json.loads((directory/'placement.json').read_text())
            assert {p['component']: p for p in actual['poses']} == candidate
        title = f'{row["component"]} to {row["rotation_degrees"]:g} degrees: '+('legal' if row['legal'] else 'REJECTED')
        draw(candidate, directory/'preview.svg', title)
        rows.append(row)
        cards.append(f'<article><h2>{html.escape(title)}</h2><p>Attraction benefit {row["benefit"]:.3f}; {html.escape(row["error"] or "fixed centers preserved")}</p><a href="{directory.name}/preview.svg"><img src="{directory.name}/preview.svg"></a></article>')
    report = {'scope': __doc__, 'executable_sha256': hashlib.sha256(exe.read_bytes()).hexdigest(),
              'problem_sha256': hashlib.sha256(args.problem.read_bytes()).hexdigest(),
              'placement_sha256': hashlib.sha256(args.placement.read_bytes()).hexdigest(),
              'positive_counterfactuals': len(ranking), 'tested': rows, 'untested': ranking[args.limit:]}
    (out/'report.json').write_text(json.dumps(report, indent=2)+'\n')
    (out/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Orientation counterfactuals</title><style>body{background:#101923;color:white;font:16px system-ui}main{display:grid;grid-template-columns:repeat(auto-fit,minmax(400px,1fr))}img{width:100%}h2{font-size:18px}</style><h1>Fixed-center orientation counterfactuals</h1><p>Each card is a separate one-part rotation from the same input. They are not cumulative. Production legality is checked; copper is not routed. Ranking uses declared attraction weights.</p><main>'+''.join(cards)+'</main>')
    from orientation_details import render
    render(args.problem, args.placement, out)
    print(json.dumps({'positive_counterfactuals': len(ranking), 'tested': [{k:v for k,v in r.items() if k != 'incident'} for r in rows]}), flush=True)


if __name__ == '__main__': main()
