#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Scores held-out samples with the congestion network and with cheap
baselines, and reports how well each ranks placements of the same board.

    build/venv-nn/bin/python experiments/congestion/evaluate.py build/congestion \
        [--model build/congestion/models/m1] [--side test] [--variants task-]

Rankers:
  model_open      the network's scalar head (predicted unfinished nets)
  model_overflow  the network's overflow map summed over the board
  wirelength      half-perimeter wirelength of the placement (the placer's
                  own objective)
  rudy_sum, rudy_over, rudy_max, rudy_top
                  RUDY demand against the tiles' free capacity (see
                  `rudy_scores`); `rudy_over`'s capacity factor is fitted on
                  the training boards
Truth: the probe's unfinished nets (`unfinished`)."""

import argparse
import itertools
import json
import math
import re
import sys
from collections import defaultdict
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dataset import PLANES, SLOTS, channel_names, load_board  # noqa: E402


def spearman(a, b):
    a, b = np.asarray(a, dtype=float), np.asarray(b, dtype=float)
    if len(a) < 3:
        return float("nan")
    ra = np.argsort(np.argsort(a, kind="stable"), kind="stable").astype(float)
    rb = np.argsort(np.argsort(b, kind="stable"), kind="stable").astype(float)
    # Average ranks for ties.
    for values, ranks in ((a, ra), (b, rb)):
        for value in np.unique(values):
            where = values == value
            ranks[where] = ranks[where].mean()
    if ra.std() == 0 or rb.std() == 0:
        return float("nan")
    return float(np.corrcoef(ra, rb)[0, 1])


def channel(record, channels, name):
    return record["features"][channels.index(name)].astype(np.float32)


def capacity(record, channels):
    """Free routing share per tile summed over the layers present."""
    total = np.zeros(record["features"].shape[1:], dtype=np.float32)
    for slot in ("F", "B", "In1", "InN"):
        present = channel(record, channels, f"layer_present.{slot}")
        free = 1.0 - channel(record, channels, f"obstacle.{slot}") - channel(record, channels, f"pad_copper.{slot}")
        total += present * np.clip(free, 0.0, 1.0)
    return total


def rudy_scores(record, channels, alpha=0.5):
    demand = channel(record, channels, "rudy_mst")
    bbox = channel(record, channels, "rudy")
    inside = channel(record, channels, "inside") > 0
    cap = capacity(record, channels)
    ratio = np.where(inside, demand / np.maximum(cap, 0.05), 0.0)
    over = np.maximum(demand - alpha * cap, 0.0) * inside
    values = ratio[inside]
    top = np.sort(values)[-max(1, len(values) // 20):] if len(values) else np.zeros(1)
    return {
        "rudy_bbox_over": float((np.maximum(bbox - alpha * cap, 0.0) * inside).sum()),
        "rudy_sum": float((demand * inside).sum()),
        "rudy_over": float(over.sum()),
        "rudy_max": float(values.max()) if len(values) else 0.0,
        "rudy_top": float(top.mean()),
    }


def ranking_metrics(groups, ranker):
    """Within-board ranking quality of `ranker` (lower score = better)."""
    rhos, pairs_right, pairs, regrets, hits, boards = [], 0, 0, [], 0, 0
    for samples in groups.values():
        truth = [s["truth"] for s in samples]
        score = [s[ranker] for s in samples]
        if len(samples) < 2 or len(set(truth)) < 2:
            continue
        boards += 1
        rho = spearman(score, truth)
        if not math.isnan(rho):
            rhos.append(rho)
        for i in range(len(samples)):
            for j in range(i + 1, len(samples)):
                if truth[i] != truth[j] and score[i] != score[j]:
                    pairs += 1
                    pairs_right += (score[i] < score[j]) == (truth[i] < truth[j])
                elif truth[i] != truth[j]:
                    pairs += 1
                    pairs_right += 0.5
        chosen = int(np.argmin(score))
        regrets.append(truth[chosen] - min(truth))
        order = np.argsort(score, kind="stable")[:3]
        hits += any(truth[k] == min(truth) for k in order)
    return {
        "boards": boards,
        "spearman_within": float(np.mean(rhos)) if rhos else float("nan"),
        "pair_accuracy": pairs_right / pairs if pairs else float("nan"),
        "top1_regret": float(np.mean(regrets)) if regrets else float("nan"),
        "best_in_top3": hits / boards if boards else float("nan"),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--cache", type=Path, default=None)
    parser.add_argument("--model", type=Path, default=None)
    parser.add_argument("--side", default="test")
    parser.add_argument("--variants", default=None, help="only variants whose name starts with this")
    parser.add_argument("--boards", default=None, help="only boards whose name starts with this")
    parser.add_argument("--json", type=Path, default=None)
    parser.add_argument("--threshold", type=float, default=1.0,
                        help="overflow (label units) above which a tile counts as overflowing")
    arguments = parser.parse_args()
    cache = arguments.cache or arguments.root / "cache"
    split = json.loads((arguments.root / "split.json").read_text())
    records, train_records = [], []
    for path in sorted(cache.glob("*.npz")):
        if arguments.boards and not path.stem.startswith(arguments.boards):
            continue
        side = split.get(path.stem, "train")
        for record in load_board(path):
            if arguments.variants and not record["variant"].startswith(arguments.variants):
                continue
            if side == arguments.side:
                records.append(record)
            elif side == "train" and arguments.side != "train":
                train_records.append(record)
    channels = channel_names()
    # Fit rudy_over's capacity factor on training boards (within-board
    # Spearman, coarse grid).
    alpha = 0.5
    if train_records:
        best = None
        for candidate in (0.05, 0.1, 0.2, 0.3, 0.5, 0.75, 1.0, 1.5):
            groups = defaultdict(list)
            for record in train_records[:3000]:
                groups[record["board"]].append({"truth": float(record["unfinished"]),
                                                "rudy_over": rudy_scores(record, channels, candidate)["rudy_over"]})
            quality = ranking_metrics(groups, "rudy_over")["spearman_within"]
            if best is None or (not math.isnan(quality) and quality > best[0]):
                best = (quality, candidate)
        alpha = best[1]
    net = None
    label = "conflict"
    if arguments.model:
        import torch
        from model import CongestionNet
        from train import collate, target_map
        checkpoint = torch.load(arguments.model / "model.pt", map_location="cpu")
        net = CongestionNet(checkpoint["channels"], np.array(checkpoint["mean"]), np.array(checkpoint["std"]),
                            PLANES, checkpoint["width"])
        net.load_state_dict(checkpoint["state"])
        device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
        net = net.to(device).eval()
        label = checkpoint["label"]
    groups = defaultdict(list)
    tile_errors, tp, fp, fn = [], 0, 0, 0
    for index, record in enumerate(records):
        row = {"truth": float(record["unfinished"]), "wirelength": float(record["wirelength_mm"]),
               "board": record["board"], "variant": record["variant"]}
        row.update(rudy_scores(record, channels, alpha))
        if net is not None:
            import torch
            with torch.no_grad():
                x, y, present, _ = collate([record], [0], label, device)
                overflow, _, open_log = net(x)
                predicted = torch.clamp(torch.expm1(overflow), min=0.0)
                board = (x[:, net.normalize.inside_index:net.normalize.inside_index + 1] > 0).float()
                mask = (board * present).expand_as(y) > 0
                row["model_open"] = float(open_log.exp().item() - 1.0)
                row["model_overflow"] = float((predicted * mask).sum().item())
                truth = y[mask].cpu().numpy()
                guess = predicted[mask].cpu().numpy()
                tile_errors.append(np.abs(truth - guess).mean())
                hot, called = truth > arguments.threshold, guess > arguments.threshold
                tp += int((hot & called).sum())
                fp += int((~hot & called).sum())
                fn += int((hot & ~called).sum())
        groups[record["board"]].append(row)
    rankers = ["wirelength", "rudy_sum", "rudy_over", "rudy_bbox_over", "rudy_max", "rudy_top"]
    if net is not None:
        rankers = ["model_open", "model_overflow"] + rankers
    all_rows = [row for rows in groups.values() for row in rows]
    report = {"samples": len(all_rows), "boards": len(groups), "rudy_alpha": alpha, "label": label, "rankers": {}}
    for ranker in rankers:
        metrics = ranking_metrics(groups, ranker)
        metrics["spearman_pooled"] = spearman([r[ranker] for r in all_rows], [r["truth"] for r in all_rows])
        report["rankers"][ranker] = metrics
    if net is not None:
        report["tiles"] = {
            "mae": float(np.mean(tile_errors)) if tile_errors else float("nan"),
            "precision": tp / (tp + fp) if tp + fp else float("nan"),
            "recall": tp / (tp + fn) if tp + fn else float("nan"),
            "threshold": arguments.threshold,
        }
    # The placement race simulated on the task placements (the placer's
    # seeds on the benchmark task): today's race probes seeds 1-3; a
    # pre-ranker probes its best `k` of all seeds. The race keeps the
    # fewest unfinished nets among the probed.
    race = {}
    for board, rows in groups.items():
        seeds = sorted((r for r in rows if re.fullmatch(r"task-seed\d+", r["variant"])),
                       key=lambda r: int(r["variant"][len("task-seed"):]))
        if len(seeds) < 4:
            continue
        entry = {"seeds": len(seeds), "default": min(r["truth"] for r in seeds[:3]),
                 "oracle": min(r["truth"] for r in seeds), "worst": max(r["truth"] for r in seeds)}
        # A ranker that knows nothing: the expected best of k seeds drawn
        # at random.
        truths = [r["truth"] for r in seeds]
        for k in (1, 3):
            subsets = list(itertools.combinations(truths, k))
            entry[f"random@{k}"] = sum(min(subset) for subset in subsets) / len(subsets)
        # Hit rates: the ranker's best seed among the probe's best three
        # (ties with the third count), and the probe's best among the
        # ranker's top three.
        third = sorted(truths)[2]
        best = min(truths)
        n = len(truths)
        entry["random_hit1_in_best3"] = sum(t <= third for t in truths) / n
        entry["random_best_in_top3"] = 1.0 - (
            math.comb(sum(t > best for t in truths), 3) / math.comb(n, 3))
        for ranker in rankers:
            ordered = sorted(seeds, key=lambda r: r[ranker])
            for k in (1, 3):
                entry[f"{ranker}@{k}"] = min(r["truth"] for r in ordered[:k])
            entry[f"{ranker}_hit1_in_best3"] = float(ordered[0]["truth"] <= third)
            entry[f"{ranker}_best_in_top3"] = float(min(r["truth"] for r in ordered[:3]) == best)
        race[board] = entry
    if race:
        report["race"] = {"boards": race}
        keys = [key for key in next(iter(race.values())) if key != "seeds" and "hit" not in key and "top3" not in key]
        report["race"]["total"] = {key: sum(entry[key] for entry in race.values()) for key in keys}
        # Hit rates over the boards whose seeds differ at all.
        varied = [entry for entry in race.values() if entry["worst"] > entry["oracle"]]
        report["race"]["hit_rates"] = {"boards": len(varied)}
        for name in ["random"] + rankers:
            for kind in ("hit1_in_best3", "best_in_top3"):
                key = f"{name}_{kind}"
                report["race"]["hit_rates"][key] = sum(entry[key] for entry in varied) / len(varied) if varied else float("nan")
        for ranker in rankers:
            for k in (1, 3):
                key = f"{ranker}@{k}"
                report["race"].setdefault("versus_default", {})[key] = {
                    "better": sum(entry[key] < entry["default"] for entry in race.values()),
                    "same": sum(entry[key] == entry["default"] for entry in race.values()),
                    "worse": sum(entry[key] > entry["default"] for entry in race.values()),
                }
    print(json.dumps(report, indent=1))
    if arguments.json:
        arguments.json.write_text(json.dumps({"report": report, "rows": all_rows}, indent=1))


if __name__ == "__main__":
    main()
