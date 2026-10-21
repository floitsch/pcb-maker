# Copyright (C) 2026 Toit contributors.
"""Tests of the licence classes and harvest.py's gate:
python3 -m pytest benchmarks/github/test_licences.py"""

import importlib.util
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import licences  # noqa: E402

# Under its own name: benchmarks/expansion has a harvest.py too.
_spec = importlib.util.spec_from_file_location("github_harvest", HERE / "harvest.py")
harvest = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(harvest)


class Classes(unittest.TestCase):
    def test_spdx_identifiers(self):
        self.assertEqual(licences.record("MIT")["kind"], "permissive")
        self.assertEqual(licences.record("CERN-OHL-P-2.0")["kind"], "permissive")
        self.assertEqual(licences.record("CERN-OHL-W-2.0")["kind"], "weak-copyleft")
        self.assertEqual(licences.record("CERN-OHL-S-2.0")["kind"], "copyleft")
        self.assertEqual(licences.record("GPL-3.0-or-later")["kind"], "copyleft")
        self.assertEqual(licences.record("CC-BY-SA-4.0")["kind"], "copyleft")
        self.assertEqual(licences.record("Other")["kind"], "unlicensed")

    def test_licence_files_where_github_names_none(self):
        self.assertEqual(licences.record(None, [])["kind"], "unlicensed")
        self.assertFalse(licences.record("NOASSERTION", [])["allowed"])
        mit = [("LICENSE", "Permission is hereby granted, free of charge, to any person")]
        self.assertEqual(licences.record("NOASSERTION", mit)["kind"], "permissive")
        cern = [("LICENSE.md", "CERN Open Hardware Licence Version 2")]
        self.assertEqual((licences.record(None, cern)["kind"], licences.record(None, cern)["allowed"]),
                         ("unclassified", True))
        reserved = [("LICENSE", "Copyright 2024. All rights reserved.")]
        self.assertFalse(licences.record(None, reserved)["allowed"])
        nc = [("LICENSE", "Creative Commons Attribution-NonCommercial 4.0")]
        self.assertEqual(licences.record(None, nc)["kind"], "restricted")

    def test_training_is_permissive_only(self):
        for spdx, eligible in (("MIT", True), ("Apache-2.0", True), ("CC0-1.0", True), ("CERN-OHL-W-2.0", False),
                               ("GPL-2.0", False), ("CERN-OHL-S-2.0", False), (None, False), ("NOASSERTION", False)):
            self.assertEqual(licences.spdx_training_eligible(spdx), eligible, spdx)
            self.assertEqual(licences.record(spdx, [])["training"], eligible, spdx)


class Gate(unittest.TestCase):
    def test_benchmarks_need_a_licence_unless_allowed_unlicensed(self):
        unlicensed = licences.record(None, [])
        self.assertIn("all rights reserved", harvest.licence_refusal(unlicensed, False, False))
        self.assertIsNone(harvest.licence_refusal(unlicensed, False, True))
        self.assertIsNone(harvest.licence_refusal(licences.record("GPL-3.0"), False, False))

    def test_training_data_needs_a_permissive_licence(self):
        self.assertIn("permissive only", harvest.licence_refusal(licences.record("GPL-3.0"), True, True))
        self.assertIsNone(harvest.licence_refusal(licences.record("MIT"), True, False))

    def test_licence_files_are_read_only_without_an_spdx_identifier(self):
        read = []

        def texts(repository, commit, path):
            read.append(path)
            return [("LICENSE", "Permission is hereby granted")]

        self.assertEqual(harvest.board_licence({"license": {"spdx_id": "MIT"}}, "o/r", "c", "a/b.kicad_pcb", texts)
                         ["kind"], "permissive")
        self.assertEqual(read, [])
        record = harvest.board_licence({"license": None}, "o/r", "c", "a/b.kicad_pcb", texts)
        self.assertEqual((record["kind"], record["files"], read), ("permissive", ["LICENSE"], ["a/b.kicad_pcb"]))

    def test_the_committed_corpus_is_marked(self):
        import json
        boards = json.loads((HERE / "boards.json").read_text())["boards"]
        self.assertTrue(all(board["licence"]["kind"] in licences.KINDS for board in boards))
        self.assertTrue(all(board["licence"]["training"] == (board["licence"]["kind"] == "permissive")
                            for board in boards))


if __name__ == "__main__":
    unittest.main()
