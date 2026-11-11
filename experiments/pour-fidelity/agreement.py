# Copyright (C) 2026 Toit contributors.
"""The pour model's connected pads against KiCad's, over many routed boards.

    python3 -I agreement.py [--jobs N] [--kicad-cache DIR] <binary> <out-directory> <result-directory>...

Each result directory holds one routed `<id>.kicad_pcb` (a run's
`layout/result` or `route`). Per board: `dump-pour-map` writes, per pour
net and terminal, whether the model joins the terminal to its net's main
piece (`<net>.terminals`: what the router counts as routed), and
kicad_clusters.py whether KiCad's refill and connectivity put the pad in
its net's largest cluster (cached per board in the cache directory: the
boards do not change between two binaries). The terminals are matched to
KiCad's pads by name, copper layers and the nearest place.

Prints per board and in total: the pour-net pads, those the model calls
connected that KiCad leaves unconnected (`optimistic`: the router stops
with the pad open in KiCad's check) and those the model calls open that
KiCad connects (`pessimistic`: the router adds copper it does not need),
then each disagreement. Writes `<out>/<name>.json` per board.
"""

import concurrent.futures
import glob
import hashlib
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def board_id(directory):
    boards = glob.glob(os.path.join(directory, "*.kicad_pcb"))
    return os.path.basename(boards[0])[:-len(".kicad_pcb")] if len(boards) == 1 else None


def name_of(directory):
    """The board's task name (the path's part with `__` in it), prefixed by
    the run's directory."""
    parts = os.path.normpath(directory).split(os.sep)
    for index in range(len(parts) - 1, -1, -1):
        if "__" in parts[index]:
            return (parts[index - 1] + "-" if index > 0 else "") + parts[index]
    return parts[-2] + "-" + parts[-1]


def ours(dump):
    """The model's terminals from a dump directory, with layer names."""
    names = json.load(open(os.path.join(dump, "layers.json")))
    rows = []
    for path in sorted(glob.glob(os.path.join(dump, "*.terminals"))):
        for line in open(path):
            row = json.loads(line)
            row["layer_names"] = [names[layer] for layer in row["layers"]]
            rows.append(row)
    return rows


def match(terminals, pads):
    """Per terminal, its row joined with KiCad's pad of the same net and
    name (less a leading `/`) on its layers nearest its anchor (None where there is none).
    KiCad's connected pads are those of the net's cluster that holds the
    most terminals the model calls connected (the largest among equals): a
    net KiCad splits in two large parts has no main part of its own, and
    the model's main piece names which part the router counted as the
    net."""
    by_name = {}
    for pad in pads:
        by_name.setdefault((pad["net"].lstrip("/"), pad["pad"]), []).append(pad)
    rows = []
    for terminal in terminals:
        candidates = [pad for pad in by_name.get((terminal["net"].lstrip("/"), terminal["pad"]), [])
                      if all(layer in pad["layers"] for layer in terminal["layer_names"])]
        best = min(candidates, key=lambda pad: (pad["at"][0] - terminal["anchor"][0]) ** 2
                   + (pad["at"][1] - terminal["anchor"][1]) ** 2, default=None)
        rows.append({
            "net": terminal["net"],
            "pad": terminal["pad"],
            "anchor": terminal["anchor"],
            "layers": terminal["layer_names"],
            "ours": terminal["connected"],
            "kicad_cluster": None if best is None else best["cluster"],
            "kicad_cluster_pads": None if best is None else best["cluster_pads"],
        })
    for net in {row["net"] for row in rows}:
        votes = {}
        for row in rows:
            if row["net"] == net and row["kicad_cluster"] is not None:
                entry = votes.setdefault(row["kicad_cluster"], [0, row["kicad_cluster_pads"]])
                entry[0] += row["ours"]
        main = max(votes, key=lambda cluster: (votes[cluster][0], votes[cluster][1], -cluster), default=None)
        for row in rows:
            if row["net"] == net:
                row["kicad"] = None if row["kicad_cluster"] is None else row["kicad_cluster"] == main
    return rows


def prepare(binary, out, cache, directory):
    identifier = board_id(directory)
    if identifier is None:
        print(f"skip {directory}", file=sys.stderr)
        return None
    name = name_of(directory)
    result = os.path.join(out, name + ".json")
    if os.path.exists(result):
        return name, result
    board = os.path.join(directory, identifier + ".kicad_pcb")
    dump = os.path.join(out, name + ".dump")
    # A dump of an earlier run into this directory (one binary's) is kept.
    if not os.path.exists(os.path.join(dump, "layers.json")):
        done = subprocess.run([binary, "dump-pour-map", directory, identifier, dump],
                              stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        if done.returncode != 0:
            print(f"dump failed {directory}: {done.stderr.strip()[-300:]}", file=sys.stderr)
            return None
        # Only the terminals are read: the pour maps (a few bytes a lattice
        # node, per net and layer) would fill the disk over a census.
        for path in glob.glob(os.path.join(dump, "*.pour")):
            os.remove(path)
    terminals = ours(dump)
    # The model names a net without the sheet path's leading `/`.
    nets = sorted({name for terminal in terminals for name in (terminal["net"], "/" + terminal["net"])})
    digest = hashlib.sha256(open(board, "rb").read()).hexdigest()[:16]
    kicad = os.path.join(cache, f"{name}-{digest}.json")
    if not os.path.exists(kicad):
        done = subprocess.run([sys.executable, "-I", os.path.join(HERE, "kicad_clusters.py"), board,
                               kicad + ".part"] + nets,
                              stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        if done.returncode != 0:
            print(f"kicad failed {directory}: {done.stderr.strip()[-300:]}", file=sys.stderr)
            return None
        os.replace(kicad + ".part", kicad)
    rows = match(terminals, json.load(open(kicad)))
    with open(result, "w") as handle:
        json.dump(rows, handle)
    return name, result


def main():
    arguments = sys.argv[1:]
    cache, jobs = None, 1
    while arguments and arguments[0].startswith("--"):
        if arguments[0] == "--kicad-cache":
            cache = arguments[1]
        elif arguments[0] == "--jobs":
            jobs = int(arguments[1])
        arguments = arguments[2:]
    binary, out = arguments[0], arguments[1]
    os.makedirs(out, exist_ok=True)
    cache = cache or os.path.join(out, "kicad")
    os.makedirs(cache, exist_ok=True)
    with concurrent.futures.ThreadPoolExecutor(jobs) as pool:
        prepared = list(pool.map(lambda directory: prepare(binary, out, cache, directory), arguments[2:]))
    totals = {"boards": 0, "pads": 0, "unmatched": 0, "optimistic": 0, "pessimistic": 0}
    details = []
    for entry in prepared:
        if entry is None:
            continue
        name, result = entry
        counts = {"pads": 0, "unmatched": 0, "optimistic": 0, "pessimistic": 0}
        for row in json.load(open(result)):
            counts["pads"] += 1
            if row["kicad"] is None:
                counts["unmatched"] += 1
            elif row["ours"] and not row["kicad"]:
                counts["optimistic"] += 1
                details.append(f"  optimistic  {name[:50]:50} {row['net']:12} {row['pad']:10} {','.join(row['layers'])}")
            elif not row["ours"] and row["kicad"]:
                counts["pessimistic"] += 1
                details.append(f"  pessimistic {name[:50]:50} {row['net']:12} {row['pad']:10} {','.join(row['layers'])}")
        print(name, json.dumps(counts))
        totals["boards"] += 1
        for key in counts:
            totals[key] += counts[key]
    print("total", json.dumps(totals))
    print("\n".join(details))


if __name__ == "__main__":
    main()
