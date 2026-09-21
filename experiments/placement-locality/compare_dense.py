#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Compare corrected placement against historical harmonic prefixes, with scope limits."""
import argparse
import copy
import json
import os
from pathlib import Path


def read(path): return json.loads(path.read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('corrected', type=Path)
    parser.add_argument('--reference', type=Path, default=Path('build/placement-dense-prefix19-2026-09-07'))
    parser.add_argument('--current-name', default='corrected')
    parser.add_argument('--reference-name', default='harmonic')
    parser.add_argument('--orientations-only', action='store_true')
    args = parser.parse_args(); root = args.corrected.resolve(); old = args.reference.resolve()
    current = read(root/'summary.json'); reference = read(old/'summary.json')
    assert current['terminal'] and reference['terminal']
    cm, rm = read(root/'manifest.json'), read(old/'manifest.json')
    cases = []; comparisons = []
    for profile in ['blocked','open']:
        cr = next(c for c in current['cases'] if c['id'] == f'{args.current_name}-{profile}')
        rr = next(c for c in reference['cases'] if c['id'] == f'{args.reference_name}-{profile}')
        for filename in ['routing.json','template.json']:
            assert read(root/cr['id']/filename) == read(old/rr['id']/filename)
        cp, rp = read(root/cr['id']/'problem.json'), read(old/rr['id']/'problem.json')
        if args.orientations_only:
            assert {c['id']:c['position'] for c in cp['components']} == {c['id']:c['position'] for c in rp['components']}
        for p in [cp, rp]:
            for c in p['components']:
                c.pop('position'); c.pop('rotation_degrees',None)
        assert cp == rp, 'The comparison must preserve every non-pose problem property'
        common = min(cr['final_rung'], rr['final_rung'])
        cf = next(f for f in cr['frames'] if f['rung'] == common)
        rf = next(f for f in rr['frames'] if f['rung'] == common)
        comparisons.append({'profile':profile,'matched_rung':common,
                            'reference':{k:rf[k] for k in ['length_mm','vias','search_state_expansions']},
                            'corrected':{k:cf[k] for k in ['length_mm','vias','search_state_expansions']}})
        for source, row in [(old,rr),(root,cr)]:
            row = copy.deepcopy(row)
            if source == old: row['id'] = 'historical-'+row['id']
            for frame in row['frames']:
                frame['preview'] = os.path.relpath(source/frame['preview'],root)
            cases.append(row)
    scope = f'Historical {args.reference_name} versus {args.current_name}, with identical non-pose problem properties and routing/template configurations. Executables differ; this is not an isolated binary comparison. Use matched prefixes. The first 19 signal connections exclude LED_SERIES, VCC and GND; their full connectivity still determined placement. Search states are A* plus preflight, not CPU time.'
    data = {'terminal':True,'target_connections':19,'cases':cases,'scope':scope}
    result = {'scope':scope,'reference_executable':rm['executable_sha256'],'corrected_executable':cm['executable_sha256'],
              'orientations_only_asserted':args.orientations_only,'same_non_pose_problem':True,'same_routing_and_template':True,'comparisons':comparisons}
    (root/'historical-comparison.json').write_text(json.dumps(result,indent=2)+'\n')
    template = (Path(__file__).resolve().parents[1]/'placement-exploration/dense_viewer.html').read_text()
    template = template.replace('summary.json','historical-comparison.json').replace('Frozen inputs and subprocess outcomes','Corrected-run frozen inputs and subprocess outcomes')
    (root/'historical-comparison.html').write_text(template.replace('DATA_PLACEHOLDER',json.dumps(data).replace('</','<\\/')).replace('DIAGNOSTICS_LINK',''))
    print(json.dumps(result,indent=2))


if __name__ == '__main__':main()
