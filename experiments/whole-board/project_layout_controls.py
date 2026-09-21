# Copyright (C) 2026 Toit contributors.
"""Native-project extraction controls, rendered alongside their assertions."""
import argparse
import json
from pathlib import Path
import subprocess

from area_probe import extract, load_pcbnew, outline_polygon, render, vec


def run(output, binary):
    output.mkdir(parents=True, exist_ok=False)
    text = ['(kicad_pcb (version 20241229) (generator "pcbnew")',
            '(general (thickness 1.6)) (paper "A4")',
            '(layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))']
    # A subdivided, nonrectangular boundary has a known area of 375 mm².
    points = [(0, 0), (10, 0), (20, 0), (20, 20), (0, 17.5)]
    for a, b in zip(points, points[1:]+points[:1]):
        text.append(f'(gr_line (start {a[0]} {a[1]}) (end {b[0]} {b[1]}) '
                    '(stroke (width 0.05) (type default)) (layer "Edge.Cuts"))')
    for ref, x, y in [('A', 4, 5), ('B', 15, 5), ('H', 10, 14)]:
        text.append(f'(footprint "test" (layer "F.Cu") (at {x} {y}) '
                    f'(property "Reference" "{ref}" (at 0 -2) (layer "F.SilkS") '
                    '(effects (font (size 1 1) (thickness 0.15))))'
                    '(fp_circle (center 0 0) (end 2 0) '
                    '(stroke (width 0.05) (type default)) (layer "F.CrtYd"))')
        if ref == 'H':
            text.append('(pad "" np_thru_hole circle (at 0 0) (size 1 1) (drill 1) (layers "*.Cu" "*.Mask")))')
        else:
            for dy in ([0, 1] if ref == 'A' else [0]):
                drill='(drill 0.4 (offset 0 0.1))' if ref=='A' and dy==1 else '(drill 0.4)'
                text.append(f'(pad "1" thru_hole circle (at 0 {dy}) (size 0.8 0.8) '
                            f'{drill} (layers "*.Cu" "*.Mask") (net "/TARGET"))')
            text.append(')')
    text.append(')')
    source = output/'control.kicad_pcb'
    source.write_text('\n'.join(text))
    with (output/'render.log').open('w') as log:
        subprocess.run(['kicad-cli', 'pcb', 'export', 'svg', '--mode-single',
                        '--layers', 'F.Cu,B.Cu,Edge.Cuts', '--page-size-mode', '2',
                        '--exclude-drawing-sheet', '--output', str(output/'combined.svg'), str(source)],
                       stdout=log, stderr=subprocess.STDOUT, check=True)
    native = load_pcbnew().LoadBoard(str(source))
    policy = dict(movement='explicit', movable_components=['A'], rotation='original_fixed',
                  body_overhang_mm={}, outline='free_rectangle')
    rules = {'TARGET':dict(trace_width_mm=0.27, clearance_mm=0.13, via_size_mm=0.8, via_drill_mm=0.4)}
    problem, mapping, bounds = extract(native, policy, rules)
    assert len(outline_polygon(native)) == 5
    assert abs((bounds[2]-bounds[0])*(bounds[3]-bounds[1])-375) < 1e-9
    assert len(problem['components']) == 3 and len(problem['electrical_nets']) == 1
    net = problem['electrical_nets'][0]
    assert net['width'] == 0.27 and len(net['terminals']) == 2
    components = {c['id']:c for c in problem['components']}
    assert len(components['A']['pins']) == 1
    assert len(components['A']['pins'][0]['pads']) == 4
    assert len({(p['local_center']['x'],p['local_center']['y']) for p in components['A']['pins'][0]['pads']}) == 2
    pcbnew=load_pcbnew()
    for footprint in native.GetFootprints():
        ref=str(footprint.GetReference());center=mapping[ref]['source_center']
        shapes={p['id']:p for pin in components[ref]['pins'] for p in pin['pads']}
        for pad in footprint.Pads():
            for layer,name in [(pcbnew.F_Cu,'top'),(pcbnew.B_Cu,'bottom')]:
                if not pad.IsOnLayer(layer):continue
                shape=shapes[pad.m_Uuid.AsString()+':'+name]
                actual=pad.ShapePos(layer)
                assert abs(center['x']+shape['local_center']['x']-actual.x/1e6)<1e-6
                assert abs(center['y']+shape['local_center']['y']-actual.y/1e6)<1e-6
    assert components['A']['constraints']['movement'] == 'free'
    for ref in ['B', 'H']:
        assert components[ref]['constraints']['movement'] == 'fixed'
        assert components[ref]['position'] == mapping[ref]['source_center']
    assert components['H']['pins'][0]['id'].startswith('@unnumbered:')
    assert problem['rules']['clearance'] >= 0.13
    # Native component centers are used here only as a schema/control pose.
    # Real cold runs randomize every component allowed to move.
    components['A']['position'] = vec(4, 5)
    serialized = output/'problem.json'
    serialized.write_text(json.dumps(problem, indent=2)+'\n')
    with (output/'check.log').open('w') as log:
        subprocess.run([str(binary), 'check', str(serialized)], stdout=log,
                       stderr=subprocess.STDOUT, check=True)
    poses = [dict(component=c['id'], position=c['position'], rotation_degrees=0) for c in problem['components']]
    render(problem, poses, output/'placement.svg', 'Grouped physical pads and fixed-component control')
    for invalid in [dict(policy, movable_components=['MISSING']),
                    dict(policy, fixed_components=['A']),
                    dict(policy, outline='scaled_source')]:
        try:
            extract(native, invalid, rules)
        except AssertionError:
            pass
        else:
            raise AssertionError('Invalid constraint policy accepted')
    (output/'results.json').write_text(json.dumps(dict(
        passed=True, actual_source_area_mm2=375, physical_pads_retained=True, offset_copper_matches_native_shape_positions=True,
        logical_terminals=2, explicit_fixed_positions_preserved=True,
        unknown_policy_and_reference_rejected=True, native_rule_width_mm=net['width'],
        scope='Native extraction and production schema validation; not a routed-board or ERC result.'), indent=2)+'\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--binary', type=Path, default=Path('target/release/pcb-maker'))
    args = parser.parse_args()
    run(args.output.resolve(), args.binary.resolve())
