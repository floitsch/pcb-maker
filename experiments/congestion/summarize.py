#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""One table of evaluate.py's JSON reports (`--json`): ranking quality on
held-out boards and the simulated race, per model and baseline.

    experiments/congestion/summarize.py build/cdata/models/*.race.json"""

import json
import sys
from pathlib import Path


def main():
    printed_baselines = False
    for path in sys.argv[1:]:
        report = json.loads(Path(path).read_text())["report"]
        name = Path(path).name.split(".")[0]
        rankers = report["rankers"]
        race = report.get("race", {})
        hits = race.get("hit_rates", {})
        total = race.get("total", {})
        names = ["model_open", "model_overflow"] + ([] if printed_baselines else
                                                   ["wirelength", "rudy_sum", "rudy_over", "rudy_bbox_over", "rudy_max", "rudy_top"])
        if not printed_baselines and total:
            print(f"race on {hits.get('boards')} held-out boards ({report['samples']} samples): default (seeds 1-3) {total['default']:.0f}, "
                  f"random@3 {total['random@3']:.0f}, oracle {total['oracle']:.0f}; random hit1-in-best3 {hits['random_hit1_in_best3']:.2f}, "
                  f"best-in-top3 {hits['random_best_in_top3']:.2f}")
        for ranker in names:
            if ranker not in rankers:
                continue
            m = rankers[ranker]
            label = f"{name}:{ranker}" if ranker.startswith("model") else ranker
            line = (f"{label:>34}: within-board Spearman {m['spearman_within']:+.3f}, pairs {m['pair_accuracy']:.3f}, "
                    f"pooled {m['spearman_pooled']:+.3f}")
            if total:
                line += (f" | race@1 {total[ranker + '@1']:.0f} @3 {total[ranker + '@3']:.0f}; hit1-in-best3 "
                         f"{hits[ranker + '_hit1_in_best3']:.2f}, best-in-top3 {hits[ranker + '_best_in_top3']:.2f}")
            print(line)
        if "tiles" in report:
            t = report["tiles"]
            print(f"{'':>34}  tiles: MAE {t['mae']:.3f}, precision {t['precision']:.3f}, recall {t['recall']:.3f} (overflow > {t['threshold']})")
        printed_baselines = True


if __name__ == "__main__":
    main()
