#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Named copper-clearance witnesses for proposed native straight tracks.

Diagnostic geometry only. Native DRC/connectivity remains the acceptance gate.
"""
import argparse
import json
import math
from pathlib import Path
from inspect_board import point_segment_distance,rectangle_distance,segment_distance


def pad_distance(a,b,pad):
    angle=math.radians(pad['rotation_degrees'])
    x,y=pad['at']
    ox,oy=pad.get('offset',[0,0])
    def local(p):
        dx,dy=p[0]-x,p[1]-y
        return [dx*math.cos(angle)-dy*math.sin(angle)-ox,
                dx*math.sin(angle)+dy*math.cos(angle)-oy]
    a,b=local(a),local(b)
    w,h=pad['size']
    shape=pad['shape']
    if shape=='circle':
        return max(0,point_segment_distance([0,0],a,b)-w/2)
    if shape=='rect':
        return rectangle_distance(a,b,[-w/2,-h/2,w/2,h/2])
    if shape in ['oval','roundrect']:
        radius=min(w,h)/2 if shape=='oval' else pad['roundrect_radius']
        return max(0,rectangle_distance(a,b,[-w/2+radius,-h/2+radius,w/2-radius,h/2-radius])-radius)
    raise ValueError('Unsupported pad shape '+shape)


def board_floor(board):
    return board.get('effective_board_rules',{}).get('min_clearance',{}).get('value',(board.get('board_rules') or {}).get('min_clearance',0))


def clearance(board,net):
    if board.get('effective_netclasses'):
        name=board['netclasses_by_net'].get(net,'Default')
        value=board['effective_netclasses'][name]['clearance']
    else:
        classes=board['net_settings']['classes']
        if len(classes)!=1:
            raise ValueError('Need resolved per-net rules for multiple netclasses')
        value=classes[0]['clearance']
    return max(value,board_floor(board))


def pad_clearance(board,pad,layer):
    record=pad.get('own_clearance_by_layer',{}).get(layer)
    if record is None:
        return clearance(board,pad['net']),'pad netclass fallback; resolved own clearance not exported'
    value=record['value_mm']
    if not isinstance(value,(int,float)) or not math.isfinite(value) or value<0:
        raise ValueError('Invalid resolved native pad own clearance')
    return max(value,board_floor(board)),record['source']


def check_tracks(board,proposal):
    if board.get('zones'):
        raise ValueError('Zone geometry is not supported by this diagnostic')
    if any(not area.get('is_rule_area') or area.get('do_not_allow_tracks') or area.get('do_not_allow_vias')
           for area in board.get('rule_areas',[])):
        raise ValueError('Only non-copper rule-area metadata allowing tracks/vias is supported')
    removed=set(proposal.get('remove_tracks',[])+proposal.get('remove_vias',[]))
    tracks=[t for t in board['tracks'] if t['id'] not in removed]
    new=[dict(t,id=f'new:{i}') for i,t in enumerate(proposal.get('add_tracks',[]))]
    all_tracks=tracks+new
    vias=[v for v in board['vias'] if v['id'] not in removed]
    results=[]
    for track in new:
        a,b=track['start'],track['end']
        net,layer=track['net'],track['layer']
        witnesses=[]
        def add(obj,kind,gap):
            selected_clearance=clearance(board,net)
            if kind=='pad':
                other_clearance,source=pad_clearance(board,obj,layer)
            else:
                other_clearance,source=clearance(board,obj['net']),'netclass and board minimum'
            required=max(selected_clearance,other_clearance)
            witness=dict(object_id=obj['id'],kind=kind,net=obj['net'],copper_gap_mm=gap,
                         required_mm=required,margin_mm=gap-required)
            if kind=='pad':
                witness['clearance_basis']=dict(proposed_track_netclass_and_floor_mm=selected_clearance,
                    pad_own_and_floor_mm=other_clearance,pad_source=source,board_floor_mm=board_floor(board))
            witnesses.append(witness)
        for pad in board['pads']:
            if pad['net']!=net and layer in pad['layers']:
                add(pad,'pad',pad_distance(a,b,pad)-track['width']/2)
        for other in all_tracks:
            if other['net']!=net and other['layer']==layer:
                add(other,'track',segment_distance(a,b,other['start'],other['end'])-(track['width']+other['width'])/2)
        for via in vias:
            if via['net']!=net and layer in via['layers']:
                add(via,'via',point_segment_distance(via['at'],a,b)-(track['width']+via['diameter'])/2)
        witnesses.sort(key=lambda w:w['margin_mm'])
        results.append(dict(track=track,blockers=[w for w in witnesses if w['margin_mm'] < -1e-7],nearest=witnesses[:4]))
    return dict(scope='Pad-outline and copper-clearance diagnostic for added straight tracks only; resolved pad own rules when exported, netclass fallback otherwise. No connectivity, dangling-object, general hole/hole-pair, custom-pair, board-edge or manufacturing-rule admission.',
                tracks=results,blocked_tracks=sum(bool(r['blockers']) for r in results))


def check_batch(board,candidates,compact=False):
    """Inspect explicit candidates in input order; no generation or selection."""
    seen=set()
    results=[]
    for candidate in candidates:
        candidate_id=candidate['id']
        if not isinstance(candidate_id,str) or candidate_id in seen:
            raise ValueError('Batch candidate IDs must be unique strings')
        seen.add(candidate_id)
        result=check_tracks(board,candidate['proposal'])
        if compact:
            nearest=[w for r in result['tracks'] for w in r['nearest']]
            result=dict(blocked_tracks=result['blocked_tracks'],
                        minimum_margin_mm=min((w['margin_mm'] for w in nearest),default=None),
                        blockers=[dict(track_id=r['track']['id'],**w) for r in result['tracks'] for w in r['blockers']])
        results.append(dict(id=candidate_id,assessment=result))
    return dict(scope='Independent proposed-candidate copper queries; candidates are not applied together, optimized, ranked, or admitted. Each proposal contains its own simultaneous removals/additions. No connectivity, drill or board-edge check.',
                candidates=results)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('board')
    p.add_argument('proposal',nargs='?')
    p.add_argument('--batch',help='JSON array of {id, proposal}; explicit candidates only')
    p.add_argument('--compact',action='store_true',help='Return blockers and minimum margin per batch candidate')
    args=p.parse_args()
    if bool(args.proposal)==bool(args.batch):
        p.error('Supply exactly one proposal file or --batch FILE')
    if args.compact and not args.batch:
        p.error('--compact requires --batch')
    board=json.loads(Path(args.board).read_text())
    if args.batch:
        result=check_batch(board,json.loads(Path(args.batch).read_text()),args.compact)
    else:
        result=check_tracks(board,json.loads(Path(args.proposal).read_text()))
    print(json.dumps(result,indent=2))


if __name__=='__main__':
    main()
