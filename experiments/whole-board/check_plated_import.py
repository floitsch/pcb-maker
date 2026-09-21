# Copyright (C) 2026 Toit contributors.
"""Round-trip every via-bearing net and verify/render each complete native board.

Coverage scans must already be terminal. The importer may normalize copper
inside pads; this is a connectivity/geometry check, not a routing quality win.
"""
import argparse
import hashlib
import html
import json
from pathlib import Path
import shutil
import subprocess
import xml.etree.ElementTree as ET

from render_inspection_layers import render
from report_routing_demand import pcbnew, pose


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(root, binary):
    output = root / 'roundtrips'
    output.mkdir(exist_ok=False)
    exe = output / 'pcb-maker'
    shutil.copy2(binary, exe)
    results = []
    for name in ['ecc83', 'pic']:
        discovery = json.loads((root / f'{name}-after/discovery.json').read_text())
        source = Path(discovery['source']).resolve()
        source_hash = digest(source)
        source_board = pcbnew.LoadBoard(str(source))
        source_pose = pose(source_board)
        source_vias = sorted((v.GetNetname(), v.GetPosition().x, v.GetPosition().y,
                              v.GetWidth(pcbnew.F_Cu), v.GetDrillValue())
                             for v in source_board.GetTracks() if isinstance(v, pcbnew.PCB_VIA))
        board_id = source.stem
        for index, entry in enumerate(discovery['coverage']):
            assert entry['status'] == 'scanned', entry
            directory = output / name / f'net-{index:02}'
            shutil.copytree(source.parent, directory)
            board = directory / source.name
            # A copied verification is not evidence for changed copper.
            for filename in ['verification.json', 'erc.json', 'drc.json']:
                (directory / filename).unlink(missing_ok=True)
            candidate = directory / 'import.json'
            commands = [
                ('import', 'import-kicad-route-candidate', source, entry['connection'], candidate),
                ('apply', 'apply-kicad-route-candidate', source, candidate, board),
                ('verify', 'verify-kicad-rung', directory, board_id),
            ]
            for label, *arguments in commands:
                with (directory / f'{label}.log').open('w') as log:
                    subprocess.run([str(exe), *map(str, arguments)], stdout=log,
                                   stderr=subprocess.STDOUT, check=True)
            native = json.loads((directory / 'verification.json').read_text())
            assert native['complete'], native
            for key in ['erc_violations', 'drc_design_violations', 'schematic_parity_issues',
                        'selected_net_unconnected_items']:
                assert native[key] == 0, native
            after = pcbnew.LoadBoard(str(board))
            assert pose(after) == source_pose
            vias = sorted((v.GetNetname(), v.GetPosition().x, v.GetPosition().y,
                           v.GetWidth(pcbnew.F_Cu), v.GetDrillValue())
                          for v in after.GetTracks() if isinstance(v, pcbnew.PCB_VIA))
            assert vias == source_vias, 'Importer changed physical vias'
            for source_file in source.parent.rglob('*'):
                if source_file.suffix in ['.kicad_pro', '.kicad_sch', '.kicad_dru', '.kicad_sym']:
                    assert source_file.read_bytes() == (directory / source_file.relative_to(source.parent)).read_bytes()
            preview = json.loads((directory / 'preview.json').read_text())
            assert preview['source_sha256'] == digest(board)
            render(directory, board_id)
            ET.parse(directory / 'inspection-combined.svg')
            row = dict(board=name, connection=entry['connection'], native=native,
                       source_sha256=source_hash, result_sha256=digest(board),
                       physical_vias_preserved=True, component_poses_preserved=True,
                       image=str((directory/'inspection-combined.svg').relative_to(root)))
            results.append(row)
            (output / 'results.json').write_text(json.dumps(results, indent=2)+'\n')
            print(f'{name} {entry["connection"]}: native-complete, original vias preserved', flush=True)
        assert digest(source) == source_hash
    report = dict(executable_sha256=digest(exe), results=results,
                  scope='Per-net round-trip, full native verification; no routing improvement claimed.')
    (output / 'report.json').write_text(json.dumps(report, indent=2)+'\n')
    page = ['<!doctype html><meta charset="utf-8"><title>Plated pad import validation</title>',
            '<style>body{font:17px system-ui;margin:24px}img{max-width:100%;width:900px}</style>',
            '<h1>Plated pad import validation</h1>',
            '<p>Each image follows an independently imported net written back into the complete board. ',
            'All original vias and component poses are preserved. All boards pass native verification.</p>']
    for row in results:
        page.append(f'<h2>{html.escape(row["board"]+": "+row["connection"])}</h2>'
                    f'<img src="{html.escape(row["image"])}">')
    (root / 'index.html').write_text(''.join(page))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    run(args.root.resolve(), args.binary.resolve())
