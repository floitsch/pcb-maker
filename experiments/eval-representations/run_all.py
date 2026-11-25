# Copyright (C) 2026 Florian Loitsch.
"""Runs whyopen.py over the window census's items and compares its causes
with the census's classes (docs/experiments/evaluation-representations.md).

Usage: run_all.py ITEMS.json SUMMARY.json OUT [--jobs N]
"""
import json
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))


def run(board, out):
    d = os.path.dirname(board["board"])
    name = board["tier"].replace("/", "-") + "__" + board["name"]
    odir = os.path.join(out, name)
    cmd = [sys.executable, os.path.join(HERE, "whyopen.py"), board["board"], "--drc", os.path.join(d, "drc.json"), "--out", odir, "--formats", "dossier"]
    if "--verify" in sys.argv:
        cmd.append("--verify")
    router = os.path.join(d, "board-router.json")
    if os.path.exists(router):
        cmd += ["--router", router]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        return name, None, r.stderr[-2000:]
    return name, json.loads(r.stdout[r.stdout.index("[") :]), None


def main():
    items, summary, out = sys.argv[1:4]
    jobs = int(sys.argv[sys.argv.index("--jobs") + 1]) if "--jobs" in sys.argv else 3
    boards = json.load(open(items))
    census = {(s["tier"], s["name"], s["item"]): s for s in json.load(open(summary))}
    os.makedirs(out, exist_ok=True)
    rows = []
    with ThreadPoolExecutor(jobs) as ex:
        for board, (name, res, err) in zip(boards, ex.map(lambda b: run(b, out), boards)):
            if res is None:
                print("FAILED", name, err, file=sys.stderr)
                continue
            for r in res:
                c = census.get((board["tier"], board["name"], r["item"]))
                rows.append({"tier": board["tier"], "name": board["name"], **r, "census": c["class"] if c else None, "core": (c or {}).get("core2")})
    json.dump(rows, open(os.path.join(out, "causes.json"), "w"), indent=1)
    table = {}
    for r in rows:
        table.setdefault(r["census"], {}).setdefault(r["cause"], 0)
        table[r["census"]][r["cause"]] += 1
    for c, t in sorted(table.items(), key=lambda kv: str(kv[0])):
        print(f"{str(c):14}", ", ".join(f"{k}: {v}" for k, v in sorted(t.items())))


if __name__ == "__main__":
    main()
