# Copyright (C) 2026 Toit contributors.
"""ERC (`kicad-cli sch erc --format json --severity-all`) on each schematic
with the user's configuration and with empty library tables
(KICAD_CONFIG_HOME=<home>), alternating; findings per type and severity
compared, wall, CPU and peak RSS printed.

Usage: judge_erc.py <empty-config-home> <out-dir> <root.kicad_sch>... [--cpus 0-3]
"""
import argparse, collections, json, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from timed import run


def tally(path):
    with open(path) as f:
        data = json.load(f)
    counts = collections.Counter()
    for sheet in data.get("sheets", []):
        for v in sheet.get("violations", []):
            counts[(v["type"], v["severity"])] += 1
    return counts


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("home")
    parser.add_argument("out")
    parser.add_argument("schematics", nargs="+")
    parser.add_argument("--cpus", default="0-3")
    args = parser.parse_args()
    os.makedirs(args.out, exist_ok=True)
    for schematic in args.schematics:
        schematic = os.path.abspath(schematic)
        name = os.path.basename(os.path.dirname(schematic))
        results, counts = {}, {}
        for mode in ("user", "empty"):
            env = dict(os.environ)
            if mode == "empty":
                os.environ["KICAD_CONFIG_HOME"] = args.home
            report = os.path.join(args.out, f"{name}-{mode}.json")
            os.chdir(os.path.dirname(schematic))
            r = run(["taskset", "-c", args.cpus, "kicad-cli", "sch", "erc", "--format", "json", "--severity-all",
                     "--output", report, os.path.basename(schematic)])
            os.environ.clear()
            os.environ.update(env)
            if r["status"] != 0 or not os.path.exists(report):
                raise SystemExit(f"{name} {mode}: {r['lines'][-5:]}")
            results[mode] = {"wall": r["wall"], "cpu": round(r["user"] + r["sys"], 1), "rss_mb": r["rss_mb"]}
            counts[mode] = tally(report)
        u, e = counts["user"], counts["empty"]
        differ = {f"{t}/{s}": [u[(t, s)], e[(t, s)]] for (t, s) in sorted(set(u) | set(e)) if u[(t, s)] != e[(t, s)]}
        print(json.dumps({"schematic": name, "user": results["user"], "empty": results["empty"],
                          "total": [sum(u.values()), sum(e.values())], "differ": differ}), flush=True)


if __name__ == "__main__":
    main()
