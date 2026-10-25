# Copyright (C) 2026 Toit contributors.
import json
import tempfile
import unittest
from pathlib import Path

import tier_estimates


class TierEstimates(unittest.TestCase):
    def test_the_largest_measured_peak_and_seconds_replace_the_estimate(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "designer.log"
            rows = [{"name": "a", "placement": "designer", "peak_rss_mb": 700, "seconds": 99.1},
                    {"name": "a", "placement": "designer", "peak_rss_mb": 1500, "seconds": 80.0},
                    {"name": "a", "placement": "auto", "peak_rss_mb": 9000, "seconds": 900.0},
                    {"name": "b", "placement": "designer", "peak_rss_mb": None}]
            log.write_text("run.py: a line that is not a row\n" + "".join(json.dumps(r) + "\n" for r in rows))
            found = tier_estimates.measured([log], "designer")
            self.assertEqual(found, {"a": {"peak_rss_mb": 1500, "seconds": 99.1, "rows": 2}})
            tier = {"boards": {"a": {"peak_rss_mb": 23215, "seconds": 1523.8, "v17": {"routed": "68/70"}},
                               "b": {"peak_rss_mb": 300, "seconds": 10.0}}}
            self.assertEqual(tier_estimates.update(tier, found), [("a", 23215, 1500)])
            self.assertEqual(tier["boards"]["a"], {"peak_rss_mb": 1500, "seconds": 99.1, "v17": {"routed": "68/70"}})
            self.assertEqual(tier["boards"]["b"], {"peak_rss_mb": 300, "seconds": 10.0})


if __name__ == "__main__":
    unittest.main()
