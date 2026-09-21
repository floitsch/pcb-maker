# Copyright (C) 2026 Toit contributors.
"""Summarize terminal edge-exchange controls and the unanchored placement probe."""
import argparse
import json
from pathlib import Path
import xml.etree.ElementTree as ET

from run import read,write


def report(root):
    external=read(root/'complex-router/report.json')
    assert external['status']=='finished' and external['edge_translation']['loaded_edge_rule_matches']
    assert external['edge_translation']['non_edge_clearances_unchanged']
    assert external['fixed_placement_matches'] and not external['dimensional_mismatches']
    controls=read(root/'translation-controls/report.json')
    assert controls['complete']
    ablation=read(root/'placement-attraction-control/report.json')
    assert ablation['poses_identical']
    seeds=read(root/'spectral-seeds/report.json')
    rank=next(row for row in seeds['cases'] if row['kind']=='rank')
    assert rank['physical_placement_audit']['passed']
    native=read(root/'spectral-native-final/report.json')
    assert native['status']=='verified_cold_placement' and native['inventory_audit']
    baseline=seeds['baseline_evidence'];candidate=rank['evidence']
    comparison=dict(baseline_distance_mm=baseline['connectivity_distance_after_mm'],
        candidate_distance_mm=candidate['connectivity_distance_after_mm'],
        baseline_crossings=baseline['demand']['cross_net_proper_crossings'],
        candidate_crossings=candidate['demand']['cross_net_proper_crossings'])
    comparison['distance_reduction_percent']=100*(1-comparison['candidate_distance_mm']/comparison['baseline_distance_mm'])
    summary=dict(external_native=external['native'],external_seconds=external['router_seconds'],
        edge_translation=external['edge_translation'],ecc83_edge_translation=read(root/'ecc83-translation/report.json'),
        controls=controls,attraction_disabled_poses_identical=True,placement=comparison,
        candidate_native=native['native'],candidate_routing_tested=False)
    summary['svg_xml_checked']=0
    for p in root.rglob('*.svg'):
        ET.parse(p);summary['svg_xml_checked']+=1
    write(root/'summary.json',summary)
    (root/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Unanchored placement and edge exchange</title>'
        '<style>body{font:17px system-ui;margin:2rem}img{max-width:100%}.pair{display:grid;grid-template-columns:1fr 1fr;gap:1rem}</style>'
        '<h1>Unanchored placement is the next target</h1>'
        '<p>All 68 components in the current larger benchmark use the harmonic initializer’s random fallback. '
        'Disabling every net attraction leaves every pose identical. No positional anchors are present.</p>'
        f'<p>A net-aware spectral rank seed reduces the placement distance estimate by {comparison["distance_reduction_percent"]:.1f}% '
        f'and estimated cross-net crossings from {comparison["baseline_crossings"]} to {comparison["candidate_crossings"]}. '
        'Production legalization and native geometry/inventory audits pass. Routing has not been tested on this seed.</p>'
        '<div class="pair"><figure><figcaption>Earlier random fallback</figcaption><img src="complex-router/source/inspection-combined.svg"></figure>'
        '<figure><figcaption>Net-aware rank seed; 3 annotation findings remain</figcaption><img src="spectral-native-final/native-silk/inspection-combined.svg"></figure></div>'
        '<h2>Edge exchange corrected</h2><p>The translated input has matching nominal global edge clearance and unchanged non-edge clearance matrix entries. '
        'All 14 cold-input edge findings disappear on the larger board. Its new external run still leaves 66 native opens and seven annotation findings. '
        'The prior run left 65 opens; neither is complete, and full exchange equivalence remains unproven.</p>'
        '<p>Strict-rule controls retain real violations. Rules below the external outline half-width or off its 100 nm grid are rejected. '
        'ECC83 retains two tiny outline conflicts caused by exported boundary rounding; these are not waived.</p>'
        '<p><a href="complex-router/index.html">External result</a> · '
        '<a href="complex-translation/after/index.html">Corrected larger input</a> · '
        '<a href="ecc83-translation/after/index.html">ECC83 residual findings</a> · '
        '<a href="translation-controls/report.json">Translation controls</a> · '
        '<a href="placement-attraction-control/report.json">Attraction ablation</a> · '
        '<a href="spectral-seeds/report.json">Seed evidence</a> · '
        '<a href="spectral-native-final/report.json">Native placement audit</a> · '
        '<a href="summary.json">Summary</a></p>')
    print(json.dumps({k:v for k,v in summary.items() if k not in ['controls','edge_translation','ecc83_edge_translation']}))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('root',type=Path)
    report(p.parse_args().root.resolve())
