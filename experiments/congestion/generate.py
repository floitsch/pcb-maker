#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Runs the sample jobs prepare.py wrote, many boards side by side.

    experiments/congestion/generate.py build/congestion --binary build/bin/pcb-maker-x400
        [--jobs 16] [--cores 16-31] [--min-free-gb 20] [--only NAME...] [--kinds github pcbench]
        [--samples DIR] [--probe-seconds S]

Every exporter runs pinned to `--cores` at the lowest priority (nice 19)
with one search thread, and starts only when the memory available, less
what the running exporters are still expected to grow by, leaves
`--min-free-gb` after its own expected peak (benchmarks/agent-tasks/run.py's
MemoryGate). A board's variants run as one exporter per source (designer,
free, task). Finished samples are skipped, so a run resumes."""

import argparse
import json
import os
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

import importlib.util

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("agent_run", ROOT / "benchmarks/agent-tasks/run.py")
agent_run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agent_run)


def groups(job):
    """(source, variants) per source directory, in job order."""
    out = {}
    for variant in job["variants"]:
        source = variant["source"]
        out.setdefault(source, []).append({key: value for key, value in variant.items() if key != "source"})
    return out


def expected_mb(job, peaks):
    if job["name"] in peaks:
        return peaks[job["name"]] * 1.2
    return 1200 if job["kind"] == "github" else 500


def run_group(job, source, variants, arguments, peaks, lock):
    name = job["name"]
    out = arguments.samples / name
    todo = [variant for variant in variants if not (out / f"{variant['name']}.json").exists()]
    # A variant whose base is done already still needs the base's placement.
    names = {variant["name"] for variant in todo}
    needed = set()
    for variant in todo:
        base = variant.get("base")
        while base and base not in names and base not in needed:
            needed.add(base)
            base = next((v.get("base") for v in variants if v["name"] == base), None)
    if not todo:
        return {"name": name, "source": source, "skipped": True}
    run = [dict(variant, skip_probe=variant["name"] in needed and variant["name"] not in names)
           for variant in variants if variant["name"] in names or variant["name"] in needed]
    for variant in run:
        if variant["skip_probe"]:
            # Rewritten as features only; keep the finished sample.
            variant["name_out"] = True
    out.mkdir(parents=True, exist_ok=True)
    directory = arguments.root / "sources" / name / source
    rust_job = {"probe_seconds": arguments.probe_seconds or job["probe_seconds"],
                "placement_seconds": job["placement_seconds"],
                "variants": [{k: v for k, v in variant.items() if k != "name_out"} for variant in run]}
    layout = directory / "layout.json"
    if source == "free":
        rust_job["layout"] = {"placer": {"constraints": "constraints.json"}}
    elif layout.exists():
        rust_job["layout"] = json.loads(layout.read_text())
    work = arguments.root / "work" / name
    work.mkdir(parents=True, exist_ok=True)
    job_path = work / f"{source}.json"
    job_path.write_text(json.dumps(rust_job))
    # Features-only reruns of a base go to a scratch directory, so they do
    # not overwrite the probed sample.
    scratch = None
    if any(variant["skip_probe"] for variant in run):
        scratch = work / f"{source}-out"
        target = scratch
    else:
        target = out
    command = ["taskset", "-c", arguments.cores, "nice", "-n", "19", str(arguments.binary),
               "export-congestion-sample", str(directory), job["board_id"], str(target), str(job_path)]
    env = dict(os.environ, RAYON_NUM_THREADS="1")
    token = arguments.gate.admit(f"{name}/{source}", expected_mb(job, peaks))
    try:
        started = time.monotonic()
        code, output, _, own_mb = agent_run.run_measured(command, arguments.timeout, env,
                                                         lambda pid: arguments.gate.started(token, pid))
    finally:
        arguments.gate.done(token)
    (work / f"{source}.log").write_text(output)
    if scratch is not None:
        for variant in run:
            if not variant["skip_probe"]:
                for suffix in (".json", ".bin"):
                    produced = scratch / f"{variant['name']}{suffix}"
                    if produced.exists():
                        produced.replace(out / produced.name)
    with lock:
        peaks[name] = max(peaks.get(name, 0), own_mb)
    return {"name": name, "source": source, "binary": arguments.binary.name, "code": code, "seconds": round(time.monotonic() - started, 1),
            "own_mb": own_mb, "variants": len(run)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--samples", type=Path, default=None)
    parser.add_argument("--jobs", type=int, default=16)
    parser.add_argument("--cores", default="16-31")
    parser.add_argument("--min-free-gb", type=float, default=20.0)
    parser.add_argument("--timeout", type=int, default=7200)
    parser.add_argument("--probe-seconds", type=float, default=None)
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--kinds", nargs="*")
    parser.add_argument("--sources", nargs="*", help="only these sources (designer, free, task)")
    arguments = parser.parse_args()
    arguments.binary = arguments.binary.resolve()
    arguments.samples = arguments.samples or arguments.root / "samples"
    arguments.gate = agent_run.MemoryGate(arguments.min_free_gb)
    peaks_path = arguments.root / "peaks.json"
    peaks = json.loads(peaks_path.read_text()) if peaks_path.exists() else {}
    lock = threading.Lock()
    work = []
    for path in sorted((arguments.root / "jobs").glob("*.json")):
        job = json.loads(path.read_text())
        if arguments.only and job["name"] not in arguments.only:
            continue
        if arguments.kinds and job["kind"] not in arguments.kinds:
            continue
        for source, variants in groups(job).items():
            if arguments.sources and source not in arguments.sources:
                continue
            work.append((job, source, variants))
    # Big boards first, so that the last exporters are not the slow ones.
    # The race boards' task placements first (the evaluation needs them).
    split_path = arguments.root / "split.json"
    split = json.loads(split_path.read_text()) if split_path.exists() else {}
    work.sort(key=lambda item: (item[1] != "task", split.get(item[0]["name"]) != "test", -expected_mb(item[0], peaks)))
    print(f"{len(work)} exporter runs", file=sys.stderr, flush=True)
    log = (arguments.root / "generate.log").open("a")
    with ThreadPoolExecutor(arguments.jobs) as pool:
        futures = [pool.submit(run_group, job, source, variants, arguments, peaks, lock) for job, source, variants in work]
        for future in as_completed(futures):
            try:
                row = future.result()
            except Exception as error:
                row = {"error": f"{type(error).__name__}: {error}"}
            log.write(json.dumps(row) + "\n")
            log.flush()
            with lock:
                peaks_path.write_text(json.dumps(peaks, indent=1, sort_keys=True))


if __name__ == "__main__":
    main()
