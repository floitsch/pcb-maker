#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Audit and render retained dense-routing counterfactuals."""
import argparse
from collections import Counter
import hashlib
import html
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import xml.etree.ElementTree as ET

import pcbnew
if not hasattr(pcbnew.SwigPyIterator, 'next'):
    pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__


def read(path):
    return json.loads(path.read_text())


def copper(board, ignored=()):
    rows = []
    for item in board.GetTracks():
        net = str(item.GetNetname()).removeprefix('/')
        if net in ignored:
            continue
        if isinstance(item, pcbnew.PCB_VIA):
            rows.append((net, 'via', item.GetPosition().x, item.GetPosition().y,
                         item.GetWidth(pcbnew.F_Cu), item.GetWidth(pcbnew.B_Cu), item.GetDrillValue(),
                         item.TopLayer(), item.BottomLayer()))
        else:
            assert not isinstance(item, pcbnew.PCB_ARC)
            ends = sorted([(item.GetStart().x, item.GetStart().y), (item.GetEnd().x, item.GetEnd().y)])
            rows.append((net, 'segment', item.GetLayer(), item.GetWidth(), *ends))
    return Counter(rows)


def pads(board, target):
    return sorted((str(f.GetReference()), str(p.GetNumber()), p.GetPosition().x, p.GetPosition().y,
                   p.GetSize().x, p.GetSize().y, p.GetShape(), round(p.GetOrientationDegrees(), 9),
                   p.GetDrillSize().x, p.GetDrillSize().y, p.GetOffset().x, p.GetOffset().y,
                   p.GetLayerSet().FmtHex(), p.GetRoundRectRadiusRatio(),
                   str(p.GetNetname()).removeprefix('/') == target)
                  for f in board.GetFootprints() for p in f.Pads())


def fixed_board_shapes(path):
    # Compare serialized outlines and rule areas independently of UUIDs.
    tokens = iter(re.findall(r'"(?:\\.|[^"\\])*"|[()]|[^\s()]+', path.read_text()))
    def node():
        result = []
        for token in tokens:
            if token == ')':
                return result
            if token == '(':
                child = node()
                if child and child[0] not in ['uuid', 'tstamp']:
                    result.append(child)
            else:
                result.append(token)
        return result
    assert next(tokens) == '('
    board = node()
    return [n for n in board if isinstance(n, list) and (n[0] == 'zone' or
            (n[0].startswith('gr_') and ['layer', '"Edge.Cuts"'] in n))]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    root = parser.parse_args().directory.resolve()
    exe = root / 'pcb-maker'
    digest = hashlib.sha256(exe.read_bytes()).hexdigest()
    records = []
    for kind in ['guided', 'isolated', 'ripup']:
        for manifest_path in sorted(root.glob(f'{kind}-followups/*/manifest.json')):
            manifest = read(manifest_path)
            folder = manifest_path.parent
            record = {'kind': kind, 'directory': str(folder), 'case': manifest['case'], 'status': manifest['status']}
            records.append(record)
            if manifest['status'] != 'terminal':
                continue
            assert digest == manifest['executable_sha256']
            original = root / manifest['case']
            base = read(original / 'run/solve/progression.json')
            progress = read(folder / ('run/solve/progression.json' if kind == 'isolated' else 'run/progression.json'))
            assert progress['termination'] != 'running'
            final = Path(progress['final_directory'])
            final_board = final / 'dual-esp32.kicad_pcb'
            initial = Path(progress['initial_directory'])
            stats = json.loads(subprocess.run([str(exe), 'inspect-kicad-board', str(final_board)], capture_output=True, text=True, check=True).stdout)
            initial_stats = json.loads(subprocess.run([str(exe), 'inspect-kicad-board', str(initial / 'dual-esp32.kicad_pcb')], capture_output=True, text=True, check=True).stdout)
            assert stats['component_placement_sha256'] == initial_stats['component_placement_sha256']
            verification = read(final / 'verification.json')
            assert verification['complete']
            target = base['steps'][-1]['result']['evidence']['inserted_connection']
            if kind == 'isolated':
                cold = read(folder / 'run/cold-prefix.json')
                assert cold['placement']['poses'] == read(original / 'run/cold-prefix.json')['placement']['poses']
                before_path = Path(base['steps'][-1]['transaction_directory']) / 'unrouted-target/dual-esp32.kicad_pcb'
                before = pcbnew.LoadBoard(str(before_path))
                isolated_path = Path(progress['steps'][0]['transaction_directory']) / 'unrouted-target/dual-esp32.kicad_pcb'
                isolated = pcbnew.LoadBoard(str(isolated_path))
                assert not copper(isolated)
                assert pads(before, target) == pads(isolated, target)
                assert fixed_board_shapes(before_path) == fixed_board_shapes(isolated_path)
                assert read(before_path.with_suffix('.kicad_pro'))['board']['design_settings'] == read(isolated_path.with_suffix('.kicad_pro'))['board']['design_settings']
                record['isolation_audit'] = 'Same proposed poses, physical pads and target membership, outlines, rule areas and native design settings; zero initial copper.'
            prep = 0
            preflight = 0
            work_complete = True
            for step in progress['steps']:
                result = step['result']
                evidence = result['evidence']
                assert evidence['parent_unchanged']
                if kind == 'ripup':
                    for attempt in evidence['attempts']:
                        repair = attempt.get('single_connection_ripup')
                        if repair:
                            assert repair['source_unchanged']
                            preflight += repair['diagnosis']['total_reachability_expansions']
                            for reroute in repair['attempts']:
                                if reroute['rerouted_candidate']:
                                    candidate = read(Path(reroute['rerouted_candidate']))
                                    preflight += candidate.get('reachability_expansions', 0)
                                    prep += candidate.get('heuristic_expansions', 0)
                                elif reroute['rerouted_route_expansions'] is not None:
                                    work_complete = False
                            if repair['selected_attempt'] is not None:
                                selected_repair = repair['attempts'][repair['selected_attempt']]
                                ignored = [evidence['inserted_connection'], selected_repair['yielding_connection']]
                                parent = Path(step['transaction_directory']) / 'source-parent/dual-esp32.kicad_pcb'
                                child = Path(result['selected_directory']) / 'dual-esp32.kicad_pcb'
                                assert copper(pcbnew.LoadBoard(str(parent)), ignored) == copper(pcbnew.LoadBoard(str(child)), ignored)
                                record['yielded_connection'] = selected_repair['yielding_connection']
                                record['other_retained_copper_unchanged'] = True
                            record['yielding_trials'] = len(repair['diagnosis']['trials'])
                            record['successful_counterfactuals'] = repair['diagnosis']['successful_counterfactuals']
                        elif attempt.get('error'):
                            count = re.search(r'reachability preflight used (\d+) expansions', attempt['error'])
                            assert count
                            preflight += int(count[1])
                if result['disposition'] != 'committed':
                    assert evidence['rollback_exact']
                    if kind != 'ripup':
                        work_complete = False
                    continue
                selected = Path(result['selected_directory'])
                ET.parse(selected / 'preview.svg')
                assert read(selected / 'verification.json')['complete']
                if kind in ['guided', 'isolated']:
                    candidate = read(Path(result['selected_candidate']))
                    prep += candidate.get('heuristic_expansions', 0)
                    preflight += candidate.get('reachability_expansions', 0)
                if kind == 'guided':
                    parent = Path(step['transaction_directory']) / 'source-parent/dual-esp32.kicad_pcb'
                    assert copper(pcbnew.LoadBoard(str(parent)), [candidate['connection']]) == copper(pcbnew.LoadBoard(str(selected / 'dual-esp32.kicad_pcb')), [candidate['connection']])
            record.update(final_rung=progress['final_rung'], target_connection=target,
                          termination=progress['termination'], exit_code=manifest['exit_code'],
                          length_mm=stats['physical_copper']['physical_centerline_length_mm'], vias=stats['vias'],
                          astar_expansions=progress['total_route_expansions'], heuristic_expansions=prep if work_complete else None,
                          preflight_expansions=preflight if work_complete else None, work_accounting_complete=work_complete,
                          native_placement_unchanged=True, verification=verification)
            if kind == 'guided':
                record['all_non_target_copper_unchanged_per_step'] = True
            if kind == 'ripup':
                record['steps'] = progress['steps']
            shutil.copy2(final / 'preview.svg', folder / 'final-preview.svg')
            subprocess.run(['rsvg-convert', '--background-color', 'white', '--width', '1000', '--output', str(folder / 'final-preview.png'), str(folder / 'final-preview.svg')], check=True)
            record['preview'] = os.path.relpath(folder / 'final-preview.svg', root)
            (folder / 'statistics.json').write_text(json.dumps(stats, indent=2) + '\n')
    (root / 'diagnostics.json').write_text(json.dumps(records, indent=2) + '\n')
    cards = []
    for record in records:
        visible = {k: v for k, v in record.items() if k not in ['steps', 'directory', 'preview']}
        card = '<article><h2>' + html.escape(record['kind'] + ' / ' + record['case']) + '</h2><pre>' + html.escape(json.dumps(visible, indent=2)) + '</pre>'
        if 'preview' in record:
            card += '<a href="' + html.escape(record['preview']) + '"><img src="' + html.escape(record['preview']) + '"></a>'
        cards.append(card + '</article>')
    (root / 'diagnostics.html').write_text('<!doctype html><meta charset="utf-8"><title>Dense routing diagnostics</title><style>body{background:#17222d;color:white;font:16px system-ui;margin:24px}a{color:#9df}article{border-top:1px solid #abc;padding-top:20px}pre{white-space:pre-wrap}img{width:100%;max-width:1000px;background:white}</style><h1>Dense routing diagnostics</h1><p>Separate counterfactuals from the frozen six-case comparison. Guided search keeps its preprocessing work explicit. An isolated connection is a one-connection test, not a complete dense board.</p><p><a href="index.html">Original comparison</a> · <a href="diagnostics.json">Full diagnostic evidence</a></p>' + ''.join(cards))
    print(json.dumps([{k: v for k, v in r.items() if k not in ['steps', 'verification']} for r in records], indent=2))


if __name__ == '__main__':
    main()
