# Copyright (C) 2026 Toit contributors.
"""Cold-route a native placement with compiled rules and fresh adaptive demand.

This runs every discovered net, with bounded whole-board passes. Original
annotation findings are provisional only; they cannot earn complete-layout
credit. Native routing automatically retains verification and renderings.
"""
import argparse
import copy
import json
from pathlib import Path
import shutil
import sys

from run import command,digest,read,write


def add_budget_guidance(sequential):
    """Keep ordinary entries first and append explicitly triggered retries."""
    portfolio=sequential['routing_portfolio']
    fallbacks=sequential.setdefault('obstacle_distance_fallbacks',{})
    for index,routing in enumerate(list(portfolio)):
        if routing.get('heuristic','geometric')!='geometric' or index in fallbacks.values():
            continue
        guided=copy.deepcopy(routing)
        guided['heuristic']='obstacle_distances'
        # Preserve an explicitly supplied identical guided entry's policy.
        if guided in portfolio:
            continue
        assert len(portfolio)<64, 'budget guidance exceeds the 64-entry portfolio bound'
        fallbacks[str(len(portfolio))]=index
        portfolio.append(guided)


def add_bounded_restoration(sequential):
    """Allow one nested recovery when rip-up is enabled but no policy is given."""
    repair=sequential.get('ripup')
    if repair is not None and 'restoration' not in repair:
        repair['restoration']={
            'maximum_invocations':1, 'maximum_depth':1,
            'maximum_diagnosis_trials':8, 'routing':None,
        }


def run(source,board_id,config_path,binary,output,passes,budget_guidance=True,nested_restoration=True):
    output.mkdir(exist_ok=False)
    shutil.copy2(__file__,output/Path(__file__).name)
    executable=output/'pcb-maker';shutil.copy2(binary,executable)
    native=output/'source';shutil.copytree(source,native)
    board=native/(board_id+'.kicad_pcb')
    report=dict(status='preflight',source=str(source),source_sha256=digest(board),
        source_project_sha256=digest(board.with_suffix('.kicad_pro')),
        binary_sha256=digest(executable),configuration_template_sha256=digest(config_path),
        maximum_passes=passes,processes=[])
    def call(label,argv):
        result=command(argv,output/(label+'.log'))
        report['processes'].append(dict(stage=label,**result));write(output/'run.json',report)
        return result['exit_code']
    assert call('statistics',[executable,'inspect-kicad-board',board])==0
    statistics=read(output/'statistics.log')
    assert all(statistics[k]==0 for k in ['segments','arcs','vias','copper_zones','copper_graphics'])
    helper=Path(__file__).with_name('compare_native_router.py')
    assert call('classes',[sys.executable,helper,'--native','rules',board,output/'classes.json'])==0
    compiled=read(output/'classes.json')
    rules=compiled['rules']
    global_rules=compiled['evidence']['global_rules']
    config=read(config_path)
    config['maximum_passes']=passes
    config['allow_existing_annotation_findings']=True
    sequential=config['sequential']
    assert not sequential.get('connection_order'), 'Use independently generated full-board order'
    for routing in sequential['routing_portfolio']:
        assert not routing.get('routing_demand'), 'Demand must be regenerated from this placement'
        routing['connection_rules']=rules
        routing['edge_clearance_mm']=global_rules['edge_clearance_mm']
        routing['hole_to_hole_clearance_mm']=global_rules['hole_to_hole_clearance_mm']
    if budget_guidance:
        add_budget_guidance(sequential)
    if nested_restoration:
        add_bounded_restoration(sequential)
    write(output/'config.json',config)
    report.update(status='routing',configuration_sha256=digest(output/'config.json'))
    write(output/'run.json',report)
    code=call('adaptive',[executable,'route-kicad-board-adaptive',native,board_id,output/'adaptive',output/'config.json'])
    report['exit_code']=code
    journal=output/'adaptive/adaptive-routing.json'
    if journal.exists():
        result=read(journal)
        assert result['source_unchanged'] and digest(board)==report['source_sha256']
        report.update(status='finished',native=result['result_verification'],
            routing_complete=result['routing_complete'],complete=result['complete'],
            selected_pass=result['selected_pass'])
    else:
        report.update(status='failed',error='No terminal adaptive journal; inspect process log')
    write(output/'run.json',report)
    print(json.dumps(report))
    return code


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('source',type=Path);p.add_argument('board_id')
    for name in ['config','binary','output']:p.add_argument(name,type=Path)
    p.add_argument('--passes',type=int,default=2)
    p.add_argument('--no-budget-guidance',action='store_true',
        help='Use the supplied portfolio without adding conditional search-budget retries')
    p.add_argument('--no-nested-restoration',action='store_true',
        help='Use the supplied rip-up policy without supplying a nested restoration default')
    a=p.parse_args()
    assert 1<=a.passes<=64
    raise SystemExit(run(a.source.resolve(),a.board_id,a.config.resolve(),a.binary.resolve(),a.output.resolve(),a.passes,not a.no_budget_guidance,not a.no_nested_restoration))
