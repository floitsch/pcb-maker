# Copyright (C) 2026 Toit contributors.
"""Starved thermals against the router's pour model, over many boards.

    python3 -I census.py <binary> <out-directory> <result-directory>...

Each result directory holds a routed `<id>.kicad_pcb` and KiCad's
`drc.json` for it. Per board: `dump-pour-map` on the routed board (the
model on the final copper), then `spokes.py` (KiCad's spokes by its DRC's
rule). Writes `<out>/<name>.json` per board and prints, per DRC-starved pad,
what the model believed, and a summary: starved pads the model also saw
short of two good spokes (a routing failure, not a fill one), and those it
believed connected (a fidelity miss), plus the model's pessimistic pads
(ours < 2 good, KiCad >= 2).
"""

import glob
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import spokes  # noqa: E402


def board_id(directory):
    boards = [path for path in glob.glob(os.path.join(directory, "*.kicad_pcb"))]
    return os.path.basename(boards[0])[:-len(".kicad_pcb")] if len(boards) == 1 else None


def name_of(directory):
    """The board's directory name, prefixed by the run's (a board may be in
    several runs)."""
    parts = os.path.normpath(directory).split(os.sep)
    for index in range(len(parts) - 1, -1, -1):
        if "__" in parts[index]:
            return (parts[index - 1] + "-" if index > 0 else "") + parts[index]
    return parts[-2] + "-" + parts[-1]


def main():
    binary, out = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    totals = {"starved": 0, "both": 0, "missed": 0, "false_alarm": 0, "model_missing": 0, "pads": 0}
    details = []
    for directory in sys.argv[3:]:
        identifier = board_id(directory)
        if identifier is None or not os.path.exists(os.path.join(directory, "drc.json")):
            print(f"skip {directory}", file=sys.stderr)
            continue
        name = name_of(directory)
        dump = os.path.join(out, name + ".dump")
        result = os.path.join(out, name + ".json")
        if not os.path.exists(result):
            subprocess.run([binary, "dump-pour-map", directory, identifier, dump], check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            subprocess.run([sys.executable, "-I", os.path.join(HERE, "spokes.py"), os.path.join(directory, identifier + ".kicad_pcb"),
                            dump, "--drc", os.path.join(directory, "drc.json"), "--json", result],
                           check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        rows = json.load(open(result))
        counts = {"starved": 0, "both": 0, "missed": 0, "false_alarm": 0, "model_missing": 0, "pads": len(rows)}
        for row in rows:
            drc = bool(row.get("drc_starved"))
            mine = spokes.our_starved(row)
            counts["starved"] += drc
            if drc and "ours" not in row:
                counts["model_missing"] += 1
            elif drc and mine:
                counts["both"] += 1
            elif drc:
                counts["missed"] += 1
            elif mine:
                counts["false_alarm"] += 1
            if drc or mine:
                verdict = "both" if drc and mine else "MISSED" if drc else "false alarm"
                details.append(f"  {name[:40]:40} {row['pad']:10} {row['layer']:7} kicad {row.get('kicad')}+{row.get('kicad_single')}s+{row.get('kicad_tracks')}t {row.get('directions')} ours {row.get('our_spokes')} +{row.get('ours_tracks')}t -> {verdict}")
        print(name, json.dumps(counts))
        for key in totals:
            totals[key] += counts[key]
    print("total", json.dumps(totals))
    print("\n".join(details))


if __name__ == "__main__":
    main()
