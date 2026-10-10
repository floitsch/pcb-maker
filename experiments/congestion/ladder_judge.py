#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""The probe ladder's judge: every rung of a board continued to its final
(route mode, PCB_LADDER_FORCE_RUNG), its probe statistics
(PCB_LADDER_STATS, PCB_LADDER_SHORT_RESUME) and its final row.

    experiments/congestion/ladder_judge.py build/ladder-judge"""

import re
import sys
from pathlib import Path

NAMES = {0: "connect, exclusive planes", 1: "connect", 2: "connect, plane stubs", 3: "tracks"}


def main():
    root = Path(sys.argv[1])
    boards = sorted({re.sub(r"__rung\d+\.log$", "", p.name) for p in root.glob("*__rung*.log")})
    totals = {"probe": 0, "conflicted": 0, "incomplete": 0, "short": 0}
    counted = 0
    for board in boards:
        stats, finals, short = {}, {}, {}
        for log in root.glob(f"{board}__rung*.log"):
            rung = int(re.search(r"rung(\d+)\.log$", log.name).group(1))
            text = log.read_text(errors="replace")
            for match in re.finditer(r"ladder stats: rung (\d+) unfinished (\d+) conflicted (\d+) incomplete (\d+)", text):
                stats[int(match.group(1))] = tuple(int(v) for v in match.groups()[1:])
            match = re.search(r"ladder stats: rung (\d+) short resume [\d.]+ s: (\d+) open", text)
            if match:
                short[int(match.group(1))] = int(match.group(2))
            match = re.search(r"\(probed, continued\): (\d+) open, (\d+) starved, (\d+) vias", text)
            if match:
                finals[rung] = tuple(int(v) for v in match.groups())
        print(f"{board}:")
        for rung in sorted(stats):
            final = finals.get(rung)
            print(f"  rung {rung} ({NAMES[rung]}): probe unfinished {stats[rung][0]}, conflicted {stats[rung][1]}, "
                  f"incomplete {stats[rung][2]}, open after a 30 s continuation {short.get(rung, '-')}; final: "
                  + (f"{final[0]} open, {final[1]} starved, {final[2]} vias" if final else "not finished"))
        done = [rung for rung in finals if rung in stats]
        if len(done) < 2:
            continue
        counted += 1
        # Best final: fewest open, then fewest starved, then fewest vias.
        best = min(finals[rung] for rung in done)
        right = {rung for rung in done if finals[rung] == best}
        picks = {
            "probe": min(done, key=lambda r: (stats[r][0], r)),
            "conflicted": min(done, key=lambda r: (stats[r][1], r)),
            "incomplete": min(done, key=lambda r: (stats[r][2], r)),
            "short": min((r for r in done if r in short), key=lambda r: (short[r], r), default=None),
        }
        print(f"  best final: rung {sorted(right)}; picked by " + ", ".join(f"{k}: {v}{' (right)' if v in right else ''}" for k, v in picks.items()))
        for key, rung in picks.items():
            totals[key] += rung in right
    print(f"right picks over {counted} boards: " + ", ".join(f"{k} {v}" for k, v in totals.items()))


if __name__ == "__main__":
    main()
