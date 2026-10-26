#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Per-board memory and time estimates for every tier, from measured rows.

run.py's memory gate reserves a board's expected peak in the machine's
ledger and starts the boards in an order by their expected peak or
seconds. Since 2026-10-26 those come from the sidecar estimates.json (next
to this file), which `--refresh` writes from run.py logs:

    # after a sweep (or any run whose rows should count), into the sidecar:
    benchmarks/agent-tasks/tier_estimates.py --refresh --layout build/github-layout-v22.log
    benchmarks/agent-tasks/tier_estimates.py --refresh --designer build/designer-x1900-matched.log
    # a night's stages (sweep, designer, short, expansion); scripts/nightly.sh:
    benchmarks/agent-tasks/tier_estimates.py --refresh --night build/nightly/2026-10-27 \\
        --estimates build/nightly/estimates.json --merge benchmarks/agent-tasks/estimates.json \\
        --output build/nightly/estimates.json
    # without any log: the newest night under build/nightly

Per board and kind (layout: the layout sweep and the quick tier; short,
schematic, designer, expansion: those tiers; README.md, "How to measure")
the sidecar keeps the rows of the newest WINDOW (3) sources that measured
the board, by the source's time: so the estimates follow the binary. The
estimate is the largest of those rows' `peak_rss_mb` times MARGIN (1.25),
and the largest of their `seconds`. Spliced rows are skipped (their own
log is the source). A board without a row of its kind falls back, in
run.py (`declared`), to its layout rows (short, schematic and designer:
the full layout is the larger run as a rule), to the tier file's own
entry, then to the size model: per family (layout: layout, short,
schematic; route: designer, expansion) a line over pads x copper layers
fitted to the boards' largest measured peaks, raised so that QUANTILE
(0.9) of them lie at or below it, times MARGIN; and without a size, to
run.py's `--expected-gb` (3 GB). `--expected-gb` given on the command
line overrides every estimate.

The older form sets a tier file's own estimates (`peak_rss_mb`,
`seconds`) from the rows of runs of that tier, the largest of a board's
rows, as for designer.json on 2026-10-26:

    benchmarks/agent-tasks/tier_estimates.py benchmarks/agent-tasks/designer.json \\
        --placement designer build/nightly/2026-10-24/designer.log ...

Until 2026-10-26 the designer tier's estimates were its boards' v17 layout
rows (x1170: placement and routing), an upper bound for a route on the
designer's placement that was 1.8 times the measured peak at the median
and 34 times on the ULK boards (23 215 MB expected, 674-1512 MB
measured): the nightly's last boards waited for 33-38 GB of available
memory they never used.
"""

import argparse
import datetime
import json
import math
import re
import sys
from pathlib import Path

sys.path.append(str(Path(__file__).resolve().parents[1]))
import harness_common  # noqa: E402

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DEFAULT_ESTIMATES = HERE / "estimates.json"
# The task lists whose boards the sidecar sizes (pads, copper layers).
TASK_LISTS = ("benchmarks/agent-tasks/tasks/v7/tasks.json", "benchmarks/agent-tasks/expansion-tasks.json")
KINDS = ("layout", "short", "schematic", "designer", "expansion")
# The size model's families: a layout places and routes, a route only routes.
FAMILY = {"layout": "layout", "short": "layout", "schematic": "layout", "designer": "route", "expansion": "route"}
# The measured kinds an estimate of a kind uses, in order.
FALLBACK = {"layout": ("layout",), "short": ("short", "layout"), "schematic": ("schematic", "layout"),
            "designer": ("designer", "layout"), "expansion": ("expansion",)}
# A night's stages and the kinds of their rows (the fingerprints, a route
# from v16's placements at a quarter of the budget, count for none).
NIGHT_STAGES = {"sweep": "layout", "designer": "designer", "short": "short", "expansion": "expansion"}
# The estimate is the largest measured peak of a board's newest WINDOW
# sources times MARGIN. Peaks vary between runs (the load, the thread
# count, the binary): the matched designer tier of x1832 and the 2026-10-25
# nightly's (x1817) differ by a factor of 1.16 at the median and by at most
# 1.25 on 86 % of their 106 common boards (the larger over the smaller);
# the gate's error is cheap one way (a board waits a little longer) and
# dear the other (the machine runs short).
MARGIN = 1.25
WINDOW = 3
QUANTILE = 0.9


# --- the older form: a tier file's own estimates ---------------------------------

def measured(logs, placement=None):
    """Per board name the largest `peak_rss_mb` and `seconds` of its rows
    in `logs` (JSON lines; other lines are skipped), of `placement` where
    given, and how many rows."""
    found = {}
    for log in logs:
        for line in Path(log).read_text().splitlines():
            if not line.startswith("{"):
                continue
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if not isinstance(row, dict) or "name" not in row or not row.get("peak_rss_mb"):
                continue
            if placement is not None and row.get("placement") != placement:
                continue
            entry = found.setdefault(row["name"], {"peak_rss_mb": 0, "seconds": 0.0, "rows": 0})
            entry["peak_rss_mb"] = max(entry["peak_rss_mb"], int(row["peak_rss_mb"]))
            entry["seconds"] = max(entry["seconds"], float(row.get("seconds") or 0.0))
            entry["rows"] += 1
    return found


def update(tier, found):
    """Sets the estimates of `tier`'s boards from `found` (measured);
    returns (name, old peak, new peak) of the boards that changed."""
    changed = []
    for name, entry in tier["boards"].items():
        if name not in found:
            continue
        old = entry.get("peak_rss_mb")
        entry["peak_rss_mb"] = found[name]["peak_rss_mb"]
        if found[name]["seconds"] > 0:
            entry["seconds"] = round(found[name]["seconds"], 1)
        if old != entry["peak_rss_mb"]:
            changed.append((name, old, entry["peak_rss_mb"]))
    return changed


# --- the sidecar --------------------------------------------------------------------

def board_size(path):
    """(pads, copper layers) of a .kicad_pcb, or None: every `(pad ` and
    the board's layer table's copper layers (signal, power, mixed,
    jumper)."""
    try:
        text = Path(path).read_text(errors="replace")
    except OSError:
        return None
    pads = len(re.findall(r"\(pad\s", text))
    start = text.find("(layers")
    if start < 0:
        return None
    depth = 0
    for end in range(start, len(text)):
        if text[end] == "(":
            depth += 1
        elif text[end] == ")":
            depth -= 1
            if depth == 0:
                break
    table = text[start:end]
    layers = sum(1 for name in re.findall(r'\(\s*\d+\s+"?([^\s"()]+)"?\s+(?:signal|power|mixed|jumper)\b', table)
                 if name.endswith(".Cu"))
    return (pads, layers) if layers else None


def kind_matches(row, kind):
    """Whether a row counts for an estimate of `kind`."""
    if kind == "designer":
        return row.get("placement") == "designer"
    if kind == "expansion":
        return row.get("mode") == "route" and row.get("placement") != "designer" and not row.get("route_from")
    if kind == "schematic":
        return row.get("mode") == "schematic"
    return row.get("mode", "layout") == "layout"


def log_rows(path, kind):
    """Per board name (peak MB, seconds) of the log's rows of `kind` with a
    peak, spliced rows left out (the largest where a board repeats)."""
    rows = {}
    for line in Path(path).read_text(errors="replace").splitlines():
        if not line.startswith("{"):
            continue
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if not isinstance(row, dict) or "name" not in row or not row.get("peak_rss_mb") or row.get("spliced"):
            continue
        if not kind_matches(row, kind):
            continue
        peak, seconds = int(row["peak_rss_mb"]), round(float(row.get("seconds") or 0.0), 1)
        old = rows.get(row["name"])
        rows[row["name"]] = (max(peak, old[0]), max(seconds, old[1])) if old else (peak, seconds)
    return rows


def source_label(path, root=ROOT):
    """A log's name in the sidecar: relative to the checkout (build/... for
    the shared build directory, wherever the path came from), else as
    given."""
    path = Path(path).absolute()
    for base, prefix in ((root, ""), ((root / "build").resolve(), "build/")):
        try:
            return prefix + str(path.relative_to(base))
        except ValueError:
            pass
        try:
            return prefix + str(path.resolve().relative_to(base))
        except ValueError:
            pass
    return str(path)


def source_time(path):
    """When a log was written (its modification time, to the minute)."""
    return datetime.datetime.fromtimestamp(Path(path).stat().st_mtime).strftime("%Y-%m-%dT%H:%M")


def night_sources(night):
    """(log, kind) of a night's stages that left a log."""
    return [(Path(night) / f"{stage}.log", kind) for stage, kind in NIGHT_STAGES.items()
            if (Path(night) / f"{stage}.log").is_file()]


def newest_night(nights):
    names = sorted(path.name for path in Path(nights).glob("[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]*")
                   if path.is_dir())
    return Path(nights) / names[-1] if names else None


def load(path):
    """The sidecar at `path`, or None when there is none."""
    try:
        return json.loads(Path(path).read_text())
    except FileNotFoundError:
        return None


def fit(points, quantile=QUANTILE):
    """The size model over (pads x layers, peak MB) points: the
    least-squares slope (at least 0), with the base raised so that
    `quantile` of the points lie at or below the line; None below three
    points."""
    if len(points) < 3:
        return None
    mean_x = sum(x for x, _ in points) / len(points)
    mean_y = sum(y for _, y in points) / len(points)
    spread = sum((x - mean_x) ** 2 for x, _ in points)
    slope = max(0.0, sum((x - mean_x) * (y - mean_y) for x, y in points) / spread) if spread else 0.0
    residuals = sorted(y - slope * x for x, y in points)
    base = residuals[max(0, math.ceil(quantile * len(points)) - 1)]
    return {"base_mb": round(base), "mb_per_pad_layer": round(slope, 4), "boards": len(points),
            "quantile": quantile}


def model_mb(model, pads, layers, margin=MARGIN):
    return math.ceil(margin * max(0.0, model["base_mb"] + model["mb_per_pad_layer"] * pads * layers))


def refresh(bases, new_sources, sizes=None, margin=MARGIN, window=WINDOW, now=None):
    """The sidecar from the rows of `bases` (earlier sidecars, the first
    one's description kept) and `new_sources` ((path, kind) of run.py
    logs; a source of the same name replaces a base's), keeping per board
    and kind the rows of the newest `window` sources. `sizes` gives (pads,
    layers) per board name for boards the bases do not size."""
    sources = {}
    rows = {}  # name -> kind -> label -> (peak, seconds)
    board_sizes = {}
    description = None
    for base in bases:
        if not base:
            continue
        description = description or base.get("description")
        for label, source in base.get("sources", {}).items():
            if label not in sources or source["date"] > sources[label]["date"]:
                sources[label] = dict(source)
        for name, board in base.get("boards", {}).items():
            if board.get("pads") is not None and name not in board_sizes:
                board_sizes[name] = (board["pads"], board["layers"])
            for kind in KINDS:
                for label, peak, seconds in (board.get(kind) or {}).get("rows", []):
                    rows.setdefault(name, {}).setdefault(kind, {})[label] = (peak, seconds)
    for path, kind in new_sources:
        label = source_label(path)
        sources[label] = {"kind": kind, "date": source_time(path)}
        for found in rows.values():
            found.get(kind, {}).pop(label, None)
        for name, measured_row in log_rows(path, kind).items():
            rows.setdefault(name, {}).setdefault(kind, {})[label] = measured_row
    for name, size in (sizes or {}).items():
        if size and name not in board_sizes:
            board_sizes[name] = size
    boards = {}
    for name in sorted(set(rows) | set(board_sizes), key=str.lower):
        board = {}
        if name in board_sizes:
            board["pads"], board["layers"] = board_sizes[name]
        for kind in KINDS:
            found = rows.get(name, {}).get(kind)
            if not found:
                continue
            kept = sorted(found, key=lambda label: (sources[label]["date"], label), reverse=True)[:window]
            peak = max(found[label][0] for label in kept)
            board[kind] = {"peak_rss_mb": math.ceil(peak * margin), "measured_mb": peak,
                           "seconds": max(found[label][1] for label in kept),
                           "rows": [[label, *found[label]] for label in kept]}
        boards[name] = board
    used = {row[0] for board in boards.values() for kind in KINDS for row in (board.get(kind) or {}).get("rows", [])}
    models = {}
    for family in ("layout", "route"):
        points = []
        for board in boards.values():
            peaks = [board[kind]["measured_mb"] for kind in KINDS if FAMILY[kind] == family and kind in board]
            if peaks and board.get("pads") is not None:
                points.append((board["pads"] * board["layers"], max(peaks)))
        model = fit(points)
        if model:
            models[family] = model
    return {"description": description or DESCRIPTION,
            "refreshed": (now or datetime.datetime.now()).strftime("%Y-%m-%dT%H:%M"),
            "margin": margin, "window": window, "quantile": QUANTILE, "model": models,
            "sources": {label: sources[label] for label in sorted(used, key=lambda label: sources[label]["date"])},
            "boards": boards}


DESCRIPTION = ("Per-board estimates for run.py's memory gate and its order (tier_estimates.py --refresh writes "
               "this file from run.py logs; README.md, 'How to measure'). Per board: pads and copper layers, "
               "and per kind (layout: the sweep and the quick tier; short, schematic, designer, expansion: "
               "those tiers) peak_rss_mb (the largest measured peak of the rows times margin), measured_mb, "
               "seconds (the largest) and the rows ([source, peak MB, seconds]) of the newest `window` sources "
               "that measured the board; sources: each log's kind and time. model: per family (layout, route) "
               "the size model over pads x layers for boards without a row (fit to the largest measured "
               "peaks, raised to the quantile; times margin).")


def dumps(sidecar):
    """The sidecar as JSON with one line a board and a source (diffs
    show which boards' estimates changed)."""
    lines = ["{"]
    for key in ("description", "refreshed", "margin", "window", "quantile", "model"):
        lines.append(f"{json.dumps(key)}: {json.dumps(sidecar[key])},")
    for key in ("sources", "boards"):
        items = list(sidecar[key].items())
        lines.append(f"{json.dumps(key)}: {{")
        lines.extend(f" {json.dumps(name)}: {json.dumps(value)}" + ("," if index < len(items) - 1 else "")
                     for index, (name, value) in enumerate(items))
        lines.append("}," if key == "sources" else "}")
    lines.append("}")
    return "\n".join(lines) + "\n"


def task_sizes(task_lists, root=ROOT, known=()):
    """(pads, layers) per board of the task lists not in `known`, from the
    board files that are there."""
    sys.path.insert(0, str(HERE))
    import run  # the task's board directory, as run.py finds it
    sizes = {}
    for task_list in task_lists:
        path = root / task_list
        if not path.is_file():
            continue
        for task in json.loads(path.read_text())["tasks"]:
            if task["name"] in known or task["name"] in sizes:
                continue
            size = board_size(run.task_directory(task) / f"{task['board_id']}.kicad_pcb")
            if size:
                sizes[task["name"]] = size
    return sizes


def declared(sidecar, name, kind, tier_entry=None, size=None):
    """What run.py declares for board `name` in a run of `kind`: (peak MB,
    seconds, where from), or (None, seconds, None) when nothing estimates
    it (run.py then declares --expected-gb). In order: the board's
    measured rows of `kind` and its fallbacks (FALLBACK), the tier file's
    own entry, the size model of the kind's family on the board's size
    (the sidecar's, else `size`)."""
    board = ((sidecar or {}).get("boards") or {}).get(name) or {}
    tier_seconds = (tier_entry or {}).get("seconds")
    for measured_kind in FALLBACK[kind]:
        entry = board.get(measured_kind)
        if entry:
            where = f"{measured_kind} rows"
            return entry["peak_rss_mb"], entry.get("seconds") or tier_seconds, where
    if tier_entry and tier_entry.get("peak_rss_mb"):
        return tier_entry["peak_rss_mb"], tier_seconds, "the tier's entry"
    model = ((sidecar or {}).get("model") or {}).get(FAMILY[kind])
    if board.get("pads") is not None:
        size = (board["pads"], board["layers"])
    if model and size:
        return model_mb(model, *size, (sidecar or {}).get("margin", MARGIN)), tier_seconds, "size model"
    return None, tier_seconds, None


def coverage(sidecar):
    """Per kind the boards with a measured row; the boards with none."""
    counts = {kind: sum(1 for board in sidecar["boards"].values() if kind in board) for kind in KINDS}
    unmeasured = sorted(name for name, board in sidecar["boards"].items() if not any(kind in board for kind in KINDS))
    return counts, unmeasured


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0],
                                     formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__)
    parser.add_argument("tier", type=Path, nargs="?", help="the older form: the tier file (designer.json), rewritten "
                        "in place")
    parser.add_argument("logs", type=Path, nargs="*", help="the older form: run.py logs of the tier")
    parser.add_argument("--placement", help="the older form: only rows of this placement (designer)")
    parser.add_argument("--refresh", action="store_true", help="write the sidecar from its rows and these logs")
    parser.add_argument("--estimates", type=Path, default=DEFAULT_ESTIMATES,
                        help="the sidecar to start from (default %(default)s; absent: none)")
    parser.add_argument("--merge", type=Path, nargs="*", default=[],
                        help="more sidecars whose rows count (the nightly's own with the committed one)")
    parser.add_argument("--output", type=Path, default=None, help="where to write it (default: --estimates)")
    parser.add_argument("--night", type=Path, nargs="*", default=[],
                        help="nights (build/nightly/<date>): their stages' logs, sweep as layout")
    for kind in KINDS:
        parser.add_argument(f"--{kind}", type=Path, nargs="*", default=[], help=f"logs of {kind} rows")
    arguments = parser.parse_args()
    if not arguments.refresh:
        if arguments.tier is None or not arguments.logs:
            parser.error("give a tier file and its logs, or --refresh")
        tier = json.loads(arguments.tier.read_text())
        found = measured(arguments.logs, arguments.placement)
        changed = update(tier, found)
        harness_common.atomic_json(arguments.tier, tier, indent=1, newline=True)
        missing = sorted(set(tier["boards"]) - set(found))
        print(f"{len(found)} boards with rows, {len(changed)} estimates changed, {len(missing)} without a row")
        for name, old, new in sorted(changed, key=lambda c: -abs((c[1] or 0) - c[2]))[:10]:
            print(f"  {name}: {old} -> {new} MB")
        return
    if arguments.tier is not None:
        parser.error("--refresh takes its logs by kind (--layout, --designer, ...) or --night")
    sources = [(path, kind) for kind in KINDS for path in getattr(arguments, kind)]
    for night in arguments.night:
        sources += night_sources(night)
    if not sources:
        night = newest_night(ROOT / "build/nightly")
        if night is None:
            parser.error("no logs given and no night under build/nightly")
        sources = night_sources(night)
        print(f"the newest night: {night}")
    for path, _ in sources:
        if not Path(path).is_file():
            parser.error(f"{path} not found")
    output = arguments.output or arguments.estimates
    old = load(arguments.estimates)
    bases = [old] + [load(path) for path in arguments.merge]
    known = {name for base in bases if base for name, board in base.get("boards", {}).items()
             if board.get("pads") is not None}
    sidecar = refresh(bases, sources, task_sizes(TASK_LISTS, known=known))
    harness_common.atomic_text(output, dumps(sidecar))
    counts, unmeasured = coverage(sidecar)
    print(f"{output}: {len(sidecar['boards'])} boards; with a measured row: "
          + ", ".join(f"{kind} {count}" for kind, count in counts.items())
          + f"; with none (the size model or --expected-gb): {len(unmeasured)}")
    for family, model in sidecar["model"].items():
        print(f"  {family} size model: {model['base_mb']} MB + {model['mb_per_pad_layer']} MB a pad and layer "
              f"(over {model['boards']} boards), times {sidecar['margin']}")
    changes = []
    for name, board in sidecar["boards"].items():
        for kind in KINDS:
            before = (((old or {}).get("boards") or {}).get(name) or {}).get(kind, {}).get("peak_rss_mb")
            after = board.get(kind, {}).get("peak_rss_mb")
            if after is not None and before != after:
                changes.append((name, kind, before, after))
    print(f"  {len(changes)} estimates changed")
    for name, kind, before, after in sorted(changes, key=lambda c: -abs((c[2] or 0) - c[3]))[:10]:
        print(f"    {name} ({kind}): {before} -> {after} MB")


if __name__ == "__main__":
    main()
