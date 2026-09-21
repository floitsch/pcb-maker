# Copyright (C) 2026 Toit contributors.
"""Render and check plated contacts, disconnected SMD pads, and a copper cycle."""
import argparse
import json
from pathlib import Path
import subprocess
import xml.etree.ElementTree as ET


def run(output, binary):
    output.mkdir(parents=True, exist_ok=False)
    fixture = Path('crates/pcb-kicad/tests/fixtures/plated-branch.kicad_pcb').read_text()
    smd = fixture.replace('thru_hole circle', 'smd circle').replace('(drill 0.8)', '')
    smd = smd.replace('(layers "*.Cu" "*.Mask") (net "/TARGET")',
                      '(layers "F.Cu") (net "/TARGET")) '
                      '(pad "2" smd circle (at 0 0) (size 2 2) (layers "B.Cu") (net "/TARGET")')
    cycle = fixture.rstrip()[:-1] + '\n(segment (start 7.8 10) (end 12 10) (width 0.25) (layer "F.Cu") (net "/TARGET")))\n'
    results = []

    def check_native(directory, board):
        # Every native check is accompanied by combined copper, even the
        # intentionally disconnected control. These fixtures have no ERC.
        drc = directory / 'drc.json'
        with (directory / 'native.log').open('w') as log:
            subprocess.run(['kicad-cli', 'pcb', 'drc', '--format', 'json',
                            '--output', str(drc), str(board)], stdout=log,
                           stderr=subprocess.STDOUT, check=True)
            image = directory / 'combined.svg'
            subprocess.run(['kicad-cli', 'pcb', 'export', 'svg', '--mode-single',
                            '--layers', 'F.Cu,B.Cu,Edge.Cuts', '--page-size-mode', '2',
                            '--exclude-drawing-sheet', '--output', str(image), str(board)],
                           stdout=log, stderr=subprocess.STDOUT, check=True)
            ET.parse(image)
        return json.loads(drc.read_text())

    for name, text, error, opens in [
        ('plated', fixture, None, 0),
        ('coincident-smd', smd, 'without a plated through-hole pad', 1),
        ('cycle', cycle, 'cycle', 0),
    ]:
        directory = output / name
        directory.mkdir()
        board = directory / 'control.kicad_pcb'
        board.write_text(text)
        native = check_native(directory, board)
        assert len(native['unconnected_items']) == opens, native
        assert not native['violations'], native
        candidate = directory / 'candidate.json'
        result = subprocess.run([str(binary), 'import-kicad-route-candidate', str(board),
                                 'TARGET', str(candidate)], capture_output=True, text=True)
        (directory / 'import.log').write_text(result.stdout + result.stderr)
        if error:
            assert result.returncode != 0 and error in result.stderr, result
        else:
            assert result.returncode == 0, result
            imported = json.loads(candidate.read_text())
            assert len(imported['supplemental_vias']) == 1
            assert len(imported['supplemental_segments']) == 3
            after = directory / 'roundtrip'
            after.mkdir()
            applied = after / board.name
            subprocess.run([str(binary), 'apply-kicad-route-candidate', str(board),
                            str(candidate), str(applied)], check=True)
            checked = check_native(after, applied)
            assert not checked['unconnected_items']
            assert [v['type'] for v in checked['violations']] == [v['type'] for v in native['violations']]
        results.append(dict(control=name, native_opens=opens, import_accepted=not error,
                            native_violation_types=[v['type'] for v in native['violations']],
                            scope='PCB connectivity; synthetic footprints, no schematic/ERC.'))
    (output / 'results.json').write_text(json.dumps(results, indent=2)+'\n')
    (output / 'index.html').write_text(
        '<!doctype html><meta charset="utf-8"><title>Plated contact controls</title>'
        '<style>body{font:17px system-ui}img{width:600px;max-width:100%}</style>'
        '<h1>Plated contact controls</h1>' + ''.join(
            f'<h2>{r["control"]}</h2><p>Native opens: {r["native_opens"]}; '
            f'import accepted: {r["import_accepted"]}</p><img src="{r["control"]}/combined.svg">'
            for r in results))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    run(args.output.resolve(), args.binary.resolve())
