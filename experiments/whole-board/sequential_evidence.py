# Copyright (C) 2026 Toit contributors.
"""Read ordinary and repaired step receipts without confusing success modes."""
import json
import hashlib
import itertools
import math
from pathlib import Path


def selected_admission(step):
    selected = [a['native_admission'] for a in step['attempts'] if a['selected']]
    repair = step.get('repair')
    if repair and repair['selected']:
        selected.append(repair['native_admission'])
    assert len(selected) <= 1, 'a step cannot commit both an ordinary route and a repair'
    return selected[0] if selected else None


def selected_rules(step, portfolio):
    ordinary = next((a for a in step['attempts'] if a['selected']), None)
    repair = step.get('repair')
    if repair and repair['selected'] and repair.get('changed_routes'):
        assert not ordinary
        return receipt_rules(repair['changed_routes'])
    if ordinary:
        assert not (repair and repair['selected'])
        routing = portfolio[ordinary['config_index']]
        nets = [step['connection']]
    elif repair and repair['selected']:
        routing = portfolio[repair['routing_config_index']]
        nets = [step['connection'],repair['yielded_connection']]
    else:
        return {}
    rules = routing.get('connection_rules',{})
    return {net.removeprefix('/'): rules[net.removeprefix('/')] if rules else routing for net in nets}


def recorded_rules(step):
    """Read historical dimensions when a resumed run changes its portfolio."""
    ordinary = next((a for a in step['attempts'] if a['selected']),None)
    repair = step.get('repair')
    if repair and repair['selected'] and repair.get('changed_routes'):
        assert not ordinary
        return receipt_rules(repair['changed_routes'])
    if ordinary:
        routing = json.loads(Path(ordinary['candidate']).read_text())['config']
        nets = [step['connection']]
    elif repair and repair['selected']:
        routing = json.loads((Path(step['artifact_directory'])/'ripup-config.json').read_text())['diagnosis']['routing']
        nets = [step['connection'],repair['yielded_connection']]
    else:
        return {}
    rules=routing.get('connection_rules',{})
    return {net.removeprefix('/'): rules[net.removeprefix('/')] if rules else routing for net in nets}


def receipt_rules(receipts):
    result = {}
    for net, receipt in receipts.items():
        path = Path(receipt['candidate'])
        assert hashlib.sha256(path.read_bytes()).hexdigest() == receipt['candidate_sha256']
        candidate = json.loads(path.read_text())
        assert candidate['connection'].removeprefix('/') == net.removeprefix('/')
        config = candidate['config']
        rules = config.get('connection_rules',{})
        result[net.removeprefix('/')] = rules[net.removeprefix('/')] if rules else config
    return result


def audit_search_attempts(report, config):
    """Check schema-7 conditional decisions independently of native admission."""
    if report['schema_version'] < 7:
        return {'supported': False, 'reason': 'legacy journal has no typed failure contract'}
    initial = (report.get('resume') or {}).get('initial_completed_connections', 0)
    portfolio = config['routing_portfolio']
    fallbacks = {int(k): v for k, v in config.get('obstacle_distance_fallbacks', {}).items()}
    for index, source in fallbacks.items():
        assert 0 <= source < index < len(portfolio)
        a, b = dict(portfolio[source]), dict(portfolio[index])
        assert a.pop('heuristic', 'geometric') == 'geometric'
        assert b.pop('heuristic', 'geometric') == 'obstacle_distances'
        assert a == b, 'conditional guidance changes more than the heuristic'
    rows = []
    for step in report['steps'][initial:]:
        attempts = step['attempts']
        assert [a['config_index'] for a in attempts] == list(range(len(attempts)))
        assert len(attempts) <= len(portfolio)
        selected = [i for i, a in enumerate(attempts) if a['selected']]
        if config.get('portfolio_policy', 'best_score') == 'ordered_first_admitted' and selected:
            assert selected == [len(attempts)-1]
            assert not any((a.get('native_admission') or {}).get('complete') for a in attempts[:-1])
        else:
            assert len(attempts) == len(portfolio)
        for index, attempt in enumerate(attempts):
            failure = attempt.get('route_failure')
            skipped = attempt.get('skip_reason')
            if index in fallbacks:
                source_failure = attempts[fallbacks[index]].get('route_failure') or {}
                activated = source_failure.get('kind') == 'search_budget_exhausted'
                assert bool(skipped) == (not activated), 'conditional fallback ignored its typed trigger'
                rows.append({'ordinal': step['ordinal'], 'connection': step['connection'],
                             'config_index': index, 'activated': activated})
            else:
                assert not skipped
            if skipped:
                assert attempt['expansions'] == 0 and not attempt['selected']
                assert all(attempt[k] is None for k in ['candidate','error','native_admission'])
                assert not failure
            if failure:
                assert failure['kind'] in ['search_budget_exhausted', 'grid_disconnected']
                assert failure['connection'] == step['connection']
                assert attempt['expansions'] == failure['astar_expansions']
                assert not attempt['candidate'] and not attempt['selected'] and attempt['error']
            if attempt['candidate']:
                candidate = json.loads(Path(attempt['candidate']).read_text())
                assert candidate['config'].get('heuristic','geometric') == portfolio[index].get('heuristic','geometric')
                assert candidate['expansions'] == attempt['expansions']
    return {'supported': True, 'new_steps_checked': len(report['steps'])-initial,
            'conditional_decisions': rows,
            'activated': sum(r['activated'] for r in rows),
            'skipped': sum(not r['activated'] for r in rows),
            'scope': 'New schema-7 steps only; inherited failure counts are not reconstructed.'}


def audit_yielding_priority(diagnosis, config):
    """Hints may reorder the search, but may not remove foreign-net options."""
    if diagnosis['schema_version'] < 3:
        return False
    foreign = diagnosis['foreign_routed_connections']
    remaining = {name.removeprefix('/'): name for name in foreign}
    expected = []
    hints = []
    cut = diagnosis.get('routing_cut')
    if cut:
        assert config.get('prioritize_boundary_blockers', False)
        assert not diagnosis.get('routing_cut_error')
        assert cut['branch'] == diagnosis['base_route_failure']['branch']
        scores = {}
        for blocker in cut['blockers']:
            if blocker['kind'] == 'foreign_net_copper' and blocker['net'] is not None:
                name = blocker['net']
                scores[name] = scores.get(name, 0) + blocker['planar_hits'] + blocker['via_hits']
        hints = sorted(scores, key=lambda name: (-scores[name], name))
    elif config.get('prioritize_boundary_blockers', False) and not diagnosis['base_route_found']:
        assert diagnosis.get('routing_cut_error'), 'missing boundary inspection outcome'
    for name in hints + config.get('preferred_yielding_connections', []):
        original = remaining.pop(name.removeprefix('/'), None)
        if original is not None:
            expected.append(original)
    expected.extend(remaining[key] for key in sorted(remaining))
    assert diagnosis['yielding_connection_order'] == expected
    assert sorted(expected) == sorted(foreign), 'priority lost a foreign net'
    minimum = config.get('minimum_yielding_connections', 1)
    maximum = config['maximum_yielding_connections']
    combinations = itertools.chain.from_iterable(
        itertools.combinations(expected,size) for size in range(minimum,maximum+1))
    prefix = [list(c) for c in itertools.islice(combinations,config['maximum_trials'])]
    assert [t['yielding_connections'] for t in diagnosis['trials']] == prefix
    total = sum(math.comb(len(expected),size) for size in range(minimum,maximum+1))
    assert diagnosis['truncated'] == (total>config['maximum_trials'])
    return True


def audit_ripup_selection(report, config):
    """Account for every eligible restoration candidate, including skipped ones."""
    policy = config.get('selection_policy', 'best_score')
    assert policy in ['best_score', 'ordered_first_admitted']
    assert report.get('selection_policy', 'best_score') == policy
    protected = {net.removeprefix('/') for net in report.get('protected_connections', [])}
    eligible = [trial for trial in report['diagnosis']['trials']
        if trial['route_found'] and trial.get('candidate') is not None
        and trial['yielding_connections']
        and trial['yielding_connections'][0].removeprefix('/') not in protected]
    attempts = report['attempts']
    assert [a['ordinal'] for a in attempts] == list(range(len(attempts)))
    assert [a['yielding_connection'].removeprefix('/') for a in attempts] == [
        t['yielding_connections'][0].removeprefix('/') for t in eligible[:len(attempts)]]
    admitted = [a['ordinal'] for a in attempts if a['status'] in ['complete', 'progress']]
    if policy == 'ordered_first_admitted' and admitted:
        assert len(attempts) == admitted[0] + 1, 'continued after first admission'
        assert report['selected_attempt'] == admitted[0]
        skipped = [t['ordinal'] for t in eligible[len(attempts):]]
    else:
        assert len(attempts) == len(eligible), 'stopped without an admitted candidate'
        skipped = []
    assert report.get('skipped_diagnosis_trials', []) == skipped, 'unaccounted restoration candidates'
    return dict(policy=policy, attempted=len(attempts), skipped_diagnosis_trials=skipped)
