#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""The table and summary of a KiCadRoutingTools head-to-head
(kicadroutingtools.py) against our route-mode rows of the same placements.

    benchmarks/github/kicadroutingtools_compare.py build/krt-route-x1367 --ours build/fr2-route-x1367.log

Per board (Markdown): KiCad's unconnected items and copper errors (beyond
the designer's per type), the strict rule's pass, wall and CPU seconds, for
both sides; KiCadRoutingTools' own count of routed nets. Then the summary:
passes per side (KiCadRoutingTools also within our wall), the boards where
it beats us (fewer KiCad unconnected items, then fewer errors) with what
KiCad reports on ours there, and the speed ratios (medians over the boards
where it finished)."""

import argparse
import json
import re
import statistics
import sys
from pathlib import Path


def load(path):
    path = Path(path)
    if path.is_dir():
        if not (path / "results.json").exists():
            return {}
        rows = json.loads((path / "results.json").read_text())
    else:
        rows = [json.loads(line) for line in path.read_text().splitlines() if line.startswith("{")]
    return {row["name"]: row for row in rows}


def progress(log):
    """Where a cut KiCadRoutingTools run was, from its log: the main loop's
    last "[i/n (f failed)]" and whether its rescue pass had finished."""
    try:
        text = log.read_text(errors="replace")
    except OSError:
        return ""
    found = re.findall(r"\[(\d+)/(\d+) \((\d+) failed\)", text)
    if not found:
        return "before the main loop"
    done, total, failed = found[-1]
    rescued = re.findall(r"^Rescue pass: (\d+) recovered", text, re.M)
    stage = f"main loop {done}/{total} nets, {failed} failed"
    if rescued:
        stage += f"; rescue pass done, {rescued[-1]} recovered"
    return stage


def errors(row):
    return sum((row.get("native") or {}).get("errors", {}).values())


def unconnected(row):
    native = row.get("native")
    return None if native is None else native["unconnected"]


def short(name, width=44):
    """The board's repository and board name, shortened."""
    parts = name.split("__")
    text = parts[-1] if len(parts) < 3 else f"{parts[1]}/{parts[-1]}"
    return text if len(text) <= width else text[:width - 1] + "…"


def describe(row):
    """What KiCad reports on a row: unconnected items and errors by type."""
    native = row.get("native") or {}
    found = [f"{native.get('unconnected', '?')} unconnected"]
    found += [f"{count} {kind}" for kind, count in sorted(native.get("errors", {}).items())]
    if row.get("new_findings_strict") and not row.get("pass_strict") and native.get("unconnected") == 0:
        found.append("strict: " + ", ".join(f"{c} {k}" for k, c in sorted(row["new_findings_strict"].items())))
    return ", ".join(found)


def key(row):
    """Lower is better: KiCad's unconnected items, then errors; no result last."""
    value = unconnected(row)
    return (1, 0, 0) if value is None else (0, value, errors(row))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("krt", nargs="+", help="kicadroutingtools.py outputs (a later one's rows win)")
    parser.add_argument("--ours", required=True, help="run.py's route-mode output or log")
    arguments = parser.parse_args()
    theirs, logs = {}, {}
    for path in arguments.krt:
        rows = load(path)
        theirs.update(rows)
        if Path(path).is_dir():
            logs.update({name: Path(path) / name / "krt.log" for name in rows})
    ours = load(arguments.ours)
    # A row of ours killed from outside (the kernel's OOM killer: exit -9 or
    # 137) is not the router's answer: left out until it is rerun.
    killed = sorted(n for n, row in ours.items() if row.get("exit") in (-9, 137))
    for name in killed:
        del ours[name]
    if killed:
        print(f"left out, our row killed (exit -9): {' '.join(killed)}", file=sys.stderr)
    # Likewise a KiCadRoutingTools row killed from outside (not by the cap).
    for name in [n for n, row in theirs.items() if row.get("exit") in (-9, 137)]:
        del theirs[name]
    names = [name for name in ours if name in theirs]
    # Within our wall, against our row as given here (a rerun of ours may
    # have another wall than the one the KRT run was capped by).
    for name in names:
        b, wall = theirs[name], ours[name].get("seconds")
        within = b.get("native") is not None and wall is not None and (b.get("seconds") or 0) <= wall
        b["within_wall"] = within
        b["pass_in_wall"] = within and bool(b.get("pass"))
        b["pass_strict_in_wall"] = within and bool(b.get("pass_strict"))
    names.sort(key=lambda n: (ours[n].get("routed") or "0/0").split("/")[1].zfill(6) + n)
    print("| board | conn. | ours: unconn. / errors / strict | KRT: nets / unconn. / errors / strict | wall s ours / KRT | CPU s ours / KRT |")
    print("|---|---:|---|---|---:|---:|")
    for name in names:
        a, b = ours[name], theirs[name]
        connections = (a.get("routed") or "?/?").split("/")[1]
        mine = f"{unconnected(a)} / {errors(a)} / {'pass' if a.get('pass_strict') else 'fail'}" \
            if a.get("native") is not None else ("timed out" if a.get("cut") else "no result")
        if b.get("native") is None:
            krt = b.get("status") or b.get("error") or "no result"
            if b.get("timed_out"):
                krt = f"cut at {b.get('cap_seconds')} s"
                if name in logs:
                    krt += f" ({progress(logs[name])})"
        else:
            summary = b.get("krt_summary") or {}
            nets = f"{summary.get('successful', '?')}/{summary.get('successful', 0) + summary.get('failed', 0)}" \
                if "successful" in summary else "?"
            krt = f"{nets} / {unconnected(b)} / {errors(b)} / {'pass' if b.get('pass_strict') else 'fail'}"
            if b.get("timed_out"):
                krt += f" (cut at {b.get('cap_seconds')} s, board written)"
        print(f"| {short(name)} | {connections} | {mine} | {krt} | {a.get('seconds', 0):.0f} / {b.get('seconds', 0):.0f} "
              f"| {a.get('cpu_seconds', 0):.0f} / {b.get('cpu_seconds', 0):.0f} |")
    print()
    finished = [n for n in names if theirs[n].get("native") is not None]
    done = sum(1 for n in names if theirs[n].get("native") is not None and not theirs[n].get("timed_out"))
    cut_judged = sum(1 for n in names if theirs[n].get("native") is not None and theirs[n].get("timed_out"))
    cut_empty = sum(1 for n in names if theirs[n].get("native") is None and theirs[n].get("timed_out"))
    print(f"{len(names)} boards; KiCadRoutingTools finished {done}, cut by the cap after writing its board "
          f"{cut_judged} (judged), cut before {cut_empty}, other failures {len(names) - done - cut_judged - cut_empty}")
    count = lambda rows, field: sum(1 for n in names if rows[n].get(field))
    print(f"pass (KiCad, designer's errors subtracted): ours {count(ours, 'pass')}, KRT {count(theirs, 'pass')} "
          f"({count(theirs, 'pass_in_wall')} within our wall)")
    print(f"strict pass: ours {count(ours, 'pass_strict')}, KRT {count(theirs, 'pass_strict')} "
          f"({count(theirs, 'pass_strict_in_wall')} within our wall)")
    print(f"KRT finished within our wall: {count(theirs, 'within_wall')}")
    better = [n for n in names if key(theirs[n]) < key(ours[n])]
    worse = [n for n in names if key(theirs[n]) > key(ours[n])]
    print(f"better (unconnected, then errors): KRT {len(better)}, ours {len(worse)}, tie {len(names) - len(better) - len(worse)}")
    both = [n for n in finished if ours[n].get("native") is not None]
    print(f"both finished ({len(both)}): KiCad unconnected ours {sum(unconnected(ours[n]) for n in both)}, "
          f"KRT {sum(unconnected(theirs[n]) for n in both)}; errors ours {sum(errors(ours[n]) for n in both)}, "
          f"KRT {sum(errors(theirs[n]) for n in both)}; vias ours {sum(ours[n].get('vias') or 0 for n in both)}, "
          f"KRT {sum(theirs[n].get('vias') or 0 for n in both)}; length ours "
          f"{sum(ours[n].get('length_mm') or 0 for n in both) / 1000:.1f} m, KRT "
          f"{sum(theirs[n].get('length_mm') or 0 for n in both) / 1000:.1f} m")
    # Speed only where KiCadRoutingTools finished on its own (a cut run's
    # time is the cap's).
    timed = [n for n in both if ours[n].get("cpu_seconds") and theirs[n].get("cpu_seconds")
             and not theirs[n].get("timed_out")]
    if timed:
        cpu = [theirs[n]["cpu_seconds"] / ours[n]["cpu_seconds"] for n in timed]
        wall = [theirs[n]["seconds"] / ours[n]["seconds"] for n in timed if ours[n].get("seconds")]
        print(f"KRT over ours, per board ({len(timed)} boards): CPU median x{statistics.median(cpu):.2f} "
              f"(range x{min(cpu):.2f} to x{max(cpu):.2f}), wall median x{statistics.median(wall):.2f}")
    bounded = [n for n in names if ours[n].get("seconds") and theirs[n].get("seconds")]
    if bounded:
        ratio = [theirs[n]["seconds"] / ours[n]["seconds"] for n in bounded]
        print(f"KRT wall over ours on all {len(bounded)} boards, a cut run at its cap (a lower bound): median "
              f"x{statistics.median(ratio):.2f}, at least ours on {sum(1 for r in ratio if r >= 1)}")
    print()
    for name in better:
        print(f"KRT better on {name}: KRT {describe(theirs[name])}; ours {describe(ours[name])}")
    missing = [n for n in ours if n not in theirs]
    if missing:
        print(f"\nno KRT row for {len(missing)} boards", file=sys.stderr)


if __name__ == "__main__":
    main()
