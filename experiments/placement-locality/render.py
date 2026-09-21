#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Render a saved placement, including the LED circuit and its region constraint."""
import argparse
import json
import math
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'placement-exploration'))
import run as placement


def render(problem, result, output):
    poses = result['poses']
    by = {p['component']: p for p in poses}
    circuit = {'J_POWER', 'R_LED', 'D_LED'}
    distance = math.dist(placement.xy(by['R_LED']['position']), placement.xy(by['D_LED']['position']))
    drawing = placement.svg(problem, poses, f'{output.stem}: LED / resistor centers {distance:.3f} mm')
    overlay = []
    for component in problem['components']:
        if component['id'] not in circuit:
            continue
        x, y = placement.xy(by[component['id']]['position'])
        overlay.append(f'<circle cx="{x}" cy="{y}" r="2" fill="none" stroke="#ffb74d" stroke-width=".18"/>')
        overlay.append(f'<text x="{x}" y="{y-2.2}" fill="#ffb74d" font-size=".5" text-anchor="middle">{component["id"]}</text>')
        region = component.get('constraints', {}).get('region')
        if region:
            x0, y0 = placement.xy(region['min']); x1, y1 = placement.xy(region['max'])
            overlay.append(f'<rect x="{x0}" y="{y0}" width="{x1-x0}" height="{y1-y0}" fill="none" stroke="#ffb74d" stroke-width=".1" stroke-dasharray=".4 .25"/>')
    for net in problem['nets']:
        if net['id'] != 'LED_SERIES':
            continue
        points = []
        for terminal in (net['from'], net['to']):
            c = next(c for c in problem['components'] if c['id'] == terminal['component'])
            pin = next(p for p in c['pins'] if p['id'] == terminal['pin'])
            points.append(placement.transform(pin['offset'], by[c['id']]))
        a, b = points
        overlay.append(f'<path d="M {a[0]} {a[1]} L {b[0]} {b[1]}" stroke="#ffb74d" stroke-width=".2" fill="none"/>')
    drawing = drawing.replace('</svg>', ''.join(overlay) + '</svg>')
    output.with_suffix('.svg').write_text(drawing)
    subprocess.run(['rsvg-convert', '--background-color', 'white', '--width', '1100', '--output', str(output.with_suffix('.png')), str(output.with_suffix('.svg'))], check=True)
    audit = placement.independent_pose_check(problem, poses)
    audit.update(led_resistor_center_distance_mm=distance, led_circuit_poses={k: by[k] for k in sorted(circuit)})
    output.with_suffix('.audit.json').write_text(json.dumps(audit, indent=2)+'\n')
    print(json.dumps(audit))
    return audit


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('problem', type=Path)
    parser.add_argument('result', type=Path)
    args = parser.parse_args()
    render(json.loads(args.problem.read_text()), json.loads(args.result.read_text()), args.result)
