#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Cold-position, whole-netlist placement and routing for native KiCad projects.

This deliberately defines an all-free geometric repacking benchmark, not a
drop-in mechanical replacement: mounting holes and connectors may move. Native
footprints and their orientations are retained; movable centers are randomized
without using human positions, while explicitly fixed components retain theirs. The default uses conservative graphics/pad envelopes;
an explicit placement policy opts into native courtyard geometry and overhang.
Only a fully routed, native-clean result earns an area score. Failed placements
are rendered too. Unknown product constraints are not inferred from coordinates.
"""
import argparse
import copy
import importlib.util
import math
from pathlib import Path
import random
import shutil
import sys

from run import ROOT, command, digest, read, write


def load_pcbnew():
    import pcbnew
    if not hasattr(pcbnew.SwigPyIterator, 'next'):
        pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__
    return pcbnew


def vec(x, y):
    return {'x': x, 'y': y}


def apply_net_tension_weights(problem, placement_policy):
    """Apply explicit attraction intent using exact native net identities.

    Weights affect placement attraction only. Validate the entire map before
    mutating anything; omitted policy preserves the original problem exactly.
    """
    if placement_policy is None or 'net_tension_weights' not in placement_policy:
        return
    weights = placement_policy['net_tension_weights']
    if not isinstance(weights, dict):
        raise ValueError('net_tension_weights must be a net-name to numeric-weight map')
    nets = {net['id']: net for net in problem['electrical_nets']}
    for name, weight in weights.items():
        if not isinstance(name, str) or name not in nets:
            raise ValueError(f'Unknown placement net identity: {name!r}')
        try:
            valid = type(weight) in (int, float) and math.isfinite(weight) and weight >= 0
        except OverflowError:
            valid = False
        if not valid:
            raise ValueError(f'Placement tension weight for {name!r} must be finite and nonnegative')
    for name, weight in weights.items():
        nets[name]['tension_weight'] = weight


def outline_polygon(board, native_units=False):
    """One closed native segment loop, in millimetres; reject ambiguous graphs."""
    pcbnew=load_pcbnew()
    graph={}
    for edge in board.GetDrawings():
        if edge.GetLayer()!=pcbnew.Edge_Cuts:
            continue
        assert edge.GetShape()==pcbnew.SHAPE_T_SEGMENT, 'Curved outline import is not supported'
        a=(edge.GetStart().x,edge.GetStart().y);b=(edge.GetEnd().x,edge.GetEnd().y)
        assert a!=b
        graph.setdefault(a,[]).append(b);graph.setdefault(b,[]).append(a)
    assert len(graph)>=3 and all(len(v)==2 for v in graph.values()), 'Outline must be one closed segment loop'
    start=min(graph);current=start;previous=None;points=[]
    while True:
        assert current not in points, 'Outline revisits a vertex'
        points.append(current)
        next_=next(p for p in sorted(graph[current]) if p!=previous)
        if next_==start:
            break
        previous,current=current,next_
    assert len(points)==len(graph), 'Multiple outline loops are not supported'
    # Crossings invalidate shoelace area even when every vertex has degree two.
    def cross(a,b,c):return (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
    segments=list(zip(points,points[1:]+points[:1]))
    for i,(a,b) in enumerate(segments):
        for j,(c,d) in enumerate(segments[i+1:],i+1):
            if j==i+1 or (i==0 and j==len(segments)-1):continue
            boxes_overlap = max(min(a[0],b[0]),min(c[0],d[0])) <= min(max(a[0],b[0]),max(c[0],d[0])) and max(min(a[1],b[1]),min(c[1],d[1])) <= min(max(a[1],b[1]),max(c[1],d[1]))
            assert not (boxes_overlap and cross(a,b,c)*cross(a,b,d)<=0 and cross(c,d,a)*cross(c,d,b)<=0), 'Self-intersecting outline'
    return points if native_units else [(x/1e6,y/1e6) for x,y in points]


def rectangular_outline_bounds(board):
    """Classify an exact native rectangle without replacing split edge segments."""
    points = outline_polygon(board, native_units=True)
    def cross(a, b, c):
        return (b[0]-a[0])*(c[1]-b[1])-(b[1]-a[1])*(c[0]-b[0])
    corners = [b for a,b,c in zip(points[-1:]+points[:-1], points, points[1:]+points[:1])
               if cross(a,b,c) != 0]
    xs = [p[0] for p in points]; ys = [p[1] for p in points]
    x0,y0,x1,y1 = min(xs),min(ys),max(xs),max(ys)
    assert x0 < x1 and y0 < y1 and len(corners) == 4
    assert set(corners) == {(x0,y0),(x0,y1),(x1,y0),(x1,y1)}, 'Nonrectangular native contour'
    assert all(a[0] == b[0] or a[1] == b[1]
               for a,b in zip(corners,corners[1:]+corners[:1])), 'Sloped rectangle edge'
    return x0,y0,x1,y1


def extract(board, placement_policy=None, connection_rules=None):
    """Local envelopes retain orientation; native copper is never translated.

    References and pad numbers are the mapping, not normalized net aliases.
    Pads in the placement model use conservative bounding rectangles. The exact
    original pads are retained for routing and native admission.
    """
    pcbnew = load_pcbnew()
    assert board.GetCopperLayerCount() == 2, 'Placement adapter currently requires two copper layers'
    assert not any(z.GetIsRuleArea() for z in board.Zones()), 'Placement rule areas need explicit translation'
    # Preserve the actual polygon area for an explicitly free board shape.
    # A fixed/scaled outline still requires the existing rectangular model.
    edges = [x for x in board.GetDrawings() if x.GetLayer() == pcbnew.Edge_Cuts]
    assert len(edges) >= 3 and all(x.GetShape() == pcbnew.SHAPE_T_SEGMENT for x in edges), 'Placement source outline currently requires straight segments'
    points = [(p.x/1e6, p.y/1e6) for e in edges for p in (e.GetStart(), e.GetEnd())]
    x0, y0 = min(p[0] for p in points), min(p[1] for p in points)
    x1, y1 = max(p[0] for p in points), max(p[1] for p in points)
    if placement_policy and placement_policy.get('outline') == 'free_rectangle':
        polygon = outline_polygon(board)
        area = abs(sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(polygon,polygon[1:]+polygon[:1])))/2
        aspect = (x1-x0)/(y1-y0)
        assert area > 0 and aspect > 0
        x1, y1 = x0+math.sqrt(area*aspect), y0+math.sqrt(area/aspect)
    else:
        rectangular_outline_bounds(board)
    problem = {'schema_version': 1,
               'board': {'bounds': {'min': vec(0,0), 'max': vec(x1-x0,y1-y0)},
                         'layers': [{'id':'top'}, {'id':'bottom'}]},
               'rules': {'clearance': .4, 'via_diameter': 1.2, 'via_drill': .6},
               'components': [], 'electrical_nets': []}
    mapping, nets = {}, {}
    from preserved_board_graphics import preserved_graphics, ROLE
    graphics = preserved_graphics(board.GetFileName())
    courtyards = None
    if placement_policy is not None:
        from courtyard_geometry import extract_courtyards
        allowed = {'scope','movement','rotation','body_overhang_mm','movable_components','outline',
                   'net_tension_weights'}
        assert placement_policy.get('outline','scaled_source') in ('scaled_source','free_rectangle')
        assert not set(placement_policy)-allowed, 'Unknown placement policy fields'
        assert placement_policy.get('movement') == 'explicit' or 'movable_components' not in placement_policy
        courtyards = extract_courtyards(board)
        assert placement_policy['movement'] in ('all_free', 'explicit')
        if placement_policy['movement'] == 'explicit':
            assert len(set(placement_policy['movable_components'])) == len(placement_policy['movable_components'])
            assert set(placement_policy['movable_components']) <= set(courtyards)
        assert placement_policy['rotation'] == 'original_fixed'
        assert set(placement_policy['body_overhang_mm']) <= set(courtyards)
    for fp in sorted(board.GetFootprints(), key=lambda f: f.GetReference()):
        ref = str(fp.GetReference())
        if ref in graphics:
            mapping[ref] = {'placement_role': ROLE}
            continue
        # Keep the BOX2I alive while querying it (SWIG temporaries can dangle).
        box = fp.GetBoundingBox(False, False)
        center = box.GetCenter()
        cx, cy = center.x/1e6, center.y/1e6
        w, h = box.GetWidth()/1e6, box.GetHeight()/1e6
        if courtyards is not None:
            cx, cy = courtyards[str(fp.GetReference())]['center']
            w, h = courtyards[str(fp.GetReference())]['size']
        assert math.isfinite(w) and math.isfinite(h) and w > 0 and h > 0, 'Invalid native component extent'
        ref = fp.GetReference()
        assert ref not in mapping
        mapping[ref] = {'anchor_offset': vec(fp.GetPosition().x/1e6-cx, fp.GetPosition().y/1e6-cy),
                        'source_center': vec(cx,cy),
                        'orientation_degrees': fp.GetOrientationDegrees()}
        pins = {}
        pin_nets = {}
        for pad in fp.Pads():
            number = str(pad.GetNumber())
            # Physical pads may share a logical pin, and mounting holes may
            # have no number. Retain all physical copper locations.
            if not number:
                assert not pad.GetNetname(), f'{ref}: unnumbered pad has a net'
                number = '@unnumbered:' + pad.m_Uuid.AsString()
            pin = pins.setdefault(number, {'id':number, 'offset':vec(0,0), 'pads':[]})
            net = str(pad.GetNetname())
            if number in pin_nets:
                assert pin_nets[number] == net, f'{ref}.{number}: conflicting physical pad nets'
            pin_nets[number] = net
            pos = pad.GetPosition()
            bounds = pad.GetBoundingBox()
            copper_center = bounds.GetCenter()
            copper = []
            for layer, name in ((pcbnew.F_Cu, 'top'), (pcbnew.B_Cu, 'bottom')):
                if pad.IsOnLayer(layer):
                    copper.append({'id':pad.m_Uuid.AsString()+':'+name,
                                   'local_center':vec(copper_center.x/1e6-cx,copper_center.y/1e6-cy),
                                   'layer': name, 'shape': {'kind': 'rect',
                                   'size': vec(bounds.GetWidth()/1e6, bounds.GetHeight()/1e6)}})
            if not pin['pads']:
                pin['offset'] = vec(pos.x/1e6-cx,pos.y/1e6-cy)
            pin['pads'].extend(copper)
        for number, net in pin_nets.items():
            if net:
                nets.setdefault(net, []).append({'component': ref, 'pin': number})
        # All free, fixed orientation. The random seed assigns centers later.
        problem['components'].append({'id':ref, 'position':vec(0,0),
            'size':vec(w,h), 'constraints':{'movement':'free','rotation':'fixed'},
            'body_is_routing_keepout':False, 'pins':list(pins.values())})
        component = problem['components'][-1]
        if placement_policy and placement_policy['movement'] == 'explicit' and ref not in placement_policy['movable_components']:
            component['constraints']['movement'] = 'fixed'
            component['position'] = vec(cx-x0,cy-y0)
        if courtyards is not None:
            problem['components'][-1]['placement_geometry'] = {
                'body':courtyards[ref]['shape'], 'clearance':0.0,
                'body_layers':courtyards[ref]['layers'],
                'board_overhang':placement_policy['body_overhang_mm'].get(ref,0.0),
                'pad_edge_clearance':board.GetDesignSettings().m_CopperEdgeClearance/1e6}
            if courtyards[ref].get('body_parts'):
                component['placement_geometry']['body_parts']=courtyards[ref]['body_parts']
    if connection_rules is not None:
        assert connection_rules, 'Native connection rules are required'
        # Placement has one global copper margin. Use a conservative maximum;
        # the native router receives the actual per-net dimensions below.
        problem['rules'] = {
            'clearance':max(board.GetDesignSettings().m_MinClearance/1e6, max(r['clearance_mm'] for r in connection_rules.values())),
            'via_diameter':max(r['via_size_mm'] for r in connection_rules.values()),
            'via_drill':max(r['via_drill_mm'] for r in connection_rules.values())}
    for name, terminals in sorted(nets.items()):
        if len(terminals) > 1:
            problem['electrical_nets'].append({'id':name, 'width':(connection_rules[name.removeprefix('/')]['trace_width_mm'] if connection_rules is not None else .8),
                'layer':'top', 'allowed_layers':['top','bottom'], 'terminals':terminals})
    apply_net_tension_weights(problem, placement_policy)
    return problem, mapping, (x0,y0,x1,y1)


def render(problem, poses, path, title):
    if all('placement_geometry' in c for c in problem['components']):
        from courtyard_geometry import audit, render as render_courtyards, signed_gap
        by = {p['component']:p for p in poses}
        assert len(by) == len(problem['components'])
        bodies, pads, overhang = {}, {}, {}
        for c in problem['components']:
            p=by[c['id']]; assert p['rotation_degrees'] == 0
            center=[p['position']['x'],p['position']['y']]
            bodies[c['id']]={'center':center,'size':[c['size']['x'],c['size']['y']],
                             'shape':c['placement_geometry']['body'],
                             'layers':c['placement_geometry'].get('body_layers',[])}
            if c['placement_geometry'].get('body_parts'):
                bodies[c['id']]['body_parts']=c['placement_geometry']['body_parts']
            overhang[c['id']]=c['placement_geometry'].get('board_overhang',0)
            pads[c['id']]=[]
            for pin in c['pins']:
                for pad in pin['pads']:
                    assert pad['shape']['kind']=='rect'
                    size=pad['shape']['size']
                    pads[c['id']].append({'center':[center[0]+pad.get('local_center',pin['offset'])['x'],center[1]+pad.get('local_center',pin['offset'])['y']],
                                         'size':[size['x'],size['y']], 'shape':pad['shape'], 'layers':[pad['layer']] if c['placement_geometry'].get('body_layers') else []})
        b=problem['board']['bounds']; bounds=[b['min']['x'],b['min']['y'],b['max']['x'],b['max']['y']]
        margins={c['id']:c['placement_geometry']['clearance'] for c in problem['components']}
        result=audit(bodies,bounds,overhang,margins)
        issues=[]
        for ref,items in pads.items():
            component=next(c for c in problem['components'] if c['id']==ref)
            edge_clearance=component['placement_geometry'].get('pad_edge_clearance',0)
            pad_bounds=[bounds[0]+edge_clearance,bounds[1]+edge_clearance,bounds[2]-edge_clearance,bounds[3]-edge_clearance]
            for pad in items:
                if not audit({'pad':pad},pad_bounds)['passed']:issues.append('pad edge clearance '+ref)
                for other,body in bodies.items():
                    if other!=ref and signed_gap(pad,body)<margins[other]-1e-6:issues.append('pad/body overlap '+ref+'/'+other)
        refs=sorted(pads)
        for i,ref in enumerate(refs):
            for other in refs[i+1:]:
                if any(signed_gap(a,b)<problem['rules']['clearance']-1e-6 for a in pads[ref] for b in pads[other]):
                    issues.append('pad clearance '+ref+'/'+other)
        result['pad_issues']=sorted(set(issues));result['passed'] &= not issues
        result['scope']='Independent circle/convex-part courtyard union and conservative pad bounds on matching sides; explicit body-only overhang; fixed original orientations.'
        render_courtyards(bodies,bounds,path,title,pads)
        return result
    spec = importlib.util.spec_from_file_location('placement_view', ROOT/'experiments/placement-exploration/run.py')
    view = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(view)
    illustrated = copy.deepcopy(problem)
    illustrated['nets'] = [{'from': n['terminals'][0], 'to': t}
                           for n in problem['electrical_nets'] for t in n['terminals'][1:]]
    path.write_text(view.svg(illustrated, poses, title))
    return view.independent_pose_check(problem, poses)


def audit_native(source, placed, poses, mapping, bounds, ratio, output):
    # Isolate native readback from the long-lived pcbnew mutation process.
    # KiCad's SWIG bindings have returned untyped board pointers when multiple
    # edited/loaded boards coexist; a fresh process provides a stable boundary.
    request=output.with_name(output.stem+'-request.json')
    write(request,{'source':str(source),'placed':str(placed),'poses':poses,
                   'mapping':mapping,'source_bounds':bounds,'ratio':ratio})
    checked=command([sys.executable,ROOT/'experiments/whole-board/audit_native_placement.py',request,output],
                    output.with_suffix('.log'))
    assert checked['exit_code']==0, f'Native inventory audit failed: {checked["log"]}'
    return read(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--ratios', type=float, nargs='+', default=[1.0,.9,.8])
    parser.add_argument('--seed', type=int, default=0)
    parser.add_argument('--route', action='store_true', help='Route every legal placement with the selected backend (legacy paired comparison by default)')
    parser.add_argument('--materialize-only',action='store_true',help='Materialize and natively verify accepted placements without starting routing')
    parser.add_argument('--trace-placement',action='store_true',help='Retain harmonic legalization playback and final poses even when placement fails')
    legalization = parser.add_mutually_exclusive_group()
    legalization.add_argument('--coupled-legalization',dest='coupled_legalization',action='store_true',help='Try bounded joint contact recovery after 128 ordinary sweeps (default)')
    legalization.add_argument('--pairwise-legalization',dest='coupled_legalization',action='store_false',help='Use only the original sequential pair corrections for comparisons')
    parser.set_defaults(coupled_legalization=True)
    parser.add_argument('--legalization-sweeps',type=int,default=512,help='Explicit harmonic legalization sweep budget')
    parser.add_argument('--placement-pair-checks',type=int,default=1000000,help='Explicit harmonic legalization pair-check budget')
    parser.add_argument('--skip-collision-intervals',action='store_true',
                        help='Skip certified colliding lattice intervals within the shared placement pair-work budget')
    parser.add_argument('--seed-extra-pad-gap-mm',type=float,default=0.0,
                        help='Transient extra copper-pair gap during largest-first insertion; native geometry and rules stay unchanged')
    parser.add_argument('--largest-first-seed-insertion',action='store_true',
                        help='Insert largest footprint bounds first against anchors and already inserted parts, using the same sampled domain and shared budget')
    parser.add_argument('--fixed-obstacle-seed-projection',action='store_true',
                        help='Project harmonic centers onto a 0.25 mm fixed-obstacle-free sampled domain using the shared pair budget')
    parser.add_argument('--contact-branch-trials',type=int,default=0,
                        help='Bounded alternate contact directions after coupled failure; shared work limits (default disabled)')
    parser.add_argument('--placement-policy',type=Path,help='Explicit all-free courtyard/overhang benchmark policy')
    parser.add_argument('--router-seconds',type=int,default=600,help='Paired-router process limit; adaptive routing uses its configuration work limits')
    parser.add_argument('--placement-attempts',type=int,default=1,help='Bounded random starts for an underanchored courtyard problem')
    parser.add_argument('--placement-seed',choices=['random','spectral_rank'],default='random',
                        help='Initial positions for free components; spectral_rank requires all-free fixed orientations and partitions disconnected attraction blocks')
    parser.add_argument('--repair-silkscreen',action='store_true',help='Run the protected native annotation repair before routing')
    parser.add_argument('--source-board',type=Path,help='Native project board; schematic and project must be beside it')
    routing_backend = parser.add_mutually_exclusive_group()
    routing_backend.add_argument('--adaptive-config',type=Path,help='Use the general adaptive native router with compiled source net classes')
    routing_backend.add_argument('--freerouting-config',type=Path,help='Use the source-preserving external native-board backend with explicit tool and time bounds')
    parser.add_argument('--binary',type=Path,default=ROOT/'target/release/pcb-maker')
    parser.add_argument('--route-with-annotation-findings',action='store_true',help='Allow unchanged source annotation findings during provisional adaptive routing; complete-layout credit still requires a fully clean native result')
    args = parser.parse_args()
    if not math.isfinite(args.seed_extra_pad_gap_mm) or args.seed_extra_pad_gap_mm < 0:
        parser.error('--seed-extra-pad-gap-mm must be finite and nonnegative')
    if args.seed_extra_pad_gap_mm and not args.largest_first_seed_insertion:
        parser.error('--seed-extra-pad-gap-mm requires --largest-first-seed-insertion')
    if args.route and args.materialize_only:
        parser.error('--route and --materialize-only are mutually exclusive')
    if args.source_board and (not args.placement_policy or (args.route and not (args.adaptive_config or args.freerouting_config))):
        parser.error('A general source requires an explicit placement policy and, for routing, --adaptive-config or --freerouting-config')
    assert all(0 < ratio <= 1 for ratio in args.ratios)
    assert args.placement_attempts > 0 and args.router_seconds > 0
    assert args.legalization_sweeps > 0 and args.placement_pair_checks > 0
    if not 0 <= args.contact_branch_trials <= 256:
        parser.error('--contact-branch-trials must be between 0 and 256')
    if args.contact_branch_trials and not args.coupled_legalization:
        parser.error('--contact-branch-trials requires coupled legalization')
    if args.placement_seed=='spectral_rank' and args.placement_attempts!=1:
        parser.error('spectral_rank is deterministic; --placement-attempts must be 1')
    out = args.output.resolve()
    planned_source = args.source_board.resolve().parent if args.source_board else ROOT/'benchmarks/real/external/kicad-ecc83-pp'
    assert not out.is_relative_to(planned_source), 'Output must be outside the source project'
    out.mkdir(parents=True, exist_ok=False)
    exe = out/'pcb-maker'
    shutil.copy2(args.binary.resolve(), exe)
    snapshot=out/'driver-snapshot';snapshot.mkdir()
    driver_files=['area_probe.py','materialize_native_placement.py','audit_native_placement.py','audit_dsn_classes.py','compile_net_classes.py','courtyard_geometry.py','preserved_board_graphics.py','placement_polygons.py','run.py']
    if args.placement_seed=='spectral_rank':driver_files.append('spectral_placement_seed.py')
    if args.trace_placement:driver_files.append('trace_placement.py')
    provenance={}
    for name in driver_files:
        path=ROOT/'experiments/whole-board'/name
        shutil.copy2(path,snapshot/name);provenance[name]=digest(path)
    write(out/'driver-sources.json',provenance)
    source_board = args.source_board.resolve() if args.source_board else ROOT/'benchmarks/real/external/kicad-ecc83-pp/ecc83-pp.kicad_pcb'
    source = source_board.parent
    board_id = source_board.stem
    assert not out.is_relative_to(source), 'Output must be outside the source project'
    assert source_board.is_file() and source_board.with_suffix('.kicad_pro').is_file()
    source_hash = digest(source_board)
    if not args.source_board:
        expected = next(s for s in read(ROOT/'benchmarks/real/sources.json')['sources'] if s['id']=='kicad-ecc83-pp')
        for name, sha in expected['files'].items():
            assert digest(source/name) == sha
    # Retain a combined native input view even if semantic extraction fails.
    command(['kicad-cli','pcb','export','svg','--mode-single','--layers','F.Cu,B.Cu,Edge.Cuts',
             '--page-size-mode','2','--exclude-drawing-sheet','--output',out/'source-combined.svg',source_board],out/'source-render.log')
    pcbnew = load_pcbnew()
    board = pcbnew.LoadBoard(str(source_board))
    native_edge_clearance_mm = board.GetDesignSettings().m_CopperEdgeClearance/1e6
    native_hole_to_hole_mm = board.GetDesignSettings().m_HoleToHoleMin/1e6
    placement_policy = read(args.placement_policy) if args.placement_policy else None
    connection_rules = None
    if args.source_board or args.adaptive_config or args.freerouting_config:
        # Read class assignments in a separate process: loading the same
        # project twice has invalidated existing pcbnew objects in this binding.
        code = 'import sys,json; from pathlib import Path; sys.path.insert(0,sys.argv[1]); from compile_net_classes import compile_rules; rules,evidence=compile_rules(Path(sys.argv[2])); Path(sys.argv[3]).write_text(json.dumps(evidence,indent=2)+chr(10))'
        result=command([sys.executable,'-c',code,ROOT/'experiments/whole-board',source_board,out/'source-rules.json'],out/'source-rules.log')
        assert result['exit_code']==0, 'Source rule compilation failed; see source-rules.log'
        connection_rules=read(out/'source-rules.json')['connection_rules']
    try:
        base, mapping, bounds = extract(board,placement_policy,connection_rules)
    except Exception as error:
        write(out/'report.json',{'status':'unsupported_input','source':str(source_board),
              'source_sha256':source_hash,'error':str(error),'scope':__doc__})
        raise
    write(out/'source-problem.json',base)
    reference_poses = []
    for fp in board.GetFootprints():
        if mapping[fp.GetReference()].get('placement_role') == 'preserved_board_graphic':
            continue
        center = mapping[fp.GetReference()]['source_center']
        reference_poses.append({'component':fp.GetReference(),
            'position':vec(center['x']-bounds[0],center['y']-bounds[1]),
            'rotation_degrees':0})
    reference_control = render(base,reference_poses,out/'human-envelope-control.svg',
                               'Human placement: native courtyards' if placement_policy else 'Human placement: conservative envelopes')
    if placement_policy is not None:
        reference_problem=copy.deepcopy(base)
        reference_by={p['component']:p for p in reference_poses}
        for c in reference_problem['components']:
            c['position']=reference_by[c['id']]['position']
            c['constraints']['movement']='fixed'
        write(out/'human-control-problem.json',reference_problem)
        write(out/'human-control-config.json',{'policy':{'kind':'declared'},'projection_sweeps':16})
        control_process=command([exe,'place',out/'human-control-problem.json',out/'human-control-config.json',out/'human-control-placement.json'],out/'human-control.log')
        reference_control['production_process']=control_process
        reference_control['passed'] &= control_process['exit_code']==0
    write(out/'human-envelope-control.json',reference_control)
    write(out/'mapping.json', mapping)
    report = {'schema_version':1, 'scope':__doc__, 'source':str(source_board),'board_id':board_id,
              'source_sha256':source_hash,'components':len(base['components']),
              'electrical_nets':len(base['electrical_nets']),
              'executable_sha256':digest(exe), 'seed':args.seed,
              'placement_policy':placement_policy,
              'placement_seed':args.placement_seed,
              'reference_area_mm2':(bounds[2]-bounds[0])*(bounds[3]-bounds[1]),
              'reference_envelope_control':reference_control,
              'benchmark_status':('geometric diagnostic; product constraints not modeled' if reference_control['passed']
                                  else 'selected placement policy excludes the human reference; report this independently from native validity of new layouts'),
              'attempts':[], 'best_admitted_area_ratio':None}
    for i, ratio in enumerate(args.ratios):
        trial = out/f'area-{i:02d}'
        trial.mkdir()
        problem = copy.deepcopy(base)
        w = base['board']['bounds']['max']['x']*math.sqrt(ratio)
        h = base['board']['bounds']['max']['y']*math.sqrt(ratio)
        problem['board']['bounds']['max'] = vec(w,h)
        rng = random.Random(args.seed)
        for c in problem['components']:
            if c['constraints']['movement'] == 'fixed':
                continue
            c['position'] = vec(rng.uniform(c['size']['x']/2,w-c['size']['x']/2),
                                rng.uniform(c['size']['y']/2,h-c['size']['y']/2))
        if args.placement_seed=='spectral_rank':
            from spectral_placement_seed import seed
            problem,seed_evidence=seed(problem,'rank')
            write(trial/'seed-evidence.json',seed_evidence)
        config = {'policy':{'kind':'harmonic_ports','iterations':128,
                           'legalization_sweeps':args.legalization_sweeps,'maximum_pair_checks':args.placement_pair_checks,
                           'connected_pair_spacing_floor':placement_policy is None},
                  'seed':args.seed,'projection_sweeps':1024}
        if args.fixed_obstacle_seed_projection or args.largest_first_seed_insertion or args.skip_collision_intervals:
            config['policy']['fixed_obstacle_seed_projection']=True
        if args.skip_collision_intervals:
            config['policy']['skip_collision_intervals']=True
        if args.largest_first_seed_insertion:
            config['policy']['largest_first_seed_insertion']=True
        if args.seed_extra_pad_gap_mm:
            config['policy']['seed_extra_pad_gap_mm']=args.seed_extra_pad_gap_mm
        if placement_policy is not None and args.placement_attempts > 1:
            config['policy']['underanchored_seed']={'kind':'random','attempts':args.placement_attempts}
        if args.coupled_legalization:
            config['policy']['coupled_legalization']={}
            if args.contact_branch_trials:
                config['policy']['coupled_legalization']['maximum_branch_trials']=args.contact_branch_trials
        write(trial/'problem.json',problem)
        write(trial/'placement-config.json',config)
        initial = [{'component':c['id'],'position':c['position'],'rotation_degrees':0} for c in problem['components']]
        render(problem,initial,trial/'input.svg',f'Cold {args.placement_seed} positions; area ratio {ratio}')
        if args.trace_placement:
            from trace_placement import render as render_trace, summarize as summarize_trace
            trace_directory=trial/'trace';trace_directory.mkdir()
            result=command([exe,'trace-placement',trial/'problem.json',trial/'placement-config.json',trace_directory/'trace.json'],trial/'place.log')
            trace=read(trace_directory/'trace.json')
            write(trace_directory/'summary.json',summarize_trace(trace))
            if trace['frames']:render_trace(problem,trace,trace_directory)
            if not result['exit_code']:
                assert trace['result'] is not None and trace['error'] is None
                write(trial/'placement.json',trace['result'])
        else:
            result = command([exe,'place',trial/'problem.json',trial/'placement-config.json',trial/'placement.json'],trial/'place.log')
        attempt = {'area_ratio':ratio,'area_mm2':w*h,'directory':str(trial),
                   'placement_process':result,'admitted':False,'status':'placement rejected'}
        report['attempts'].append(attempt)
        write(out/'report.json',report)
        if result['exit_code']:
            print(f'{ratio}: placement rejected',flush=True)
            continue
        placed = read(trial/'placement.json')
        attempt['placement_audit'] = render(problem,placed['poses'],trial/'placed.svg',f'Proposed placement; area ratio {ratio}; not yet routed')
        assert attempt['placement_audit']['passed']
        attempt['status'] = 'legal placement; routing not run'
        write(out/'report.json',report)
        if not args.route and not args.materialize_only:
            continue
        native = trial/'native'
        shutil.copytree(source,native)
        for filename in ('verification.json','erc.json','drc.json'):
            (native/filename).unlink(missing_ok=True)
        assert command([exe,'strip-kicad-copper',source_board,native/source_board.name],trial/'strip.log')['exit_code'] == 0
        attempt['status']='materializing native placement';write(out/'report.json',report)
        write(trial/'materialize-request.json',{'board':str(native/source_board.name),
            'poses':placed['poses'],'mapping':mapping,'bounds':bounds,'ratio':ratio,
            'outline':placement_policy.get('outline','scaled_source') if placement_policy else 'scaled_source'})
        materialized=command([sys.executable,ROOT/'experiments/whole-board/materialize_native_placement.py',
            trial/'materialize-request.json'],trial/'materialize.log')
        attempt['materialize_process']=materialized
        assert materialized['exit_code']==0, 'Native materialization failed; see materialize.log'
        shutil.copy2(source_board.with_suffix('.kicad_pro'),(native/source_board.name).with_suffix('.kicad_pro'))
        assert command([exe,'normalize-kicad-footprint-links',native,board_id],trial/'normalize.log')['exit_code'] == 0
        attempt['status']='verifying native placement';write(out/'report.json',report)
        command([exe,'verify-kicad-rung',native,board_id],trial/'native-placement.log')
        attempt['native_inventory_audit']=audit_native(source_board,native/source_board.name,placed['poses'],mapping,bounds,ratio,trial/'native-inventory-audit.json')
        verification = read(native/'verification.json')
        attempt['native_placement'] = verification
        if args.repair_silkscreen:
            attempt['status']='repairing native annotations';write(out/'report.json',report)
            annotated=trial/'native-silk'
            attempt['silkscreen_repair_process']=command([exe,'repair-kicad-silkscreen',native,board_id,annotated],trial/'silkscreen.log')
            if attempt['silkscreen_repair_process']['exit_code'] and not args.route_with_annotation_findings:
                attempt['status']='silkscreen repair rejected'
                write(out/'report.json',report)
                continue
            attempt['silkscreen_repair']=read(annotated/'silkscreen-repair.json')
            attempt['silkscreen_native_inventory_audit']=audit_native(source_board,annotated/source_board.name,placed['poses'],mapping,bounds,ratio,trial/'silkscreen-native-inventory-audit.json')
            native=annotated
            verification=read(native/'verification.json')
            attempt['native_placement_after_silkscreen']=verification
        # Route diagnostics may proceed with annotation-only findings, but a
        # changed placement may not grandfather those findings for area credit.
        attempt['native_placement_clean']=not any(verification[k] for k in ('erc_violations','drc_design_violations','schematic_parity_issues'))
        drc=read(native/'drc.json')
        # Match pcb-kicad's shared native policy: edited library footprints are
        # retained metadata warnings, not newly introduced physical findings.
        design=[v for v in drc['violations'] if not (v['type']=='lib_footprint_mismatch' and v['severity']=='warning')]
        assert len(design)==verification['drc_design_violations']
        physical=[v for v in design if v['type'] not in ('silk_overlap','silk_over_copper','silk_edge_clearance')]
        attempt['annotation_findings']=len(design)-len(physical)
        if physical or verification['erc_violations'] or verification['schematic_parity_issues']:
            attempt['status'] = 'native placement rejected'
            write(out/'report.json',report)
            continue
        if args.materialize_only:
            attempt['status']='native placement has no physical findings; routing not run'
            attempt['routing_tested']=False
            write(out/'report.json',report)
            continue
        if args.adaptive_config or args.freerouting_config:
            if not attempt['native_placement_clean'] and not args.route_with_annotation_findings:
                attempt['status'] = 'native placement rejected before routing'
                write(out/'report.json',report)
                continue
            attempt['status'] = 'routing in progress'
            write(out/'report.json',report)
            if args.freerouting_config:
                attempt['routing_backend'] = 'freerouting'
                attempt['routing_process'] = command([exe,'route-kicad-board-freerouting',native,board_id,
                    trial/'freerouting',args.freerouting_config.resolve()],trial/'freerouting.log')
                routed = read_external_routing(trial/'freerouting')
                attempt['external_routing_report'] = routed['external_report']
                attempt['external_final_selection'] = routed['final_selection']
                if routed['routing_result']:
                    attempt['routing_result'] = routed['routing_result']
            else:
                adaptive = read(args.adaptive_config)
                sequential = adaptive.setdefault('sequential',{})
                sequential['maximum_connections'] = len(base['electrical_nets'])
                sequential.setdefault('routing_portfolio',[{}])
                for routing in sequential['routing_portfolio']:
                    routing['connection_rules'] = connection_rules
                    routing['edge_clearance_mm'] = native_edge_clearance_mm
                    routing['hole_to_hole_clearance_mm'] = native_hole_to_hole_mm
                adaptive['allow_existing_annotation_findings'] = args.route_with_annotation_findings
                write(trial/'adaptive-config.json',adaptive)
                attempt['routing_process'] = command([exe,'route-kicad-board-adaptive',native,board_id,
                    trial/'adaptive',trial/'adaptive-config.json'],trial/'adaptive.log')
                result_file = trial/'adaptive/adaptive-routing.json'
                routed = read(result_file) if result_file.exists() else None
                if routed:
                    attempt['routing_result'] = {'native':routed['result_verification'],
                        'statistics':routed['result_statistics'],'directory':routed['result_directory']}
            attempt['electrically_connected'] = bool(routed and routed.get('routing_complete'))
            attempt['pcb_maker_solved'] = bool(routed and routed.get('complete'))
            attempt['admitted'] = bool(attempt['routing_process']['exit_code']==0 and attempt['pcb_maker_solved'])
            attempt['status'] = ('native-complete layout' if attempt['admitted'] else
                'routing connected; outstanding native findings' if attempt['electrically_connected'] else
                'native routing incomplete')
            if attempt['admitted']:
                report['best_admitted_area_ratio'] = min(ratio,report['best_admitted_area_ratio'] or ratio)
            write(out/'report.json',report)
            print(f'{board_id} ratio {ratio}: {attempt["status"]}',flush=True)
            continue
        route_config = read(ROOT/'benchmarks/real/ecc83-pp/freerouting.json')
        route_config['source']['directory'] = str(native)
        route_config['journal_directory'] = str(out/'progress')
        route_config['freerouting']['maximum_seconds'] = args.router_seconds
        write(trial/'freerouting.json',route_config)
        comparison = read(ROOT/'benchmarks/real/ecc83-pp/comparison.json')
        comparison['pcb_maker']['sequential_config'] = str(ROOT/'benchmarks/real/ecc83-pp/pcb-maker-sequential.json')
        comparison['pcb_maker']['verification_cache_directory'] = str(out/'verification-cache')
        comparison['pcb_maker']['maximum_seconds'] = args.router_seconds
        write(trial/'comparison.json',comparison)
        attempt['status']='routing in progress'
        write(out/'report.json',report)
        attempt['routing_process'] = command([exe,'benchmark-route-compare',trial/'comparison.json',trial/'comparison'],trial/'comparison.log')
        paired = read(trial/'comparison/competitive-comparison.json')
        attempt['pcb_maker_solved'] = paired['pcb_maker_solved']
        attempt['freerouting_solved'] = paired['freerouting_solved']
        attempt['admitted'] = (reference_control['passed'] and attempt['native_placement_clean'] and paired['status']=='finished' and paired['fixed_placement_matches']
                               and paired['rules_match'] and paired['pcb_maker_solved'])
        attempt['status'] = 'routed comparison complete'
        if not attempt['native_placement_clean']:
            attempt['status'] += '; silkscreen cleanup required before area credit'
        if attempt['admitted']:
            report['best_admitted_area_ratio'] = min(ratio, report['best_admitted_area_ratio'] or 1.0)
        write(out/'report.json',report)
        print(f'{ratio}: {attempt["status"]}, admitted={attempt["admitted"]}',flush=True)
    assert digest(source_board) == report['source_sha256']
    report['finished'] = True
    write(out/'report.json',report)
    write_index(out,report)


def read_external_routing(output):
    """Read the published incumbent without overwriting independent router evidence."""
    external_path = output/'routing/report.json'
    raw = read(external_path) if external_path.exists() else None
    selection_path = output/'final-selection.json'
    selection = read(selection_path) if selection_path.exists() else None
    if selection_path.exists():
        # A published null/rejected selection must not fall back to stale success.
        selection = selection if isinstance(selection, dict) else {}
        native = selection.get('selected_native')
        directory = selection.get('selected_directory')
        native = native if isinstance(native, dict) and isinstance(directory, str) else None
        external_selected = (selection.get('selected_stage') == 'external' and
            directory is not None and Path(directory).resolve() == (output/'routing/result').resolve())
        result = {'native':native, 'directory':directory,
            'render':selection.get('selected_render'),
            'statistics':raw.get('statistics') if external_selected and raw else None,
            'statistics_scope':'selected external board' if external_selected else 'unavailable for selected board'}
        complete = bool(selection.get('complete') is True and selection.get('status') == 'complete' and
                        native and native.get('complete') is True)
    else:
        native = raw.get('native') if raw else None
        result = {'native':native, 'directory':str(output/'routing/result'),
            'render':str(output/'routing/result/inspection-combined.svg'),
            'statistics':raw.get('statistics') if raw else None}
        complete = bool(raw and raw.get('complete') is True and native and native.get('complete') is True)
    opens = native.get('selected_net_unconnected_items') if native else None
    connected = type(opens) is int and opens == 0
    return {'routing_result':result if native else None,
            'routing_complete':connected,
            'complete':complete and connected, 'external_report':raw, 'final_selection':selection}


def write_index(out,report):
    links = ['<!doctype html><meta charset="utf-8"><title>Cold placement area probe</title>',
             '<h1>Cold placement area probe</h1><p>Movement follows the explicit placement policy. Original orientations retained. Full native routing and clean placement are required for area credit.</p>',
             '<p>' + report['benchmark_status'] + '</p>',
             '<p><a href="human-envelope-control.svg">Human placement envelope control</a></p>']
    for a in report['attempts']:
        directory = Path(a['directory'])
        links.append(f'<h2>Area ratio {a["area_ratio"]}: {a["status"]}</h2>')
        selected_render = (a.get('routing_result') or {}).get('render')
        if selected_render and Path(selected_render).is_file():
            import html
            import os
            link = html.escape(os.path.relpath(selected_render, out), quote=True)
            links.append(f'<p><a href="{link}">Final selected routing result</a></p><img style="width:750px" src="{link}">')
        for name in ('input.svg','placed-with-pads.svg','placed.svg','native/preview.svg','native-silk/preview.svg',
                     'adaptive/result/preview.svg','freerouting/routing/result/inspection-combined.svg','diagnostic-routing/result/preview.svg','comparison/result/preview.svg','comparison/pcb-maker/result/preview.svg'):
            if selected_render and name == 'freerouting/routing/result/inspection-combined.svg':
                continue
            if (directory/name).exists():
                links.append(f'<a href="{directory.name}/{name}">{name}</a><br><img style="width:750px" src="{directory.name}/{name}">')
        if (directory/'freerouting/routing/report.json').exists():
            links.append(f'<p><a href="{directory.name}/freerouting/routing/report.json">Raw external native routing report</a></p>')
        if (directory/'comparison/competitive-comparison.json').exists():
            links.append(f'<p><a href="{directory.name}/comparison/competitive-comparison.json">Router comparison</a></p>')
    links.append('<p><a href="report.json">Full evidence</a></p>')
    (out/'index.html').write_text('\n'.join(links))



if __name__ == '__main__':
    main()
