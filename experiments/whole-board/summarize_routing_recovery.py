# Copyright (C) 2026 Toit contributors.
"""Compact, source-linked evidence for the last step in a routing journal.

This reads journals and creates missing copper views; it neither reroutes nor
independently admits a board.
Call after the command is terminal. Combined native copper renders are linked.
"""
import argparse
import html
import hashlib
import json
import os
from pathlib import Path


def read(path):
    return json.loads(path.read_text())


def repair_tree(directory, seen):
    directory = Path(directory).resolve()
    if directory in seen:
        return {'directory': str(directory), 'evidence_missing': 'repeated repair reference'}
    seen.add(directory)
    path = directory/'ripup.json'
    if not path.exists():
        return {'directory': str(directory), 'evidence_missing': 'repair journal is absent'}
    report = read(path)
    diagnosis = report['diagnosis']
    tested = diagnosis['trials']
    node = {
        'directory': str(directory), 'journal': str(path),
        'target': report['target_connection'],
        'selected_attempt': report['selected_attempt'],
        'expected_open_items': report['expected_unconnected_items'],
        'protected_connections': report.get('protected_connections', []),
        'diagnosis': {
            'tested': len(tested), 'available_foreign_nets': len(diagnosis['foreign_routed_connections']),
            'truncated': diagnosis['truncated'],
            'base_route_failure': diagnosis.get('base_route_failure'),
            'boundary_inspection': ({k: diagnosis['routing_cut'][k] for k in [
                'connected', 'start_reachable_states', 'finish_reachable_states',
                'inspected_endpoint', 'boundary_sampling_truncated']}
                if diagnosis.get('routing_cut') else None),
            'boundary_inspection_error': diagnosis.get('routing_cut_error'),
            'yielding_connection_order': diagnosis.get('yielding_connection_order'),
            'route_found_with': [t['yielding_connections'] for t in tested if t['route_found']],
            'unsuccessful_trials': [{k: t.get(k) for k in ['yielding_connections', 'route_expansions', 'route_failure', 'error']} for t in tested if not t['route_found']],
        },
        'attempts': [],
    }
    for attempt in report['attempts']:
        row = {k: attempt.get(k) for k in [
            'ordinal', 'yielding_connection', 'status', 'selected', 'error',
            'restoration_skip_reason', 'provisional_directory', 'artifact_directory']}
        row['changed_nets'] = sorted(attempt.get('changed_routes', {}))
        row['final_native'] = attempt.get('final_verification')
        restoration = attempt.get('restoration')
        if restoration:
            row['restoration_trigger'] = restoration.get('trigger_error')
            row['restoration_error'] = restoration.get('error')
            row['restoration'] = repair_tree(restoration['directory'], seen)
        node['attempts'].append(row)
    return node


def summarize(root):
    root = Path(root).resolve()
    report = read(root/'sequential-route.json')
    step = report['steps'][-1] if report['steps'] else None
    result = {
        'schema_version': 1,
        'board_id': report['board_id'],
        'scope': 'Journal summary only; process liveness is not checked. Use linked native audits for admission. An unsuccessful bounded search does not prove physical infeasibility.',
        'journal': str(root/'sequential-route.json'),
        'termination': report['termination'],
        'completed_connections': report['completed_connections'],
        'total_connections': len(report['connection_order']),
        'retained_native': read(Path(report['result_directory'])/'verification.json'),
        'retained_render': str(Path(report['result_directory'])/'inspection-combined.svg'),
        'last_step': None,
    }
    if step:
        row = {k: step.get(k) for k in ['ordinal', 'connection', 'selected_config_index', 'repair_skip_reason']}
        row['ordinary_attempts'] = [{k: a.get(k) for k in [
            'config_index', 'selected', 'expansions', 'route_failure', 'skip_reason', 'error']}
            for a in step['attempts']]
        repair = step.get('repair')
        if repair:
            row['repair_committed'] = repair['selected']
            row['repair_error'] = repair.get('error')
            row['repair'] = repair_tree(repair['directory'], set())
            if repair.get('compound'):
                evidence = dict(repair['compound'])
                path = Path(evidence['directory'])/'compound.json'
                if path.exists():
                    compound = read(path)
                    evidence.update(journal=str(path),target=compound['target_connection'],
                        diagnosis_trials=len(compound['diagnosis']['trials']),
                        source_snapshot=compound['source_snapshot'],attempts=[])
                    for attempt in compound['attempts']:
                        item = {k:attempt[k] for k in ['ordinal','pair_trial','order','selected','error',
                            'provisional_directory','stages','final_directory','final_verification']}
                        item['changed_nets'] = sorted(attempt['changed_routes'])
                        if attempt['restoration_directory']:
                            item['restoration'] = repair_tree(attempt['restoration_directory'],set())
                        evidence['attempts'].append(item)
                else:
                    evidence['evidence_missing'] = 'compound journal is absent'
                row['compound'] = evidence
        result['last_step'] = row
    return result


def write_summary(root):
    root = Path(root).resolve()
    result = summarize(root)
    def ensure_render(directory):
        directory = Path(directory).resolve()
        existing = directory/'inspection-combined.svg'
        if existing.exists():
            return str(existing)
        # Retained provisional boards can be the immutable source of a child
        # repair. Generate missing views in a separate, content-keyed cache.
        names = ['preview.svg', 'preview-labels.json', result['board_id']+'.kicad_pcb']
        project = result['board_id']+'.kicad_pro'
        if (directory/project).exists():
            names.append(project)
        blobs = {name:(directory/name).read_bytes() for name in names}
        digest = hashlib.sha256(str(directory).encode())
        for name, blob in blobs.items():
            digest.update(name.encode()); digest.update(blob)
        cache = root/'analysis-views'/digest.hexdigest()
        image = cache/'inspection-combined.svg'
        if not image.exists():
            cache.mkdir(parents=True, exist_ok=True)
            for name, blob in blobs.items():
                (cache/name).write_bytes(blob)
            from render_inspection_layers import render
            render(cache, result['board_id'])
            (cache/'source.json').write_text(json.dumps({'directory':str(directory),
                'files':{name:hashlib.sha256(blob).hexdigest() for name,blob in blobs.items()}},indent=2)+'\n')
        return str(image)
    def ensure_tree(node):
        for attempt in node.get('attempts', []):
            if attempt['provisional_directory']:
                attempt['provisional_render'] = ensure_render(attempt['provisional_directory'])
            if attempt['final_native']:
                attempt['final_render'] = ensure_render(attempt['artifact_directory'])
            if attempt.get('restoration'):
                ensure_tree(attempt['restoration'])
    result['retained_render'] = ensure_render(Path(result['retained_render']).parent)
    if result['last_step'] and result['last_step'].get('repair'):
        ensure_tree(result['last_step']['repair'])
    compound = (result['last_step'] or {}).get('compound')
    if compound:
        for attempt in compound.get('attempts',[]):
            attempt['provisional_render'] = ensure_render(attempt['provisional_directory'])
            for stage in attempt['stages']:
                stage['render'] = ensure_render(stage['directory'])
            if attempt.get('restoration'):
                ensure_tree(attempt['restoration'])
            if attempt['final_directory']:
                attempt['final_render'] = ensure_render(attempt['final_directory'])
    (root/'analysis.json').write_text(json.dumps(result, indent=2)+'\n')
    esc = lambda value: html.escape(str(value))
    def link(path, label):
        return '<a href="'+esc(os.path.relpath(path, root))+'">'+esc(label)+'</a>'
    def tree(node):
        if node.get('evidence_missing'):
            return '<p class="failed">'+esc(node['evidence_missing'])+'</p>'
        d = node['diagnosis']
        parts = ['<section><h3>Route '+esc(node['target'])+'</h3>',
                 '<p>'+link(node['journal'], 'Full evidence')+': '+str(d['tested'])+' trials; '+
                 str(len(d['route_found_with']))+' counterfactual routes; truncated='+str(d['truncated'])+'.</p>']
        if node['protected_connections']:
            parts.append('<p>Protected: '+esc(', '.join(node['protected_connections']))+'</p>')
        parts.append('<details><summary>Counterfactual coverage</summary><pre>'+esc(json.dumps(d,indent=2))+'</pre></details>')
        for a in node['attempts']:
            parts.append('<article class="'+('accepted' if a['selected'] else 'failed')+'"><strong>Yield '+
                         esc(a['yielding_connection'])+': '+esc(a['status'])+
                         (' (selected)' if a['selected'] else '')+'</strong>')
            for field in ['error','restoration_trigger','restoration_error','restoration_skip_reason']:
                if a.get(field): parts.append('<p>'+esc(field)+': '+esc(a[field])+'</p>')
            if a['provisional_directory']:
                parts.append('<p>'+link(a['provisional_render'], 'Provisional copper: yielded net not yet restored')+'</p>')
            if a['final_native']:
                parts.append('<p>'+link(a['final_render'], 'Final attempt copper')+
                             '; native opens='+str(a['final_native']['selected_net_unconnected_items'])+'</p>')
            if a.get('restoration'): parts.append(tree(a['restoration']))
            parts.append('</article>')
        parts.append('</section>')
        return ''.join(parts)
    page = ['<!doctype html><meta charset="utf-8"><title>Routing recovery analysis</title>',
            '<style>body{font:17px system-ui;margin:2rem;max-width:1300px}article,section{padding:.7rem;margin:.6rem 0;border-left:3px solid #8293a1}.accepted{border-color:#258c56}.failed{border-color:#c17b22}pre{white-space:pre-wrap;font-size:13px}img{max-width:100%}</style>',
            '<h1>Retained routing: '+str(result['completed_connections'])+'/'+str(result['total_connections'])+' nets</h1>',
            '<p>Native opens: '+str(result['retained_native']['selected_net_unconnected_items'])+
            '; full native completion: '+str(result['retained_native']['complete'])+'.</p>',
            '<p>'+esc({'bound_reached':'Configured connection limit reached.', 'routing_failed':'Routing stopped at an unresolved connection.', 'complete':'All scheduled connections processed.'}.get(result['termination'],result['termination']))+'</p>',
            '<p>'+esc(result['scope'])+'</p>', '<p>'+link(root/'analysis.json','Compact JSON')+' · '+link(result['journal'],'Full journal')+
            (' · '+link(root/'recovery-audit.json','Independent native audit') if (root/'recovery-audit.json').exists() else '')+'</p>',
            '<img src="'+esc(os.path.relpath(result['retained_render'],root))+'" alt="Retained front and back copper, component bodies hidden">']
    step = result['last_step']
    if step:
        page += ['<h2>Last attempted connection: '+esc(step['connection'])+'</h2>',
                 '<details><summary>Ordinary search evidence</summary><pre>'+esc(json.dumps(step['ordinary_attempts'],indent=2))+'</pre></details>']
        if step.get('repair_skip_reason'): page.append('<p>'+esc(step['repair_skip_reason'])+'</p>')
        if step.get('repair'): page.append(tree(step['repair']))
        if compound:
            page.append('<h2>Compound recovery</h2><p>'+esc(compound.get('error') or 'Bounded pair displacement followed by restoration.')+'</p>')
            if compound.get('journal'):
                page.append('<p>'+link(compound['journal'],'Full compound evidence')+'</p>')
            for a in compound.get('attempts',[]):
                page.append('<section><h3>Restoration order: '+esc(' → '.join(a['order']))+
                            (' (selected)' if a['selected'] else '')+'</h3>')
                page.append('<p>'+link(a['provisional_render'],'Target routed; pair displaced')+'</p>')
                for s in a['stages']:
                    page.append('<p>'+link(s['render'],'Restore '+s['connection'])+': '+
                                esc(s['error'] or 'direct route found')+'</p>')
                if a.get('restoration'): page.append(tree(a['restoration']))
                if a.get('final_render'):
                    page.append('<p>'+link(a['final_render'],'Admitted final copper')+'; changed nets: '+esc(', '.join(a['changed_nets']))+'</p>')
                page.append('</section>')
    (root/'analysis.html').write_text(''.join(page))
    return result


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root',type=Path)
    args=parser.parse_args()
    result=write_summary(args.root)
    print(json.dumps({'completed_connections':result['completed_connections'],
                      'last_connection':(result['last_step'] or {}).get('connection'),
                      'analysis':str(args.root/'analysis.html')}))
