# Copyright (C) 2026 Toit contributors.
"""Splits the wall of `kicad-cli pcb drc` as pcb-maker calls it into
process start, KiCad's footprint library preload, zone refill and the DRC
proper (board load is measured by daemon_drc.py through the pcbnew module).

Usage: measure_cli.py <work-dir> <board.kicad_pcb>... [--repeat N] [--cpus 0-3]

Each board is copied into <work-dir>/<stem>/ (with its .kicad_pro and
.kicad_dru). Configurations, each run --repeat times (minimum wall kept):
  called      default config, --refill-zones (pcb-maker's call on a board with zones)
  nolib       --refill-zones, KICAD_CONFIG_HOME with empty library tables
  filled      on the board saved by --refill-zones --save-board, no refill, default config
  filled_nolib  same, empty library tables
plus an empty board and `kicad-cli version`. Prints one JSON line per run
and a summary table; also checks that the per-type finding counts of
`called` and `nolib` agree (errors and warnings apart).
"""
import argparse, collections, json, os, shutil, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from timed import run

DRC = ["kicad-cli", "pcb", "drc", "--format", "json", "--severity-all"]


def empty_config(work):
    home = os.path.join(work, "config-nolib")
    version = os.path.join(home, "10.0")
    os.makedirs(version, exist_ok=True)
    with open(os.path.join(version, "fp-lib-table"), "w") as f:
        f.write("(fp_lib_table\n\t(version 7)\n)\n")
    with open(os.path.join(version, "sym-lib-table"), "w") as f:
        f.write("(sym_lib_table\n\t(version 7)\n)\n")
    common = os.path.expanduser("~/.config/kicad/10.0/kicad_common.json")
    if os.path.exists(common):
        shutil.copy(common, version)
    return home


def tally(report):
    with open(report) as f:
        data = json.load(f)
    counts = collections.Counter()
    for v in data.get("violations", []):
        counts[(v["type"], v["severity"])] += 1
    counts[("unconnected_items", "error")] = len(data.get("unconnected_items", []))
    for v in data.get("schematic_parity", []):
        counts[(v["type"], v["severity"])] += 1
    return counts


def timed(argv, cpus, env_home, cwd, repeat):
    best = None
    old = os.environ.get("KICAD_CONFIG_HOME")
    if env_home:
        os.environ["KICAD_CONFIG_HOME"] = env_home
    try:
        for _ in range(repeat):
            os.chdir(cwd)
            r = run(["taskset", "-c", cpus] + argv)
            if r["status"] != 0:
                raise SystemExit(f"failed: {argv}: {r['lines'][-5:]}")
            found = [t for t, l in r["lines"] if l.startswith("Found ")]
            saved = [t for t, l in r["lines"] if l.startswith("Saved DRC Report")]
            r["report_write"] = round(saved[0] - found[0], 3) if found and saved else None
            del r["lines"]
            if best is None or r["wall"] < best["wall"]:
                best = r
    finally:
        if env_home:
            if old is None:
                del os.environ["KICAD_CONFIG_HOME"]
            else:
                os.environ["KICAD_CONFIG_HOME"] = old
    return best


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("work")
    parser.add_argument("boards", nargs="+")
    parser.add_argument("--repeat", type=int, default=2)
    parser.add_argument("--cpus", default="0-3")
    args = parser.parse_args()
    work = os.path.abspath(args.work)
    os.makedirs(work, exist_ok=True)
    nolib = empty_config(work)
    rows = {}

    version = timed(["kicad-cli", "version"], args.cpus, None, work, args.repeat)
    empty_dir = os.path.join(work, "empty")
    os.makedirs(empty_dir, exist_ok=True)
    empty = os.path.join(empty_dir, "empty.kicad_pcb")
    if not os.path.exists(empty):
        import subprocess
        subprocess.run([sys.executable, "-c",
                        "import pcbnew,sys; b=pcbnew.NewBoard(sys.argv[1]); pcbnew.SaveBoard(sys.argv[1], b)", empty],
                       check=True, capture_output=True)
    report = os.path.join(empty_dir, "report.json")
    fixed = {
        "version": version,
        "empty": timed(DRC + ["--refill-zones", "--output", report, empty], args.cpus, None, empty_dir, args.repeat),
        "empty_nolib": timed(DRC + ["--refill-zones", "--output", report, empty], args.cpus, nolib, empty_dir, args.repeat),
    }
    print(json.dumps({"fixed": fixed}), flush=True)

    for board in args.boards:
        board = os.path.abspath(board)
        stem = os.path.splitext(os.path.basename(board))[0]
        name = os.path.basename(os.path.dirname(board)) or stem
        base = os.path.join(work, name)
        filled = os.path.join(work, name + "-filled")
        for target in (base, filled):
            os.makedirs(target, exist_ok=True)
            for ext in (".kicad_pcb", ".kicad_pro", ".kicad_dru"):
                source = os.path.join(os.path.dirname(board), stem + ext)
                if os.path.exists(source):
                    shutil.copy(source, os.path.join(target, stem + ext))
        pcb = stem + ".kicad_pcb"
        # KiCad's own fill, saved: the DRC proper is then measured without the refill.
        os.chdir(filled)
        save = run(["taskset", "-c", args.cpus] + DRC + ["--refill-zones", "--save-board", "--output", "save.json", pcb])
        if save["status"] != 0:
            raise SystemExit(f"save failed for {name}")
        row = {}
        row["called"] = timed(DRC + ["--refill-zones", "--output", "called.json", pcb], args.cpus, None, base, args.repeat)
        row["nolib"] = timed(DRC + ["--refill-zones", "--output", "nolib.json", pcb], args.cpus, nolib, base, args.repeat)
        row["filled"] = timed(DRC + ["--output", "filled.json", pcb], args.cpus, None, filled, args.repeat)
        row["filled_nolib"] = timed(DRC + ["--output", "filled_nolib.json", pcb], args.cpus, nolib, filled, args.repeat)
        a = tally(os.path.join(base, "called.json"))
        b = tally(os.path.join(base, "nolib.json"))
        row["findings_called"] = {f"{t}/{s}": n for (t, s), n in sorted(a.items()) if n}
        row["findings_differ"] = {f"{t}/{s}": [a.get((t, s), 0), b.get((t, s), 0)]
                                  for (t, s) in sorted(set(a) | set(b)) if a.get((t, s), 0) != b.get((t, s), 0)}
        rows[name] = row
        print(json.dumps({name: row}), flush=True)

    start = fixed["version"]["wall"]
    library = fixed["empty"]["wall"] - fixed["empty_nolib"]["wall"]
    print()
    print("| board | called wall s | CPU s | RSS MB | process+init | library preload | refill | DRC proper + load | report write | nolib wall | nolib RSS MB |")
    print("|---|---|---|---|---|---|---|---|---|---|---|")
    for name, row in rows.items():
        c = row["called"]
        lib = c["wall"] - row["nolib"]["wall"]
        refill = row["nolib"]["wall"] - row["filled_nolib"]["wall"]
        init = fixed["empty_nolib"]["wall"]
        drc = row["filled_nolib"]["wall"] - init
        w = c["wall"]
        pct = lambda x: f"{x:.1f} ({100 * x / w:.0f}%)"
        print(f"| {name} | {w:.1f} | {c['user'] + c['sys']:.1f} | {c['rss_mb']} | {pct(init)} | {pct(lib)} | {pct(refill)} | {pct(drc)} | {c['report_write']} | {row['nolib']['wall']:.1f} | {row['nolib']['rss_mb']} |")


if __name__ == "__main__":
    main()
