# Copyright (C) 2026 Toit contributors.
"""Native controls for stopping at connectivity completion, with automatic renders.

Replay an archived complete first pass, opt back into multiple optimization
passes, and repeat with a deliberate silkscreen finding. The latter must stop
routing but retain failed full-layout admission and a nonzero CLI exit.
"""
import argparse
import copy
import json
from pathlib import Path
import shutil
import sys

from run import command, digest, read, write


def annotation(board_path):
    from area_probe import load_pcbnew
    board = load_pcbnew().LoadBoard(str(board_path))
    pads = sorted(board.GetPads(), key=lambda p: p.m_Uuid.AsString())
    pad = next(p for p in pads if p.GetNetname())
    x, y = pad.GetPosition().x/1e6, pad.GetPosition().y/1e6
    text = board_path.read_text()
    at = text.rfind(')')
    assert at > 0
    mark = (f'\n(gr_text "ANNOTATION CONTROL" (at {x} {y}) (layer "F.SilkS") '
        '(uuid "8e4a5579-6f06-41f2-9eab-af7f6229cb44") '
        '(effects (font (size 1 1) (thickness 0.15))))\n')
    board_path.write_text(text[:at]+mark+text[at:])


def controls(reference, binary, output):
    output.mkdir(exist_ok=False)
    shutil.copy2(__file__, output/Path(__file__).name)
    exe = output/'pcb-maker'
    shutil.copy2(binary, exe)
    historical = read(reference/'adaptive-routing.json')
    assert historical['passes'][0]['native']['complete']
    source = output/'source'
    shutil.copytree(Path(historical['source_directory']), source)
    board_id = historical['board_id']
    name = board_id+'.kicad_pcb'
    source_hash = digest(source/name)
    marked = output/'annotated-source'
    shutil.copytree(source, marked)
    assert command([sys.executable, __file__, '--annotation', marked/name], output/'annotation.log')['exit_code'] == 0
    assert digest(marked/name) != source_hash
    rows = []
    for label, native, optimize, allow_annotations in [
            ('completion', source, False, False),
            ('optimization', source, True, False),
            ('annotation-completion', marked, False, True)]:
        config = copy.deepcopy(historical['config'])
        config['maximum_passes'] = 2 if optimize else 3
        # Omit the new field for the default-policy controls.
        config.pop('optimize_after_routing_complete', None)
        if optimize:
            config['optimize_after_routing_complete'] = True
        config['allow_existing_annotation_findings'] = allow_annotations
        config_path = output/(label+'-config.json')
        write(config_path, config)
        process = command([exe, 'route-kicad-board-adaptive', native, board_id,
            output/label, config_path], output/(label+'.log'))
        result = read(output/label/'adaptive-routing.json')
        assert result['routing_complete']
        assert len(result['passes']) == (2 if optimize else 1)
        assert result['config']['optimize_after_routing_complete'] == optimize
        assert result['termination'] == ('maximum_passes' if optimize else 'routing_complete')
        assert result['complete'] == (not allow_annotations)
        assert process['exit_code'] == (1 if allow_annotations else 0)
        assert (result['outstanding_annotation_findings'] > 0) == allow_annotations
        first = read(output/label/'pass-000/sequential-route.json')
        expected = historical['passes'][0]['sequential']['final_statistics']['copper_geometry_sha256']
        assert first['final_statistics']['copper_geometry_sha256'] == expected
        rows.append(dict(case=label, process=process, passes=len(result['passes']),
            termination=result['termination'], native=result['result_verification'],
            first_pass_copper_matches_reference=True))
        write(output/'controls.json', dict(status='running', cases=rows))
    assert digest(source/name) == source_hash
    audit = command([sys.executable, Path(__file__).with_name('report_adaptive_routing.py'), output], output/'audit.log')
    assert audit['exit_code'] == 0
    write(output/'controls.json', dict(status='passed', cases=rows, audit=audit,
        executable_sha256=digest(exe), source_unchanged=True))
    print(json.dumps(rows))


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--annotation':
        annotation(Path(sys.argv[2]))
    else:
        parser = argparse.ArgumentParser(description=__doc__)
        for name in ['reference', 'binary', 'output']:
            parser.add_argument(name, type=Path)
        args = parser.parse_args()
        controls(args.reference.resolve(), args.binary.resolve(), args.output.resolve())
