# Copyright (C) 2026 Toit contributors.
"""Reads the exporter's samples (`<variant>.json` header plus `<variant>.bin`
arrays) and caches them per board as compact `.npz` files for training.

Labels are mapped to the model's layer slots the way the Rust features are
(`pcb_congestion::features::slot_of`): front, back, first inner, other
inner layers (averaged), then the via plane."""

import json
from pathlib import Path

import numpy as np

SLOTS = 4
PLANES = SLOTS + 1
LABELS = ("conflict", "history", "tile_history", "usage")


def channel_names():
    """The exporter's channels, in order (`pcb_congestion::features::channel_names`)."""
    names = []
    for channel in ["pins", "pad_copper", "obstacle", "pour", "layer_present"]:
        for slot in ["F", "B", "In1", "InN"]:
            names.append(f"{channel}.{slot}")
    names += ["inside", "rudy", "rudy_mst", "pin_rudy", "rudy_pour", "nets_here", "through_pins", "layer_count",
              "track_pitch", "via_pitch", "tile_mm", "pins_scaled"]
    return names


def slot_of(layer, layers):
    if layer == 0:
        return 0
    if layer + 1 == layers:
        return 1
    if layer == 1:
        return 2
    return 3


def read_sample(json_path):
    json_path = Path(json_path)
    header = json.loads(json_path.read_text())
    raw = np.fromfile(json_path.with_suffix(".bin"), dtype="<f4")
    arrays = {}
    for name, entry in header["arrays"].items():
        count = int(np.prod(entry["shape"]))
        start = entry["offset"] // 4
        arrays[name] = raw[start:start + count].reshape(entry["shape"])
    return header, arrays


def to_planes(array, layers):
    """`[layers (+ via), H, W]` -> `[PLANES, H, W]`: layers averaged into
    slots, the via plane (when present) last."""
    out = np.zeros((PLANES,) + array.shape[1:], dtype=np.float32)
    counts = np.zeros(SLOTS)
    for layer in range(layers):
        slot = slot_of(layer, layers)
        out[slot] += array[layer]
        counts[slot] += 1
    for slot in range(SLOTS):
        if counts[slot]:
            out[slot] /= counts[slot]
    if array.shape[0] > layers:
        out[SLOTS] = array[layers]
    return out


def sample_record(json_path):
    """One sample as training arrays (None without a probe)."""
    header, arrays = read_sample(json_path)
    if "probe" not in header:
        return None
    layers = header["layers"]
    tile = header["frame"]["tile"]
    nodes = float(tile * tile)
    record = {
        "features": arrays["features"].astype(np.float16),
        "unfinished": np.float32(header["probe"]["unfinished"]),
        "conflicted_nets": np.float32(header["probe"]["conflicted_nets"]),
        "incomplete_nets": np.float32(header["probe"]["incomplete_nets"]),
        "routable_nets": np.float32(header["probe"]["routable_nets"]),
        "wirelength_mm": np.float32(header["wirelength_mm"]),
        "layers": np.int32(layers),
        "pitch": np.float32(header["frame"]["pitch"]),
    }
    for name in LABELS:
        planes = to_planes(arrays[name], layers)
        # Per node of the tile: conflict nodes, history per node, route
        # nodes; the tile history is per tile already.
        if name != "tile_history":
            planes /= nodes
        record[name] = planes.astype(np.float16)
    record["routable"] = to_planes(arrays["routable"], layers)[:SLOTS].astype(np.float16)
    record["claimed"] = to_planes(arrays["claimed"], layers)[:SLOTS].astype(np.float16)
    record["open_terminals"] = arrays["open_terminals"].astype(np.float16)
    return record


def cache_board(sample_directory, cache_path):
    """All probed samples of one board in one `.npz` (variant names in
    `variants`, arrays stacked per variant as `<variant>/<name>`)."""
    sample_directory = Path(sample_directory)
    payload = {}
    names = []
    for json_path in sorted(sample_directory.glob("*.json")):
        try:
            record = sample_record(json_path)
        except (OSError, ValueError, KeyError) as error:
            print(f"skipping {json_path}: {error}")
            continue
        if record is None:
            continue
        names.append(json_path.stem)
        for key, value in record.items():
            payload[f"{json_path.stem}/{key}"] = value
    if not names:
        return 0
    payload["variants"] = np.array(names)
    cache_path.parent.mkdir(parents=True, exist_ok=True)
    np.savez(cache_path, **payload)
    return len(names)


def load_board(cache_path):
    data = np.load(cache_path, allow_pickle=False)
    board = cache_path.stem
    samples = []
    for variant in data["variants"]:
        record = {key.split("/", 1)[1]: data[key] for key in data.files if key.startswith(f"{variant}/")}
        record["board"] = board
        record["variant"] = str(variant)
        samples.append(record)
    return samples


def build_cache(samples_root, cache_root, boards=None):
    samples_root, cache_root = Path(samples_root), Path(cache_root)
    total = 0
    for directory in sorted(samples_root.iterdir()):
        if not directory.is_dir() or (boards is not None and directory.name not in boards):
            continue
        cache = cache_root / f"{directory.name}.npz"
        newest = max((path.stat().st_mtime for path in directory.glob("*.json")), default=0)
        if cache.exists() and cache.stat().st_mtime >= newest:
            total += len(np.load(cache)["variants"])
            continue
        total += cache_board(directory, cache)
    return total
