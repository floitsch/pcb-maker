#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Compares two layout sweeps board by board, on KiCad's connectivity.

    benchmarks/agent-tasks/compare.py <before.log> <after.log> [--splice <rerun.log> ...] [--score BINARY]
    benchmarks/agent-tasks/compare.py --gap <layout.log> <designer.log>

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
objectives over the boards in both (README.md, "How to measure"): the raw
sums, then the per-board measures against the designer's board as
medians over the boards (normalised: vias and length over the designer's,
from designer-numbers.json or the rows' own scores; KiCad's unconnected items over
the board's connections; the ground pours' cut where rows carry scores,
run.py --quality).

The strict rule (run.py's `pass_strict`: only the designer's findings at
the same place are subtracted) has its own column beside each cell
(`yes`, `no`, `-` for rows written before it) and its pass count beside
the pass count; the objectives add the strict new errors and the repairs
the binary reported (connection-width tracks and their length, island
stubs where counted, rows kept with the pours as tracks).

Rows record the strict rule's version they were judged under
(`strict_rule`; rows without it are rule 1): when the two runs' rows
differ, a line in capitals says so before the table. Rule 2 does not count
the solder mask bridges of a designer's opening that already bridges
(`strict_mask_openings`, summed in the objectives).

The first line names each run's task list (run.py's `tasks` and
`tasks_sha256` rows; for logs before them, the build/github-tasks-*
lists whose constraints every prepared board contains). When the lists
differ it says so in capitals. A board whose prepared constraints
(`constraints_sha256`, else `<run>/<board>/source/constraints.json`)
differ between the runs ran on another input: it is marked INPUT and left
out of better / same / worse, and the objectives are given again over the
boards with the same input (x768 on tasks v5 against x776 on v6 read as
174 more vias from three commits that changed no routing).

`--gap` joins a layout run with a designer-tier run (run.py --tier
designer: the same boards routed on the designer's placement) and prints
per board the placement gap (layout open less designer-route open) and the
routing gap (designer-route open), with corpus totals (README.md, "The
designer tier"); `--targets` leaves out the boards that are not targets.
It names the routing work of both sides where the rows record it.

A row whose binary was killed (run.py's `killed`; in older logs an exit
code that is none of the verdict's, 0 to 4) has no result: KILLED, a
failure, out of better / same / worse. Rows without a result count as
failures in the strict count too. Each run's strict rules are named
("strict: before rule 2; after rules 1 (105 rows), 3 (4 rows)"), and a
strict count over several rules is marked MIXED RULES."""

import argparse
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))


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


def task_digest(task, constraints):
    """sha256 of a task's entry as run.py reads it (run.py's `task_sha256`):
    every field but where the board lies (`directory`) and a tier's
    estimates, with its constraints (constraints_digest) in place of the
    constraint file's name. tasks.json alone does not tell the lists apart:
    github-tasks-v2 to v7 have the same tasks.json (sha256 c8f95caf5089)
    and differ only in their constraint files."""
    entry = {key: value for key, value in task.items()
             if key not in ("directory", "peak_rss_mb", "expected_seconds")}
    if "constraints" in entry:
        entry["constraints"] = constraints_digest(constraints) if constraints is not None else None
    return hashlib.sha256(json.dumps(entry, sort_keys=True).encode()).hexdigest()


PROJECT_EXTENSIONS = (".kicad_pro", ".kicad_dru")


def project_files_digest(paths):
    """sha256 of a board's project files (its .kicad_pro and .kicad_dru,
    in that order; run.py's `project_sha256`): the rules KiCad's DRC and
    the router read, which `board_sha256` leaves out. A .kicad_dru that
    appears changes it (the OpenESC boards on 2026-10-24: re-fetched with
    their custom rules, rows before and after compared as one input)."""
    digest = hashlib.sha256()
    for path in sorted(paths, key=lambda path: (PROJECT_EXTENSIONS.index(path.suffix), path.name)):
        digest.update(path.suffix.encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def project_digest(row, directory):
    """The project files' digest of a row: its own field, else computed
    from the copies in the run's prepared source (layout rows); None when
    neither is there."""
    if row is None:
        return None
    if row.get("project_sha256"):
        return row["project_sha256"]
    source = Path(directory) / row["name"] / "source"
    paths = [path for extension in PROJECT_EXTENSIONS for path in source.glob(f"*{extension}")]
    if not any(path.suffix == ".kicad_pro" for path in paths):
        return None
    return project_files_digest(paths)


def other_inputs(before, after):
    """The fields that show two rows of a board ran on other inputs: the
    task's entry (`task_sha256`) or the designer's board (`board_sha256`),
    where both rows record them."""
    return [key for key in ("task_sha256", "board_sha256")
            if before.get(key) and after.get(key) and before[key] != after[key]]


def on_other_input(before, after, before_directory, after_directory):
    """Whether two rows of a board ran on other inputs (INPUT): their
    prepared constraints, task entry, designer's board or project files
    (.kicad_pro, .kicad_dru) differ, where both rows (or their runs'
    output directories) say. Such a pair is not a change of the binary."""
    digest_before = input_digest(before, before_directory)
    digest_after = input_digest(after, after_directory)
    if digest_before is not None and digest_after is not None and digest_before != digest_after:
        return True
    if other_inputs(before, after):
        return True
    project_before = project_digest(before, before_directory)
    project_after = project_digest(after, after_directory)
    return project_before is not None and project_after is not None and project_before != project_after


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
    for base, pattern in ((output_directory(path).resolve().parent, "github-tasks-*/tasks.json"),
                          (ROOT / "build", "github-tasks-*/tasks.json"),
                          (ROOT / "benchmarks/agent-tasks/tasks", "*/tasks.json")):
        for candidate in sorted(base.glob(pattern)):
            if candidate.resolve() not in [c.resolve() for c in found]:
                found.append(candidate)
    return found


def list_name(label):
    """The list a task-list path names, the same for build/github-tasks-v7
    and its committed copy benchmarks/agent-tasks/tasks/v7."""
    parent = Path(label).parent
    if parent.parent.name == "tasks" and parent.parent.parent.name == "agent-tasks":
        return f"github-tasks-{parent.name}"
    return parent.name


def task_list(path, rows):
    """(label, identity) of the run's task list. Recorded (the rows' `tasks`
    and `tasks_sha256`): ("recorded", digest, {list directory}); else
    inferred from the prepared constraints: ("inferred", None, {the list
    directories every prepared board fits}); None when unknown."""
    recorded = {(row.get("tasks"), row.get("tasks_sha256")) for row in rows
                if row.get("tasks") and not row.get("spliced")}
    if len(recorded) == 1:
        label, digest = next(iter(recorded))
        return f"{label} (sha256 {(digest or '?')[:12]})", ("recorded", digest, {list_name(label)})
    if len(recorded) > 1:
        labels = ", ".join(sorted(str(label) for label, _ in recorded))
        return f"several: {labels}", ("recorded", "several:" + labels, {list_name(label) for label, _ in recorded})
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
        label = str(matching[0].relative_to(ROOT)) if matching[0].is_relative_to(ROOT) else str(matching[0])
        return (f"{label} (inferred from {len(prepared)} prepared boards)",
                ("inferred", None, {list_name(matching[0])}))
    if matching:
        names = ", ".join(sorted({list_name(c) for c in matching}))
        return (f"no tasks field; one of {names} (inferred from {len(prepared)} prepared boards)",
                ("inferred", None, {list_name(c) for c in matching}))
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


def killed(row):
    """Whether a row's binary was killed (run.py's `killed`; in older logs
    an exit code that is none of the verdict's, 0 to 4: the OOM killer's
    -9 left fr2-route-x1367's bumwings_v001_core and ghoul and
    fr2-route-x1371-r2's katia with the best attempt so far, judged as a
    result). Such a row has no result: a failure, never a pass, and out of
    better / same / worse."""
    if row is None:
        return False
    if row.get("killed"):
        return True
    code = row.get("exit")
    return isinstance(code, int) and not isinstance(code, bool) and not 0 <= code <= 4


def unconnected(row):
    native = row.get("native")
    if row.get("error") or native is None or killed(row):
        return None
    return native.get("unconnected")


def errors(row):
    native = row.get("native") or {}
    return sum((native.get("errors") or {}).values())


def passes(row):
    """The pass rule (KiCad's connectivity, no copper error beyond the
    designer's, no unexpected constraint miss): run.py sets each row's
    `pass` with it, and compare.py applies it again, since rows written
    before the rule changed carry the old flag."""
    if row is None or unconnected(row) is None:
        return False
    # The schematic tier also needs every footprint found (`missing`).
    return unconnected(row) == 0 and not errors(row) and not row.get("unexpected_misses") and not row.get("missing")


def judged_strictly(row):
    """Whether a row carries the strict rule's judgement or a run's
    verdict under it (`pass_strict`, `new_findings_strict`)."""
    return row is not None and ("new_findings_strict" in row or "pass_strict" in row)


def passes_strict(row):
    """The strict rule's pass (run.py sets each row's `pass_strict` with
    it, compare.py applies it again as `passes`): None for a row with a
    result written before the rule. A row without a result (an error, a
    timeout, a kill) fails under every rule: False."""
    if row is None:
        return None
    if unconnected(row) is None:
        return False
    if "new_findings_strict" not in row:
        return False if "pass_strict" in row else None
    return (unconnected(row) == 0 and not row["new_findings_strict"] and not row.get("unexpected_misses")
            and not row.get("missing"))


def strict_errors(row):
    return sum((row.get("new_findings_strict") or {}).values())


def strict_rule(row):
    """The strict rule's version a row was judged under (run.py's
    STRICT_RULE; rows of rule 1 do not say): None for a row without it."""
    if row is None or "new_findings_strict" not in row:
        return None
    return row.get("strict_rule", 1)


def strict_rules(rows):
    """The strict rules a run's rows were judged under, sorted."""
    return sorted({rule for rule in map(strict_rule, rows) if rule is not None})


def rule_mix(rows):
    """{rule: rows judged under it} of a run (strict_rule)."""
    mix = {}
    for rule in map(strict_rule, rows):
        if rule is not None:
            mix[rule] = mix.get(rule, 0) + 1
    return mix


def rule_mix_text(rows):
    """A run's strict rules with their rows ("rule 3" or "rules 1 (105
    rows), 3 (4 rows)"), None without a strict judgement."""
    mix = rule_mix(rows)
    if not mix:
        return None
    if len(mix) == 1:
        return f"rule {next(iter(mix))}"
    return "rules " + ", ".join(f"{rule} ({count} rows)" for rule, count in sorted(mix.items()))


def rules_warning(before_rows, after_rows):
    """A line when two runs were judged under different strict rules (or one
    run under several): their pass_strict and strict errors then differ by
    the rule as well. None when they agree."""
    before, after = strict_rules(before_rows), strict_rules(after_rows)
    if not before or not after or (before == after and len(before) == 1):
        return None
    text = "/".join(map(str, before)) + " before, " + "/".join(map(str, after)) + " after"
    return (f"STRICT RULES DIFFER ({text}): the strict columns compare judgements as well as boards "
            "(rule 2 does not count solder mask bridges of a designer's opening that bridges already, "
            "strict_mask_openings)")


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
    # The strict count: the rows judged by it and, where any is, the rows
    # without a result (failures under every rule).
    with_strict = [row for row in rows if passes_strict(row) is not None] \
        if any(judged_strictly(row) for row in rows) else []
    with_repairs = [row["repairs"] for row in rows if row.get("repairs")]
    stubs = [repairs["island_stubs"] for repairs in with_repairs if repairs.get("island_stubs") is not None]
    return {
        "rows": len(rows),
        "passes": sum(1 for row in rows if passes(row)),
        "strict_rows": len(with_strict),
        "passes_strict": sum(1 for row in with_strict if passes_strict(row)),
        "new_errors_strict": sum(strict_errors(row) for row in with_strict if unconnected(row) is not None),
        "strict_mask_openings": sum(row.get("strict_mask_openings") or 0 for row in with_strict),
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


DESIGNER_NUMBERS = Path(__file__).resolve().parent / "designer-numbers.json"
_DESIGNER = {}


def designer_numbers(path=DESIGNER_NUMBERS):
    """designer-numbers.json's boards (quality.py --designer-numbers): the
    designer's vias, track length and ground pour cut per task."""
    key = str(path)
    if key not in _DESIGNER:
        try:
            _DESIGNER[key] = json.loads(Path(path).read_text())["boards"]
        except (OSError, ValueError, KeyError):
            _DESIGNER[key] = {}
    return _DESIGNER[key]


def connections(row):
    """The board's connections (the denominator of the row's `routed`)."""
    try:
        return int(str(row.get("routed", "")).split("/")[1])
    except (IndexError, ValueError):
        return None


def designer_of(row, numbers):
    """The designer's numbers for a row: its own scores (run.py --quality),
    else designer-numbers.json's when they are of the same board."""
    quality = row.get("quality") or {}
    if quality.get("designer"):
        return quality["designer"]
    entry = numbers.get(row["name"])
    if entry is None:
        return None
    if row.get("board_sha256") and entry.get("board_sha256") and row["board_sha256"] != entry["board_sha256"]:
        return None
    return entry


def median(values):
    ordered = sorted(values)
    if not ordered:
        return None
    middle = len(ordered) // 2
    return ordered[middle] if len(ordered) % 2 else (ordered[middle - 1] + ordered[middle]) / 2


def normalised(rows, numbers=None):
    """Per-board measures against the designer's board, as medians over the
    boards (README.md, "How to measure"): our vias over the designer's,
    our track length over the designer's, KiCad's unconnected items over
    the board's connections, and, for rows with scores (run.py --quality),
    the ground pours' cut by other tracks, ours less the designer's (mm),
    and the boards whose designer has a ground pour that ours lost, and
    the aesthetics (score-kicad-board's `aesthetics`): our bends per
    connection over the designer's, our stair-steps and pin-row crossings
    less the designer's. Rows
    without a result are left out; a ratio needs the designer's number
    above zero."""
    numbers = designer_numbers() if numbers is None else numbers
    vias, length, open_share, ground, lost = [], [], [], [], 0
    bends, stairs, crossings = [], [], []
    results = [row for row in rows if unconnected(row) is not None]
    for row in results:
        total = connections(row)
        if total:
            open_share.append(unconnected(row) / total)
        designer = designer_of(row, numbers)
        if not designer:
            continue
        if designer.get("vias") and row.get("vias") is not None:
            vias.append(row["vias"] / designer["vias"])
        if designer.get("track_length_mm") and row.get("length_mm") is not None:
            length.append(row["length_mm"] / designer["track_length_mm"])
        ours = (row.get("quality") or {}).get("ours")
        if ours is not None and designer.get("ground_cut_mm") is not None:
            if ours.get("ground_cut_mm") is None:
                lost += 1
            else:
                ground.append(ours["ground_cut_mm"] - designer["ground_cut_mm"])
        if ours is not None:
            if designer.get("bends_per_connection") and ours.get("bends_per_connection") is not None:
                bends.append(ours["bends_per_connection"] / designer["bends_per_connection"])
            for key, values in (("stair_steps", stairs), ("pin_row_crossings", crossings)):
                if designer.get(key) is not None and ours.get(key) is not None:
                    values.append(ours[key] - designer[key])

    def rounded(value, digits):
        return None if value is None else round(value, digits)

    return {"boards": len(results),
            "vias_ratio": rounded(median(vias), 3), "vias_boards": len(vias),
            "length_ratio": rounded(median(length), 3), "length_boards": len(length),
            "open_share": rounded(median(open_share), 4),
            "open_share_mean": rounded(sum(open_share) / len(open_share), 4) if open_share else None,
            "open_boards": len(open_share),
            "ground_cut_delta_mm": rounded(median(ground), 1), "ground_boards": len(ground),
            "ground_pour_lost": lost,
            "bends_ratio": rounded(median(bends), 3), "bends_boards": len(bends),
            "stair_steps_delta": rounded(median(stairs), 1),
            "pin_row_crossings_delta": rounded(median(crossings), 1)}


def result_directory(row, directory):
    """Where a row's result board lies in its run's output directory."""
    name = directory / row["name"]
    return name / "route" if row.get("mode") == "route" else name / "layout" / "result"


def add_scores(rows, directory, binary):
    """Scores the result boards of rows without scores (score-kicad-board,
    about 0.1 s a board) into their `quality.ours` (quality.py's measures);
    the designer's side comes from designer-numbers.json."""
    import quality
    for row in rows:
        if unconnected(row) is None or (row.get("quality") or {}).get("ours"):
            continue
        result = result_directory(row, directory)
        boards = sorted(result.glob("*.kicad_pcb")) if result.is_dir() else []
        if not boards:
            continue
        report = quality.score(binary, result, boards[0].stem)
        if report:
            row["quality"] = {**(row.get("quality") or {}), "ours": quality.measures(report)}


def normalised_line(n, label="normalised"):
    def ratio(value):
        return "-" if value is None else f"x{value:.2f}"

    text = (f"{label}: medians over the boards against the designer's: vias {ratio(n['vias_ratio'])} "
            f"({n['vias_boards']} boards), length {ratio(n['length_ratio'])} ({n['length_boards']}), open ")
    text += "-" if n["open_share"] is None else (f"{n['open_share']:.3f} of the connections (mean "
                                                 f"{n['open_share_mean']:.3f}, {n['open_boards']} boards)")
    if n["ground_boards"] or n["ground_pour_lost"]:
        delta = n["ground_cut_delta_mm"]
        text += (", ground pour cut " + ("-" if delta is None else f"{delta:+.0f} mm") +
                 f" against the designer's ({n['ground_boards']} boards; {n['ground_pour_lost']} lost the pour)")
    else:
        text += ", ground pour cut - (no scores: run.py --quality or compare.py --score)"
    if n.get("bends_boards"):
        text += (f", bends per connection {ratio(n['bends_ratio'])} ({n['bends_boards']} boards), stair-steps "
                 f"{n['stair_steps_delta']:+g}, pin-row crossings {n['pin_row_crossings_delta']:+g} against the "
                 "designer's")
    return text


def objectives_line(o, label="objectives"):
    text = f"{label}: unconnected {o['unconnected']}, new errors {o['new_errors']}"
    if o.get("strict_rows"):
        text += f" (strict {o['new_errors_strict']}"
        if o.get("strict_mask_openings"):
            text += f", {o['strict_mask_openings']} mask-opening bridges not counted"
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
    if killed(row) and not row.get("killed"):
        # An older log's killed row: no `error`, its files judged.
        text = f"KILLED (exit {row.get('exit')})"
        return text + " SPL" if spliced(row) else text
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


def gap_rows(layout, designer):
    """The placement and routing gap per board (`--gap`): `layout` and
    `designer` map names to rows of a layout run and of a designer-tier
    run. Per board in both: KiCad's unconnected items of the layout
    (`layout`), of the route on the designer's placement (`designer`),
    the placement gap (layout less designer: what our placement adds) and
    the routing gap (designer: what our router leaves on a human's
    placement); None where a run has no result (a placement that failed,
    a timeout)."""
    table = []
    for name in sorted(set(layout) & set(designer)):
        ours, theirs = unconnected(layout[name]), unconnected(designer[name])
        table.append({"name": name, "connections": connections(designer[name]) or connections(layout[name]),
                      "layout": ours, "designer": theirs,
                      "placement_gap": None if ours is None or theirs is None else ours - theirs,
                      "routing_gap": theirs,
                      "cut": cut(layout[name]) or cut(designer[name]),
                      "layout_error": (layout[name].get("error") or "")[:60] if ours is None else None})
    return table


def gap_totals(table):
    """Corpus totals of gap_rows over the boards where both runs have a
    result, and the boards where the layout has none."""
    both = [entry for entry in table if entry["placement_gap"] is not None]
    layout_open = sum(entry["layout"] for entry in both)
    return {"boards": len(table), "both": len(both),
            "layout_open": layout_open, "designer_open": sum(entry["designer"] for entry in both),
            "placement_gap": sum(entry["placement_gap"] for entry in both),
            "routing_gap": sum(entry["routing_gap"] for entry in both),
            "placement_worse": sum(1 for entry in both if entry["placement_gap"] > 0),
            "placement_same": sum(1 for entry in both if entry["placement_gap"] == 0),
            "placement_better": sum(1 for entry in both if entry["placement_gap"] < 0),
            "routing_open_boards": sum(1 for entry in both if entry["routing_gap"] > 0),
            "cut": sum(1 for entry in table if entry["cut"]),
            "layout_without_result": [entry["name"] for entry in table if entry["layout"] is None],
            "designer_without_result": [entry["name"] for entry in table if entry["designer"] is None]}


# The boards docs/handover.md lists under "Not targets" for their rules or
# their panel, which the gap split leaves out with --targets (the 97 target
# boards of the v18 split): the PolyKybd molecule panels, the OpenESC 4in1
# panel, the OpenRX panel, krishveercard and SUMEC.
NOT_TARGETS = ("thpoll83__PolyKybd__poly_kb_molecule_4x4", "thpoll83__PolyKybd__poly_kb_molecule_4x5",
               "thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted", "thpoll83__PolyKybd__poly_kb_molecule_5x4_wave",
               "thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left", "OpenDrone-hw__OpenESC-30x30__4in1-panel",
               "OpenDrone-hw__OpenRX__OpenRX-panel-rev2", "hackclub__OnBoard__krishveercard",
               "bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV")


def budget_line(layout, designer):
    """The routing work each side had, where the rows say (run.py's
    `ladder_budget_seconds` and `ladder_budget_share` on layout rows, the
    designer tier's `budget_seconds`): the gap is only a split of causes
    when the router had the same work on both placements (review
    2026-10-21, H3). None where neither side says."""
    shares = [row["ladder_budget_share"] for row in layout.values() if row.get("ladder_budget_share") is not None]
    budgets = sorted({row["budget_seconds"] for row in designer.values() if row.get("budget_seconds") is not None})
    parts = []
    if budgets:
        parts.append("designer route " + "/".join(f"{budget:g}" for budget in budgets) + " s of work")
    if shares:
        parts.append(f"layout's final ladder a median {median(shares):.3f} of its total ({len(shares)} rows where "
                     "it ran)")
    elif layout:
        parts.append("layout's final ladder not recorded (rows before x1500: rejudge or derive it from layout.log)")
    return "budgets: " + "; ".join(parts) if parts else None


def print_gap(layout_path, designer_path, markdown=False, targets=False):
    layout, designer = by_name(rows_of(layout_path)), by_name(rows_of(designer_path))
    if targets:
        layout = {name: row for name, row in layout.items() if name not in NOT_TARGETS}
        designer = {name: row for name, row in designer.items() if name not in NOT_TARGETS}
    not_designer = [name for name, row in designer.items() if row.get("placement") != "designer"]
    if not_designer:
        print(f"warning: {len(not_designer)} rows of {designer_path} are not designer-tier rows "
              f"(run.py --tier designer): {not_designer[:3]}")
    table = gap_rows(layout, designer)
    width = max((len(entry["name"]) for entry in table), default=10)
    sep = " | " if markdown else "  "
    print(f"placement gap = layout open - designer-route open; routing gap = designer-route open "
          f"(KiCad's unconnected items); layout {layout_path}, designer {designer_path}"
          + (f"; the {len(NOT_TARGETS)} boards that are not targets left out" if targets else ""))
    binaries = [sorted({row.get("binary_sha256") or row.get("binary") or "?" for row in table.values()})
                for table in (layout, designer)]
    if binaries[0] != ["?"] and binaries[1] != ["?"] and binaries[0] != binaries[1]:
        print(f"BINARIES DIFFER: layout {', '.join(b[:12] for b in binaries[0])}; designer "
              f"{', '.join(b[:12] for b in binaries[1])}: the split mixes the two binaries' changes")
    budgets = budget_line(layout, designer)
    if budgets:
        print(budgets)
    header = ["connections", "layout", "designer", "placement gap", "routing gap"]
    print(f"{'board':{width}}{sep}" + sep.join(f"{h:>13}" for h in header))
    if markdown:
        print(f"{'-' * width} | " + " | ".join("-" * 13 for _ in header))

    def text(value):
        return "-" if value is None else str(value)

    for entry in sorted(table, key=lambda e: (-(e["placement_gap"] if e["placement_gap"] is not None else -1e9),
                                              e["name"])):
        cells = [text(entry["connections"]), text(entry["layout"]), text(entry["designer"]),
                 text(entry["placement_gap"]), text(entry["routing_gap"])]
        note = " CUT" if entry["cut"] else ""
        if entry["layout_error"]:
            note += f" (layout: {entry['layout_error']})"
        print(f"{entry['name']:{width}}{sep}" + sep.join(f"{c:>13}" for c in cells) + note)
    totals = gap_totals(table)
    print()
    print(f"{totals['both']} of {totals['boards']} boards with both results: layout open {totals['layout_open']}, "
          f"designer-route open {totals['designer_open']}; placement gap {totals['placement_gap']:+d} "
          f"(worse on {totals['placement_worse']} boards, same {totals['placement_same']}, better "
          f"{totals['placement_better']}), routing gap {totals['routing_gap']} (on {totals['routing_open_boards']} "
          f"boards); {totals['cut']} rows cut by the wall clock")
    if totals["layout_open"]:
        print(f"share of the layout's open connections: placement {totals['placement_gap'] / totals['layout_open']:.0%}, "
              f"routing {totals['routing_gap'] / totals['layout_open']:.0%}")
    if totals["layout_without_result"]:
        print(f"{len(totals['layout_without_result'])} boards the layout has no result for (placement or timeout): "
              + ", ".join(totals["layout_without_result"][:8]))
    if totals["designer_without_result"]:
        print(f"{len(totals['designer_without_result'])} boards the designer route has no result for: "
              + ", ".join(totals["designer_without_result"][:8]))
    only = sorted(set(layout) - set(designer))
    if only:
        print(f"{len(only)} layout boards without a designer row")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before")
    parser.add_argument("after")
    parser.add_argument("--splice", nargs="*", default=[])
    parser.add_argument("--markdown", action="store_true")
    parser.add_argument("--score", type=Path, metavar="BINARY",
                        help="score the result boards with this binary's score-kicad-board (the ground pours' cut "
                             "for the normalised line; rows from run.py --quality carry it already)")
    parser.add_argument("--gap", action="store_true",
                        help="before is a layout run, after a designer-tier run (run.py --tier designer): per "
                             "board the placement gap (layout open less designer-route open) and the routing gap "
                             "(designer-route open), with corpus totals")
    parser.add_argument("--targets", action="store_true",
                        help="with --gap: leave out the boards that are not targets (NOT_TARGETS, docs/handover.md)")
    arguments = parser.parse_args()
    if arguments.gap:
        print_gap(arguments.before, arguments.after, arguments.markdown, arguments.targets)
        return
    before_rows, after_rows = rows_of(arguments.before), rows_of(arguments.after)
    if arguments.score:
        add_scores(before_rows, output_directory(arguments.before), arguments.score)
        add_scores(after_rows, output_directory(arguments.after), arguments.score)
    before = by_name(before_rows)
    after = by_name(after_rows)
    after_directory = {name: output_directory(arguments.after) for name in after}
    for path in arguments.splice:
        # Marked as run.py --splice-from marks them: SPL, counted below.
        spliced_in = {name: {**row, "spliced": True, "spliced_from": row.get("spliced_from") or str(path)}
                      for name, row in by_name(rows_of(path)).items()}
        after.update(spliced_in)
        after_directory.update({name: output_directory(path) for name in spliced_in})
    common = sorted(set(before) & set(after))
    before_list, before_identity = task_list(arguments.before, before_rows)
    after_list, after_identity = task_list(arguments.after, after_rows)
    # Boards prepared from other constraints: another input, not a change.
    other_input = set()
    for name in common:
        if on_other_input(before[name], after[name], output_directory(arguments.before), after_directory[name]):
            other_input.add(name)
    lists = f"before {before_list}; after {after_list}"
    warning = rules_warning([before[n] for n in common], [after[n] for n in common])
    if warning:
        print(warning)
    mixes = (rule_mix_text([before[n] for n in common]), rule_mix_text([after[n] for n in common]))
    if any(mixes):
        print(f"strict: before {mixes[0] or '-'}; after {mixes[1] or '-'}")
    dead = [(label, n) for label, table in (("before", before), ("after", after)) for n in common if killed(table[n])]
    if dead:
        print(f"KILLED: {len(dead)} rows whose binary was killed (no result, a failure, not in better / same / "
              "worse): " + ", ".join(f"{n} ({label}, exit {(before if label == 'before' else after)[n].get('exit')})"
                                     for label, n in dead))
    if lists_differ(before_identity, after_identity):
        print(f"TASK LISTS DIFFER, {len(other_input)} BOARDS ON OTHER INPUTS (INPUT): {lists}")
    elif other_input:
        print(f"INPUTS DIFFER ON {len(other_input)} BOARDS (INPUT: their prepared constraints, task, board or "
              f"project files differ): {lists}")
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
        rows = [table[n] for n in common]
        if not any(judged_strictly(row) for row in rows):
            return "-"
        known = [n for n in common if passes_strict(table[n]) is not None]
        count = sum(1 for n in known if passes_strict(table[n]))
        text = f"{count}" + (f" of {len(known)} rows" if len(known) != len(common) else "")
        # A sum over several rules is not one rule's count.
        return text + (" MIXED RULES" if len(rule_mix(rows)) > 1 else "")

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
    print(normalised_line(normalised([before[n] for n in common]), "before"))
    print(normalised_line(normalised([after[n] for n in common]), "after "))
    if other_input:
        same_input = [n for n in common if n not in other_input]
        print(f"over the {len(same_input)} boards with the same input:")
        print(objectives_line(objectives([before[n] for n in same_input]), "before"))
        print(objectives_line(objectives([after[n] for n in same_input]), "after "))
        print(normalised_line(normalised([before[n] for n in same_input]), "before"))
        print(normalised_line(normalised([after[n] for n in same_input]), "after "))
    if only_after:
        print(f"{len(only_after)} boards only after: {sum(1 for n in only_after if passes(after[n]))} pass")
    if only_before:
        print(f"{len(only_before)} boards only before")


if __name__ == "__main__":
    main()
