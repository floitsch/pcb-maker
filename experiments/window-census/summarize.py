# Copyright (C) 2026 Florian Loitsch.
"""Tables of the window census (docs/experiments/window-census.md).

    summarize.py items.json census-dir [census-dir-h4] [--apply apply-dir]

Per item, the classification over both half-sizes:
  sat-kicad    a joint solution on the lattice, and KiCad finds the item
               closed with no error type more often than on the routed board
  sat-lattice  a solution on the lattice that KiCad does not accept
  unsat        no solution in the window (the full region without a detour
               budget, or the budget's UNSAT when the full run timed out)
  walled-dead  a side of the item is a pad with no lattice node at all (a
               dead pad: the class's track admitted by no node near it)
  walled-pads  a side of the item no lattice path reaches inside the window
               even without any copper: pads, graphics, rule areas
  walled-cu    walled by copper kept as it is (differential pairs, nets the
               lattice cannot rejoin, copper outside the window)
  timeout      no answer in the time limit
  zone         an item between pieces of a pour (fill connectivity, not the
               lattice's)
  too-big      the window's connections span more than 150,000 nodes or
               the CNF would pass 12 million clauses
  long         the item's window would be wider than 20 mm
  error        the census failed (pads off the board, ...)
"""
import argparse, json, os, sys
from collections import Counter, defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run as R

ORDER = ["sat-kicad", "sat-lattice", "unsat", "walled-dead", "walled-pads", "walled-cu", "timeout", "too-big",
         "zone", "long", "error"]


def load(path):
    out = {}
    if path and os.path.exists(path):
        for line in open(path):
            r = json.loads(line)
            out[(r["tier"], r["name"], r["item"])] = r
    return out


def classify(r, base):
    if r is None:
        return None
    s = r.get("status")
    full = (r.get("full") or {}).get("status")
    if s == "sat" or full == "sat":
        d = r.get("drc")
        if d and d["unconnected"] < base["unconnected"] and R_no_worse(d, base):
            return "sat-kicad"
        return "sat-lattice"
    if s == "unsat":
        return "unsat" if full in ("unsat", "timeout", None, "separated") else "unsat"
    if s == "target_walled":
        if r.get("dead_terminals"):
            return "walled-dead"
        return "walled-pads" if r.get("walled_without_copper") else "walled-cu"
    if s == "too_big":
        return "too-big"
    if s == "timeout":
        return "timeout"
    if s == "zone_item":
        return "zone"
    if s == "long":
        return "long"
    return "error"


def R_no_worse(after, before):
    return all(after["violations"].get(t, 0) <= before["violations"].get(t, 0) for t in after["violations"])


def main():
    p = argparse.ArgumentParser()
    p.add_argument("items")
    p.add_argument("h2")
    p.add_argument("h4", nargs="?")
    p.add_argument("--apply", default=None)
    p.add_argument("--json", default=None)
    p.add_argument("--markdown", default=None, help="write the per-item table here")
    a = p.parse_args()
    boards = json.load(open(a.items))
    r2 = load(os.path.join(a.h2, "results.jsonl"))
    r4 = load(os.path.join(a.h4, "results.jsonl")) if a.h4 else {}
    rows = []
    for b in boards:
        base = R.baseline(b["board"], os.path.join(a.h2, b["tier"].replace("/", "-"), b["name"]))
        for i, ends in enumerate(b["items"]):
            k = (b["tier"], b["name"], i)
            c2 = classify(r2.get(k), base)
            c4 = classify(r4.get(k), base)
            if c4 is None and c2 in ("sat-kicad", "zone"):
                c4 = c2 + " (implied)" if c2 == "sat-kicad" else c2
            best = c2
            if c2 not in ("sat-kicad",) and c4 and c4 in ORDER and ORDER.index(c4) < ORDER.index(c2 or "error"):
                best = c4
            x = r2.get(k) or {}
            y = r4.get(k) or {}
            rows.append({"tier": b["tier"], "name": b["name"], "item": i, "net": x.get("net"),
                         "length": x.get("length"), "h2": c2, "h4": c4, "class": best,
                         "moved2": x.get("moved"), "moved4": y.get("moved"),
                         "core2": x.get("core"), "core4": y.get("core"),
                         "full2": (x.get("full") or {}).get("status"), "full4": (y.get("full") or {}).get("status"),
                         "nets2": x.get("window_nets"), "nets4": y.get("window_nets"),
                         "unrepro2": x.get("unreproducible"), "fixed2": len(x.get("fixed_in_window") or []),
                         "descriptions": x.get("descriptions"), "other_new_findings": b["other_new_findings"],
                         "binary": b["binary"], "attempt": x.get("attempt"),
                         "seconds2": x.get("seconds"), "vars2": x.get("variables"),
                         "seconds4": y.get("seconds"), "vars4": y.get("variables"),
                         "stages2": x.get("stages"), "stages4": y.get("stages"),
                         "dead2": x.get("dead_terminals")})
    if a.json:
        json.dump(rows, open(a.json, "w"), indent=1)
    if a.markdown:
        with open(a.markdown, "w") as f:
            f.write("| tier | board | net | ends mm | h = 2 | h = 4 | note |\n| --- | --- | --- | ---: | --- | --- | --- |\n")
            for r in rows:
                note = []
                for h in ("2", "4"):
                    if r["moved" + h] is not None:
                        note.append(f"h{h}: {r['moved' + h]} other nets free")
                    if r["core" + h]:
                        note.append(f"h{h} core ({len(r['core' + h])}): " + ", ".join(r["core" + h]))
                    if r["full" + h]:
                        note.append(f"h{h} without budget: {r['full' + h]}")
                    stages = r["stages" + h] or []
                    if r["h" + h] in ("timeout", "unsat") and stages:
                        shown = ", ".join(f"{st['free']}: {st['status']}" for st in stages)
                        note.append(f"h{h} stages (nets free: answer) {shown}")
                if r["dead2"]:
                    note.append("dead pad " + ", ".join(r["dead2"]))
                board = r["name"].split("__")[-1]
                tier = "sweep" if "sweep" in r["tier"] else "designer"
                net = (r["net"] or "").replace("|", "/")
                f.write(f"| {tier} | {board} | {net} | {r['length']} | {r['h2']} | {r['h4'] or ''} | {'; '.join(note)} |\n")
    by_tier = defaultdict(Counter)
    for r in rows:
        by_tier[r["tier"]][r["class"]] += 1
        by_tier["all"][r["class"]] += 1
    for t, c in by_tier.items():
        n = sum(c.values())
        print(t, n, {k: c[k] for k in ORDER if c[k]})
    for h in ("h2", "h4"):
        c = Counter(r[h] for r in rows if r[h])
        print(h, dict(c))
    for r in rows:
        print(f'{r["tier"][11:]:8} {r["name"][:34]:34} {r["item"]} {str(r["net"])[:22]:22} {r["length"]!s:6} '
              f'{r["h2"]!s:12} {r["h4"]!s:20} m={r["moved2"]} core={r["core2"]} full={r["full2"]}')


if __name__ == "__main__":
    main()
