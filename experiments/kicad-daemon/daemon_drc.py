# Copyright (C) 2026 Toit contributors.
"""Prototype of a persistent KiCad checker: one Python process with KiCad's
pcbnew module (SWIG bindings, KiCad 10) loads boards, refills their zones
and runs the DRC, more than once, timing each step.

Usage: daemon_drc.py <board.kicad_pcb> [<kicad-cli report.json>] [...]
       [--no-refill] [--rounds N] [--out DIR]

Per board: LoadBoard, ZONE_FILLER.Fill, WriteDRCReport (DRC 1); DRC 2 on
the same loaded board; then N fresh rounds (LoadBoard + fill + DRC from
the file again, the call a daemon behind run_kicad_report would serve).
With a kicad-cli JSON report after a board the finding counts per type
and severity are compared (WriteDRCReport writes KiCad's text report;
it is parsed back into counts; it adds no markers to the board in 10.0.7).
Prints one JSON line per board.
"""
import collections, json, os, re, resource, sys, time

T0 = time.monotonic()
import ctypes  # noqa: E402
import pcbnew  # noqa: E402


def bootstrap_pgm():
    """KiCad 10's DRC needs Pgm() (the library manager, for the library
    parity test); standalone pcbnew has none and WriteDRCReport segfaults
    (assert "process" failed in Pgm()). Build what kicad-cli builds: a
    console wxApp (wxInitialize) and a PGM_BASE, initialised headless and
    installed with SetPgm, through the exported C++ symbols of
    libkicommon (KiCad 10.0.7). PGM_BASE's pure virtuals are never called
    on this path. The buffer is larger than sizeof(PGM_BASE) and is never
    freed (the process owns it until it exits)."""
    mode = os.RTLD_NOLOAD | os.RTLD_GLOBAL
    wx = ctypes.CDLL("libwx_baseu-3.2.so.0", mode=mode)
    initialize = wx["_Z12wxInitializeRiPPc"]
    initialize.restype = ctypes.c_bool
    argc = ctypes.c_int(1)
    argv = (ctypes.c_char_p * 2)(b"kicad-daemon", None)
    if not initialize(ctypes.byref(argc), argv):
        raise SystemExit("wxInitialize failed")
    common = ctypes.CDLL("libkicommon.so.10.0.7", mode=mode)
    construct = common["_ZN8PGM_BASEC1Ev"]
    construct.argtypes = [ctypes.c_void_p]
    init = common["_ZN8PGM_BASE7InitPgmEbbb"]
    init.argtypes = [ctypes.c_void_p, ctypes.c_bool, ctypes.c_bool, ctypes.c_bool]
    init.restype = ctypes.c_bool
    set_pgm = common["_Z6SetPgmP8PGM_BASE"]
    set_pgm.argtypes = [ctypes.c_void_p]
    program = ctypes.create_string_buffer(1 << 20)
    construct(program)
    set_pgm(program)
    if not init(program, True, True, False):
        raise SystemExit("PGM_BASE::InitPgm failed")
    return (argc, argv, program)  # kept alive


PGM = bootstrap_pgm()
IMPORT_SECONDS = time.monotonic() - T0


def rss_mb():
    with open("/proc/self/status") as f:
        for line in f:
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) // 1024
    return None


def peak_mb():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss // 1024


def parse_text_report(path):
    """Counts per (type, severity) of KiCad's text DRC report; the
    unconnected items section is counted as unconnected_items/error and the
    footprint (schematic parity) section is kept apart."""
    counts = collections.Counter()
    section = None
    current = None
    with open(path, errors="replace") as f:
        for line in f:
            if line.startswith("** Found"):
                if "unconnected" in line:
                    section = "unconnected"
                elif "Footprint" in line or "footprint" in line:
                    section = "footprints"
                else:
                    section = "violations"
                continue
            m = re.match(r"^\[(\w+)\]:", line)
            if m:
                current = m.group(1)
                if section == "unconnected":
                    counts[("unconnected_items", "error")] += 1
                    current = None
                continue
            # The line after the header: "    <rule or override>; <severity>".
            m = re.match(r"^    .*; (error|warning|exclusion|info)\s*$", line)
            if m and current:
                counts[(current, m.group(1))] += 1
                current = None
    return counts


def kicad_cli_counts(path):
    with open(path) as f:
        data = json.load(f)
    counts = collections.Counter()
    for v in data.get("violations", []):
        counts[(v["type"], v["severity"])] += 1
    counts[("unconnected_items", "error")] = len(data.get("unconnected_items", []))
    for v in data.get("schematic_parity", []):
        counts[(v["type"], v["severity"])] += 1
    return counts


def cpu():
    usage = resource.getrusage(resource.RUSAGE_SELF)
    return usage.ru_utime + usage.ru_stime


def check(path, refill, report):
    times = {}
    c = cpu()
    t = time.monotonic()
    board = pcbnew.LoadBoard(path)
    times["load"] = time.monotonic() - t
    if refill:
        t = time.monotonic()
        filler = pcbnew.ZONE_FILLER(board)
        filler.Fill(board.Zones())
        times["refill"] = time.monotonic() - t
    t = time.monotonic()
    ok = pcbnew.WriteDRCReport(board, report, pcbnew.EDA_UNITS_MM, False)
    times["drc"] = time.monotonic() - t
    if not ok:
        raise SystemExit(f"WriteDRCReport failed on {path}")
    times["cpu"] = cpu() - c
    return board, times


def main():
    args = sys.argv[1:]
    refill = "--no-refill" not in args
    rounds = 2
    out = "."
    if "--rounds" in args:
        rounds = int(args[args.index("--rounds") + 1])
    if "--out" in args:
        out = args[args.index("--out") + 1]
    os.makedirs(out, exist_ok=True)
    positional = []
    skip = False
    for i, a in enumerate(args):
        if skip:
            skip = False
            continue
        if a in ("--rounds", "--out"):
            skip = True
            continue
        if a.startswith("--"):
            continue
        positional.append(a)
    jobs = []
    for a in positional:
        if a.endswith(".kicad_pcb"):
            jobs.append([os.path.abspath(a), None])
        elif jobs:
            jobs[-1][1] = a
    print(json.dumps({"import_seconds": round(IMPORT_SECONDS, 3), "rss_mb": rss_mb()}), flush=True)
    for path, reference in jobs:
        stem = os.path.splitext(os.path.basename(path))[0].replace(" ", "_")
        report = os.path.join(out, stem + ".rpt")
        row = {"board": path, "rounds": []}
        board, first = check(path, refill, report)
        row["first"] = {k: round(v, 3) for k, v in first.items()}
        t = time.monotonic()
        pcbnew.WriteDRCReport(board, report + ".2", pcbnew.EDA_UNITS_MM, False)
        row["drc_again_same_board"] = round(time.monotonic() - t, 3)
        del board
        for _ in range(rounds):
            t = time.monotonic()
            board, times = check(path, refill, report + ".r")
            times["total"] = time.monotonic() - t
            row["rounds"].append({k: round(v, 3) for k, v in times.items()})
            del board
        row["rss_mb"] = rss_mb()
        row["peak_mb"] = peak_mb()
        counts = parse_text_report(report)
        row["findings"] = {f"{k}/{s}": n for (k, s), n in sorted(counts.items()) if n}
        again = parse_text_report(report + ".2")
        row["again_equal"] = again == counts
        if reference:
            cli = kicad_cli_counts(reference)
            keys = set(cli) | set(counts)
            row["differ_from_kicad_cli"] = {f"{k}/{s}": [cli.get((k, s), 0), counts.get((k, s), 0)]
                                            for (k, s) in sorted(keys) if cli.get((k, s), 0) != counts.get((k, s), 0)}
        print(json.dumps(row), flush=True)


if __name__ == "__main__":
    main()
