#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""The tables of docs/experiments/objective-terms.md from stored data.

    report.py --runs DIR --traces FILE --measures FILE

`--runs`: `<board>/<variant>/trace.json` (labelled.py's and
skeleton_gap.py's place checks: `ours-s<base>`, `designer`, and where run
`ours-plain-s1`, `designer-plain`); `--traces`: measures.py's `traces`
output for those runs; `--measures`: measures.py's `compute` output (the
cut census's extracted boards: the designer's and nine October seeds per
dense board with their first routes). Prints markdown.
"""

import argparse
import json
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import measures  # noqa: E402
import terms  # noqa: E402

GAP = ["GlasgowEmbedded__glasgow__glasgow__revC3", "OpenDrone-hw__OpenESC-30x30__4in1",
       "OpenDrone-hw__OpenESC-20x20__4in1-mini"]
OLD = {  # docs/placer.md before 66346adc, "The objective at the designer's arrangement and ours" (x1904)
    "Huaqiu-Electronics__ecad-viewer__video": (0.59, 0.55),
    "OpenDrone-hw__OpenESC-30x30__4in1": (0.72, 0.74),
    "GlasgowEmbedded__glasgow__glasgow__revC3": (1.02, 1.05),
    "stonedDiscord__nonSNES__SNSP-CPU-01": (0.99, 1.04),
}


def short(board):
    return board.split("__")[-1]


def seeds_of(runs, board):
    """{label: seed entry} of every traced seed of every variant."""
    result = {}
    for variant in sorted(p for p in (runs / board).iterdir() if (p / "trace.json").exists()):
        for seed in json.loads((variant / "trace.json").read_text())["seeds"]:
            result[f"{variant.name}@{seed['seed']}"] = seed
    return result


def position(value, values):
    values = sorted(values)
    below = sum(1 for v in values if v < value)
    return f"{value / statistics.median(values):.2f} ({below}/{len(values)})" if statistics.median(values) else "-"


def reconciliation(runs):
    print("| board | 2026-11-04: designer / ours (seeds 1, 101) | today, same: designer / ours (kept seeds 1, 101) | "
          "today, whole: designer / our median (seeds below it) | today, skeleton: designer / our median "
          "(seeds below) | today, plain pipeline: designer / ours-plain kept seed 1 |")
    print("| --- " * 6 + "|")
    for board in sorted(p.name for p in runs.iterdir() if p.is_dir()):
        seeds = seeds_of(runs, board)
        designer = next((s for label, s in seeds.items() if label.startswith("designer@") and s["kept"]), None)
        if designer is None:
            continue
        ours = {label: s for label, s in seeds.items() if label.startswith("ours-s")}
        whole = [terms.terms(s["objective"]["final"])["total_ov"] for s in ours.values()]
        d = terms.terms(designer["objective"]["final"])["total_ov"]
        kept = [terms.terms(s["objective"]["final"])["total_ov"] for label, s in sorted(ours.items()) if s["kept"]]
        skeleton = [terms.terms(s["skeleton_objective"]["skeleton_final"])["total"] for s in ours.values()
                    if (s.get("skeleton_objective") or {}).get("skeleton_final")
                    and not (s.get("skeleton") or {}).get("fallback_kept")]
        dsk = (designer.get("skeleton_objective") or {}).get("skeleton_final")
        plain_d = next((s for label, s in seeds.items() if label.startswith("designer-plain@") and s["kept"]), None)
        plain_o = next((s for label, s in seeds.items() if label.startswith("ours-plain-s1@") and s["kept"]), None)
        plain = (f"{terms.terms(plain_d['objective']['final'])['total_ov'] / terms.terms(plain_o['objective']['final'])['total_ov']:.2f}"
                 if plain_d and plain_o else "-")
        old = OLD.get(board)
        print(f"| {short(board)} | {'-' if old is None else f'{old[0]:.2f}, {old[1]:.2f}'} | "
              f"{', '.join(f'{d / k:.2f}' for k in kept)} | {position(d, whole)} | "
              f"{position(terms.terms(dsk)['total'], skeleton) if dsk and skeleton else '-'} | {plain} |")


def per_term(runs, scope):
    keys = ["total", "w2", "w3_8", "w9", "hp9", "crossings", "relations", "overlap_mm2"]
    print("| board | placement | " + " | ".join(keys) + " |")
    print("| --- " * (len(keys) + 2) + "|")
    for board in GAP + ["Huaqiu-Electronics__ecad-viewer__video"]:
        if not (runs / board).exists():
            continue
        seeds = seeds_of(runs, board)
        ours = [s for label, s in seeds.items() if label.startswith("ours-s")]

        def objective(seed, stage):
            if scope == "skeleton":
                return (seed.get("skeleton_objective") or {}).get("skeleton_final" if stage == "final" else stage)
            return seed["objective_bare"].get(stage)

        ours_terms = [terms.terms(objective(s, "final")) for s in ours if objective(s, "final")
                      and not (scope == "skeleton" and (s.get("skeleton") or {}).get("fallback_kept"))]
        rows = [("ours: median [min, max]", None)]
        rows.append(("designer, raw poses", terms.terms(objective(ours[0], "designer"))))
        designer = next((s for label, s in seeds.items() if label.startswith("designer@") and s["kept"]), None)
        if designer and objective(designer, "final"):
            rows.append(("designer, legalised and refined", terms.terms(objective(designer, "final"))))
        plain = next((s for label, s in seeds.items() if label.startswith("designer-plain@") and s["kept"]), None)
        if plain and scope == "whole":
            rows.append(("designer, plain pipeline", terms.terms(objective(plain, "final"))))
        for name, values in rows:
            if values is None:
                cells = [f"{statistics.median(t[k] for t in ours_terms):.0f} [{min(t[k] for t in ours_terms):.0f}, "
                         f"{max(t[k] for t in ours_terms):.0f}]" for k in keys]
            else:
                cells = [f"{values[k]:.0f} ({sum(1 for t in ours_terms if t[k] < values[k])}/{len(ours_terms)})"
                         for k in keys]
            print(f"| {short(board)} | {name} | " + " | ".join(cells) + " |")


def pad_measures(rows, keys):
    print("| board | placement | " + " | ".join(keys) + " |")
    print("| --- " * (len(keys) + 2) + "|")
    for board in sorted({r["board"] for r in rows}):
        mine = [r for r in rows if r["board"] == board]
        ours = [r for r in mine if r["label"].startswith("ours-s") or (r["label"].startswith("s") and
                                                                        r["label"][1:].isdigit())]
        if not ours:
            continue
        print(f"| {short(board)} | ours: median [min, max] | " + " | ".join(
            f"{statistics.median(r[k] for r in ours):.0f} [{min(r[k] for r in ours):.0f}, {max(r[k] for r in ours):.0f}]"
            for k in keys) + " |")
        for label, name in (("designer-raw", "designer, raw"), ("designer", "designer, raw"),
                            ("designer@kept", "designer, legalised and refined"),
                            ("designer-plain@kept", "designer, plain pipeline")):
            if label.endswith("@kept"):
                found = [r for r in mine if r["label"].startswith(label[:-4]) and r.get("kept")]
            else:
                found = [r for r in mine if r["label"] == label]
            if found:
                r = found[0]
                print(f"| {short(board)} | {name} | " + " | ".join(
                    f"{r[k]:.0f} ({sum(1 for o in ours if o[k] < r[k])}/{len(ours)})" for k in keys) + " |")


def first_routes(rows):
    print("| board | placement | first-route open | closed_active | crossings | wire |")
    print("| --- " * 6 + "|")
    for row in rows:
        if row.get("first_open") is not None and not (row["label"].startswith("s") and row["label"][1:].isdigit()):
            print(f"| {short(row['board'])} | {row['label']} | {row['first_open']} | {row['closed_active']} | "
                  f"{row['crossings']} | {row['wire']:.0f} |")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--runs", required=True)
    parser.add_argument("--traces", required=True)
    parser.add_argument("--measures", required=True)
    arguments = parser.parse_args()
    runs = Path(arguments.runs)
    print("## Reconciliation\n")
    reconciliation(runs)
    for scope in ("whole", "skeleton"):
        print(f"\n## Per term, {scope}\n")
        per_term(runs, scope)
    keys = ["closed", "closed_active", "narrow_active", "rudy_over_half", "crossings", "hp2", "hp3_8", "hp9"]
    print("\n## Pad measures, today's placements\n")
    pad_measures(json.loads(Path(arguments.traces).read_text()), keys)
    print("\n## Pad measures, October seeds\n")
    pad_measures(json.loads(Path(arguments.measures).read_text()), keys)
    print("\n## First routes\n")
    first_routes(json.loads(Path(arguments.traces).read_text()))
    print("\n## Seed ranking (October seeds)\n")
    measures.table(argparse.Namespace(data=arguments.measures))


if __name__ == "__main__":
    sys.exit(main())
