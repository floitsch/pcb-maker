# Copyright (C) 2026 Toit contributors.
"""Native controls for edge translation, including deliberate real violations."""
import argparse
import json
from pathlib import Path
import shutil
import sys

from run import command, digest, read, write


def test(comparison,binary,output):
    output.mkdir(exist_ok=False)
    original=read(comparison/'report.json')
    name=original['source_verification']['board_id']
    source=comparison/'source'
    dsn=comparison/'input.dsn'
    cases=[('strict',0.5,None),('below-outline',0.005,'smaller than the external outline half-width'),
           ('off-grid',0.01005,'off the external 100 nm grid')]
    rows=[]
    for label,clearance,error in cases:
        root=output/label
        native=root/'native'
        shutil.copytree(source,native)
        project=native/(name+'.kicad_pro')
        settings=read(project)
        settings['board']['design_settings']['rules']['min_copper_edge_clearance']=clearance
        write(project,settings)
        project_hash=digest(project)
        verify=command([binary,'verify-kicad-rung',native,name],root/'verify.log')
        assert verify['exit_code'] in [0,1]
        result=command([sys.executable,Path(__file__).with_name('translate_dsn_edge.py'),dsn,native,
                        name,root/'translation'],root/'translation.log')
        assert digest(project)==project_hash
        assert digest(native/(name+'.kicad_pcb'))==original['source_sha256']
        if error:
            assert result['exit_code']!=0 and error in (root/'translation.log').read_text()
            assert not (root/'translation/translated.dsn').exists()
            row=dict(control=label,unrepresentable_rule_rejected=True)
        else:
            assert result['exit_code']==0
            receipt=read(root/'translation/report.json')
            external=read(root/'translation/after/report.json')
            native_drc=read(native/'drc.json')
            native_edges=[v for v in native_drc['violations'] if v['type']=='copper_edge_clearance']
            assert native_edges and external['edge_violation_count']>0
            assert receipt['loaded_edge_rule_matches'] and receipt['non_edge_clearances_unchanged']
            row=dict(control=label,native_edge_findings=len(native_edges),
                external_edge_findings=external['edge_violation_count'],
                true_violations_preserved=True)
        rows.append(dict(**row,source_unchanged=True,verification_process=verify,translation_process=result))
        write(output/'report.json',dict(controls=rows,complete=len(rows)==len(cases)))
    print(json.dumps(rows))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('comparison',type=Path)
    p.add_argument('binary',type=Path)
    p.add_argument('output',type=Path)
    a=p.parse_args()
    test(a.comparison.resolve(),a.binary.resolve(),a.output.resolve())
