# Copyright (C) 2026 Toit contributors.
"""Compare finished first passes without mistaking live adaptive runs for final results."""
import argparse
import html
from pathlib import Path
import xml.etree.ElementTree as ET

from run import read, write


def report(root):
    contract = read(root/'comparison-contract.json')
    assert all(contract[k] for k in ['same_configuration', 'same_native_classes',
        'same_class_assignments', 'same_project', 'same_binary', 'same_initial_order', 'cold_sources'])
    parity = read(root/'driver-seed-parity.json')
    assert parity['entire_seed_problem_identical'] and parity['accepted_poses_identical']
    rows = []
    for name in ['random', 'spectral']:
        directory = root/name/'adaptive/pass-000'
        sequence = read(directory/'sequential-route.json')
        # The parent has advanced to another pass, so this subtree is immutable.
        assert (directory.parent/'pass-001/config.json').exists()
        assert sequence['termination'] in ['complete', 'routing_failed']
        audit = read(directory/'recovery-audit.json')
        assert all(audit[k] for k in ['native_connectivity_ledger_reproduced',
            'unrelated_copper_and_pose_preserved', 'native_dimensions_match_selected_rules', 'source_unchanged'])
        native = read(directory/'result/verification.json')
        assert audit['native'] == native
        rows.append(dict(tool='pcb-maker, finished first pass', placement=name,
            nets=sequence['completed_connections'], native=native,
            image=f'{name}/adaptive/pass-000/result/inspection-combined.svg',
            evidence=f'{name}/adaptive/pass-000/recovery.html'))
    for name in ['random', 'spectral']:
        directory = root/f'freerouting-{name}-two-layer'
        external = read(directory/'report.json')
        assert external['status'] == 'finished' and external['router_exit_code'] == 0
        assert external['fixed_placement_matches'] and external['project_and_classes_unchanged']
        assert not external['dimensional_mismatches']
        assert external['layer_adaptation']['geometry_stack_and_net_rules_unchanged']
        layers = external['exchange_geometry']['routing_layers']
        assert len(layers) == 2 and all(l['is_signal'] and l['router_active'] for l in layers)
        assert external['edge_translation']['loaded_edge_rule_matches']
        assert external['edge_translation']['non_edge_clearances_unchanged']
        rows.append(dict(tool='Freerouting, two signal layers', placement=name,
            nets=None, native=external['native'],
            image=f'{directory.name}/result/inspection-combined.svg',
            evidence=f'{directory.name}/index.html'))
    for row in rows:
        ET.parse(root/row['image'])
    processes = {name:read(root/name/'run.json')['status'] for name in ['random', 'spectral']}
    finished = {}
    for name, status in processes.items():
        if status == 'finished' and (root/name/'audit.json').exists():
            result = read(root/name/'adaptive/adaptive-routing.json')
            audit = read(root/name/'audit.json')['runs']
            assert len(audit) == 1 and audit[0]['incumbent_selection_reproduced']
            assert audit[0]['selected_pass'] == result['selected_pass']
            finished[name] = dict(passes=len(result['passes']), selected_pass=result['selected_pass'],
                native=result['result_verification'])
    write(root/'summary.json', dict(scope=__doc__, contract=contract, rows=rows,
        adaptive_process_status_at_report=processes, driver_seed_parity=parity,
        audited_terminal_adaptive_results=finished,
        full_layout_complete=False, area_reduction_claimed=False))
    page = ['<!doctype html><meta charset="utf-8"><title>Placement and whole-board completion</title>',
        '<style>body{font:17px system-ui;margin:2rem}table{border-collapse:collapse}'
        'td,th{border:1px solid #aaa;padding:.5rem}.pair{display:grid;grid-template-columns:1fr 1fr;gap:1rem}'
        'img{width:100%}@media(max-width:800px){.pair{grid-template-columns:1fr}}</style>',
        '<h1>Placement unlocks all 50 nets</h1><p>The net-aware spectral seed connects every net '
        'in pcb-maker’s first pass. The random placement leaves 30 native open items. '
        'Both retain 68 components, the original net classes and the same board area.</p>',
        '<p>These are finished first-pass results. The configured two-pass jobs have separate terminal '
        'reports; this page does not present intermediate checkpoints as their final selections.</p>',
        '<table><tr><th>Router</th><th>Placement</th><th>Native opens</th><th>Annotation findings</th></tr>']
    for row in rows:
        n = row['native']
        page.append(f'<tr><td><a href="{row["evidence"]}">{html.escape(row["tool"])}</a></td>'
            f'<td>{row["placement"]}</td><td>{n["selected_net_unconnected_items"]}</td>'
            f'<td>{n["drc_design_violations"]}</td></tr>')
    page.append('</table><p>Three annotations remain on the spectral placement; seven remain on '
        'the random placement. Neither earns complete-layout or smaller-area credit.</p>'
        '<p>The external controls explicitly adapt the exported front layer from power to signal. '
        'Without this adaptation Freerouting uses only the back layer. Nominal edge clearance and '
        'basic source classes are audited; full exchange-geometry equivalence remains unproven. '
        'Concurrent runs and different router budgets preclude an equal-budget speed ranking.</p>')
    for name, result in finished.items():
        page.append(f'<p><a href="{name}/index.html">Finished {name} adaptive run</a>: '
            f'{result["passes"]} passes; selected pass {result["selected_pass"]}; '
            f'{result["native"]["selected_net_unconnected_items"]} native opens. '
            'The final selection has an independent audit.</p>')
    for start in [0, 2]:
        page.append('<div class="pair">')
        for row in rows[start:start+2]:
            page.append(f'<figure><figcaption>{html.escape(row["tool"])} / {row["placement"]}</figcaption>'
                f'<a href="{row["image"]}"><img src="{row["image"]}"></a></figure>')
        page.append('</div>')
    page.append('<p>Combined front/back copper; component bodies hidden. '
        '<a href="summary.json">Evidence summary</a> · '
        '<a href="driver-seed-parity.json">General-runner seed replay</a></p>')
    (root/'index.html').write_text(''.join(page))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    report(parser.parse_args().root.resolve())
