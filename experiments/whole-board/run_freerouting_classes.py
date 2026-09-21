#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Run a class-preserving Freerouting reference on an existing cold source.

The caller supplies a terminal pcb-maker source-class study for matched-input
comparison, including incomplete runs. This does not reuse the uniform-rule
competitive manifest.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import time

import pcbnew
from audit_dsn_classes import audit
from compile_net_classes import compile_rules
from run import ROOT, command, digest, read, write


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('pcb_maker_study',type=Path)
    parser.add_argument('output',type=Path)
    parser.add_argument('--resume-import',action='store_true',help='Resume only a recorded successful Java session; preserve the earlier import')
    args = parser.parse_args()
    study = args.pcb_maker_study.resolve()
    recorded = read(study/'run/sequential-route.json')
    assert recorded['termination'] in ('complete','routing_failed','bound_reached')
    out = args.output.resolve()
    if args.resume_import:
        report = read(out/'report.json')
        assert report['router_exit_code']==0
        assert digest(out/'output.ses')==report['session_sha256']
        assert digest(out/'source'/(recorded['board_id']+'.kicad_pcb'))==report['source_sha256']
        finish_import(study,out,out/'pcb-maker',report)
        return
    out.mkdir(parents=True,exist_ok=False)
    exe = out/'pcb-maker'
    shutil.copy2(study/'pcb-maker',exe)
    source = out/'source'
    shutil.copytree(study/'source',source)
    name = recorded['board_id']
    board_path = source/(name+'.kicad_pcb')
    assert digest(board_path)==recorded['source_board_sha256_before']
    project_hash = digest(board_path.with_suffix('.kicad_pro'))
    jar = Path('/tmp/freerouting-2.2.4.jar')
    assert digest(jar)=='f5ed374182900ccc78e473518bbb9f6b869f4a07159495f663a76f52bb10523b'
    report = {'scope':__doc__,'pcb_maker_study':str(study),
              'source_sha256':digest(board_path),'source_project_sha256':project_hash,
              'jar_sha256':digest(jar),'maximum_seconds':600,'maximum_passes':100,'maximum_threads':1,
              'executable_sha256':digest(exe),'status':'preflight'}
    write(out/'report.json',report)
    initial = command([exe,'verify-kicad-rung',source,name],out/'source-verify.log')
    assert initial['exit_code'] in (0,1)
    report['source_verification'] = read(source/'verification.json')
    assert report['source_verification']['selected_net_unconnected_items']==recorded['expected_source_unconnected_items']
    assert command([exe,'inspect-kicad-board',board_path],out/'source-statistics.json')['exit_code']==0
    source_stats = read(out/'source-statistics.json')
    assert all(source_stats[k]==0 for k in ['segments','arcs','vias','copper_zones','copper_graphics'])
    board = pcbnew.LoadBoard(str(board_path))
    dsn = out/'input.dsn'
    assert pcbnew.ExportSpecctraDSN(board,str(dsn))
    report['dsn_audit'] = audit(board_path,dsn)
    assert report['dsn_audit']['class_geometry_matches']
    expected_rules, _ = compile_rules(board_path)
    our_config = read(study/'rules/sequential-config.json')
    assert all(entry.get('connection_rules') == expected_rules for entry in our_config['routing_portfolio'])
    report['pcb_maker_config_class_rules_match'] = True
    write(out/'dsn-audit.json',report['dsn_audit'])
    assert digest(board_path)==report['source_sha256']
    session = out/'output.ses'
    java = '/usr/lib/jvm/java-26-openjdk/bin/java'
    cmd = [java,'-Djava.awt.headless=true','--enable-final-field-mutation=ALL-UNNAMED',
           '-jar',str(jar),'-de',str(dsn),'-do',str(session),'-mp','100','-mt','1']
    report.update(status='routing',command=cmd)
    write(out/'report.json',report)
    start = time.monotonic()
    with (out/'freerouting.log').open('w') as log:
        try:
            result = subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT,timeout=600)
            report['router_exit_code'] = result.returncode
        except subprocess.TimeoutExpired:
            report.update(status='timed out',router_seconds=time.monotonic()-start)
            write(out/'report.json',report)
            raise
    report['router_seconds'] = time.monotonic()-start
    assert result.returncode==0 and session.exists()
    report.update(status='importing',session_sha256=digest(session))
    write(out/'report.json',report)
    finish_import(study,out,exe,report)


def finish_import(study,out,exe,report):
    recorded = read(study/'run/sequential-route.json')
    name = recorded['board_id']
    source = out/'source'
    board_path = source/(name+'.kicad_pcb')
    project_hash = report['source_project_sha256']
    assert digest(board_path)==report['source_sha256']==recorded['source_board_sha256_before']
    assert digest(board_path.with_suffix('.kicad_pro'))==project_hash
    expected_rules, _ = compile_rules(board_path)
    our_config = read(study/'rules/sequential-config.json')
    assert our_config['routing_portfolio']
    assert all(entry.get('connection_rules') == expected_rules for entry in our_config['routing_portfolio'])
    report['pcb_maker_config_class_rules_match'] = True
    session = out/'output.ses'
    source_stats = read(out/'source-statistics.json')
    destination = out/'result'
    if destination.exists():
        destination.rename(out/'prior-import')
    shutil.copytree(source,destination)
    final_path = destination/board_path.name
    final = pcbnew.LoadBoard(str(final_path))
    assert pcbnew.ImportSpecctraSES(final,str(session))
    pcbnew.SaveBoard(str(final_path),final)
    # SaveBoard migrates project defaults as well as writing the PCB. Routing
    # admission must use the original project, including severity settings.
    shutil.copy2(final_path.with_suffix('.kicad_pro'),out/'project-after-import.kicad_pro')
    report['imported_project_sha256'] = digest(final_path.with_suffix('.kicad_pro'))
    shutil.copy2(board_path.with_suffix('.kicad_pro'),final_path.with_suffix('.kicad_pro'))
    report['original_project_restored'] = True
    write(out/'report.json',report)
    result_rules, assignments = compile_rules(final_path)
    original_rules, original_assignments = compile_rules(board_path)
    assert result_rules == original_rules
    assert assignments['assignments'] == original_assignments['assignments']
    assert digest(final_path.with_suffix('.kicad_pro')) == project_hash
    report['class_assignments_and_project_unchanged'] = True
    # Read dimensions after native serialization, not just from the Java log.
    check = pcbnew.LoadBoard(str(final_path))
    dimensional_mismatches = []
    for item in check.GetTracks():
        net = str(item.GetNetname()).removeprefix('/')
        expected = original_rules[net]
        if isinstance(item,pcbnew.PCB_VIA):
            actual = (item.GetWidth(pcbnew.F_Cu),item.GetDrillValue())
            wanted = (round(expected['via_size_mm']*1e6),round(expected['via_drill_mm']*1e6))
        else:
            actual = item.GetWidth()
            wanted = round(expected['trace_width_mm']*1e6)
        if actual != wanted:dimensional_mismatches.append({'net':net,'actual':actual,'expected':wanted})
    report['dimensional_mismatches'] = dimensional_mismatches
    verification = command([exe,'verify-kicad-rung',destination,name],out/'result-verify.log')
    assert verification['exit_code'] in (0,1)
    report['native'] = read(destination/'verification.json')
    assert command([exe,'inspect-kicad-board',final_path],out/'result-statistics.json')['exit_code']==0
    stats = read(out/'result-statistics.json')
    report['fixed_placement_matches'] = (source_stats['component_placement_100nm_sha256']==stats['component_placement_100nm_sha256'])
    report['source_unchanged'] = digest(board_path)==report['source_sha256']
    report['statistics'] = stats
    report['admitted'] = (report['native']['complete'] and report['fixed_placement_matches']
                           and not dimensional_mismatches and report['source_unchanged'])
    report['status'] = 'finished'
    write(out/'report.json',report)
    our_stats = recorded['final_statistics']
    our_verification = read(Path(recorded['result_directory'])/'verification.json')
    rows = []
    for label,stat,native in [('pcb-maker',our_stats,our_verification),('Freerouting',stats,report['native'])]:
        rows.append(f'<tr><td>{label}</td><td>{native["selected_net_unconnected_items"]}</td><td>{native["drc_design_violations"]}</td>'
                    f'<td>{stat["physical_copper"]["physical_centerline_length_mm"]:.3f}</td><td>{stat["vias"]}</td></tr>')
    (out/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>PIC source-class comparison</title>'
        '<style>body{font:18px system-ui;margin:25px}td,th{padding:10px;border:1px solid #bbb}table{border-collapse:collapse}img{max-width:95vw;max-height:85vh}</style>'
        '<h1>PIC source-class comparison</h1><p>Identical cold board and source class geometry. '
        'Exported net/pin inventory and class dimensions audited. Native import is checked independently.</p>'
        '<table><tr><th>Router</th><th>Native opens</th><th>Design findings</th><th>Track mm</th><th>Vias</th></tr>'+''.join(rows)+'</table>'
        '<h2>Freerouting result</h2><img src="result/preview.svg">'
        '<p><a href="source/preview.svg">Cold source</a> · <a href="report.json">Complete report</a> · '
        '<a href="dsn-audit.json">DSN rule audit</a></p>')
    print(json.dumps({'admitted':report['admitted'],'native':report['native'],
                      'track_mm':stats['physical_copper']['physical_centerline_length_mm'],
                      'vias':stats['vias'],'seconds':report['router_seconds']}),flush=True)


if __name__=='__main__':main()
