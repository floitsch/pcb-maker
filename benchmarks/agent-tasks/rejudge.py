#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Judges a stored run again under this checkout's rules, without routing.

    benchmarks/agent-tasks/rejudge.py <log>[:<dir>] [<log>[:<dir>] ...] [--binary PATH] [--output OUT.log]

The first log is the run. Its rows are judged again from what the run left
in its output directory (`<dir>`, default the log's name without `.log`):
the result's KiCad report (`drc.json`), the strict rule's baseline
(`baseline-drc.json`) and the designer's own solder mask bridges
(`designer-mask-bridges.json`). A spliced row is judged from the run it
came from: its `spliced_from` log when the row names one, else the next
log given (older logs do not say; the first row of the board there that is
not spliced itself). With `--binary` the designer's own board is checked
with its drc-kicad-board (the DRC cache answers) where a run kept no
bridges, so that the strict rule's pre-bridged openings are judged as a
run of today would; without it they are judged from the stored files
alone.

Each judged row keeps the run's fields and gets `native`, `pass` and the
strict rule's fields anew, with `rejudged_from` (the log its files are
from). Rows without a result (an error, a timeout, a kill) stay as they
were. Prints the rows whose `pass` or `pass_strict` changed and the
summary (compare.py's objectives and normalised measures, the pass counts,
the strict rule); `--output` writes the judged rows as a log and the
summary beside it (`<output>.summary.json`). README.md, "The strict rule"."""

import argparse
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import compare  # noqa: E402
import run  # noqa: E402

# The fields the strict rule writes (run.strict_row); a judged row gets
# them anew.
STRICT_FIELDS = ("strict_rule", "new_findings_strict", "preexisting_strict", "strict_mask_openings",
                 "strict_unknown", "strict_baseline_error", "clean_without_subtraction", "pass_strict")


def chain_of(arguments):
    """[(rows by name, output directory, log path)] of `<log>[:<dir>]`
    arguments."""
    chain = []
    for argument in arguments:
        log, _, directory = argument.partition(":")
        log = Path(log)
        chain.append((compare.by_name(compare.rows_of(log)), Path(directory) if directory else log.with_suffix(""),
                      str(log)))
    return chain


def source(name, shown, chain, opened):
    """(row, task directory, log) of the run a row of `name` was judged in:
    the first log of `chain` whose row is not spliced. A row naming its
    source (`spliced_from`) is looked up there first (`opened` caches the
    logs read)."""
    candidates = list(chain)
    named = shown.get("spliced_from")
    if named:
        if named not in opened:
            path = Path(named)
            opened[named] = (compare.by_name(compare.rows_of(path)), path.with_suffix(""), named)
        candidates.insert(1, opened[named])
    for rows, directory, log in candidates:
        row = rows.get(name)
        if row is not None and not row.get("spliced"):
            return row, directory / name, log
    return None, None, None


def drc_directory(row, work):
    """Where the run's KiCad report of the result lies."""
    if row.get("mode") == "schematic":
        return work / "drc"
    return work / "route" if row.get("mode") == "route" else work / "layout" / "result"


def designer_bridges(binary, task, cache):
    """The designer's own board's solder mask bridges (counting_findings),
    checked with `binary`'s drc-kicad-board into `cache`; None when it
    cannot be checked."""
    report = cache / f"{task['name']}.json"
    if not report.exists():
        cache.mkdir(parents=True, exist_ok=True)
        try:
            run.harness_common.run_group([str(binary), "drc-kicad-board", str(run.task_directory(task)),
                                          task["board_id"], str(report)], capture_output=True, timeout=900)
        except Exception:  # noqa: BLE001
            return None
    try:
        findings = run.counting_findings(json.loads(report.read_text()))
    except (OSError, ValueError):
        return None
    return [finding for finding in findings if finding[0] == "solder_mask_bridge"]


class StoredBaseline(run.StrictBaseline):
    """The strict rule's baseline from a run's stored files: its
    baseline-drc.json (an error where the run kept none) and its designer
    bridges, else `bridges` (designer_bridges)."""

    def __init__(self, work, bridges=None):
        super().__init__(work, "board", work, None)
        self.enabled, self.taken = True, True
        report = work / "baseline-drc.json"
        try:
            self.result = run.counting_findings(json.loads(report.read_text()))
        except (OSError, ValueError):
            self.result, self.error = None, f"no stored {report.name}"
        self.bridges = bridges

    def designer(self):
        stored = super().designer()
        if stored is not None:
            return stored
        return self.bridges() if callable(self.bridges) else self.bridges


def judge(shown, row, work, log, bridges=None):
    """`shown` (the run's row) judged again from `row`'s run's files in
    `work`."""
    judged = {key: value for key, value in shown.items() if key not in STRICT_FIELDS}
    directory = drc_directory(row, work)
    allowed = row.get("designer_findings") or {}
    copper = run.native(directory, allowed)
    if copper is None:
        return None
    judged["native"] = copper
    judged["rejudged_from"] = log
    judged["pass"] = compare.passes(judged)
    unexpected = judged.get("unexpected_misses") or []
    run.strict_row(judged, run.strict_native(directory, StoredBaseline(work, bridges)), unexpected)
    return judged


def summary(rows):
    """compare.py's measures of judged rows (the run's summary.json's
    continuous part), with the pass counts and the strict rule."""
    return {"objectives": compare.objectives(rows), "normalised": compare.normalised(rows),
            "passes": sum(1 for row in rows if row.get("pass")),
            "passes_strict": sum(1 for row in rows if row.get("pass_strict")),
            "strict_rule": run.STRICT_RULE, "rows": len(rows)}


def killed(row):
    """compare.killed where this checkout has it (a run's own logs before
    it judged a killed route as a result)."""
    return getattr(compare, "killed", lambda row: False)(row)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("logs", nargs="+", help="the run's log, then the logs its spliced rows came from")
    parser.add_argument("--binary", type=Path,
                        help="check the designer's board with this binary's drc-kicad-board where a run kept no "
                             "designer bridges (the DRC cache answers)")
    parser.add_argument("--designer-cache", type=Path, default=run.ROOT / "build/rejudge-designer-drc",
                        help="where those reports are kept")
    parser.add_argument("--output", type=Path, help="write the judged rows here and the summary beside them")
    arguments = parser.parse_args()
    chain = chain_of(arguments.logs)
    final = chain[0][0]
    tasks = {}
    opened = {}
    rows, changed, missing = [], [], []
    for name, shown in final.items():
        row, work, log = source(name, shown, chain, opened)
        judged = None
        if row is not None and row.get("native") is not None and not killed(row):
            bridges = None
            if arguments.binary is not None:
                if row["tasks"] not in tasks:
                    listed = json.loads((run.ROOT / row["tasks"]).read_text())["tasks"]
                    tasks[row["tasks"]] = {task["name"]: task for task in listed}
                task = tasks[row["tasks"]].get(name)
                if task is not None:
                    def bridges(task=task):
                        return designer_bridges(arguments.binary, task, arguments.designer_cache)
            judged = judge(shown, row, work, log, bridges)
            if judged is None:
                missing.append(name)
        if judged is None:
            rows.append(shown)
            continue
        rows.append(judged)
        if bool(shown.get("pass")) != judged["pass"] or bool(shown.get("pass_strict")) != judged["pass_strict"]:
            changed.append((name, shown, judged))
    for name, shown, judged in changed:
        print(f"{name}: pass {bool(shown.get('pass'))} -> {judged['pass']}, pass_strict "
              f"{bool(shown.get('pass_strict'))} -> {judged['pass_strict']}; strict new "
              f"{shown.get('new_findings_strict')} -> {judged.get('new_findings_strict')}")
    if missing:
        print(f"{len(missing)} rows with a result have no stored KiCad report (kept as they were): {missing[:5]}")
    totals = summary(rows)
    shown_rows = list(final.values())
    print(f"{len(rows)} rows: pass {sum(1 for r in shown_rows if r.get('pass'))} -> {totals['passes']}, "
          f"pass_strict {sum(1 for r in shown_rows if r.get('pass_strict'))} -> {totals['passes_strict']} "
          f"(strict rule {run.STRICT_RULE}); {len(changed)} rows changed")
    print(compare.objectives_line(totals["objectives"]))
    print(compare.normalised_line(totals["normalised"]))
    if arguments.output is not None:
        arguments.output.write_text("".join(json.dumps(row) + "\n" for row in rows))
        Path(str(arguments.output) + ".summary.json").write_text(json.dumps(totals, indent=1, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
