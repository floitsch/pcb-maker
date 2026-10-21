#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Network-free checks for dataset identity, admission, resume and provenance."""

import argparse
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("expansion_harvest", Path(__file__).with_name("harvest.py"))
harvest = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(harvest)

BOARD = '''(kicad_pcb (version 20240108)
 (layers
  (0 "F.Cu" signal)
  (31 "B.Cu" signal)
  (44 "Edge.Cuts" user)
 )
 (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts"))
''' + ' (footprint "R")\n' * 8 + ' (segment (start 0 0) (end 1 1) (width 0.2) (layer "F.Cu") (net 1))\n' * 8 + ')'


class FakeGitHub:
    blocked_until = 0

    def __init__(self):
        self.calls = []
        self.fail_project = False

    def api(self, endpoint):
        self.calls.append(endpoint)
        if endpoint == "repos/example/design":
            return {"default_branch": "main", "license": {"spdx_id": "MIT"}, "stargazers_count": 9}
        if endpoint == "repos/example/design/commits/main":
            return {"sha": "pinned-commit", "commit": {"tree": {"sha": "tree-sha"}}}
        if endpoint.endswith("tree-sha?recursive=1"):
            return {"tree": [{"type": "blob", "path": "a/board.kicad_pcb", "sha": "blob-a", "size": 50000},
                             {"type": "blob", "path": "b/board.kicad_pcb", "sha": "blob-b", "size": 60000}]}
        raise AssertionError(endpoint)

    def raw(self, repository, commit, path, maximum):
        assert commit == "pinned-commit"
        self.calls.append(path)
        if path.endswith(".kicad_pcb"):
            return BOARD.encode()
        if path.endswith(".kicad_pro") and self.fail_project:
            self.fail_project = False
            raise TimeoutError("transient project failure")
        return None


class HarvestTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.directory = Path(self.temporary.name)
        self.seed = self.directory / "seed.json"
        self.seed.write_text(json.dumps([{"repository": "example/design", "path": "a/board.kicad_pcb"}]))
        self.args = argparse.Namespace(data=self.directory / "data", seed=self.seed,
                                      refresh_hours=168, max_bytes=40_000_000, retry_after=0,
                                      workers=2, limit=0, non_kicad=False, command="fetch",
                                      scan_limit=1, scan_batch=1)
        self.patch = mock.patch.object(harvest, "load_old", return_value=(set(), set()))
        self.patch.start()
        self.github = FakeGitHub()
        self.collector = harvest.Harvest(self.args, self.github)

    def tearDown(self):
        self.patch.stop()
        self.temporary.cleanup()

    def test_edge_layer_declaration_is_not_outline(self):
        layer_only = BOARD.replace(' (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts"))\n', '')
        self.assertEqual(harvest.admission(harvest.features(layer_only)), "no outline geometry")
        self.assertIsNone(harvest.admission(harvest.features(BOARD)))

    def test_fetch_pins_commit_preserves_inferred_rules_and_resumes(self):
        self.collector.fetch()
        manifest = json.loads((self.args.data / "manifest.json").read_text())
        self.assertEqual(len(manifest["boards"]), 1)
        board = manifest["boards"][0]
        self.assertEqual(board["commit"], "pinned-commit")
        self.assertTrue(board["rules_inferred"])
        self.assertTrue(board["nn_license_eligible"])
        self.assertTrue(Path(board["board_path"]).exists())
        self.assertEqual(Path(board["board_path"]).read_text(), BOARD)
        calls = len(self.github.calls)
        resumed = harvest.Harvest(self.args, self.github)
        resumed.fetch()
        self.assertEqual(len(self.github.calls), calls)

    def test_tree_scan_discovers_other_boards_in_already_known_repository(self):
        self.collector.scan("example/design")
        self.assertIn("example/design:b/board.kicad_pcb", self.collector.candidates)
        self.collector.fetch()
        statuses = [record["status"] for record in self.collector.outcomes.values()]
        self.assertCountEqual(statuses, ["downloaded", "duplicate"])
        self.assertEqual(len(self.collector.boards), 1)
        duplicate = next(record for record in self.collector.outcomes.values() if record["status"] == "duplicate")
        self.assertTrue(Path(duplicate["board_path"]).exists())

    def test_transient_sidecar_error_can_retry_without_false_duplicate(self):
        self.github.fail_project = True
        self.collector.fetch()
        self.assertEqual(next(iter(self.collector.outcomes.values()))["status"], "error")
        self.collector.fetch()
        self.assertEqual(next(iter(self.collector.outcomes.values()))["status"], "downloaded")

    def test_previous_board_hash_is_excluded_but_new_sources_retained(self):
        import hashlib
        self.collector.hashes.add(hashlib.sha256(BOARD.encode()).hexdigest())
        self.collector.fetch()
        self.assertEqual(next(iter(self.collector.outcomes.values()))["status"], "duplicate")
        self.assertEqual(self.collector.boards, {})

    def test_safe_names_distinguish_same_stem_and_cannot_escape_store(self):
        first = harvest.safe_name("owner/repo:a/board.kicad_pcb")
        second = harvest.safe_name("owner/repo:b/board.kicad_pcb")
        self.assertNotEqual(first, second)
        self.assertNotIn("/", harvest.safe_name("../../../unsafe:$path"))

    def test_limited_runs_start_with_different_repositories(self):
        candidates = []
        for repository, paths in (("example/a", ["main.kicad_pcb", "backup/old.kicad_pcb"]),
                                  ("example/b", ["main.kicad_pcb"])):
            for path in paths:
                candidates.append({"source_id": f"{repository}:{path}", "repository": repository,
                                   "path": path, "size": 500_000})
        ordered = harvest.diverse_candidates(candidates)
        self.assertEqual(len({item["repository"] for item in ordered[:2]}), 2)
        self.assertIn("historical_or_backup_variant", harvest.quality_flags(ordered[-1]["path"]))

    def test_truncated_recursive_tree_falls_back_to_all_subtrees(self):
        class TruncatedGitHub(FakeGitHub):
            def api(self, endpoint):
                if endpoint.endswith("root?recursive=1"):
                    return {"truncated": True, "tree": []}
                if endpoint.endswith("/root"):
                    return {"tree": [{"type": "tree", "path": "hardware", "sha": "child"}]}
                if endpoint.endswith("/child"):
                    return {"tree": [{"type": "blob", "path": "board.kicad_pcb", "sha": "board-sha"}]}
                raise AssertionError(endpoint)
        self.collector.github = TruncatedGitHub()
        entries = self.collector.tree("example/design", "root")
        self.assertEqual(entries[0]["path"], "hardware/board.kicad_pcb")

    def test_unknown_license_keeps_board_but_disables_training_license_flag(self):
        original = self.github.api
        def api(endpoint):
            result = original(endpoint)
            if endpoint == "repos/example/design":
                result["license"] = None
            return result
        self.github.api = api
        self.collector.fetch()
        board = next(iter(self.collector.boards.values()))
        self.assertFalse(board["nn_license_eligible"])
        self.assertEqual(board["license_spdx"], "NOASSERTION")

    def test_copyleft_license_keeps_board_but_not_for_training(self):
        original = self.github.api
        def api(endpoint):
            result = original(endpoint)
            if endpoint == "repos/example/design":
                result["license"] = {"spdx_id": "GPL-3.0"}
            return result
        self.github.api = api
        self.collector.fetch()
        board = next(iter(self.collector.boards.values()))
        self.assertFalse(board["nn_license_eligible"])
        self.assertTrue(harvest.nn_license_eligible("CERN-OHL-P-2.0"))
        self.assertFalse(harvest.nn_license_eligible("CERN-OHL-W-2.0"))

    def test_partial_journal_tail_recovers_on_next_append(self):
        journal = self.args.data / "harvest.jsonl"
        journal.write_text('{"partial":')
        self.collector.fetch()
        resumed = harvest.Harvest(self.args, self.github)
        self.assertEqual(len(resumed.boards), 1)


if __name__ == "__main__":
    unittest.main()
