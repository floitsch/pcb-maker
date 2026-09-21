# Copyright (C) 2026 Toit contributors.
"""Replay a frozen adaptive run to check lazy-forecast equivalence.

Use terminal whole-board runs as references: first-pass completion should skip
forecasts, while failed first passes must retain forecast geometry, subsequent
policies, observed priorities, and native results. Explicit optimization keeps
eager forecasts. Every replay uses the existing native verification/render path.
"""
import argparse
import copy
import json
from pathlib import Path
import shutil

from run import command, digest, read, write


def forecast_decisions(rows):
    result = copy.deepcopy(rows)
    for row in result:
        for attempt in row['attempts']:
            attempt.pop('directory', None)
            attempt.pop('elapsed_micros', None)
    return result


def controls(reference, binary, output):
    before = read(reference/'adaptive-routing.json')
    output.mkdir(exist_ok=False)
    shutil.copy2(__file__, output/Path(__file__).name)
    executable = output/'pcb-maker'
    shutil.copy2(binary, executable)
    source = output/'source'
    shutil.copytree(reference/'source', source)
    board = source/(before['board_id']+'.kicad_pcb')
    source_hash = digest(board)
    write(output/'config.json', before['config'])
    process = command([executable, 'route-kicad-board-adaptive', source,
        before['board_id'], output/'replay', output/'config.json'], output/'replay.log')
    after = read(output/'replay/adaptive-routing.json')
    assert process['exit_code'] == (0 if after['complete'] else 1)
    assert digest(board) == source_hash
    assert after['source_unchanged'] and after['source_board_sha256'] == source_hash
    for key in ['routing_complete', 'complete', 'selected_pass', 'result_verification']:
        assert after[key] == before[key], key
    assert len(after['passes']) == len(before['passes'])

    optimized = after['config']['optimize_after_routing_complete']
    first_native = before['passes'][0]['native']
    baseline_complete = read(reference/'source/verification.json')['selected_net_unconnected_items'] == 0
    first_complete = baseline_complete or (first_native is not None
        and before['passes'][0]['native_progress_admissible']
        and before['passes'][0]['fixed_design_preserved']
        and first_native['selected_net_unconnected_items'] == 0)
    if optimized:
        status = 'eager_for_optimization'
    elif first_complete:
        status = 'skipped_routing_complete'
    elif before['config']['maximum_passes'] == 1:
        status = 'skipped_pass_limit'
    else:
        status = 'generated_after_initial_pass'
    assert after['forecast_generation'] == status
    assert read(output/'replay/forecast-generation.json')['status'] == status
    assert after['termination'] == ('maximum_passes' if status == 'skipped_pass_limit'
        else before['termination'])
    skipped = status.startswith('skipped_')
    if skipped:
        assert after['forecasts'] == [] and after['forecast_elapsed_micros'] == 0
        assert read(output/'replay/forecast-evidence.json') == []
        assert read(output/'replay/forecast-alternatives.json') == []
        assert not (output/'replay/forecasts').exists()
    else:
        assert forecast_decisions(after['forecasts']) == forecast_decisions(before['forecasts'])
        assert read(output/'replay/forecast-alternatives.json') == read(reference/'forecast-alternatives.json')
        assert after['remaining_queued_trials'] == before['remaining_queued_trials']

    for old, new in zip(before['passes'], after['passes'], strict=True):
        for key in ['ordinal', 'parent_pass', 'reason', 'config', 'native',
                'fixed_design_preserved', 'native_progress_admissible', 'score_mm', 'selected']:
            assert new[key] == old[key], (old['ordinal'], key)
        if not skipped:
            assert new['observed_difficulty'] == old['observed_difficulty']
        left, right = old['sequential'], new['sequential']
        assert (left is None) == (right is None)
        if left is not None:
            for key in ['connection_order', 'completed_connections', 'termination', 'total_expansions']:
                assert right[key] == left[key], (old['ordinal'], key)
            assert right['final_statistics']['copper_geometry_sha256'] == left['final_statistics']['copper_geometry_sha256']
            assert [(s['connection'], s['selected_config_index']) for s in right['steps']] == [
                (s['connection'], s['selected_config_index']) for s in left['steps']]
            assert (Path(new['directory'])/'result/preview.svg').is_file()
    assert (output/'replay/result/preview.svg').is_file()
    assert after['result_statistics']['copper_geometry_sha256'] == before['result_statistics']['copper_geometry_sha256']
    summary = dict(status='passed', process=process, reference=str(reference),
        binary_sha256=digest(executable), forecast_generation=status,
        old_forecast_seconds=before['forecast_elapsed_micros']/1e6,
        new_forecast_seconds=after['forecast_elapsed_micros']/1e6,
        passes=len(after['passes']), native=after['result_verification'],
        source_unchanged=True, pass_policy_and_copper_equivalent=True)
    write(output/'control.json', summary)
    print(json.dumps(summary))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['reference', 'binary', 'output']:
        parser.add_argument(name, type=Path)
    args = parser.parse_args()
    controls(args.reference.resolve(), args.binary.resolve(), args.output.resolve())
