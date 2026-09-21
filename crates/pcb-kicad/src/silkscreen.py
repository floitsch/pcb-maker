# Copyright (C) 2026 Toit contributors.
"""Bounded front-silkscreen repair using native text metrics.

Straight strokes are clipped to the rectangular board and separated from
other footprints' strokes. Shorter strokes have priority, retaining small
marks ahead of long outlines. Isolated small centered crosses are protected
as rigid groups and moved locally before clipping other lines. Recognition is
geometric, not an inference of electrical polarity. Visible reference fields are moved locally
without changing their text, size or pen width. Curves remain unchanged.
Only selected F.SilkS records are patched into the original board text.
Native verification is required after this provisional operation.
"""
import hashlib
import json
import math
from pathlib import Path
import re
import shutil
import sys
import tempfile
import uuid

import pcbnew
if not hasattr(pcbnew.SwigPyIterator, 'next'):
    pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__

MARGIN = 0.10
EDGE_MARGIN = 0.15
SAMPLE = 0.025


def xy(p): return (p.x/1e6, p.y/1e6)
def vector(p): return pcbnew.VECTOR2I(round(p[0]*1e6), round(p[1]*1e6))
def key(item): return item.m_Uuid.AsString()
def box(item):
    b=item.GetBoundingBox()
    return (b.GetX()/1e6,b.GetY()/1e6,(b.GetX()+b.GetWidth())/1e6,(b.GetY()+b.GetHeight())/1e6)
def inside(p,b): return b[0]<=p[0]<=b[2] and b[1]<=p[1]<=b[3]
def expanded(b,d): return (b[0]-d,b[1]-d,b[2]+d,b[3]+d)
def rect_overlap(a,b): return a[0]<b[2] and b[0]<a[2] and a[1]<b[3] and b[1]<a[3]
def corners(b): return [(b[0],b[1]),(b[2],b[1]),(b[2],b[3]),(b[0],b[3])]
def point_segment(p,a,b):
    dx,dy=b[0]-a[0],b[1]-a[1];d=dx*dx+dy*dy
    t=max(0,min(1,((p[0]-a[0])*dx+(p[1]-a[1])*dy)/d)) if d else 0
    return math.hypot(p[0]-a[0]-t*dx,p[1]-a[1]-t*dy)
def point_rect(p,b): return math.hypot(max(b[0]-p[0],0,p[0]-b[2]),max(b[1]-p[1],0,p[1]-b[3]))


def rectangular_outline_bounds(board):
    """Recognize exact side coverage in native integer units; never edit edges."""
    edges=[s for s in board.GetDrawings() if s.GetLayer()==pcbnew.Edge_Cuts]
    assert len(edges)>=4 and all(s.GetShape()==pcbnew.SHAPE_T_SEGMENT for s in edges),'Rectangular board required'
    segments=[((s.GetStart().x,s.GetStart().y),(s.GetEnd().x,s.GetEnd().y)) for s in edges]
    points=[p for segment in segments for p in segment]
    x0,y0=min(p[0] for p in points),min(p[1] for p in points)
    x1,y1=max(p[0] for p in points),max(p[1] for p in points)
    assert x0<x1 and y0<y1,'Degenerate board outline'
    sides=[[],[],[],[]]
    for a,b in segments:
        assert a!=b,'Zero-length board edge'
        if a[1]==b[1] and a[1] in (y0,y1):
            sides[0 if a[1]==y0 else 1].append(tuple(sorted((a[0],b[0]))))
        elif a[0]==b[0] and a[0] in (x0,x1):
            sides[2 if a[0]==x0 else 3].append(tuple(sorted((a[1],b[1]))))
        else:
            raise AssertionError('Nonrectangular board edge')
    # Each side must be tiled exactly once. Integer equality rejects even a
    # one-unit gap/overlap; no tolerance can turn a nearly closed contour legal.
    for intervals,(start,end) in zip(sides,[(x0,x1),(x0,x1),(y0,y1),(y0,y1)]):
        cursor=start
        for lo,hi in sorted(intervals):
            assert lo==cursor,'Gap or overlap in board edge'
            cursor=hi
        assert cursor==end,'Incomplete board edge'
    return x0/1e6,y0/1e6,x1/1e6,y1/1e6


def line_interval(a,b,bounds):
    lo,hi=0.0,1.0
    for i in range(2):
        d=b[i]-a[i]
        if abs(d)<1e-12:
            if a[i]<bounds[i] or a[i]>bounds[i+2]:return None
        else:
            u,v=(bounds[i]-a[i])/d,(bounds[i+2]-a[i])/d
            lo=max(lo,min(u,v));hi=min(hi,max(u,v))
            if lo>=hi:return None
    return lo,hi


def shape_record(item,owner):
    shape=item.GetShape()
    if shape==pcbnew.SHAPE_T_SEGMENT:
        return {'kind':'line','a':xy(item.GetStart()),'b':xy(item.GetEnd()),'radius':item.GetWidth()/2e6,'owner':owner}
    if shape==pcbnew.SHAPE_T_CIRCLE:
        a,b=xy(item.GetStart()),xy(item.GetEnd())
        return {'kind':'circle','center':a,'r':math.dist(a,b),'radius':item.GetWidth()/2e6,'owner':owner}
    return {'kind':'box','box':box(item),'radius':0,'owner':owner}


def distance_to_stroke(p,s):
    if s['kind']=='line':return point_segment(p,s['a'],s['b'])-s['radius']
    if s['kind']=='circle':return abs(math.dist(p,s['center'])-s['r'])-s['radius']
    return point_rect(p,s['box'])


def hits_stroke(b,s):
    b=expanded(b,MARGIN+s['radius'])
    if s['kind']=='line':
        return inside(s['a'],b) or inside(s['b'],b) or line_interval(s['a'],s['b'],b) is not None
    if s['kind']=='circle':
        return point_rect(s['center'],b)<=s['r']<=max(math.dist(s['center'],p) for p in corners(b))
    return rect_overlap(b,s['box'])


def silk_nodes(text):
    """Locate editable complete records without reserializing unrelated data."""
    tokens=re.finditer(r'"(?:\\.|[^"\\])*"|[()]|[^\s()"]+',text)
    stack=[];nodes={}
    for token in tokens:
        value=token.group()
        if value=='(':stack.append([token.start(),None])
        elif value==')':
            assert stack,'Unbalanced board'
            start,head=stack.pop()
            if head not in ('fp_line','fp_circle','fp_arc','fp_poly','fp_rect','fp_text','property'):continue
            fragment=text[start:token.end()]
            if not re.search(r'\(layer\s+"F\.SilkS"\)',fragment):continue
            identity=re.search(r'\(uuid\s+"?([0-9a-f-]{36})"?\)',fragment)
            assert identity,'Silk record without UUID'
            identity=identity.group(1);assert identity not in nodes
            nodes[identity]=(start,token.end(),fragment)
        elif stack and stack[-1][1] is None:stack[-1][1]=value
    assert not stack,'Unbalanced board'
    return nodes


def centered_cross(a,b):
    """Conservative, rotation-independent recognition of a two-stroke mark."""
    if a['kind']!='line' or b['kind']!='line':return False
    u=tuple(a['b'][i]-a['a'][i] for i in range(2))
    v=tuple(b['b'][i]-b['a'][i] for i in range(2))
    la,lb=math.hypot(*u),math.hypot(*v)
    if not (.3<=la<=2 and .3<=lb<=2):return False
    if abs(la-lb)>.02 or abs(a['radius']-b['radius'])>1e-6:return False
    if abs(sum(u[i]*v[i] for i in range(2)))>la*lb*.01:return False
    ca=tuple((a['a'][i]+a['b'][i])/2 for i in range(2))
    cb=tuple((b['a'][i]+b['b'][i])/2 for i in range(2))
    return math.dist(ca,cb)<=.01


def line_near_stroke(line,stroke,clearance):
    a,b=line['a'],line['b'];length=math.dist(a,b)
    n=max(1,math.ceil(length/SAMPLE));guard=length/(2*n)
    return any(distance_to_stroke(tuple(a[i]+(j+.5)/n*(b[i]-a[i]) for i in range(2)),stroke)
               <line['radius']+clearance+guard for j in range(n))


def cross_groups(shapes):
    records={key(s):shape_record(s,str(fp.GetReference())) for fp,s in shapes}
    groups=[];used=set()
    for fp,a in sorted(shapes,key=lambda pair:key(pair[1])):
        if key(a) in used:continue
        for other,b in sorted(shapes,key=lambda pair:key(pair[1])):
            if key(a)>=key(b) or key(b) in used or key(fp)!=key(other):continue
            ra,rb=records[key(a)],records[key(b)]
            if not centered_cross(ra,rb):continue
            # A junction of a larger drawing is not a standalone symbol.
            if any(k not in (key(a),key(b)) and r['owner']==ra['owner']
                   and (line_near_stroke(ra,r,.02) or line_near_stroke(rb,r,.02))
                   for k,r in records.items()):continue
            groups.append((fp,[a,b]));used.update((key(a),key(b)));break
    return groups


def move_cross_groups(shapes,bounds,pads,fixed_text,fixed_shapes,bodies,replacements,report):
    groups=cross_groups(shapes)
    protected={key(s) for _,group in groups for s in group}
    for fp,group in groups:
        ref=str(fp.GetReference());ids=[key(s) for s in group]
        original=[shape_record(s,ref) for s in group]
        center=tuple((original[0]['a'][i]+original[0]['b'][i])/2 for i in range(2))
        own_pads=[(key(p),xy(p.GetPosition())) for p in fp.Pads()]
        def nearest(p):return min(own_pads,key=lambda item:(math.dist(p,item[1]),item[0]))[0] if own_pads else None
        associated=nearest(center)
        obstacles=fixed_shapes+[shape_record(s,str(owner.GetReference())) for owner,s in shapes if key(s) not in ids]
        def legal(delta):
            new_center=tuple(center[i]+delta[i] for i in range(2))
            if nearest(new_center)!=associated:return False
            for record in original:
                a=tuple(record['a'][i]+delta[i] for i in range(2))
                b=tuple(record['b'][i]+delta[i] for i in range(2))
                length=math.dist(a,b);radius=record['radius'];n=max(1,math.ceil(length/SAMPLE))
                guard=length/(2*n)
                for j in range(n):
                    p=tuple(a[i]+(j+.5)/n*(b[i]-a[i]) for i in range(2))
                    if not inside(p,expanded(bounds,-EDGE_MARGIN-radius-guard)):return False
                    if any(point_rect(p,pad)<radius+guard for pad in pads):return False
                    if any(point_rect(p,text)<radius+MARGIN+guard for text in fixed_text):return False
                    if any(distance_to_stroke(p,s)<radius+MARGIN+guard for s in obstacles):return False
                    if any(body['owner']!=ref and (
                        math.dist(p,body['center'])<body['radius']+radius+MARGIN+guard if body['kind']=='circle'
                        else point_rect(p,body['box'])<radius+MARGIN+guard) for body in bodies):return False
            return True
        candidates=sorted(((dx*.1,dy*.1) for dx in range(-20,21) for dy in range(-20,21)),key=lambda p:(math.hypot(*p),p))
        selected=next((delta for delta in candidates if legal(delta)),None)
        evidence={'reference':ref,'kind':'isolated_centered_cross','uuids':ids,'center_mm':center,
                  'associated_pad_uuid':associated,'translation_mm':selected,'status':'unresolved' if selected is None else 'preserved'}
        report['protected_marks'].append(evidence)
        if selected is None:
            report['unresolved_marks'].append(ids)
            continue
        for s,record in zip(group,original):
            if selected!=(0.0,0.0):
                s.SetStart(vector(tuple(record['a'][i]+selected[i] for i in range(2))))
                s.SetEnd(vector(tuple(record['b'][i]+selected[i] for i in range(2))))
                replacements[key(s)]=[key(s)]
    return protected


def repair(source,output,evidence_path):
    assert source.resolve()!=output.resolve()
    original=source.read_text();original_nodes=silk_nodes(original)
    output.parent.mkdir(parents=True,exist_ok=True)
    report={'scope':__doc__,'source_sha256':hashlib.sha256(original.encode()).hexdigest(),
            'margin_mm':MARGIN,'edge_margin_mm':EDGE_MARGIN,'sampling_mm':SAMPLE,
            'strokes':[],'references':[],'unresolved_references':[],
            'protected_marks':[],'unresolved_marks':[]}
    replacements={}
    with tempfile.TemporaryDirectory(prefix='pcb-maker-silk-',dir=output.parent) as scratch:
        temporary=Path(scratch)/source.name;shutil.copy2(source,temporary)
        project=source.with_suffix('.kicad_pro')
        if project.exists():shutil.copy2(project,temporary.with_suffix('.kicad_pro'))
        board=pcbnew.LoadBoard(str(temporary))
        bounds=rectangular_outline_bounds(board)
        shapes=[];fields=[];pads=[];fixed_text=[];bodies=[]
        for fp in board.GetFootprints():
            ref=str(fp.GetReference())
            courtyard=[s for s in fp.GraphicalItems() if isinstance(s,pcbnew.PCB_SHAPE) and s.GetLayer()==pcbnew.F_CrtYd]
            if len(courtyard)==1 and courtyard[0].GetShape()==pcbnew.SHAPE_T_CIRCLE:
                s=courtyard[0];center=xy(s.GetStart())
                bodies.append({'owner':ref,'kind':'circle','center':center,'radius':math.dist(center,xy(s.GetEnd()))})
            elif courtyard:
                boxes=[box(s) for s in courtyard]
                bodies.append({'owner':ref,'kind':'box','box':(min(b[0] for b in boxes),min(b[1] for b in boxes),max(b[2] for b in boxes),max(b[3] for b in boxes))})
            for item in fp.GraphicalItems():
                if item.GetLayer()==pcbnew.F_SilkS:
                    if isinstance(item,pcbnew.PCB_SHAPE):shapes.append((fp,item))
                    elif hasattr(item,'IsVisible') and item.IsVisible():fixed_text.append(box(item))
            for field in fp.GetFields():
                if field.GetLayer()==pcbnew.F_SilkS and field.IsVisible():
                    if key(field)==key(fp.Reference()):fields.append((fp,field))
                    else:fixed_text.append(box(field))
            for pad in fp.Pads():
                if pad.IsOnLayer(pcbnew.F_Mask):pads.append(expanded(box(pad),max(0,pad.GetSolderMaskExpansion(pcbnew.F_Mask)/1e6)+MARGIN))
        for track in board.GetTracks():
            if isinstance(track,pcbnew.PCB_VIA):pads.append(expanded(box(track),MARGIN))
        # Unrecognized board-level silk remains an obstacle, not an editable record.
        fixed_shapes=[shape_record(s,'board') for s in board.GetDrawings() if s.GetLayer()==pcbnew.F_SilkS and isinstance(s,pcbnew.PCB_SHAPE)]
        protected=move_cross_groups(shapes,bounds,pads,fixed_text,fixed_shapes,bodies,replacements,report)
        occupied=fixed_shapes+[dict(shape_record(s,str(fp.GetReference())),protected=key(s) in protected)
                              for fp,s in shapes if s.GetShape()!=pcbnew.SHAPE_T_SEGMENT or key(s) in protected]
        lines=sorted([(fp,s) for fp,s in shapes if s.GetShape()==pcbnew.SHAPE_T_SEGMENT and key(s) not in protected],key=lambda x:(math.dist(xy(x[1].GetStart()),xy(x[1].GetEnd())),key(x[1])))
        for fp,item in lines:
            ref=str(fp.GetReference());a,b=xy(item.GetStart()),xy(item.GetEnd());length=math.dist(a,b);radius=item.GetWidth()/2e6
            n=max(1,math.ceil(length/SAMPLE));chunks=[];start=None
            def at(t):return (a[0]+t*(b[0]-a[0]),a[1]+t*(b[1]-a[1]))
            # Each interval is kept only if its midpoint has an extra half-step
            # of distance. The distance functions are 1-Lipschitz, so the full
            # interval is safe; this also avoids missing narrow intersections.
            allowed_bounds=expanded(bounds,-EDGE_MARGIN-radius)
            for i in range(n):
                p=at((i+.5)/n);guard=length/(2*n)
                safe=inside(p,expanded(allowed_bounds,-guard)) and all(point_rect(p,pad)>=radius+guard for pad in pads) and all(
                    (s['owner']==ref and not s.get('protected')) or distance_to_stroke(p,s)>=radius+MARGIN+guard for s in occupied)
                if safe and start is None:start=i/n
                if not safe and start is not None:chunks.append((start,i/n));start=None
            if start is not None:chunks.append((start,1.0))
            chunks=[(u,v) for u,v in chunks if (v-u)*length>=0.10]
            if chunks==[(0.0,1.0)]:
                occupied.append(shape_record(item,ref));continue
            identity=key(item);new_ids=[]
            for j,(u,v) in enumerate(chunks):
                piece=item.Duplicate()
                assert piece.GetLayer()==pcbnew.F_SilkS and piece.GetWidth()==item.GetWidth()
                new_id=str(uuid.uuid5(uuid.NAMESPACE_URL,f'pcb-maker:silk:{identity}:{j}:{u:.9f}:{v:.9f}'))
                piece.SetUuidDirect(pcbnew.KIID(new_id));piece.SetStart(vector(at(u)));piece.SetEnd(vector(at(v)))
                fp.Add(piece);piece.thisown=False
                new_ids.append(new_id);occupied.append(shape_record(piece,ref))
            fp.Remove(item);replacements[identity]=new_ids
            report['strokes'].append({'uuid':identity,'reference':ref,'original_length_mm':length,
                'retained_length_mm':sum((v-u)*length for u,v in chunks),'pieces':new_ids})
        all_strokes=occupied
        field_boxes={key(f):box(f) for _,f in fields}
        for fp,field in sorted(fields,key=lambda pair:str(pair[0].GetReference())):
            identity=key(field);old=xy(field.GetTextPos());angle=field.GetTextAngleDegrees();old_box=field_boxes.pop(identity)
            def legal():
                b=box(field)
                if any(body['owner']!=str(fp.GetReference()) and (
                    point_rect(body['center'],b)<body['radius']+MARGIN if body['kind']=='circle'
                    else rect_overlap(expanded(b,MARGIN),body['box'])) for body in bodies):return False
                return all(inside(p,expanded(bounds,-EDGE_MARGIN)) for p in corners(b)) and not any(
                    rect_overlap(b,obstacle) for obstacle in pads+fixed_text+[expanded(v,MARGIN) for v in field_boxes.values()]) and not any(hits_stroke(b,s) for s in all_strokes)
            if legal():field_boxes[identity]=old_box;continue
            center=xy(fp.GetPosition())
            candidates=[]
            for dx in range(-16,17):
                for dy in range(-16,17):
                    p=(old[0]+dx*.5,old[1]+dy*.5)
                    for turn in [angle,(angle+90)%180]:
                        cost=math.dist(p,old)+.1*math.dist(p,center)+(0 if turn==angle else .3)
                        candidates.append((cost,p,turn))
            selected=None
            for _,p,turn in sorted(candidates):
                field.SetTextPos(vector(p));field.SetTextAngleDegrees(turn)
                if legal():selected=(p,turn);break
            if selected is None:
                field.SetTextPos(vector(old));field.SetTextAngleDegrees(angle)
                report['unresolved_references'].append(str(fp.GetReference()))
            else:
                replacements[identity]=[identity]
                report['references'].append({'uuid':identity,'reference':str(fp.GetReference()),'from_mm':old,'to_mm':selected[0],
                                            'from_angle':angle,'to_angle':selected[1],'text_size':xy(field.GetTextSize()),'thickness_mm':field.GetTextThickness()/1e6})
            field_boxes[identity]=box(field)
        pcbnew.SaveBoard(str(temporary),board)
        generated=silk_nodes(temporary.read_text());patches=[]
        for identity,new_ids in replacements.items():
            start,end,_=original_nodes[identity]
            replacement='\n'.join(generated[k][2] for k in new_ids)
            patches.append((start,end,replacement))
        result=original
        for start,end,replacement in sorted(patches,reverse=True):result=result[:start]+replacement+result[end:]
        # All bytes outside explicitly selected complete front-silk nodes are
        # copied verbatim. No full-board SaveBoard serialization reaches output.
        cursor=0;protected=[]
        for start,end,_ in sorted(patches):protected.append(original[cursor:start]);cursor=end
        protected.append(original[cursor:])
        report['protected_chunks_sha256']=hashlib.sha256('\0'.join(protected).encode()).hexdigest()
        report['patched_records']=len(patches)
        report['reference_other_courtyard_exclusion']=True
        report['courtyard_obstacles']=len(bodies)
        report['output_sha256']=hashlib.sha256(result.encode()).hexdigest()
        output.write_text(result)
        report['native_verification_required']=True
        evidence_path.write_text(json.dumps(report,indent=2)+'\n')
    assert source.read_text()==original


if __name__=='__main__':
    repair(*(Path(p) for p in sys.argv[1:4]))
