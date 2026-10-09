#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Exports several trained networks as one ONNX ensemble (outputs
averaged), the file the layout's `congestion_model` loads.

    build/venv-nn/bin/python experiments/congestion/export_ensemble.py OUT.onnx MODEL_DIR..."""

import sys
from pathlib import Path

import numpy as np
import torch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dataset import PLANES, channel_names  # noqa: E402
from model import CongestionNet, Ensemble  # noqa: E402
from train import export_onnx  # noqa: E402


def main():
    out, directories = Path(sys.argv[1]), [Path(p) for p in sys.argv[2:]]
    nets = []
    for directory in directories:
        checkpoint = torch.load(directory / "model.pt", map_location="cpu")
        net = CongestionNet(checkpoint["channels"], np.array(checkpoint["mean"]), np.array(checkpoint["std"]),
                            PLANES, checkpoint["width"])
        net.load_state_dict(checkpoint["state"])
        nets.append(net)
    export_onnx(Ensemble(nets), channel_names(), out)
    print(f"wrote {out} ({len(nets)} networks)")


if __name__ == "__main__":
    main()
