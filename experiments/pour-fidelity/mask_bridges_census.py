# Copyright (C) 2026 Toit contributors.
"""KiCad's solder_mask_bridge findings on our boards, beyond the designer's.

    python3 -I mask_bridges_census.py --tasks build/github-tasks-v6/tasks.json \
        --binary build/bin/pcb-maker-x1022 [--json out.json] <output dir>...

Each output directory is a run.py output (layout mode: <task>/layout/result,
route mode: <task>/route). Per board: the findings of the result's drc.json
by item pair, the same for the designer's own board (`drc-kicad-board`,
cached), and the project values KiCad's rule reads (pcbnew on the result:
the board's mask expansion `pad_to_mask_clearance`, the minimum web
`solder_mask_min_width`, `solder_mask_to_copper_clearance`, via tenting).

KiCad's rule (DRC_TEST_PROVIDER_SOLDER_MASK, 10.0.7): an item on a mask
layer (a pad with the mask layer, an untented via, a track or a graphic on
a mask layer) bridges another's opening when their shapes come within the
minimum web plus both mask expansions; a copper item that is covered (a
track, a tented via) bridges an opening when it comes within the
solder-mask-to-copper clearance plus the opening's expansion; a zone's fill
likewise. Items of one net never bridge. A plain opening (a mask graphic, a
mask-only pad) bridges once it exposes two nets.

Pair kinds: via-pad, via-via, track-pad, pad-pad (pads of one footprint:
pad-pad same part), opening-<item> (a mask graphic or text with what it
exposes), zone-<item>, track-track, other.
"""

import argparse
import json
import os
import subprocess
import sys
from collections import Counter
from pathlib import Path

NM = 1.0e6


def item_kind(description):
    text = description
    if " on F.Mask" in text or " on B.Mask" in text:
        if "pad" not in text.lower().split(" [")[0]:
            return "opening"
    head = text.split(" ")[0]
    if head in ("Pad", "PTH", "NPTH", "SMD"):
        return "pad"
    if head == "Via":
        return "via"
    if head in ("Track", "Arc"):
        return "track"
    if head == "Zone":
        return "zone"
    return "graphic"


def footprint_of(description):
    if " of " in description and ("pad" in description.lower().split(" [")[0] or description.startswith("Pad")):
        return description.split(" of ")[1].split(" ")[0]
    return None


def pair_kind(items):
    kinds = [item_kind(item["description"]) for item in items[:2]]
    if len(kinds) < 2:
        return "other"
    order = ["opening", "zone", "via", "track", "pad", "graphic"]
    kinds.sort(key=order.index)
    a, b = kinds
    if a == "opening":
        return f"opening-{b}"
    if a == "zone":
        return f"zone-{b}"
    if (a, b) == ("pad", "pad"):
        same = footprint_of(items[0]["description"]) == footprint_of(items[1]["description"])
        return "pad-pad same part" if same else "pad-pad"
    return f"{a}-{b}"


def bridges(report):
    if not report.exists():
        return None
    drc = json.loads(report.read_text())
    return Counter(pair_kind(v.get("items", [])) for v in drc.get("violations", [])
                   if v["type"] == "solder_mask_bridge" and not v.get("excluded"))


def project_values(board_path):
    try:
        import pcbnew
    except ImportError:
        return {}
    board = pcbnew.LoadBoard(str(board_path))
    settings = board.GetDesignSettings()
    return {
        "pad_to_mask": settings.m_SolderMaskExpansion / NM,
        "min_web": settings.m_SolderMaskMinWidth / NM,
        "mask_to_copper": settings.m_SolderMaskToCopperClearance / NM,
        "tented": f"{'F' if settings.m_TentViasFront else '-'}{'B' if settings.m_TentViasBack else '-'}",
        "bridges_in_fps": bool(settings.m_AllowSoldermaskBridgesInFPs),
    }


def designer(task, binary, cache):
    report = cache / f"{task['name']}.json"
    if not report.exists():
        cache.mkdir(parents=True, exist_ok=True)
        subprocess.run([str(binary), "drc-kicad-board", task["directory"], task["board_id"], str(report)],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=900)
    return bridges(report)


def results(root):
    for board in sorted(Path(root).iterdir()):
        for sub in ("layout/result", "route"):
            report = board / sub / "drc.json"
            if report.exists():
                yield board.name, report


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("outputs", nargs="+")
    parser.add_argument("--tasks", required=True)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--cache", default=None, help="designer reports (default: <scratch>/designer-drc)")
    parser.add_argument("--json", default=None)
    parser.add_argument("--all", action="store_true", help="also boards without a bridge beyond the designer's")
    args = parser.parse_args()
    tasks = {task["name"]: task for task in json.load(open(args.tasks))["tasks"]}
    cache = Path(args.cache or os.path.join(os.path.dirname(os.path.abspath(args.json or ".")), "designer-drc"))
    rows = []
    for root in args.outputs:
        for name, report in results(root):
            ours = bridges(report)
            task = tasks.get(name)
            theirs = designer(task, args.binary, cache) if task else None
            theirs = theirs or Counter()
            beyond = Counter({kind: count - theirs.get(kind, 0) for kind, count in ours.items()
                              if count > theirs.get(kind, 0)})
            if not args.all and sum(ours.values()) <= sum(theirs.values()) and not beyond:
                continue
            boards = sorted(report.parent.glob("*.kicad_pcb"))
            values = project_values(boards[0]) if boards and sum(ours.values()) else {}
            rows.append({"run": Path(root).name, "board": name, "ours": dict(ours), "designer": dict(theirs),
                         "beyond": dict(beyond), "total": sum(ours.values()), "designer_total": sum(theirs.values()),
                         "values": values})
    kinds = Counter()
    for row in rows:
        kinds.update(row["beyond"])
    print("| run | board | ours | designer | beyond the designer, by pair | mask exp. | web | mask-cu | tented |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- | --- |")
    for row in rows:
        values = row["values"]
        beyond = ", ".join(f"{kind} {count}" for kind, count in sorted(row["beyond"].items())) or "-"
        print(f"| {row['run']} | {row['board']} | {row['total']} | {row['designer_total']} | {beyond} | "
              f"{values.get('pad_to_mask', '')} | {values.get('min_web', '')} | {values.get('mask_to_copper', '')} | "
              f"{values.get('tented', '')} |")
    print()
    print("beyond the designer, by pair: " + ", ".join(f"{kind} {count}" for kind, count in kinds.most_common()))
    if args.json:
        json.dump(rows, open(args.json, "w"), indent=1)


if __name__ == "__main__":
    main()
