# Copyright (C) 2026 Toit contributors.
"""A census again with a new pour model, KiCad's side reused.

    python3 -I rejoin.py <binary> <old-census> <new-census> <result-directory>...

`census.py` refills every board in pcbnew to count KiCad's spokes; that
side does not change with the router. This takes each board's rows from
`<old-census>/<name>.json` (KiCad's counts and the DRC's verdicts), runs
`dump-pour-map` of `<binary>` on the board again, replaces the model's
fields, writes `<new-census>/<name>.json` and prints the totals as
`census.py` does (both, missed, false alarms).
"""

import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import census  # noqa: E402
import spokes  # noqa: E402

MODEL_FIELDS = ["ours", "ours_island", "ours_tracks", "our_spokes", "round"]


def main():
    binary, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
    os.makedirs(new, exist_ok=True)
    totals = {"starved": 0, "both": 0, "missed": 0, "false_alarm": 0, "model_missing": 0, "pads": 0}
    details = []
    for directory in sys.argv[4:]:
        identifier = census.board_id(directory)
        name = census.name_of(directory)
        source = os.path.join(old, name + ".json")
        if identifier is None or not os.path.exists(source):
            print(f"skip {directory}", file=sys.stderr)
            continue
        dump = os.path.join(new, name + ".dump")
        result = os.path.join(new, name + ".json")
        if not os.path.exists(result):
            subprocess.run([binary, "dump-pour-map", directory, identifier, dump], check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            model = spokes.ours(dump)
            by_label = {}
            for key in model:
                by_label.setdefault(key[:2], []).append(key)
            rows = json.load(open(source))
            for row in rows:
                for field in MODEL_FIELDS:
                    row.pop(field, None)
                if row.get("at") is None:
                    continue
                best = None
                for candidate in by_label.get((row["pad"], row["layer"]), []):
                    distance = (candidate[2] - row["at"][0]) ** 2 + (candidate[3] - row["at"][1]) ** 2
                    if distance < 0.05 ** 2 and (best is None or distance < best[0]):
                        best = (distance, candidate)
                if best:
                    row.update(model[best[1]])
            json.dump(rows, open(result, "w"), indent=1)
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
        sys.stdout.flush()
        for key in totals:
            totals[key] += counts[key]
    print("total", json.dumps(totals))
    print("\n".join(details))


if __name__ == "__main__":
    main()
