#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Summarises a corpus run over the harvested boards, by cause.

    benchmarks/github/summary.py <run-output-or-log> [--markdown]

Reads the runner's per-board JSON lines (its stdout, or results.json) and
sorts the boards: **clean** (all routed, no native copper finding beyond
the source's own), **open** (connections left), **drc** (native copper
findings), **error** (adapter error or timeout), with the designer's via
count for comparison from manifest.json."""

import argparse
import importlib.util
import json
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("corpus_run", HERE.parent / "corpus" / "run.py")
corpus_run = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(corpus_run)


def recount(directory, row):
    """The native findings again from the saved reports, so runs made
    before a counting change read like new ones."""
    work = directory / row["name"]
    baseline = frozenset()
    if (work / "baseline" / "drc.json").exists():
        baseline = frozenset(
            corpus_run.finding_key(v)
            for v in json.loads((work / "baseline" / "drc.json").read_text()).get("violations", []))
    return corpus_run.drc_summary(work / "routed", baseline, row.get("reference_findings"))


def rows_of(path):
    if path.is_dir():
        results = path / "results.json"
        if results.exists():
            return json.loads(results.read_text())
        path = path.with_suffix(".log")
    rows = []
    for line in path.read_text().splitlines():
        if line.startswith("{"):
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--markdown", action="store_true")
    arguments = parser.parse_args()
    manifest = {b["name"]: b for b in json.loads((HERE / "manifest.json").read_text())["boards"]}
    classes = defaultdict(list)
    for row in rows_of(arguments.output):
        name = row["name"]
        route = row.get("route") or {}
        designer = manifest.get(name, {})
        if "error" in route:
            text = route["error"].strip().splitlines()
            reason = text[-1][:100] if text else "?"
            classes["error"].append((name, reason, designer))
            continue
        native = route.get("native") or {}
        if arguments.output.is_dir() and (arguments.output / name / "routed" / "drc.json").exists():
            native = recount(arguments.output, row) or native
        # The runner's copper errors beyond the source's and the designer's
        # own (drc_summary in benchmarks/corpus/run.py).
        findings = native.get("errors") if isinstance(native, dict) else None
        unconnected = native.get("unconnected") if isinstance(native, dict) else None
        open_ = route["connections"] - route["routed"]
        summary = f'{route["routed"]}/{route["connections"]}, {route["vias"]} vias (designer {designer.get("vias")}), {route["routing_seconds"]:.0f} s'
        if open_ > 0:
            classes["open"].append((name, f"{open_} open; {summary}", designer))
        elif findings:
            classes["drc"].append((name, f"{findings}; {summary}", designer))
        elif unconnected:
            classes["mismatch"].append((name, f"{unconnected} unconnected in KiCad; {summary}", designer))
        else:
            classes["clean"].append((name, summary, designer))
    total = sum(len(v) for v in classes.values())
    if arguments.markdown:
        print("| Board | Layers | Footprints | Result | Vias (designer) | Time |")
        print("| --- | --- | --- | --- | --- | --- |")
        for kind in ("clean", "open", "drc", "mismatch", "error"):
            for name, text, designer in sorted(classes.get(kind, [])):
                print(f"| {name} | {designer.get('copper_layers', '?')} | {designer.get('footprints', '?')} | {kind}: {text} | {designer.get('vias', '?')} | |")
        counts = ", ".join(f"{kind} {len(classes.get(kind, []))}" for kind in ("clean", "open", "drc", "mismatch", "error"))
        print(f"\n{total} boards: {counts}")
        return
    for kind in ("clean", "open", "drc", "mismatch", "error"):
        entries = classes.get(kind, [])
        if not entries:
            continue
        print(f"\n## {kind}: {len(entries)} of {total}")
        for name, text, designer in sorted(entries):
            layers = designer.get("copper_layers", "?")
            print(f"- {name} ({layers}L, {designer.get('footprints', '?')} fp): {text}")


if __name__ == "__main__":
    main()
