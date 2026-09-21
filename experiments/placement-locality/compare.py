#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Build the retained three-way locality comparison without rerunning placement."""
import argparse
import hashlib
import html
import json
from pathlib import Path
import xml.etree.ElementTree as ET


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    root = parser.parse_args().directory
    cases = [('Original', 'baseline', 'baseline-pcb-maker'),
             ('Bounds during relaxation', 'bounds', 'bounds-pcb-maker'),
             ('Bounds + declared attraction weights', 'weighted/placement', 'weighted/pcb-maker')]
    rows = []; cards = []
    for title, stem, exe in cases:
        result = json.loads((root/f'{stem}.audit.json').read_text())
        assert result['passed']
        result.update(label=title, executable_sha256=hashlib.sha256((root/exe).read_bytes()).hexdigest())
        rows.append(result)
        source = (root/f'{stem}.svg').read_text(); ET.fromstring(source)
        cards.append(f'<article><h2>{html.escape(title)}</h2><p>{result["led_resistor_center_distance_mm"]:.3f} mm between centers</p>{source}</article>')
    assert rows[-1]['led_resistor_center_distance_mm'] < 5
    assert rows[-1]['led_resistor_center_distance_mm'] < rows[0]['led_resistor_center_distance_mm']/3
    (root/'comparison.json').write_text(json.dumps(rows, indent=2)+'\n')
    doc = '''<!doctype html><meta charset="utf-8"><title>LED placement locality</title>
<style>body{background:#101923;color:#edf2f7;font:16px system-ui;margin:1.5rem}main{display:flex;gap:1rem}article{flex:1;min-width:0}h2{font-size:1rem}svg{width:100%;height:70vh}button{font:inherit;padding:.4rem}</style>
<h1>LED / resistor locality</h1><p>Same complete netlist, constraints and seed. Orange: LED circuit and permitted regions. Lines show connectivity, not routed copper.</p>
<button onclick="zoom(!local)">Local area / whole board</button><main>CARDS</main>
<p>The full placement passes the production constraint projector and an independent body/lock check. This comparison measures locality; it does not establish full-board routability.</p>
<script>const drawings=[...document.querySelectorAll('svg')];const boxes=drawings.map(s=>s.getAttribute('viewBox'));let local=false;function zoom(value){local=value;drawings.forEach((s,i)=>s.setAttribute('viewBox',local?'24 0 19 34':boxes[i]));}zoom(true);</script>'''.replace('CARDS',''.join(cards))
    (root/'index.html').write_text(doc)
    print(root/'index.html')


if __name__ == '__main__': main()
