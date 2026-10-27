#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Which term held run.py's boards back (ledger6, 2026-10-27): the waiting
lines of harness_common.admit in run logs.

A board that does not fit at once logs `<label>: waiting for memory (...)`
with the ledger's numbers when its wait began and `<label>: started after
waiting N s (...)` with them when it started. At the wait's beginning:

- ledger: the reservations and the board exceed the capacity;
- floor: the ledger has room, but MemAvailable less the reservations'
  growth less the board leaves less than the floor ("to keep");
- order: both hold, it waits behind an older request (or a drain).

For the waits held by the floor alone, the slack (MemAvailable less growth
less the board) is the floor that would have started the board at once.

    python3 experiments/ledger-floor/waits.py [--since 2026-10-22] LOG ...
    (no logs: every *.err and *.log under build/ with such lines)
"""

import argparse
import datetime
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
NUMBERS = re.compile(
    r"\((?P<reserved>[\d.]+) GB reserved by \d+ run\(s\) of (?P<capacity>[\d.]+) GB(?:, \d+ waiting)?, "
    r"(?P<available>[\d.]+) GB available \((?P<growth>[\d.]+) GB still to grow\), "
    r"(?P<expected>[\d.]+) GB expected, (?P<floor>[\d.]+) GB to keep(?P<rest>.*)\)\s*$")
WAITING = re.compile(r"^(?P<label>.+?): waiting for memory (?P<numbers>\(.*)$")
STARTED = re.compile(r"^(?P<label>.+?): started after waiting (?P<seconds>\d+) s (?P<numbers>\(.*)$")


def numbers(text):
    match = NUMBERS.match(text)
    if not match:
        return None
    values = {key: float(value) for key, value in match.groupdict().items() if key != "rest"}
    values["behind"] = "behind an older request" in match.group("rest")
    return values


def term(values):
    if values["reserved"] + values["expected"] > values["capacity"] + 0.05:
        return "ledger"
    if values["available"] - values["growth"] - values["expected"] < values["floor"] - 0.05:
        return "floor"
    return "order"


def waits(paths, since):
    found = []
    for path in paths:
        if datetime.datetime.fromtimestamp(path.stat().st_mtime) < since:
            continue
        open_waits = {}
        for line in path.read_text(errors="replace").splitlines():
            if match := WAITING.match(line):
                values = numbers(match.group("numbers"))
                if values:
                    open_waits[match.group("label")] = values
            elif match := STARTED.match(line):
                begun = open_waits.pop(match.group("label"), None)
                ended = numbers(match.group("numbers"))
                if begun and ended:
                    found.append({"file": str(path), "label": match.group("label"),
                                  "seconds": int(match.group("seconds")), "begun": begun, "ended": ended,
                                  "term": term(begun)})
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("logs", nargs="*", type=Path)
    parser.add_argument("--since", default="2026-10-22")
    arguments = parser.parse_args()
    paths = arguments.logs or sorted(p for pattern in ("*.err", "*.log") for p in (ROOT / "build").rglob(pattern)
                                     if p.is_file())
    since = datetime.datetime.fromisoformat(arguments.since)
    found = waits(paths, since)
    print(f"{len(found)} waits ({sum(w['seconds'] for w in found)} s) in "
          f"{len({w['file'] for w in found})} logs since {arguments.since}")
    for name in ("ledger", "floor", "order"):
        chosen = [w for w in found if w["term"] == name]
        seconds = sorted(w["seconds"] for w in chosen)
        if not chosen:
            continue
        print(f"  {name:6}: {len(chosen):5} waits, {sum(seconds):8} s, median {seconds[len(seconds) // 2]} s, "
              f"max {seconds[-1]} s")
    floor = [w for w in found if w["term"] == "floor"]
    floors = sorted({w["begun"]["floor"] for w in floor})
    print(f"held by the floor alone (floors {floors} GB): the slack, MemAvailable less growth less the board, "
          f"at the wait's beginning")
    for limit in (8.8, 8, 6, 4, 2):
        started = [w for w in floor
                   if w["begun"]["available"] - w["begun"]["growth"] - w["begun"]["expected"] >= limit]
        print(f"  a floor of {limit} GB starts {len(started):4} of {len(floor)} at once "
              f"({sum(w['seconds'] for w in started)} s of {sum(w['seconds'] for w in floor)} s waited)")
    slack = sorted(w["begun"]["available"] - w["begun"]["growth"] - w["begun"]["expected"] for w in floor)
    if slack:
        print("  slack quantiles (GB): " + ", ".join(
            f"p{q}: {slack[min(len(slack) - 1, round(q / 100 * (len(slack) - 1)))]:.1f}" for q in (5, 25, 50, 75, 95)))
    longest = sorted(floor, key=lambda w: -w["seconds"])[:8]
    for w in longest:
        b = w["begun"]
        print(f"    {w['seconds']:6} s {w['label'][:60]:60} {b['available']:.1f} avail - {b['growth']:.1f} growth "
              f"- {b['expected']:.1f} = {b['available'] - b['growth'] - b['expected']:.1f} < {b['floor']:.0f} "
              f"({Path(w['file']).name})")


if __name__ == "__main__":
    main()
