# Copyright (C) 2026 Florian Loitsch.
"""Scores the subjects' answers against cases.json.

Usage: score.py results/r1.jsonl [results/r2.jsonl ...]
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def main():
    cases = {c["id"]: c for c in json.load(open(os.path.join(HERE, "cases.json")))["cases"]}
    rows = []
    for path in sys.argv[1:]:
        for line in open(path):
            if line.strip():
                r = json.loads(line)
                case, rep = r["subject"].split("-", 1)
                c = cases[case]
                r.update(case=case, rep=rep, truth=c["class"], census=c["census_class"], ok=r["answer"]["cause"] == c["class"], ok_census=r["answer"]["cause"] == c["census_class"])
                rows.append(r)
    reps = []
    for r in rows:
        if r["rep"] not in reps:
            reps.append(r["rep"])
    print("| representation | runs | correct | against the census's first key | mean confidence right / wrong | tokens | tool uses | seconds |")
    print("| --- | ---: | ---: | ---: | --- | ---: | ---: | ---: |")
    for rep in reps:
        rs = [r for r in rows if r["rep"] == rep]
        ok = [r for r in rs if r["ok"]]
        bad = [r for r in rs if not r["ok"]]
        mean = lambda xs: sum(xs) / len(xs) if xs else float("nan")
        print(
            f"| {rep} | {len(rs)} | {len(ok)} | {sum(r['ok_census'] for r in rs)} | {mean([r['answer']['confidence'] for r in ok]):.0f} / {mean([r['answer']['confidence'] for r in bad]):.0f} "
            f"| {mean([r['tokens'] for r in rs]) / 1000:.0f}k | {mean([r['tool_uses'] for r in rs]):.1f} | {mean([r['seconds'] for r in rs]):.0f} |"
        )
    print()
    print("| case | truth | " + " | ".join(reps) + " |")
    print("| --- | --- | " + " | ".join("---" for _ in reps) + " |")
    for cid, c in cases.items():
        cells = []
        for rep in reps:
            r = [r for r in rows if r["case"] == cid and r["rep"] == rep]
            cells.append(f"{r[0]['answer']['cause']} ({r[0]['answer']['confidence']})" + ("" if r[0]["ok"] else " x") if r else "")
        print(f"| {cid} | {c['class']}" + (f" (census {c['census_class']})" if c["class"] != c["census_class"] else "") + " | " + " | ".join(cells) + " |")
    print()
    for truth in "ABC":
        line = []
        for rep in reps:
            rs = [r for r in rows if r["rep"] == rep and r["truth"] == truth]
            line.append(f"{rep} {sum(r['ok'] for r in rs)}/{len(rs)}")
        print(f"class {truth}: " + ", ".join(line))


if __name__ == "__main__":
    main()
