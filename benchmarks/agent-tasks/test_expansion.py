# Copyright (C) 2026 Toit contributors.
"""Tests of the expansion tier's route-mode tasks in run.py (task list,
strip arguments, fetch verification, the intake's outcome classes):

    python3 -I -m unittest discover -s benchmarks/agent-tasks -p 'test_expansion.py'"""

import hashlib
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import run  # noqa: E402
import os

# The tests' own memory ledger, not the machine's (harness_common.Ledger;
# their subprocesses inherit it).
os.environ.setdefault("PCB_MAKER_LEDGER", str(Path(tempfile.gettempdir()) / "pcb-maker-test-ledger.json"))

ROOT = HERE.parents[1]


def digest(data):
    return hashlib.sha256(data).hexdigest()


BOARD = b"(kicad_pcb (version 20240108))\n"
PROJECT = b'{"board": {}}\n'


def task(license="MIT", files=None, path="hw/main/demo.kicad_pcb"):
    return {"name": "owner__repo__demo", "mode": "route", "board_id": "demo", "budget_seconds": 30,
            "source": {"repository": "owner/repo", "commit": "0123456789abcdef", "path": path,
                       "url": "https://github.com/owner/repo/blob/0123456789abcdef/" + path,
                       "license": license, "harvest_name": "owner_repo_demo__abc",
                       "files": files or {"demo.kicad_pcb": digest(BOARD), "demo.kicad_pro": digest(PROJECT)}}}


class FakeGitHub:
    """GitHub.raw's signature; serves `files` by repository path."""

    def __init__(self, files):
        self.files = files
        self.calls = []

    def __call__(self, repository, commit, path, maximum):
        self.calls.append((repository, commit, path))
        return self.files.get(path)


class TaskList(unittest.TestCase):
    def setUp(self):
        self.tier = json.loads((HERE / "expansion.json").read_text())
        self.tasks = json.loads((ROOT / self.tier["tasks"]).read_text())["tasks"]

    def test_the_tier_names_every_task_once_at_scale_one(self):
        names = [entry["name"] for entry in self.tasks]
        self.assertEqual(len(names), 46)
        self.assertEqual(len(set(names)), 46)
        self.assertEqual(set(names), set(self.tier["boards"]))
        self.assertEqual(self.tier["budget_scale"], 1)
        self.assertIn("expansion", run.TIERS)

    def test_every_task_routes_a_verified_licensed_board(self):
        for entry in self.tasks:
            with self.subTest(entry["name"]):
                self.assertEqual(entry["mode"], "route")
                self.assertEqual(entry["budget_seconds"], 30)
                self.assertTrue(entry["designer_budget"])
                self.assertNotIn("constraints", entry)
                source = entry["source"]
                self.assertNotIn(source["license"], run.NO_LICENCE)
                self.assertRegex(source["commit"], r"^[0-9a-f]{40}$")
                self.assertIn(source["commit"], source["url"])
                self.assertTrue(source["path"].endswith(entry["board_id"] + ".kicad_pcb"))
                self.assertIn(entry["board_id"] + ".kicad_pcb", source["files"])
                for name, sha in source["files"].items():
                    self.assertTrue(name.startswith(entry["board_id"] + "."))
                    self.assertRegex(sha, r"^[0-9a-f]{64}$")
                self.assertEqual(entry["directory"], f"build/github-expansion/{entry['name']}/source")

    def test_the_router_gets_the_intakes_settings(self):
        config = run.route_task_config(self.tasks[0], 1)
        self.assertEqual(config, {"seeds": 1, "skip_native_verification": True,
                                  "ladder_budget_seconds": 30, "deadline_seconds": 90.0})
        deep = run.route_task_config(self.tasks[0], 10)
        self.assertEqual((deep["ladder_budget_seconds"], deep["deadline_seconds"]), (300, 900.0))
        self.assertEqual((run.intake_timeout(30), run.intake_timeout(300)), (180.0, 960.0))

    def test_a_task_without_budget_seconds_keeps_the_route_default(self):
        config = run.route_task_config({"router": {"add_pours": True}}, 0.5)
        self.assertEqual(config, {"add_pours": True, "ladder_budget_seconds": 600.0})


class Strip(unittest.TestCase):
    def test_the_intakes_arguments(self):
        self.assertEqual(run.strip_command(Path("/bin/pcb"), Path("/a/b.kicad_pcb"), Path("/c/b.kicad_pcb")),
                         ["/bin/pcb", "strip-kicad-tracks", "/a/b.kicad_pcb", "/c/b.kicad_pcb"])

    def test_the_intake_strips_with_the_same_arguments(self):
        # triage.py's call: the binary, the command, the original, the copy.
        triage = (ROOT / "benchmarks/expansion/triage.py").read_text()
        call = re.search(r"run_process\(\[binary, 'strip-kicad-tracks', ([^\]]*)\]", triage)
        self.assertIsNotNone(call)
        self.assertEqual([part.strip() for part in call.group(1).split(",")],
                         ["original", "source / (board_id + '.kicad_pcb')"])


class Fetch(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.root = Path(self.scratch.name)
        self.directory = self.root / "build/github-expansion/owner__repo__demo/source"

    def tearDown(self):
        self.scratch.cleanup()

    def test_fetches_from_github_next_to_the_board(self):
        github = FakeGitHub({"hw/main/demo.kicad_pcb": BOARD, "hw/main/demo.kicad_pro": PROJECT})
        self.assertEqual(run.ensure_source(task(), self.directory, None, github), "github")
        self.assertEqual((self.directory / "demo.kicad_pcb").read_bytes(), BOARD)
        self.assertEqual((self.directory / "demo.kicad_pro").read_bytes(), PROJECT)
        self.assertEqual({call[1] for call in github.calls}, {"0123456789abcdef"})
        self.assertEqual(sorted(call[2] for call in github.calls), ["hw/main/demo.kicad_pcb", "hw/main/demo.kicad_pro"])
        self.assertFalse(self.directory.with_name("source.partial").exists())

    def test_a_board_at_the_repository_root(self):
        github = FakeGitHub({"demo.kicad_pcb": BOARD, "demo.kicad_pro": PROJECT})
        self.assertEqual(run.ensure_source(task(path="demo.kicad_pcb"), self.directory, None, github), "github")

    def test_a_present_verified_directory_is_not_fetched_again(self):
        github = FakeGitHub({"hw/main/demo.kicad_pcb": BOARD, "hw/main/demo.kicad_pro": PROJECT})
        run.ensure_source(task(), self.directory, None, github)
        github.calls.clear()
        self.assertIsNone(run.ensure_source(task(), self.directory, None, github))
        self.assertEqual(github.calls, [])

    def test_a_changed_file_on_github_is_refused_and_nothing_is_kept(self):
        github = FakeGitHub({"hw/main/demo.kicad_pcb": BOARD + b"changed", "hw/main/demo.kicad_pro": PROJECT})
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            run.ensure_source(task(), self.directory, None, github)
        self.assertFalse(self.directory.exists())
        self.assertFalse(self.directory.with_name("source.partial").exists())

    def test_a_missing_file_is_refused(self):
        github = FakeGitHub({"hw/main/demo.kicad_pcb": BOARD})
        with self.assertRaisesRegex(ValueError, "not in owner/repo"):
            run.ensure_source(task(), self.directory, None, github)
        self.assertFalse(self.directory.exists())

    def test_a_corrupted_local_copy_is_fetched_again(self):
        github = FakeGitHub({"hw/main/demo.kicad_pcb": BOARD, "hw/main/demo.kicad_pro": PROJECT})
        run.ensure_source(task(), self.directory, None, github)
        (self.directory / "demo.kicad_pcb").write_bytes(b"edited")
        self.assertEqual(run.ensure_source(task(), self.directory, None, github), "github")
        self.assertEqual((self.directory / "demo.kicad_pcb").read_bytes(), BOARD)

    def test_the_harvests_copy_first_when_it_matches(self):
        harvest = self.root / "harvest/boards"
        (harvest / "owner_repo_demo__abc").mkdir(parents=True)
        (harvest / "owner_repo_demo__abc/demo.kicad_pcb").write_bytes(BOARD)
        (harvest / "owner_repo_demo__abc/demo.kicad_pro").write_bytes(PROJECT)
        (harvest / "owner_repo_demo__abc/other.txt").write_bytes(b"not copied")
        github = FakeGitHub({})
        self.assertEqual(run.ensure_source(task(), self.directory, harvest, github), "harvest")
        self.assertEqual(github.calls, [])
        self.assertEqual(sorted(path.name for path in self.directory.iterdir()), ["demo.kicad_pcb", "demo.kicad_pro"])

    def test_a_harvest_copy_that_differs_goes_to_github(self):
        harvest = self.root / "harvest/boards"
        (harvest / "owner_repo_demo__abc").mkdir(parents=True)
        (harvest / "owner_repo_demo__abc/demo.kicad_pcb").write_bytes(b"another revision")
        (harvest / "owner_repo_demo__abc/demo.kicad_pro").write_bytes(PROJECT)
        github = FakeGitHub({"hw/main/demo.kicad_pcb": BOARD, "hw/main/demo.kicad_pro": PROJECT})
        self.assertEqual(run.ensure_source(task(), self.directory, harvest, github), "github")

    def test_no_licence_no_fetch(self):
        for licence in (None, "", "NOASSERTION"):
            with self.subTest(licence=licence):
                github = FakeGitHub({"hw/main/demo.kicad_pcb": BOARD, "hw/main/demo.kicad_pro": PROJECT})
                entry = task(license=licence)
                with self.assertRaises(run.FetchRefused):
                    run.ensure_source(entry, self.directory, None, github)
                self.assertEqual(github.calls, [])
                self.assertFalse(self.directory.exists())
        entry = task()
        del entry["source"]["license"]
        with self.assertRaises(run.FetchRefused):
            run.ensure_source(entry, self.directory, None, FakeGitHub({}))


class Outcome(unittest.TestCase):
    clean = {"errors": {}, "unconnected": 0}

    def report(self, **fields):
        result = {"unconnected_terminals": 0, "routed_connections": 10, "routable_connections": 10,
                  "routing_seconds": 12.0, "work_seconds": 20.0}
        result.update(fields)
        return result

    def test_complete_routes_by_routing_seconds(self):
        self.assertEqual(run.intake_outcome(self.report(routing_seconds=2), self.clean, 30), "clean_fast")
        self.assertEqual(run.intake_outcome(self.report(), self.clean, 30), "clean_medium")
        self.assertEqual(run.intake_outcome(self.report(routing_seconds=45, cut=["deadline"]), self.clean, 30),
                         "clean_slow")

    def test_complete_but_kicad_disagrees(self):
        self.assertEqual(run.intake_outcome(self.report(), {"errors": {"clearance": 1}, "unconnected": 0}, 30),
                         "native_drc_failure")
        self.assertEqual(run.intake_outcome(self.report(), None, 30), "verification_unknown")

    def test_incomplete_routes(self):
        open_ = {"unconnected_terminals": 2, "routed_connections": 9}
        self.assertEqual(run.intake_outcome(self.report(cut=["deadline"], **open_), self.clean, 30), "cut_wall_clock")
        self.assertEqual(run.intake_outcome(self.report(work_seconds=29, **open_), self.clean, 30),
                         "incomplete_at_budget")
        self.assertEqual(run.intake_outcome(self.report(work_seconds=5, routing_seconds=4, **open_), self.clean, 30),
                         "incomplete_early")
        self.assertEqual(run.intake_outcome(self.report(work_seconds=5, routing_seconds=4,
                                                        internal_violations=[{}], **open_), self.clean, 30),
                         "internal_drc_failure")


if __name__ == "__main__":
    unittest.main()
