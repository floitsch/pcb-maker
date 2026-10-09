#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Checks that tract (the Rust side) computes what PyTorch computes for a
trained model on real samples.

    build/venv-nn/bin/python experiments/congestion/check_tract.py MODEL_DIR BINARY SAMPLE.json..."""

import json
import math
import subprocess
import sys
from pathlib import Path

import numpy as np
import torch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dataset import PLANES, read_sample  # noqa: E402
from model import CongestionNet, Exported  # noqa: E402


def main():
    model_dir, binary, samples = Path(sys.argv[1]), sys.argv[2], sys.argv[3:]
    checkpoint = torch.load(model_dir / "model.pt", map_location="cpu")
    net = CongestionNet(checkpoint["channels"], np.array(checkpoint["mean"]), np.array(checkpoint["std"]),
                        PLANES, checkpoint["width"])
    net.load_state_dict(checkpoint["state"])
    exported = Exported(net).eval()
    worst = 0.0
    for sample in samples:
        _, arrays = read_sample(sample)
        features = arrays["features"]
        c, h, w = features.shape
        x = np.zeros((1, c, math.ceil(h / 8) * 8, math.ceil(w / 8) * 8), dtype=np.float32)
        x[0, :, :h, :w] = features
        with torch.no_grad():
            overflow, open_log = exported(torch.from_numpy(x))
        torch_open = float(open_log.exp().item() - 1.0)
        torch_planes = overflow[0, :, :h, :w].sum(dim=(1, 2)).tolist()
        rust = json.loads(subprocess.run([binary, "score-congestion-sample", sample, str(model_dir / "model.onnx")],
                                         capture_output=True, text=True, check=True).stdout)
        difference = max([abs(rust["open"] - torch_open) / max(1.0, abs(torch_open))] +
                         [abs(a - b) / max(1.0, abs(b)) for a, b in zip(rust["planes"], torch_planes)])
        worst = max(worst, difference)
        print(f"{Path(sample).parent.name}/{Path(sample).stem}: {w} x {h} tiles; open torch {torch_open:.4f} tract {rust['open']:.4f}; "
              f"overflow torch {sum(torch_planes):.3f} tract {rust['overflow']:.3f}; largest relative difference {difference:.2e}")
    print(f"largest relative difference over {len(samples)} samples: {worst:.2e}")


if __name__ == "__main__":
    main()
