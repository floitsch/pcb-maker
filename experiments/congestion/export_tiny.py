#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Writes the fixed tiny congestion model the Rust inference test runs
(crates/pcb-congestion/testdata/tiny.onnx) and prints the outputs PyTorch
computes for the test's input, to paste into the test.

    build/venv-nn/bin/python experiments/congestion/export_tiny.py"""

import sys
from pathlib import Path

import numpy as np
import torch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dataset import channel_names  # noqa: E402
from model import CongestionNet  # noqa: E402
from train import export_onnx  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def test_input(channels, height, width):
    """The pattern the Rust test builds: ((7c + 3y + 5x) mod 11) / 10."""
    c, y, x = np.meshgrid(np.arange(channels), np.arange(height), np.arange(width), indexing="ij")
    return (((7 * c + 3 * y + 5 * x) % 11) / 10.0).astype(np.float32)


def main():
    torch.manual_seed(0)
    channels = channel_names()
    mean = np.linspace(0.1, 0.5, len(channels))
    std = np.linspace(0.5, 1.5, len(channels))
    net = CongestionNet(channels, mean, std, planes=5, width=8).eval()
    path = ROOT / "crates/pcb-congestion/testdata/tiny.onnx"
    path.parent.mkdir(parents=True, exist_ok=True)
    export_onnx(net, channels, path)
    # A board of 13 x 10 tiles: the Rust side pads it to 16 x 16.
    height, width = 10, 13
    x = np.zeros((1, len(channels), 16, 16), dtype=np.float32)
    x[0, :, :height, :width] = test_input(len(channels), height, width)
    from model import Exported
    with torch.no_grad():
        overflow, open_log = Exported(net)(torch.from_numpy(x))
    overflow = overflow[0, :, :height, :width].numpy()
    print(f"open (unfinished) = {float(open_log.exp().item() - 1.0):.6f}")
    print(f"overflow sum = {float(overflow.sum()):.6f}")
    for plane in range(overflow.shape[0]):
        print(f"plane {plane} sum = {float(overflow[plane].sum()):.6f}")
    index = np.unravel_index(np.argmax(overflow), overflow.shape)
    print(f"largest overflow{tuple(int(i) for i in index)} = {float(overflow[index]):.6f}")
    print(f"wrote {path} ({path.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
