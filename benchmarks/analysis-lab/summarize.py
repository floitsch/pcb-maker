#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Summarize retained agent submissions without rerunning or pooling trials."""
import argparse
import json
import os
from pathlib import Path

NATIVE_TRIALS = [
    'native-json-evaluation', 'native-interactive-evaluation',
    'dense-json-evaluation', 'dense-interactive-evaluation', 'dense-combined-evaluation',
    'dense-combined-assisted-eval-r1', 'dense-graph-tools-evaluation',
    'residual-native-evaluation', 'residual-batch-chains-evaluation',
]


def summarize(results):
    rows=[]
    for path in sorted(results.glob('*-scored.json')):
        score=json.loads(path.read_text())
        name=path.name.removesuffix('-scored.json')
        rows.append(dict(kind='synthetic',name=name,
            phase='recovery, cumulative' if 'assisted' in name or 'crops' in name else 'initial',
            cases=len(score['cases']),improved=score['improved'],rejected=score['invalid'],
            abstained=score['abstained'],accepted_gain_mm=sum(
                case['assessment'].get('cost_reduction_mm',0) for case in score['cases']
                if case['assessment'].get('valid') and case['assessment'].get('improved')),
            evidence=str(path)))
    for name in NATIVE_TRIALS:
        path=results/name/'assessment.json'
        if not path.exists():
            continue
        score=json.loads(path.read_text())
        gain=score['original']['cost_mm']-score['new']['cost_mm']
        rows.append(dict(kind='native',name=name,
            phase='recovery' if 'assisted' in name else 'initial',cases=1,
            improved=int(score['valid'] and score['improved']),rejected=int(not score['valid']),
            abstained=int(score['valid'] and not score['improved']),
            accepted_gain_mm=gain if score['valid'] and score['improved'] else 0,
            unadmitted_or_accepted_raw_gain_mm=gain,
            vias_removed=score['original']['vias']-score['new']['vias'],
            added_findings=len(score['added_findings']),evidence=str(path)))
    return rows


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results',type=Path,default=Path('build/analysis-lab/results'))
    parser.add_argument('--output',type=Path,default=Path('build/analysis-lab/scoreboard'))
    args=parser.parse_args()
    rows=summarize(args.results)
    args.output.parent.mkdir(parents=True,exist_ok=True)
    notes=('Development submissions, not independent samples. Recovery rows reuse earlier work; '
           'synthetic case totals include controls. Gains use the declared trace-length + 2mm/via objective, '
           'not a production-quality metric. Different boards and stages have different opportunity sizes. '
           'Normalization controls, diagnostic negatives and fixture witnesses are excluded. '
           'Missing artifacts are not silently scored as failures.')
    args.output.with_suffix('.json').write_text(json.dumps(dict(notes=notes,trials=rows),indent=2)+'\n')
    lines=['# Board analysis scoreboard','',notes,'',
           '| Kind | Submission | Phase | Cases | Improved | Rejected | Abstained | Accepted gain |',
           '| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |']
    for row in rows:
        link=os.path.relpath(row['evidence'],args.output.parent)
        lines.append(f"| {row['kind']} | [{row['name']}]({link}) | {row['phase']} | {row['cases']} | {row['improved']} | {row['rejected']} | {row['abstained']} | {row['accepted_gain_mm']:.6f} |")
    args.output.with_suffix('.md').write_text('\n'.join(lines)+'\n')
    print(json.dumps(dict(trials=len(rows),json=str(args.output.with_suffix('.json')),markdown=str(args.output.with_suffix('.md')))))


if __name__=='__main__':
    main()
