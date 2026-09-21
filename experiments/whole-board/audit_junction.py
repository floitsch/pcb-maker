#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Audit a control/split junction experiment using native integer geometry."""
import collections
import hashlib
import json
import math
from pathlib import Path
import sys

import pcbnew

if not hasattr(pcbnew.SwigPyIterator, 'next'):
    pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__


def native(path):
    board = pcbnew.LoadBoard(str(path))
    assert board.GetCopperLayerCount() == 2 and not list(board.Zones())
    poses = sorted((f.GetReference(), f.GetPosition().x, f.GetPosition().y,
                    f.GetOrientationDegrees()) for f in board.GetFootprints())
    intervals = collections.defaultdict(list)
    tracks = []
    for t in board.GetTracks():
        assert not isinstance(t, (pcbnew.PCB_ARC, pcbnew.PCB_VIA))
        a, b = t.GetStart(), t.GetEnd()
        dx, dy = b.x-a.x, b.y-a.y
        divisor = math.gcd(dx, dy)
        assert divisor
        dx, dy = dx//divisor, dy//divisor
        if dx < 0 or (dx == 0 and dy < 0):
            dx, dy = -dx, -dy
        key = (t.GetNetname(), t.GetLayer(), t.GetWidth(), dx, dy, -dy*a.x+dx*a.y)
        intervals[key].append(tuple(sorted((dx*a.x+dy*a.y, dx*b.x+dy*b.y))))
        tracks.append((t.GetNetname(), t.GetLayer(), (a.x/1e6,a.y/1e6),
                       (b.x/1e6,b.y/1e6), t.GetWidth()/1e6))
    support = {}
    for key, values in intervals.items():
        merged = []
        for start, end in sorted(values):
            if merged and start <= merged[-1][1]:
                merged[-1] = (merged[-1][0], max(end, merged[-1][1]))
            else:
                merged.append((start,end))
        support[key] = merged
    return poses, support, tracks


def main():
    out = Path(sys.argv[1])
    evidence = []
    geometries = []
    for label in ('control', 'split'):
        case = out/label
        path = case/'pic_programmer.kicad_pcb'
        poses, support, tracks = native(path)
        geometries.append((poses, support))
        report = json.loads((case/'verification.json').read_text())
        assert report['selected_net_unconnected_items'] == 75
        assert report['erc_violations'] == report['schematic_parity_issues'] == 0
        assert report['drc_design_violations'] == (1 if label == 'control' else 0)
        evidence.append({'case':label, 'board_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),
                         'verification':report, 'stored_segments':len(tracks)})
        parts = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="109 81 10 10" width="700" height="700">',
                 '<rect x="109" y="81" width="10" height="10" fill="#12232b"/>',
                 f'<text x="109.5" y="81.8" fill="white" font-size=".4">{label}: same GND copper, explicit endpoints shown</text>']
        for net, layer, a, b, width in tracks:
            if net != 'GND' or layer != pcbnew.B_Cu:
                continue
            parts.append(f'<path d="M {a[0]} {a[1]} L {b[0]} {b[1]}" fill="none" stroke="#64b5f6" stroke-width="{width}"/>')
            for x,y in (a,b):
                parts.append(f'<circle cx="{x}" cy="{y}" r=".10" fill="#ffcc66"/>')
        parts += ['<circle cx="113.91" cy="85.89" r=".65" fill="none" stroke="#ffcc66" stroke-width=".06"/>',
                  '</svg>']
        (case/'junction.svg').write_text(''.join(parts))
    assert geometries[0] == geometries[1], 'Native poses or exact net/layer/width centerline support changed'
    result = {'cases':evidence, 'native_poses_unchanged':True,
              'native_integer_centerline_support_unchanged':True,
              'same_net_layer_and_width':True, 'board_complete':False}
    (out/'audit.json').write_text(json.dumps(result,indent=2)+'\n')
    (out/'index.html').write_text('<!doctype html><meta charset="utf-8"><h1>PIC explicit junction control</h1>'
        '<p>Identical native centerline support, widths, layers, nets and component poses. '
        'Explicitly splitting the parent segment removes the dangling-track finding.</p>'
        '<img width="650" src="control/junction.svg"><img width="650" src="split/junction.svg">'
        '<p><a href="control/preview.svg">Full control board</a> · '
        '<a href="split/preview.svg">Full split board</a> · <a href="audit.json">Audit</a></p>')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
