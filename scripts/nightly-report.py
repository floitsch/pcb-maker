#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""The nightly's summary and its write-up into the docs (scripts/nightly.sh;
benchmarks/agent-tasks/README.md, "Nightly"; docs/benchmarks.md, "Nightly").

    scripts/nightly-report.py --docs <night>      # publish a night into this checkout's docs
    scripts/nightly-report.py --docs <night> --dry-run   # print its section, write nothing
    scripts/nightly-report.py --docs <night> --checkout <dir>   # into another checkout's docs
    scripts/nightly-report.py --print             # print the newest night's table row and state line
    scripts/nightly-report.py --commit            # nightly.sh's last step: into main, committed, pushed
    scripts/nightly-report.py --summarize <night> --nights build/nightly ...   # nightly.sh's summary
    scripts/nightly-report.py --ladder-share <sweep.log> --default 0.695      # the designer tier's scale

`--summarize` reads a night's stages (`<night>/<stage>.log`, the rows, and
`<night>/<stage>/summary.json`, run.py's summary) and writes
`<night>/nightly.json` and `<night>/summary.md`: pass and strict counts,
the medians against the designer's boards, the gap split (compare.py --gap
--targets, also `gap.txt`), the cut and killed rows, the fingerprints, and
the diff against the previous nightly that ran each stage (compare.py,
also `diff-<stage>.txt`: better, same, worse, and the boards whose pass
status changed). Its last line is the guard's verdict.

`--docs <night>` publishes a night the way the hand-written sweep
sections v19 to v21 did, into four files:

- docs/benchmarks.md: a section "Layout sweep vN (<date>)" (vN: the
  night's own number when it has a section, else one more than the
  highest numbered sweep; a marker comment names the night, its sweep log
  and main) after the newest numbered sweep: the totals table against
  the previous night that ran the sweep and the previous numbered sweep
  (its log: the marker's, else build/github-layout-vN.log), better /
  same / worse and the strict passes gained and lost with the rows'
  findings, the INPUT boards (compare.on_other_input) named and left out,
  the rows without a result, the strict errors, the matched designer
  tier's gap split over the target boards, the five costliest
  placements, the losers beyond the seed spreads of dense-noise.json;
  and the night's row of the "Nightly" table (newest first, 14 nights);
- README.md: the "Boards harvested from GitHub" bullet from "pcb-maker
  places and routes" on, and the two rows of the newest nightly above
  the table's first layout row (an earlier nightly's two rows replaced);
- docs/handover.md: the title's state, the nightly state line, the
  night note after the last "**Layout sweep vN**" paragraph (an earlier
  nightly's note replaced);
- docs/changelog.md: "Measured on <date> (nightly: layout sweep vN,
  matched designer tier)", above the first section of that date or
  older.

Writing a night again replaces its own parts. The guard (guard()): only
a night on which every stage ran, no row of any stage was cut or killed
and the sweep has at least 100 rows with a result is published; else
nothing is written and the reason is printed.

`--commit` publishes the nights after the newest one docs/benchmarks.md
has (the newest night alone when it has none; at most three, oldest
first) into the main checkout and commits the four files alone on main
("nightly <date>: layout sweep vN, ...") and pushes. Only when the
checkout is on main, its working tree has no change to a tracked file
and `git pull --ff-only origin main` succeeds; otherwise it writes
nothing and the next night publishes this one with its own."""

import argparse
import hashlib
import json
import re
import subprocess
import sys
import textwrap
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
STAGES = ("fingerprints", "expansion", "short", "sweep", "designer")
KEEP_NIGHTS = 14
TABLE_BEGIN, TABLE_END = "<!-- nightly:begin -->", "<!-- nightly:end -->"
STATE_BEGIN, STATE_END = "<!-- nightly-state:begin -->", "<!-- nightly-state:end -->"
HEADER = ("| Night | main | Fingerprints | Expansion | Short | Sweep | Designer | Gap (placement / routing) "
          "| Against the previous night (better/same/worse) |")
NIGHT_NAME = re.compile(r"^\d{4}-\d{2}-\d{2}")
FINGERPRINT = re.compile(r"fingerprint ([^:]+): ([0-9a-f]+)")
BOTH = re.compile(r"(\d+) boards in both: better (\d+), same (\d+), worse (\d+)")


def load_compare(harness=REPO):
    sys.path.insert(0, str(Path(harness) / "benchmarks/agent-tasks"))
    import compare
    return compare


# --- reading run.py's results ------------------------------------------------------


def stage_summary(path):
    """The numbers the summary and the table take from a run.py
    summary.json (`path`: the file or its output directory); None when
    there is none. Older summaries without the strict rule or the
    normalised measures give None for those."""
    path = Path(path)
    if path.is_dir():
        path = path / "summary.json"
    try:
        summary = json.loads(path.read_text())
    except (OSError, ValueError):
        return None
    normalised = summary.get("normalised") or {}
    strict = summary.get("passes_strict") if summary.get("strict_rows") else None
    return {"rows": summary.get("rows"), "passes": summary.get("passes"), "passes_strict": strict,
            "strict_rule": summary.get("strict_rule"), "unconnected": summary.get("unconnected"),
            "new_errors": summary.get("new_errors"), "new_errors_strict": summary.get("new_errors_strict"),
            "failed": summary.get("failed"), "cut": summary.get("cut"), "vias": summary.get("vias"),
            "length_mm": summary.get("length_mm"), "work_seconds": summary.get("work_seconds"),
            "wall_seconds": summary.get("wall_seconds"), "budget_scale": summary.get("budget_scale"),
            "jobs": summary.get("jobs"), "threads": summary.get("threads"),
            "vias_ratio": normalised.get("vias_ratio"), "length_ratio": normalised.get("length_ratio"),
            "open_share": normalised.get("open_share"), "open_share_mean": normalised.get("open_share_mean"),
            "ground_cut_delta_mm": normalised.get("ground_cut_delta_mm")}


def median(values):
    ordered = sorted(values)
    if not ordered:
        return None
    middle = len(ordered) // 2
    return ordered[middle] if len(ordered) % 2 else (ordered[middle - 1] + ordered[middle]) / 2


def ladder_share(log, compare):
    """The median `ladder_budget_share` of a layout log's rows (the
    designer tier's matched budget scale), None without one."""
    if not Path(log).is_file():
        return None
    shares = [row["ladder_budget_share"] for row in compare.by_name(compare.rows_of(log)).values()
              if row.get("ladder_budget_share") is not None]
    value = median(shares)
    return None if value is None else round(value, 3)


def fingerprints_of(directory):
    """{board: {"fingerprints": sorted "phase: hash" lines of its
    route.log, "board_sha256": the routed board's}} of a fingerprint
    stage's output directory. Sorted: attempts that run side by side log
    in any order (docs/handover.md, PCB_ROUTER_FINGERPRINT)."""
    boards = {}
    for log in sorted(Path(directory).glob("*/route.log")):
        lines = [f"{phase}: {value}" for phase, value in FINGERPRINT.findall(log.read_text(errors="replace"))]
        routed = sorted((log.parent / "route").glob("*.kicad_pcb"))
        boards[log.parent.name] = {
            "fingerprints": sorted(lines),
            "board_sha256": hashlib.sha256(routed[0].read_bytes()).hexdigest() if routed else None}
    return boards


def fingerprint_changes(current, previous):
    """Per board of `current`: "identical", "new" (no previous), or what
    changed: the phases whose hashes differ, the board. The lines compare
    as a multiset: rungs side by side print theirs as they finish, so a
    pure reordering is identical. A line names its phase, not its rung,
    so the phase is all a change can name."""
    changes = {}
    for board, now in current.items():
        before = (previous or {}).get(board)
        if before is None:
            changes[board] = "new"
            continue
        missing = Counter(now["fingerprints"]) - Counter(before["fingerprints"])
        extra = Counter(before["fingerprints"]) - Counter(now["fingerprints"])
        parts = []
        if missing or extra:
            phases = sorted({line.split(":")[0] for line in list(missing) + list(extra)})
            parts.append("fingerprints changed (" + ", ".join(phases) + ")")
        if now["board_sha256"] != before["board_sha256"]:
            parts.append("routed board changed")
        changes[board] = "; ".join(parts) or "identical"
    return changes


def short_board(name):
    """A board's name without its owner (and without the repeated part:
    jetson-nano-baseboard__jetson-nano-baseboard)."""
    parts = name.split("__")[1:] or [name]
    return "__".join(parts[-1:] if len(set(parts)) == 1 else parts)


# --- the summary of one night ----------------------------------------------------


def night_names(nights):
    return sorted(path.name for path in Path(nights).iterdir()
                  if path.is_dir() and NIGHT_NAME.match(path.name)) if Path(nights).is_dir() else []


def read_night(directory):
    try:
        return json.loads((Path(directory) / "nightly.json").read_text())
    except (OSError, ValueError):
        return None


def previous_night(nights, night, stage):
    """The newest night before `night` that ran `stage` with its log
    still there: (name, its nightly.json) or (None, None)."""
    for name in reversed(night_names(nights)):
        if name >= night:
            continue
        record = read_night(Path(nights) / name)
        if record and (record.get("stages", {}).get(stage) or {}).get("status") == "ran" \
                and (Path(nights) / name / f"{stage}.log").is_file():
            return name, record
    return None, None


def diff_stage(compare, harness, before_log, after_log, output):
    """compare.py before after into `output`; its better / same / worse
    line and the boards whose pass status changed."""
    result = subprocess.run([sys.executable, str(Path(harness) / "benchmarks/agent-tasks/compare.py"),
                             str(before_log), str(after_log)], capture_output=True, text=True)
    output.write_text(result.stdout + result.stderr)
    match = BOTH.search(result.stdout)
    before = compare.by_name(compare.rows_of(before_log))
    after = compare.by_name(compare.rows_of(after_log))
    common = sorted(set(before) & set(after))
    diff = {"line": next((line for line in result.stdout.splitlines() if BOTH.search(line)), None),
            "both": None, "better": None, "same": None, "worse": None,
            "gained": [n for n in common if compare.passes(after[n]) and not compare.passes(before[n])],
            "lost": [n for n in common if compare.passes(before[n]) and not compare.passes(after[n])],
            "strict_gained": [n for n in common if compare.passes_strict(after[n])
                              and not compare.passes_strict(before[n])],
            "strict_lost": [n for n in common if compare.passes_strict(before[n])
                            and not compare.passes_strict(after[n])]}
    if match:
        diff.update(zip(("both", "better", "same", "worse"), map(int, match.groups())))
    return diff


def gap(compare, harness, sweep_log, designer_log, output):
    """compare.py --gap --targets into `output`, and its totals."""
    result = subprocess.run([sys.executable, str(Path(harness) / "benchmarks/agent-tasks/compare.py"), "--gap",
                             str(sweep_log), str(designer_log), "--targets"], capture_output=True, text=True)
    output.write_text(result.stdout + result.stderr)
    layout = {n: r for n, r in compare.by_name(compare.rows_of(sweep_log)).items() if n not in compare.NOT_TARGETS}
    designer = {n: r for n, r in compare.by_name(compare.rows_of(designer_log)).items()
                if n not in compare.NOT_TARGETS}
    totals = compare.gap_totals(compare.gap_rows(layout, designer))
    record = {key: totals[key] for key in ("boards", "both", "layout_open", "designer_open", "placement_gap",
                                            "routing_gap", "cut")}
    record["placement_share"] = round(totals["placement_gap"] / totals["layout_open"], 3) \
        if totals["layout_open"] else None
    record["routing_share"] = round(totals["routing_gap"] / totals["layout_open"], 3) \
        if totals["layout_open"] else None
    record["lines"] = [line for line in result.stdout.splitlines()
                       if line.startswith(("budgets:", "share of", "BINARIES")) or " boards with both results" in line]
    return record


def summarize(directory, nights, sha, binary, harness, planned, statuses, seconds):
    compare = load_compare(harness)
    directory = Path(directory)
    night = directory.name
    exits = dict(status.split("=", 1) for status in statuses)
    record = {"night": night, "sha": sha, "binary": str(binary), "seconds": seconds,
              "binary_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest() if Path(binary).is_file()
              else None, "stages": {}}
    for stage in STAGES:
        log = directory / f"{stage}.log"
        if stage not in planned:
            record["stages"][stage] = {"status": "skipped"}
            continue
        rows = compare.by_name(compare.rows_of(log)) if log.is_file() else {}
        summary = stage_summary(directory / stage)
        entry = {"status": "ran" if rows and summary else "failed",
                 "exit": int(exits[stage]) if stage in exits else None, "summary": summary,
                 "cut": sorted(n for n, r in rows.items() if compare.cut(r) and not compare.killed(r)),
                 "killed": sorted(n for n, r in rows.items() if compare.killed(r)),
                 "errors": sorted(n for n, r in rows.items() if r.get("error") and not compare.killed(r)
                                  and not compare.cut(r))}
        if entry["status"] == "ran":
            name, _ = previous_night(nights, night, stage)
            if name:
                entry["previous"] = name
                entry["diff"] = diff_stage(compare, harness, Path(nights) / name / f"{stage}.log", log,
                                           directory / f"diff-{stage}.txt")
        record["stages"][stage] = entry
    if record["stages"]["fingerprints"].get("status") == "ran":
        boards = fingerprints_of(directory / "fingerprints")
        name, previous = previous_night(nights, night, "fingerprints")
        record["fingerprints"] = {"boards": boards, "previous": name,
                                  "changes": fingerprint_changes(boards, (previous or {}).get("fingerprints", {})
                                                                 .get("boards") if previous else None)}
    if all(record["stages"][stage].get("status") == "ran" for stage in ("sweep", "designer")):
        record["gap"] = gap(compare, harness, directory / "sweep.log", directory / "designer.log",
                            directory / "gap.txt")
    (directory / "nightly.json").write_text(json.dumps(record, indent=1) + "\n")
    (directory / "summary.md").write_text(summary_markdown(record))
    return record


def ratio(value):
    return "-" if value is None else f"x{value:.2f}"


def summary_markdown(record):
    stages = record["stages"]
    ran = [s for s in STAGES if stages[s].get("status") == "ran"]
    skipped = [s for s in STAGES if stages[s].get("status") == "skipped"]
    failed = [s for s in STAGES if stages[s].get("status") == "failed"]
    lines = [f"# Nightly {record['night']}", "",
             f"main `{record['sha'][:12]}` ({record['sha']}), binary `{record['binary']}` (sha256 "
             f"{(record.get('binary_sha256') or '?')[:12]}), {record['seconds']} s in all.", "",
             "Ran: " + (", ".join(ran) or "nothing") + ". Skipped: " + (", ".join(skipped) or "nothing") + "."
             + (" FAILED: " + ", ".join(f"{s} (exit {stages[s].get('exit')}, {s}.err)" for s in failed) + "."
                if failed else ""), "",
             "## Results", "",
             "| Stage | Pass | Strict | Rows | Unconnected | New errors (strict) | Vias / designer "
             "| Length / designer | Open share (median, mean) | Cut | Killed | Budget scale | Wall s |",
             "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for stage in ran:
        s, entry = stages[stage]["summary"], stages[stage]
        share = "-" if s["open_share"] is None else f"{s['open_share']:.3f}, {s['open_share_mean']:.3f}"
        lines.append(f"| {stage} | {s['passes']} | {'-' if s['passes_strict'] is None else s['passes_strict']} "
                     f"| {s['rows']} | {s['unconnected']} | {s['new_errors']} ({s['new_errors_strict']}) "
                     f"| {ratio(s['vias_ratio'])} | {ratio(s['length_ratio'])} | {share} | {len(entry['cut'])} "
                     f"| {len(entry['killed'])} | {s['budget_scale']:g} | {s['wall_seconds']} |")
    fingerprints = record.get("fingerprints")
    if fingerprints:
        lines += ["", "## Fingerprints", "",
                  "Route mode from build/github-layout-v16, 1 x 4, budget scale 0.25, "
                  "`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`; against "
                  + (fingerprints["previous"] or "no earlier nightly") + ".", "",
                  "| Board | Phases | Routed board | Against the previous night |", "| --- | ---: | --- | --- |"]
        for board, entry in sorted(fingerprints["boards"].items()):
            lines.append(f"| {short_board(board)} | {len(entry['fingerprints'])} "
                         f"| {(entry['board_sha256'] or '-')[:12]} | {fingerprints['changes'].get(board, '-')} |")
    if record.get("gap"):
        lines += ["", "## Placement or routing (compare.py --gap --targets, gap.txt)", "", "```"]
        lines += record["gap"]["lines"] + ["```"]
    lines += ["", "## Cut and killed rows", ""]
    listed = False
    for stage in ran + failed:
        entry = stages[stage]
        for kind in ("killed", "cut", "errors"):
            if entry.get(kind):
                listed = True
                label = {"killed": "killed", "cut": "cut by the wall clock", "errors": "without a result"}[kind]
                lines.append(f"- {stage}, {label}: " + ", ".join(entry[kind]))
    if not listed:
        lines.append("None.")
    lines += ["", "## Against the previous nightly", ""]
    for stage in ran:
        entry = stages[stage]
        diff = entry.get("diff")
        if not diff:
            lines.append(f"- {stage}: no earlier nightly ran it.")
            continue
        text = f"- {stage} against {entry['previous']} (diff-{stage}.txt): {diff['line'] or 'no comparison line'}"
        changes = []
        for key, label in (("gained", "gained the pass"), ("lost", "lost the pass"),
                           ("strict_gained", "gained the strict pass"), ("strict_lost", "lost the strict pass")):
            if diff[key]:
                changes.append(f"{label}: " + ", ".join(short_board(n) for n in diff[key]))
        lines.append(text + ("; " + "; ".join(changes) if changes else "; no board changed pass status"))
    if not ran:
        lines.append("Nothing ran.")
    return "\n".join(lines) + "\n"


# --- the report into the docs ------------------------------------------------------


def run_cell(entry):
    if not entry or entry.get("status") == "skipped":
        return "-"
    if entry.get("status") != "ran":
        return f"FAILED (exit {entry.get('exit')})"
    s = entry["summary"]
    text = f"{s['passes']}/{s['rows']}"
    if s["passes_strict"] is not None:
        text += f" (strict {s['passes_strict']})"
    text += f", {s['unconnected']} open"
    if entry["cut"]:
        text += f", {len(entry['cut'])} cut"
    if entry["killed"]:
        text += f", {len(entry['killed'])} KILLED"
    return text


def fingerprint_cell(record):
    entry = record["stages"].get("fingerprints") or {}
    if entry.get("status") != "ran":
        return run_cell(entry)
    fingerprints = record.get("fingerprints") or {}
    if not fingerprints.get("previous"):
        return "first"
    changed = [short_board(b) for b, change in sorted(fingerprints.get("changes", {}).items())
               if change != "identical"]
    return "identical" if not changed else "changed: " + ", ".join(changed)


def previous_cell(record):
    parts = []
    for stage in STAGES:
        diff = (record["stages"].get(stage) or {}).get("diff")
        if not diff or diff.get("better") is None:
            continue
        text = f"{stage} {diff['better']}/{diff['same']}/{diff['worse']}"
        if diff["gained"] or diff["lost"]:
            text += f" (pass +{len(diff['gained'])} -{len(diff['lost'])})"
        parts.append(text)
    return "; ".join(parts) if parts else "-"


def gap_cell(record):
    g = record.get("gap")
    if not g or g.get("placement_share") is None:
        return "-"
    return f"{g['placement_share']:.0%} / {g['routing_share']:.0%}"


def table_row(record):
    designer = run_cell(record["stages"].get("designer"))
    scale = ((record["stages"].get("designer") or {}).get("summary") or {}).get("budget_scale")
    if scale is not None and designer != "-":
        designer += f" at {scale:g}"
    cells = [record["night"], f"`{record['sha'][:12]}`", fingerprint_cell(record),
             run_cell(record["stages"].get("expansion")), run_cell(record["stages"].get("short")),
             run_cell(record["stages"].get("sweep")), designer, gap_cell(record), previous_cell(record)]
    return "| " + " | ".join(cells) + " |"


def state_line(record):
    stages = record["stages"]
    parts = [f"fingerprints {fingerprint_cell(record)}"]
    for stage in ("expansion", "short", "sweep", "designer"):
        cell = run_cell(stages.get(stage))
        scale = ((stages.get(stage) or {}).get("summary") or {}).get("budget_scale")
        if stage == "designer" and scale is not None and cell != "-":
            cell += f" at {scale:g}"
        if cell != "-":
            parts.append(f"{stage} {cell}")
    if gap_cell(record) != "-":
        parts.append(f"gap placement {gap_cell(record).replace(' / ', ', routing ')}")
    return (f"**Nightly** {record['night']} (main {record['sha'][:12]}, "
            f"`build/nightly/{record['night']}/summary.md`): " + "; ".join(parts) + ".")


def merge_table(text, row, keep=KEEP_NIGHTS):
    """`text` (docs/benchmarks.md) with `row` in the table between the
    markers: replacing the row of its night, newest first, at most `keep`
    rows."""
    if TABLE_BEGIN not in text or TABLE_END not in text:
        raise ValueError(f"docs/benchmarks.md has no {TABLE_BEGIN} ... {TABLE_END} block")
    head, rest = text.split(TABLE_BEGIN, 1)
    body, tail = rest.split(TABLE_END, 1)
    rows = {}
    for line in body.splitlines():
        night = line.split("|")[1].strip() if line.startswith("|") and line.count("|") > 1 else ""
        if NIGHT_NAME.match(night):
            rows[night] = line
    rows[row.split("|")[1].strip()] = row
    kept = [rows[night] for night in sorted(rows, reverse=True)[:keep]]
    separator = "|" + " --- |" * (HEADER.count("|") - 1)
    return head + TABLE_BEGIN + "\n" + "\n".join([HEADER, separator] + kept) + "\n" + TABLE_END + tail


def merge_state(text, line):
    if STATE_BEGIN not in text or STATE_END not in text:
        raise ValueError(f"docs/handover.md has no {STATE_BEGIN} ... {STATE_END} block")
    head, rest = text.split(STATE_BEGIN, 1)
    _, tail = rest.split(STATE_END, 1)
    return head + STATE_BEGIN + "\n" + line + "\n" + STATE_END + tail


def newest(nights):
    for name in reversed(night_names(nights)):
        record = read_night(Path(nights) / name)
        if record:
            return record
    return None


def git(checkout, *arguments, check=True):
    return subprocess.run(["git", "-C", str(checkout), *arguments], capture_output=True, text=True, check=check)


def main_checkout():
    listing = git(REPO, "worktree", "list", "--porcelain").stdout
    return Path(next(line[len("worktree "):] for line in listing.splitlines() if line.startswith("worktree ")))



# --- the published write-up: --docs and --commit ------------------------------------

# The guard: a night is published only when every stage ran, no row of any
# stage was cut by the wall clock or killed, and the sweep has at least
# this many rows with a result.
MIN_SWEEP_RESULTS = 100
# The files a published night changes, relative to the checkout.
DOC_FILES = ("docs/benchmarks.md", "README.md", "docs/handover.md", "docs/changelog.md")
SWEEP_HEADING = re.compile(r"^## Layout sweep v(\d+) \(([^)\n]*)\)[^\n]*$", re.M)
SWEEP_MARKER = re.compile(r"<!-- nightly-sweep night=(\S+) sweep=(\S+) main=(\S+) -->")
CHANGELOG_HEADING = re.compile(r"^## [A-Z][a-z]+ on (\d{4}-\d{2}-\d{2}) \(", re.M)
README_BULLET = "- **Boards harvested from GitHub**"
README_SPLIT = "pcb-maker places and routes"
README_LAYOUT_ROW = "| Layout from stacked parts and constraints"
README_DESIGNER_ROW = "| Routing the designer's placement at the layout's routing budget"
HANDOVER_TITLE = "# Hand-over ("
HANDOVER_NOTE = "**Layout sweep v"
NOISE = REPO / "benchmarks/agent-tasks/dense-noise.json"
WIDTH = 72
# Board names that say little alone: name_of puts the repository's part
# before them.
GENERIC_NAMES = ("main", "module", "mainboard", "motherboard", "power", "keyboard")


def guard(record, min_results=MIN_SWEEP_RESULTS):
    """Why a night is not published (empty: it is): a stage that did not
    run (skipped or failed), rows cut by the wall clock or killed in any
    stage, a sweep with fewer than `min_results` rows with a result."""
    reasons = []
    stages = (record or {}).get("stages") or {}
    for stage in STAGES:
        entry = stages.get(stage) or {}
        status = entry.get("status", "missing")
        if status != "ran":
            exit_code = entry.get("exit")
            reasons.append(f"{stage} {status}" + (f" (exit {exit_code})" if exit_code not in (None, 0) else ""))
        for kind, label in (("cut", "cut by the wall clock"), ("killed", "killed")):
            if entry.get(kind):
                reasons.append(f"{stage}: {len(entry[kind])} rows {label} ("
                               + ", ".join(short_board(n) for n in entry[kind][:4])
                               + (", ..." if len(entry[kind]) > 4 else "") + ")")
    sweep = stages.get("sweep") or {}
    if sweep.get("status") == "ran":
        summary = sweep.get("summary") or {}
        results = (summary.get("rows") or 0) - (summary.get("failed") or 0)
        if results < min_results:
            reasons.append(f"sweep: {results} rows with a result, fewer than {min_results}")
    return reasons


def guard_line(record, min_results=MIN_SWEEP_RESULTS):
    reasons = guard(record, min_results)
    if reasons:
        return f"not published: {'; '.join(reasons)} (the night is summarised only)"
    return ("publishable: every stage ran, no row cut or killed, the sweep has at least "
            f"{min_results} rows with a result")


def name_of(name):
    """A board's name for the prose: the last part of its task name, with
    the part before it where the last is short or generic (pcb, main,
    4in1)."""
    parts = name.split("__")
    last = parts[-1]
    generic = len(last) <= 4 or last.lower() in GENERIC_NAMES or "panel" in last or last.lower().startswith("rev")
    if len(parts) > 1 and parts[-2].lower() not in last.lower() and generic:
        return f"{parts[-2]} {last}"
    return last


# A wrapped line must not begin like a list item, a heading or a quote.
BLOCK_START = re.compile(r"^\s*(?:[-+*] |\d+[.)] |#+ |>)")


def wrap(text, initial="", subsequent=""):
    """Markdown prose wrapped at WIDTH: a number stays with its " %", a
    " - " dash with the word before it, and a line that would begin like
    a list item, a heading or a quote joins the line before."""
    text = text.replace(" %", "\u00a0%").replace(" - ", "\u00a0- ")
    lines = textwrap.fill(text, width=WIDTH, initial_indent=initial, subsequent_indent=subsequent,
                          break_long_words=False, break_on_hyphens=False).split("\n")
    joined = []
    for line in lines:
        if joined and BLOCK_START.match(line[len(subsequent):] if line.startswith(subsequent) else line):
            joined[-1] += " " + line.strip()
        else:
            joined.append(line)
    return "\n".join(joined).replace("\u00a0", " ")


def capital(text):
    return text[:1].upper() + text[1:]


def scientific(value):
    """1.1e6, not 1.1e+06."""
    mantissa, exponent = f"{value:.1e}".split("e")
    return f"{mantissa}e{int(exponent)}"


def plural(count, word, words=None):
    return f"{count} {word if count == 1 else (words or word + 's')}"


def listing(items, limit=10):
    items = list(items)
    if not items:
        return "none"
    text = ", ".join(items[:limit])
    return text + (f" and {len(items) - limit} more" if len(items) > limit else "")


def load_run(compare, log, label, kind):
    """A run for the write-up: its rows by board, its output directory
    (compare.py's INPUT test reads prepared sources there), a label for
    the prose and the table."""
    log = Path(log)
    return {"label": label, "kind": kind, "log": log, "rows": compare.by_name(compare.rows_of(log)),
            "directory": compare.output_directory(log)}


def no_result_text(compare, row):
    if compare.killed(row):
        return "killed"
    error = str(row.get("error") or "no result")
    error = re.sub(r"\s*\(.*$", "", error.splitlines()[0])
    error = re.sub(r"^incomplete: ", "", error)
    return error[:80]


def verdict_text(compare, row):
    """Why a row is not a strict pass, from its own fields: open
    connections and the strict findings by type ("4 open, clearance 3"),
    or why it has no result; "none" for a strict pass."""
    if row is None:
        return "no row"
    if compare.unconnected(row) is None:
        return no_result_text(compare, row)
    parts = []
    if compare.unconnected(row):
        parts.append(f"{compare.unconnected(row)} open")
    findings = row.get("new_findings_strict") or {}
    parts += [f"{kind} {count}" for kind, count in sorted(findings.items(), key=lambda item: (-item[1], item[0]))]
    if row.get("unexpected_misses"):
        parts.append("constraints missed")
    return ", ".join(parts) or "none"


def identical(compare, before, after):
    return all(before.get(key) == after.get(key) for key in ("routed", "vias", "length_mm", "new_findings_strict")) \
        and compare.unconnected(before) == compare.unconnected(after)


def comparison(compare, before, after):
    """`after` against `before` board by board, as compare.py counts it:
    the boards in both on the same input (INPUT boards named and left
    out), better / same / worse by KiCad's unconnected count over the
    boards with a result in both and no row cut, the strict passes gained
    and lost."""
    b, a = before["rows"], after["rows"]
    common = sorted(set(b) & set(a))
    inputs = [n for n in common if compare.on_other_input(b[n], a[n], before["directory"], after["directory"])]
    same_input = [n for n in common if n not in inputs]
    result = {"before": before, "after": after, "common": common, "inputs": inputs, "same_input": same_input,
              "better": [], "worse": [], "same": [], "cut": [], "both": [], "identical": 0}
    for n in same_input:
        ub, ua = compare.unconnected(b[n]), compare.unconnected(a[n])
        if ub is None or ua is None:
            continue
        result["both"].append(n)
        if compare.cut(a[n]) or compare.cut(b[n]):
            result["cut"].append(n)
        elif ua < ub:
            result["better"].append((n, ub, ua))
        elif ua > ub:
            result["worse"].append((n, ub, ua))
        else:
            result["same"].append(n)
        result["identical"] += identical(compare, b[n], a[n])
    result["better"].sort(key=lambda item: (item[2] - item[1], item[0]))
    result["worse"].sort(key=lambda item: (item[1] - item[2], item[0]))
    result["open_before"] = sum(compare.unconnected(b[n]) for n in result["both"])
    result["open_after"] = sum(compare.unconnected(a[n]) for n in result["both"])
    result["gained"] = [n for n in same_input if compare.passes_strict(a[n]) and not compare.passes_strict(b[n])]
    result["lost"] = [n for n in same_input if compare.passes_strict(b[n]) and not compare.passes_strict(a[n])]
    result["strict_before"] = sum(1 for n in same_input if compare.passes_strict(b[n]))
    result["strict_after"] = sum(1 for n in same_input if compare.passes_strict(a[n]))
    return result


def numbers(compare, run):
    """The totals table's numbers of one run, from its rows."""
    rows = list(run["rows"].values())
    objectives = compare.objectives(rows)
    normalised = compare.normalised(rows)
    results = [row for row in rows if compare.unconnected(row) is not None]
    in_pads = [((row.get("quality") or {}).get("ours") or {}).get("vias_in_pads") for row in results]
    shares = [row["ladder_budget_share"] for row in rows if row.get("ladder_budget_share") is not None]
    return {"objectives": objectives, "normalised": normalised, "results": len(results),
            "rules": compare.strict_rules(rows), "killed": sum(1 for row in rows if compare.killed(row)),
            "in_pads": sum(value or 0 for value in in_pads if value is not None),
            "in_pads_boards": sum(1 for value in in_pads if value), "scored": sum(1 for v in in_pads if v is not None),
            "in_pads_names": [name_of(row["name"]) for row, value in zip(results, in_pads) if value],
            "share": median(shares), "share_rows": len(shares),
            "peak_mb": max((row.get("peak_rss_mb") or 0 for row in rows), default=0),
            "wall_seconds": None}


def fails_split(compare, run):
    """The strict failures: near (at most five open and five strict
    errors), far (more than 20 open), no result; and the ones with
    nothing open."""
    rows = [row for row in run["rows"].values() if not compare.passes_strict(row)]
    near = [row for row in rows if compare.unconnected(row) is not None and compare.unconnected(row) <= 5
            and compare.strict_errors(row) <= 5]
    far = [row for row in rows if compare.unconnected(row) is not None and compare.unconnected(row) > 20]
    none = [row for row in rows if compare.unconnected(row) is None]
    closed = [row for row in rows if compare.unconnected(row) == 0]
    return {"fails": len(rows), "near": len(near), "far": len(far), "none": none, "closed": closed}


def ratio_cell(value, boards):
    return "-" if value is None else f"x{value:.2f} ({boards})"


def signed(value, unit=""):
    return "-" if value is None else f"{value:+g}{unit}"


def totals_table(compare, columns):
    """The totals table: one column per run (label, numbers), the last
    this night's."""
    rule = max((rule for _, n in columns for rule in n["rules"]), default=None)
    lines = ["| | " + " | ".join(label for label, _ in columns) + " |", "|---|" + "---|" * len(columns)]

    def row(label, cells):
        lines.append(f"| {label} | " + " | ".join(cells) + " |")

    o = [n["objectives"] for _, n in columns]
    m = [n["normalised"] for _, n in columns]
    last = len(columns) - 1

    def bold(index, text):
        return f"**{text}**" if index == last else text

    row("pass, per-type rule", [bold(i, f"{x['passes']} of {x['rows']}") for i, x in enumerate(o)])
    row(f"pass, strict rule{f' (rule {rule})' if rule else ''}",
        [bold(i, f"{x['passes_strict']} of {x['rows']}") for i, x in enumerate(o)])
    row("unconnected (all rows)", [str(x["unconnected"]) for x in o])
    row("new DRC errors (per-type rule)", [str(x["new_errors"]) for x in o])
    row("new DRC errors (strict rule)", [str(x["new_errors_strict"]) for x in o])
    results = [n["results"] for _, n in columns]
    row("vias (rows with a result)", [f"{x['vias']} ({r})" for x, r in zip(o, results)])
    row("length (rows with a result)", [f"{x['length_mm'] / 1000:.0f} m ({r})" for x, r in zip(o, results)])
    row("work s", [f"{x['work_seconds']} ({x['work_rows']})" for x in o])
    row("CPU s (unpaired, a shared machine)", [str(x["cpu_seconds"]) for x in o])
    row("first-route open terminals", [f"{x['first_unconnected_terminals']} ({x['first_rows']})" for x in o])
    row("rows cut by the wall clock / killed", [bold(i, f"{x['cut']} / {n['killed']}")
                                                for i, (x, (_, n)) in enumerate(zip(o, columns))])
    row("rows without a result", [str(x["failed"]) for x in o])
    row("our vias in small SMD pads (score-kicad-board)",
        ["-" if not n["scored"] else (f"0 on all {n['scored']}" if not n["in_pads"]
                                      else f"{n['in_pads']} on {n['in_pads_boards']} of {n['scored']} ("
                                      + ", ".join(n["in_pads_names"][:4]) + ")")
         for _, n in columns])
    row("vias against the designer's (median)", [ratio_cell(x["vias_ratio"], x["vias_boards"]) for x in m])
    row("length against the designer's (median)", [ratio_cell(x["length_ratio"], x["length_boards"]) for x in m])
    row("open over connections (median, mean)",
        ["-" if x["open_share"] is None else f"{x['open_share']:.3f}, {x['open_share_mean']:.3f}" for x in m])
    row("bends per connection against the designer's (median)",
        ["-" if x["bends_ratio"] is None else f"x{x['bends_ratio']:.2f}" for x in m])
    row("stair-steps, ours less the designer's (median)", [signed(x["stair_steps_delta"]) for x in m])
    row("pin-row crossings, ours less the designer's (median)", [signed(x["pin_row_crossings_delta"]) for x in m])
    row("ground pour cut, ours less the designer's (median)",
        ["-" if x["ground_cut_delta_mm"] is None else
         f"{x['ground_cut_delta_mm']:+g} mm ({x['ground_boards']}; {x['ground_pour_lost']} lost)" for x in m])
    row("final ladder's share of the budget (median)",
        ["-" if n["share"] is None else bold(i, f"{n['share']:.3f}") + f" ({n['share_rows']} rows)"
         for i, (_, n) in enumerate(columns)])
    return lines


def comparison_paragraph(compare, title, diff):
    """The prose of one comparison: the counts, the largest moves, the
    strict passes gained and lost with their finding types."""
    before, after = diff["before"]["rows"], diff["after"]["rows"]
    text = (f"**{title}** (compare.py, {plural(len(diff['same_input']), 'board')} in both on the same input"
            + (f", {len(diff['inputs'])} INPUT left out" if diff["inputs"] else "") + "): "
            f"better {len(diff['better'])}, same {len(diff['same'])}, worse {len(diff['worse'])}"
            + (f" ({len(diff['cut'])} cut in either run, not counted)" if diff["cut"] else "")
            + f"; {diff['identical']} of the {len(diff['both'])} with a result in both are identical in routed "
            "count, unconnected, vias, length and strict findings; unconnected there "
            f"{diff['open_before']} -> {diff['open_after']}.")
    if diff["better"]:
        text += " Better, the largest: " + listing(f"{name_of(n)} {b} -> {a}" for n, b, a in diff["better"]) + "."
    if diff["worse"]:
        text += " Worse: " + listing(f"{name_of(n)} {b} -> {a}" for n, b, a in diff["worse"]) + "."
    gained = [f"{name_of(n)} ({verdict_text(compare, before[n])} -> none)" for n in diff["gained"]]
    lost = [f"{name_of(n)} ({verdict_text(compare, after[n])})" for n in diff["lost"]]
    text += (f" Strict passes {diff['strict_before']} -> {diff['strict_after']}: gained {len(gained)}"
             + (": " + listing(gained, 20) if gained else "") + f"; lost {len(lost)}"
             + (": " + listing(lost, 20) if lost else "") + ".")
    return wrap(text)


def noise_spreads(path=NOISE):
    """{board: the spread (max - min) of its final unconnected count over
    the placer seeds of dense-noise.json}; {} without the file."""
    try:
        boards = json.loads(Path(path).read_text())["boards"]
    except (OSError, ValueError, KeyError):
        return {}
    spreads = {}
    for name, entry in boards.items():
        values = [value for value in entry.get("unconnected") or [] if value is not None]
        if len(values) >= 2:
            spreads[name] = max(values) - min(values)
    return spreads


def losers(diff, spreads):
    """The boards worse than before, split: beyond noise (the loss is
    larger than the board's seed spread, or the board has no measured
    spread) and within it."""
    beyond, within = [], []
    for name, before, after in diff["worse"]:
        spread = spreads.get(name)
        (within if spread is not None and after - before <= spread else beyond).append((name, before, after, spread))
    return beyond, within


def gap_split(compare, layout, designer, targets=True):
    """compare.py --gap's totals and table over the boards both runs have
    (`targets`: without NOT_TARGETS)."""
    keep = (lambda name: name not in compare.NOT_TARGETS) if targets else (lambda name: True)
    table = compare.gap_rows({n: r for n, r in layout["rows"].items() if keep(n)},
                             {n: r for n, r in designer["rows"].items() if keep(n)})
    return table, compare.gap_totals(table)


def share_text(totals):
    if not totals["layout_open"]:
        return "nothing open"
    return (f"placement {totals['placement_gap'] / totals['layout_open']:.0%}, "
            f"routing {totals['routing_gap'] / totals['layout_open']:.0%}").replace("%", " %")


def designer_without(compare, table):
    """The target board whose designer route leaves most open, when it
    leaves at least as much as our layout (Batch_1 on v18 to v21): the
    split without it, else None."""
    both = [entry for entry in table if entry["placement_gap"] is not None]
    if not both:
        return None
    worst = max(both, key=lambda entry: (entry["designer"], entry["name"]))
    if worst["designer"] == 0 or worst["designer"] < worst["layout"]:
        return None
    rest = compare.gap_totals([entry for entry in table if entry["name"] != worst["name"]])
    return worst, rest


def previous_sweep(text, number):
    """The numbered sweep before `number` in docs/benchmarks.md: (its
    number, its night or None, its sweep log relative to the root, its
    main or None), or None."""
    best = None
    for match, marker in sweep_sections(text):
        n = int(match.group(1))
        if n < number and (best is None or n > best[0]):
            if marker:
                best = (n, marker.group(1), marker.group(2), marker.group(3))
            else:
                best = (n, None, f"build/github-layout-v{n}.log", None)
    return best


def sweep_sections(text):
    """[(heading match, marker match or None)] of the "Layout sweep vN"
    sections of docs/benchmarks.md, in the file's order."""
    headings = list(SWEEP_HEADING.finditer(text))
    sections = []
    for match in headings:
        end = section_end(text, match.start())
        marker = SWEEP_MARKER.search(text, match.end(), end)
        sections.append((match, marker))
    return sections


def section_end(text, start):
    """The end of the "## " section that begins at `start`: the next "## "
    heading, or the end of the text."""
    following = re.compile(r"^## ", re.M).search(text, start + 3)
    return following.start() if following else len(text)


def sweep_number(text, night):
    """The night's sweep number: the one of its own section when
    docs/benchmarks.md has it (written again in place), else one more than
    the highest "Layout sweep vN" heading."""
    highest = 0
    for match, marker in sweep_sections(text):
        if marker and marker.group(1) == night:
            return int(match.group(1))
        highest = max(highest, int(match.group(1)))
    return highest + 1


def published_nights(text):
    return sorted(marker.group(1) for _, marker in sweep_sections(text) if marker)


def git_count(root, older, newer):
    if not older or not newer:
        return None
    result = git(root, "rev-list", "--count", f"{older}..{newer}", check=False)
    return int(result.stdout.strip()) if result.returncode == 0 and result.stdout.strip().isdigit() else None


def binary_label(run):
    names = sorted({Path(str(row["binary"])).name for row in run["rows"].values() if row.get("binary")})
    if not names:
        return None
    label = names[0] if len(names) == 1 else f"{len(names)} binaries"
    return label.removeprefix("pcb-maker-")


class Writeup:
    """The night's published write-up: the sweep section of
    docs/benchmarks.md, the README's headline and rows, the hand-over's
    title and night note, the changelog section."""

    def __init__(self, compare, record, nights, benchmarks_text, root=None, spreads=None):
        self.compare = compare
        self.record = record
        self.nights = Path(nights)
        self.root = Path(root) if root else self.nights.resolve().parent.parent
        self.night = record["night"]
        self.date = self.night[:10]
        self.sha = record["sha"]
        self.number = sweep_number(benchmarks_text, self.night)
        self.title = f"Layout sweep v{self.number} ({self.date})"
        self.anchor = f"layout-sweep-v{self.number}-{self.date}"
        directory = self.nights / self.night
        self.sweep = load_run(compare, directory / "sweep.log", f"v{self.number}", "this")
        self.designer = load_run(compare, directory / "designer.log", "the matched designer tier", "designer")
        self.spreads = noise_spreads() if spreads is None else spreads
        self.mine = numbers(compare, self.sweep)
        self.mine["wall_seconds"] = ((record["stages"]["sweep"].get("summary") or {}).get("wall_seconds"))
        # The previous night that ran the sweep.
        self.previous = None
        name, previous_record = previous_night(self.nights, self.night, "sweep")
        if name:
            self.previous = load_run(compare, self.nights / name / "sweep.log", f"nightly {name}", "night")
            self.previous.update(night=name, sha=previous_record.get("sha"))
        # The previous numbered sweep (its log where it still is).
        self.numbered = None
        found = previous_sweep(benchmarks_text, self.number)
        self.numbered_missing = None
        if found:
            number, night, log, main = found
            if night and self.previous and night == self.previous.get("night"):
                self.numbered = self.previous
                self.previous["label"] = f"v{number} (nightly {night})"
                self.previous["number"] = number
            elif (self.root / log).is_file():
                self.numbered = load_run(compare, self.root / log, f"v{number}", "numbered")
                self.numbered.update(number=number, night=night, sha=main if main != "-" else None)
                binary = binary_label(self.numbered)
                self.numbered["label"] = f"v{number} (nightly {night})" if night \
                    else f"v{number} ({binary})" if binary else f"v{number}"
            else:
                self.numbered_missing = (number, log)
        self.against_previous = comparison(compare, self.previous, self.sweep) if self.previous else None
        self.against_numbered = comparison(compare, self.numbered, self.sweep) \
            if self.numbered and self.numbered is not self.previous else None
        self.gap_table, self.gap = gap_split(compare, self.sweep, self.designer)
        self.gap_all_table, self.gap_all = gap_split(compare, self.sweep, self.designer, targets=False)
        self.fails = fails_split(compare, self.sweep)
        self.designer_numbers = numbers(compare, self.designer)

    # --- pieces ------------------------------------------------------------------

    def short_sha(self, sha=None):
        return (sha or self.sha)[:7]

    def strict(self):
        return self.mine["objectives"]["passes_strict"]

    def rows(self):
        return self.mine["objectives"]["rows"]

    def runs_label(self):
        return f"v{self.number} (nightly {self.night}, {self.short_sha()})"

    def first_paragraph(self):
        stages = self.record["stages"]
        sweep = stages["sweep"].get("summary") or {}
        rows = list(self.sweep["rows"].values())
        tasks = sorted({(row.get("tasks"), (row.get("tasks_sha256") or "")[:12]) for row in rows if row.get("tasks")})
        rates = [row.get("work_rate_measured") for row in rows if row.get("work_rate_measured")]
        ratios = [row.get("work_wall_ratio") for row in rows if row.get("work_wall_ratio") is not None]
        since = []
        if self.numbered and self.numbered.get("sha"):
            count = git_count(self.root, self.numbered["sha"], self.sha)
            if count is not None:
                since.append(f"{plural(count, 'commit')} after v{self.numbered['number']}'s main "
                             f"{self.short_sha(self.numbered['sha'])}")
        if self.previous and self.previous.get("sha"):
            count = git_count(self.root, self.previous["sha"], self.sha)
            if count is not None:
                since.append(f"{plural(count, 'commit')} after the nightly of {self.previous['night']}'s main "
                             f"{self.short_sha(self.previous['sha'])}")
        task_text = ", ".join(f"{Path(path).parent.name} (sha256 {digest})" for path, digest in tasks) or "?"
        cut = sum(len(stages[s].get("cut") or []) for s in STAGES)
        killed = sum(len(stages[s].get("killed") or []) for s in STAGES)
        text = (f"Written by the nightly (`scripts/nightly-report.py --docs`) from `build/nightly/{self.night}/`: "
                f"main {self.short_sha()}" + (f" ({'; '.join(since)})" if since else "")
                + f", binary `{Path(self.record['binary']).name}` (sha256 {(self.record.get('binary_sha256') or '?')[:12]}), "
                f"tasks {task_text}, `--quality`, at the pinned work rates"
                + (f" (the measured router rate had a median of {scientific(median(rates))}, work over wall "
                   f"{median(ratios):.2f})" if rates and ratios else "")
                + f", {sweep.get('jobs', '?')} jobs x {sweep.get('threads', '?')} threads, `--order peak-desc`, "
                "through the memory ledger with `--ledger-priority nightly`: "
                f"{sweep.get('wall_seconds', '?')} s wall, largest peak {self.mine['peak_mb'] / 1000:.1f} GB, "
                + ("no row cut or killed in any stage" if not cut and not killed
                   else f"{cut} rows cut, {killed} killed")
                + f". Per board: `build/nightly/{self.night}/summary.md`, `diff-sweep.txt` and `gap.txt`.")
        return wrap(text)

    def inputs_paragraph(self):
        parts = []
        for diff in (self.against_previous, self.against_numbered):
            if diff is None:
                continue
            label = diff["before"]["label"]
            if diff["inputs"]:
                parts.append(f"{plural(len(diff['inputs']), 'board')} ran on other inputs than in {label} "
                             f"(INPUT: project files, task, board or constraints changed; left out of that "
                             f"comparison): {listing((name_of(n) for n in diff['inputs']), 30)}")
            else:
                parts.append(f"every board in both ran on the same input as in {label}")
        if self.numbered_missing:
            parts.append(f"v{self.numbered_missing[0]}'s log ({self.numbered_missing[1]}) is not there, so this "
                         "section does not compare with it")
        if not parts:
            return None
        return wrap("**Inputs.** " + "; ".join(parts)[0].upper() + "; ".join(parts)[1:] + ".")

    def table(self):
        columns = []
        if self.numbered and self.numbered is not self.previous:
            columns.append((self.numbered["label"], numbers(self.compare, self.numbered)))
        if self.previous:
            label = self.previous["label"] + (f" ({self.short_sha(self.previous['sha'])})"
                                              if self.previous.get("sha") and "number" not in self.previous else "")
            columns.append((label, numbers(self.compare, self.previous)))
        columns.append((self.runs_label(), self.mine))
        lines = totals_table(self.compare, columns)
        # Where both have a result, against each earlier column.
        cells = []
        for run in [self.numbered if self.numbered is not self.previous else None, self.previous]:
            if run is None:
                continue
            diff = self.against_numbered if run is self.numbered and run is not self.previous else self.against_previous
            cells.append(f"{diff['open_before']} / **{diff['open_after']}** ({len(diff['both'])} boards)")
        if cells:
            lines.insert(5, "| unconnected where both have a result on the same input (that run / this) | "
                         + " | ".join(cells) + " | - |")
        for diff, run in ((self.against_numbered, self.numbered), (self.against_previous, self.previous)):
            if diff and diff["inputs"]:
                cells = ["-"] * len(columns)
                cells[len(columns) - 2 if run is self.previous else 0] = str(diff["strict_before"])
                cells[-1] = str(diff["strict_after"])
                lines.insert(4, f"| strict passes on the {len(diff['same_input'])} boards of the same input as "
                             f"{diff['before']['label']} | " + " | ".join(cells) + " |")
        return lines

    def without_result_paragraph(self):
        rows = self.fails["none"]
        if not rows:
            return wrap("**Rows without a result**: none.")
        before = self.previous["rows"] if self.previous else {}
        items = []
        for row in sorted(rows, key=lambda row: name_of(row["name"]).lower()):
            now = no_result_text(self.compare, row)
            text = f"{name_of(row['name'])} ({now}"
            if row["name"] in before:
                then = verdict_text(self.compare, before[row["name"]])
                same_kind = re.match(r"\d+ (.*)$", now), re.match(r"\d+ (.*)$", then)
                if all(same_kind) and same_kind[0].group(1) == same_kind[1].group(1):
                    then = then.split(" ", 1)[0]
                text += f"; the previous night {then}"
            items.append(text + ")")
        previous = ""
        if self.previous:
            previous = f" (the nightly of {self.previous['night']}: " \
                       f"{numbers(self.compare, self.previous)['objectives']['failed']})"
        return wrap(f"**Rows without a result, {len(rows)}**{previous}: " + listing(items, 30) + ".")

    def strict_errors_paragraph(self):
        rows = [row for row in self.sweep["rows"].values() if self.compare.unconnected(row) is not None
                and self.compare.strict_errors(row)]
        rows.sort(key=lambda row: (-self.compare.strict_errors(row), name_of(row["name"]).lower()))
        total = sum(self.compare.strict_errors(row) for row in rows)
        items = [name_of(row["name"]) + " (" + ", ".join(f"{kind} {count}" for kind, count in sorted(
            row["new_findings_strict"].items(), key=lambda item: (-item[1], item[0]))) + ")" for row in rows]
        closed = [f"{name_of(row['name'])} ({verdict_text(self.compare, row)})" for row in self.fails["closed"]]
        text = (f"**Strict errors, {total} on {plural(len(rows), 'board')}**: " + listing(items, 30) + ". "
                + (f"Failing the strict rule with nothing open, {len(closed)}: " + listing(closed, 20) + ". "
                   if closed else "No board fails the strict rule with nothing open. ")
                + f"Of the {self.fails['fails']} that fail, {self.fails['near']} are within five connections and "
                f"five errors of clean, {self.fails['far']} leave more than 20 open, {len(self.fails['none'])} "
                "have no result.")
        return wrap(text)

    def designer_section(self):
        d = self.designer_numbers
        o, m = d["objectives"], d["normalised"]
        rows = list(self.designer["rows"].values())
        summary = self.record["stages"]["designer"].get("summary") or {}
        budgets = sorted({row["budget_seconds"] for row in rows if row.get("budget_seconds") is not None})
        scale = summary.get("budget_scale")
        lines = [f"### Placement or routing on one binary (matched designer tier, nightly {self.night})", ""]
        text = (f"`build/nightly/{self.night}/designer.log`: {plural(o['rows'], 'board')} routed by the same "
                f"binary on the designer's placement at `--budget-scale {scale if scale is not None else '?'}` (the median final-ladder "
                "share of the previous night's sweep, `nightly-report.py --ladder-share`"
                + (f"; {'/'.join(f'{b:g}' for b in budgets)} s of work" if budgets else "") + "), `--quality`, "
                f"{summary.get('jobs', '?')} jobs x {summary.get('threads', '?')} thread"
                f"{'s' if summary.get('threads') != 1 else ''}, `--order peak-desc`: "
                f"{summary.get('wall_seconds', '?')} s wall, largest peak {d['peak_mb'] / 1000:.1f} GB, "
                + ("no row cut or killed" if not o["cut"] and not d["killed"]
                   else f"{o['cut']} rows cut, {d['killed']} killed") + f", {d['results']} with a result. "
                f"**{o['passes']} of {o['rows']} pass the per-type rule, {o['passes_strict']} the strict rule**; "
                f"{o['unconnected']} unconnected in all; strict errors {o['new_errors_strict']}; medians against "
                f"the designer's own board: vias {ratio(m['vias_ratio'])}, length {ratio(m['length_ratio'])}, "
                f"bends {ratio(m['bends_ratio'])}, stair-steps {signed(m['stair_steps_delta'])}.")
        lines += [wrap(text), ""]
        g = self.gap
        text = (f"`compare.py --gap build/nightly/{self.night}/sweep.log build/nightly/{self.night}/designer.log "
                f"--targets` over the **{g['both']} target boards with both results** (of {g['boards']}): "
                f"**layout open {g['layout_open']}, designer-route open {g['designer_open']}: placement gap "
                f"{g['placement_gap']:+d}"
                + (f" ({g['placement_gap'] / g['layout_open']:.0%}), routing gap {g['routing_gap']} "
                   f"({g['routing_gap'] / g['layout_open']:.0%})**" if g["layout_open"]
                   else f", routing gap {g['routing_gap']}**")
                + f"; ours is worse on {g['placement_worse']}, the same on {g['placement_same']}, better on "
                f"{g['placement_better']}.").replace("%)", " %)")
        without = designer_without(self.compare, self.gap_table)
        if without:
            worst, rest = without
            text += (f" {name_of(worst['name'])} dominates the routing side (the designer route {worst['designer']} "
                     f"open, ours {worst['layout']}); without it {rest['layout_open']} against "
                     f"{rest['designer_open']}: {share_text(rest)}.")
        a = self.gap_all
        if a["both"] != g["both"]:
            text += (f" Over all {a['both']} boards with both results (the panels included): {a['layout_open']} "
                     f"against {a['designer_open']}, {share_text(a)}.")
        previous = (self.record_of_previous() or {}).get("gap")
        if previous and previous.get("placement_share") is not None:
            text += (f" The previous night's split ({self.previous['night']}, {previous['both']} target boards): "
                     f"{previous['placement_share']:.0%} / {previous['routing_share']:.0%}.").replace("%", " %")
        lines += [wrap(text), ""]
        costly = sorted((entry for entry in self.gap_table if entry["placement_gap"] is not None),
                        key=lambda entry: (-entry["placement_gap"], entry["name"]))
        lines += ["**The five boards whose placement costs most** (ours / designer route, open connections):", "",
                  "| Board | Connections | Layout open | Designer route open | Placement gap |",
                  "| --- | --- | --- | --- | --- |"]
        for entry in costly[:5]:
            lines.append(f"| {name_of(entry['name'])} | {entry['connections']} | {entry['layout']} "
                         f"| {entry['designer']} | {entry['placement_gap']:+d} |")
        routing = sorted((entry for entry in self.gap_table if entry["placement_gap"] is not None
                          and entry["designer"] and entry["placement_gap"] <= 0),
                         key=lambda entry: (-entry["designer"], entry["name"]))
        text = ("Next: " + listing((f"{name_of(e['name'])} {e['placement_gap']:+d} ({e['layout']} / {e['designer']})"
                                    for e in costly[5:10] if e["placement_gap"] > 0), 5) + ". "
                + "Routing first (the designer route leaves as much or more): "
                + listing((f"{name_of(e['name'])} {e['designer']} / ours {e['layout']}" for e in routing), 6) + ".")
        lines += ["", wrap(text)]
        return lines

    def record_of_previous(self):
        return read_night(self.nights / self.previous["night"]) if self.previous else None

    def losers_paragraph(self):
        diff = self.against_previous or self.against_numbered
        if diff is None:
            return wrap("**Losers**: no earlier sweep to compare with.")
        beyond, within = losers(diff, self.spreads)
        label = (f"the nightly of {diff['before']['night']}" if diff["before"].get("night")
                 else diff["before"]["label"])
        text = (f"**Worse than {label}, beyond noise** (the next triage; a layout is deterministic for its "
                "binary, so every change is the binary's; on the dense boards of `dense-noise.json` a loss "
                "counts only when it exceeds the spread of the board's unconnected count over the placer seeds "
                f"there), {len(beyond)}: "
                + listing((f"{name_of(n)} {b} -> {a}" + (f" (spread {s})" if s is not None else "")
                           for n, b, a, s in beyond), 20) + ".")
        if diff["lost"]:
            text += (" Losing the strict pass: " + listing(f"{name_of(n)} ({verdict_text(self.compare, diff['after']['rows'][n])})"
                                                          for n in diff["lost"]) + ".")
        if within:
            text += (" Within the seed spread: " + listing(f"{name_of(n)} {b} -> {a} (spread {s})"
                                                         for n, b, a, s in within) + ".")
        return wrap(text)

    # --- the four documents ------------------------------------------------------

    def section(self):
        lines = [f"## {self.title}", "",
                 f"<!-- nightly-sweep night={self.night} sweep=build/nightly/{self.night}/sweep.log main={self.sha} -->",
                 "", self.first_paragraph(), ""]
        inputs = self.inputs_paragraph()
        if inputs:
            lines += [inputs, ""]
        lines += self.table() + [""]
        if self.against_previous:
            lines += [comparison_paragraph(self.compare, f"Against the nightly of {self.previous['night']}"
                                           + (f" (v{self.previous['number']})" if "number" in self.previous else ""),
                                           self.against_previous), ""]
        if self.against_numbered:
            lines += [comparison_paragraph(self.compare, f"Against v{self.numbered['number']}",
                                           self.against_numbered), ""]
        lines += [self.without_result_paragraph(), "", self.strict_errors_paragraph(), ""]
        lines += self.designer_section() + ["", self.losers_paragraph()]
        return "\n".join(lines) + "\n"

    def comparisons_short(self):
        parts = []
        for diff, label in ((self.against_previous, f"the nightly of {self.previous['night']}" if self.previous
                             else None),
                            (self.against_numbered, f"v{self.numbered['number']}" if self.numbered else None)):
            if diff is None:
                continue
            text = (f"against {label}: better {len(diff['better'])}, same {len(diff['same'])}, worse "
                    f"{len(diff['worse'])}, strict passes {diff['strict_before']} -> {diff['strict_after']}")
            if diff["gained"]:
                text += f" (gained {listing(name_of(n) for n in diff['gained'])}"
                text += f"; lost {listing(name_of(n) for n in diff['lost'])})" if diff["lost"] else ")"
            elif diff["lost"]:
                text += f" (lost {listing(name_of(n) for n in diff['lost'])})"
            if diff["inputs"]:
                text += f", {len(diff['inputs'])} INPUT left out"
            parts.append(text)
        return parts

    def medians_text(self):
        m = self.mine["normalised"]
        return (f"vias {ratio(m['vias_ratio'])}, length {ratio(m['length_ratio'])}, open "
                + ("-" if m["open_share"] is None else f"{m['open_share']:.3f} (mean {m['open_share_mean']:.3f})")
                + f", bends {ratio(m['bends_ratio'])}, stair-steps {signed(m['stair_steps_delta'])}, crossings "
                f"{signed(m['pin_row_crossings_delta'])}")

    def gap_short(self):
        g = self.gap
        text = f"over the {g['both']} target boards {share_text(g)} of the open connections"
        without = designer_without(self.compare, self.gap_table)
        if without:
            text += f" ({share_text(without[1])} without {name_of(without[0]['name'])})"
        return text

    def handover_note(self):
        o = self.mine["objectives"]
        d = self.designer_numbers["objectives"]
        scale = (self.record["stages"]["designer"].get("summary") or {}).get("budget_scale")
        none = self.fails["none"]
        text = (f"**Layout sweep v{self.number}** (the nightly of {self.night}, main {self.short_sha()}, written by "
                f"the nightly): **{o['passes_strict']} of {o['rows']} pass the strict rule** ({o['passes']} the "
                f"per-type rule), {o['unconnected']} unconnected in all, strict errors {o['new_errors_strict']}, "
                f"no row cut or killed, {len(none)} without a result; medians against the designer's board: "
                f"{self.medians_text()}. " + capital("; ".join(self.comparisons_short()))
                + ("." if self.comparisons_short() else "")
                + f" The matched designer tier on the same binary (budget scale {scale:g}): {d['passes_strict']} "
                f"strict passes ({d['passes']} per type); {self.gap_short()}. `build/nightly/{self.night}/`; "
                f"table, gap split, the costliest placements and the losers in "
                f"[benchmarks.md](benchmarks.md#{self.anchor}).")
        return wrap(text)

    def handover_title(self):
        return (f"# Hand-over (state as of {self.date}; measured by layout sweep v{self.number}, the nightly of "
                f"{self.night} on main {self.short_sha()})")

    def readme_headline(self):
        o = self.mine["objectives"]
        m = self.mine["normalised"]
        none = self.fails["none"]
        reasons = Counter("without a legal place for every part" if "without a legal place" in
                          no_result_text(self.compare, row) else no_result_text(self.compare, row) for row in none)
        none_text = "; ".join(("all" if count == len(none) and count > 1 else str(count)) + f" {reason}"
                              for reason, count in sorted(reasons.items(), key=lambda i: -i[1]))
        open_text = ("nothing left open on more than half the boards" if m["open_share"] == 0
                     else f"a median {m['open_share']:.1%} of the connections open")
        pads = ("no via in a small SMD pad" if not self.mine["in_pads"]
                else f"vias in small SMD pads on {self.mine['in_pads_boards']} boards")
        g = self.gap
        without = designer_without(self.compare, self.gap_table)
        medians = "".join(f"{m[key]:.2f} times the {label}, " for key, label in (
            ("vias_ratio", "vias"), ("length_ratio", "track length"), ("bends_ratio", "bends per connection"))
            if m[key] is not None)
        text = (f"{README_SPLIT} **{o['passes_strict']} of {o['rows']}** to a clean KiCad verdict under the "
                "strict rule (0 unconnected, no copper error beyond what the designer's board has at the same "
                f"place; {o['passes']} under the per-type rule) within 1500 s of work each, every board placed "
                f"from stacked parts (sweep v{self.number}, the nightly of {self.night}). Per board against the "
                f"designer's own board (medians): {medians}{open_text} "
                f"({(m['open_share_mean'] or 0):.0%} of the connections open on average), {pads}. Of the "
                f"{self.fails['fails']} that fail, {self.fails['near']} are within five connections and five "
                f"errors of clean, {self.fails['far']} leave more than 20 connections open, and {len(none)} have "
                f"no result" + (f" ({none_text})" if none_text else "") + ". Routed by the same binary on the "
                f"designer's own placement at the layout's routing budget, on {'all ' if g['both'] == g['boards'] else ''}"
                f"{g['both']} target boards our placement accounts for "
                + (f"{g['placement_gap'] / g['layout_open']:.0%} of the open connections and our router for "
                   f"{g['routing_gap'] / g['layout_open']:.0%}" if g["layout_open"] else "none of the open connections")
                + (f" ({without[1]['placement_gap'] / without[1]['layout_open']:.0%} and "
                   f"{without[1]['routing_gap'] / without[1]['layout_open']:.0%} without "
                   f"{name_of(without[0]['name'])}, whose designer placement leaves as much open as ours or more)"
                   if without and without[1]["layout_open"] else "")
                + f" ([results](docs/benchmarks.md#{self.anchor})).").replace("%", " %")
        return text

    def readme_rows(self):
        o = self.mine["objectives"]
        rule = max(self.mine["rules"], default=None)
        rule_text = f"strict rule {rule}" if rule else "strict rule"
        compared = ""
        diff = self.against_numbered or self.against_previous
        if diff:
            compared = (f" ({diff['open_after']} on the {len(diff['both'])} boards with a result in "
                        f"{diff['before']['label'].split(' (')[0]} too, which left {diff['open_before']} there)")
        cut, killed = o["cut"], self.mine["killed"]
        layout = (f"{README_LAYOUT_ROW}, 1500 s of work (v{self.number}, nightly {self.night}, main "
                  f"{self.short_sha()}) | {o['rows']} | {o['passes_strict']} clean ({rule_text}; {o['passes']} per "
                  f"type), {o['unconnected']} connections open in all{compared}, "
                  + ("no row cut or killed" if not cut and not killed else f"{cut} rows cut, {killed} killed")
                  + " | - |")
        d = self.designer_numbers["objectives"]
        budgets = sorted({row["budget_seconds"] for row in self.designer["rows"].values()
                          if row.get("budget_seconds") is not None})
        g = self.gap
        scale = (self.record["stages"]["designer"].get("summary") or {}).get("budget_scale")
        work = (f"{'/'.join(f'{b:g}' for b in budgets)} s of work" if budgets
                else f"budget scale {scale:g}" if scale is not None else "the matched budget")
        designer = (f"{README_DESIGNER_ROW}, {work} (matched "
                    f"designer tier, nightly {self.night}) | {d['rows']} | {d['passes_strict']} clean ({rule_text}; "
                    f"{d['passes']} per type), {g['designer_open']} open on the {g['both']} target boards, against "
                    f"{g['layout_open']} of the v{self.number} layout there | - |")
        return layout, designer

    def changelog_section(self):
        o = self.mine["objectives"]
        d = self.designer_numbers["objectives"]
        scale = (self.record["stages"]["designer"].get("summary") or {}).get("budget_scale")
        costly = sorted((entry for entry in self.gap_table if entry["placement_gap"] is not None),
                        key=lambda entry: (-entry["placement_gap"], entry["name"]))[:5]
        diff = self.against_previous or self.against_numbered
        beyond = losers(diff, self.spreads)[0] if diff else []
        lines = [f"## Measured on {self.date} (nightly: layout sweep v{self.number}, matched designer tier)", "",
                 wrap(f"Written by the nightly from `build/nightly/{self.night}/` (main {self.short_sha()}; "
                      f"[benchmarks.md](benchmarks.md#{self.anchor})). The guard held: every stage ran, no row "
                      "was cut or killed, the sweep has at least 100 rows with a result."), ""]
        items = [
            f"**Layout sweep v{self.number}**: **{o['passes_strict']} of {o['rows']} pass the strict rule** "
            f"({o['passes']} per type), {o['unconnected']} unconnected in all, strict errors "
            f"{o['new_errors_strict']}, {len(self.fails['none'])} without a result. "
            + capital("; ".join(self.comparisons_short())) + ".",
            f"**Medians against the designer's**: {self.medians_text()}.",
            f"**Matched designer tier** (budget scale {scale:g}): {d['passes_strict']} strict passes "
            f"({d['passes']} per type); {self.gap_short()}.",
            "**Placement costs most** on " + listing(f"{name_of(e['name'])} ({e['placement_gap']:+d}: "
                                                     f"{e['layout']} / {e['designer']})" for e in costly) + ".",
            "**Worse beyond noise**: " + listing(f"{name_of(n)} {b} -> {a}" for n, b, a, _ in beyond) + "."]
        for item in items:
            lines.append(wrap(item, initial="- ", subsequent="  "))
        return "\n".join(lines) + "\n"


# --- merging the write-up into the documents --------------------------------------


def merge_section(text, night, number, section):
    """docs/benchmarks.md with the night's sweep section: in place of its
    own (the same night written again), else after the newest numbered
    sweep's section (its subsections included), else at the end."""
    sections = sweep_sections(text)
    for match, marker in sections:
        if marker and marker.group(1) == night:
            rest = text[section_end(text, match.start()):]
            return text[:match.start()] + section + ("\n" + rest if rest else "")
    if not sections:
        return text.rstrip("\n") + "\n\n" + section
    newest = max(sections, key=lambda item: int(item[0].group(1)))[0]
    end = section_end(text, newest.start())
    # Its "### " subsections belong to it: the next "## " heading ends it.
    return text[:end].rstrip("\n") + "\n\n" + section + ("\n" + text[end:] if end < len(text) else "")


def merge_readme(text, headline, rows):
    """README.md with the headline bullet's measured part and the two
    rows of the newest nightly: the bullet that begins with README_BULLET
    from README_SPLIT on; the nightly's earlier rows (the ones that name
    a nightly) removed and the two rows put above the first layout row."""
    lines = text.split("\n")
    start = next((i for i, line in enumerate(lines) if line.startswith(README_BULLET)), None)
    if start is None:
        raise ValueError(f"README.md has no bullet that begins with {README_BULLET}")
    end = start + 1
    while end < len(lines) and lines[end].startswith("  ") and not lines[end].startswith("- "):
        end += 1
    bullet = " ".join(line.strip() for line in lines[start:end])
    if README_SPLIT not in bullet:
        raise ValueError(f"README.md's {README_BULLET} bullet has no '{README_SPLIT}'")
    bullet = bullet.split(README_SPLIT, 1)[0] + headline
    wrapped = wrap(bullet, subsequent="  ").split("\n")
    lines[start:end] = wrapped
    lines = [line for line in lines if not ((line.startswith(README_LAYOUT_ROW) or line.startswith(README_DESIGNER_ROW))
                                            and ", nightly 20" in line.split("|")[1])]
    first = next((i for i, line in enumerate(lines) if line.startswith(README_LAYOUT_ROW)
                  or line.startswith(README_DESIGNER_ROW)), None)
    if first is None:
        raise ValueError(f"README.md has no row that begins with {README_LAYOUT_ROW}")
    lines[first:first] = list(rows)
    return "\n".join(lines)


def merge_handover(text, title, note):
    """docs/handover.md with the title line (its state) and the night note:
    the nightly's earlier note replaced, else the note after the last
    "**Layout sweep vN**" paragraph."""
    lines = text.split("\n")
    if lines and lines[0].startswith(HANDOVER_TITLE):
        lines[0] = title
    text = "\n".join(lines)
    paragraphs = text.split("\n\n")
    notes = [i for i, paragraph in enumerate(paragraphs) if paragraph.startswith(HANDOVER_NOTE)]
    if not notes:
        raise ValueError(f"docs/handover.md has no paragraph that begins with {HANDOVER_NOTE}")
    generated = [i for i in notes if "written by the nightly)" in paragraphs[i].replace("\n", " ")]
    if generated:
        paragraphs[generated[-1]] = note
        for i in reversed(generated[:-1]):
            del paragraphs[i]
    else:
        paragraphs.insert(notes[-1] + 1, note)
    return "\n\n".join(paragraphs)


def merge_changelog(text, heading, section):
    """docs/changelog.md with the section: in place of a section of the same
    heading, else above the first section dated on or before its date
    (newest first)."""
    if heading in text:
        start = text.index(heading)
        rest = text[section_end(text, start):]
        return text[:start] + section + ("\n" + rest if rest else "")
    date = re.search(r"\d{4}-\d{2}-\d{2}", heading).group(0)
    for match in CHANGELOG_HEADING.finditer(text):
        if match.group(1) <= date:
            return text[:match.start()] + section + "\n" + text[match.start():]
    return text.rstrip("\n") + "\n\n" + section


def write_up(checkout, record, nights, harness=REPO, root=None, spreads=None):
    """{path: new text} of the four documents with the night published,
    and the Writeup."""
    compare = load_compare(harness)
    checkout = Path(checkout)
    texts = {path: (checkout / path).read_text() for path in DOC_FILES}
    writeup = Writeup(compare, record, nights, texts["docs/benchmarks.md"], root=root, spreads=spreads)
    benchmarks = merge_section(texts["docs/benchmarks.md"], writeup.night, writeup.number, writeup.section())
    benchmarks = merge_table(benchmarks, table_row(record))
    handover = merge_state(texts["docs/handover.md"], state_line(record))
    handover = merge_handover(handover, writeup.handover_title(), writeup.handover_note())
    changelog = writeup.changelog_section()
    new = {"docs/benchmarks.md": benchmarks,
           "README.md": merge_readme(texts["README.md"], writeup.readme_headline(), writeup.readme_rows()),
           "docs/handover.md": handover,
           "docs/changelog.md": merge_changelog(texts["docs/changelog.md"], changelog.split("\n", 1)[0], changelog)}
    return new, writeup


def publish(checkout, night_directory, harness=REPO, min_results=MIN_SWEEP_RESULTS, dry_run=False, root=None,
            spreads=None):
    """`--docs`: the night into `checkout`'s four documents when the guard
    lets it (else the reason, nothing written). Returns the Writeup or
    None."""
    night_directory = Path(night_directory)
    record = read_night(night_directory)
    if record is None:
        print(f"{night_directory} has no nightly.json; nothing published")
        return None
    reasons = guard(record, min_results)
    if reasons:
        print(f"nightly {record['night']}: {guard_line(record, min_results)}")
        return None
    new, writeup = write_up(checkout, record, night_directory.parent, harness, root, spreads)
    if dry_run:
        print(writeup.section())
        for path, text in new.items():
            changed = text != (Path(checkout) / path).read_text()
            print(f"{path}: {'would change' if changed else 'unchanged'}")
        print(f"commit message: {commit_message([writeup])}")
        return writeup
    for path, text in new.items():
        (Path(checkout) / path).write_text(text)
    print(f"wrote nightly {record['night']} as layout sweep v{writeup.number} into {checkout}: "
          + ", ".join(DOC_FILES))
    return writeup


def commit_message(writeups):
    newest = writeups[-1]
    o = newest.mine["objectives"]
    text = f"nightly {newest.date}: layout sweep v{newest.number}"
    if len(writeups) > 1:
        text += " (with " + ", ".join(f"v{w.number} of {w.night}" for w in writeups[:-1]) + ")"
    text += f", {o['passes_strict']} of {o['rows']} strict"
    diff = newest.against_previous
    if diff:
        text += f" ({diff['strict_after'] - diff['strict_before']:+d} against the nightly of {diff['before']['night']})"
    d = newest.designer_numbers["objectives"]
    text += f", matched designer tier {d['passes_strict']} strict"
    if newest.gap["layout_open"]:
        text += (f", gap placement {newest.gap['placement_gap'] / newest.gap['layout_open']:.0%} / routing "
                 f"{newest.gap['routing_gap'] / newest.gap['layout_open']:.0%}")
    return text


def pending(nights, benchmarks_text, min_results=MIN_SWEEP_RESULTS, keep=3):
    """The nights to publish: those after the newest night docs/benchmarks.md
    has a section of (the newest night alone when it has none), that pass
    the guard, oldest first, at most `keep`. Prints why a night is left
    out."""
    names = night_names(nights)
    published = published_nights(benchmarks_text)
    if published:
        candidates = [name for name in names if name > published[-1]]
    else:
        candidates = names[-1:]
    records = []
    for name in candidates:
        record = read_night(Path(nights) / name)
        if record is None:
            print(f"nightly {name}: no nightly.json; not published")
            continue
        reasons = guard(record, min_results)
        if reasons:
            print(f"nightly {name}: {guard_line(record, min_results)}")
            continue
        records.append(record)
    return records[-keep:]


def commit(nights, checkout, harness=REPO, min_results=MIN_SWEEP_RESULTS, root=None, spreads=None):
    """`--commit` (nightly.sh's last step): the nights not yet published,
    into the main checkout's documents, committed alone on main and pushed.
    Only when the checkout is on main, its working tree has no change to a
    tracked file (the supervisor's merges and edits happen there) and `git
    pull --ff-only origin main` succeeds; otherwise nothing is written and
    the next night publishes this one with its own. Returns whether it
    committed."""
    checkout = Path(checkout)
    branch = git(checkout, "rev-parse", "--abbrev-ref", "HEAD", check=False).stdout.strip()
    if branch != "main":
        print(f"report not committed: {checkout} is on {branch or '?'}, not main; the night waits for the next one")
        return False
    dirty = git(checkout, "status", "--porcelain", "--untracked-files=no").stdout.splitlines()
    if dirty:
        print(f"report not committed: main's working tree {checkout} has changes to {plural(len(dirty), 'file')} ("
              + ", ".join(line[3:] for line in dirty[:5]) + (", ..." if len(dirty) > 5 else "")
              + "); the night's docs wait for the next night")
        return False
    pulled = git(checkout, "pull", "--ff-only", "--quiet", "origin", "main", check=False)
    if pulled.returncode:
        print("report not committed: git pull --ff-only origin main failed ("
              + (pulled.stderr.strip().splitlines() or ["?"])[-1] + "); the night's docs wait for the next night")
        return False
    records = pending(nights, (checkout / "docs/benchmarks.md").read_text(), min_results)
    if not records:
        print("report: no night to publish")
        return False
    writeups = []
    for record in records:
        new, writeup = write_up(checkout, record, nights, harness, root, spreads)
        for path, text in new.items():
            (checkout / path).write_text(text)
        writeups.append(writeup)
    if not git(checkout, "status", "--porcelain", "--", *DOC_FILES).stdout.strip():
        print("report: the docs already have these nights")
        return False
    message = commit_message(writeups)
    # The paths given: only these files go into the commit.
    git(checkout, "commit", "--quiet", "-m", message, "--", *DOC_FILES)
    print(f"committed on main: {message}")
    pushed = git(checkout, "push", "--quiet", "origin", "main", check=False)
    if pushed.returncode:
        print(f"git push origin main failed (the commit stays on main): {pushed.stderr.strip()}")
    else:
        print("pushed main")
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--nights", type=Path, default=None,
                        help="the nights' directory (default: build/nightly of the main checkout)")
    parser.add_argument("--print", action="store_true", help="print the newest night's table row and state line")
    parser.add_argument("--docs", type=Path, metavar="NIGHT",
                        help="publish NIGHT (a night's directory) into --checkout's documents, if the guard lets it")
    parser.add_argument("--dry-run", action="store_true", help="with --docs: print the section, write nothing")
    parser.add_argument("--commit", action="store_true",
                        help="publish the nights not yet published into --checkout (main), commit and push")
    parser.add_argument("--checkout", type=Path, default=None,
                        help="the checkout whose documents to write (default: this one; with --commit the main "
                             "checkout)")
    parser.add_argument("--min-results", type=int, default=MIN_SWEEP_RESULTS,
                        help="the guard's least number of sweep rows with a result (tests only)")
    parser.add_argument("--summarize", type=Path, metavar="NIGHT", help="write NIGHT's nightly.json and summary.md")
    parser.add_argument("--sha", default="?")
    parser.add_argument("--binary", type=Path, default=Path("?"))
    parser.add_argument("--harness", type=Path, default=REPO,
                        help="the checkout whose compare.py the summary runs (default: this one)")
    parser.add_argument("--planned", nargs="*", default=list(STAGES))
    parser.add_argument("--statuses", nargs="*", default=[], help="<stage>=<run.py's exit code>")
    parser.add_argument("--seconds", type=int, default=0)
    parser.add_argument("--ladder-share", type=Path, metavar="LOG",
                        help="print the median ladder_budget_share of a layout log (else --default)")
    parser.add_argument("--default", type=float, default=0.695)
    arguments = parser.parse_args()
    if arguments.ladder_share is not None:
        share = ladder_share(arguments.ladder_share, load_compare(arguments.harness))
        print(f"{share if share is not None else arguments.default:g}")
        return
    nights = arguments.nights
    if nights is None:
        nights = (main_checkout() if (REPO / ".git").exists() else REPO) / "build/nightly"
    if arguments.summarize is not None:
        record = summarize(arguments.summarize, nights, arguments.sha, arguments.binary, arguments.harness,
                           arguments.planned, arguments.statuses, arguments.seconds)
        print(f"nightly {record['night']}: {guard_line(record, arguments.min_results)}")
        return
    if arguments.commit:
        commit(nights, arguments.checkout or main_checkout(), min_results=arguments.min_results)
        return
    night = arguments.docs
    if night is None:
        record = newest(nights)
        if record is None:
            sys.exit(f"no night with a nightly.json in {nights}")
        if arguments.print:
            print(table_row(record))
            print(state_line(record))
            return
        night = Path(nights) / record["night"]
    publish(arguments.checkout or REPO, night, min_results=arguments.min_results, dry_run=arguments.dry_run)


if __name__ == "__main__":
    main()
