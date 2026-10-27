#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""What the headroom samples of a busy night say about the floor (ledger6,
2026-10-27). The samples are JSON lines of the read-only sampler
(experiments/ledger-floor/sampler.py): every 10 s MemTotal, MemAvailable,
swap, the ledger's entries with each holder's resident memory (its own and
its process tree's) and the memory outside the ledger (MemTotal less
MemAvailable less the holders' trees).

Prints:
- the outside memory's distribution and its rises within 1, 10, 30 and 60
  minutes of a sample (harness_common.outside_rises);
- the runs' memory above their reservations (tree less megabytes), summed;
- the floor's view, MemAvailable less the reservations' growth, and
  MemAvailable itself;
- the least MemAvailable, and the floor that would have kept it above
  2 GB at every sample given the admissions that happened (a floor lower
  by D lets the harnesses commit at most D more);
- per harness admission (a new holder with an expected end: run.py's
  boards) how far the outside memory and the runs above their
  reservations rose within 10 minutes;
- the waits of run.py's boards held by the floor alone (`waits_on_floor`)
  and how many of their seconds a lower floor would have saved (the
  sample's MemAvailable less growth less the board at or above it).

    python3 experiments/ledger-floor/samples.py build/ledger6-work/samples.jsonl [--floor 5300]
"""

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "benchmarks"))
import harness_common  # noqa: E402

q = harness_common.nearest_rank


def quantiles(values, points=(0.05, 0.5, 0.9, 0.95, 0.99, 1.0)):
    return ", ".join(f"p{round(p * 100)} {q(values, p) / 1024:.1f}" for p in points) if values else "none"


def growth(sample):
    return sum(max(0.0, e["megabytes"] - e["rss"]) for e in sample["entries"] if (e.get("megabytes") or 0) > 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("samples", type=Path)
    parser.add_argument("--floor", type=float, action="append", default=[],
                        help="floors (MB) to count the floor-held waits against")
    arguments = parser.parse_args()
    samples = [json.loads(line) for line in arguments.samples.read_text().splitlines() if line.strip()]
    samples.sort(key=lambda s: s["t"])
    t0 = samples[0]["t"]
    span = samples[-1]["t"] - t0
    print(f"{len(samples)} samples over {span / 3600:.2f} h; MemTotal {samples[0]['total'] / 1024:.1f} GB")
    outside = [s["outside"] for s in samples]
    print(f"outside the ledger (GB): {quantiles(outside)}")
    compact = [[s["t"], s["available"], s["outside"]] for s in samples]
    for window in (60, 600, 1800, 3600):
        print(f"  its rises within {window // 60:2} min (GB): {quantiles(harness_common.outside_rises(compact, window))}")
    # (a harness's own reservation, run.py: ..., is left out: its tree holds
    # its boards' binaries, which hold their own)
    over = [sum(max(0.0, e["tree"] - e["megabytes"]) for e in s["entries"]
                if (e.get("megabytes") or 0) > 0 and not e["label"].startswith("run.py:"))
            for s in samples]
    print(f"the runs above their reservations, summed (GB): {quantiles(over)}")
    view = [s["available"] - growth(s) for s in samples]
    print(f"MemAvailable less growth (GB): {quantiles(view)}")
    print(f"MemAvailable (GB): {quantiles([s['available'] for s in samples])}; "
          f"reserved: {quantiles([s['reserved'] for s in samples])}; "
          f"the holders' trees: {quantiles([s['ledger_rss'] for s in samples])}; "
          f"swap used: {quantiles([s['swap_used'] for s in samples])}")
    tops = {}
    for s in samples:
        for name, megabytes in s["top"]:
            tops.setdefault(name, []).append(megabytes)
    print("outside, by command (GB, median and largest): " + ", ".join(
        f"{name} {q(v, 0.5) / 1024:.1f}/{max(v) / 1024:.1f}"
        for name, v in sorted(tops.items(), key=lambda kv: -max(kv[1]))[:8]))

    # The floor that would have kept MemAvailable above 2 GB given the
    # admissions that happened: a harness with a floor lower by D commits
    # at most D more than it did (every harness checks the one
    # MemAvailable less growth, and an admission lowers it at once by the
    # board's reservation), so MemAvailable would have been at most D
    # lower at any sample.
    used = 10240.0
    lowest = min(samples, key=lambda s: s["available"])
    print(f"the least MemAvailable: {lowest['available'] / 1024:.1f} GB at +{(lowest['t'] - t0) / 3600:.2f} h "
          f"(outside {lowest['outside'] / 1024:.1f}, the holders' trees {lowest['ledger_rss'] / 1024:.1f}, "
          f"reserved {lowest['reserved'] / 1024:.1f}); with the harnesses' floor of {used / 1024:.0f} GB, any floor "
          f"of at least {max(0.0, used + 2048 - lowest['available']) / 1024:.1f} GB keeps 2 GB at every sample")
    # Per harness admission (a new holder with an expected end: run.py's
    # boards) what the ledger does not count grew by within 10 minutes:
    # the outside memory and the runs above their reservations.
    seen = set()
    rises = []
    for index in range(1, len(samples)):
        before = samples[index - 1]
        for e in samples[index]["entries"]:
            if (e.get("megabytes") or 0) <= 0 or e.get("expected_end") is None:
                continue
            key = (e["pid"], e.get("held_since"))
            fresh = key not in seen and not any(o["pid"] == e["pid"] and (o.get("megabytes") or 0) > 0
                                                for o in before["entries"])
            seen.add(key)
            if not fresh:
                continue
            base = before["outside"] + over[index - 1]
            later = [s["outside"] + over[j] for j, s in enumerate(samples[index:], index)
                     if s["t"] - before["t"] <= 600]
            rises.append(max(0.0, max(later) - base))
    print(f"{len(rises)} harness admissions; the outside memory and the runs above their reservations rose "
          f"within 10 min of one by (GB): {quantiles(rises)}")

    # The waits held by the floor alone.
    waits = {}
    for index, s in enumerate(samples):
        view_now = s["available"] - growth(s)
        for e in s["entries"]:
            if e.get("waiting_mb") is None or not e["label"].startswith("run.py:"):
                continue
            key = (e["pid"], e["label"], e.get("waiting_since"))
            w = waits.setdefault(key, {"mb": e["waiting_mb"], "floor": e.get("floor_mb"), "samples": [],
                                       "first": s["t"], "last": s["t"]})
            w["last"] = s["t"]
            w["samples"].append((s["t"], view_now - e["waiting_mb"], bool(e.get("waits_on_floor"))))
    held = [w for w in waits.values() if any(on for _, _, on in w["samples"])]
    total = sum(w["last"] - w["first"] + 10 for w in waits.values())
    on_floor = sum(10 for w in waits.values() for _, _, on in w["samples"] if on)
    print(f"{len(waits)} run.py waits seen ({total / 3600:.1f} board-hours), {len(held)} held by the floor alone at "
          f"some sample ({on_floor / 3600:.1f} board-hours on the floor)")
    for floor in sorted(set(arguments.floor) | {10240.0, 8192.0, 6144.0}):
        saved = sum(10 for w in waits.values() for _, slack, on in w["samples"] if on and slack >= floor)
        boards = sum(1 for w in held if any(on and slack >= floor for _, slack, on in w["samples"]))
        print(f"  a floor of {floor / 1024:.1f} GB: {boards} of them could have started earlier, "
              f"{saved / 3600:.1f} board-hours of floor-held waiting had room")


if __name__ == "__main__":
    main()
