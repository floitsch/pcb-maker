# Copyright (C) 2026 Toit contributors.
"""Materialize a positions-only seed experiment through existing native gates."""
import argparse
import json
from pathlib import Path
import shutil
import sys

from render_inspection_layers import render
from run import command,digest,read,write


def materialize(reference,probe,binary,output):
    output.mkdir(exist_ok=False)
    shutil.copy2(__file__,output/Path(__file__).name)
    original=read(reference/'problem.json');new=read(probe/'problem.json')
    for value in [original,new]:
        for component in value['components']:component.pop('position')
    assert original==new, 'Probe changed more than initial component positions'
    request=read(reference/'materialize-request.json')
    source_board=Path(request['board'])
    name=source_board.stem
    native=output/'native'
    shutil.copytree(source_board.parent,native)
    board=native/source_board.name
    request.update(board=str(board),poses=read(probe/'placement.json')['poses'])
    write(output/'request.json',request)
    report=dict(status='materializing',source_problem_sha256=digest(reference/'problem.json'),
        probe_problem_sha256=digest(probe/'problem.json'),binary_sha256=digest(binary),processes=[],routing_tested=False)
    def call(label,argv,allowed=(0,)):
        result=command(argv,output/(label+'.log'))
        report['processes'].append(dict(stage=label,**result));write(output/'report.json',report)
        assert result['exit_code'] in allowed,label
    helper=Path(__file__).parent
    call('materialize',[sys.executable,helper/'materialize_native_placement.py',output/'request.json'])
    # SaveBoard may migrate a project. Preserve the native source contract,
    # exactly as the general area runner does before verification.
    shutil.copy2(source_board.with_suffix('.kicad_pro'),board.with_suffix('.kicad_pro'))
    call('normalize',[binary,'normalize-kicad-footprint-links',native,name])
    call('verify',[binary,'verify-kicad-rung',native,name],(0,1))
    audit_request=dict(source=str(source_board),placed=str(board),poses=request['poses'],
        mapping=request['mapping'],source_bounds=request['bounds'],ratio=request['ratio'])
    write(output/'audit-request.json',audit_request)
    call('inventory-audit',[sys.executable,helper/'audit_native_placement.py',output/'audit-request.json',output/'audit.json'])
    render(native,name)
    report['before_annotations']=read(native/'verification.json')
    repaired=output/'native-silk'
    call('annotations',[binary,'repair-kicad-silkscreen',native,name,repaired],(0,1))
    assert (repaired/'verification.json').exists()
    audit_request['placed']=str(repaired/source_board.name)
    write(output/'repaired-audit-request.json',audit_request)
    call('repaired-inventory-audit',[sys.executable,helper/'audit_native_placement.py',output/'repaired-audit-request.json',output/'repaired-audit.json'])
    render(repaired,name)
    verification=read(repaired/'verification.json')
    physical=[v for v in read(repaired/'drc.json')['violations']
        if v['type'] not in ['silk_overlap','silk_over_copper','silk_edge_clearance']
        and not (v['type']=='lib_footprint_mismatch' and v['severity']=='warning')]
    assert not physical and verification['erc_violations']==verification['schematic_parity_issues']==0
    report.update(status='verified_cold_placement',native=verification,
        inventory_audit=True,only_semantic_positions_changed=True,source_rules_preserved=True,
        no_native_physical_findings=True,native_directory=str(repaired))
    write(output/'report.json',report)
    print(json.dumps(report))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['reference','probe','binary','output']:p.add_argument(name,type=Path)
    a=p.parse_args()
    materialize(a.reference.resolve(),a.probe.resolve(),a.binary.resolve(),a.output.resolve())
