#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Bounded, non-destructive non-KiCad import and baseline validation.

Only uses the installed KiCad CLI. Converted layouts remain unvalidated
training labels: a successful import/DRC is not proof of semantic fidelity.
Caller must keep source licence, repository, commit and original file path.
"""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import time
import xml.etree.ElementTree as ET

FORMATS = ("auto", "eagle", "altium", "pads", "cadstar", "fabmaster", "pcad", "solidworks")


def command(arguments, timeout):
    """Limit time, process tree and memory; keep logs separate from JSON."""
    started = time.monotonic()
    environment = dict(os.environ, OMP_NUM_THREADS="1", OPENBLAS_NUM_THREADS="1")
    limiter = shutil.which("prlimit")
    bounded = ([limiter, "--as=8589934592", "--core=0", "--"] if limiter else []) + list(map(str, arguments))
    with subprocess.Popen(bounded, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          text=True, errors="replace", env=environment,
                          start_new_session=True) as process:
        timed_out = False
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate()
    return {"command": list(map(str, arguments)), "exit_code": process.returncode,
            "timeout": timed_out, "seconds": round(time.monotonic() - started, 3),
            "stdout": stdout[-16000:], "stderr": stderr[-16000:]}


def eagle_inventory(source):
    """Record independently countable source semantics; do not change source."""
    raw = source.read_bytes()
    if b"<!ENTITY" in raw:
        raise ValueError("XML entity declarations are unsupported")
    root = ET.fromstring(raw)
    board = root.find("./drawing/board")
    if board is None:
        raise ValueError("not an Eagle XML PCB (binary Eagle is unsupported)")
    signals = board.findall("./signals/signal")
    contactrefs = {(s.get("name"), p.get("element"), p.get("pad"))
                   for s in signals for p in s.findall("contactref")}
    return {"elements": len(board.findall("./elements/element")),
            "signals": len(signals), "contactrefs": len(contactrefs),
            "wires": sum(len(s.findall("wire")) for s in signals),
            "vias": sum(len(s.findall("via")) for s in signals),
            "polygons": sum(len(s.findall("polygon")) for s in signals),
            "design_rules": {p.get("name"): p.get("value")
                             for p in board.findall("./designrules/param")},
            "net_classes": [dict(c.attrib) for c in board.findall("./classes/class")]}


def convert_board(source, output, source_format="auto", timeout=60, metadata=None,
                  kicad_cli="kicad-cli"):
    """Import one file into its own directory and return its persisted record."""
    source, output = Path(source).resolve(), Path(output).resolve()
    if source_format not in FORMATS:
        raise ValueError(f"unsupported format hint: {source_format}")
    if source.stat().st_size > 100 * 1024**2:
        raise ValueError("source larger than 100 MiB")
    output.mkdir(parents=True, exist_ok=True)
    destination = output / "board.kicad_pcb"
    if destination == source:
        raise ValueError("source and output must differ")
    # Never reuse stale converted output or stale validation reports.
    for name in ("board.kicad_pcb", "import.json", "stats.json", "drc.json"):
        (output / name).unlink(missing_ok=True)
    record = {"schema_version": 1, "source": str(source), "format": source_format,
              "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
              "metadata": metadata or {}, "converted": str(destination),
              "semantic_validation": "unvalidated", "eligible_as_routing_label": False,
              "project_rules": "not verified against source; no original KiCad project",
              "steps": {}}
    record["kicad_version"] = command([kicad_cli, "version"], 5)["stdout"].strip()
    if source_format == "eagle":
        try:
            record["source_inventory"] = eagle_inventory(source)
        except (ET.ParseError, ValueError) as error:
            record.update(status="unsupported_source", reason=str(error))
            (output / "conversion.json").write_text(json.dumps(record, indent=2) + "\n")
            return record
    arguments = [kicad_cli, "pcb", "import", "--format", source_format,
                 "--output", str(destination), "--report-format", "json",
                 "--report-file", str(output / "import.json"), str(source)]
    imported = record["steps"]["import"] = command(arguments, timeout)
    if imported["exit_code"] or not destination.exists():
        record.update(status="import_failed", reason="import timed out" if imported["timeout"] else "KiCad import failed")
    else:
        report = json.loads((output / "import.json").read_text()) if (output / "import.json").exists() else {}
        record["import_report"] = report
        if report.get("errors"):
            record.update(status="invalid_conversion", reason="KiCad import reported errors")
        elif re.search(r'\(layer\s+"UNDEFINED"\)', destination.read_text()):
            record.update(status="invalid_conversion", reason="KiCad importer emitted undefined layers; retain original for importer repair")
        else:
            stats = record["steps"]["readback"] = command(
                [kicad_cli, "pcb", "export", "stats", "--format", "json", "--output",
                 str(output / "stats.json"), str(destination)], timeout)
            if stats["exit_code"] or not (output / "stats.json").exists():
                record.update(status="invalid_conversion", reason="native read-back failed")
            else:
                record["statistics"] = json.loads((output / "stats.json").read_text())
                baseline = record["steps"]["drc"] = command(
                    [kicad_cli, "pcb", "drc", "--format", "json", "--severity-all",
                     "--refill-zones", "--output", str(output / "drc.json"), str(destination)], timeout)
                if baseline["exit_code"] or not (output / "drc.json").exists():
                    record.update(status="baseline_failed", reason="native DRC baseline failed or timed out")
                else:
                    drc = json.loads((output / "drc.json").read_text())
                    violations = drc.get("violations", [])
                    record["baseline"] = {"violations": len(violations),
                        "by_type": dict(Counter(v.get("type", "unknown") for v in violations)),
                        "unconnected_items": len(drc.get("unconnected_items", []))}
                    record.update(status="imported_unvalidated", reason="native read-back and baseline recorded; source semantic checks still required")
    (output / "conversion.json").write_text(json.dumps(record, indent=2) + "\n")
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--format", choices=FORMATS, default="auto")
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--metadata", type=Path, help="JSON provenance, copied into conversion record")
    parser.add_argument("--kicad-cli", default="kicad-cli")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    result = convert_board(args.source, args.output, args.format, args.timeout,
                           json.loads(args.metadata.read_text()) if args.metadata else None, args.kicad_cli)
    print(json.dumps({k: result.get(k) for k in ("status", "reason", "converted", "semantic_validation")}))
    return 0 if result["status"] == "imported_unvalidated" else 1


if __name__ == "__main__":
    raise SystemExit(main())
