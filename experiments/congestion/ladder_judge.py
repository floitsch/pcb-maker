#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""The probe ladder's judge: every rung of a board continued to its final
(route mode, PCB_LADDER_FORCE_RUNG), its probe statistics
(PCB_LADDER_STATS, PCB_LADDER_SHORT_RESUME) and its final row.

    experiments/congestion/ladder_judge.py build/ladder-judge [more directories]

Each directory holds <board>__rung<N>.log and the run's output directory
<board>__rung<N>/ (KiCad's unconnected count from verification.json). A
judge picks the rung with the lowest statistic, ties to the earlier rung (a
rung at 0 ends the probing, which picks the same rung). A pick is right
when no other rung's final is better by the criterion."""

import json
import re
import sys
from pathlib import Path

NAMES = {0: "connect, exclusive planes", 1: "connect", 2: "connect, plane stubs", 3: "tracks"}
JUDGES = ["unfinished", "conflicted", "incomplete", "nets", "short"]


def board_rows(root, board):
    stats, finals, short = {}, {}, {}
    for log in root.glob(f"{board}__rung*.log"):
        rung = int(re.search(r"rung(\d+)\.log$", log.name).group(1))
        text = log.read_text(errors="replace")
        for match in re.finditer(
                r"ladder stats: rung (\d+) unfinished (\d+) conflicted (\d+) incomplete (\d+)(?: nets (\d+) stranded (\d+))?", text):
            values = [int(v) if v is not None else None for v in match.groups()[1:]]
            stats[int(match.group(1))] = dict(zip(["unfinished", "conflicted", "incomplete", "nets", "stranded"], values))
        match = re.search(r"ladder stats: rung (\d+) short resume [\d.]+ s: (\d+) open", text)
        if match:
            short[int(match.group(1))] = int(match.group(2))
        match = re.search(r"\(probed, continued\): (\d+) open, (\d+) starved, (\d+) vias", text)
        if match:
            open_, starved, vias = (int(v) for v in match.groups())
            kicad = None
            verification = root / f"{board}__rung{rung}" / "verification.json"
            if verification.exists():
                kicad = json.loads(verification.read_text()).get("selected_net_unconnected_items")
            finals[rung] = {"open": open_, "starved": starved, "vias": vias, "kicad": kicad}
    for rung in stats:
        if rung in short:
            stats[rung]["short"] = short[rung]
    return stats, finals


CRITERIA = {
    "open": lambda f: (f["open"],),
    "open, starved": lambda f: (f["open"], f["starved"]),
    "open, starved, vias": lambda f: (f["open"], f["starved"], f["vias"]),
}


def main():
    roots = [Path(argument) for argument in sys.argv[1:]]
    right = {criterion: {judge: 0 for judge in JUDGES} for criterion in CRITERIA}
    regret = {judge: [0, 0] for judge in JUDGES}
    counted = {judge: 0 for judge in JUDGES}
    for root in roots:
        boards = sorted({re.sub(r"__rung\d+\.log$", "", p.name) for p in root.glob("*__rung*.log")})
        for board in boards:
            stats, finals = board_rows(root, board)
            done = sorted(rung for rung in finals if rung in stats)
            print(f"{root.name}/{board}:")
            for rung in sorted(stats):
                s, f = stats[rung], finals.get(rung)
                print(f"  rung {rung} ({NAMES[rung]}): probe unfinished {s['unfinished']}, conflicted {s['conflicted']}, "
                      f"incomplete {s['incomplete']}, nets {s.get('nets', '-')}, stranded pour pads {s.get('stranded', '-')}, "
                      f"open after 30 s more {s.get('short', '-')}; final: "
                      + (f"{f['open']} open (KiCad unconnected {f['kicad']}), {f['starved']} starved, {f['vias']} vias" if f else "not finished"))
            if len(done) < 2:
                continue
            best_open = min(finals[r]["open"] for r in done)
            best_starved = min(finals[r]["starved"] for r in done if finals[r]["open"] == best_open)
            picks = {}
            for judge in JUDGES:
                if any(stats[r].get(judge) is None for r in done):
                    continue
                pick = min(done, key=lambda r: (stats[r][judge], r))
                picks[judge] = pick
                counted[judge] += 1
                regret[judge][0] += finals[pick]["open"] - best_open
                if finals[pick]["open"] == best_open:
                    regret[judge][1] += finals[pick]["starved"] - best_starved
                for criterion, key in CRITERIA.items():
                    best = min(key(finals[r]) for r in done)
                    right[criterion][judge] += key(finals[pick]) == best
            best = min(CRITERIA["open, starved, vias"](finals[r]) for r in done)
            print(f"  best final (open, starved, vias): rung {[r for r in done if CRITERIA['open, starved, vias'](finals[r]) == best]}; "
                  "picked by " + ", ".join(f"{judge}: {rung} ({finals[rung]['open']} open, {finals[rung]['starved']} starved)" for judge, rung in picks.items()))
    print()
    for criterion in CRITERIA:
        print(f"right by ({criterion}): " + ", ".join(f"{judge} {right[criterion][judge]}/{counted[judge]}" for judge in JUDGES))
    print("open above the best rung's, summed (and starved above the best where open ties): "
          + ", ".join(f"{judge} +{regret[judge][0]} (+{regret[judge][1]} starved)" for judge in JUDGES))


if __name__ == "__main__":
    main()
