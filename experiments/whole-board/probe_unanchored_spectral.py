# Copyright (C) 2026 Toit contributors.
"""Net-aware seed experiment for a wholly movable connected placement problem.

Generate an unanchored spectral seed, then use the production legality pass.
This is placement-only evidence; native routing decides whether it is useful.
"""
import argparse
import copy
import json
from pathlib import Path
import shutil

import numpy as np

from area_probe import render
from run import command,digest,read,write


def probe(source,binary,output):
    output.mkdir(exist_ok=False)
    shutil.copy2(__file__,output/Path(__file__).name)
    problem=read(source/'problem.json')
    original=read(source/'placement.json')
    components=sorted(problem['components'],key=lambda c:c['id'])
    assert all(c['constraints']['movement']=='free' for c in components)
    assert all(c['constraints']['rotation']=='fixed' for c in components)
    indexes={c['id']:i for i,c in enumerate(components)}
    count=len(components)
    adjacency=np.zeros((count,count))
    for net in sorted(problem['electrical_nets'],key=lambda n:n['id']):
        members=sorted({indexes[t['component']] for t in net['terminals']})
        if len(members)<2: continue
        # Each terminal receives a net's declared attraction once, rather than
        # multiplying it by the number of other terminals on a shared rail.
        weight=net['width']*net.get('tension_weight',1)/(len(members)-1)
        for i in members:
            for j in members:
                if i!=j: adjacency[i,j]+=weight
    degree=adjacency.sum(axis=1)
    assert np.all(degree>0), 'Disconnected components need a separate packing policy'
    inv=1/np.sqrt(degree)
    laplacian=np.eye(count)-adjacency*inv[:,None]*inv[None,:]
    values,vectors=np.linalg.eigh(laplacian)
    assert values[1]>1e-9, 'Disconnected graph needs independent block placement'
    coordinates=vectors[:,1:3]*inv[:,None]
    for axis in range(2):
        pivot=np.argmax(np.abs(coordinates[:,axis]))
        if coordinates[pivot,axis]<0: coordinates[:,axis]*=-1
    bounds=problem['board']['bounds']
    config=read(source/'placement-config.json')
    config['policy']['underanchored_seed']={'kind':'declared'}
    rows=[]
    for kind in ['continuous','rank']:
        candidate=copy.deepcopy(problem)
        for c in candidate['components']:
            i=indexes[c['id']]
            for axis,key in enumerate(['x','y']):
                column=coordinates[:,axis]
                if kind=='continuous':
                    fraction=float((column[i]-column.min())/(column.max()-column.min()))
                else:
                    order=sorted(range(count),key=lambda j:(float(column[j]),components[j]['id']))
                    fraction=order.index(i)/(count-1)
                lo,hi=bounds['min'][key],bounds['max'][key]
                c['position'][key]=lo+(hi-lo)*(.05+.9*fraction)
        directory=output/kind; directory.mkdir()
        write(directory/'problem.json',candidate);write(directory/'config.json',config)
        poses=[dict(component=c['id'],position=c['position'],rotation_degrees=0) for c in candidate['components']]
        render(candidate,poses,directory/'seed.svg',f'{kind} spectral proposal; not yet legalized')
        process=command([binary,'place',directory/'problem.json',directory/'config.json',directory/'placement.json'],directory/'place.log')
        row=dict(kind=kind,process=process)
        if process['exit_code']==0:
            placed=read(directory/'placement.json')
            audit=render(candidate,placed['poses'],directory/'placed.svg',f'{kind} spectral seed after production legalization; not yet routed')
            assert audit['passed']
            row.update(evidence=placed['evidence'],physical_placement_audit=audit)
        rows.append(row)
    result=dict(scope=__doc__,source_problem_sha256=digest(source/'problem.json'),
        binary_sha256=digest(binary),baseline_evidence=original['evidence'],cases=rows,
        numpy_version=np.__version__,lowest_eigenvalues=values[:5].tolist(),
        components=count,nets=len(problem['electrical_nets']),routing_tested=False)
    write(output/'report.json',result)
    print(json.dumps(result))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['source','binary','output']:p.add_argument(name,type=Path)
    a=p.parse_args()
    probe(a.source.resolve(),a.binary.resolve(),a.output.resolve())
