#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Compile KiCad-resolved net classes into explicit native-router geometry.

KiCad resolves class assignments and inherited routing fields; this script reads
the native effective class without approximating matching or inheritance.
Track width, clearance, and through-via size/drill are compiled per net. Evidence
also exposes KiCad's effective global copper-edge and hole-to-hole clearances.
Trace widths respect the native global minimum as well as the class preference.
Clearances respect the native board minimum as well as the class clearance.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import pcbnew

if not hasattr(pcbnew.SwigPyIterator, 'next'):
    pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__


def validate_explicit_global_rules(project):
    """Reject malformed explicit values before native loading can default them."""
    rules = project
    for key in ('board', 'design_settings', 'rules'):
        if not isinstance(rules, dict):
            raise ValueError(f'Expected a project object before {key}')
        rules = rules.get(key, {})
    if not isinstance(rules, dict):
        raise ValueError('Expected board.design_settings.rules to be an object')
    for key in ('min_copper_edge_clearance', 'min_hole_to_hole', 'min_track_width', 'min_clearance'):
        if key not in rules:
            continue
        value = rules[key]
        try:
            finite = isinstance(value, (int, float)) and math.isfinite(value)
        except OverflowError:
            finite = False
        if isinstance(value, bool) or not finite or value < 0:
            raise ValueError(f'{key} must be a finite nonnegative number')


def compile_rules(path):
    project = path.with_suffix('.kicad_pro')
    assert project.is_file(), 'An explicit project is required for net classes'
    custom = path.with_suffix('.kicad_dru')
    if custom.exists() and custom.read_text().strip():
        raise ValueError('Custom design rules require a separate lowering audit')
    validate_explicit_global_rules(json.loads(project.read_text()))
    board = pcbnew.LoadBoard(str(path))
    settings = board.GetDesignSettings()
    global_rules = dict(edge_clearance_mm=settings.m_CopperEdgeClearance/1e6,
                       hole_to_hole_clearance_mm=settings.m_HoleToHoleMin/1e6)
    minimum_track_width_mm = settings.m_TrackMinWidth/1e6
    minimum_clearance_mm = settings.m_MinClearance/1e6
    if any(not math.isfinite(value) or value < 0
           for value in [*global_rules.values(), minimum_track_width_mm, minimum_clearance_mm]):
        raise ValueError('Unsupported effective native global geometry')
    assignments, geometry = {}, {}
    for raw_name, net in board.GetNetsByName().items():
        name = str(raw_name)
        if not name:
            continue
        connection = name.removeprefix('/')
        assert connection not in assignments, f'Ambiguous normalized net identity: {connection}'
        value = net.GetNetClassSlow()
        class_name = str(value.GetName())
        rule = dict(trace_width_mm=value.GetTrackWidth()/1e6,
                    clearance_mm=value.GetClearance()/1e6,
                    via_size_mm=value.GetViaDiameter()/1e6,
                    via_drill_mm=value.GetViaDrill()/1e6)
        assert rule['trace_width_mm'] > 0 and rule['clearance_mm'] >= 0
        assert 0 < rule['via_drill_mm'] < rule['via_size_mm']
        rule['trace_width_mm'] = max(rule['trace_width_mm'], minimum_track_width_mm)
        rule['clearance_mm'] = max(rule['clearance_mm'], minimum_clearance_mm)
        assignments[connection] = {'native_name':name,'effective_class':class_name}
        geometry[connection] = rule
    return geometry, {'scope':__doc__, 'pcbnew_version':pcbnew.GetBuildVersion(),
        'board_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),
        'project_sha256':hashlib.sha256(project.read_bytes()).hexdigest(),
        'assignments':assignments,'connection_rules':geometry,'global_rules':global_rules,
        'minimum_track_width_mm':minimum_track_width_mm,'minimum_clearance_mm':minimum_clearance_mm}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('board',type=Path)
    parser.add_argument('sequential_config',type=Path)
    parser.add_argument('output',type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True,exist_ok=False)
    rules,evidence = compile_rules(args.board.resolve())
    config = json.loads(args.sequential_config.read_text())
    for entry in config['routing_portfolio']:
        entry['connection_rules'] = rules
    (args.output/'net-class-evidence.json').write_text(json.dumps(evidence,indent=2)+'\n')
    (args.output/'sequential-config.json').write_text(json.dumps(config,indent=2)+'\n')
    print(json.dumps({'nets':len(rules),'classes':sorted({v['effective_class'] for v in evidence['assignments'].values()})}))


if __name__ == '__main__':
    main()
