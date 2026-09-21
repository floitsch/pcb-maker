# Copyright (C) 2026 Toit contributors.
"""Net-aware seeds for wholly movable fixed-orientation problems.

These poses require production legalization and native verification. Fixed blocks need another policy. Disconnected attraction blocks receive
separate area-weighted seed regions; production legalization decides legality.
"""
import copy


def seed(problem,kind='rank'):
    import numpy as np
    assert kind in ['continuous','rank']
    components=sorted(problem['components'],key=lambda c:c['id'])
    assert components, 'Placement requires at least one component'
    assert all(c['constraints']['movement']=='free' for c in components), 'Spectral seed requires wholly movable placement'
    assert all(c['constraints']['rotation']=='fixed' for c in components), 'Spectral seed requires fixed original orientations'
    indexes={c['id']:i for i,c in enumerate(components)}
    count=len(components)
    adjacency=np.zeros((count,count))
    for net in sorted(problem['electrical_nets'],key=lambda n:n['id']):
        members=sorted({indexes[t['component']] for t in net['terminals']})
        if len(members)<2:continue
        weight=net['width']*net.get('tension_weight',1)/(len(members)-1)
        assert np.isfinite(weight) and weight>=0
        for i in members:
            for j in members:
                if i!=j:adjacency[i,j]+=weight
    for net in sorted(problem.get('nets',[]),key=lambda n:n['id']):
        a,b=indexes[net['from']['component']],indexes[net['to']['component']]
        weight=net['width']*net.get('tension_weight',1)
        assert np.isfinite(weight) and weight>=0
        if a!=b:
            adjacency[a,b]+=weight;adjacency[b,a]+=weight
    assert np.all(np.isfinite(adjacency))
    unseen=set(range(count));blocks=[]
    while unseen:
        first=min(unseen);unseen.remove(first);block=[first];pending=[first]
        while pending:
            node=pending.pop()
            for neighbor in sorted(unseen):
                if adjacency[node,neighbor]>0:
                    unseen.remove(neighbor);block.append(neighbor);pending.append(neighbor)
        blocks.append(sorted(block))
    bounds=problem['board']['bounds']
    weights=[sum(components[i]['size']['x']*components[i]['size']['y'] for i in b) for b in blocks]
    assert all(np.isfinite(w) and w>0 for w in weights)
    regions={}
    def divide(ids,region):
        if len(ids)==1:
            regions[ids[0]]=region;return
        total=sum(weights[i] for i in ids)
        split=min(range(1,len(ids)),key=lambda n:abs(sum(weights[i] for i in ids[:n])-total/2))
        fraction=max(.05,min(.95,sum(weights[i] for i in ids[:split])/total))
        axis=max(['x','y'],key=lambda a:region['max'][a]-region['min'][a])
        cut=region['min'][axis]+fraction*(region['max'][axis]-region['min'][axis])
        left,right=copy.deepcopy(region),copy.deepcopy(region)
        left['max'][axis]=cut;right['min'][axis]=cut
        divide(ids[:split],left);divide(ids[split:],right)
    divide(sorted(range(len(blocks)),key=lambda i:(-weights[i],components[blocks[i][0]]['id'])),copy.deepcopy(bounds))
    candidate=copy.deepcopy(problem)
    by={c['id']:c for c in candidate['components']}
    evidence=[]
    for block_id,block in enumerate(blocks):
        coordinates=np.zeros((len(block),2));values=np.zeros(1)
        if len(block)>1:
            graph=adjacency[np.ix_(block,block)]
            degree=graph.sum(axis=1)
            inv=1/np.sqrt(degree)
            laplacian=np.eye(len(block))-graph*inv[:,None]*inv[None,:]
            values,vectors=np.linalg.eigh(laplacian)
            assert values[1]>1e-9, 'Numerically disconnected attraction block'
            coordinates[:,:min(2,len(block)-1)]=vectors[:,1:3]*inv[:,None]
            for axis in range(2):
                pivot=np.argmax(np.abs(coordinates[:,axis]))
                if coordinates[pivot,axis]<0:coordinates[:,axis]*=-1
        for local,i in enumerate(block):
            c=by[components[i]['id']]
            for axis,key in enumerate(['x','y']):
                column=coordinates[:,axis]
                if column.max()==column.min():fraction=.5
                elif kind=='continuous':fraction=float((column[local]-column.min())/(column.max()-column.min()))
                else:
                    order=sorted(range(len(block)),key=lambda j:(float(column[j]),components[block[j]]['id']))
                    fraction=order.index(local)/(len(block)-1)
                lo,hi=regions[block_id]['min'][key],regions[block_id]['max'][key]
                c['position'][key]=lo+(hi-lo)*(.05+.9*fraction)
        evidence.append(dict(components=[components[i]['id'] for i in block],region=regions[block_id],
            lowest_eigenvalues=values[:5].tolist(),envelope_area_weight=weights[block_id]))
    return candidate,dict(kind=kind,numpy_version=np.__version__,components=count,
        lowest_eigenvalues=evidence[0]['lowest_eigenvalues'] if len(blocks)==1 else None,
        attraction_blocks=evidence,seed_regions_are_constraints=False,
        net_weight='width * tension_weight / (other component count)',
        source_positions_used=False,legalized=False,routing_tested=False)
