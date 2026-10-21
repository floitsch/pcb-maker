#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Compares two layout sweeps board by board, on KiCad's connectivity.

    benchmarks/agent-tasks/compare.py <before.log> <after.log> [--splice <rerun.log> ...]

Each argument is a runner log (its JSON lines) or an output directory (its
results.json). A spliced log replaces the after-run's rows of the same
boards, so a board rerun on a newer binary counts with the sweep. The
table lists the boards of the after-run that the before-run has too,
sorted by the change of KiCad's unconnected count, then the boards only
one run has, then the totals: better / same / worse, unconnected in all,
passes. A row the wall clock cut short (its `cut` field: the layout's
deadline under load, the harness's timeout) depends on the machine's load,
not only on the binary: it is marked CUT and left out of better / same /
worse. A row copied from an earlier log (run.py --splice-from) is marked
SPL and counted in the totals. Below the totals, each run's continuous
objectives over the boards in both (README.md, "How to measure").

The strict rule (run.py's `pass_strict`: only the designer's findings at
the same place are subtracted) has its own column beside each cell
(`yes`, `no`, `-` for rows written before it) and its pass count beside
the pass count; the objectives add the strict new errors and the repairs
the binary reported (connection-width tracks and their length, island
stubs where counted, rows kept with the pours as tracks).

The first line names each run's task list (run.py's `tasks` and
`tasks_sha256` rows; for logs before them, the build/github-tasks-*
lists whose constraints every prepared board contains). When the lists
differ it says so in capitals. A board whose prepared constraints
(`constraints_sha256`, else `<run>/<board>/source/constraints.json`)
differ between the runs ran on another input: it is marked INPUT and left
out of better / same / worse, and the objectives are given again over the
boards with the same input (x768 on tasks v5 against x776 on v6 read as
174 more vias from three commits that changed no routing)."""

import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def rows_of(path):
    path = Path(path)
    if path.is_dir():
        results = path / "results.json"
        if results.exists():
            return json.loads(results.read_text())
        path = path.with_suffix(".log")
    rows = []
    for line in path.read_text().splitlines():
        if line.startswith("{"):
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return rows


def output_directory(path):
    """The run's output directory: the argument itself, or the log's name
    without `.log` (run.py's layout: build/quick30-x776.log next to
    build/quick30-x776/)."""
    path = Path(path)
    return path if path.is_dir() else path.with_suffix("")


def constraints_digest(constraints):
    """sha256 of a constraints object, independent of key order and
    formatting (run.py's `constraints_sha256`)."""
    return hashlib.sha256(json.dumps(constraints, sort_keys=True).encode()).hexdigest()


def prepared_constraints(directory, name):
    try:
        return json.loads((directory / name / "source" / "constraints.json").read_text())
    except (OSError, ValueError):
        return None


def input_digest(row, directory):
    """The prepared constraints' digest of a layout row: its own field, or
    computed from the run's output directory; None for route rows and when
    neither is there."""
    if row is None or row.get("mode") == "route":
        return None
    if row.get("constraints_sha256"):
        return row["constraints_sha256"]
    constraints = prepared_constraints(directory, row["name"])
    return constraints_digest(constraints) if constraints is not None else None


def contains(task, prepared):
    """Whether run.py could have prepared `prepared` from the task's
    constraints: it only adds fixed parts (the netless footprints)."""
    return (set(task.get("fixed", [])) <= set(prepared.get("fixed", []))
            and all(task.get(key) == prepared.get(key) for key in task if key != "fixed"))


def candidate_lists(path):
    found = []
    for base in (output_directory(path).resolve().parent, ROOT / "build"):
        for candidate in sorted(base.glob("github-tasks-*/tasks.json")):
            if candidate.resolve() not in [c.resolve() for c in found]:
                found.append(candidate)
    return found


def task_list(path, rows):
    """(label, identity) of the run's task list. Recorded (the rows' `tasks`
    and `tasks_sha256`): ("recorded", digest, {list directory}); else
    inferred from the prepared constraints: ("inferred", None, {the list
    directories every prepared board fits}); None when unknown."""
    recorded = {(row.get("tasks"), row.get("tasks_sha256")) for row in rows
                if row.get("tasks") and not row.get("spliced")}
    if len(recorded) == 1:
        label, digest = next(iter(recorded))
        return f"{label} (sha256 {(digest or '?')[:12]})", ("recorded", digest, {Path(label).parent.name})
    if len(recorded) > 1:
        labels = ", ".join(sorted(str(label) for label, _ in recorded))
        return f"several: {labels}", ("recorded", "several:" + labels, {Path(label).parent.name for label, _ in recorded})
    directory = output_directory(path)
    prepared = {row["name"]: prepared_constraints(directory, row["name"]) for row in rows
                if row.get("mode") != "route"}
    prepared = {name: value for name, value in prepared.items() if value is not None}
    if not prepared:
        return "unknown (no tasks field, no prepared constraints)", None
    matching = []
    for candidate in candidate_lists(path):
        try:
            tasks = {task["name"]: task for task in json.loads(candidate.read_text())["tasks"]}
        except (OSError, ValueError, KeyError):
            continue
        ok = True
        for name, value in prepared.items():
            task = tasks.get(name)
            try:
                own = json.loads((candidate.parent / task["constraints"]).read_text()) if task else None
            except (OSError, ValueError):
                own = None
            if own is None or not contains(own, value):
                ok = False
                break
        if ok:
            matching.append(candidate)
    if len(matching) == 1:
        label = f"build/{matching[0].parent.name}/tasks.json"
        return f"{label} (inferred from {len(prepared)} prepared boards)", ("inferred", None, {matching[0].parent.name})
    if matching:
        names = ", ".join(c.parent.name for c in matching)
        return (f"no tasks field; one of {names} (inferred from {len(prepared)} prepared boards)",
                ("inferred", None, {c.parent.name for c in matching}))
    return "unknown (no tasks field; no build/github-tasks-* list matches)", None


def lists_differ(before, after):
    """Whether two task lists (task_list's identities) are known to differ:
    both recorded with other digests, or no list directory both could be."""
    if before is None or after is None:
        return False
    if before[0] == "recorded" and after[0] == "recorded":
        return before[1] != after[1]
    return not (before[2] & after[2])


def by_name(rows):
    table = {}
    for row in rows:
        table[row["name"]] = row  # the last row of a board wins
    return table


def unconnected(row):
    native = row.get("native")
    if row.get("error") or native is None:
        return None
    return native.get("unconnected")


def errors(row):
    native = row.get("native") or {}
    return sum((native.get("errors") or {}).values())


def passes(row):
    """The pass rule of the runner (KiCad's connectivity, no copper error,
    no unexpected constraint miss), applied again: rows written before the
    rule changed carry the old flag."""
    if row is None or row.get("error") or row.get("native") is None:
        return False
    return unconnected(row) == 0 and not errors(row) and not row.get("unexpected_misses")


def passes_strict(row):
    """The strict rule's pass (run.py's `pass_strict`, recomputed as
    `passes`): None for a row written before the rule."""
    if row is None or "new_findings_strict" not in row and "pass_strict" not in row:
        return None
    if row.get("error") or row.get("native") is None or "new_findings_strict" not in row:
        return False
    return unconnected(row) == 0 and not row["new_findings_strict"] and not row.get("unexpected_misses")


def strict_errors(row):
    return sum((row.get("new_findings_strict") or {}).values())


def strict_cell(row):
    verdict = passes_strict(row)
    return "-" if verdict is None else "yes" if verdict else "no"


def cut(row):
    """Whether the wall clock, not the work, ended part of the run."""
    return row is not None and bool(row.get("cut"))


def spliced(row):
    return row is not None and bool(row.get("spliced"))


def objectives(rows):
    """Continuous measures over `rows`, usable from one run: KiCad's
    unconnected items and new copper errors, vias, length, CPU and work
    seconds, rows cut by the wall clock, and the router's unconnected
    terminals after the first route (layouts only). Rows without a result
    (an error, a timeout) add only to `failed`."""
    results = [row for row in rows if unconnected(row) is not None]
    with_work = [row for row in rows if row.get("work_seconds") is not None]
    with_first = [row for row in rows if row.get("first_unconnected_terminals") is not None]
    with_strict = [row for row in rows if passes_strict(row) is not None]
    with_repairs = [row["repairs"] for row in rows if row.get("repairs")]
    stubs = [repairs["island_stubs"] for repairs in with_repairs if repairs.get("island_stubs") is not None]
    return {
        "rows": len(rows),
        "passes": sum(1 for row in rows if passes(row)),
        "strict_rows": len(with_strict),
        "passes_strict": sum(1 for row in with_strict if passes_strict(row)),
        "new_errors_strict": sum(strict_errors(row) for row in with_strict if unconnected(row) is not None),
        "repair_rows": len(with_repairs),
        "neck_tracks": sum(repairs.get("neck_tracks") or 0 for repairs in with_repairs),
        "neck_track_mm": round(sum(repairs.get("neck_track_mm") or 0 for repairs in with_repairs), 2),
        "island_stubs": sum(stubs) if stubs else None,
        "pours_as_tracks": sum(1 for repairs in with_repairs if repairs.get("pours_as_tracks")),
        "failed": len(rows) - len(results),
        "unconnected": sum(unconnected(row) for row in results),
        "new_errors": sum(errors(row) for row in results),
        "vias": sum(row.get("vias") or 0 for row in results),
        "length_mm": round(sum(row.get("length_mm") or 0 for row in results), 1),
        "cpu_seconds": round(sum(row.get("cpu_seconds") or 0 for row in rows)),
        "work_seconds": round(sum(row["work_seconds"] for row in with_work)),
        "work_rows": len(with_work),
        "cut": sum(1 for row in rows if cut(row)),
        "first_unconnected_terminals": sum(row["first_unconnected_terminals"] for row in with_first),
        "first_rows": len(with_first),
    }


def objectives_line(o, label="objectives"):
    text = f"{label}: unconnected {o['unconnected']}, new errors {o['new_errors']}"
    if o.get("strict_rows"):
        text += f" (strict {o['new_errors_strict']}"
        text += f", {o['strict_rows']} rows)" if o["strict_rows"] != o["rows"] else ")"
    text += f", vias {o['vias']}, length {o['length_mm']:.0f} mm, CPU {o['cpu_seconds']} s"
    if o["work_rows"]:
        text += f", work {o['work_seconds']} s"
        if o["work_rows"] != o["rows"]:
            text += f" ({o['work_rows']} rows)"
    text += f", cut {o['cut']}"
    if o["first_rows"]:
        text += f", first-route open terminals {o['first_unconnected_terminals']}"
        if o["first_rows"] != o["rows"]:
            text += f" ({o['first_rows']} rows)"
    if o.get("repair_rows"):
        stubs = "not counted" if o["island_stubs"] is None else o["island_stubs"]
        text += (f", repairs: {o['neck_tracks']} neck tracks ({o['neck_track_mm']:.2f} mm), island stubs {stubs}, "
                 f"{o['pours_as_tracks']} kept with pours as tracks")
        if o["repair_rows"] != o["rows"]:
            text += f" ({o['repair_rows']} rows)"
    text += f"; {o['rows']} rows"
    if o["failed"]:
        text += f", {o['failed']} without a result"
    return text


def cell(row):
    if row is None:
        return "-"
    if row.get("error"):
        text = f"ERROR ({str(row['error'])[:30]})"
        text += " CUT" if cut(row) else ""
        return text + " SPL" if spliced(row) else text
    text = f"{row.get('routed', '?')} {unconnected(row)}"
    if errors(row):
        text += f" +{errors(row)}e"
    if passes(row):
        text += " pass"
    if cut(row):
        text += " CUT"
    if spliced(row):
        text += " SPL"
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before")
    parser.add_argument("after")
    parser.add_argument("--splice", nargs="*", default=[])
    parser.add_argument("--markdown", action="store_true")
    arguments = parser.parse_args()
    before_rows, after_rows = rows_of(arguments.before), rows_of(arguments.after)
    before = by_name(before_rows)
    after = by_name(after_rows)
    after_directory = {name: output_directory(arguments.after) for name in after}
    for path in arguments.splice:
        spliced_in = by_name(rows_of(path))
        after.update(spliced_in)
        after_directory.update({name: output_directory(path) for name in spliced_in})
    common = sorted(set(before) & set(after))
    before_list, before_identity = task_list(arguments.before, before_rows)
    after_list, after_identity = task_list(arguments.after, after_rows)
    # Boards prepared from other constraints: another input, not a change.
    other_input = set()
    for name in common:
        digest_before = input_digest(before[name], output_directory(arguments.before))
        digest_after = input_digest(after[name], after_directory[name])
        if digest_before is not None and digest_after is not None and digest_before != digest_after:
            other_input.add(name)
    lists = f"before {before_list}; after {after_list}"
    if lists_differ(before_identity, after_identity):
        print(f"TASK LISTS DIFFER, {len(other_input)} BOARDS ON OTHER INPUTS (INPUT): {lists}")
    elif other_input:
        print(f"INPUTS DIFFER ON {len(other_input)} BOARDS (INPUT: their prepared constraints differ): {lists}")
    else:
        print(f"tasks: {lists}")

    def delta(name):
        a, b = unconnected(after[name]), unconnected(before[name])
        if a is None or b is None:
            return float("inf")
        return a - b

    common.sort(key=lambda n: (delta(n), n))
    width = max((len(n) for n in list(before) + list(after)), default=10)
    sep = " | " if arguments.markdown else "  "
    print(f"{'board':{width}}{sep}{'before':>24}{sep}{'strict':>6}{sep}{'after':>24}{sep}{'strict':>6}{sep}{'s':>6}")
    if arguments.markdown:
        print(f"{'-' * width} | {'-' * 24} | {'-' * 6} | {'-' * 24} | {'-' * 6} | {'-' * 6}")
    better = same = worse = cut_rows = input_rows = 0
    total_before = total_after = 0
    for name in common:
        b, a = before[name], after[name]
        ub, ua = unconnected(b), unconnected(a)
        if ub is not None and ua is not None:
            total_before += ub
            total_after += ua
            if name in other_input:
                input_rows += 1
            elif cut(a) or cut(b):
                cut_rows += 1
            elif ua < ub:
                better += 1
            elif ua > ub:
                worse += 1
            else:
                same += 1
        after_cell = cell(a) + (" INPUT" if name in other_input else "")
        print(f"{name:{width}}{sep}{cell(b):>24}{sep}{strict_cell(b):>6}{sep}{after_cell:>24}{sep}"
              f"{strict_cell(a):>6}{sep}{round(a.get('seconds', 0)):>6}")
    only_after = sorted(set(after) - set(before))
    only_before = sorted(set(before) - set(after))
    for name in only_after:
        a = after[name]
        print(f"{name:{width}}{sep}{'-':>24}{sep}{'-':>6}{sep}{cell(a):>24}{sep}{strict_cell(a):>6}{sep}"
              f"{round(a.get('seconds', 0)):>6}")
    for name in only_before:
        print(f"{name:{width}}{sep}{cell(before[name]):>24}{sep}{strict_cell(before[name]):>6}{sep}{'-':>24}{sep}"
              f"{'-':>6}{sep}{'':>6}")
    passes_before = sum(1 for n in common if passes(before[n]))
    passes_after = sum(1 for n in common if passes(after[n]))

    def strict_count(table):
        known = [n for n in common if passes_strict(table[n]) is not None]
        if not known:
            return "-"
        count = sum(1 for n in known if passes_strict(table[n]))
        return f"{count}" + (f" of {len(known)} rows" if len(known) != len(common) else "")

    print()
    print(f"{len(common)} boards in both: better {better}, same {same}, worse {worse}; "
          f"unconnected {total_before} -> {total_after}; passes {passes_before} -> {passes_after} "
          f"(strict {strict_count(before)} -> {strict_count(after)})"
          + (f"; {cut_rows} cut by the wall clock in either run (not in better/same/worse)" if cut_rows else "")
          + (f"; {input_rows} on other inputs (INPUT, not in better/same/worse)" if input_rows else ""))
    for label, table in (("before", before), ("after", after)):
        count = sum(1 for n in common if spliced(table[n]))
        if count:
            print(f"{label}: {count} of the {len(common)} rows spliced from an earlier run (SPL)")
    print(objectives_line(objectives([before[n] for n in common]), "before"))
    print(objectives_line(objectives([after[n] for n in common]), "after "))
    if other_input:
        same_input = [n for n in common if n not in other_input]
        print(f"over the {len(same_input)} boards with the same input:")
        print(objectives_line(objectives([before[n] for n in same_input]), "before"))
        print(objectives_line(objectives([after[n] for n in same_input]), "after "))
    if only_after:
        print(f"{len(only_after)} boards only after: {sum(1 for n in only_after if passes(after[n]))} pass")
    if only_before:
        print(f"{len(only_before)} boards only before")


if __name__ == "__main__":
    main()
