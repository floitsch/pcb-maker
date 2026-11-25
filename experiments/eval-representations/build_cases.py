# Copyright (C) 2026 Florian Loitsch.
"""Builds every representation of the cases in cases.json.

Usage: build_cases.py OUT   (OUT/<case>/{dossier,plain,card,ascii}/...)
"""
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def result_dir(nightly, case):
    for sub in ("layout/result", "route"):
        d = os.path.join(nightly, case["tier"], case["name"], sub)
        if os.path.exists(os.path.join(d, "drc.json")):
            return d
    raise SystemExit("no result for " + case["id"])


def main():
    out = sys.argv[1]
    spec = json.load(open(os.path.join(HERE, "cases.json")))
    for case in spec["cases"]:
        if len(sys.argv) > 2 and case["id"] not in sys.argv[2:]:
            continue
        d = result_dir(spec["nightly"], case)
        board = [f for f in os.listdir(d) if f.endswith(".kicad_pcb")][0]
        base = [sys.executable, os.path.join(HERE, "whyopen.py"), os.path.join(d, board), "--drc", os.path.join(d, "drc.json"),
                "--router", os.path.join(d, "board-router.json"), "--item", str(case["item"])]
        cdir = os.path.join(out, case["id"])
        runs = [
            (["--formats", "dossier", "--verify", "--out", os.path.join(cdir, "dossier")]),
            (["--formats", "png", "--plain", "--out", os.path.join(cdir, "plain")]),
            (["--formats", "png", "--out", os.path.join(cdir, "card")]),
            (["--formats", "ascii", "--out", os.path.join(cdir, "ascii")]),
        ]
        for extra in runs:
            r = subprocess.run(base + extra, capture_output=True, text=True)
            if r.returncode:
                print(case["id"], "FAILED", r.stderr[-1500:])
        doss = json.load(open(os.path.join(cdir, "dossier", f"item{case['item']}.json")))
        json.dump({"result_dir": d, "board": board}, open(os.path.join(cdir, "source.json"), "w"))
        print(case["id"], case["class"], doss["cause"], "|", doss["because"][:160])


if __name__ == "__main__":
    main()
