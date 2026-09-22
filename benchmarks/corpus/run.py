#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Runs the board corpus: strip tracks and vias (pours stay), route the designer's placement, and
place plus route automatically. Judges copper only: native unconnected items
and native DRC errors other than silkscreen/library cosmetics, compared with
what the stripped source already had."""

import argparse
import json
import re
import shutil
import subprocess
import sys
import time
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
COSMETIC = re.compile(r"^(silk_|nonmirrored_text|lib_footprint|text_|footprint_type_mismatch|missing_courtyard|isolated_copper)")


def finding_key(violation):
    return (violation["type"], tuple(sorted(i.get("description", "") for i in violation.get("items", []))))


def drc_summary(directory, baseline=frozenset(), reference=None):
    """Copper findings that the stripped source did not already have, beyond
    what the designer's own routed board has of the same type."""
    path = directory / "drc.json"
    if not path.exists():
        return None
    report = json.loads(path.read_text())
    errors = Counter(
        v["type"] for v in report.get("violations", [])
        if not COSMETIC.match(v["type"]) and finding_key(v) not in baseline
    )
    for kind, allowed in (reference or {}).items():
        errors[kind] = max(0, errors.get(kind, 0) - allowed)
    errors = +errors
    cosmetic = sum(1 for v in report.get("violations", []) if COSMETIC.match(v["type"]))
    return {
        "unconnected": len(report.get("unconnected_items", [])),
        "errors": dict(errors),
        "cosmetic": cosmetic,
    }


def run(binary, arguments, log, timeout):
    started = time.monotonic()
    try:
        process = subprocess.run(
            [str(binary), *map(str, arguments)],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=timeout,
        )
        output, code = process.stdout, process.returncode
    except subprocess.TimeoutExpired as error:
        output, code = (error.stdout or b"").decode(errors="replace") if isinstance(error.stdout, bytes) else (error.stdout or ""), "timeout"
    log.write_text("\n".join(l for l in output.splitlines() if "PROPERTY_ENUM" not in l))
    return code, time.monotonic() - started


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--corpus", type=Path, default=Path(__file__).with_name("corpus.json"))
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/pcb-maker")
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--skip-layout", action="store_true")
    parser.add_argument("--timeout", type=int, default=1800)
    arguments = parser.parse_args()
    arguments.output.mkdir(parents=True, exist_ok=True)
    rows = []
    for board in json.loads(arguments.corpus.read_text())["boards"]:
        if arguments.only and board["name"] not in arguments.only:
            continue
        directory = Path(board["directory"])
        if not directory.is_absolute():
            directory = ROOT / directory
        board_id = board["board_id"]
        if not (directory / f"{board_id}.kicad_pcb").exists():
            print(f"skipping {board['name']}: {directory} not found", file=sys.stderr)
            continue
        work = arguments.output / board["name"]
        if work.exists():
            shutil.rmtree(work)
        source = work / "source"
        shutil.copytree(directory, source, ignore=shutil.ignore_patterns(
            ".history", "*-backups", "Gerbers", "packages3D", "*.pdf", "*.zip", "fp-info-cache"))
        row = {"name": board["name"]}
        # The cold strip only provides the reference statistics; the board
        # that is routed keeps its copper pours.
        code, _ = run(arguments.binary, ["strip-kicad-copper", directory / f"{board_id}.kicad_pcb",
                                         work / "cold.kicad_pcb", work / "strip.json"],
                      work / "strip.log", 300)
        if code == 0:
            code, _ = run(arguments.binary, ["strip-kicad-tracks", directory / f"{board_id}.kicad_pcb",
                                             source / f"{board_id}.kicad_pcb"],
                          work / "strip-tracks.log", 300)
        if code != 0:
            row["error"] = "strip failed: " + (work / "strip.log").read_text()[-300:]
            rows.append(row)
            continue
        strip = json.loads((work / "strip.json").read_text())
        row["reference"] = {"segments": strip.get("removed_segments"), "vias": strip.get("removed_vias"),
                            "zones": strip.get("removed_copper_zones")}

        # Findings the designer's own placement already has are not ours.
        baseline_directory = work / "baseline"
        shutil.copytree(source, baseline_directory)
        run(arguments.binary, ["verify-kicad-rung", baseline_directory, board_id], work / "baseline.log", 600)
        baseline = frozenset()
        if (baseline_directory / "drc.json").exists():
            baseline = frozenset(
                finding_key(v)
                for v in json.loads((baseline_directory / "drc.json").read_text()).get("violations", []))
        row["baseline_findings"] = len(baseline)
        # The designer's routed board sets the bar for each kind of finding.
        reference_directory = work / "reference"
        shutil.copytree(directory, reference_directory, ignore=shutil.ignore_patterns(
            ".history", "*-backups", "Gerbers", "packages3D", "*.pdf", "*.zip", "fp-info-cache"))
        try:
            subprocess.run(["kicad-cli", "pcb", "drc", "--refill-zones", "--severity-error", "--format", "json",
                            "-o", "drc.json", f"{board_id}.kicad_pcb"], cwd=reference_directory,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=600)
        except (OSError, subprocess.TimeoutExpired):
            pass
        reference = {}
        if (reference_directory / "drc.json").exists():
            reference = dict(Counter(
                v["type"] for v in json.loads((reference_directory / "drc.json").read_text()).get("violations", [])
                if not COSMETIC.match(v["type"])))
        row["reference_findings"] = reference

        code, seconds = run(arguments.binary, ["route-kicad-board", source, board_id, work / "routed", "auto"],
                            work / "route.log", arguments.timeout)
        report = work / "routed/board-router.json"
        if report.exists():
            result = json.loads(report.read_text())
            row["route"] = {
                "routed": result["routed_connections"], "connections": result["routable_connections"],
                "unconnected_terminals": result["unconnected_terminals"],
                "vias": result["vias"], "length_mm": round(result["length_mm"], 1),
                "pours": result.get("pours", "none"),
                "routing_seconds": round(result["routing_seconds"], 2),
                "internal_violations": len(result["internal_violations"]),
                "native": drc_summary(work / "routed", baseline, reference),
            }
        else:
            row["route"] = {"error": (work / "route.log").read_text()[-400:], "exit": code}

        if not arguments.skip_layout:
            code, seconds = run(arguments.binary, ["layout-kicad-board", source, board_id, work / "layout", "auto"],
                                work / "layout.log", arguments.timeout)
            report = work / "layout/board-layout.json"
            if report.exists():
                result = json.loads(report.read_text())
                routed = result["routed"]
                row["layout"] = {
                    "moves": len(result.get("moves", [])),
                    "kept_moves": sum(1 for m in result.get("moves", []) if m.get("kept")),
                    "first_open": result.get("first_unconnected_terminals"),
                    "routed": routed["routed_connections"], "connections": routed["routable_connections"],
                    "unconnected_terminals": routed["unconnected_terminals"],
                    "vias": routed["vias"], "length_mm": round(routed["length_mm"], 1),
                    "pours": routed.get("pours", "none"),
                    "seconds": round(seconds, 1),
                    "internal_violations": len(routed["internal_violations"]),
                    "native": drc_summary(work / "layout/result", baseline, reference),
                }
            else:
                row["layout"] = {"error": (work / "layout.log").read_text()[-400:], "exit": code}
        rows.append(row)
        print(json.dumps(row), flush=True)

    (arguments.output / "results.json").write_text(json.dumps(rows, indent=1))
    lines = ["| Board | Reference copper | Route (designer placement) | Place + route (automatic) |",
             "| --- | --- | --- | --- |"]

    def cell(entry):
        if entry is None:
            return "—"
        if "error" in entry:
            return "**failed**: " + entry["error"].strip().splitlines()[-1][:80]
        native = entry.get("native") or {}
        problems = []
        if native.get("unconnected"):
            problems.append(f"{native['unconnected']} unconnected")
        if native.get("errors"):
            problems.append(", ".join(f"{k}×{v}" for k, v in native["errors"].items()))
        if entry.get("internal_violations"):
            problems.append(f"{entry['internal_violations']} internal")
        verdict = "clean" if not problems else "; ".join(problems)
        seconds = entry.get("routing_seconds", entry.get("seconds"))
        return (f"{entry['routed']}/{entry['connections']}, {entry['vias']} vias, "
                f"{entry['length_mm']:.0f} mm, {seconds} s, pours {entry.get('pours', 'none')} — {verdict}")

    for row in rows:
        reference = row.get("reference", {})
        lines.append(f"| {row['name']} | {reference.get('segments')} tracks, {reference.get('vias')} vias, "
                     f"{reference.get('zones')} zones | {cell(row.get('route')) if 'error' not in row else row['error'][:80]} | {cell(row.get('layout'))} |")
    (arguments.output / "results.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
