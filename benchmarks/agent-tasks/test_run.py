# Copyright (C) 2026 Toit contributors.
"""Tests of run.py's defaults: python3 -m unittest benchmarks/agent-tasks/test_run.py"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run  # noqa: E402


class Parallelism(unittest.TestCase):
    def test_a_tier_without_jobs_or_threads_runs_its_default(self):
        jobs, threads, source = run.parallelism(True, None, None, 16)
        self.assertEqual((jobs * threads, threads, source), (16, run.TIER_THREADS, "tier default"))

    def test_either_given_keeps_the_older_defaults(self):
        self.assertEqual(run.parallelism(True, 6, None, 16), (6, None, "command line"))
        self.assertEqual(run.parallelism(True, None, 4, 16), (1, 4, "command line"))
        self.assertEqual(run.parallelism(True, 6, 4, 16), (6, 4, "command line"))
        self.assertEqual(run.default_threads(6, 16), 2)

    def test_without_a_tier_one_job(self):
        self.assertEqual(run.parallelism(False, None, None, 16), (1, None, "command line"))

    def test_at_least_one_job(self):
        jobs, _, _ = run.parallelism(True, None, None, 1)
        self.assertGreaterEqual(jobs, 1)


if __name__ == "__main__":
    unittest.main()
