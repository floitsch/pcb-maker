# Copyright (C) 2026 Toit contributors.
"""Run pinned Freerouting on a cold native project or a sequence's cold source.

Native export, import and readback run in isolated processes. Basic DSN class
geometry and pin inventory are audited; arbitrary exchange-shape equivalence
is not claimed. Every materialized source/result receives native verification
and the usual combined rendering.
"""
import argparse
import html
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

from run import command, digest, read, write

JAR_SHA256 = 'f5ed374182900ccc78e473518bbb9f6b869f4a07159495f663a76f52bb10523b'


def router_deadline_reached(log):
    markers = re.findall(r'^ROUTER_DEADLINE_REACHED=(true|false)$', log, re.MULTILINE)
    if len(markers) != 1:
        raise ValueError('Expected one explicit router deadline result')
    return markers[0] == 'true'


def native(operation, board_path, target):
    # Keep each mutated SWIG board in its own process.
    from compile_net_classes import compile_rules
    import pcbnew
    if operation == 'rules':
        rules, evidence = compile_rules(board_path)
        write(target,dict(rules=rules,evidence=evidence))
        return
    board = pcbnew.LoadBoard(str(board_path))
    if operation == 'export':
        assert pcbnew.ExportSpecctraDSN(board,str(target))
    elif operation == 'import':
        # This is a fixed-placement comparison. KiCad's SES importer rewrites
        # poses at the session resolution, even when the router did not move
        # them. Restore only translations within half of the pinned 100 nm
        # exchange grid; reject rotations, side changes and actual moves.
        resolutions = re.findall(r'\(resolution\s+(\w+)\s+(\d+)\)',target.read_text())
        assert resolutions and set(resolutions) == {('um','10')}, 'Unsupported SES resolution'
        before = {f.m_Uuid.AsString():(f.GetReference(),f.GetPosition().x,f.GetPosition().y,
                  f.GetOrientationDegrees(),f.GetLayer()) for f in board.GetFootprints()}
        assert pcbnew.ImportSpecctraSES(board,str(target))
        footprints = {f.m_Uuid.AsString():f for f in board.GetFootprints()}
        assert before.keys() == footprints.keys(), 'SES changed component inventory'
        restored = []
        for uuid, (reference,x,y,angle,layer) in before.items():
            f = footprints[uuid]
            assert (f.GetReference(),f.GetOrientationDegrees(),f.GetLayer()) == (reference,angle,layer), 'SES changed component orientation/side'
            dx,dy = f.GetPosition().x-x,f.GetPosition().y-y
            assert max(abs(dx),abs(dy)) <= 50, f'SES moved {reference} beyond exchange rounding'
            if dx or dy:
                restored.append(dict(reference=reference,uuid=uuid,import_translation_nm=[dx,dy]))
                f.SetPosition(pcbnew.VECTOR2I(x,y))
        pcbnew.SaveBoard(str(board_path),board)
        write(board_path.parent/'import-pose-restoration.json',dict(session_sha256=digest(target),
              resolution_nm=100,maximum_allowed_translation_nm=50,restored=restored))
    else:
        raise ValueError(operation)


def compare(args):
    outside_outline = getattr(args, 'outside_outline', False)
    assert not outside_outline or args.translate_edge_rule, 'Outside outline requires edge-rule translation'
    from audit_partial_ripup import isolated_snapshot
    from report_adaptive_routing import finding_counts, progress_admissible
    from render_inspection_layers import render
    root = args.output.resolve()
    root.mkdir(exist_ok=False)
    sequence = read(args.sequence/'sequential-route.json') if args.sequence else None
    if sequence:
        assert sequence['termination'] in ['complete','routing_failed','bound_reached']
        name = sequence['board_id']
        source_original = Path(sequence['source_directory']).resolve()
    else:
        name = args.board_id
        source_original = args.source.resolve()
    source_hash = digest(source_original/(name+'.kicad_pcb'))
    source = root/'source'
    shutil.copytree(source_original,source)
    executable = root/'pcb-maker'
    shutil.copy2(args.binary,executable)
    shutil.copy2(__file__,root/Path(__file__).name)
    jar = args.jar.resolve()
    assert digest(jar) == JAR_SHA256
    board_name = name+'.kicad_pcb'
    board = source/board_name
    expected_hash = source_hash
    assert digest(board) == expected_hash
    if sequence:
        assert expected_hash == sequence['source_board_sha256_before'] == sequence['source_board_sha256_after']
    report = dict(status='preflight',sequence=str(args.sequence.resolve()) if args.sequence else None,
                  source_kind='sequence' if sequence else 'native_directory',
                  source_original=str(source_original),source_sha256=expected_hash,
                  source_project_sha256=digest(board.with_suffix('.kicad_pro')),
                  executable_sha256=digest(executable),jar_sha256=digest(jar),
                  maximum_seconds=args.seconds,maximum_passes=args.passes,maximum_threads=1,
                  outside_outline=outside_outline,
                  processes=[])
    def call(label, argv):
        result = command(argv,root/(label+'.log'))
        report['processes'].append(dict(stage=label,**result))
        write(root/'report.json',report)
        return result['exit_code']
    assert call('source-verify',[executable,'verify-kicad-rung',source,name]) in [0,1]
    baseline = read(source/'verification.json')
    baseline_drc = read(source/'drc.json')
    if sequence:
        assert baseline['selected_net_unconnected_items'] == sequence['expected_source_unconnected_items']
    import report_adaptive_routing
    report_adaptive_routing.BASELINE_NATIVE = {
        'erc_violations': baseline['erc_violations'],
        'schematic_parity_issues': baseline['schematic_parity_issues']}
    assert progress_admissible(baseline,baseline_drc,baseline_drc,True)
    assert call('source-stats',[executable,'inspect-kicad-board',board]) == 0
    stats = read(root/'source-stats.log')
    assert all(stats[k] == 0 for k in ['segments','arcs','vias','copper_zones','copper_graphics'])
    report['source_verification'] = baseline
    report['source_statistics'] = stats
    helper = Path(__file__).resolve()
    assert call('source-rules',[sys.executable,helper,'--native','rules',board,root/'source-rules.json']) == 0
    dsn = root/'input.dsn'
    assert call('export',[sys.executable,helper,'--native','export',board,dsn]) == 0
    if args.all_copper_signal_layers:
        from adapt_dsn_layers import all_signal
        shutil.copy2(dsn,root/'input-source.dsn')
        adapted=root/'input-all-signal.dsn'
        report['layer_adaptation']=all_signal(dsn,adapted)
        write(root/'layer-adaptation.json',report['layer_adaptation'])
        shutil.copy2(adapted,dsn)
    if args.translate_edge_rule:
        assert call('edge-translation',[sys.executable,helper.parent/'translate_dsn_edge.py',dsn,
            source,name,root/'edge-translation','--jar',jar,'--java-bin',args.java.parent]
                    + (['--outside-outline'] if outside_outline else [])) == 0
        report['edge_translation']=read(root/'edge-translation/report.json')
        shutil.copy2(dsn,root/'input-untranslated.dsn')
        shutil.copy2(root/'edge-translation/translated.dsn',dsn)
    assert call('class-translation',[sys.executable,helper.parent/'translate_dsn_classes.py',dsn,
        root/'source-rules.json',root/'class-translation.json']) == 0
    report['class_translation']=read(root/'class-translation.json')
    assert call('dsn-audit',[sys.executable,helper.parent/'audit_dsn_classes.py',board,dsn,root/'dsn-audit.json']) == 0
    report['dsn_audit'] = read(root/'dsn-audit.json')
    assert call('exchange-geometry',[sys.executable,helper.parent/'inspect_dsn_geometry.py',dsn,
        source,name,root/'exchange-geometry','--jar',jar,'--java-bin',args.java.parent]
                + (['--outside-outline'] if outside_outline else [])) == 0
    report['exchange_geometry'] = read(root/'exchange-geometry/report.json')
    report['routing_layer_policy']='all_copper_signal' if args.all_copper_signal_layers else 'preserve_source'
    report['external_signal_layer_count']=report['exchange_geometry']['signal_layer_count']
    if args.all_copper_signal_layers:
        layers=report['exchange_geometry']['routing_layers']
        assert len(layers)==2 and all(l['is_signal'] and l.get('router_active',True) for l in layers), 'External router did not activate both signal layers'
    # Absence of findings is not evidence of full rule/shape equivalence.
    report['equal_rule_comparison'] = False if report['exchange_geometry']['edge_requirement_mismatch_observed'] else None
    rules = read(root/'source-rules.json')['rules']
    if sequence:
        config = read(args.sequence/'config.json')
        assert all(profile['connection_rules'] == rules for profile in config['routing_portfolio'])
    report['pcb_maker_class_rules_match'] = True if sequence else None
    assert digest(board)==expected_hash and digest(board.with_suffix('.kicad_pro'))==report['source_project_sha256']
    session = root/'output.ses'
    argv = [str(args.java),'-Djava.awt.headless=true','--enable-final-field-mutation=ALL-UNNAMED',
            '-jar',str(jar),'-de',str(dsn),'-do',str(session),'-mp',str(args.passes),'-mt','1']
    if outside_outline:
        runner = root/'outside-runner'
        runner.mkdir()
        sources = []
        for filename in ['NativeOutlineMode.java', 'RouteOutsideDsn.java']:
            copied = runner/filename
            shutil.copy2(helper.parent/filename, copied)
            sources.append(copied)
        assert call('outside-runner-compile', [args.java.parent/'javac', '-cp', jar,
                    '-d', runner, *sources]) == 0
        report['outside_runner_source_sha256'] = {p.name:digest(p) for p in sources}
        argv = [str(args.java), '-Djava.awt.headless=true',
                '--enable-final-field-mutation=ALL-UNNAMED', '-cp', f'{jar}:{runner}',
                'RouteOutsideDsn', str(dsn), str(session), str(args.passes), 'true',
                str(root/'external-router-settings.json'), str(args.seconds)]
    report.update(status='routing',router_command=argv)
    write(root/'report.json',report)
    if args.replay_session_from:
        from audit_dsn_classes import parse_dsn
        previous_root = args.replay_session_from.resolve()
        previous = read(previous_root/'report.json')
        assert previous['router_exit_code']==0
        assert previous.get('outside_outline',False)==outside_outline, 'Session replay outline mode differs'
        if outside_outline:
            assert previous.get('outside_runner_source_sha256')==report['outside_runner_source_sha256'], 'Session replay outline runner differs'
        for key in ['source_sha256','source_project_sha256','jar_sha256']:
            assert previous[key]==report[key], f'Session replay input differs: {key}'
        old_tree,new_tree = parse_dsn((previous_root/'input.dsn').read_text()),parse_dsn(dsn.read_text())
        # The export's root name includes the output directory. All actual
        # placement, rules, pad geometry and wiring must match exactly.
        old_tree[1]=new_tree[1]=None
        assert old_tree==new_tree, 'Session replay DSN differs'
        assert digest(previous_root/'output.ses')==previous['session_sha256'], 'Session replay hash differs'
        shutil.copy2(previous_root/'output.ses',session)
        report.update(router_exit_code=0,router_seconds=previous['router_seconds'],
            router_execution_reused=True,replay_source=str(previous_root),
            replay_report_sha256=digest(previous_root/'report.json'),
            router_command=previous['router_command'],
            router_deadline_reached=previous.get('router_deadline_reached',False))
        for key in ['maximum_seconds','maximum_passes','maximum_threads']:
            report[key]=previous[key]
    else:
        started = time.monotonic()
        with (root/'freerouting.log').open('w') as log:
            try:
                # The outside runner requests a cooperative stop at the search
                # limit and exports only after joining its worker. Leave time
                # for that bounded shutdown; the frontend pipeline deadline is
                # still independent and can terminate the entire process group.
                watchdog_seconds = args.seconds + 35 if outside_outline else args.seconds
                result = subprocess.run(argv,stdout=log,stderr=subprocess.STDOUT,timeout=watchdog_seconds)
                report['router_exit_code'] = result.returncode
            except subprocess.TimeoutExpired:
                report.update(status='timed_out',router_seconds=time.monotonic()-started)
                write(root/'report.json',report)
                raise
        report['router_seconds'] = time.monotonic()-started
        report['router_deadline_reached'] = (router_deadline_reached((root/'freerouting.log').read_text())
            if outside_outline and report['router_exit_code'] == 0 else False)
    write(root/'report.json',report)
    assert report['router_exit_code']==0 and session.is_file()
    report.update(status='importing',session_sha256=digest(session))
    write(root/'report.json',report)
    destination = root/'result'
    shutil.copytree(source,destination)
    final = destination/board_name
    project = final.with_suffix('.kicad_pro')
    original_project = project.read_bytes()
    try:
        imported = call('import',[sys.executable,helper,'--native','import',final,session]) == 0
    finally:
        if project.exists(): shutil.copy2(project,root/'project-after-import.kicad_pro')
        project.write_bytes(original_project)
    assert call('result-verify',[executable,'verify-kicad-rung',destination,name]) in [0,1]
    assert call('result-rules',[sys.executable,helper,'--native','rules',final,root/'result-rules.json']) == 0
    assert read(root/'source-rules.json')['rules']==read(root/'result-rules.json')['rules']
    assert read(root/'source-rules.json')['evidence']['assignments']==read(root/'result-rules.json')['evidence']['assignments']
    before, after = isolated_snapshot(board), isolated_snapshot(final)
    assert before['pose']==after['pose']
    mismatches=[]
    for track in after['tracks'].values():
        rule=rules[track['net']]
        if track['width'] != round(rule['via_size_mm' if 'drill' in track else 'trace_width_mm']*1e6):
            mismatches.append(track)
        elif 'drill' in track and (track['drill']!=round(rule['via_drill_mm']*1e6) or track['width']!=track['back_width']):
            mismatches.append(track)
    native_result = read(destination/'verification.json')
    drc = read(destination/'drc.json')
    assert digest(project)==report['source_project_sha256']
    assert digest(source_original/board_name)==digest(board)==expected_hash
    assert call('result-stats',[executable,'inspect-kicad-board',final]) == 0
    report.update(status='finished',native=native_result,statistics=read(root/'result-stats.log'),
                  imported=imported,dimensional_mismatches=mismatches,source_unchanged=True,
                  pose_restoration=read(destination/'import-pose-restoration.json'),
                  fixed_placement_matches=True,project_and_classes_unchanged=True,
                  new_unconnected_findings=len(finding_counts(drc['unconnected_items'])-finding_counts(baseline_drc['unconnected_items'])))
    report['routing_complete']=(imported and not mismatches and native_result['selected_net_unconnected_items']==0
                                and progress_admissible(native_result,drc,baseline_drc,True))
    report['complete']=report['routing_complete'] and native_result['complete']
    render(source,name)
    render(destination,name)
    write(root/'report.json',report)
    title=html.escape(name)+' router comparison'
    (root/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>'+title+'</title>'
        '<style>body{font:18px system-ui;margin:2rem}img{max-width:95vw;max-height:80vh}</style><h1>'+title+'</h1>'
        f'<p>Freerouting: {native_result["selected_net_unconnected_items"]} native opens; {native_result["drc_design_violations"]} design findings. '
        f'Routing complete under unchanged source findings: {report["routing_complete"]}. Full native completion: {report["complete"]}.</p>'
        f'<p>External signal routing layers: {report["external_signal_layer_count"]}; layer policy: {report["routing_layer_policy"]}.</p>'
        f'<p>Exact source placement restored after bounded session rounding; basic source-class geometry matches. Equal-rule comparison: {report["equal_rule_comparison"]} (null means unproven). '
        'Exchange audit covers net/pin inventory and class dimensions; arbitrary footprint-shape equivalence is not claimed. Java search time excludes native verification and is not comparable to our verification-inclusive runtime.</p>'
        '<img src="result/inspection-combined.svg"><p><a href="source/inspection-combined.svg">Cold source</a> · '
        '<a href="report.json">Evidence</a> · <a href="dsn-audit.json">Exchange audit</a> · '
        '<a href="exchange-geometry/index.html">Exchange geometry findings</a></p>')
    print(json.dumps({k:report[k] for k in ['routing_complete','complete','native','router_seconds','dimensional_mismatches']}),flush=True)


def run_recorded(args):
    if args.output.exists():
        raise FileExistsError(args.output)
    try:
        compare(args)
    except Exception as error:
        path = args.output/'report.json'
        if path.exists():
            report = read(path)
            previous = report['status']
            report.update(status='timed_out' if previous=='timed_out' else 'failed',
                          failed_stage=previous,error=f'{type(error).__name__}: {error}')
            write(path,report)
        raise


if __name__=='__main__':
    if len(sys.argv)>1 and sys.argv[1]=='--native':
        native(sys.argv[2],Path(sys.argv[3]),Path(sys.argv[4]))
    else:
        parser=argparse.ArgumentParser(description=__doc__)
        sources=parser.add_mutually_exclusive_group(required=True)
        sources.add_argument('--sequence',type=Path)
        sources.add_argument('--source',type=Path,help='Cold native project directory; requires --board-id')
        parser.add_argument('--board-id')
        for name in ['binary','output']:
            parser.add_argument('--'+name,type=Path,required=True)
        parser.add_argument('--jar',type=Path,default=Path('/tmp/freerouting-2.2.4.jar'))
        parser.add_argument('--java',type=Path,default=Path('/usr/lib/jvm/java-26-openjdk/bin/java'))
        parser.add_argument('--seconds',type=int,default=600)
        parser.add_argument('--passes',type=int,default=100)
        parser.add_argument('--translate-edge-rule',action='store_true',
                            help='Audit and translate the native global copper-edge rule before routing')
        parser.add_argument('--outside-outline',action='store_true',
                            help='Use the existing Freerouting outside-area boundary mode; requires --translate-edge-rule')
        parser.add_argument('--all-copper-signal-layers',action='store_true',
                            help='Explicit capability adaptation: enable both copper layers for signal routing')
        parser.add_argument('--replay-session-from',type=Path,
                            help='Re-import a hashed successful session after exact source and DSN checks; no new router run')
        args=parser.parse_args()
        if args.source and not args.board_id:
            parser.error('--source requires --board-id')
        assert args.seconds>0 and args.passes>0
        run_recorded(args)
