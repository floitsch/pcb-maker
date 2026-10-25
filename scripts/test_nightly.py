# Copyright (C) 2026 Toit contributors.
"""Tests of scripts/nightly.sh (a dry run: run.py replaced by a stub, no
build) and of scripts/nightly-report.py (its parsing of run.py's
summary.json files, the docs' table and state line)."""

import importlib.util
import json
import os
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
REPO = HERE.parent

spec = importlib.util.spec_from_file_location("nightly_report", HERE / "nightly-report.py")
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)

# The stub for run.py: rows by stage (the output directory's name) and
# night (FAKE_NIGHT), summary.json by compare.py's own functions, the
# fingerprint stage's route.log files, and its command line and
# environment in <output>/argv.json.
STUB = textwrap.dedent('''\
    import json, os, sys
    from pathlib import Path
    sys.path.insert(0, os.environ["FAKE_COMPARE"])
    import compare
    output = Path(sys.argv[1])
    output.mkdir(parents=True, exist_ok=True)
    stage, night = output.name, int(os.environ["FAKE_NIGHT"])
    (output / "argv.json").write_text(json.dumps({"argv": sys.argv[2:], "env": {
        k: v for k, v in os.environ.items() if k.startswith("PCB_ROUTER")}}))

    def row(name, unconnected, **extra):
        r = {"name": name, "mode": "route" if stage != "sweep" else "layout", "routed": f"{50 - unconnected}/50",
             "native": {"unconnected": unconnected, "errors": {}}, "new_findings_strict": {}, "vias": 10,
             "length_mm": 100.0, "work_seconds": 30.0, "pass": unconnected == 0, "pass_strict": unconnected == 0}
        r.update(extra)
        return r

    rows = [row("repo__Alpha", 0), row("repo__Beta", 2 if night == 1 else 0), row("repo__Gamma", 5)]
    if stage == "sweep":
        for r, share in zip(rows, (0.6, 0.7, 0.8)):
            r["ladder_budget_share"] = share
        if night == 2:
            rows[2]["cut"] = ["wall_guard"]
    if stage == "designer":
        for r in rows:
            r["placement"] = "designer"
            r["native"]["unconnected"] = max(0, r["native"]["unconnected"] - 2)
    if stage == "expansion" and night == 2:
        rows[2] = {"name": "repo__Gamma", "mode": "route", "pass": False, "error": "killed by signal 9 (SIGKILL)",
                   "killed": True, "cut": ["killed"], "exit": -9}
    if stage == "fingerprints":
        for r in rows:
            board = output / r["name"]
            (board / "route").mkdir(parents=True, exist_ok=True)
            changed = "b" if night == 2 and r["name"] == "repo__Gamma" else "a"
            lines = [f"fingerprint before resolve: 1111{changed}\\n", "fingerprint after clean-up: 2222\\n"]
            if night == 2 and r["name"] == "repo__Alpha":  # rungs side by side: another order, the same lines
                lines.reverse()
            (board / "route.log").write_text("".join(lines))
            (board / "route" / "x.kicad_pcb").write_text(f"board {changed}")
    for r in rows:
        print(json.dumps(r))
    summary = compare.objectives(rows)
    summary.update(wall_seconds=12, budget_scale=float(sys.argv[sys.argv.index("--budget-scale") + 1])
                   if "--budget-scale" in sys.argv else 1.0, jobs=1, threads=1)
    summary["normalised"] = compare.normalised(rows, {})
    print(f"{summary['passes']}/{len(rows)} tasks pass (12 s wall)")
    (output / "summary.json").write_text(json.dumps(summary))
''')


def night(tmp_path, date, sha, fake_night, *arguments):
    stub = tmp_path / "fake_run.py"
    stub.write_text(STUB)
    binary = tmp_path / "pcb-maker"
    binary.write_text(f"binary {sha}\n")
    environment = {**os.environ, "PCB_NIGHTLY_RUN_PY": str(stub), "PCB_NIGHTLY_FAKE_BINARY": str(binary),
                   "PCB_NIGHTLY_ROOT": str(tmp_path), "PCB_NIGHTLY_DATE": date, "PCB_NIGHTLY_SHA": sha,
                   "PCB_NIGHTLY_SOURCE": str(REPO), "FAKE_NIGHT": str(fake_night),
                   "FAKE_COMPARE": str(REPO / "benchmarks/agent-tasks")}
    environment.pop("PCB_NIGHTLY_SWEEP_DAYS", None)
    environment.pop("PCB_NIGHTLY_COMMIT_REPORT", None)
    return subprocess.run(["bash", str(HERE / "nightly.sh"), *arguments], env=environment, capture_output=True,
                          text=True, timeout=120)


def argv(tmp_path, date, stage):
    return json.loads((tmp_path / "build/nightly" / date / stage / "argv.json").read_text())


@pytest.fixture(scope="module")
def two_nights(tmp_path_factory):
    tmp_path = tmp_path_factory.mktemp("nightly")
    bin_directory = tmp_path / "build/bin"
    bin_directory.mkdir(parents=True)
    for day in range(1, 9):  # eight older nightly snapshots: one of them goes
        (bin_directory / f"pcb-maker-nightly-2026-10-{day:02d}-aaaaaaaaaaaa").write_text("old")
    first = night(tmp_path, "2026-10-22", "1" * 40, 1)
    assert first.returncode == 0, first.stdout + first.stderr
    second = night(tmp_path, "2026-10-23", "2" * 40, 2)
    assert second.returncode == 0, second.stdout + second.stderr
    return tmp_path


def test_the_stages_run_with_their_settings(two_nights):
    fingerprints = argv(two_nights, "2026-10-23", "fingerprints")
    assert fingerprints["env"] == {"PCB_ROUTER_FINGERPRINT": "1", "PCB_ROUTER_NO_WALL_GUARD": "1"}
    text = " ".join(fingerprints["argv"])
    assert "--jobs 1 --threads 4" in text and "--budget-scale 0.25" in text
    assert "--route-from " + str(two_nights / "build/github-layout-v16") in text
    for board in ("Castor_and_Pollux", "k30-SBC", "jetson-nano", "Sisu_SSE-9"):
        assert board in text
    assert "--tier expansion --jobs 8 --threads 1" in " ".join(argv(two_nights, "2026-10-23", "expansion")["argv"])
    assert "--tier short --jobs 3 --threads 2" in " ".join(argv(two_nights, "2026-10-23", "short")["argv"])
    sweep = " ".join(argv(two_nights, "2026-10-23", "sweep")["argv"])
    assert "tasks/v7/tasks.json --jobs 10 --threads 2 --quality" in sweep
    # The matched designer tier: the median of the newest earlier night's
    # sweep shares (it runs before tonight's sweep, the largest boards
    # first); the first night has none: 0.695.
    designer = " ".join(argv(two_nights, "2026-10-23", "designer")["argv"])
    assert "--tier designer --budget-scale 0.7 --quality --jobs 8 --threads 1 --order peak-desc" in designer
    assert "--budget-scale 0.695 " in " ".join(argv(two_nights, "2026-10-22", "designer")["argv"])
    log = (two_nights / "build/nightly/2026-10-23/nightly.log").read_text()
    assert log.index("== short:") < log.index("== designer:") < log.index("== sweep:")
    for stage in report.STAGES:
        words = argv(two_nights, "2026-10-23", stage)["argv"]
        assert "--lock" not in words
        assert words[words.index("--min-free-gb") + 1] == "10"
        assert words[words.index("--ledger-priority") + 1] == "nightly"
        assert words[words.index("--binary") + 1].endswith("pcb-maker-nightly-2026-10-23-222222222222")


def test_snapshots_keep_the_newest_seven(two_nights):
    names = sorted(path.name for path in (two_nights / "build/bin").iterdir())
    assert len(names) == 7
    assert names[-2:] == ["pcb-maker-nightly-2026-10-22-111111111111", "pcb-maker-nightly-2026-10-23-222222222222"]
    assert (two_nights / "build/bin/pcb-maker-nightly-2026-10-23-222222222222").read_text() == f"binary {'2' * 40}\n"


def test_the_summary(two_nights):
    directory = two_nights / "build/nightly/2026-10-23"
    for name in ("summary.md", "nightly.json", "gap.txt", "diff-short.txt", "sweep.log", "short.err", "build.log"):
        assert (directory / name).is_file(), name
    summary = (directory / "summary.md").read_text()
    assert summary.startswith("# Nightly 2026-10-23\n")
    assert "main `222222222222`" in summary
    assert "Ran: fingerprints, expansion, short, sweep, designer. Skipped: nothing." in summary
    assert "| short | 2 | 2 | 3 | 5 | 0 (0) |" in summary
    assert "| designer | 2 | 2 | 3 | 3 |" in summary and "| 0.7 |" in summary
    # The gap: layout open 5 (Gamma), designer route 3.
    assert "share of the layout's open connections: placement 40%, routing 60%" in summary
    assert "- expansion, killed: repo__Gamma" in summary
    assert "- sweep, cut by the wall clock: repo__Gamma" in summary
    assert ("- short against 2026-10-22 (diff-short.txt): 3 boards in both: better 1, same 2, worse 0; "
            "unconnected 7 -> 5; passes 1 -> 2 (strict 1 -> 2); gained the pass: Beta; gained the strict pass: Beta"
            in summary)
    assert "| Gamma | 2 | " in summary and "fingerprints changed (before resolve); routed board changed |" in summary
    assert "| Alpha | 2 | " in summary and "| identical |" in summary
    first = (two_nights / "build/nightly/2026-10-22/summary.md").read_text()
    assert "- short: no earlier nightly ran it." in first
    record = json.loads((directory / "nightly.json").read_text())
    assert record["stages"]["short"]["diff"]["gained"] == ["repo__Beta"]
    assert record["gap"]["placement_share"] == 0.4


def test_the_report_into_the_docs(two_nights, tmp_path):
    docs = tmp_path / "docs"
    docs.mkdir()
    old = "| 2026-10-01 | `000000000000` | first | - | - | - | - | - | - |"
    (docs / "benchmarks.md").write_text(f"# B\n\n## Nightly\n\n{report.TABLE_BEGIN}\n{report.HEADER}\n"
                                        f"| --- |\n{old}\n{report.TABLE_END}\n\n## Other\n")
    (docs / "handover.md").write_text(f"# H\n\n{report.STATE_BEGIN}\nnone yet\n{report.STATE_END}\nrest\n")
    result = subprocess.run([sys.executable, str(HERE / "nightly-report.py"), "--nights",
                             str(two_nights / "build/nightly"), "--docs", str(tmp_path)],
                            capture_output=True, text=True)
    assert result.returncode == 0, result.stderr
    benchmarks = (docs / "benchmarks.md").read_text()
    table = benchmarks.split(report.TABLE_BEGIN)[1].split(report.TABLE_END)[0].strip().splitlines()
    assert table[0] == report.HEADER
    assert table[2].startswith("| 2026-10-23 | `222222222222` | changed: Gamma | 2/3 (strict 2), 0 open, 1 KILLED "
                               "| 2/3 (strict 2), 5 open | 2/3 (strict 2), 5 open, 1 cut "
                               "| 2/3 (strict 2), 3 open at 0.7 | 40% / 60% |")
    assert "short 1/2/0 (pass +1 -0)" in table[2]
    assert table[3] == old and len(table) == 4
    assert benchmarks.endswith("\n\n## Other\n")
    handover = (docs / "handover.md").read_text()
    line = report.state_line(report.newest(two_nights / "build/nightly"))
    assert handover == f"# H\n\n{report.STATE_BEGIN}\n{line}\n{report.STATE_END}\nrest\n"
    assert "**Nightly** 2026-10-23 (main 222222222222" in handover
    assert "gap placement 40%, routing 60%." in handover


def test_an_unchanged_main_is_not_measured_again(two_nights):
    again = night(two_nights, "2026-10-24", "2" * 40, 2)
    assert again.returncode == 0, again.stderr
    assert "nothing to do" in again.stdout
    assert not (two_nights / "build/nightly/2026-10-24").exists()


def test_a_quick_night_skips_the_sweep_and_the_designer_tier(tmp_path):
    result = night(tmp_path, "2026-10-25", "3" * 40, 1, "--quick")
    assert result.returncode == 0, result.stdout + result.stderr
    summary = (tmp_path / "build/nightly/2026-10-25/summary.md").read_text()
    assert "Ran: fingerprints, expansion, short. Skipped: sweep, designer." in summary
    assert not (tmp_path / "build/nightly/2026-10-25/gap.txt").exists()
    # Only on Mondays: 2026-10-26 is one, 2026-10-27 is not.
    monday = night(tmp_path, "2026-10-26", "4" * 40, 1, "--sweep-days", "mon")
    assert monday.returncode == 0, monday.stderr
    assert "Skipped: nothing." in (tmp_path / "build/nightly/2026-10-26/summary.md").read_text()
    tuesday = night(tmp_path, "2026-10-27", "5" * 40, 1, "--sweep-days", "1,mon")
    assert tuesday.returncode == 0, tuesday.stderr
    assert "Skipped: sweep, designer." in (tmp_path / "build/nightly/2026-10-27/summary.md").read_text()


def test_stage_summary_parses_run_py_summaries(tmp_path):
    (tmp_path / "summary.json").write_text(json.dumps({
        "rows": 46, "passes": 18, "strict_rows": 46, "passes_strict": 17, "new_errors": 3, "new_errors_strict": 2,
        "unconnected": 631, "failed": 1, "cut": 2, "vias": 9000, "length_mm": 1234.5, "work_seconds": 1380,
        "wall_seconds": 272, "budget_scale": 1.0, "strict_rule": 3, "jobs": 8, "threads": 1,
        "normalised": {"vias_ratio": 0.71, "length_ratio": 1.03, "open_share": 0.009, "open_share_mean": 0.284,
                       "ground_cut_delta_mm": 0.0}}))
    parsed = report.stage_summary(tmp_path)
    assert parsed == report.stage_summary(tmp_path / "summary.json")
    assert (parsed["rows"], parsed["passes"], parsed["passes_strict"], parsed["unconnected"], parsed["cut"]) == \
        (46, 18, 17, 631, 2)
    assert (parsed["vias_ratio"], parsed["length_ratio"], parsed["open_share"]) == (0.71, 1.03, 0.009)
    assert parsed["strict_rule"] == 3 and parsed["wall_seconds"] == 272
    # A summary from before the strict rule and the normalised measures.
    (tmp_path / "old.json").write_text(json.dumps({"rows": 30, "passes": 23, "unconnected": 26, "cut": 0}))
    old = report.stage_summary(tmp_path / "old.json")
    assert old["passes_strict"] is None and old["vias_ratio"] is None and old["passes"] == 23
    # A summary whose rows have no strict judgement (strict_rows 0).
    (tmp_path / "unjudged.json").write_text(json.dumps({"rows": 4, "passes": 2, "strict_rows": 0,
                                                        "passes_strict": 0}))
    assert report.stage_summary(tmp_path / "unjudged.json")["passes_strict"] is None
    assert report.stage_summary(tmp_path / "missing.json") is None
    (tmp_path / "broken.json").write_text("{")
    assert report.stage_summary(tmp_path / "broken.json") is None


def test_fingerprints_compare_as_a_multiset(tmp_path):
    # Rungs side by side print their lines as they finish: two logs that
    # differ only in order are identical; one copy of a repeated line
    # changed, or one more copy, is a change of that phase.
    def logs(name, *texts):
        for board, text in zip(("repo__One", "repo__Two", "repo__Three"), texts):
            (tmp_path / name / board).mkdir(parents=True)
            (tmp_path / name / board / "route.log").write_text(text)
        return report.fingerprints_of(tmp_path / name)

    a, b, c = ("fingerprint before resolve: 00aa\n", "fingerprint after clean-up: 00bb\n",
               "fingerprint before stitching 2: 00cc, 3 pieces, main 1, spoke islands []\n")
    before = logs("before", a + b + a + c, a + b, a + b)
    after = logs("after", c + a + b + a, a + "fingerprint after clean-up: 00bd\n", a + b + b)
    changes = report.fingerprint_changes(after, before)
    assert changes == {"repo__One": "identical", "repo__Two": "fingerprints changed (after clean-up)",
                       "repo__Three": "fingerprints changed (after clean-up)"}
    record = {"stages": {"fingerprints": {"status": "ran"}},
              "fingerprints": {"previous": "2026-10-22", "changes": {"repo__One": "identical"}}}
    assert report.fingerprint_cell(record) == "identical"


def test_the_table_keeps_fourteen_nights_newest_first():
    text = f"x\n{report.TABLE_BEGIN}\n{report.TABLE_END}\ny\n"
    for day in range(1, 17):
        text = report.merge_table(text, f"| 2026-10-{day:02d} | `a` | x | x | x | x | x | x | x |")
    rows = [line for line in text.split(report.TABLE_BEGIN)[1].split(report.TABLE_END)[0].splitlines()
            if line.startswith("| 2026")]
    assert [row.split("|")[1].strip() for row in rows] == [f"2026-10-{day:02d}" for day in range(16, 2, -1)]
    # The same night again replaces its row.
    text = report.merge_table(text, "| 2026-10-16 | `b` | x | x | x | x | x | x | x |")
    assert text.count("| 2026-10-16 |") == 1 and "| 2026-10-16 | `b` |" in text
    assert text.startswith("x\n") and text.endswith("\ny\n")
    with pytest.raises(ValueError):
        report.merge_table("no markers", "| a |")


def test_ladder_share(tmp_path):
    compare = report.load_compare()
    log = tmp_path / "sweep.log"
    log.write_text("\n".join(json.dumps({"name": f"b{i}", "ladder_budget_share": share})
                             for i, share in enumerate((0.5, None, 0.7, 0.9, 0.6))) + "\nnot a row\n")
    assert report.ladder_share(log, compare) == 0.65
    assert report.ladder_share(tmp_path / "none.log", compare) is None


def test_commit_commits_the_two_docs_alone_and_pushes(two_nights, tmp_path):
    def git(*arguments, cwd=tmp_path / "main"):
        return subprocess.run(["git", *arguments], cwd=cwd, capture_output=True, text=True, check=True).stdout

    subprocess.run(["git", "init", "--quiet", "--bare", str(tmp_path / "origin.git")], check=True)
    subprocess.run(["git", "init", "--quiet", "-b", "main", str(tmp_path / "main")], check=True)
    git("config", "user.email", "nightly@example.com")
    git("config", "user.name", "Nightly")
    (tmp_path / "main/docs").mkdir()
    (tmp_path / "main/docs/benchmarks.md").write_text(f"{report.TABLE_BEGIN}\n{report.TABLE_END}\n")
    (tmp_path / "main/docs/handover.md").write_text(f"{report.STATE_BEGIN}\n{report.STATE_END}\n")
    (tmp_path / "main/other.txt").write_text("other\n")
    git("add", ".")
    git("commit", "--quiet", "-m", "start")
    git("remote", "add", "origin", str(tmp_path / "origin.git"))
    git("push", "--quiet", "origin", "main")
    # Something else staged and changed: never part of the commit.
    (tmp_path / "main/other.txt").write_text("changed\n")
    git("add", "other.txt")
    record = report.newest(two_nights / "build/nightly")
    report.commit(record, tmp_path / "main")
    assert git("log", "-1", "--format=%s") == "Docs: nightly 2026-10-23 (main 222222222222)\n"
    assert git("show", "--name-only", "--format=", "HEAD").split() == ["docs/benchmarks.md", "docs/handover.md"]
    assert git("diff", "--cached", "--name-only") == "other.txt\n"
    assert git("rev-parse", "main", cwd=tmp_path / "origin.git") == git("rev-parse", "HEAD")
    # The same night again: nothing to commit.
    report.commit(record, tmp_path / "main")
    assert git("rev-list", "--count", "HEAD") == "2\n"
    # A change of either file in the checkout: refused, untouched.
    (tmp_path / "main/docs/handover.md").write_text("edited by hand\n")
    with pytest.raises(SystemExit):
        report.commit(record, tmp_path / "main")
    assert (tmp_path / "main/docs/handover.md").read_text() == "edited by hand\n"
    # Not on main: refused.
    git("checkout", "--quiet", "-f", "-b", "other")
    with pytest.raises(SystemExit):
        report.commit(record, tmp_path / "main")
