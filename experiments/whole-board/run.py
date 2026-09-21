#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Whole-board upstream references and paired cold routing, with automatic views.

Every source remains in the report, including unsupported and untested cases.
Original project copies are verified without deleting reference copper. Existing
paired manifests provide independent native admission, timing and cold routing.
This runner does not yet generate placement/area candidates.
"""
import argparse
import hashlib
import html
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]


def read(path):
    return json.loads(path.read_text())


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def command(args, log):
    start = time.monotonic()
    with log.open('w') as stream:
        result = subprocess.run([str(x) for x in args], stdout=stream,
                                stderr=subprocess.STDOUT, cwd=ROOT)
    return {'exit_code': result.returncode,
            'seconds': time.monotonic() - start, 'log': str(log)}


def metrics(statistics):
    copper = statistics['physical_copper']
    return {'footprints': statistics['footprints'],
            'area_mm2': copper['board_outline_area_mm2'],
            'track_length_mm': copper['physical_centerline_length_mm'],
            'vias': statistics['vias'], 'copper_zones': statistics['copper_zones'],
            'filled_zone_area_mm2': copper['filled_zone_area_mm2'],
            'used_layers': copper['used_copper_layers'],
            'centerline_union_exact': copper['centerline_union_exact']}


def result_row(evidence, admitted):
    if not evidence:
        return {'admitted': False, 'status': 'missing result'}
    admission = evidence.get('route_admission') or {}
    return {'admitted': admitted,
            'metrics': metrics(evidence['statistics']),
            'native': evidence['verification'],
            'introduced_findings': {k: admission.get(k) for k in (
                'introduced_erc_violations', 'introduced_drc_design_violations',
                'introduced_schematic_parity_issues')},
            'render': evidence.get('render') or evidence.get('result_render')}


def publish(out, report):
    write(out/'report.json', report)
    def link(path, label):
        return '<a href="' + html.escape(os.path.relpath(path, out), quote=True) + '">' + html.escape(label) + '</a>'
    parts = ['<!doctype html><meta charset="utf-8"><title>Whole-board references</title>',
             '<style>body{font:16px system-ui;max-width:1250px;margin:30px auto;padding:15px}table{border-collapse:collapse;width:100%}td,th{border:1px solid #ccc;padding:8px;text-align:left}img{width:48%;vertical-align:top}small{color:#555}</style>',
             '<h1>Whole-board references</h1>',
             '<p>Complete netlists, cold copper, fixed reference placement. Area reduction is an explicit next track; no smaller-board result is claimed.</p>',
             '<p>Track length excludes connectivity inside pours. Human length ratios are descriptive, not an electrical-quality score. Native opens and absolute findings remain visible.</p>']
    parts += ['<p>Human admission means absolute native cleanliness. Router admission means zero opens and no new findings relative to the frozen source; existing findings remain listed in the evidence.</p>']
    for case in report['cases']:
        parts += ['<h2>' + html.escape(case['source']) + '</h2>',
                  '<p>' + html.escape(case['rule_scope']) + '</p>',
                  '<p>Status: ' + html.escape(case['status']) + '</p>']
        if case.get('error'):
            parts += ['<pre>' + html.escape(case['error']) + '</pre>']
        parts += ['<table><tr><th>Layout</th><th>Admitted</th><th>Native opens</th><th>Area mm²</th><th>Track mm</th><th>Vias</th><th>Zones</th></tr>']
        for name in ('human', 'pcb_maker', 'freerouting'):
            row = case.get(name)
            if not row:
                continue
            m = row.get('metrics', {})
            cells = [name, row.get('admitted', 'reference'),
                     row.get('native', {}).get('selected_net_unconnected_items', '?'),
                     m.get('area_mm2'), m.get('track_length_mm'),
                     m.get('vias'), m.get('copper_zones')]
            parts += ['<tr>' + ''.join('<td>' + html.escape(f'{v:.3f}' if isinstance(v, float) else str(v)) + '</td>' for v in cells) + '</tr>']
        parts += ['</table>']
        directory = out/case['source']
        svg = directory/'reference/preview.svg'
        if svg.exists():
            parts += ['<p>' + link(svg, 'Human reference rendering') + '</p>',
                      '<img src="' + html.escape(os.path.relpath(svg, out), quote=True) + '">']
        for name in ('pcb_maker', 'freerouting'):
            render = case.get(name, {}).get('render') or {}
            if render.get('front'):
                parts += ['<p>' + link(render['front'], name + ' front') + ' · ' + link(render['back'], 'back') + '</p>',
                          '<img src="' + html.escape(os.path.relpath(render['front'], out), quote=True) + '">',
                          '<img src="' + html.escape(os.path.relpath(render['back'], out), quote=True) + '">']
    parts += ['<p>' + link(out/'report.json', 'Full evidence and limitations') + '</p>']
    (out/'index.html').write_text('\n'.join(parts))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--manifest', type=Path, default=ROOT/'benchmarks/real/whole-board.json')
    parser.add_argument('--executable', type=Path, default=ROOT/'target/release/pcb-maker')
    parser.add_argument('--references-only', action='store_true')
    parser.add_argument('--case', action='append', help='Run only these source IDs; retain other cases as untested')
    parser.add_argument('--router-seconds', type=int, help='Explicitly override both competitors process budgets')
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    exe = out/'pcb-maker'
    shutil.copy2(args.executable, exe)
    manifest = read(args.manifest)
    assert manifest['schema_version'] == 1
    assert not args.case or set(args.case) <= {c['source'] for c in manifest['cases']}
    assert args.router_seconds is None or args.router_seconds > 0
    sources = {s['id']: s for s in read(ROOT/'benchmarks/real/sources.json')['sources']}
    report = {'schema_version': 1, 'suite': manifest,
              'manifest_sha256': digest(args.manifest),
              'sources_manifest_sha256': digest(ROOT/'benchmarks/real/sources.json'),
              'executable_sha256': digest(exe), 'cases': [],
              'router_seconds_override': args.router_seconds,
              'area_track_status': 'not implemented; no placement or area score',
              'scope': 'Current-program replay with manifest budgets or an explicit equal override. Cached native checks and raw timing are retained by the paired runner. All upstream reference copper is withheld from routing.'}
    for entry in manifest['cases']:
        case = dict(entry, status='checking source')
        report['cases'].append(case)
        directory = out/entry['source']
        directory.mkdir()
        if args.case and entry['source'] not in args.case:
            case['status'] = 'not selected; untested in this run'
            publish(out, report)
            continue
        publish(out, report)
        try:
            source = sources[entry['source']]
            case['provenance'] = {k: source[k] for k in ('upstream', 'commit', 'license')}
            fetched = command(['bash', ROOT/'benchmarks/real/fetch.sh', source['id']], directory/'fetch.log')
            case['fetch'] = fetched
            if fetched['exit_code']:
                raise RuntimeError('Source fetch/checksum verification failed')
            upstream = ROOT/'benchmarks/real/external'/source['id']
            board = upstream/(source['stem'] + '.kicad_pcb')
            case['source_board_sha256'] = digest(board)
            reference = directory/'reference'
            shutil.copytree(upstream, reference)
            inspected = command([exe, 'inspect-kicad-board', board], directory/'human-statistics.json')
            if inspected['exit_code']:
                raise RuntimeError('Reference metrics unsupported; see human-statistics.json')
            case['human'] = {'metrics': metrics(read(directory/'human-statistics.json'))}
            verified = command([exe, 'verify-kicad-rung', reference, source['stem']], directory/'reference-verification.log')
            case['reference_verification_process'] = verified
            if verified['exit_code'] not in (0, 1):
                raise RuntimeError('Reference verifier crashed; emitted files are not an admitted result')
            native_path = reference/'verification.json'
            if not native_path.exists() or not (reference/'preview.svg').exists():
                raise RuntimeError('Reference verification/render did not finish')
            case['human']['native'] = read(native_path)
            case['human']['admitted'] = case['human']['native']['complete']
            assert digest(board) == case['source_board_sha256']
            case['status'] = 'reference audited; paired run pending'
            publish(out, report)
            if not entry['comparison'] or args.references_only:
                case['status'] = 'reference audited; paired routing not run'
                continue
            comparison = (args.manifest.parent/entry['comparison']).resolve()
            if args.router_seconds:
                config = read(comparison)
                route_path = (comparison.parent/config['route_benchmark']).resolve()
                route = read(route_path)
                assert route['source']['kind'] == 'cold_kicad_project'
                route['source']['directory'] = str((route_path.parent/route['source']['directory']).resolve())
                route['journal_directory'] = str(out/'progress')
                route['freerouting']['maximum_seconds'] = args.router_seconds
                config['route_benchmark'] = 'route-config.json'
                for key in ('sequential_config', 'verification_cache_directory'):
                    config['pcb_maker'][key] = str((comparison.parent/config['pcb_maker'][key]).resolve())
                config['pcb_maker']['maximum_seconds'] = args.router_seconds
                write(directory/'route-config.json', route)
                write(directory/'comparison-config.json', config)
                comparison = directory/'comparison-config.json'
            run = directory/'comparison'
            case['comparison_process'] = command([exe, 'benchmark-route-compare', comparison, run], directory/'comparison.log')
            paired = read(run/'competitive-comparison.json')
            external = read(run/'competitive-result.json')
            case['comparison_report'] = str(run/'competitive-comparison.json')
            case['fairness'] = {k: paired[k] for k in ('status', 'fixed_placement_matches', 'rules_match')}
            case['pcb_maker'] = result_row(paired.get('pcb_maker'), paired['pcb_maker_solved'])
            case['freerouting'] = result_row(external.get('result'), paired['freerouting_solved'])
            process = paired.get('pcb_maker') or {}
            case['pcb_maker']['process'] = {k: process.get(k) for k in (
                'maximum_seconds', 'elapsed_micros', 'timed_out', 'termination',
                'final_rung', 'target_rung', 'total_route_expansions')}
            case['freerouting']['process'] = external.get('routing')
            case['status'] = 'paired run ' + paired['status']
            assert digest(board) == case['source_board_sha256']
        except Exception as error:
            case.update(status='infrastructure failure', error=str(error))
        finally:
            publish(out, report)
            print(case['source'] + ': ' + case['status'], flush=True)
    report['finished'] = True
    report['total_sources'] = len(report['cases'])
    report['paired_runs'] = sum('fairness' in c for c in report['cases'])
    report['native_source_rule_claim'] = 'No aggregate source-rule solve rate: adapted and unsupported cases remain explicitly visible.'
    publish(out, report)
    if any(c['status'] == 'infrastructure failure' for c in report['cases']):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
