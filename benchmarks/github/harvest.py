#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Harvests open-source KiCad boards from GitHub as benchmarks.

    benchmarks/github/harvest.py search [--pages N] [--out candidates.json]
    benchmarks/github/harvest.py fetch [--candidates candidates.json] [--limit N]
    benchmarks/github/harvest.py boards

`search` runs GitHub code searches for KiCad 7+ boards (needs `gh auth`)
and writes the candidates. `fetch` downloads each candidate's board and
project file at the repository's current head into
benchmarks/real/external/github/<owner>__<repo>__<stem>/ (gitignored),
keeps the ones that are finished designs of some size (see `judge`), and
records them in manifest.json (committed: origin, commit, licence and the
designer's numbers). With `fetch --training`, boards go to benchmarks/real/external/training/
instead, judged by relaxed criteria (`judge_training`: any finished board
of some size), recorded in training-manifest.json: training data for the
congestion model (docs/congestion-model.md), not benchmarks. `boards` writes boards.json in the corpus runner's
format from the manifest; run it with
`benchmarks/corpus/run.py <out> --corpus benchmarks/github/boards.json`,
and make layout tasks with
`benchmarks/agent-tasks/from_pcbench.py <out> --corpus benchmarks/github/boards.json`.

Files are downloaded for benchmarking only and are not redistributed; the
manifest records each repository's licence."""

import argparse
import base64
import hashlib
import json
import re
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
STORE = ROOT / "benchmarks/real/external/github"
MANIFEST = HERE / "manifest.json"
TRAINING_STORE = ROOT / "benchmarks/real/external/training"
TRAINING_MANIFEST = HERE / "training-manifest.json"

# Every query is restricted to KiCad 7+ board files by their generator
# version line; the second term picks a kind of board. Code search indexes
# files up to 384 KB only, so large boards come through the repository
# topic search below.
QUERIES = [
    ('"generator_version \\"9.0\\""', '"In2.Cu"'),
    ('"generator_version \\"8.0\\""', '"In2.Cu"'),
    ('"generator_version \\"9.0\\""', '"Package_BGA"'),
    ('"generator_version \\"8.0\\""', '"Package_BGA"'),
    ('"generator_version \\"9.0\\""', '"Package_DFN_QFN"'),
    ('"generator_version \\"8.0\\""', '"Package_DFN_QFN"'),
    ('"generator_version \\"9.0\\""', '"Package_QFP"'),
    ('"generator_version \\"8.0\\""', '"Package_QFP"'),
    ('"generator_version \\"9.0\\""', '"Package_LGA"'),
    ('"generator_version \\"9.0\\""', '"Connector_USB"'),
    ('"(version 20231120)"', '"In2.Cu"'),
]
TOPICS = ["kicad", "kicad-project", "pcb-design", "open-hardware", "hardware"]


def gh(*arguments, retries=6):
    """A `gh api` call as JSON, waiting out rate limits."""
    for attempt in range(retries):
        process = subprocess.run(["gh", "api"] + list(arguments), capture_output=True, text=True)
        if process.returncode == 0:
            return json.loads(process.stdout) if process.stdout.strip() else None
        message = process.stderr.strip()
        if "rate limit" in message.lower() or "abuse" in message.lower() or "secondary" in message.lower() or "403" in message:
            wait = 65 if attempt < 2 else 120
            print(f"  rate limited, waiting {wait} s", file=sys.stderr)
            time.sleep(wait)
            continue
        if "404" in message or "422" in message:
            return None
        time.sleep(5)
    return None


def search(arguments):
    candidates = {}
    if arguments.out.exists():
        candidates = {c["key"]: c for c in json.loads(arguments.out.read_text())}
    for first, second in QUERIES:
        query = f"extension:kicad_pcb {first} {second}"
        for page in range(1, arguments.pages + 1):
            result = gh("-X", "GET", "search/code", "-f", f"q={query}", "-f", "per_page=100", "-f", f"page={page}")
            if not result or not result.get("items"):
                break
            new = 0
            for item in result["items"]:
                repository = item["repository"]
                if repository.get("fork"):
                    continue
                key = f'{repository["full_name"]}:{item["path"]}'
                if key not in candidates:
                    candidates[key] = {"key": key, "repository": repository["full_name"], "path": item["path"],
                                       "found_by": second}
                    new += 1
            print(f"{query} page {page}: {len(result['items'])} items, {new} new "
                  f"(total {result.get('total_count')})", file=sys.stderr)
            arguments.out.write_text(json.dumps(list(candidates.values()), indent=1))
            time.sleep(6.5)  # code search: 10 requests a minute
            if len(result["items"]) < 100:
                break
    # Large boards: repositories by topic, their trees searched for boards
    # over 300 KB.
    for topic in TOPICS:
        for page in range(1, min(arguments.pages, 3) + 1):
            result = gh("-X", "GET", "search/repositories", "-f", f"q=topic:{topic} fork:false", "-f", "sort=stars",
                        "-f", "per_page=100", "-f", f"page={page}")
            if not result or not result.get("items"):
                break
            new = 0
            for repository in result["items"]:
                name = repository["full_name"]
                if any(c["repository"] == name for c in candidates.values()):
                    continue
                branch = repository.get("default_branch") or "main"
                tree = gh(f"repos/{name}/git/trees/{branch}?recursive=1")
                if not tree:
                    continue
                for entry in tree.get("tree", []):
                    if entry["path"].endswith(".kicad_pcb") and entry.get("size", 0) >= 300_000:
                        key = f'{name}:{entry["path"]}'
                        if key not in candidates:
                            candidates[key] = {"key": key, "repository": name, "path": entry["path"],
                                               "found_by": f"topic:{topic}", "size": entry["size"]}
                            new += 1
                time.sleep(1)
            print(f"topic {topic} page {page}: {new} new boards", file=sys.stderr)
            arguments.out.write_text(json.dumps(list(candidates.values()), indent=1))
            time.sleep(3)
    print(f"{len(candidates)} candidates", file=sys.stderr)


def judge(text):
    """The designer's numbers, and why the board is no benchmark (or None)."""
    version = re.search(r'\(kicad_pcb\s*\(version (\d+)\)', text)
    if not version or int(version.group(1)) < 20211014:
        return None, "older than KiCad 6"
    layers = re.search(r'\n\s*\(layers\s*\n((?:\s*\(\d+ "[^"]+" \w+[^)]*\)\s*\n)+)', text)
    copper = len([line for line in (layers.group(1).splitlines() if layers else []) if '.Cu"' in line])
    footprints = len(re.findall(r'\n\s*\(footprint ', text))
    segments = len(re.findall(r'\n\s*\(segment', text)) + len(re.findall(r'\n\s*\(arc\s*\n', text))
    vias = len(re.findall(r'\n\s*\(via\s*\n', text))
    zones = len(re.findall(r'\n\s*\(zone\s*\n', text))
    pads = re.findall(r'\(pad "[^"]*" (?:smd|thru_hole)[^\n]*\n\s*\(at [^)]*\)\s*\(size ([\d.]+) ([\d.]+)\)', text)
    fine = sum(1 for w, h in pads if min(float(w), float(h)) <= 0.3)
    edge = '"Edge.Cuts"' in text
    numbers = {"copper_layers": copper, "footprints": footprints, "pads": len(pads), "fine_pads": fine,
               "segments": segments, "vias": vias, "zones": zones}
    if not edge:
        return numbers, "no outline"
    if copper < 2 or copper > 8:
        return numbers, f"{copper} copper layers"
    if footprints < 25:
        return numbers, f"only {footprints} footprints"
    if footprints > 900:
        return numbers, f"{footprints} footprints"
    if segments < 40:
        return numbers, "not routed"
    return numbers, None


def judge_training(text):
    """`judge` relaxed for training data: any finished board (an outline,
    tracks) with a few parts on 2 to 8 copper layers."""
    numbers, reason = judge(text)
    if numbers is None:
        return numbers, reason
    if not '"Edge.Cuts"' in text:
        return numbers, "no outline"
    if numbers["copper_layers"] < 2 or numbers["copper_layers"] > 8:
        return numbers, f'{numbers["copper_layers"]} copper layers'
    if numbers["footprints"] < 8:
        return numbers, f'only {numbers["footprints"]} footprints'
    if numbers["footprints"] > 900:
        return numbers, f'{numbers["footprints"]} footprints'
    if numbers["segments"] < 15:
        return numbers, "not routed"
    return numbers, None


def fetch(arguments):
    candidates = json.loads(arguments.candidates.read_text())
    store = TRAINING_STORE if arguments.training else STORE
    manifest_path = TRAINING_MANIFEST if arguments.training else MANIFEST
    judged = judge_training if arguments.training else judge
    # Hard boards first: large files, BGAs, inner layers, fine pitch.
    priority = ["topic:", '"Package_BGA"', '"In2.Cu"', '"Package_LGA"', '"Package_DFN_QFN"', '"Package_QFP"']
    candidates.sort(key=lambda c: (next((i for i, p in enumerate(priority) if c["found_by"].startswith(p)), 99),
                                   -c.get("size", 0)))
    manifest = json.loads(manifest_path.read_text()) if manifest_path.exists() else {"boards": [], "rejected": {}}
    known = {b["key"] for b in manifest["boards"]} | set(manifest["rejected"])
    hashes = {b["sha256"] for b in manifest["boards"]}
    if arguments.training and MANIFEST.exists():
        # The benchmark boards are never training data.
        benchmark = json.loads(MANIFEST.read_text())
        known |= {b["key"] for b in benchmark["boards"]}
        hashes |= {b["sha256"] for b in benchmark["boards"]}
    store.mkdir(parents=True, exist_ok=True)
    taken = 0
    for candidate in candidates:
        if arguments.limit and taken >= arguments.limit:
            break
        key = candidate["key"]
        if key in known:
            continue
        repository, path = candidate["repository"], candidate["path"]
        stem = Path(path).stem
        directory = str(Path(path).parent)
        info = gh(f"repos/{repository}")
        if not info:
            manifest["rejected"][key] = "repository gone"
            continue
        branch = info.get("default_branch") or "main"
        head = gh(f"repos/{repository}/commits/{branch}")
        commit = head["sha"] if head else None
        if not commit:
            manifest["rejected"][key] = "no head commit"
            continue
        listing = gh(f"repos/{repository}/contents/{directory}?ref={commit}") if directory != "." \
            else gh(f"repos/{repository}/contents?ref={commit}")
        if not isinstance(listing, list):
            manifest["rejected"][key] = "directory unreadable"
            continue
        names = {entry["name"]: entry for entry in listing}
        board_entry = names.get(f"{stem}.kicad_pcb")
        project_entry = names.get(f"{stem}.kicad_pro")
        if not board_entry:
            manifest["rejected"][key] = "board not at head"
            continue
        if not project_entry:
            manifest["rejected"][key] = "no project file"
            continue
        if board_entry.get("size", 0) < 60_000:
            manifest["rejected"][key] = f"board file {board_entry.get('size')} bytes"
            continue
        if board_entry.get("size", 0) > 40_000_000:
            manifest["rejected"][key] = f"board file {board_entry.get('size')} bytes"
            continue
        try:
            with urllib.request.urlopen(board_entry["download_url"], timeout=120) as response:
                board_bytes = response.read()
            with urllib.request.urlopen(project_entry["download_url"], timeout=60) as response:
                project_bytes = response.read()
        except Exception as error:  # noqa: BLE001
            manifest["rejected"][key] = f"download failed: {error}"
            continue
        text = board_bytes.decode("utf-8", errors="replace")
        numbers, reason = judged(text)
        if reason:
            manifest["rejected"][key] = reason
            print(f"  {key}: {reason}", file=sys.stderr)
            manifest_path.write_text(json.dumps(manifest, indent=1))
            continue
        digest = hashlib.sha256(board_bytes).hexdigest()
        if digest in hashes:
            manifest["rejected"][key] = "duplicate board"
            continue
        hashes.add(digest)
        name = f'{repository.replace("/", "__")}__{stem}'
        name = re.sub(r"[^A-Za-z0-9_.-]", "_", name)
        if any(board["name"] == name for board in manifest["boards"]):
            # The same stem twice in one repository (revisions in
            # directories): the directory tells them apart.
            name += "__" + re.sub(r"[^A-Za-z0-9_.-]", "_", Path(path).parent.name or "root")
            if any(board["name"] == name for board in manifest["boards"]):
                name += "__" + hashlib.sha256(path.encode()).hexdigest()[:6]
        target = store / name
        target.mkdir(parents=True, exist_ok=True)
        (target / f"{stem}.kicad_pcb").write_bytes(board_bytes)
        (target / f"{stem}.kicad_pro").write_bytes(project_bytes)
        origin = {"repository": repository, "path": path, "commit": commit,
                  "url": f"https://github.com/{repository}/blob/{commit}/{path}"}
        (target / "origin.json").write_text(json.dumps(origin, indent=1))
        license_ = (info.get("license") or {}).get("spdx_id")
        manifest["boards"].append({
            "key": key, "name": name, "board_id": stem, "repository": repository, "path": path,
            "commit": commit, "license": license_, "stars": info.get("stargazers_count"),
            "sha256": digest, "size": len(board_bytes), **numbers})
        taken += 1
        print(f"{name}: {numbers['copper_layers']} layers, {numbers['footprints']} footprints, "
              f"{numbers['fine_pads']} fine pads, {numbers['segments']} segments, {license_}", file=sys.stderr)
        manifest_path.write_text(json.dumps(manifest, indent=1))
    manifest_path.write_text(json.dumps(manifest, indent=1))
    print(f"{len(manifest['boards'])} boards in the manifest, {len(manifest['rejected'])} rejected", file=sys.stderr)


def boards(arguments):
    manifest = json.loads(MANIFEST.read_text())
    rows = []
    for board in sorted(manifest["boards"], key=lambda b: (-b["copper_layers"], -b["fine_pads"], -b["footprints"])):
        rows.append({"name": board["name"], "directory": f"benchmarks/real/external/github/{board['name']}",
                     "board_id": board["board_id"],
                     "note": f'{board["repository"]} @ {board["commit"][:8]}, {board["license"]}: '
                             f'{board["copper_layers"]} layers, {board["footprints"]} footprints, '
                             f'{board["fine_pads"]} fine pads, designer {board["segments"]} segments / {board["vias"]} vias'})
    (HERE / "boards.json").write_text(json.dumps({
        "comment": "Open-source boards harvested from GitHub by harvest.py; sources under "
                   "benchmarks/real/external/github (gitignored), origins in manifest.json.",
        "boards": rows}, indent=1))
    print(f"{len(rows)} boards", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    s = commands.add_parser("search")
    s.add_argument("--pages", type=int, default=3)
    s.add_argument("--out", type=Path, default=HERE / "candidates.json")
    f = commands.add_parser("fetch")
    f.add_argument("--candidates", type=Path, default=HERE / "candidates.json")
    f.add_argument("--limit", type=int, default=0)
    f.add_argument("--training", action="store_true",
                   help="relaxed criteria, into benchmarks/real/external/training (congestion model training data)")
    commands.add_parser("boards")
    arguments = parser.parse_args()
    {"search": search, "fetch": fetch, "boards": boards}[arguments.command](arguments)


if __name__ == "__main__":
    main()
