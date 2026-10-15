# Copyright (C) 2026 Toit contributors.
"""Runs `pcb-maker drc-kicad-board` (DRC cache off) on each board with
pcb-maker's library-free KiCad configuration and with the user's
(PCB_MAKER_KICAD_USER_LIBS=1), alternating, and compares the findings per
type and severity; prints wall, CPU and peak RSS (pcb-maker and kicad-cli).

Usage: judge_override.py <pcb-maker binary> <out-dir> <board.kicad_pcb>... [--cpus 0-3] [--repeat 2]
"""
import argparse, collections, json, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from timed import run


def tally(path):
    with open(path) as f:
        data = json.load(f)
    counts = collections.Counter()
    for v in data.get("violations", []):
        counts[(v["type"], v["severity"])] += 1
    counts[("unconnected_items", "error")] = len(data.get("unconnected_items", []))
    for v in data.get("schematic_parity", []):
        counts[("parity:" + v["type"], v["severity"])] += 1
    return counts


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary")
    parser.add_argument("out")
    parser.add_argument("boards", nargs="+")
    parser.add_argument("--cpus", default="0-3")
    parser.add_argument("--repeat", type=int, default=2)
    args = parser.parse_args()
    os.makedirs(args.out, exist_ok=True)
    os.environ["PCB_MAKER_NO_CACHE"] = "1"
    rows = []
    for board in args.boards:
        board = os.path.abspath(board)
        project = os.path.dirname(board)
        board_id = os.path.splitext(os.path.basename(board))[0]
        name = os.path.basename(project)
        best = {}
        for _ in range(args.repeat):
            for mode in ("user", "empty"):
                if mode == "user":
                    os.environ["PCB_MAKER_KICAD_USER_LIBS"] = "1"
                else:
                    os.environ.pop("PCB_MAKER_KICAD_USER_LIBS", None)
                report = os.path.join(args.out, f"{name}-{mode}.json")
                r = run(["taskset", "-c", args.cpus, args.binary, "drc-kicad-board", project, board_id, report])
                if not os.path.exists(report):
                    raise SystemExit(f"{name} {mode}: no report: {r['lines'][-5:]}")
                r["cpu"] = round(r["user"] + r["sys"], 1)
                del r["lines"]
                if mode not in best or r["wall"] < best[mode]["wall"]:
                    best[mode] = r
        user = tally(os.path.join(args.out, f"{name}-user.json"))
        empty = tally(os.path.join(args.out, f"{name}-empty.json"))
        differ = {f"{t}/{s}": [user[(t, s)], empty[(t, s)]] for (t, s) in sorted(set(user) | set(empty))
                  if user[(t, s)] != empty[(t, s)]}
        library = {k: v for k, v in differ.items() if k.startswith("lib_footprint") and k.endswith("/warning")}
        other = {k: v for k, v in differ.items() if k not in library}
        row = {"board": name, "user": best["user"], "empty": best["empty"], "library_warnings": library, "other_differences": other}
        rows.append(row)
        print(json.dumps(row), flush=True)
    print()
    print("| board | wall s user / empty | CPU s user / empty | peak RSS MB user / empty | other differences |")
    print("|---|---|---|---|---|")
    for row in rows:
        u, e = row["user"], row["empty"]
        print(f"| {row['board']} | {u['wall']:.1f} / {e['wall']:.1f} | {u['cpu']:.1f} / {e['cpu']:.1f} | {u['rss_mb']} / {e['rss_mb']} | {row['other_differences'] or 'none'} |")


if __name__ == "__main__":
    main()
