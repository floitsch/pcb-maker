#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Reproduce the LED locality regression, automatically rendering every proposal.

The complete netlist determines placement. The optional native probe then routes
only LED_SERIES at its declared width, preserving all 43 component poses.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time
from render import render

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--executable', type=Path, default=ROOT/'target/release/pcb-maker')
    parser.add_argument('--native', action='store_true')
    parser.add_argument('--refine-orientations', action='store_true', help='Enable bounded production orientation refinement after legalization')
    parser.add_argument('--poses', type=Path, help='Use a previously validated full placement instead of computing a harmonic seed')
    args = parser.parse_args()
    out = args.output.resolve(); out.mkdir(parents=True, exist_ok=False)
    exe = out/'pcb-maker'; shutil.copy2(args.executable, exe)
    source = ROOT/'benchmarks/imported/layout-trace/dual-esp32-benchmark.json'
    problem = json.loads(source.read_text())
    config = {'policy': {'kind': 'harmonic_ports', 'iterations': 128,
                        'legalization_sweeps': 256, 'maximum_pair_checks': 1000000,
                        'connected_pair_spacing_floor': True},
              'seed': 0, 'projection_sweeps': 512}
    if args.refine_orientations:
        assert not args.poses, "Use harmonic input for this refinement comparison"
        config["orientation_refinement"] = {}
    supplied = None
    if args.poses:
        supplied = json.loads(args.poses.read_text())['poses']
        by = {p['component']: p for p in supplied}
        assert set(by) == {c['id'] for c in problem['components']}
        assert len(by) == len(supplied)
        from render import placement
        assert placement.independent_pose_check(problem, supplied)['passed']
        for component in problem['components']:
            pose = by[component['id']]
            component.update(position=pose['position'], rotation_degrees=pose['rotation_degrees'])
        config['policy'] = {'kind': 'declared'}
    def write(name, data):
        (out/name).write_text(json.dumps(data, indent=2)+'\n')
    write('problem.json', problem); write('config.json', config)
    manifest = {'scope': __doc__, 'executable_sha256': hashlib.sha256(exe.read_bytes()).hexdigest(),
                'input_sha256': hashlib.sha256((out/'problem.json').read_bytes()).hexdigest()}
    if args.poses:
        manifest['supplied_poses_sha256'] = hashlib.sha256(args.poses.read_bytes()).hexdigest()
        manifest['placement_mode'] = 'retain supplied poses; validate without admitting any projection move'
    write('manifest.json', manifest)
    start = time.monotonic()
    with (out/'place.log').open('w') as log:
        result = subprocess.run([str(exe), 'place', str(out/'problem.json'), str(out/'config.json'), str(out/'placement.json')], stdout=log, stderr=subprocess.STDOUT)
    manifest.update(placement_exit_code=result.returncode, placement_seconds=time.monotonic()-start)
    write('manifest.json', manifest)
    if result.returncode:
        from render import placement
        render(problem, {'poses': placement.poses_of(problem)}, out/'rejected-input.json')
        raise RuntimeError('Placement rejected; input rendered and error retained')
    poses = json.loads((out/'placement.json').read_text())
    if supplied is not None:
        assert {p['component']: p for p in poses['poses']} == {p['component']: p for p in supplied}
    audit = render(problem, poses, out/'placement.json')
    assert audit['passed'], audit
    if args.native:
        native = out/'native'; native.mkdir()
        proposed = copy.deepcopy(problem); by = {p['component']: p for p in poses['poses']}
        for component in proposed['components']:
            p = by[component['id']]
            component.update(position=p['position'], rotation_degrees=p['rotation_degrees'])
        template = json.loads((ROOT/'benchmarks/esp32-pad-gaps/dual-prefix19-blocked/template.json').read_text())
        template['connection_order'] = ['LED_SERIES'] + [n for n in template['connection_order'] if n != 'LED_SERIES']
        routing = json.loads((ROOT/'benchmarks/esp32-pad-gaps/dual-prefix19-blocked/routing-physical.json').read_text())
        routing['local_routing_portfolio'][0]['trace_width_mm'] = next(n['width'] for n in problem['nets'] if n['id'] == 'LED_SERIES')
        for name, data in [('problem.json', proposed), ('placement.json', {'policy': {'kind': 'declared'}, 'projection_sweeps': 1024}), ('template.json', template), ('routing.json', routing)]:
            (native/name).write_text(json.dumps(data, indent=2)+'\n')
        command = [str(exe), 'solve-semantic-kicad-prefix', str(native/'problem.json'), str(native/'placement.json'), str(native/'template.json'), '1', str(native/'run'), str(native/'routing.json')]
        manifest['native_command'] = command; write('manifest.json', manifest)
        start = time.monotonic()
        with (native/'run.log').open('w') as log:
            result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
        manifest.update(native_exit_code=result.returncode, native_seconds=time.monotonic()-start)
        write('manifest.json', manifest)
        assert result.returncode == 0
        from audit_native import audit
        audit(out)
    print(json.dumps(manifest), flush=True)


if __name__ == '__main__':
    main()
