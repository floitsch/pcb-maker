#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""The night's run.py boards that the floor held back, replayed with other
floors against the machine as it was without them (ledger6, 2026-10-27).

From the read-only samples (sampler.py): the boards whose waiting entry
said, at some sample, that only the floor held them back (`waits_on_floor`),
when they began waiting, their expected peak, when they started (the
holder whose label names the board and whose holding began then) and
their measured resident memory (their process tree) until they ended. The
machine without them: the floor's view (MemAvailable less growth) with their
megabytes added back, and MemAvailable with their trees added back. A
board looks every 15 s from when it began waiting, in the order the night
started them, and starts when the view less the replayed boards running
less its expected peak leaves the floor; it then uses the memory it used
that night, shifted (a board that did not start within the samples holds
its expected peak until the end). The other runs are replayed as they
happened (a board started earlier would have let them go earlier too:
not counted).

    python3 experiments/ledger-floor/replay.py build/ledger6-work/samples.jsonl --floor 5300 [--knots 300]
    python3 experiments/ledger-floor/replay.py SAMPLES --capacity [MB]

`--capacity` (ledger7, 2026-10-27): the replays again with the ledger's
capacity from the machine (MemTotal less the samples' median outside memory,
or the megabytes given): a board then starts only when the other runs'
reservations at that sample, the boards running and its own fit it too; and
how the runs that held memory in the samples sat against it (the samples
with more reserved than it, the binaries admitted by the capacity alone
that the floor would have held: a new holder without an expected end whose
first sample's view less its megabytes was below the floor).

`--knots S` prints the machine without the boards as steps of S seconds
(each the least view and MemAvailable within it) and the boards, for the
tests (test_run.py, ledger.rs: `tonight`).
"""

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "benchmarks"))
import harness_common  # noqa: E402


def growth(sample):
    return sum(max(0.0, e["megabytes"] - e["rss"]) for e in sample["entries"] if (e.get("megabytes") or 0) > 0)


def name_of(label):
    return label[len("run.py: "):].rsplit(" (expected peak)", 1)[0]


def boards_held_by_the_floor(samples):
    waits = {}
    for index, s in enumerate(samples):
        for e in s["entries"]:
            if e.get("waiting_mb") is None or not e["label"].startswith("run.py:"):
                continue
            key = (e["pid"], e["label"], e.get("waiting_since"))
            w = waits.setdefault(key, {"pid": e["pid"], "name": name_of(e["label"]), "mb": e["waiting_mb"],
                                       "since": round(e.get("waiting_since") or s["t"]), "floor": e.get("floor_mb"),
                                       "on_floor": False, "last_index": index})
            w["last_index"] = index
            w["on_floor"] = w["on_floor"] or bool(e.get("waits_on_floor"))
    boards = [w for w in waits.values() if w["on_floor"]]
    for board in boards:
        # Its start: a holder naming the board whose holding began after
        # its last waiting sample (run.py's entry, or the binary's after the
        # move), within a minute.
        last = samples[board["last_index"]]["t"]
        board["start"] = board["profile"] = None
        for s in samples[board["last_index"] + 1:]:
            if s["t"] - last > 60:
                break
            for e in s["entries"]:
                if (e.get("megabytes") or 0) > 0 and board["name"] in e["label"] and \
                        (e.get("held_since") or 0) >= last - 20:
                    board["start"], board["holder"] = e.get("held_since") or s["t"], e["pid"]
                    break
            if board["start"] is not None:
                break
        if board["start"] is not None:
            profile = []
            for s in samples:
                if s["t"] < board["start"] - 1:
                    continue
                # run.py's entry, then the binary's it was moved to (the
                # same holding time; the binary's label names the board).
                entry = next((e for e in s["entries"] if (e.get("megabytes") or 0) > 0 and (
                    e["pid"] == board["holder"] or (board["name"] in e["label"]
                                                    and abs((e.get("held_since") or 0) - board["start"]) < 3))),
                             None)
                if entry is None and s["t"] > board["start"] + 30:
                    break
                if entry is not None:
                    profile.append((s["t"] - board["start"], entry["megabytes"], entry["tree"]))
            board["profile"] = profile
            board["seconds"] = (profile[-1][0] + 10) if profile else 0
    return sorted(boards, key=lambda b: (b["start"] if b["start"] is not None else float("inf"), b["since"]))


def at(profile, offset):
    """(megabytes, tree) of a board `offset` s into its run, None past it."""
    found = None
    for entry in profile:
        if entry[0] > offset:
            break
        found = entry
    return found[1:] if found is not None and offset < profile[-1][0] + 10 else None


def machine_without(samples, boards):
    series = []
    for s in samples:
        view, available = s["available"] - growth(s), s["available"]
        for board in boards:
            if board["start"] is None:
                continue
            held = at(board["profile"], s["t"] - board["start"]) if s["t"] >= board["start"] else None
            if held:
                view += held[0]
                available += held[1]
        series.append((s["t"], view, available))
    return series


def others_reserved(samples, boards):
    """Per sample, what the runs other than the replayed boards reserved."""
    series = []
    for s in samples:
        reserved = sum(e.get("megabytes") or 0 for e in s["entries"])
        for board in boards:
            if board["start"] is None or s["t"] < board["start"]:
                continue
            held = at(board["profile"], s["t"] - board["start"])
            if held:
                reserved -= held[0]
        series.append(reserved)
    return series


def replay(series, boards, floor_mb, t0, poll=15, worst=False, capacity=None, others=None):
    """Second by second, each board looking every `poll` s from when it
    began waiting against the newest sample; `worst`: every board holds
    its expected peak (reserved and resident) for its measured seconds (to
    the end where it did not start within the samples)."""
    starts = {}
    lowest = float("inf")
    index = 0
    first, last = int(series[0][0]), int(series[-1][0])
    for t in range(first, last + 1):
        while index + 1 < len(series) and series[index + 1][0] <= t:
            index += 1
        _, view, available = series[index]
        running = []
        for board in boards:
            start = starts.get(id(board))
            if start is None:
                continue
            if board["profile"] and not worst:
                held = at(board["profile"], t - start)
                if held:
                    running.append(held)
            elif t - start < (board.get("seconds") or float("inf")):
                running.append((board["mb"], board["mb"]))
        reserved = sum(r[0] for r in running)
        room = float("inf") if capacity is None else capacity - others[index]
        for board in boards:
            if id(board) in starts or t < board["since"] or int(t - board["since"]) % poll:
                continue
            if view - reserved - board["mb"] >= floor_mb and reserved + board["mb"] <= room:
                starts[id(board)] = t
                reserved += board["mb"]
                running.append((board["mb"], board["mb"]))
        lowest = min(lowest, available - sum(r[1] for r in running))
    waited = sum((starts.get(id(b), last) - b["since"]) for b in boards)
    return [None if id(b) not in starts else round(starts[id(b)] - t0) for b in boards], waited, lowest


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("samples", type=Path)
    parser.add_argument("--floor", type=float, action="append", default=[])
    parser.add_argument("--stress", type=float, default=0.0, help="MB less at every sample (the outside's rise)")
    parser.add_argument("--knots", type=float, default=None)
    parser.add_argument("--capacity", type=float, nargs="?", const=-1.0, default=None,
                        help="also with the ledger's capacity (MB; none given: MemTotal less the median outside)")
    arguments = parser.parse_args()
    samples = [json.loads(line) for line in arguments.samples.read_text().splitlines() if line.strip()]
    samples.sort(key=lambda s: s["t"])
    t0 = samples[0]["t"]
    boards = boards_held_by_the_floor(samples)
    print(f"{len(boards)} boards held by the floor alone at some sample:")
    for b in boards:
        peak = max((p[2] for p in b["profile"] or []), default=None)
        print(f"  {b['name'][:50]:50} {b['mb']:6.0f} MB expected, floor {b['floor']}, waited from "
              f"+{b['since'] - t0:.0f} s, started " + ("not within the samples" if b["start"] is None else
                                                      f"+{b['start'] - t0:.0f} s, ran {b['seconds']:.0f} s, "
                                                      f"its tree's peak {peak:.0f} MB"))
    series = machine_without(samples, boards)
    for floor in [10240.0] + arguments.floor:
        for worst in (False, True):
            starts, waited, lowest = replay(series, boards, floor, t0, worst=worst)
            print(f"floor {floor / 1024:.2f} GB{' (each board at its expected peak)' if worst else ''}: starts "
                  f"{starts}, {waited / 3600:.2f} board-hours waited, {sum(s is not None for s in starts)} of "
                  f"{len(boards)} started; the least MemAvailable {lowest / 1024:.1f} GB, "
                  f"less {arguments.stress / 1024:.1f} GB of stress {(lowest - arguments.stress) / 1024:.1f} GB")
    if arguments.capacity is not None:
        outside = sorted(s["outside"] for s in samples)
        capacity = arguments.capacity if arguments.capacity >= 0 else \
            samples[0]["total"] - harness_common.nearest_rank(outside, 0.5)
        others = others_reserved(samples, boards)
        print(f"capacity {capacity / 1024:.1f} GB (MemTotal {samples[0]['total'] / 1024:.1f} GB, the outside memory's "
              f"median {harness_common.nearest_rank(outside, 0.5) / 1024:.1f} GB, least {outside[0] / 1024:.1f})")
        for floor in [10240.0] + arguments.floor:
            for worst in (False, True):
                plain = replay(series, boards, floor, t0, worst=worst)
                starts, waited, lowest = replay(series, boards, floor, t0, worst=worst, capacity=capacity,
                                                others=others)
                print(f"floor {floor / 1024:.2f} GB{' (each board at its expected peak)' if worst else ''} with the "
                      f"capacity: {sum(s is not None for s in starts)} of {len(boards)} started, "
                      f"{waited / 3600:.2f} board-hours waited (without it {plain[1] / 3600:.2f}), the least "
                      f"MemAvailable {lowest / 1024:.1f} GB; the same starts: {starts == plain[0]}")
        over = [s for s in samples if s["reserved"] > capacity]
        print(f"samples with more reserved than the capacity: {len(over)} of {len(samples)}"
              + (f" (most {max(s['reserved'] for s in over) / 1024:.1f} GB)" if over else ""))
        # New holders: their first sample holding, and the floor's view
        # without them then.
        first = {}
        for s in samples:
            for e in s["entries"]:
                key = (e["pid"], e.get("held_since"))
                if (e.get("megabytes") or 0) > 0 and key not in first:
                    first[key] = (s, e)
        known = {(e["pid"], e.get("held_since")) for e in samples[0]["entries"]}
        floor = arguments.floor[0] if arguments.floor else 10240.0
        held = []
        for key, (s, e) in first.items():
            if key in known or e.get("expected_end") is not None or e["label"].startswith("run.py:"):
                continue
            view = s["available"] - growth(s) + max(0.0, e["megabytes"] - e["rss"])
            if view - e["megabytes"] < (2048 if e.get("short") else floor):
                held.append((s["t"] - t0, e["megabytes"], e["label"][:60], view))
        print(f"binaries that began holding within the samples: {sum(1 for k in first if k not in known)}; "
              f"admitted below the floor ({floor / 1024:.1f} GB, a short one 2 GB) a binary keeps since ledger7: "
              f"{len(held)}")
        for t, mb, label, view in held[:20]:
            print(f"  +{t:.0f} s {mb:.0f} MB, the view {view / 1024:.1f} GB: {label}")
    if arguments.knots:
        knots = []
        for t, view, available in series:
            step = int((t - t0) // arguments.knots * arguments.knots)
            if knots and knots[-1][0] == step:
                knots[-1] = (step, min(knots[-1][1], view), min(knots[-1][2], available))
            else:
                knots.append((step, view, available))
        print("knots: [" + ", ".join(f"({t}, {v:.0f}, {a:.0f})" for t, v, a in knots) + "]")
        print("boards: [" + ", ".join(
            f"({b['since'] - t0:.0f}, {b['mb']:.0f}, {b.get('seconds') or 0:.0f})" for b in boards) + "]")


if __name__ == "__main__":
    main()
