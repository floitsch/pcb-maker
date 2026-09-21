#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Check native LED completion and preservation of the full placement."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET


def audit(out):
    out = Path(out).resolve()
    manifest = json.loads((out/'manifest.json').read_text())
    assert manifest['native_exit_code'] == 0
    cold = json.loads((out/'native/run/cold-prefix.json').read_text())
    assert cold['completed'] and cold['final_rung'] == 1
    proposal = json.loads((out/'placement.json').read_text())
    assert proposal['poses'] == cold['placement']['poses']
    progress = cold['progression']
    final = Path(cold['final_directory'])
    verification = json.loads((final/'verification.json').read_text())
    assert verification['complete']
    for field in ['erc_violations', 'drc_design_violations', 'schematic_parity_issues', 'selected_net_unconnected_items']:
        assert verification[field] == 0
    def inspect(folder):
        return json.loads(subprocess.run([str(out/'pcb-maker'), 'inspect-kicad-board', str(folder/'dual-esp32.kicad_pcb')], capture_output=True, text=True, check=True).stdout)
    initial = inspect(Path(progress['initial_directory'])); stats = inspect(final)
    assert stats['footprints'] == 43
    assert initial['component_placement_sha256'] == stats['component_placement_sha256']
    ET.parse(final/'preview.svg')
    shutil.copy2(final/'preview.svg', out/'native-preview.svg')
    subprocess.run(['rsvg-convert', '--background-color', 'white', '--width', '1100', '--output', str(out/'native-preview.png'), str(out/'native-preview.svg')], check=True)
    result = {'complete': True, 'proposal_poses_preserved': True, 'native_poses_preserved': True,
              'verification': verification, 'physical_copper_mm': stats['physical_copper']['physical_centerline_length_mm'],
              'vias': stats['vias'], 'astar_expansions': progress['total_route_expansions'],
              'scope': 'Only LED_SERIES routed, at 0.35 mm width / 0.20 mm clearance; all 43 components retained.'}
    (out/'native-audit.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result))
    return result


if __name__ == '__main__': audit(sys.argv[1])
