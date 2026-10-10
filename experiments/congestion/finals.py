#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Which probe statistic predicts a placement's final layout result?

The 92 final layouts (build/seed-finals: 23 held-out boards, the two best
and two worst of 16 placer seeds by one 75 s probe, each laid out alone
for 1800 s) against predictors of the same placements: the probe (router
seed none), probes under router seeds 1-3, their means, the probe's
conflicted and incomplete nets, RUDY, wirelength, the network.

    build/venv-nn/bin/python experiments/congestion/finals.py [--model DIR...]"""

import argparse
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from evaluate import spearman, rudy_scores  # noqa: E402
from dataset import channel_names, read_sample  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--finals", type=Path, default=ROOT / "build/seed-finals")
    parser.add_argument("--samples", type=Path, default=ROOT / "build/cdata/samples")
    parser.add_argument("--noise", type=Path, default=ROOT / "build/cnoise2")
    parser.add_argument("--model", type=Path, nargs="*", default=None)
    arguments = parser.parse_args()
    rows = []
    for line in Path(str(arguments.finals) + ".log").read_text().splitlines():
        if not line.startswith("{"):
            continue
        result = json.loads(line)
        native = result.get("native") or {}
        if native.get("unconnected") is None:
            continue
        board, seed = re.match(r"(.*)__seed(\d+)$", result["name"]).groups()
        report = arguments.finals / result["name"] / "layout/board-layout.json"
        layout = json.loads(report.read_text()) if report.exists() else {}
        row = {"board": board, "seed": int(seed), "final": native["unconnected"],
               "final_routed": int(result["routed"].split("/")[0]) - int(result["routed"].split("/")[1]),
               "first_route_open": layout.get("first_unconnected_terminals")}
        sample = arguments.samples / board / f"task-seed{seed}.json"
        header, arrays = read_sample(sample)
        row["probe"] = header["probe"]["unfinished"]
        row["conflicted"] = header["probe"]["conflicted_nets"]
        row["incomplete"] = header["probe"]["incomplete_nets"]
        row["wirelength"] = header["wirelength_mm"]
        record = {"features": arrays["features"]}
        row["rudy_over"] = rudy_scores(record, channel_names(), 0.25)["rudy_over"]
        others = []
        for index in (1, 2, 3):
            path = arguments.noise / f"samples-r{index}" / board / f"task-seed{seed}.json"
            if path.exists():
                others.append(json.loads(path.read_text())["probe"]["unfinished"])
                row[f"probe_r{index}"] = others[-1]
        if others:
            row["mean2"] = (row["probe"] + others[0]) / 2
            row["mean_all"] = (row["probe"] + sum(others)) / (1 + len(others))
            row["independent"] = others[0]
        if len(others) >= 2:
            # Unbiased by the seed selection (which used `probe`): one
            # independent probe against the mean of two independent ones.
            row["mean_r12"] = (others[0] + others[1]) / 2
        rows.append(row)
    if arguments.model:
        import torch
        from dataset import PLANES
        from model import CongestionNet, Ensemble
        nets = []
        for directory in arguments.model:
            checkpoint = torch.load(directory / "model.pt", map_location="cpu")
            net = CongestionNet(checkpoint["channels"], np.array(checkpoint["mean"]), np.array(checkpoint["std"]),
                                PLANES, checkpoint["width"])
            net.load_state_dict(checkpoint["state"])
            nets.append(net)
        ensemble = Ensemble(nets).eval()
        for row in rows:
            _, arrays = read_sample(arguments.samples / row["board"] / f"task-seed{row['seed']}.json")
            features = arrays["features"]
            c, h, w = features.shape
            x = np.zeros((1, c, -(-h // 8) * 8, -(-w // 8) * 8), dtype=np.float32)
            x[0, :, :h, :w] = features
            with torch.no_grad():
                _, open_log = ensemble(torch.from_numpy(x))
            row["model"] = float(open_log.exp().item() - 1.0)
    by_board = defaultdict(list)
    for row in rows:
        by_board[row["board"]].append(row)
    predictors = ["probe", "independent", "mean_r12", "mean2", "mean_all", "conflicted", "incomplete", "rudy_over", "wirelength",
                  "first_route_open"] + (["model"] if arguments.model else [])
    print(f"{len(rows)} final layouts on {len(by_board)} boards")
    for predictor in predictors:
        rhos, right, pairs, regret = [], 0.0, 0, []
        for board_rows in by_board.values():
            usable = [r for r in board_rows if r.get(predictor) is not None]
            if len(usable) < 2 or len({r["final"] for r in usable}) < 2:
                continue
            rho = spearman([r[predictor] for r in usable], [r["final"] for r in usable])
            if not np.isnan(rho):
                rhos.append(rho)
            for i in range(len(usable)):
                for j in range(i + 1, len(usable)):
                    a, b = usable[i], usable[j]
                    if a["final"] == b["final"]:
                        continue
                    pairs += 1
                    if a[predictor] == b[predictor]:
                        right += 0.5
                    else:
                        right += (a[predictor] < b[predictor]) == (a["final"] < b["final"])
            chosen = min(usable, key=lambda r: (r[predictor], r["seed"]))
            regret.append(chosen["final"] - min(r["final"] for r in usable))
        print(f"{predictor:>17}: within-board Spearman {np.mean(rhos):+.3f} ({len(rhos)} boards), "
              f"pair accuracy {right / max(pairs, 1):.3f} ({pairs} pairs), chosen-seed regret {np.mean(regret):.2f} unconnected")
    # The probe's top two against its bottom two.
    better = worse = same = 0
    for board_rows in by_board.values():
        ordered = sorted(board_rows, key=lambda r: r["probe"])
        if len(ordered) < 4:
            continue
        top = sum(r["final"] for r in ordered[:2])
        bottom = sum(r["final"] for r in ordered[-2:])
        better += top < bottom
        worse += top > bottom
        same += top == bottom
    print(f"the probe's best two seeds against its worst two (final unconnected): better {better}, same {same}, worse {worse}")
    Path(str(arguments.finals) + "-analysis.json").write_text(json.dumps(rows, indent=1))


if __name__ == "__main__":
    main()
