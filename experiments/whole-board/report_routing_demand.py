# Copyright (C) 2026 Toit contributors.
"""Audit terminal routing experiments and assemble labelled comparisons.

Run only after every routing process has terminated: sequential-route.json is
also a progress checkpoint while a router is still running.
"""
import argparse
import hashlib
import html
import json
import math
from pathlib import Path
import shutil
import sys
import xml.etree.ElementTree as ET
import pcbnew
from render_inspection_layers import render
from run_routing_demand import write_json

if not hasattr(pcbnew.SwigPyIterator, 'next'):
    pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__


def pose(board):
    return sorted((f.GetReference(), f.GetPosition().x, f.GetPosition().y, f.GetOrientationDegrees(),
                   tuple(sorted((p.GetNumber(), p.GetNetname(), p.GetPosition().x, p.GetPosition().y,
                                 p.GetSize().x, p.GetSize().y, p.GetDrillSize().x, p.GetDrillSize().y,
                                 p.GetShape(), p.GetAttribute(), p.GetLayerSet().FmtHex()) for p in f.Pads())))
                  for f in board.GetFootprints())


def report(root):
    rows = []
    for result_path in sorted(root.rglob('sequential-route.json')):
        sequential = json.loads(result_path.read_text())
        board_id = sequential['board_id']
        directory = result_path.parent / 'result'
        source = Path(sequential['source_directory'])
        native = json.loads((directory / 'verification.json').read_text())
        before = pcbnew.LoadBoard(str(source / f'{board_id}.kicad_pcb'))
        after = pcbnew.LoadBoard(str(directory / f'{board_id}.kicad_pcb'))
        assert pose(before) == pose(after), result_path
        assert (source / f'{board_id}.kicad_pro').read_bytes() == (directory / f'{board_id}.kicad_pro').read_bytes()
        assert (source / f'{board_id}.kicad_sch').read_bytes() == (directory / f'{board_id}.kicad_sch').read_bytes()
        geometry = {}
        for step in sequential['steps']:
            chosen = next((a for a in step['attempts'] if a['selected']), None)
            if chosen:
                candidate = json.loads(Path(chosen['candidate']).read_text())
                geometry[candidate['connection'].removeprefix('/')] = candidate['config']
        for track in after.GetTracks():
            config = geometry[track.GetNetname().removeprefix('/')]
            if isinstance(track, pcbnew.PCB_VIA):
                assert track.GetWidth() == round(config['via_size_mm']*1e6)
                assert track.GetDrillValue() == round(config['via_drill_mm']*1e6)
            else:
                assert track.GetWidth() == round(config['trace_width_mm']*1e6)
        vias = sum(isinstance(t, pcbnew.PCB_VIA) for t in after.GetTracks())
        length = sum(t.GetLength()/1e6 for t in after.GetTracks() if not isinstance(t, pcbnew.PCB_VIA))
        assert vias == sequential['final_statistics']['vias']
        assert math.isclose(length, sequential['final_statistics']['stored_segment_length_mm'], abs_tol=1e-4)
        complete = sequential['termination'] == 'complete'
        if complete:
            assert native['complete']
            assert all(native[k] == 0 for k in ['erc_violations', 'drc_design_violations', 'schematic_parity_issues', 'selected_net_unconnected_items'])
        labels = json.loads((directory / 'preview-labels.json').read_text())['labels']
        assert len({row['label'] for row in labels}) == len(labels)
        assert sum(row['kind'] == 'via' for row in labels) == vias
        assert sum(row['kind'] == 'component' for row in labels) == len(list(after.GetFootprints()))
        rows.append(dict(policy=str(result_path.parent.relative_to(root)), board_id=board_id, directory=str(directory),
                         complete=complete, termination=sequential['termination'], routed_nets=sequential['completed_connections'],
                         vias=vias, track_mm=length, native=native, placement_pads_nets_rules_project_preserved=True,
                         connection_order=sequential['connection_order'], total_expansions=sequential['total_expansions']))
    # Select independently per board. Incomplete trials cannot beat a complete
    # board just because some of their copper has not been routed yet.
    selected = {}
    for row in rows:
        if not row['complete']:
            continue
        score = row['track_mm'] + 5.0*row['vias']
        incumbent = selected.get(row['board_id'])
        if incumbent is None or score < incumbent['score_mm']:
            selected[row['board_id']] = dict(policy=row['policy'], directory=row['directory'], score_mm=score)
    for board_id, winner in selected.items():
        destination = root / 'selected' / board_id
        if destination.exists():
            raise ValueError(f'Refusing to overwrite {destination}')
        shutil.copytree(Path(winner['directory']), destination)
        render(destination, board_id)
    for name in ['ecc83/original', 'ecc83/cost-control', 'ecc83/demand-1']:
        directory = root / name / 'result'
        if directory.exists():
            render(directory, 'ecc83-pp')
    svg_count = 0
    for path in root.rglob('*.svg'):
        ET.parse(path)
        svg_count += 1
    audit = dict(rows=rows, selected=selected, score='Native-complete boards only; track mm + 5 mm per via.',
                 all_svg_xml_parsed=svg_count, browser_playback_tested=False,
                 executable_sha256=hashlib.sha256((root/'ecc83/pcb-maker').read_bytes()).hexdigest())
    write_json(root / 'audit.json', audit)
    page = ['<!doctype html><meta charset="utf-8"><title>Demand and net-order experiments</title>',
            '<style>body{font:17px system-ui;margin:24px}.pair{display:grid;grid-template-columns:1fr 1fr;gap:20px}img{width:100%}td,th{padding:8px;border:1px solid #aaa}table{border-collapse:collapse}</style>',
            '<h1>Demand and net-order experiments</h1><p>Identical placements and physical rules. Routes are generated from zero copper; source human routes are not used for prediction. Component bodies are hidden, references and via IDs are labelled.</p>',
            '<table><tr><th>Board / policy</th><th>Complete</th><th>Nets routed</th><th>Vias</th><th>Track mm</th></tr>']
    for row in rows:
        relative = Path(row['directory']).relative_to(root)
        page.append(f'<tr><td><a href="{relative}/preview.svg">{html.escape(row["policy"])}</a></td><td>{row["complete"]}</td><td>{row["routed_nets"]}</td><td>{row["vias"]}</td><td>{row["track_mm"]:.3f}</td></tr>')
    page.append('</table><p>Incomplete boards are excluded from selection. Seven library metadata warnings remain on ECC83; they are not routing violations.</p>')
    for side in ['combined', 'back', 'front']:
        page.append(f'<h2>ECC83: {side} copper</h2><div class="pair"><div><h3>Original: 10 vias</h3><img src="ecc83/original/result/inspection-{side}.svg"></div><div><h3>Selected</h3><img src="selected/ecc83-pp/inspection-{side}.svg"></div></div>')
    page.append('<h2>Forecast demand before the first net</h2><p>Orange shows predicted competition, before physical obstacle masking. Both layers use the same scale.</p><div class="pair">')
    for side in ['front', 'back']:
        page.append(f'<div><h3>{side}</h3><img src="ecc83/demand-1/step-000-Net--P3-P1-/forecast-cost-{side}.svg"></div>')
    page.append('</div><p><a href="channel/index.html">Shared-passage positive control</a> · <a href="channel-too-narrow/index.html">Too-narrow control</a> · <a href="ecc83-orders/interference.html">Pairwise interference</a> · <a href="audit.json">Independent native readback and full results</a></p>')
    (root / 'index.html').write_text(''.join(page))
    print(json.dumps(selected, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    report(parser.parse_args().root.resolve())
