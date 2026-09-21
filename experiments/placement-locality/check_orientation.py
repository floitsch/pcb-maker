#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Run the planted orientation controls, retaining automatic diagnostic views."""
import copy
import json
from pathlib import Path
import subprocess
import sys


def main():
    root = Path(sys.argv[1]).resolve(); root.mkdir(parents=True, exist_ok=False)
    repo = Path(__file__).resolve().parents[2]
    planted = json.loads((repo/'benchmarks/small/orientation-region-control.json').read_text())
    poses = {'poses': [{'component':c['id'], 'position':c['position'], 'rotation_degrees':c.get('rotation_degrees',0)} for c in planted['components']]}
    (root/'poses.json').write_text(json.dumps(poses,indent=2)+'\n')
    for label in ['planted','zero-weight']:
        problem = copy.deepcopy(planted)
        if label == 'zero-weight':
            for net in problem['nets']: net['tension_weight'] = 0
        source = root/f'{label}.problem.json'; source.write_text(json.dumps(problem,indent=2)+'\n')
        with (root/f'{label}.log').open('w') as log:
            subprocess.run([sys.executable,str(Path(__file__).with_name('orientation_probe.py')),str(source),str(root/'poses.json'),str(root/label),'--executable',str(repo/'target/release/pcb-maker')],stdout=log,stderr=subprocess.STDOUT,check=True)
    positive = json.loads((root/'planted/report.json').read_text()); zero = json.loads((root/'zero-weight/report.json').read_text())
    assert positive['positive_counterfactuals'] == 3
    assert positive['tested'][0]['rotation_degrees'] == 0
    assert {r['rotation_degrees']:r['legal'] for r in positive['tested']} == {0:True,90:False,270:False}
    assert zero['positive_counterfactuals'] == 0 and not zero['tested']
    for label in ['planted','zero-weight']: assert (root/label/'details.html').exists()
    (root/'validation.json').write_text(json.dumps({'known_best_rotation_recovered':True,'illegal_quarter_turns_rejected':2,'zero_attraction_no_positive_counterfactuals':True,'automatic_detail_viewers':True},indent=2)+'\n')
    print(root/'validation.json')


if __name__ == '__main__': main()
