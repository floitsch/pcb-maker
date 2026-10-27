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
# environment in <output>/argv.json. Night 3 is night 2 without the kill
# and the cut, with Gamma's project files changed (INPUT) and Alpha's
# strict pass lost to a clearance finding.
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
             "length_mm": 100.0, "work_seconds": 30.0, "pass": unconnected == 0, "pass_strict": unconnected == 0,
             "peak_rss_mb": 1000 * night + unconnected, "seconds": 60.0, "project_sha256": "a" * 64}
        r.update(extra)
        return r

    rows = [row("repo__Alpha", 0), row("repo__Beta", 2 if night == 1 else 0), row("repo__Gamma", 5)]
    if night == 3:
        rows[2]["project_sha256"] = "b" * 64
        rows[0].update(new_findings_strict={"clearance": 2}, pass_strict=False, **{"pass": False})
        rows[0]["native"]["errors"] = {"clearance": 2}
    if stage == "sweep":
        for r, share in zip(rows, (0.6, 0.7, 0.8)):
            r["ladder_budget_share"] = share
        if night == 2:
            rows[2]["cut"] = ["wall_guard"]
        for r in rows:
            r["quality"] = {"ours": {"vias_in_pads": 0}}
    if stage == "designer":
        for r in rows:
            r["placement"] = "designer"
            r["native"]["unconnected"] = max(0, r["native"]["unconnected"] - 2)
    if stage == "short" and night == 2:
        # Ended by the memory ledger and retried once by run.py (a result).
        rows[1]["ledger_retry"] = {"error": "internal error: memory: ... past its reservation of 1415 MB in the "
                                            "memory ledger ...", "expected_mb": 1841}
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


def night(tmp_path, date, sha, fake_night, *arguments, environment_extra=None):
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
    environment.update(environment_extra or {})
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
    assert "tasks/v7/tasks.json --jobs 10 --threads 2 --quality --order peak-desc" in sweep
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
        # run.py's measured floor (since 2026-10-27), not a fixed one.
        assert "--min-free-gb" not in words
        assert words[words.index("--ledger-priority") + 1] == "nightly"
        assert words[words.index("--binary") + 1].endswith("pcb-maker-nightly-2026-10-23-222222222222")


def test_the_estimates_follow_the_nights(two_nights):
    # The first night's stages had no nightly estimates yet (run.py's
    # default, the committed sidecar); the second night's read the first
    # night's refresh, and its own refresh adds its rows.
    estimates = two_nights / "build/nightly/estimates.json"
    assert "--estimates" not in argv(two_nights, "2026-10-22", "sweep")["argv"]
    for stage in report.STAGES:
        words = argv(two_nights, "2026-10-23", stage)["argv"]
        assert words[words.index("--estimates") + 1] == str(estimates)
    sidecar = json.loads(estimates.read_text())
    alpha = sidecar["boards"]["repo__Alpha"]
    assert [row[1] for row in alpha["layout"]["rows"]] == [2000, 1000]
    assert alpha["layout"]["peak_rss_mb"] == 2500
    assert alpha["designer"]["measured_mb"] == 2000
    # The committed sidecar's boards stay.
    assert "Goga64__ULK__ULK" in sidecar["boards"]
    assert "with a measured row" in (two_nights / "build/nightly/2026-10-23/estimates.log").read_text()


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
    assert "- short, ended by the memory ledger and retried once: repo__Beta" in summary
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


# --- the published write-up -------------------------------------------------------

DOCS = {
    "docs/benchmarks.md": textwrap.dedent(f"""\
        # Board corpus

        ## Nightly

        {report.TABLE_BEGIN}
        {report.TABLE_END}

        ## Layout sweep v21 (2026-10-21)

        Written by hand.

        ### Placement or routing on one binary

        By hand too.

        ## Other

        Unrelated.
        """),
    "README.md": textwrap.dedent("""\
        # pcb-maker

        - **Boards harvested from GitHub** (3 finished designs). Reduced to
          what an agent starts from - parts stacked - pcb-maker places and
          routes **1 of 3** by hand ([results](docs/benchmarks.md)).
        - **At scale**: unrelated.

        | Sweep | Boards | pcb-maker | Freerouting |
        | --- | --- | --- | --- |
        | Layout from stacked parts and constraints, 1500 s of work (v21, x1, 2026-10-21) | 3 | 1 clean | - |
        | Routing the designer's placement at the layout's routing budget, 1000 s of work (matched designer tier x1, 2026-10-21) | 3 | 2 clean | - |
        | Routing the designer's placement cold | 3 | 2 clean | - |
        """),
    "docs/handover.md": textwrap.dedent(f"""\
        # Hand-over (state as of 2026-10-21; measured by layout sweep v21 on x1)

        Intro.

        {report.STATE_BEGIN}
        none yet
        {report.STATE_END}

        **Layout sweep v21** (x1): 1 of 3 pass, by hand.

        **Quick tier** (30 boards): unrelated.
        """),
    "docs/changelog.md": textwrap.dedent("""\
        # Changelog

        ## Committed on 2026-10-25 (later work)

        Text.

        ## Committed on 2026-10-23 (earlier work)

        Text.

        ## Measured on 2026-10-21 (layout sweep v21)

        Text.
        """),
}


def git_in(directory, *arguments):
    return subprocess.run(["git", *arguments], cwd=directory, capture_output=True, text=True, check=True).stdout


def docs_checkout(tmp_path):
    """A main checkout with the documents above, pushed to a bare origin."""
    subprocess.run(["git", "init", "--quiet", "--bare", str(tmp_path / "origin.git")], check=True)
    main = tmp_path / "main"
    subprocess.run(["git", "init", "--quiet", "-b", "main", str(main)], check=True)
    git_in(main, "config", "user.email", "nightly@example.com")
    git_in(main, "config", "user.name", "Nightly")
    for path, text in DOCS.items():
        (main / path).parent.mkdir(parents=True, exist_ok=True)
        (main / path).write_text(text)
    (main / "other.txt").write_text("other\n")
    git_in(main, "add", *DOCS, "other.txt")
    git_in(main, "commit", "--quiet", "-m", "start")
    git_in(main, "remote", "add", "origin", str(tmp_path / "origin.git"))
    git_in(main, "push", "--quiet", "-u", "origin", "main")
    return main


@pytest.fixture(scope="module")
def published(tmp_path_factory):
    """Night 1 measured and not published (--no-commit-report), a hand
    sweep v21 (night 1's rows less one open connection on Gamma), then
    night 3 measured and published by nightly.sh into a main checkout."""
    tmp_path = tmp_path_factory.mktemp("published")
    main = docs_checkout(tmp_path)
    environment = {"PCB_NIGHTLY_CHECKOUT": str(main), "PCB_NIGHTLY_MIN_RESULTS": "3"}
    first = night(tmp_path, "2026-10-22", "1" * 40, 1, "--no-commit-report", environment_extra=environment)
    assert first.returncode == 0, first.stdout + first.stderr
    rows = [json.loads(line) for line in (tmp_path / "build/nightly/2026-10-22/sweep.log").read_text().splitlines()
            if line.startswith("{")]
    rows[2]["native"]["unconnected"] = 6
    (tmp_path / "build/github-layout-v21.log").write_text("".join(json.dumps(row) + "\n" for row in rows))
    third = night(tmp_path, "2026-10-24", "3" * 40, 3, environment_extra=environment)
    assert third.returncode == 0, third.stdout + third.stderr
    return tmp_path, main


def section_of(text, heading):
    start = text.index(heading)
    end = text.find("\n## ", start + 3)
    return text[start:end if end >= 0 else len(text)]


def test_the_guard():
    def record(**changes):
        stages = {stage: {"status": "ran", "exit": 0, "cut": [], "killed": [],
                          "summary": {"rows": 109, "failed": 5}} for stage in report.STAGES}
        for stage, entry in changes.items():
            stages[stage].update(entry)
        return {"night": "2026-10-26", "stages": stages}

    assert report.guard(record()) == []
    assert report.guard(record(sweep={"summary": {"rows": 109, "failed": 9}})) == []
    assert report.guard(record(sweep={"summary": {"rows": 109, "failed": 10}})) == \
        ["sweep: 99 rows with a result, fewer than 100"]
    assert report.guard(record(sweep={"summary": {"rows": 104, "failed": 5}}), min_results=99) == []
    assert report.guard(record(designer={"status": "skipped", "exit": None})) == ["designer skipped"]
    assert report.guard(record(designer={"status": "failed", "exit": 1})) == ["designer failed (exit 1)"]
    assert report.guard(record(short={"cut": ["repo__Alpha"]}, expansion={"killed": ["repo__Beta"]})) == \
        ["expansion: 1 rows killed (Beta)", "short: 1 rows cut by the wall clock (Alpha)"]
    assert report.guard_line(record()).startswith("publishable:")
    assert report.guard_line(record(designer={"status": "skipped"})) == \
        "not published: designer skipped (the night is summarised only)"


def test_sweep_numbers():
    hand = "# B\n\n## Layout sweep v20 (2026-10-23)\n\nx\n\n## Layout sweep v21 (2026-10-26)\n\ny\n\n## Other\n"
    assert report.sweep_number(hand, "2026-10-26") == 22
    assert report.published_nights(hand) == []
    assert report.previous_sweep(hand, 22) == (21, None, "build/github-layout-v21.log", None)
    marker = "<!-- nightly-sweep night=2026-10-27 sweep=build/nightly/2026-10-27/sweep.log main=abc -->"
    generated = hand.replace("## Other", f"## Layout sweep v22 (2026-10-27)\n\n{marker}\n\nz\n\n## Other")
    # The night that has a section keeps its number; the next one counts on.
    assert report.sweep_number(generated, "2026-10-27") == 22
    assert report.sweep_number(generated, "2026-10-28") == 23
    assert report.published_nights(generated) == ["2026-10-27"]
    assert report.previous_sweep(generated, 23) == (22, "2026-10-27", "build/nightly/2026-10-27/sweep.log", "abc")
    # Written again: in place, the rest kept.
    again = report.merge_section(generated, "2026-10-27", 22, "## Layout sweep v22 (2026-10-27)\n\nnew\n")
    assert again == hand.replace("## Other", "## Layout sweep v22 (2026-10-27)\n\nnew\n\n## Other")
    # A new one: after the newest numbered sweep with its subsections.
    text = "## Layout sweep v21 (x)\n\na\n\n### Sub\n\nb\n\n## Other\n"
    assert report.merge_section(text, "n", 22, "## Layout sweep v22 (n)\n") == \
        "## Layout sweep v21 (x)\n\na\n\n### Sub\n\nb\n\n## Layout sweep v22 (n)\n\n## Other\n"


def test_pending_nights(published, capsys):
    tmp_path, main = published
    nights = tmp_path / "build/nightly"
    text = (main / "docs/benchmarks.md").read_text()
    # Night 3 is published: nothing after it.
    assert report.pending(nights, text, min_results=3) == []
    # Without a published night only the newest counts.
    assert [r["night"] for r in report.pending(nights, DOCS["docs/benchmarks.md"], min_results=3)] == ["2026-10-24"]
    # After a published night every later one that passes the guard, oldest first.
    marker = "<!-- nightly-sweep night=2026-10-21 sweep=x main=y -->"
    older = DOCS["docs/benchmarks.md"].replace("Written by hand.", marker)
    assert [r["night"] for r in report.pending(nights, older, min_results=3)] == ["2026-10-22", "2026-10-24"]
    assert report.pending(nights, older) == []
    assert "fewer than 100" in capsys.readouterr().out


def test_the_nightly_publishes_its_night(published):
    tmp_path, main = published
    log = (tmp_path / "build/nightly/2026-10-24/nightly.log").read_text()
    assert "nightly 2026-10-24: publishable: every stage ran" in log
    assert "committed on main: nightly 2026-10-24: layout sweep v22, 1 of 3 strict" in log
    assert "pushed main" in log
    assert "report not committed (--no-commit-report" in (tmp_path / "build/nightly/2026-10-22/nightly.log").read_text()
    subject = git_in(main, "log", "-1", "--format=%s").strip()
    assert subject == ("nightly 2026-10-24: layout sweep v22, 1 of 3 strict (+0 against the nightly of 2026-10-22), "
                       "matched designer tier 1 strict, gap placement 40% / routing 60%")
    assert sorted(git_in(main, "show", "--name-only", "--format=", "HEAD").split()) == sorted(report.DOC_FILES)
    assert git_in(main, "rev-parse", "HEAD") == git_in(tmp_path / "origin.git", "rev-parse", "main")
    benchmarks = (main / "docs/benchmarks.md").read_text()
    # After v21 and its subsection, before the next section.
    assert benchmarks.index("By hand too.") < benchmarks.index("## Layout sweep v22 (2026-10-24)") \
        < benchmarks.index("## Other")
    section = section_of(benchmarks, "## Layout sweep v22 (2026-10-24)")
    assert "<!-- nightly-sweep night=2026-10-24 sweep=build/nightly/2026-10-24/sweep.log main=" + "3" * 40 in section
    # Gamma's project files changed: named and left out of both comparisons.
    flat = " ".join(section.split())
    assert "1 board ran on other inputs than in nightly 2026-10-22 (INPUT" in flat
    assert "1 board ran on other inputs than in v21 (INPUT" in flat
    assert "| | v21 | nightly 2026-10-22 (1111111) | v22 (nightly 2026-10-24, 3333333) |" in section
    assert "| pass, strict rule (rule 1) | 1 of 3 | 1 of 3 | **1 of 3** |" in section
    assert "| strict passes on the 2 boards of the same input as v21 | 1 | - | 1 |" in section
    paragraph = " ".join(section_of(benchmarks, "**Against the nightly of 2026-10-22**").split())
    assert "better 1, same 1, worse 0" in paragraph and "Better, the largest: repo Beta 2 -> 0." in paragraph
    assert "Strict passes 1 -> 1: gained 1: repo Beta (2 open -> none); lost 1: Alpha (clearance 2)." in paragraph
    assert "### Placement or routing on one binary (matched designer tier, nightly 2026-10-24)" in section
    assert "layout open 5, designer-route open 3: placement gap +2 (40 %), routing gap 3 (60 %)" in flat
    assert "| Gamma | 50 | 5 | 3 | +2 |" in section
    assert "beyond noise** (" in flat and "), 0: none. Losing the strict pass: Alpha (clearance 2)." in flat
    readme = (main / "README.md").read_text()
    flat = " ".join(readme.split())
    assert "Reduced to what an agent starts from - parts stacked - pcb-maker places and routes **1 of 3**" in flat
    assert "(sweep v22, the nightly of 2026-10-24)" in flat
    assert "([results](docs/benchmarks.md#layout-sweep-v22-2026-10-24))" in flat
    assert "- **At scale**: unrelated." in readme
    rows = [line for line in readme.splitlines() if line.startswith("| ")]
    assert rows[2].startswith("| Layout from stacked parts and constraints, 1500 s of work (v22, nightly 2026-10-24")
    assert rows[3].startswith("| Routing the designer's placement at the layout's routing budget, budget scale 0.7 "
                              "(matched designer tier, nightly 2026-10-24)")
    assert "(v21, x1, 2026-10-21)" in rows[4] and len(rows) == 7
    handover = (main / "docs/handover.md").read_text()
    assert handover.startswith("# Hand-over (state as of 2026-10-24; measured by layout sweep v22, the nightly of "
                               "2026-10-24 on main 3333333)\n")
    assert "**Nightly** 2026-10-24 (main 333333333333" in handover
    assert handover.index("**Layout sweep v21** (x1)") < handover.index("**Layout sweep v22** (the nightly of") \
        < handover.index("**Quick tier**")
    changelog = (main / "docs/changelog.md").read_text()
    assert changelog.index("## Committed on 2026-10-25") \
        < changelog.index("## Measured on 2026-10-24 (nightly: layout sweep v22, matched designer tier)") \
        < changelog.index("## Committed on 2026-10-23")
    for text in (benchmarks, readme, handover, changelog):
        # No wrapped line begins like a list item or a heading by accident.
        assert not any(line.startswith(("  - ", "  + ", "  # ")) for line in text.splitlines())


def test_publishing_again_changes_nothing(published, tmp_path):
    source, main = published
    copy = tmp_path / "copy"
    for path in report.DOC_FILES:
        (copy / path).parent.mkdir(parents=True, exist_ok=True)
        (copy / path).write_text((main / path).read_text())
    writeup = report.publish(copy, source / "build/nightly/2026-10-24", min_results=3)
    assert writeup.number == 22
    for path in report.DOC_FILES:
        assert (copy / path).read_text() == (main / path).read_text(), path
    # The guard: at 100 rows nothing is written.
    assert report.publish(copy, source / "build/nightly/2026-10-24") is None


def test_a_night_that_fails_the_guard_is_only_summarised(two_nights):
    log = (two_nights / "build/nightly/2026-10-23/nightly.log").read_text()
    assert ("nightly 2026-10-23: not published: expansion: 1 rows killed (Gamma); sweep: 1 rows cut by the wall "
            "clock (Gamma); sweep: 3 rows with a result, fewer than 100") in log
    # A test run commits only into PCB_NIGHTLY_CHECKOUT.
    assert "report not committed: a test run" in log


def test_the_commit_waits_for_a_clean_main(published, tmp_path, capsys):
    source, _ = published
    nights = source / "build/nightly"
    main = docs_checkout(tmp_path)
    start = git_in(main, "rev-parse", "HEAD")
    # A change to a tracked file (the supervisor's merge in progress): nothing written.
    (main / "other.txt").write_text("changed\n")
    assert not report.commit(nights, main, min_results=3)
    assert "main's working tree" in capsys.readouterr().out
    assert (main / "README.md").read_text() == DOCS["README.md"]
    git_in(main, "checkout", "--quiet", "--", "other.txt")
    # Untracked files do not count.
    (main / "untracked.txt").write_text("x\n")
    # origin moved on and main too: the pull cannot fast-forward.
    clone = tmp_path / "clone"
    subprocess.run(["git", "clone", "--quiet", str(tmp_path / "origin.git"), str(clone)], check=True)
    git_in(clone, "config", "user.email", "a@example.com")
    git_in(clone, "config", "user.name", "A")
    (clone / "other.txt").write_text("theirs\n")
    git_in(clone, "commit", "--quiet", "-am", "theirs")
    git_in(clone, "push", "--quiet", "origin", "main")
    (main / "other.txt").write_text("ours\n")
    git_in(main, "commit", "--quiet", "-am", "ours")
    assert not report.commit(nights, main, min_results=3)
    assert "git pull --ff-only origin main failed" in capsys.readouterr().out
    assert git_in(main, "status", "--porcelain", "--untracked-files=no") == ""
    # Back on origin's line: published, on top of the pulled commit.
    git_in(main, "reset", "--quiet", "--hard", start.strip())
    assert report.commit(nights, main, min_results=3)
    assert git_in(main, "log", "-2", "--format=%s").splitlines()[1] == "theirs"
    assert (main / "untracked.txt").exists()
    # Not on main: nothing.
    git_in(main, "checkout", "--quiet", "-b", "other")
    assert not report.commit(nights, main, min_results=3)
    assert "not main" in capsys.readouterr().out


def test_losers_beyond_noise():
    diff = {"worse": [("repo__video", 165, 190), ("repo__ATAT1800", 5, 19), ("repo__glasgow", 100, 160)]}
    beyond, within = report.losers(diff, {"repo__video": 193, "repo__glasgow": 49})
    assert beyond == [("repo__ATAT1800", 5, 19, None), ("repo__glasgow", 100, 160, 49)]
    assert within == [("repo__video", 165, 190, 193)]
    spreads = report.noise_spreads()
    assert spreads["Huaqiu-Electronics__ecad-viewer__video"] == 193


def test_names_and_wrapping():
    assert report.name_of("CRImier__MyKiCad__protoesp") == "protoesp"
    assert report.name_of("GlasgowEmbedded__glasgow__glasgow__revC3") == "glasgow revC3"
    assert report.name_of("0xCB-dev__0xCB-1337__pcb-panel") == "0xCB-1337 pcb-panel"
    assert report.name_of("OpenDrone-hw__OpenRX__OpenRX-panel-rev2") == "OpenRX-panel-rev2"
    assert report.name_of("byrantech__laptop__power") == "laptop power"
    text = report.wrap("word " * 13 + "- dash and 57 % and " + "x " * 30 + "1. not a list")
    for line in text.splitlines():
        assert not line.startswith(("- ", "1. ", "% "))
    assert "57 %" in text and " " not in text
