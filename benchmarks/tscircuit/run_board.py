#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Routes one cold KiCad board with tscircuit's capacity autorouter through
the DSN/SES exchange and verifies the result natively.

  run_board.py <freerouting-run-dir> <board-id> <output-dir> [timeout-seconds]

The freerouting run directory supplies the exported, rule-translated
input.dsn and the cold source; the session is imported with the same helper
the Freerouting comparison uses, so both external routers face identical
exchange semantics and the same final gate."""

import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def main():
    run_dir, board_id, output = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
    timeout = int(sys.argv[4]) if len(sys.argv) > 4 else 1500
    dsn = run_dir / "routing/input.dsn"
    source = run_dir / "routing/source"
    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)
    shutil.copy(dsn, output / "input.dsn")
    ses = output / "input.ses"
    report = {"board_id": board_id, "status": "failed"}
    started = time.monotonic()
    routing = subprocess.run(
        ["node", str(HERE / "route_dsn.mjs"), str(output / "input.dsn"), str(ses), str(output / "router.json"), str(timeout)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=timeout + 120)
    (output / "router.log").write_text(routing.stdout)
    report["router_seconds"] = time.monotonic() - started
    if routing.returncode != 0 or not (output / "router.json").exists():
        report["error"] = routing.stdout[-500:]
        (output / "report.json").write_text(json.dumps(report, indent=2))
        print(json.dumps(report)); return 1
    report.update(json.loads((output / "router.json").read_text()))
    result = output / "result"
    shutil.copytree(source, result, ignore=shutil.ignore_patterns("drc.json", "erc.json", "verification.json", "preview*"))
    helper = ROOT / "experiments/whole-board/compare_native_router.py"
    imported = subprocess.run([sys.executable, str(helper), "--native", "import", str(result / f"{board_id}.kicad_pcb"), str(ses)],
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=600)
    (output / "import.log").write_text(imported.stdout)
    report["imported"] = imported.returncode == 0
    if imported.returncode != 0:
        report["error"] = "import failed: " + imported.stdout[-300:]
        (output / "report.json").write_text(json.dumps(report, indent=2))
        print(json.dumps(report)); return 1
    verify = subprocess.run([str(ROOT / "target/release/pcb-maker"), "verify-kicad-rung", str(result), board_id],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=900)
    (output / "verify.log").write_text(verify.stdout)
    if (result / "verification.json").exists():
        report["native"] = json.loads((result / "verification.json").read_text())
    stats = subprocess.run([str(ROOT / "target/release/pcb-maker"), "inspect-kicad-board", str(result / f"{board_id}.kicad_pcb")],
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=300)
    try:
        report["statistics"] = json.loads(stats.stdout)
    except ValueError:
        pass
    report["status"] = "finished"
    (output / "report.json").write_text(json.dumps(report, indent=2))
    print(json.dumps({k: report.get(k) for k in ("status", "seconds", "vias", "length_mm", "native")}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
