# Copyright (C) 2026 Toit contributors.
"""ablation.py's attribution on a made-up run: baselines with nulls and
replicates, groups from both sides, the signal check."""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ablation  # noqa: E402

TASK = "owner__repo__board"


def signals(wirelength, crossings):
    return {"wirelength_mm": wirelength, "channel_overflow": 0.0, "blocked_exits": 0,
            "ratsnest_crossings": crossings, "large_crossings": 0, "global": None}


def make_run(tmp_path, opens, variant_signals):
    out = tmp_path / "run"
    (out / "hybrids").mkdir(parents=True)
    variants = [{"name": name, "signals": value} for name, value in variant_signals.items()]
    groups = [{"name": "large-U1", "kind": "large", "members": ["U1"]},
              {"name": "large-U2", "kind": "large", "members": ["U2"]}]
    (out / "hybrids" / "ablation.json").write_text(json.dumps({"groups": groups, "variants": variants}))
    (out / "task.json").write_text(json.dumps({"task": TASK, "board_id": "board", "base_from": None}))
    rows = [{"name": f"{TASK}__{name}", "native": {"unconnected": count}} for name, count in opens.items()]
    (out / "routes.log").write_text("\n".join(json.dumps(row) for row in rows) + "\n")
    return out


def test_costs_from_means_and_both_sides(tmp_path):
    opens = {"designer": 10, "d-null-1": 12, "designer-r1": 8,
             "ours": 50, "o-null-1": 52, "o-null-2": 48,
             "d-large-U1": 30, "d-large-U1-r1": 34, "o-large-U1": 34,
             "d-large-U2": 10, "o-large-U2": 50}
    sig = {"designer": signals(100, 10), "ours": signals(90, 40),
           "d-large-U1": signals(95, 30), "o-large-U1": signals(95, 20),
           "d-large-U2": signals(99, 10), "o-large-U2": signals(91, 40)}
    result = ablation.analyse(make_run(tmp_path, opens, sig))
    assert result["base"] == {"designer": 10, "ours": 50}
    first = result["groups"][0]
    assert first["name"] == "large-U1"
    assert first["runs_d"] == [30, 34]
    assert first["cost_d"] == 22 and first["cost_o"] == 16 and first["cost"] == 19
    assert result["groups"][1]["cost"] == 0
    # Two sides of three runs each, deviations 0, +2, -2: sd 2.
    assert abs(result["sd"] - 2.0) < 1e-9
    checks = {check["signal"]: check for check in result["checks"]}
    assert checks["ratsnest_crossings"]["whole_right"] is True
    assert checks["wirelength_mm"]["whole_right"] is False
    # U1 costs on both sides and its crossings move with it.
    assert checks["ratsnest_crossings"]["agree"] == checks["ratsnest_crossings"]["moved"] == 2


def test_spearman():
    assert abs(ablation.spearman([1, 2, 3, 4], [10, 20, 30, 40]) - 1) < 1e-9
    assert abs(ablation.spearman([1, 2, 3, 4], [4, 3, 2, 1]) + 1) < 1e-9
    assert ablation.spearman([1, 2], [1, 2]) is None
