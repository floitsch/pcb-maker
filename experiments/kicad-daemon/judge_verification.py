# Copyright (C) 2026 Toit contributors.
"""`pcb-maker verify-kicad-rung` (caches off) on copies of each project
directory, with the user's KiCad configuration (PCB_MAKER_KICAD_USER_LIBS=1)
and without the global libraries, alternating; compares verification.json:
every field but `erc_library_warnings` must agree.

Usage: judge_verification.py <pcb-maker binary> <work-dir> <project-dir>:<board-id>... [--cpus 0-3]
"""
import argparse, json, os, shutil, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from timed import run


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary")
    parser.add_argument("work")
    parser.add_argument("projects", nargs="+")
    parser.add_argument("--cpus", default="0-3")
    args = parser.parse_args()
    os.environ["PCB_MAKER_NO_CACHE"] = "1"
    os.environ.pop("PCB_MAKER_KICAD_VERIFICATION_CACHE", None)
    for spec in args.projects:
        source, board_id = spec.rsplit(":", 1)
        name = os.path.basename(os.path.dirname(os.path.dirname(source.rstrip("/")))) or board_id
        reports = {}
        for mode in ("user", "empty"):
            target = os.path.join(args.work, f"{name}-{mode}")
            shutil.rmtree(target, ignore_errors=True)
            shutil.copytree(source, target)
            if mode == "user":
                os.environ["PCB_MAKER_KICAD_USER_LIBS"] = "1"
            else:
                os.environ.pop("PCB_MAKER_KICAD_USER_LIBS", None)
            r = run(["taskset", "-c", args.cpus, args.binary, "verify-kicad-rung", target, board_id])
            path = os.path.join(target, "verification.json")
            if not os.path.exists(path):
                raise SystemExit(f"{name} {mode}: no verification.json: {r['lines'][-5:]}")
            with open(path) as f:
                report = json.load(f)
            reports[mode] = (report, {"wall": r["wall"], "cpu": round(r["user"] + r["sys"], 1), "rss_mb": r["rss_mb"]})
        user, empty = reports["user"][0], reports["empty"][0]
        differ = {key: [user.get(key), empty.get(key)] for key in sorted(set(user) | set(empty))
                  if key != "erc_library_warnings" and user.get(key) != empty.get(key)}
        print(json.dumps({"project": name, "user": reports["user"][1], "empty": reports["empty"][1],
                          "erc_violations": user["erc_violations"], "complete": user["complete"],
                          "erc_library_warnings": [user.get("erc_library_warnings"), empty.get("erc_library_warnings")],
                          "differ": differ}), flush=True)


if __name__ == "__main__":
    main()
