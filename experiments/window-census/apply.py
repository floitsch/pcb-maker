# Copyright (C) 2026 Florian Loitsch.
"""The census's SAT answers applied to whole boards, one window after the
other (docs/experiments/window-census.md): from the routed board, each
KiCad unconnected item in turn is censused and solved on the board as the
windows before left it (half-size 2 mm, then 4 mm); a solution is kept
when KiCad's DRC finds fewer unconnected items and no error type more
often than before. The final board is a strict pass when KiCad finds no
unconnected item and no error beyond the routed board's own.

    apply.py items.json census-results.jsonl outdir [--jobs 3]
"""
import argparse, concurrent.futures as cf, json, math, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run as R


def no_worse(after, before):
    return all(after["violations"].get(t, 0) <= before["violations"].get(t, 0) for t in after["violations"])


def items_of(drc_path):
    d = json.load(open(drc_path))
    out = []
    for u in d.get("unconnected_items", []):
        ends = [(i["pos"]["x"], i["pos"]["y"], i["description"]) for i in u["items"]]
        out.append(ends)
    return out


def apply_board(task):
    b, out, args = task["board"], task["out"], task["args"]
    os.makedirs(out, exist_ok=True)
    rdir = os.path.dirname(b["board"])
    pitch, lines, attempt = R.board_settings(rdir)
    base = R.baseline(b["board"], out)
    log = {"tier": b["tier"], "name": b["name"], "baseline": base, "steps": []}
    current = b["board"]
    current_drc = os.path.join(out, "baseline-drc.json")
    summary = base
    tried = set()
    for step in range(12):
        todo = None
        for ends in items_of(current_drc):
            key = tuple(sorted((round(e[0], 1), round(e[1], 1)) for e in ends))
            nets = {R.net_of(e[2]) for e in ends}
            if key in tried or len(nets) != 1 or any("Zone [" in e[2] for e in ends):
                continue
            todo = (key, ends, nets.pop())
            break
        if todo is None:
            break
        key, ends, net = todo
        tried.add(key)
        pts = [[e[0], e[1]] for e in ends]
        kept = False
        for h in args.half:
            job = {"tier": b["tier"], "name": b["name"], "board": current, "item": step, "half": h,
                   "net": net, "ends": pts, "length": round(math.hypot(pts[0][0] - pts[-1][0], pts[0][1] - pts[-1][1]), 2),
                   "pitch": pitch, "pin_lines": lines, "limit": args.limit, "core_limit": 0,
                   "stage_limit": args.stage_limit, "slack": args.slack, "other_slack": args.other_slack,
                   "work": os.path.join(out, f"step{step}-h{h:g}"), "no_core": True}
            r = R.run_job(job)
            entry = {"step": step, "half": h, "net": net, "ends": pts, "status": r.get("status"),
                     "moved": r.get("moved"), "drc": r.get("drc")}
            if r.get("status") == "sat" or (r.get("full") or {}).get("status") == "sat":
                after = r.get("drc")
                if after and after["unconnected"] < summary["unconnected"] and no_worse(after, base):
                    current = os.path.join(job["work"], "solution", os.path.basename(b["board"]))
                    current_drc = os.path.join(job["work"], "solution-drc.json")
                    summary = after
                    entry["kept"] = True
                    kept = True
            log["steps"].append(entry)
            if kept:
                break
    log["final"] = summary
    log["final_board"] = current
    log["strict_pass"] = summary["unconnected"] == 0 and no_worse(summary, base) and not b["other_new_findings"]
    log["closed_all_opens"] = summary["unconnected"] == 0 and no_worse(summary, base)
    json.dump(log, open(os.path.join(out, "apply.json"), "w"), indent=1)
    return log


def main():
    p = argparse.ArgumentParser()
    p.add_argument("items")
    p.add_argument("results")
    p.add_argument("outdir")
    p.add_argument("--jobs", type=int, default=3)
    p.add_argument("--limit", type=float, default=600)
    p.add_argument("--stage-limit", type=float, default=120)
    p.add_argument("--half", type=float, nargs="+", default=[2.0, 4.0])
    p.add_argument("--slack", type=float, default=20)
    p.add_argument("--other-slack", type=float, default=10)
    p.add_argument("--all", action="store_true", help="every board, not only those with a SAT item")
    a = p.parse_args()
    boards = json.load(open(a.items))
    sat = set()
    for line in open(a.results):
        r = json.loads(line)
        if r.get("status") == "sat" or (r.get("full") or {}).get("status") == "sat":
            sat.add((r["tier"], r["name"]))
    tasks = [{"board": b, "out": os.path.join(a.outdir, b["tier"].replace("/", "-"), b["name"]), "args": a}
             for b in boards if a.all or (b["tier"], b["name"]) in sat]
    print(len(tasks), "boards", flush=True)
    with cf.ProcessPoolExecutor(max_workers=a.jobs) as pool:
        for log in pool.map(apply_board, tasks):
            print(log["tier"], log["name"], log["baseline"]["unconnected"], "->", log["final"]["unconnected"],
                  "strict" if log["strict_pass"] else "", flush=True)


if __name__ == "__main__":
    main()
