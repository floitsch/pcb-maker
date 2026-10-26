#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""How far a board's measured peak exceeds what run.py's gate declared for
it (ledger6, 2026-10-27): a time-ordered replay of the estimates.

For every source (a run.py log of one kind) in time order the sidecar is
rebuilt from the sources before it (tier_estimates.refresh, the committed
rule: the largest of the newest three rows times 1.25, else the fallbacks
and the size model), and each of the source's rows is compared with what
`declared` would have reserved for it then. Prints per kind the quantiles
of peak minus expected (MB) and how many rows exceeded their reservation;
`--sidecar` writes the replay's last sidecar (its `overshoot` section is
the committed estimates.json's).

    python3 experiments/ledger-floor/overshoot.py [--json out.json] LOG:KIND ...
    (no logs: the committed sidecar's sources and the newest night's stages)
"""

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "benchmarks" / "agent-tasks"))
import tier_estimates  # noqa: E402


def quantile(values, q):
    values = sorted(values)
    if not values:
        return None
    return values[min(len(values) - 1, max(0, round(q * (len(values) - 1))))]


def replay(sources, sizes):
    """Folds tier_estimates.refresh over `sources` one at a time, in time
    order (each source's rows against what the sources before it declared:
    the sidecar's `overshoot`). Returns per kind every source's overshoot
    values and the last sidecar."""
    ordered = sorted(sources, key=lambda source: tier_estimates.source_time(source[0]))
    sidecar = None
    found = {}
    for path, kind in ordered:
        sidecar = tier_estimates.refresh([sidecar], [(path, kind)], sizes)
        label = tier_estimates.source_label(path)
        entry = (sidecar.get("overshoot") or {}).get(kind, {}).get("sources", {}).get(label)
        if entry:
            found.setdefault(kind, []).extend(entry["over"])
    return found, sidecar


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("logs", nargs="*", help="LOG:KIND")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--sidecar", type=Path, help="write the last sidecar of the replay here")
    arguments = parser.parse_args()
    sidecar = tier_estimates.load(tier_estimates.DEFAULT_ESTIMATES)
    sizes = {name: (board["pads"], board["layers"]) for name, board in sidecar["boards"].items()
             if board.get("pads") is not None}
    if arguments.logs:
        sources = [(Path(log.rsplit(":", 1)[0]), log.rsplit(":", 1)[1]) for log in arguments.logs]
    else:
        sources = [(ROOT / label if not label.startswith("build/") else ROOT / label, source["kind"])
                   for label, source in sidecar["sources"].items()]
        night = tier_estimates.newest_night(ROOT / "build" / "nightly")
        sources += [source for source in tier_estimates.night_sources(night) if source[0].stat().st_size]
    sources = [(path, kind) for path, kind in sources if path.is_file()]
    found, sidecar = replay(sources, sizes)
    summary = {}
    print(f"{'kind':10} {'rows':>5} {'over':>5} {'p50':>7} {'p90':>7} {'p95':>7} {'p99':>7} {'max':>7}"
          "  (peak - expected, MB; every source)")
    for kind, overs in sorted(found.items()):
        s = {"rows": len(overs), "exceeded": sum(1 for value in overs if value > 0),
             **{f"p{int(q * 100)}": quantile(overs, q) for q in (0.5, 0.9, 0.95, 0.99)}, "max": max(overs)}
        summary[kind] = s
        print(f"{kind:10} {s['rows']:5} {s['exceeded']:5} {s['p50']:7} {s['p90']:7} {s['p95']:7} {s['p99']:7} "
              f"{s['max']:7}")
    print("the sidecar's overshoot (the newest three sources per kind): " + ", ".join(
        f"{kind} {entry['p95_mb']} MB ({entry['rows']} rows, {entry['exceeded']} over)"
        for kind, entry in sidecar["overshoot"].items()))
    if arguments.json:
        arguments.json.write_text(json.dumps(summary, indent=2) + "\n")
    if arguments.sidecar:
        arguments.sidecar.write_text(tier_estimates.dumps(sidecar))


if __name__ == "__main__":
    main()
