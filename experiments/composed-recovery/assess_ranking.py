#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Compare a geometric ranking with an existing one-net routing counterfactual oracle."""
import argparse
import hashlib
import json
from pathlib import Path


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('analysis',type=Path);parser.add_argument('oracle',type=Path)
    args=parser.parse_args();a=json.loads(args.analysis.read_text());o=json.loads(args.oracle.read_text())
    assert hashlib.sha256(Path(o['board']).read_bytes()).hexdigest()==a['board_sha256']
    assert a['candidate_connection'].removeprefix('/')==o['target_connection'].removeprefix('/')
    assert not o['base_route_found']
    assert sorted(a['foreign_nets'])==sorted(o['foreign_routed_connections'])
    found={t['yielding_connections'][0]:t['route_found'] for t in o['trials'] if len(t['yielding_connections'])==1}
    assert set(found)==set(a['foreign_nets']), 'Need exhaustive one-net oracle coverage'
    def score(order):return next((i+1 for i,n in enumerate(order) if found[n]),None)
    result={'scope':__doc__,'oracle_sha256':hashlib.sha256(args.oracle.read_bytes()).hexdigest(),
            'successful_one_net_counterfactuals':sorted(n for n,v in found.items() if v),
            'geometric_order':a['ranked_yielding_nets'],'lexical_order':sorted(found),
            'first_route_found_rank':score(a['ranked_yielding_nets']),
            'lexical_first_route_found_rank':score(sorted(found)),
            'interpretation':'Checks priority of routing counterfactuals, not necessity of blockers, target native admission, or successful restoration of yielded nets.'}
    out=args.analysis.with_name('oracle-comparison.json');out.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))


if __name__=='__main__':main()
