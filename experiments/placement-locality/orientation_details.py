#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Render local before/after evidence for retained orientation counterfactuals."""
import copy
import hashlib
import html
import json
from pathlib import Path
import subprocess
import sys
import xml.etree.ElementTree as ET

sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'placement-exploration'))
import run as geometry


def render(problem_path, placement_path, root):
    problem_path, placement_path, root = map(Path, [problem_path, placement_path, root])
    report = json.loads((root/'report.json').read_text())
    assert hashlib.sha256(problem_path.read_bytes()).hexdigest() == report['problem_sha256']
    assert hashlib.sha256(placement_path.read_bytes()).hexdigest() == report['placement_sha256']
    problem = json.loads(problem_path.read_text()); poses = json.loads(placement_path.read_text())['poses']
    cards = []; ns = '{http://www.w3.org/2000/svg}'
    for row in report['tested']:
        directory = root/row['directory']
        for label in ['before', 'after']:
            view = copy.deepcopy(poses); own = next(p for p in view if p['component'] == row['component'])
            if label == 'after': own['rotation_degrees'] = row['rotation_degrees']
            by = {p['component']: p for p in view}
            svg = ET.fromstring(geometry.svg(problem, view, label))
            x, y = geometry.xy(own['position']); svg.set('viewBox', f'{x-7} {y-7} 14 14'); svg.set('width', '650'); svg.set('height', '650')
            for c in problem['components']:
                if c['id'] != row['component']: continue
                ET.SubElement(svg, ns+'circle', {'cx':str(x), 'cy':str(y), 'r':'2.3', 'fill':'none', 'stroke':'#ffb74d', 'stroke-width':'.10'})
                for pin in c['pins']:
                    center = pin.get('pads', [{}])[0].get('local_center', pin['offset']) if pin.get('pads') else pin['offset']
                    px, py = geometry.transform(center, own)
                    node = ET.SubElement(svg, ns+'text', {'x':str(px), 'y':str(py), 'fill':'black', 'font-size':'.45', 'text-anchor':'middle', 'dominant-baseline':'middle'})
                    node.text = pin['id']
            for c in problem['components']:
                pose = by[c['id']]; cx, cy = geometry.xy(pose['position'])
                if abs(cx-x) > 7 or abs(cy-y) > 7: continue
                top = min(py for _, py in geometry.polygon(c, pose))
                node = ET.SubElement(svg, ns+'text', {'x':str(cx), 'y':str(top-.25), 'fill':'white', 'font-size':'.36', 'text-anchor':'middle', 'stroke':'#18352d', 'stroke-width':'.10', 'paint-order':'stroke'})
                node.text = c['id']
            target = directory/f'{label}-detail.svg'
            ET.ElementTree(svg).write(target, encoding='unicode')
            subprocess.run(['rsvg-convert', '--width', '650', '--height', '650', '--output', str(target.with_suffix('.png')), str(target)], check=True)
        table = '<table><tr><th>Branch</th><th>Before mm</th><th>After mm</th></tr>'+''.join(f'<tr><td>{html.escape(net)}</td><td>{cost["before"]["distance_mm"]:.3f}</td><td>{cost["after"]["distance_mm"]:.3f}</td></tr>' for net,cost in row['incident'].items())+'</table>'
        status = 'legal' if row['legal'] else 'REJECTED'
        pictures = ''.join(f'<figure><figcaption>{label}</figcaption><img src="{row["directory"]}/{label}-detail.svg"></figure>' for label in ['before','after'])
        cards.append(f'<article><h2>{html.escape(row["component"])} → {row["rotation_degrees"]:g}° ({status})</h2><p>{html.escape(row["error"] or "All centers fixed; production placement validation passed.")}</p><div class="pair">{pictures}</div>{table}<p><a href="{row["directory"]}/preview.svg">Whole board</a></p></article>')
    (root/'details.html').write_text('<!doctype html><meta charset="utf-8"><title>Local orientation evidence</title><style>body{font:16px system-ui;background:#101923;color:white;margin:24px}a{color:#95dafa}main{display:grid;grid-template-columns:repeat(auto-fit,minmax(550px,1fr));gap:24px}.pair{display:flex}figure{margin:0;flex:1;min-width:0}img{width:100%}table{border-collapse:collapse}td,th{padding:4px 12px;text-align:left}h2{font-size:18px}</style><h1>Local orientation evidence</h1><p>Each card is one independent rotation. Orange circle marks the part; its pad labels expose pin direction. Views span 14 mm. Branch distances are straight pad separations, not routed copper. Rejected alternatives remain visible.</p><main>'+''.join(cards)+'</main>')
    print(root/'details.html')


if __name__ == '__main__': render(*sys.argv[1:])
