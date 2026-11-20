# Copyright (C) 2026 Florian Loitsch.

import importlib.util
import json
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("published_layout_runner", Path(__file__).with_name("published-layout.py"))
published = importlib.util.module_from_spec(spec)
spec.loader.exec_module(published)


class PublishedLayoutTests(unittest.TestCase):
    def test_complete_pinned_archives_in_any_order_and_source_hash(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            schematic, settings = b"schematic", b"project"
            source = root / "case.kicad_sch"
            source.write_bytes(schematic)
            manifest = {
                "board_id": "case",
                "source_files": {source.name: {"sha256": published.digest(schematic)}},
                "returned_project_sha256": published.digest(settings),
                "candidates": [],
            }
            files = [root / f"archive-{i}.zip" for i in (1, 2)]
            for index, path in enumerate(files, 1):
                with zipfile.ZipFile(path, "w") as archive:
                    archive.writestr("case.kicad_pcb", f"candidate {index}")
                    archive.writestr("case.kicad_pro", settings)
                manifest["candidates"].append({"id": str(index), "sha256": published.digest(path.read_bytes())})
            source_files, boards, paths = published.verified_inputs(manifest, root, files[::-1])
            self.assertEqual(source_files, {source.name: schematic})
            self.assertEqual(list(boards), ["1", "2"])
            self.assertEqual(boards["1"], (b"candidate 1", settings))
            self.assertEqual(paths["2"], str(files[1]))
            with self.assertRaisesRegex(ValueError, "supply all 2 distinct"):
                published.verified_inputs(manifest, root, files[:1])
            with self.assertRaisesRegex(ValueError, "duplicate archive"):
                published.verified_inputs(manifest, root, files[:1] * 2)
            files[1].write_bytes(b"replaced archive")
            with self.assertRaisesRegex(ValueError, "outside the pinned June 10 set"):
                published.verified_inputs(manifest, root, files)
            source.write_bytes(b"replaced schematic")
            with self.assertRaisesRegex(ValueError, "source checksum mismatch"):
                published.verified_inputs(manifest, root, files)

    def test_native_counts_include_raw_errors_and_mark_a_capped_category(self):
        with tempfile.TemporaryDirectory() as folder:
            work = Path(folder)
            board_name = "case.kicad_pcb"
            pcb, settings = b"board", b"rules"
            board = published.write_project(work / "project", "case", b"schematic", pcb, settings)
            report = {
                "source": board_name, "coordinate_units": "mm", "kicad_version": "10.0.8",
                "included_severities": ["error", "warning", "exclusion"],
                "violations": [
                    *[{"type": "track_width", "severity": "error"}] * 199,
                    {"type": "solder_mask_bridge", "severity": "error",
                     "items": [{"description": "Track [<no net>] on F.Cu"}]},
                    {"type": "silk_overlap", "severity": "error"},
                    {"type": "clearance", "severity": "warning"},
                ],
                "unconnected_items": [{"type": "unconnected_items", "severity": "error"}],
                "schematic_parity": [{"type": "net_conflict", "severity": "error", "description": "exact text"}],
            }
            (work / "drc.json").write_text(json.dumps(report))
            command = subprocess.CompletedProcess([], 0, "Found 1 schematic parity issues\n", "")
            with patch("comparison_scoring.run_command", return_value=command) as run:
                counts, parity = published.score(board, pcb, "case", settings, "kicad-cli", {}, work, 10)
            self.assertEqual(counts["drc_errors"], 201)
            self.assertEqual(counts["raw_drc_errors_by_type"], {"track_width": 199, "solder_mask_bridge": 1, "silk_overlap": 1})
            self.assertEqual(counts["capped_error_types"], ["track_width"])
            self.assertEqual(counts["mask_bridge_errors_with_unassigned_items"], 1)
            self.assertEqual(counts["unconnected"], 1)
            self.assertEqual(parity, {("net_conflict", "exact text"): 1})
            self.assertIn("--all-track-errors", run.call_args.args[0])
            self.assertIn("--refill-zones", run.call_args.args[0])


if __name__ == "__main__":
    unittest.main()
