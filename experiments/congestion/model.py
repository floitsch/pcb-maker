# Copyright (C) 2026 Toit contributors.
"""The congestion network: input normalisation, a small U-Net, and two
heads (per-tile overflow per layer slot and the via plane; a per-tile open
density whose sum is the predicted number of unfinished nets)."""

import torch
import torch.nn as nn
import torch.nn.functional as F

# Channels whose values are counts or demand sums: log1p before scaling.
LOG_PREFIXES = ("pins.", "rudy", "pin_rudy", "nets_here", "through_pins", "pins_scaled")


class Normalize(nn.Module):
    def __init__(self, channels, mean, std, inside_index):
        super().__init__()
        log = [any(name.startswith(prefix) for prefix in LOG_PREFIXES) for name in channels]
        self.register_buffer("log", torch.tensor(log, dtype=torch.float32).view(1, -1, 1, 1))
        self.register_buffer("mean", torch.as_tensor(mean, dtype=torch.float32).view(1, -1, 1, 1))
        self.register_buffer("std", torch.as_tensor(std, dtype=torch.float32).view(1, -1, 1, 1))
        self.inside_index = inside_index

    def forward(self, x):
        x = torch.clamp(x, min=0.0)
        x = self.log * torch.log(x + 1.0) + (1.0 - self.log) * x
        board = (x[:, self.inside_index:self.inside_index + 1] > 0).float()
        return (x - self.mean) / self.std * board, board


def block(inputs, outputs):
    groups = 8
    return nn.Sequential(
        nn.Conv2d(inputs, outputs, 3, padding=1),
        nn.GroupNorm(groups, outputs),
        nn.SiLU(),
        nn.Conv2d(outputs, outputs, 3, padding=1),
        nn.GroupNorm(groups, outputs),
        nn.SiLU(),
    )


class CongestionNet(nn.Module):
    def __init__(self, channels, mean, std, planes=5, width=32):
        super().__init__()
        self.normalize = Normalize(channels, mean, std, channels.index("inside"))
        c = len(channels)
        w = width
        self.enc1 = block(c, w)
        self.enc2 = block(w, 2 * w)
        self.enc3 = block(2 * w, 4 * w)
        self.mid = block(4 * w, 4 * w)
        self.up3 = nn.ConvTranspose2d(4 * w, 4 * w, 2, stride=2)
        self.dec3 = block(8 * w, 2 * w)
        self.up2 = nn.ConvTranspose2d(2 * w, 2 * w, 2, stride=2)
        self.dec2 = block(4 * w, w)
        self.up1 = nn.ConvTranspose2d(w, w, 2, stride=2)
        self.dec1 = block(2 * w, w)
        self.overflow = nn.Conv2d(w, planes, 1)
        self.open = nn.Conv2d(w, 1, 1)

    def forward(self, features):
        """Returns (log1p of the overflow per tile and plane, the open
        density per tile, log1p of the predicted unfinished nets)."""
        x, board = self.normalize(features)
        e1 = self.enc1(x)
        e2 = self.enc2(F.max_pool2d(e1, 2))
        e3 = self.enc3(F.max_pool2d(e2, 2))
        m = self.mid(F.max_pool2d(e3, 2))
        d3 = self.dec3(torch.cat([self.up3(m), e3], 1))
        d2 = self.dec2(torch.cat([self.up2(d3), e2], 1))
        d1 = self.dec1(torch.cat([self.up1(d2), e1], 1))
        overflow = self.overflow(d1) * board
        density = F.softplus(self.open(d1)) * board
        open_log = torch.log(density.sum(dim=(2, 3)) + 1.0)
        return overflow, density, open_log


class Exported(nn.Module):
    """The ONNX graph: raw features in, overflow (label units, >= 0) and
    `ln(1 + unfinished)` out."""

    def __init__(self, net):
        super().__init__()
        self.net = net

    def forward(self, features):
        overflow, _, open_log = self.net(features)
        return torch.clamp(torch.exp(overflow) - 1.0, min=0.0), open_log
