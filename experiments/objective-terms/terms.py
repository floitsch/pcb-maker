#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Per-term study of the placer's objective, the designer's arrangement
against our seeds (docs/experiments/objective-terms.md).

    terms.py table --runs DIR [--scope skeleton|whole|whole_level] [--json OUT]

`--runs` is a directory of `<board>/<variant>/trace.json` as
experiments/cut-census/skeleton_gap.py writes them (`ours-s<base>` and
`designer`). Per board and term: the designer's value (its arrangement
legalised and refined by our stages), our kept seeds' median and
spread, the designer's rank among our seeds, and its distance from our
median in seed spreads (max - min). Scopes:

- `skeleton`: the trace's `skeleton_objective` at `skeleton_final` (the
  skeleton's own problem, halos off);
- `whole`: `objective_bare` at `final` (all parts, no halos);
- `whole_level`: `objective` at `final` (all parts, the halos of the
  placement's level; levels differ between placements).

Seeds that fell back to a plain placement are left out of `skeleton`.
"""

import argparse
import json
import statistics
import sys
from pathlib import Path

TERMS = ["total", "total_ov", "wire", "w2", "w3_8", "w9", "hp2", "hp3_8", "hp9", "crossings", "crossings_all",
         "relations", "hard", "overlap_mm2", "overlapping"]


def terms(objective):
    """The anneal's objective split: weighted wire per net size (2, 3-8,
    9+ pins), unweighted half perimeters, crossings of the anneal's nets
    (priced at `crossing_weight`) and of all counted nets, relations, hard
    violation, overlap. `total` is wire + relations + 1000 hard + priced
    crossings; `total_ov` adds the anneal's final overlap price (400 per mm²)."""
    w = objective["wirelength"]
    hp = objective["half_perimeter"]
    crossings = objective.get("crossings_anneal") or 0
    total = sum(w) + objective["relations"] + 1000.0 * objective["hard"] + objective["crossing_weight"] * crossings
    return {"total": total, "total_ov": total + 400.0 * objective["overlap"], "wire": sum(w),
            "w2": w[0], "w3_8": w[1], "w9": w[2], "hp2": hp[0], "hp3_8": hp[1], "hp9": hp[2],
            "crossings": crossings, "crossings_all": objective.get("crossings_all") or 0,
            "relations": objective["relations"], "hard": objective["hard"],
            "overlap_mm2": objective["overlap"], "overlapping": objective.get("overlapping") or 0}


def objective_at(seed, scope):
    if scope == "skeleton":
        return (seed.get("skeleton_objective") or {}).get("skeleton_final")
    if scope == "whole":
        return seed["objective_bare"].get("final")
    return seed["objective"].get("final")


def read_board(directory, scope):
    """(ours, designer): lists of {seed, kept, terms} per kept seed."""
    ours, designer = [], []
    for variant in sorted(p for p in directory.iterdir() if p.is_dir()):
        trace = variant / "trace.json"
        if not trace.exists():
            continue
        data = json.loads(trace.read_text())
        for seed in data["seeds"]:
            fallback = bool((seed.get("skeleton") or {}).get("fallback_kept"))
            if scope == "skeleton" and fallback:
                continue
            objective = objective_at(seed, scope)
            if objective is None:
                continue
            entry = {"variant": variant.name, "seed": seed["seed"], "kept": seed["kept"], "terms": terms(objective)}
            (designer if variant.name == "designer" else ours).append(entry)
    return ours, designer


def summary(ours, designer):
    """Per term the designer's value against our seeds."""
    result = {}
    d = designer[0]["terms"]  # deterministic: every designer seed is the same
    for term in TERMS:
        values = sorted(s["terms"][term] for s in ours)
        spread = values[-1] - values[0]
        median = statistics.median(values)
        worse = sum(1 for v in values if v < d[term])
        result[term] = {"designer": d[term], "best": values[0], "median": median, "worst": values[-1],
                        "spread": spread, "ratio_median": d[term] / median if median else None,
                        "spreads_from_median": (d[term] - median) / spread if spread > 0 else None,
                        "seeds_below_designer": worse, "seeds": len(values)}
    return result


def table(arguments):
    runs = Path(arguments.runs)
    output = {}
    shown = ["total", "wire", "w2", "w3_8", "w9", "hp9", "crossings", "relations", "overlap_mm2"]
    print("| board | seeds | " + " | ".join(shown) + " |")
    print("| --- " * (len(shown) + 2) + "|")
    for board in sorted(p for p in runs.iterdir() if p.is_dir()):
        ours, designer = read_board(board, arguments.scope)
        if not ours or not designer:
            print(f"| {board.name} | {len(ours)} | " + "no designer or no seeds |")
            continue
        s = summary(ours, designer)
        output[board.name] = {"summary": s, "ours": ours, "designer": designer[0]}
        cells = []
        for term in shown:
            t = s[term]
            ratio = "" if t["ratio_median"] is None else f"{t['ratio_median']:.2f}"
            spreads = "" if t["spreads_from_median"] is None else f"{t['spreads_from_median']:+.1f}"
            cells.append(f"{ratio} ({spreads}; {t['seeds_below_designer']}/{t['seeds']})")
        print(f"| {board.name} | {len(ours)} | " + " | ".join(cells) + " |")
    if arguments.json:
        Path(arguments.json).write_text(json.dumps(output, indent=1))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    t = sub.add_parser("table")
    t.add_argument("--runs", required=True)
    t.add_argument("--scope", default="skeleton", choices=["skeleton", "whole", "whole_level"])
    t.add_argument("--json")
    arguments = parser.parse_args()
    table(arguments)


if __name__ == "__main__":
    sys.exit(main())
