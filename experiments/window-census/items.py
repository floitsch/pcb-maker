# Copyright (C) 2026 Florian Loitsch.
"""Lists the near-miss boards of a nightly tier and their KiCad unconnected
items (window census, docs/experiments/window-census.md)."""
import json, math, os, sys

NIGHTLY = "/home/flo/programming/pcb-maker/build/nightly"


def result_dir(tier_dir, name):
    for sub in ("layout/result", "route"):
        d = os.path.join(tier_dir, name, sub)
        if os.path.exists(os.path.join(d, "drc.json")):
            return d
    return None


def near_misses(night, tier, low=1, high=7):
    rows = [json.loads(l) for l in open(f"{NIGHTLY}/{night}/{tier}.log") if l.startswith("{")]
    out = []
    for r in rows:
        u = r.get("native", {}).get("unconnected")
        if r.get("pass_strict") or u is None or not (low <= u <= high):
            continue
        d = result_dir(f"{NIGHTLY}/{night}/{tier}", r["name"])
        if d is None:
            continue
        board = [f for f in os.listdir(d) if f.endswith(".kicad_pcb")][0]
        drc = json.load(open(os.path.join(d, "drc.json")))
        items = []
        for it in drc.get("unconnected_items", []):
            ends = [(i["pos"]["x"], i["pos"]["y"], i["description"], i.get("uuid")) for i in it["items"]]
            items.append(ends)
        other = {k: v for k, v in (r.get("new_findings_strict") or {}).items()}
        out.append({
            "tier": f"{night}/{tier}", "name": r["name"], "board": os.path.join(d, board),
            "unconnected": u, "other_new_findings": other,
            "binary": r.get("binary", "").split("/")[-1], "items": items,
        })
    return out


if __name__ == "__main__":
    boards = near_misses("2026-11-18", "sweep") + near_misses("2026-11-24", "designer")
    json.dump(boards, open(sys.argv[1], "w"), indent=1)
    n = 0
    for b in boards:
        for ends in b["items"]:
            n += 1
            (x0, y0, d0, _), (x1, y1, d1, _) = ends[0], ends[-1]
            print(f"{b['tier'][11:]:9} {b['name'][:40]:40} {math.hypot(x1-x0, y1-y0):6.2f}  {d0[:45]:45} | {d1[:45]}")
    print(len(boards), "boards", n, "items")
