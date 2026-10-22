#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""The table and summary of a route-mode comparison with Freerouting: our
run.py route-mode output against freerouting_route_mode.py's on the same
placements, both judged by KiCad's DRC with run.py's rules.

    benchmarks/github/freerouting_route_compare.py build/fr2-route-x1367 build/fr2-freerouting-x1367 [--markdown]

A side "wins" a board on KiCad's unconnected items, then on its counting
errors (designer-subtracted, the pass rule's), then on the strict rule's
new findings; a board one side has no result for counts for the other."""

import argparse
import json
import statistics
from pathlib import Path


def rows(path):
    path = Path(path)
    if path.is_dir():
        path = path / "results.json"
    if path.suffix == ".json":
        return {row["name"]: row for row in json.loads(path.read_text())}
    return {row["name"]: row for row in (json.loads(line) for line in path.read_text().splitlines()
                                          if line.startswith("{"))}


def errors(row):
    native = row.get("native")
    return None if native is None else sum(native["errors"].values())


def strict_new(row):
    return sum((row.get("new_findings_strict") or {}).values())


def key(row):
    """Smaller is better; None: no result."""
    native = row.get("native") if row else None
    if native is None:
        return None
    return (native["unconnected"], errors(row), strict_new(row))


def short_name(name):
    return name.split("__", 1)[1] if "__" in name else name


def describe_errors(row):
    native = row.get("native") or {}
    return ", ".join(f"{kind} {count}" for kind, count in sorted(native.get("errors", {}).items())) or "none"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("ours")
    parser.add_argument("freerouting")
    parser.add_argument("--markdown", action="store_true", help="the per-board table as Markdown")
    arguments = parser.parse_args()
    ours, theirs = rows(arguments.ours), rows(arguments.freerouting)
    # The boards both sides were run on (a partial Freerouting run compares
    # over what it has done).
    names = sorted(set(ours) & set(theirs), key=str.lower)
    missing = sorted(set(ours) - set(theirs))
    if arguments.markdown:
        print("| Board | ours routed | ours KiCad open | ours errors | ours strict | ours wall s | ours CPU s "
              "| FR open (its count) | FR KiCad open | FR errors | FR strict | FR wall s | FR CPU s |")
        print("| --- | ---: | ---: | ---: | :---: | ---: | ---: | ---: | ---: | ---: | :---: | ---: | ---: |")
    wins = {"ours": [], "freerouting": [], "tie": []}
    for name in names:
        a, b = ours.get(name) or {}, theirs.get(name) or {}
        ka, kb = key(a), key(b)
        if ka is None and kb is None:
            verdict = "tie"
        elif kb is None or (ka is not None and ka < kb):
            verdict = "ours"
        elif ka is None or kb < ka:
            verdict = "freerouting"
        else:
            verdict = "tie"
        wins[verdict].append(name)
        if arguments.markdown:
            def cell(row, value):
                return "-" if row.get("native") is None else value
            fr_open = b.get("log_unrouted")
            fr_note = " (wall)" if b.get("job_timed_out") else ""
            ours_cut = " CUT" if a.get("cut") else ""
            print(f"| {short_name(name)} | {a.get('routed', '-')}{ours_cut} "
                  f"| {cell(a, (a.get('native') or {}).get('unconnected'))} | {cell(a, errors(a))} "
                  f"| {cell(a, 'pass' if a.get('pass_strict') else 'no')} | {a.get('seconds', '-')} | {a.get('cpu_seconds', '-')} "
                  f"| {'-' if fr_open is None else fr_open}{fr_note} "
                  f"| {cell(b, (b.get('native') or {}).get('unconnected'))} | {cell(b, errors(b))} "
                  f"| {cell(b, 'pass' if b.get('pass_strict') else 'no')} | {b.get('seconds', '-')} | {b.get('cpu_seconds', '-')} |")
    both = [n for n in names if key(ours.get(n)) is not None and key(theirs.get(n)) is not None]

    def total(side, measure):
        return sum(measure(side[n]) for n in both)

    print()
    if missing:
        print(f"not run by Freerouting yet: {len(missing)} boards")
    print(f"{len(names)} boards; results: ours {sum(1 for n in names if key(ours.get(n)) is not None)}, "
          f"Freerouting {sum(1 for n in names if key(theirs.get(n)) is not None)}, both {len(both)}")
    print(f"pass (designer-subtracted per type): ours {sum(1 for n in names if ours[n].get('pass'))}, "
          f"Freerouting {sum(1 for n in names if theirs[n].get('pass'))}")
    print(f"strict pass: ours {sum(1 for n in names if ours[n].get('pass_strict'))}, "
          f"Freerouting {sum(1 for n in names if theirs[n].get('pass_strict'))}")
    print(f"better (KiCad open, then errors, then strict findings): ours {len(wins['ours'])}, "
          f"Freerouting {len(wins['freerouting'])}, tie {len(wins['tie'])}")
    print(f"both with a result: KiCad open ours {total(ours, lambda r: r['native']['unconnected'])}, "
          f"Freerouting {total(theirs, lambda r: r['native']['unconnected'])}; "
          f"counting errors ours {total(ours, errors)}, Freerouting {total(theirs, errors)}; "
          f"strict new findings ours {total(ours, strict_new)}, Freerouting {total(theirs, strict_new)}")
    print(f"both with a result: vias ours {total(ours, lambda r: r.get('vias') or 0)}, "
          f"Freerouting {total(theirs, lambda r: r.get('vias') or 0)}; length ours "
          f"{total(ours, lambda r: r.get('length_mm') or 0) / 1000:.1f} m, Freerouting "
          f"{total(theirs, lambda r: r.get('length_mm') or 0) / 1000:.1f} m")
    timed = [n for n in both if ours[n].get("cpu_seconds") and theirs[n].get("cpu_seconds")]
    if timed:
        cpu = [theirs[n]["cpu_seconds"] / ours[n]["cpu_seconds"] for n in timed]
        wall = [theirs[n]["seconds"] / ours[n]["seconds"] for n in timed if ours[n].get("seconds")]
        print(f"CPU s ours {sum(ours[n]['cpu_seconds'] for n in timed):.0f}, Freerouting "
              f"{sum(theirs[n]['cpu_seconds'] for n in timed):.0f}; per board Freerouting over ours: CPU median "
              f"x{statistics.median(cpu):.2f}, wall median x{statistics.median(wall):.2f} ({len(timed)} boards)")
    stopped = [n for n in names if (theirs.get(n) or {}).get("job_timed_out")]
    print(f"Freerouting stopped by the wall on {len(stopped)} boards; without a result on "
          f"{sum(1 for n in names if n in theirs and key(theirs[n]) is None)} "
          f"({', '.join(sorted({str((theirs[n].get('freerouting') or {}).get('status', theirs[n].get('error'))) for n in names if n in theirs and key(theirs[n]) is None}))})")
    print()
    print("Freerouting better:")
    for name in wins["freerouting"]:
        a, b = ours.get(name) or {}, theirs[name]
        if key(a) is None:
            print(f"  {short_name(name)}: ours no result ({a.get('error', 'no row')}); "
                  f"Freerouting {b['native']['unconnected']} open, errors {describe_errors(b)}")
            continue
        print(f"  {short_name(name)}: ours {a.get('routed')} {a['native']['unconnected']} open, errors "
              f"{describe_errors(a)}, strict new {a.get('new_findings_strict') or {}}{' CUT ' + str(a.get('cut')) if a.get('cut') else ''}; "
              f"Freerouting {b['native']['unconnected']} open, errors {describe_errors(b)}, strict new "
              f"{b.get('new_findings_strict') or {}}")


if __name__ == "__main__":
    main()
