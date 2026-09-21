#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Audit target connectivity and unchanged retained copper in a conflict probe."""
import collections
import json
from pathlib import Path
import sys
import pcbnew
if not hasattr(pcbnew.SwigPyIterator,'next'):
    pcbnew.SwigPyIterator.next=pcbnew.SwigPyIterator.__next__


def main():
    root=Path(sys.argv[1]);manifest=json.loads((root/'manifest.json').read_text())
    yielded_name=next(r['yielded_connection'] for r in manifest['cases'] if r['yielded_connection'])
    source=Path(manifest['source']);candidate=json.loads(Path(manifest['candidate']).read_text());target=candidate['connection']
    def geometry(path):
        b=pcbnew.LoadBoard(str(path));assert b.GetCopperLayerCount()==2
        assert not any(not z.GetIsRuleArea() for z in b.Zones()), 'Filled-zone geometry needs a separate audit'
        copper=collections.defaultdict(collections.Counter)
        poses=sorted((str(f.GetReference()),f.GetPosition().x,f.GetPosition().y,f.GetOrientationDegrees()) for f in b.GetFootprints())
        for t in b.GetTracks():
            net=str(t.GetNetname())
            if isinstance(t,pcbnew.PCB_VIA):
                value=('via',t.GetPosition().x,t.GetPosition().y,t.GetWidth(pcbnew.F_Cu),t.GetDrillValue())
            else:
                assert not isinstance(t,pcbnew.PCB_ARC)
                ends=sorted([(t.GetStart().x,t.GetStart().y),(t.GetEnd().x,t.GetEnd().y)])
                value=('segment',*ends,t.GetLayer(),t.GetWidth())
            copper[net][value]+=1
        return poses,copper
    poses,original=geometry(source);results=[]
    for record in manifest['cases']:
        assert record['status']=='terminal'
        case=root/record['case'];newposes,copper=geometry(case/source.name);assert poses==newposes
        yielded=record['yielded_connection'];allowed={target,yielded}
        for net,items in original.items():
            if net.removeprefix('/') not in allowed:assert items==copper[net],net
        assert all(net in original or net.removeprefix('/')==target for net in copper)
        if yielded:assert not any(items for net,items in copper.items() if net.removeprefix('/')==yielded)
        drc=json.loads((case/'drc.json').read_text());report=json.loads((case/'verification.json').read_text())
        assert not any('['+'/'+target+']' in item['description'] or '['+target+']' in item['description'] for issue in drc['unconnected_items'] for item in issue['items'])
        if yielded:
            assert report['drc_design_violations']==0 and report['erc_violations']==0 and report['schematic_parity_issues']==0
        else:
            assert report['drc_design_violations']>0
            assert all(target in issue['description'] and yielded_name in issue['description'] for issue in drc['violations'])
        results.append({'case':record['case'],'target_has_no_native_opens':True,'other_net_copper_unchanged':True,'poses_unchanged':True,'verification':report})
    (root/'audit.json').write_text(json.dumps({'scope':__doc__,'cases':results,'board_complete':False,'yielded_net_restoration_required':True},indent=2)+'\n')
    print(json.dumps(results))


if __name__=='__main__':main()
