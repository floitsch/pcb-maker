#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Regression gates for lossy format conversion; no KiCad/network needed."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("board_convert", Path(__file__).with_name("convert.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ConversionTests(unittest.TestCase):
    def test_timeout_kills_process(self):
        result = MODULE.command([sys.executable, "-c", "import time; time.sleep(60)"], 0.05)
        self.assertTrue(result["timeout"])
        self.assertNotEqual(result["exit_code"], 0)
        self.assertLess(result["seconds"], 2)

    def test_source_entity_declarations_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "source.brd"
            source.write_text('<!DOCTYPE eagle [<!ENTITY x "test">]><eagle/>')
            with self.assertRaisesRegex(ValueError, "entity"):
                MODULE.eagle_inventory(source)

    def test_clean_import_report_cannot_hide_undefined_layers(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "source.PcbDoc"
            source.write_bytes(b"source must not change")
            output = Path(temporary) / "converted"
            def fake_command(arguments, timeout):
                if "import" in arguments:
                    (output / "board.kicad_pcb").write_text('(kicad_pcb (fp_line (layer "UNDEFINED")))')
                    (output / "import.json").write_text('{"errors": [], "warnings": []}')
                else:
                    self.assertEqual(arguments[-1], "version")
                return {"exit_code": 0, "stdout": "10.0.7", "timeout": False}
            with patch.object(MODULE, "command", side_effect=fake_command):
                result = MODULE.convert_board(source, output, "altium")
            self.assertEqual(result["status"], "invalid_conversion")
            self.assertFalse(result["eligible_as_routing_label"])
            self.assertEqual(source.read_bytes(), b"source must not change")
            self.assertEqual(json.loads((output / "conversion.json").read_text())["status"], "invalid_conversion")

    def test_native_baseline_does_not_certify_semantic_fidelity(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "source.PcbDoc"
            source.write_bytes(b"source")
            output = Path(temporary) / "converted"
            def fake_command(arguments, timeout):
                if "import" in arguments:
                    (output / "board.kicad_pcb").write_text('(kicad_pcb)')
                    (output / "import.json").write_text('{"errors": [], "warnings": []}')
                elif "stats" in arguments:
                    (output / "stats.json").write_text('{}')
                elif "drc" in arguments:
                    (output / "drc.json").write_text('{"violations": [], "unconnected_items": []}')
                return {"exit_code": 0, "stdout": "10.0.7", "timeout": False}
            with patch.object(MODULE, "command", side_effect=fake_command):
                result = MODULE.convert_board(source, output, "altium")
            self.assertEqual(result["status"], "imported_unvalidated")
            self.assertEqual(result["semantic_validation"], "unvalidated")
            self.assertFalse(result["eligible_as_routing_label"])


if __name__ == "__main__":
    unittest.main()
