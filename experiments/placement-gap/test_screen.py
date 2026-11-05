#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Tests of screen.py's units and its on-side environment (nothing runs).

    python3 -I -m unittest experiments/placement-gap/test_screen.py
"""

import argparse
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import screen  # noqa: E402


def arguments(on_env=None, sides="off,on"):
    return argparse.Namespace(on="{}", on_env=on_env, sides=sides, out="/nonexistent/out",
                              off_binary="/bin/off", on_binary="/bin/on")


class OnEnvironment(unittest.TestCase):
    def test_pairs(self):
        self.assertEqual(screen.on_environment(["PCB_A=1", "PCB_B=x=y"]), {"PCB_A": "1", "PCB_B": "x=y"})
        self.assertEqual(screen.on_environment(None), {})
        with self.assertRaises(ValueError):
            screen.on_environment(["PCB_A"])
        with self.assertRaises(ValueError):
            screen.on_environment(["=1"])

    def test_environment(self):
        base = {"PATH": "/usr/bin", "PCB_A": "stale"}
        self.assertEqual(screen.environment(base, {"PCB_A": "1"}), {"PATH": "/usr/bin", "PCB_A": "1"})
        self.assertEqual(screen.environment(base, {"PCB_A": None}), {"PATH": "/usr/bin"})
        self.assertEqual(base, {"PATH": "/usr/bin", "PCB_A": "stale"})


class Units(unittest.TestCase):
    def test_sides_get_their_environment(self):
        result = screen.units(arguments(["PCB_PLACER_SKIP_ANNEAL=1"]))
        self.assertEqual(len(result), 2 * len(screen.SEEDS) * len(screen.DENSE))
        for name, command, extra in result:
            if "-on-" in name:
                self.assertEqual(extra, {"PCB_PLACER_SKIP_ANNEAL": "1"})
                self.assertIn("/bin/on", command)
            else:
                self.assertEqual(extra, {"PCB_PLACER_SKIP_ANNEAL": None})
                self.assertIn("/bin/off", command)

    def test_no_environment(self):
        for _, _, extra in screen.units(arguments(sides="on")):
            self.assertEqual(extra, {})


if __name__ == "__main__":
    unittest.main()
