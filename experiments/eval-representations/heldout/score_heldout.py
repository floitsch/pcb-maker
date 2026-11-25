# Copyright (C) 2026 Florian Loitsch.
"""The study's scoring rule (answer cause == truth class) on the held-out cases."""
import json, os
H = os.path.dirname(os.path.abspath(__file__))
truth = {f"h{i}": t for i, t in enumerate(json.load(open(f"{H}/truth.json")))}
cards = json.load(open(f"{H}/card_verdicts.json"))
rows = [json.loads(l) for l in open(f"{H}/results.jsonl") if l.strip()]
MAP = {"walled_by_routed_copper": "B", "walled_by_fixed_objects": "C", "end_walled_by_fixed_objects": "C", "class_width_does_not_fit": "C",
       "long": "E", "single_net_path_exists": "A", "pour_island": "D", "separate_board_outlines": "C", "raster_path_rejected_by_kicad": "-"}
for rep in ("card", "raw"):
    rs = [r for r in rows if r["subject"].endswith(rep)]
    ok = [r for r in rs if r["answer"]["cause"] == truth[r["subject"].split("-")[0]]["class"]]
    sure = [r for r in rs if truth[r["subject"].split("-")[0]]["confidence"] >= 60]
    oks = [r for r in sure if r in ok]
    m = lambda k: sum(r[k] for r in rs) / len(rs)
    print(f"{rep}: {len(ok)}/{len(rs)} correct; truth confidence >= 60: {len(oks)}/{len(sure)}; mean tokens {m('tokens')/1000:.0f}k, tool uses {m('tool_uses'):.1f}, seconds {m('seconds'):.0f}")
print()
for i in range(12):
    c = f"h{i}"; t = truth[c]
    d = cards[c]
    cells = []
    for rep in ("card", "raw"):
        r = [r for r in rows if r["subject"] == f"{c}-{rep}"]
        cells.append(f"{r[0]['answer']['cause']} ({r[0]['answer']['confidence']}){'' if r[0]['answer']['cause'] == t['class'] else ' x'} {r[0]['tokens']//1000}k {r[0]['seconds']}s" if r else "-")
    print(c, t["class"], t["confidence"], d["cause"], MAP.get(d["cause"]), "|", " | ".join(cells))
