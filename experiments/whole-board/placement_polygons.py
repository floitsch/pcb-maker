# Copyright (C) 2026 Toit contributors.
"""Simple courtyard polygons and convex decomposition; no footprint hull substitution."""
import math

EPS = 1e-9


def cross(a, b, c):
    return (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])


def area(points):
    return sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(points,points[1:]+points[:1]))/2


def edges(points):
    return list(zip(points, points[1:]+points[:1]))


def segment_distance(p, a, b):
    dx,dy = b[0]-a[0],b[1]-a[1]
    t = max(0,min(1,((p[0]-a[0])*dx+(p[1]-a[1])*dy)/(dx*dx+dy*dy)))
    return math.hypot(p[0]-a[0]-t*dx,p[1]-a[1]-t*dy)


def validate(points):
    assert len(points)>=3 and all(math.isfinite(v) for p in points for v in p), 'Courtyard requires finite vertices'
    assert len(set(map(tuple,points)))==len(points), 'Courtyard repeats a vertex'
    assert abs(area(points))>EPS, 'Courtyard has zero area'
    lines = edges(points)
    for i,(a,b) in enumerate(lines):
        assert math.dist(a,b)>EPS
        for j,(c,d) in enumerate(lines[i+1:],i+1):
            if j==i+1 or (i==0 and j==len(lines)-1):continue
            if (cross(a,b,c)*cross(a,b,d)<0 and cross(c,d,a)*cross(c,d,b)<0) or min(
                    segment_distance(c,a,b),segment_distance(d,a,b),
                    segment_distance(a,c,d),segment_distance(b,c,d))<=EPS:
                raise ValueError('Self-intersecting courtyard')


def ordered_boundary(segments):
    graph = {}
    for a,b in segments:
        assert a!=b, 'Zero-length courtyard segment'
        graph.setdefault(a,[]).append(b)
        graph.setdefault(b,[]).append(a)
    assert len(graph)>=3 and all(len(v)==2 for v in graph.values()), 'Courtyard must be a closed simple loop'
    first=min(graph);current=first;previous=None;points=[]
    while True:
        assert current not in points, 'Courtyard revisits a vertex'
        points.append(current)
        next_=next(p for p in sorted(graph[current]) if p!=previous)
        if next_==first:break
        previous,current=current,next_
    assert len(points)==len(graph), 'Multiple courtyard loops require explicit handling'
    result=[(x/1e6,y/1e6) for x,y in points]
    validate(result)
    return result


def triangulate(points):
    validate(points)
    polygon=list(points if area(points)>0 else reversed(points))
    while len(polygon)>3:
        redundant=next((i for i in range(len(polygon))
            if abs(cross(polygon[i-1],polygon[i],polygon[(i+1)%len(polygon)]))<=EPS),None)
        if redundant is None:break
        polygon.pop(redundant)
    triangles=[]
    while len(polygon)>3:
        for i,b in enumerate(polygon):
            a,c=polygon[i-1],polygon[(i+1)%len(polygon)]
            if cross(a,b,c)<=EPS:continue
            if any(all(cross(x,y,p)>=-EPS for x,y in edges([a,b,c]))
                   for j,p in enumerate(polygon) if j not in [(i-1)%len(polygon),i,(i+1)%len(polygon)]):
                continue
            triangles.append([a,b,c]);polygon.pop(i);break
        else:raise ValueError('Courtyard triangulation failed')
    triangles.append(polygon)
    assert all(area(t)>EPS for t in triangles)
    assert math.isclose(sum(area(t) for t in triangles),abs(area(points)),rel_tol=1e-10,abs_tol=1e-8)
    return triangles


def convex_gap(a, b):
    """Euclidean exterior distance and negative separating-translation depth."""
    separated=False;depth=math.inf
    for first,second in edges(a)+edges(b):
        dx,dy=second[0]-first[0],second[1]-first[1];length=math.hypot(dx,dy)
        axis=(-dy/length,dx/length)
        ap=[p[0]*axis[0]+p[1]*axis[1] for p in a]
        bp=[p[0]*axis[0]+p[1]*axis[1] for p in b]
        amount=min(max(ap)-min(bp),max(bp)-min(ap))
        separated |= amount<0
        depth=min(depth,amount)
    if not separated:return -depth
    return min([segment_distance(p,x,y) for p in a for x,y in edges(b)]
               +[segment_distance(p,x,y) for p in b for x,y in edges(a)])


def circle_gap(center, radius, polygon):
    distance=min(segment_distance(center,a,b) for a,b in edges(polygon))
    signs=[cross(a,b,center) for a,b in edges(polygon)]
    inside=all(s>=-EPS for s in signs) or all(s<=EPS for s in signs)
    return (-distance if inside else distance)-radius


def body_polygons(body):
    x,y=body['center']
    if body.get('body_parts'):
        return [[(x+v['x'],y+v['y']) for v in part['vertices']] for part in body['body_parts']]
    w,h=body['size']
    return [[(x-w/2,y-h/2),(x+w/2,y-h/2),(x+w/2,y+h/2),(x-w/2,y+h/2)]]
