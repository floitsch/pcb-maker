# Copyright (C) 2026 Toit contributors.
"""One fresh native sweep; optionally rotate failed tails using admitted resumes.

The caller supplies the total wall/CPU limit. Every net is attempted at most
once, and the top-level repair allowance is shared across internal invocations.
This experiment does not accept an external checkpoint or change native rules.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time


def read(path):
    return json.loads(path.read_text())


def write(path, value):
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, indent=2) + '\n')
    temporary.replace(path)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def committed(step):
    return step.get('selected_config_index') is not None or bool(
        (step.get('repair') or {}).get('selected'))


def native_source_admissible(native, drc_path, allow_annotations):
    if native['erc_violations'] or native['schematic_parity_issues']:
        return False
    if not native['drc_design_violations']:
        return True
    if not allow_annotations:
        return False
    # Reuse the existing narrow annotation exception; physical violations
    # remain fatal even when annotations are explicitly allowed.
    from report_adaptive_routing import progress_admissible
    baseline = read(drc_path)
    return progress_admissible(native, baseline, baseline, True)


def suffix_policy(base, order, completed, attempted, repairs_used):
    """Keep only admitted identities in the prefix and never retry a failed tail."""
    assert len(set(completed)) == len(completed)
    assert set(completed) <= attempted <= set(order)
    remaining = [net for net in order if net not in attempted]
    if not remaining:
        return None
    failed = [net for net in order if net in attempted and net not in completed]
    result = copy.deepcopy(base)
    result['connection_order'] = completed + remaining + failed
    result['maximum_connections'] = len(completed) + len(remaining)
    budget = (base.get('ripup') or {}).get('maximum_invocations', 0)
    assert 0 <= repairs_used <= budget, 'global repair budget exceeded'
    if budget == repairs_used:
        result['ripup'] = None
    else:
        result['ripup']['maximum_invocations'] = budget - repairs_used
    return result


def classified_failure(step):
    """Conservatively stop on adapter/execution/native errors, not just exit 1."""
    attempted_failure = False
    for attempt in step['attempts']:
        if attempt.get('skip_reason'):
            continue
        failure = attempt.get('route_failure') or {}
        if failure.get('kind') not in ['grid_disconnected', 'search_budget_exhausted']:
            return False
        attempted_failure = True
    # Repair failure receipts still mix typed route failures with untyped
    # adapter/native/execution errors. This prototype does not infer a safe
    # continuation from source_unchanged or an error-string substring. Earlier
    # admitted repairs still count against the global allowance normally.
    return attempted_failure and not step.get('repair')


def run(args):
    source, binary, output = args.source.resolve(), args.binary.resolve(), args.output.resolve()
    assert not output.exists(), 'output must be fresh'
    assert not output.is_relative_to(source), 'output must be outside immutable source'
    base = read(args.config)
    assert not base.get('ripup') or 'maximum_invocations' in base['ripup'], 'explicit repair allowance required'
    inputs = {p.name: digest(p) for p in source.iterdir()
              if p.suffix in ['.kicad_pcb', '.kicad_sch', '.kicad_pro']}
    output.mkdir(parents=True)
    write(output / 'base-config.json', base)
    state = dict(schema_version=1, source=str(source), board_id=args.board_id,
                 source_inputs_sha256=inputs, binary_sha256=digest(binary),
                 continue_after_failure=args.continue_after_failure,
                 maximum_top_level_repairs=(base.get('ripup') or {}).get('maximum_invocations', 0),
                 repair_invocations_used=0, invocations=[], inventory=[], complete=False,
                 termination='running', manual_checkpoint_input=False,
                 independent_audit='run after the timed arm, including active journal on interruption')
    preflight = output / 'source-preflight'
    shutil.copytree(source, preflight)
    for name in ['verification.json', 'drc.json', 'erc.json']:
        (preflight / name).unlink(missing_ok=True)
    command = [str(binary), 'verify-kicad-rung', str(preflight), args.board_id]
    state['preflight'] = dict(directory=str(preflight), command=command)
    write(output / 'sweep.json', state)
    started = time.monotonic()
    with (output / 'source-preflight.log').open('w') as log:
        checked = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
    state['preflight'].update(exit_code=checked.returncode,
                             elapsed_seconds=time.monotonic() - started)
    verification = preflight / 'verification.json'
    if checked.returncode not in [0, 1] or not verification.exists():
        state.update(termination='source_preflight_execution_error')
        write(output / 'sweep.json', state)
        return 2
    state['preflight']['native'] = read(verification)
    if not native_source_admissible(read(verification), preflight / 'drc.json', args.allow_annotations):
        state.update(termination='source_preflight_rejected')
        write(output / 'sweep.json', state)
        return 2
    assert all(digest(source / name) == sha for name, sha in inputs.items())
    assert digest(preflight / (args.board_id + '.kicad_pcb')) == inputs[args.board_id + '.kicad_pcb']
    write(output / 'sweep.json', state)
    original_order = None
    attempted, completed = set(), []
    config, previous = copy.deepcopy(base), None
    inventory = {}
    while True:
        ordinal = len(state['invocations'])
        directory = output / f'invocation-{ordinal:03}'
        config_path = output / f'sequential-{ordinal:03}.json'
        write(config_path, config)
        if previous is None:
            command = [str(binary), 'route-kicad-board-sequential', str(source),
                       args.board_id, str(directory), str(config_path)]
        else:
            resume_path = output / f'resume-{ordinal:03}.json'
            write(resume_path, dict(sequential=config,
                                   allow_existing_annotation_findings=args.allow_annotations))
            command = [str(binary), 'resume-kicad-board-sequential', str(previous),
                       str(directory), str(resume_path)]
        row = dict(ordinal=ordinal, directory=str(directory), command=command,
                   inherited_commits=len(completed), repairs_used_before=state['repair_invocations_used'])
        state['invocations'].append(row)
        write(output / 'sweep.json', state)
        started = time.monotonic()
        with (output / f'invocation-{ordinal:03}.log').open('w') as log:
            process = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
        row.update(exit_code=process.returncode, elapsed_seconds=time.monotonic() - started)
        journal = directory / 'sequential-route.json'
        if not journal.exists() or process.returncode not in [0, 1]:
            state.update(termination='execution_error', error='invocation did not return a supported journal')
            write(output / 'sweep.json', state)
            return 2
        report = read(journal)
        assert native_source_admissible(report['native_source_baseline'],
            directory / 'native-source-baseline/drc.json', args.allow_annotations), 'native source gate failed'
        if original_order is None:
            original_order = report['connection_order']
            assert config.get('maximum_connections', 1024) >= len(original_order)
            inventory = {s['connection']: dict(connection=s['connection'],
                electrical_terminal_count=s.get('electrical_terminal_count', s['distinct_pad_centers']),
                status='unattempted') for s in report['discovered_connections']}
        assert report['source_board_unchanged']
        assert report['source_board_sha256_before'] == inputs[args.board_id + '.kicad_pcb']
        assert all(digest(source / name) == sha for name, sha in inputs.items())
        assert [s['connection'] for s in report['steps'][:len(completed)]] == completed
        new_steps = report['steps'][len(completed):]
        for step in new_steps:
            net = step['connection']
            assert net not in attempted, 'a failed identity was retried within one sweep'
            attempted.add(net)
            inventory[net].update(status='completed' if committed(step) else 'failed',
                                  invocation=ordinal, step_ordinal=step['ordinal'])
            if step.get('repair'):
                state['repair_invocations_used'] += 1
        assert state['repair_invocations_used'] <= state['maximum_top_level_repairs']
        completed = [s['connection'] for s in report['steps'] if committed(s)]
        assert len(completed) == report['completed_connections']
        state['inventory'] = [inventory[n] for n in original_order]
        expected = sum(n['electrical_terminal_count'] - 1 for n in state['inventory']
                       if n['status'] != 'completed')
        native = read(Path(report['result_directory']) / 'verification.json')
        assert native['selected_net_unconnected_items'] == expected
        state.update(result_directory=report['result_directory'], native=native,
                     completed_connections=len(completed), attempted_connections=len(attempted),
                     total_connections=len(original_order), expected_unconnected_items=expected,
                     final_statistics=report['final_statistics'])
        if len(completed) == len(original_order):
            state.update(complete=native['complete'], termination='routing_complete')
            write(output / 'sweep.json', state)
            return 0 if native['complete'] else 1
        failed_tail = report['steps'][-1] if report['steps'] and not committed(report['steps'][-1]) else None
        if failed_tail and not classified_failure(failed_tail):
            state.update(termination='unclassified_failure', error='not a typed routing obstruction, or unsupported failed-repair receipt')
            write(output / 'sweep.json', state)
            return 2
        if not args.continue_after_failure:
            state.update(termination='stop_at_first_failure')
            write(output / 'sweep.json', state)
            return 1
        config = suffix_policy(base, original_order, completed, attempted,
                               state['repair_invocations_used'])
        if config is None:
            state.update(termination='sweep_exhausted')
            write(output / 'sweep.json', state)
            return 1
        assert failed_tail, 'nonterminal invocation made no classified failing tail'
        previous = directory
        write(output / 'sweep.json', state)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('board_id')
    parser.add_argument('config', type=Path)
    parser.add_argument('binary', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--continue-after-failure', action='store_true')
    parser.add_argument('--allow-annotations', action='store_true')
    args = parser.parse_args()
    try:
        sys.exit(run(args))
    except Exception as error:
        receipt = args.output / 'sweep.json'
        if receipt.exists():
            state = read(receipt)
            state.update(termination='execution_or_invariant_error', error=str(error))
            write(receipt, state)
        raise
