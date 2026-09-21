#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Inspect which retained copper conflicts with a saved isolated route.

This is an advisory straight-track/through-via analysis for the frozen fixture,
not native admission or a proof that these are the only possible blockers.
Foreign pads, filled zones and obstacle-free alternative paths are not analyzed.
"""
import argparse
import hashlib
import html
import json
import math
from pathlib import Path
import subprocess

import pcbnew
if not hasattr(pcbnew.SwigPyIterator, 'next'):
    pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__


def xy(point):
    return (pcbnew.ToMM(point.x), pcbnew.ToMM(point.y))


def point_distance(point, a, b):
    delta = [b[i] - a[i] for i in range(2)]
    length = sum(v*v for v in delta)
    t = max(0, min(1, sum((point[i]-a[i])*delta[i] for i in range(2))/length)) if length else 0
    return math.dist(point, [a[i]+t*delta[i] for i in range(2)])


def segment_distance(a, b, c, d):
    def cross(p, q, r):
        return (q[0]-p[0])*(r[1]-p[1])-(q[1]-p[1])*(r[0]-p[0])
    if cross(a,b,c)*cross(a,b,d) < 0 and cross(c,d,a)*cross(c,d,b) < 0:
        return 0
    return min(point_distance(a,c,d), point_distance(b,c,d), point_distance(c,a,b), point_distance(d,a,b))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--board', type=Path, help='Board containing retained foreign copper')
    parser.add_argument('--candidate', type=Path, help='Saved isolated-route candidate on the same placement')
    args = parser.parse_args()
    assert (args.board is None) == (args.candidate is None), 'Supply both board and candidate'
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    root = Path(__file__).resolve().parents[2] / 'build/placement-dense-prefix19-2026-09-07'
    board_path = root / 'junction-blocked/run/solve/step-007-rung-07-to-08/unrouted-target/dual-esp32.kicad_pcb'
    if args.board is not None:
        board_path, candidate_path = args.board.resolve(), args.candidate.resolve()
    else:
        isolated = json.loads((root / 'isolated-followups/junction-blocked/run/solve/progression.json').read_text())
        candidate_path = Path(isolated['steps'][0]['result']['selected_candidate'])
    candidate = json.loads(candidate_path.read_text())
    clearance = candidate['config']['clearance_mm']
    assert clearance >= 0 and candidate['config']['trace_width_mm'] > 0
    assert not candidate.get('footprint_placements'), 'Candidate must preserve component placement'
    board = pcbnew.LoadBoard(str(board_path))
    assert board.GetCopperLayerCount() == 2, 'This advisory diagnostic currently supports two copper layers'
    old = []
    for item in board.GetTracks():
        net = str(item.GetNetname()).removeprefix('/')
        if net == candidate['connection'].removeprefix('/'):
            continue
        if isinstance(item, pcbnew.PCB_VIA):
            old.append({'net': net, 'a': xy(item.GetPosition()), 'b': xy(item.GetPosition()),
                        'width': pcbnew.ToMM(item.GetWidth(pcbnew.F_Cu)), 'layer': None, 'via': True})
        else:
            assert not isinstance(item, pcbnew.PCB_ARC)
            old.append({'net': net, 'a': xy(item.GetStart()), 'b': xy(item.GetEnd()),
                        'width': pcbnew.ToMM(item.GetWidth()), 'layer': {pcbnew.F_Cu: 'F.Cu', pcbnew.B_Cu: 'B.Cu'}[item.GetLayer()], 'via': False})
    new = [{'a': s['start'], 'b': s['end'], 'width': s['width'], 'layer': s['layer'], 'via': False}
           for s in candidate['supplemental_segments']]
    new += [{'a': v['at'], 'b': v['at'], 'width': v['size'], 'layer': None, 'via': True}
            for v in candidate['supplemental_vias']]
    target_pads = [(str(f.GetReference()), str(p.GetNumber()), p)
                   for f in board.GetFootprints() for p in f.Pads()
                   if str(p.GetNetname()).removeprefix('/') == candidate['connection'].removeprefix('/')]
    access_vias = {}
    for index, primitive in enumerate(new):
        if primitive['via']:
            point = pcbnew.VECTOR2I(int(pcbnew.FromMM(primitive['a'][0])), int(pcbnew.FromMM(primitive['a'][1])))
            touched = [(ref, number) for ref, number, pad in target_pads if pad.HitTest(point)]
            if touched:
                access_vias[index] = touched
    witnesses = []
    for ni, n in enumerate(new):
        for oi, o in enumerate(old):
            if n['layer'] is not None and o['layer'] is not None and n['layer'] != o['layer']:
                continue
            distance = segment_distance(n['a'],n['b'],o['a'],o['b'])
            required = (n['width']+o['width'])/2+clearance
            if distance < required-1e-7:
                witnesses.append({'net': o['net'], 'new_primitive': ni, 'old_primitive': oi,
                                  'centerline_distance_mm': distance, 'required_distance_mm': required,
                                  'via_center_in_target_pads': access_vias.get(ni, [])})
    conflicting = sorted({w['net'] for w in witnesses})
    access_conflicts = sorted({w['net'] for w in witnesses if w['via_center_in_target_pads']})
    foreign = sorted({o['net'] for o in old})
    ranked = access_conflicts + [net for net in conflicting if net not in access_conflicts] + [net for net in foreign if net not in conflicting]
    report = {'board': str(board_path), 'candidate': str(candidate_path),
              'board_sha256': hashlib.sha256(board_path.read_bytes()).hexdigest(),
              'candidate_sha256': hashlib.sha256(candidate_path.read_bytes()).hexdigest(), 'candidate_connection': candidate['connection'],
              'conflicting_nets': conflicting, 'foreign_nets': foreign,
              'target_pad_via_conflicts': access_conflicts, 'ranked_yielding_nets': ranked,
              'ranking_rule': 'First copper conflicting with an isolated-route via centered inside a target pad; then other isolated-route conflicts; then all remaining nets. Lexical ties. Advisory priorities, not proof of required layer changes or sufficient rip-up.',
              'target_pad_via_conflict_details': [
                  {'net': witness['net'], 'target_pads': witness['via_center_in_target_pads'],
                   'proposed_via': new[witness['new_primitive']], 'retained_copper': old[witness['old_primitive']],
                   'centerline_distance_mm': witness['centerline_distance_mm'],
                   'required_distance_mm': witness['required_distance_mm']}
                  for witness in witnesses if witness['via_center_in_target_pads']],
              'witnesses': witnesses, 'scope': __doc__}
    (out / 'analysis.json').write_text(json.dumps(report, indent=2) + '\n')
    bounds = board.GetBoardEdgesBoundingBox()
    x,y=xy(bounds.GetPosition());w,h=pcbnew.ToMM(bounds.GetWidth()),pcbnew.ToMM(bounds.GetHeight())
    svg=[f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{x-2} {y-5} {w+4} {h+8}"><rect x="{x-2}" y="{y-5}" width="{w+4}" height="{h+8}" fill="#142231"/><text x="{x}" y="{y-2}" fill="white" font-size="1.2">Red: target-pad via conflict; orange: other conflicts; cyan: isolated route</text><rect x="{x}" y="{y}" width="{w}" height="{h}" stroke="#aab6c4" stroke-width=".1" fill="none"/>']
    for footprint in board.GetFootprints():
        for pad in footprint.Pads():
            px,py=xy(pad.GetPosition());pw,ph=xy(pad.GetSize())
            svg.append(f'<rect x="{px-pw/2}" y="{py-ph/2}" width="{pw}" height="{ph}" transform="rotate({-pad.GetOrientationDegrees()} {px} {py})" fill="#425164"/>')
    def draw(primitive, color, dashed=False):
        ax,ay=primitive['a'];bx,by=primitive['b'];width=primitive['width']
        if primitive['via']:
            return f'<circle cx="{ax}" cy="{ay}" r="{width/2}" fill="{color}"/>'
        dash=' stroke-dasharray=".6 .3"' if dashed else ''
        return f'<path d="M {ax} {ay} L {bx} {by}" stroke="{color}" stroke-width="{width}" fill="none" stroke-linecap="round"{dash}/>'
    svg += [draw(p,'#ff6b6b' if p['net'] in access_conflicts else '#ffa33d' if p['net'] in conflicting else '#68768b') for p in old]
    svg += [draw(p,'#6fe6ff',True) for p in new]
    for index in sorted({w['new_primitive'] for w in witnesses if w['via_center_in_target_pads']}):
        px,py=new[index]['a'];label=', '.join(ref+'.'+number for ref,number in access_vias[index])
        svg.append(f'<circle cx="{px}" cy="{py}" r=".8" stroke="#fff3b0" stroke-width=".12" fill="none"/><text x="{px+1}" y="{py-.9}" font-size="1.1" fill="#fff3b0">{html.escape(label)}</text>')
    svg.append(f'<text x="{x}" y="{y+h+2}" fill="white" font-size="1.2">Target-pad via conflicts: {html.escape(", ".join(access_conflicts))}; total conflicting nets: {len(conflicting)}</text></svg>')
    (out / 'conflicts.svg').write_text(''.join(svg))
    subprocess.run(['rsvg-convert','--width','1200','--output',str(out/'conflicts.png'),str(out/'conflicts.svg')],check=True)
    if report['target_pad_via_conflict_details']:
        import re
        px,py=report['target_pad_via_conflict_details'][0]['proposed_via']['a']
        detail=re.sub(r'viewBox="[^"]+"',f'viewBox="{px-5} {py-5} 10 10"',''.join(svg),count=1)
        (out/'detail.svg').write_text(detail)
        subprocess.run(['rsvg-convert','--width','700','--output',str(out/'detail.png'),str(out/'detail.svg')],check=True)
    print(json.dumps(report,indent=2))


if __name__ == '__main__':
    main()
