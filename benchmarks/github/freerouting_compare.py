#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Freerouting against our cold route, board by board.

    benchmarks/github/freerouting_compare.py <ours-cold.log> <freerouting.log>... [--markdown]
        [--previous-ours <log> --previous-freerouting <log>...]

The logs are freerouting.py's JSON lines: the first run with `--ours`
(its `cold_route` rows), the others Freerouting's (`freerouting` rows;
a later log's row of a board replaces an earlier one's). One log that has
both kinds of rows can be given as both. Both route the same cold board
(the designer's placement, every track and via removed) under the same
wall-clock limit; the counts are KiCad's DRC of the result, errors beyond
the cold board's own and the designer's. The table lists the boards both
have, then the totals.

With --previous-ours/--previous-freerouting (an earlier round), the same
summary for that round, our cold route then against now, and the boards
whose verdict (who has fewer unconnected, then fewer errors) flipped.

CPU seconds are rusage: Freerouting's is the JVM alone, ours the whole
route-kicad-board process with the KiCad checks it runs (its own DRC
included); "+pipeline" adds Freerouting's DSN export, session import and
the DRC that judges it. Rounds before x738 recorded no CPU."""

import argparse
import json
from pathlib import Path


def rows_of(path):
    rows = {}
    for line in Path(path).read_text().splitlines():
        if line.startswith("{"):
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                continue
            rows[row["name"]] = row
    return rows


def merged(paths, key):
    rows = {}
    for path in paths:
        rows.update({name: row[key] for name, row in rows_of(path).items() if key in row})
    return rows


def errors(native):
    return sum((native or {}).get("errors", {}).values())


def fr_finished(f):
    return f is not None and f.get("status") == "finished" and (f.get("native") or {}).get("unconnected") is not None


def ours_finished(o):
    return o is not None and (o.get("native") or {}).get("unconnected") is not None


def fr_state(f):
    if f is None:
        return "missing"
    if fr_finished(f):
        return "finished"
    if f.get("status") == "timeout":
        return "timeout"
    return "failed"


def verdict(o, f):
    """Who did better on the board: 'ours', 'freerouting', 'tie', or
    'ours (FR no result)' / 'none' when a side has no judged result."""
    if not ours_finished(o):
        return "freerouting (ours no result)" if fr_finished(f) else "none"
    if not fr_finished(f):
        return "ours (FR no result)"
    a = (o["native"]["unconnected"], errors(o["native"]))
    b = (f["native"]["unconnected"], errors(f["native"]))
    return "ours" if a < b else "freerouting" if b < a else "tie"


def cell_fr(f):
    if f is None:
        return "-"
    if fr_finished(f):
        cpu = f" {f['cpu_seconds']:.0f} cpu" if "cpu_seconds" in f else ""
        return f"{f['native']['unconnected']} open +{errors(f['native'])}e {f.get('router_seconds', 0):.0f} s{cpu}"
    return f"{f.get('status')} {f.get('router_seconds', 0):.0f} s"


def cell_ours(o):
    if o is None:
        return "-"
    if not ours_finished(o):
        return f"error (exit {o.get('exit')}) {o.get('wall_seconds', 0):.0f} s"
    cpu = f" {o['cpu_seconds']:.0f} cpu" if "cpu_seconds" in o else ""
    return f"{o['routed']}/{o['connections']} {o['native']['unconnected']} open +{errors(o['native'])}e {o.get('wall_seconds', 0):.0f} s{cpu}"


def summary(ours, theirs, label):
    names = sorted(name for name in ours if name in theirs)
    states = {"finished": 0, "timeout": 0, "failed": 0}
    for name in names:
        states[fr_state(theirs[name])] = states.get(fr_state(theirs[name]), 0) + 1
    ours_ok = sum(ours_finished(ours[n]) for n in names)
    both = [n for n in names if fr_finished(theirs[n]) and ours_finished(ours[n])]
    s = {"boards": len(names), "states": states, "ours_results": ours_ok, "both": len(both)}
    fewer_open = [0, 0]
    fewer_err = [0, 0]
    verdicts = {"ours": 0, "freerouting": 0, "tie": 0}
    totals = {side: {"open": 0, "errors": 0, "clean": 0, "vias": 0, "length": 0.0, "cpu": 0.0, "cpu_pipeline": 0.0,
                     "cpu_rows": 0} for side in ("fr", "ours")}
    for n in both:
        f, o = theirs[n], ours[n]
        fo, oo = f["native"]["unconnected"], o["native"]["unconnected"]
        fe, oe = errors(f["native"]), errors(o["native"])
        fewer_open[0] += oo < fo
        fewer_open[1] += fo < oo
        fewer_err[0] += oe < fe
        fewer_err[1] += fe < oe
        verdicts[verdict(o, f)] += 1
        for side, row, opened, errs in (("fr", f, fo, fe), ("ours", o, oo, oe)):
            t = totals[side]
            t["open"] += opened
            t["errors"] += errs
            t["clean"] += opened == 0 and errs == 0
            t["vias"] += row.get("vias") or 0
            t["length"] += row.get("length_mm") or 0.0
            if "cpu_seconds" in row:
                t["cpu"] += row["cpu_seconds"]
                t["cpu_pipeline"] += row["cpu_seconds"] + row.get("pipeline_cpu_seconds", 0.0)
                t["cpu_rows"] += 1
    s.update(fewer_open=fewer_open, fewer_errors=fewer_err, verdicts=verdicts, totals=totals)
    # Clean boards over every board (a side without a result is not clean).
    s["clean_all"] = {
        "fr": sum(fr_finished(theirs[n]) and theirs[n]["native"]["unconnected"] == 0 and errors(theirs[n]["native"]) == 0
                  for n in names),
        "ours": sum(ours_finished(ours[n]) and ours[n]["native"]["unconnected"] == 0 and errors(ours[n]["native"]) == 0
                    for n in names),
    }
    s["ours_open_all"] = sum(ours[n]["native"]["unconnected"] for n in names if ours_finished(ours[n]))
    s["ours_cpu_all"] = sum(ours[n].get("cpu_seconds", 0.0) for n in names)
    s["fr_cpu_all"] = sum(theirs[n].get("cpu_seconds", 0.0) for n in names)
    s["label"] = label
    return s


def print_summary(s):
    st = s["states"]
    t = s["totals"]
    print(f"{s['label']}: {s['boards']} boards; Freerouting finished {st['finished']}, timed out {st['timeout']}, "
          f"failed {st['failed']}; pcb-maker has a judged result on {s['ours_results']}")
    print(f"  both finished: {s['both']} boards; fewer unconnected: pcb-maker {s['fewer_open'][0]}, Freerouting "
          f"{s['fewer_open'][1]}; fewer DRC errors: pcb-maker {s['fewer_errors'][0]}, Freerouting {s['fewer_errors'][1]}; "
          f"verdict (unconnected, then errors): pcb-maker {s['verdicts']['ours']}, Freerouting {s['verdicts']['freerouting']}, "
          f"tie {s['verdicts']['tie']}")
    for side, name in (("fr", "Freerouting"), ("ours", "pcb-maker")):
        x = t[side]
        cpu = (f"; CPU {x['cpu']:.0f} s" + (f" (+pipeline {x['cpu_pipeline']:.0f} s)" if side == "fr" else "")
               + (f" on {x['cpu_rows']} rows" if x["cpu_rows"] != s["both"] else "")) if x["cpu_rows"] else "; CPU not recorded"
        print(f"  {name:11} on those: unconnected {x['open']}, errors {x['errors']}, clean {x['clean']}, vias {x['vias']}, "
              f"length {x['length'] / 1000:.1f} m{cpu}")
    print(f"  clean boards over all {s['boards']}: Freerouting {s['clean_all']['fr']}, pcb-maker {s['clean_all']['ours']}; "
          f"pcb-maker unconnected over all boards {s['ours_open_all']}"
          + (f"; CPU over all boards: Freerouting {s['fr_cpu_all']:.0f} s, pcb-maker {s['ours_cpu_all']:.0f} s"
             if s["ours_cpu_all"] else ""))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("ours")
    parser.add_argument("freerouting", nargs="+")
    parser.add_argument("--ours-splice", nargs="*", default=[],
                        help="later logs whose cold_route rows replace the first log's (reruns)")
    parser.add_argument("--previous-ours")
    parser.add_argument("--previous-freerouting", nargs="*", default=[])
    parser.add_argument("--markdown", action="store_true")
    parser.add_argument("--summary-only", action="store_true")
    arguments = parser.parse_args()
    ours = merged([arguments.ours, *arguments.ours_splice], "cold_route")
    theirs = merged(arguments.freerouting, "freerouting")
    common = sorted(name for name in ours if name in theirs)
    sep = " | " if arguments.markdown else "  "
    if not arguments.summary_only:
        width = max((len(n) for n in common), default=10)
        print(f"{'board':{width}}{sep}{'Freerouting':>38}{sep}{'pcb-maker':>42}")
        if arguments.markdown:
            print(f"{'-' * width} | {'-' * 38} | {'-' * 42}")
        for name in common:
            print(f"{name:{width}}{sep}{cell_fr(theirs[name]):>38}{sep}{cell_ours(ours[name]):>42}")
        print()
    now = summary(ours, theirs, "this round")
    print_summary(now)
    if not arguments.previous_ours:
        return
    old_ours = merged([arguments.previous_ours], "cold_route")
    old_theirs = merged(arguments.previous_freerouting or [arguments.previous_ours], "freerouting")
    print()
    print_summary(summary(old_ours, old_theirs, "previous round"))

    # Our cold route then and now.
    shared = sorted(n for n in common if n in old_ours)
    better = same = worse = 0
    open_then = open_now = clean_then = clean_now = 0
    for n in shared:
        a, b = old_ours[n], ours[n]
        if not (ours_finished(a) and ours_finished(b)):
            continue
        ka = (a["native"]["unconnected"], errors(a["native"]))
        kb = (b["native"]["unconnected"], errors(b["native"]))
        better += kb < ka
        worse += kb > ka
        same += kb == ka
        open_then += ka[0]
        open_now += kb[0]
        clean_then += ka == (0, 0)
        clean_now += kb == (0, 0)
    print()
    print(f"pcb-maker cold route, previous round -> this round on {better + same + worse} boards: better {better}, "
          f"same {same}, worse {worse}; unconnected {open_then} -> {open_now}; clean {clean_then} -> {clean_now}")
    fr_moved = sum(1 for n in shared if n in old_theirs and fr_state(old_theirs[n]) != fr_state(theirs[n]))
    print(f"Freerouting state changed (finished/timeout/failed) on {fr_moved} boards")

    flips = [n for n in shared if verdict(old_ours.get(n), old_theirs.get(n)) != verdict(ours[n], theirs[n])]
    if flips:
        print()
        print(f"verdict flipped on {len(flips)} boards:")
        width = max(len(n) for n in flips)
        header = f"{'board':{width}}{sep}{'then':>28}{sep}{'now':>28}{sep}{'FR then':>24}{sep}{'FR now':>30}{sep}{'ours then':>34}{sep}{'ours now':>42}"
        print(header)
        if arguments.markdown:
            print(" | ".join("-" * w for w in (width, 28, 28, 24, 30, 34, 42)))
        for n in flips:
            print(f"{n:{width}}{sep}{verdict(old_ours.get(n), old_theirs.get(n)):>28}{sep}{verdict(ours[n], theirs[n]):>28}{sep}"
                  f"{cell_fr(old_theirs.get(n)):>24}{sep}{cell_fr(theirs[n]):>30}{sep}{cell_ours(old_ours.get(n)):>34}{sep}"
                  f"{cell_ours(ours[n]):>42}")


if __name__ == "__main__":
    main()
