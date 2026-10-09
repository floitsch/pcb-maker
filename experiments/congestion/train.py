#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Trains the congestion network on cached samples (dataset.py) and exports
it to ONNX.

    build/venv-nn/bin/python experiments/congestion/train.py build/congestion \
        --out build/congestion/models/m1 [--label conflict] [--epochs 40]

Held-out boards (split.json) are never trained on; the validation numbers
printed per epoch are theirs."""

import argparse
import json
import math
import random
import sys
import time
from pathlib import Path

import numpy as np
import torch
import torch.nn.functional as F

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dataset import PLANES, build_cache, channel_names, load_board  # noqa: E402
from model import CongestionNet, Exported, LOG_PREFIXES  # noqa: E402

# Label units: per-node shares times the nodes of a tile.
TILE_NODES = 256.0


def load_samples(root, cache, split, boards_filter=None):
    train, test = [], []
    for path in sorted(cache.glob("*.npz")):
        if boards_filter and path.stem not in boards_filter:
            continue
        side = split.get(path.stem, "train")
        for record in load_board(path):
            (test if side == "test" else train).append(record)
    return train, test


def target_map(record, label):
    values = record[label].astype(np.float32)
    if label != "tile_history":
        values = values * TILE_NODES
    return values


def channel_stats(samples, channels, limit=400):
    log = np.array([any(name.startswith(prefix) for prefix in LOG_PREFIXES) for name in channels])
    inside = channels.index("inside")
    sums = np.zeros(len(channels))
    squares = np.zeros(len(channels))
    count = 0
    for record in random.Random(1).sample(samples, min(limit, len(samples))):
        x = np.clip(record["features"].astype(np.float64), 0, None)
        x[log] = np.log1p(x[log])
        mask = x[inside] > 0
        values = x[:, mask]
        sums += values.sum(axis=1)
        squares += (values ** 2).sum(axis=1)
        count += values.shape[1]
    mean = sums / max(count, 1)
    std = np.sqrt(np.maximum(squares / max(count, 1) - mean ** 2, 0)) + 1e-3
    return mean, std


def batches(samples, tile_budget, shuffle, rng):
    order = sorted(range(len(samples)), key=lambda i: samples[i]["features"].shape[1] * samples[i]["features"].shape[2])
    groups, current, largest = [], [], 0
    for index in order:
        _, h, w = samples[index]["features"].shape
        size = (math.ceil(h / 8) * 8) * (math.ceil(w / 8) * 8)
        if current and max(largest, size) * (len(current) + 1) > tile_budget:
            groups.append(current)
            current, largest = [], 0
        current.append(index)
        largest = max(largest, size)
    if current:
        groups.append(current)
    if shuffle:
        rng.shuffle(groups)
    return groups


def board_batches(samples, tile_budget, shuffle, rng):
    """Batches of variants of one board (same tile grid: no padding
    between them, and a ranking loss compares placements of one board)."""
    by_board = {}
    for index, record in enumerate(samples):
        by_board.setdefault((record["board"], record["features"].shape), []).append(index)
    groups = []
    for indices in by_board.values():
        if shuffle:
            rng.shuffle(indices)
        _, h, w = samples[indices[0]]["features"].shape
        size = (math.ceil(h / 8) * 8) * (math.ceil(w / 8) * 8)
        per_batch = max(1, tile_budget // size)
        for start in range(0, len(indices), per_batch):
            groups.append(indices[start:start + per_batch])
    if shuffle:
        rng.shuffle(groups)
    return groups


def collate(samples, indices, label, device, flip=None):
    h = max(samples[i]["features"].shape[1] for i in indices)
    w = max(samples[i]["features"].shape[2] for i in indices)
    h, w = math.ceil(h / 8) * 8, math.ceil(w / 8) * 8
    c = samples[indices[0]]["features"].shape[0]
    x = np.zeros((len(indices), c, h, w), dtype=np.float32)
    y = np.zeros((len(indices), PLANES, h, w), dtype=np.float32)
    present = np.zeros((len(indices), PLANES, 1, 1), dtype=np.float32)
    opened = np.zeros((len(indices), 1), dtype=np.float32)
    for row, index in enumerate(indices):
        record = samples[index]
        f = record["features"].astype(np.float32)
        t = target_map(record, label)
        if flip:
            if flip[0]:
                f, t = f[:, ::-1, :], t[:, ::-1, :]
            if flip[1]:
                f, t = f[:, :, ::-1], t[:, :, ::-1]
        _, fh, fw = f.shape
        x[row, :, :fh, :fw] = f
        y[row, :, :fh, :fw] = t
        layers = int(record["layers"])
        slots = {0, 1} | ({2} if layers > 2 else set()) | ({3} if layers > 3 else set())
        for slot in slots:
            present[row, slot] = 1.0
        present[row, PLANES - 1] = 1.0
        opened[row, 0] = math.log1p(float(record["unfinished"]))
    to = lambda array: torch.from_numpy(np.ascontiguousarray(array)).to(device)
    return to(x), to(y), to(present), to(opened)


def losses(net, batch, open_weight, rank_weight=0.0):
    x, y, present, opened = batch
    overflow, _, open_log = net(x)
    board = (x[:, net.normalize.inside_index:net.normalize.inside_index + 1] > 0).float()
    mask = board * present
    target = torch.log1p(y)
    # Overflow is sparse: tiles that overflow weigh more.
    weight = mask * (1.0 + 4.0 * (y > 0).float())
    map_loss = (F.smooth_l1_loss(overflow, target, reduction="none", beta=0.5) * weight).sum() / weight.sum().clamp(min=1.0)
    open_loss = F.smooth_l1_loss(open_log, opened, beta=0.5)
    total = map_loss + open_weight * open_loss
    if rank_weight > 0 and len(opened) > 1:
        # Placements of one board, in pairs: the one that leaves fewer nets
        # unfinished should score lower (logistic loss on the difference).
        truth = opened[:, 0]
        guess = open_log[:, 0]
        apart = truth[:, None] - truth[None, :]
        pairs = (apart > 0.05).float()
        if pairs.sum() > 0:
            margin = guess[:, None] - guess[None, :]
            rank_loss = (F.softplus(-margin * 4.0) * pairs).sum() / pairs.sum()
            total = total + rank_weight * rank_loss
    return total, map_loss, open_loss


def spearman(a, b):
    a, b = np.asarray(a, dtype=float), np.asarray(b, dtype=float)
    if len(a) < 3:
        return float("nan")
    ra = np.argsort(np.argsort(a))
    rb = np.argsort(np.argsort(b))
    if ra.std() == 0 or rb.std() == 0:
        return float("nan")
    return float(np.corrcoef(ra, rb)[0, 1])


def evaluate(net, samples, label, device, tile_budget):
    net.eval()
    predictions = []
    total = [0.0, 0.0]
    with torch.no_grad():
        for indices in batches(samples, tile_budget, False, None):
            batch = collate(samples, indices, label, device)
            loss, map_loss, open_loss = losses(net, batch, 1.0)
            total[0] += map_loss.item() * len(indices)
            total[1] += open_loss.item() * len(indices)
            _, _, open_log = net(batch[0])
            for row, index in enumerate(indices):
                predictions.append((index, float(open_log[row, 0].exp().item() - 1.0)))
    net.train()
    predictions.sort()
    predicted = [value for _, value in predictions]
    truth = [float(samples[index]["unfinished"]) for index, _ in predictions]
    return total[0] / max(len(samples), 1), total[1] / max(len(samples), 1), spearman(predicted, truth)


def export_onnx(net, channels, path):
    exported = Exported(net).cpu().eval()
    example = torch.zeros(1, len(channels), 32, 48)
    torch.onnx.export(
        exported, (example,), str(path), input_names=["features"], output_names=["overflow", "open"],
        dynamic_axes={"features": {2: "height", 3: "width"}, "overflow": {2: "height", 3: "width"}},
        opset_version=17, dynamo=False,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--samples", type=Path, default=None)
    parser.add_argument("--cache", type=Path, default=None)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--label", default="conflict", choices=["conflict", "history", "tile_history", "usage"])
    parser.add_argument("--epochs", type=int, default=40)
    parser.add_argument("--width", type=int, default=32)
    parser.add_argument("--lr", type=float, default=2e-3)
    parser.add_argument("--open-weight", type=float, default=1.0)
    parser.add_argument("--tile-budget", type=int, default=120000)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--by-board", action="store_true", help="batches of one board's placements")
    parser.add_argument("--rank-weight", type=float, default=0.0,
                        help="weight of a within-batch pairwise ranking loss on the open head (with --by-board)")
    parser.add_argument("--kinds", nargs="*", help="only boards whose name starts with these prefixes")
    arguments = parser.parse_args()
    samples_root = arguments.samples or arguments.root / "samples"
    cache = arguments.cache or arguments.root / "cache"
    print(f"caching {samples_root} -> {cache}: {build_cache(samples_root, cache)} samples", flush=True)
    split = json.loads((arguments.root / "split.json").read_text())
    train, test = load_samples(arguments.root, cache, split)
    if arguments.kinds:
        keep = lambda record: any(record["board"].startswith(prefix) for prefix in arguments.kinds)
        train = [record for record in train if keep(record)]
    print(f"{len(train)} training samples ({len({r['board'] for r in train})} boards), "
          f"{len(test)} held out ({len({r['board'] for r in test})} boards)", flush=True)
    channels = channel_names()
    random.seed(arguments.seed)
    torch.manual_seed(arguments.seed)
    mean, std = channel_stats(train, channels)
    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    net = CongestionNet(channels, mean, std, PLANES, arguments.width).to(device)
    parameters = sum(p.numel() for p in net.parameters())
    print(f"{parameters} parameters on {device}", flush=True)
    optimizer = torch.optim.AdamW(net.parameters(), lr=arguments.lr, weight_decay=1e-4)
    rng = random.Random(arguments.seed)
    steps = arguments.epochs * len((board_batches if arguments.by_board else batches)(train, arguments.tile_budget, False, random.Random(0)))
    scheduler = torch.optim.lr_scheduler.OneCycleLR(optimizer, max_lr=arguments.lr, total_steps=max(steps, 1))
    arguments.out.mkdir(parents=True, exist_ok=True)
    history = []
    best = None
    for epoch in range(arguments.epochs):
        started = time.monotonic()
        totals = [0.0, 0.0, 0]
        make = board_batches if arguments.by_board else batches
        for indices in make(train, arguments.tile_budget, True, rng):
            flip = (rng.random() < 0.5, rng.random() < 0.5)
            batch = collate(train, indices, arguments.label, device, flip)
            loss, map_loss, open_loss = losses(net, batch, arguments.open_weight, arguments.rank_weight)
            optimizer.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(net.parameters(), 5.0)
            optimizer.step()
            scheduler.step()
            totals[0] += map_loss.item() * len(indices)
            totals[1] += open_loss.item() * len(indices)
            totals[2] += len(indices)
        test_map, test_open, test_rho = evaluate(net, test, arguments.label, device, arguments.tile_budget)
        row = {"epoch": epoch, "train_map": totals[0] / totals[2], "train_open": totals[1] / totals[2],
               "test_map": test_map, "test_open": test_open, "test_spearman_open": test_rho,
               "seconds": round(time.monotonic() - started, 1)}
        history.append(row)
        print(json.dumps(row), flush=True)
    torch.save({"state": net.state_dict(), "channels": channels, "mean": mean.tolist(), "std": std.tolist(),
                "width": arguments.width, "label": arguments.label}, arguments.out / "model.pt")
    (arguments.out / "train.json").write_text(json.dumps({"arguments": {k: str(v) for k, v in vars(arguments).items()},
                                                           "parameters": parameters, "history": history}, indent=1))
    export_onnx(net, channels, arguments.out / "model.onnx")
    print(f"wrote {arguments.out / 'model.onnx'}")


if __name__ == "__main__":
    main()
