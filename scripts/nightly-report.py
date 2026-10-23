#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""The nightly's summary and its report into the docs (scripts/nightly.sh;
benchmarks/agent-tasks/README.md, "Nightly").

    scripts/nightly-report.py                     # the newest night into this checkout's docs
    scripts/nightly-report.py --print             # only print the table and the state line
    scripts/nightly-report.py --commit            # into main's docs, committed and pushed
    scripts/nightly-report.py --summarize <night> --nights build/nightly ...   # nightly.sh's last step
    scripts/nightly-report.py --ladder-share <sweep.log> --default 0.695      # the designer tier's scale

`--summarize` reads a night's stages (`<night>/<stage>.log`, the rows, and
`<night>/<stage>/summary.json`, run.py's summary) and writes
`<night>/nightly.json` and `<night>/summary.md`: pass and strict counts,
the medians against the designer's boards, the gap split (compare.py --gap
--targets, also `gap.txt`), the cut and killed rows, the fingerprints, and
the diff against the previous nightly that ran each stage (compare.py,
also `diff-<stage>.txt`: better, same, worse, and the boards whose pass
status changed).

The report turns the newest night into a row of the table between the
`nightly:begin` and `nightly:end` markers of docs/benchmarks.md
("Nightly", newest first, the last 14 nights) and the line between the
`nightly-state` markers of docs/handover.md. `--commit` does that in the
main checkout, which must be on main with neither file changed, commits
those two files alone with a fixed message and pushes main; it touches
nothing else."""

import argparse
import hashlib
import json
import re
import subprocess
import sys
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


def write_docs(checkout, record):
    benchmarks, handover = Path(checkout) / "docs/benchmarks.md", Path(checkout) / "docs/handover.md"
    benchmarks.write_text(merge_table(benchmarks.read_text(), table_row(record)))
    handover.write_text(merge_state(handover.read_text(), state_line(record)))


def commit(record, checkout):
    """The night into `checkout`'s two docs (the main checkout's),
    committed alone on main and pushed to origin."""
    files = ["docs/benchmarks.md", "docs/handover.md"]
    branch = git(checkout, "rev-parse", "--abbrev-ref", "HEAD").stdout.strip()
    if branch != "main":
        sys.exit(f"--commit: {checkout} is on {branch}, not main")
    if git(checkout, "status", "--porcelain", "--", *files).stdout.strip():
        sys.exit(f"--commit: {', '.join(files)} have uncommitted changes in {checkout}; not touching them")
    write_docs(checkout, record)
    if not git(checkout, "status", "--porcelain", "--", *files).stdout.strip():
        print("the docs already have this night")
        return
    message = f"Docs: nightly {record['night']} (main {record['sha'][:12]})"
    # The paths given: only these two files go into the commit, whatever
    # else is staged.
    git(checkout, "commit", "--quiet", "-m", message, "--", *files)
    print(f"committed on main: {message}")
    pushed = git(checkout, "push", "--quiet", "origin", "main", check=False)
    if pushed.returncode:
        sys.exit(f"git push origin main failed (the commit stays on main): {pushed.stderr.strip()}")
    print("pushed main")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--nights", type=Path, default=None,
                        help="the nights' directory (default: build/nightly of the main checkout)")
    parser.add_argument("--print", action="store_true", help="print the table row and the state line only")
    parser.add_argument("--commit", action="store_true",
                        help="write the main checkout's docs, commit the two files on main and push")
    parser.add_argument("--docs", type=Path, default=REPO, help="the checkout whose docs to write (default: this one)")
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
        summarize(arguments.summarize, nights, arguments.sha, arguments.binary, arguments.harness,
                  arguments.planned, arguments.statuses, arguments.seconds)
        return
    record = newest(nights)
    if record is None:
        sys.exit(f"no night with a nightly.json in {nights}")
    if arguments.print:
        print(table_row(record))
        print(state_line(record))
    elif arguments.commit:
        commit(record, main_checkout())
    else:
        write_docs(arguments.docs, record)
        print(f"wrote {record['night']} into {arguments.docs}/docs/benchmarks.md and docs/handover.md")


if __name__ == "__main__":
    main()
