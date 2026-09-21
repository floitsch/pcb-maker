#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Collect the retained experiment reports into one browsable entry point."""
import argparse
import html
import json
import os
from pathlib import Path

parser=argparse.ArgumentParser()
parser.add_argument('directory',nargs='?',type=Path,default=Path('build/algorithm-exploration-2026-09-07'))
root=parser.parse_args().directory.resolve()
placement=json.loads((root/'placement/validation.json').read_text())
topology=json.loads((root/'topology/sensitivity.json').read_text())
field=json.loads((root/'field/final/validation.json').read_text())
engine=json.loads((root/'engine-validation.json').read_text())
summary={'field':field,'topology':topology['aggregate'],'engine':engine,'placement':placement,
         'scope':'Exploratory prototypes and actual engine/placer experiments. Five native routing probes cover only the first three connections, using placement seeds computed from the full netlist. No GPU timing or full-board completion claim.'}
dense_root=root.parent/'placement-dense-prefix19-2026-09-07'
dense_section=''
if (dense_root/'summary.json').exists():
    dense=json.loads((dense_root/'summary.json').read_text())
    if dense['terminal']:
        summary['dense_placement_followup']={k:v for k,v in dense.items() if k!='cases'}
        summary['dense_placement_followup']['cases']=[{k:v for k,v in c.items() if k not in ['frames','failures']} for c in dense['cases']]
        relative=html.escape(os.path.relpath(dense_root,root))
        dense_section=f'<h2>Follow-up: six dense placement probes</h2><p>The retained, harmonic and junction seeds now have paired 19-connection tests under both clearance profiles. All three complete with open-gap rules. Harmonic uses 322.270 mm versus retained\'s 477.701 mm, with 21 versus 18 vias. Blocked-rule failures distinguish exhausted search from obstructing retained copper; guided search and one-net rip-up provide separate successful recoveries.</p><p><a href="{relative}/index.html">Compare equal routing prefixes</a> · <a href="{relative}/diagnostics.html">See failure diagnostics and native recoveries</a></p>'
(root/'summary.json').write_text(json.dumps(summary,indent=2))
rows=[];gallery=[]
for case in placement['native_cases']:
    relative=Path(case['preview']).relative_to(root)
    image=relative.with_suffix('.png')
    assert (root/image).exists()
    label=html.escape(case['label'])
    rows.append(f'<tr><td>{label}</td><td>{case["trace_length_mm"]:.3f}</td><td>{case["vias"]}</td><td>{case["astar_expansions"]:,}</td><td>3 / 3</td></tr>')
    gallery.append(f'<figure><a href="{relative}"><img loading="lazy" src="{image}" alt="{label} native PCB"/></a><figcaption>{label} · {case["trace_length_mm"]:.3f} mm · {case["vias"]} vias</figcaption></figure>')
page='''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>PCB algorithm experiments</title>
<style>body{max-width:1240px;margin:35px auto;padding:0 22px;background:#111c2a;color:#dce9f5;font:17px/1.6 system-ui}h1{font-size:34px;line-height:1.2}h2{font-size:24px}a{color:#79d1ff}p{max-width:1050px}.cards{display:grid;grid-template-columns:1fr 1fr;gap:18px}.card{background:#203244;padding:22px;border:1px solid #36536a;border-radius:10px}.card h2{margin-top:0}table{border-collapse:collapse;width:100%;font-size:15px}th,td{text-align:left;border-bottom:1px solid #36536a;padding:10px}figure{margin:0}img{width:100%;height:auto}.gallery{display:grid;grid-template-columns:1fr 1fr;gap:25px}figcaption{padding:8px;color:#bacddd}.note{color:#b3c7d8}.hero{max-width:850px}.button{display:inline-block;background:#284f69;padding:7px 15px;border-radius:6px;margin:6px 0}@media(max-width:750px){.cards,.gallery{grid-template-columns:1fr}table{font-size:12px}}</style>
<h1>Placement, pressure and topology</h1><p>2026-09-07 · A first experimental comparison, with failures retained. The evidence favors coordinated routing, capacity-aware topology and more placement exploration. These mechanisms serve different purposes; there is no single winner across all tasks.</p>
<h2>Watch the actual engine</h2><p>This is a rigid component pulled by its attached traces, using the existing CPU engine. White endpoints stay fixed. It reaches the known 23 mm copper lower bound for this empty fixture. Gold arrows show tension proposals; red arrows show constraint corrections.</p>
<div class="hero"><a href="engine-convergence/coupled-settled.html"><img src="engine-convergence/coupled-settled.gif" alt="Recorded engine iterations pulling a rigid component between fixed terminals"/></a></div>
<a class="button" href="engine-convergence/coupled-settled.html">Open interactive playback</a>
<div class="cards">
<section class="card"><h2>1. Pressure-field routing</h2><p>150 trial rows across five small fixtures. Diffusion finds paths, but the wall detour is 42 cells versus 28 for shortest path. Present/history congestion prices resolve competing routes with either path finder.</p><a href="field/final/index.html">Explore fields and routing passes</a><p class="note">CPU resistor/diffusion analogy; no fluid inertia, physical copper widths or GPU measurements.</p></section>
<section class="card"><h2>2. Topology with capacity</h2><p>On twelve matched variations of one six-net board, guided search realizes 79 geometries versus 3,508 blind trials. Counting priority work gives 521 versus 3,508 graph checks.</p><a href="topology/viewer.html">Play six mechanisms</a> · <a href="topology/sensitivity.html">Inspect ordering sensitivity</a><p class="note">Named passages and route order carry width demand. Continuous geometry still decides acceptance.</p></section>
<section class="card"><h2>3. Continuous engine</h2><p>Ten trajectories, 5,530 independently checked frames. Tension can pull traces and bodies into a good geometry. Opposite obstacle-side seeds retain different outcomes.</p><a href="engine-checked/index.html">Compare engine controls</a> · <a href="engine-convergence/index.html">Known lower-bound controls</a><p class="note">Default tension is zero; copper-repair density weights are also zero. Nonzero field weights are an explicit experimental ablation.</p></section>
<section class="card"><h2>4. Placement exploration</h2><p>97 proposals, including failed reinsertion and random attempts. A pinless obstacle exposes a missing signal: connection springs give it no reason to open a blocked passage. Five selected real-board placements then pass the same native three-connection probe.</p><a href="placement/overview.html">Explore placement proposals</a><p class="note">Proposal playback is discrete. It does not interpolate a physically valid movement between placements.</p></section>
</div>
<h2>Native routing after different placements</h2><p>All five placements use the full 52-connection netlist when choosing positions. The unchanged router then attempts the same first three connections with the open-gap 0.15 mm trace / 0.10 mm clearance profile. Each reaches 3/3 with zero native design findings and the proposed placement preserved. These are partial routing probes, not completed full boards.</p>
<table><thead><tr><th>Placement</th><th>Copper mm</th><th>Vias</th><th>A* expansions</th><th>Completed</th></tr></thead><tbody>__ROWS__</tbody></table>
<p>The retained full-size harmonic policy wins this small probe. A* expansions exclude placement, verification and rendering work. Library metadata warning counts vary by placement and remain in the raw reports.</p>
<div class="gallery">__GALLERY__</div>
__DENSE__
<h2>What to pursue next</h2><p>Use failed-route evidence to identify scarce passages, decide which consuming route should change corridor or which movable body should make room, then relax and verify the resulting geometry. Preserve good alternatives and distinguish search-budget exhaustion from proven capacity failure. Test the strongest placement candidates at denser prefixes under both clearance profiles.</p>
<p><a href="../../docs/reviews/2026-09-07-algorithm-exploration.md">Read the assessment and research parallels</a> · <a href="summary.json">Machine-readable results</a></p></html>'''
(root/'index.html').write_text(page.replace('__ROWS__',''.join(rows)).replace('__GALLERY__',''.join(gallery)).replace('__DENSE__',dense_section))
print(root/'index.html')
