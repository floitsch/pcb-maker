#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Check source-class routing, native copper dimensions and project preservation."""
import argparse
import collections
import json
from pathlib import Path
import pcbnew

from compile_net_classes import compile_rules
from run import command, digest, read, write

if not hasattr(pcbnew.SwigPyIterator, 'next'):
    pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run',type=Path)
    args = parser.parse_args()
    out = args.run.resolve()
    evidence = read(out/'rules/net-class-evidence.json')
    run = read(out/'run/sequential-route.json')
    source = out/'source/pic_programmer.kicad_pcb'
    final = out/'run/result/pic_programmer.kicad_pcb'
    assert digest(source.with_suffix('.kicad_pro')) == evidence['project_sha256']
    assert digest(final.with_suffix('.kicad_pro')) == evidence['project_sha256']
    resolved, final_classes = compile_rules(final)
    assert resolved == evidence['connection_rules']
    assert final_classes['assignments'] == evidence['assignments']
    source_board, final_board = pcbnew.LoadBoard(str(source)), pcbnew.LoadBoard(str(final))
    def poses(board):
        return sorted((f.GetReference(),f.GetPosition().x,f.GetPosition().y,
                       f.GetOrientationDegrees(),f.GetLayer()) for f in board.GetFootprints())
    assert poses(source_board) == poses(final_board)
    dimensions = collections.defaultdict(lambda: {'segments':0,'vias':0,'length_mm':0.0,'widths_nm':set()})
    for track in final_board.GetTracks():
        connection = str(track.GetNetname()).removeprefix('/')
        rules = resolved[connection]
        entry = dimensions[connection]
        if isinstance(track,pcbnew.PCB_VIA):
            assert track.GetWidth(pcbnew.F_Cu) == round(rules['via_size_mm']*1e6)
            assert track.GetDrillValue() == round(rules['via_drill_mm']*1e6)
            entry['vias'] += 1
        else:
            assert not isinstance(track,pcbnew.PCB_ARC)
            assert track.GetWidth() == round(rules['trace_width_mm']*1e6), connection
            entry['widths_nm'].add(track.GetWidth())
            entry['segments'] += 1
            entry['length_mm'] += track.GetLength()/1e6
    for entry in dimensions.values():entry['widths_nm'] = sorted(entry['widths_nm'])
    for step in run['steps']:
        attempt = next(a for a in step['attempts'] if a['selected'])
        candidate = read(Path(attempt['candidate']))
        config = candidate['config']
        assert config['connection_rules'] == resolved
        assert all(config[k] == v for k,v in resolved[step['connection']].items())
        assert attempt['native_admission']['complete']
    verified = read(final.parent/'verification.json')
    assert verified['complete'] and run['termination']=='complete'
    assert run['source_board_unchanged']
    stats = command([out/'pcb-maker','inspect-kicad-board',final],out/'final-statistics.json')
    assert stats['exit_code']==0
    report = {'scope':__doc__,'project_unchanged':True,'native_class_assignments_unchanged':True,
              'all_track_and_via_dimensions_match_source_classes':True,'poses_unchanged':True,
              'all_accepted_candidate_rule_maps_match':True,'verification':verified,
              'per_connection':dict(dimensions),'statistics':read(out/'final-statistics.json')}
    write(out/'audit.json',report)
    rows = [{'name':s['connection'], 'preview':str(Path('run')/Path(s['artifact_directory']).name/
                f'native-candidate-{s["selected_config_index"]:02d}'/'preview.svg'),
             'opens':next(a for a in s['attempts'] if a['selected'])['native_admission']['verification']['selected_net_unconnected_items']}
            for s in run['steps']]
    data = json.dumps(rows).replace('<','\\u003c')
    (out/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>PIC source net classes</title>'
        '<style>body{font:18px system-ui;margin:25px}input{width:85vw}img{max-width:95vw;max-height:80vh}</style>'
        '<h1>PIC routed with source net classes</h1><p>GND/VCC: 0.8 mm tracks, 0.28 mm clearance. '
        'Default: 0.5 mm tracks, 0.25 mm clearance. Project and native class assignments unchanged.</p>'
        '<input id="step" type="range" min="0" max="'+str(len(rows)-1)+'" value="0"><p id="caption"></p><img id="board">'
        '<p><a href="audit.json">Native dimension and rule audit</a> · <a href="run/result/preview.svg">Final board</a></p>'
        '<script>const rows='+data+';const slider=document.getElementById("step");function show(){const r=rows[+slider.value];'
        'document.getElementById("caption").textContent=(+slider.value+1)+" / "+rows.length+": "+r.name+"; remaining opens: "+r.opens;'
        'document.getElementById("board").src=r.preview;}slider.oninput=show;show();</script>')
    print(json.dumps({'completed_nets':len(rows),'native_complete':True,
        'power_tracks':{k:dimensions[k] for k in ('GND','VCC')},
        'length_mm':report['statistics']['physical_copper']['physical_centerline_length_mm'],
        'vias':report['statistics']['vias']}))


if __name__=='__main__':
    main()
