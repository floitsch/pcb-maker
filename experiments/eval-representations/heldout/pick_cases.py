# Copyright (C) 2026 Florian Loitsch.
import json, os
N = "/home/flo/programming/pcb-maker/build/nightly/2026-11-24"
R = "/home/flo/programming/pcb-maker"
study = {c["name"] for c in json.load(open(R + "-cards-heldout/experiments/eval-representations/cases.json"))["cases"]}
hold = {b["name"] for b in json.load(open(R + "/benchmarks/expansion/holdout/manifest.json"))["boards"]}
census = {(b["tier"], b["name"]) for b in json.load(open(R + "/build/scratch/fund-window/items.json"))}
stages = {"sweep": "layout/result", "designer": "route"}
pop = {}
for st, sub in stages.items():
    rows = []
    for b in sorted(os.listdir(f"{N}/{st}")):
        p = f"{N}/{st}/{b}/{sub}/drc.json"
        if not os.path.exists(p):
            continue
        n = len(json.load(open(p)).get("unconnected_items", []))
        if 1 <= n <= 6 and b not in study and b not in hold:
            rows.append((b, n))
    nc = [r for r in rows if ("2026-11-24/" + st, r[0]) not in census]
    print(st, "after a,b:", len(rows), "after c:", len(nc))
    pop[st] = nc if len(nc) >= 6 else rows
picks = []
for st in ("sweep", "designer"):
    rows = pop[st]
    n = len(rows); k = max(1, n // 6)
    taken = {p[1] for p in picks}
    idx = k // 2; got = 0; seen = 0
    while got < 6 and seen < 2 * n:
        i = idx % n
        b, cnt = rows[i]
        if b in taken:
            idx += 1; seen += 1; continue  # the next index up, wrapping; seen bounds the loop at 2n

        picks.append((st, b, cnt)); taken.add(b); got += 1
        idx += k; seen += 1
for p in picks:
    print(*p)
json.dump([{"tier": "2026-11-24/" + s, "name": b, "unconnected": c, "item": 0} for s, b, c in picks],
          open(R + "/build/scratch/cards-heldout/picks.json", "w"), indent=1)
