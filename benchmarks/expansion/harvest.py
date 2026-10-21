#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Resumable, bounded GitHub PCB harvest; source files remain local.

run: fetch unprocessed seed candidates, scan complete repository trees in batches,
then fetch newly found boards. --watch repeats discovery and retries transient
failures. All outcomes are journaled; manifest.json atomically publishes boards
ready for triage. No source repository scripts or build systems are executed.
"""

import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
OLD = ROOT / "benchmarks/github"
FORMATS = {".kicad_pcb": "kicad", ".brd": "eagle-or-legacy", ".pcbdoc": "altium"}
# The repository licences of boards usable as benchmarks (the expansion
# tier's selection). Eligibility is conservative, not a conclusion about
# every repository file.
USABLE_LICENSES = {"MIT", "BSD-2-Clause", "BSD-3-Clause", "Apache-2.0", "ISC",
                   "CC0-1.0", "Unlicense", "CERN-OHL-P-2.0", "CERN-OHL-W-2.0",
                   "CERN-OHL-S-2.0", "GPL-2.0", "GPL-3.0", "GPL-2.0-only",
                   "GPL-3.0-only", "GPL-2.0-or-later", "GPL-3.0-or-later",
                   "CC-BY-4.0", "CC-BY-SA-4.0"}
# Training data (nn_license_eligible) is permissive only
# (benchmarks/github/licences.py; benchmarks/github/README.md, "Licences"):
# the copyleft and weakly reciprocal licences of USABLE_LICENSES
# (CERN-OHL-S and -W, GPL, CC-BY-SA) are benchmarks, never training data.
sys.path.append(str(Path(__file__).resolve().parents[1] / "github"))
from licences import spdx_training_eligible  # noqa: E402
sys.path.append(str(Path(__file__).resolve().parents[1]))
import harness_common  # noqa: E402

TOPICS = ["kicad", "kicad-project", "pcb-design", "open-hardware", "oshw",
          "fpga-board", "hardware", "eagle-pcb", "altium-designer"]
CURATED = ["antmicro/jetson-nano-baseboard", "antmicro/alkali-csd", "OLIMEX/ESP32-EVB",
           "OLIMEX/ESP32-POE", "raspberrypi/pico", "greatscottgadgets/hackrf",
           "greatscottgadgets/luna", "m-labs/sinara", "myriadrf/LimeSDR-USB",
           "Nuand/bladeRF", "wuxx/nanoDAP", "espressif/esp-dev-kits"]


def nn_license_eligible(spdx):
    """Whether a board under this repository licence may be training data:
    a permissive licence only (MIT, BSD, Apache-2.0, ISC, CC0, Unlicense,
    CERN-OHL-P, CC-BY, ...); copyleft and weakly reciprocal licences and
    NOASSERTION are not."""
    return spdx_training_eligible(spdx)


def atomic_json(path, value):
    harness_common.atomic_json(path, value, indent=1, make_parents=True)


def identity(repository, path):
    return f"{repository}:{path}"


def safe_name(source_id):
    return harness_common.safe_name(source_id, limit=100)


def features(text):
    """Cheap inventory only; the native importer is the authoritative check."""
    version = re.search(r"\(kicad_pcb\s*\(version\s+(\d+)\)", text)
    layer_block = re.search(r"\(layers\s*\n(.*?)\n\s*\)", text, re.S)
    copper = len(re.findall(r'\(\d+\s+"?[^\s\"]*\.Cu"?\s+', layer_block.group(1))) if layer_block else 0
    result = {
        "version": int(version.group(1)) if version else None,
        "copper_layers": copper,
        "footprints": len(re.findall(r"\(footprint\s", text)),
        "pads": len(re.findall(r"\(pad\s", text)),
        "segments": len(re.findall(r"\(segment\s", text)),
        "vias": len(re.findall(r"\(via\s", text)),
        "zones": len(re.findall(r"\(zone\s", text)),
        "fine_pitch_packages": len(re.findall(r"(?:Package_BGA|Package_DFN_QFN|Package_LGA)", text)),
        # An Edge.Cuts entry in the layer table alone is not an outline.
        "has_outline": bool(re.search(r'\(gr_(?:line|arc|rect|circle|poly|curve)\b(?:(?!\(gr_).)*?\(layer\s+"?Edge\.Cuts"?\)', text, re.S)),
    }
    return result


def admission(numbers):
    if numbers["version"] is None:
        return "not a KiCad S-expression board"
    if numbers["version"] < 20211014:
        return "legacy KiCad version; conversion required"
    if not numbers["has_outline"]:
        return "no outline geometry"
    if not 2 <= numbers["copper_layers"] <= 32:
        return "unsupported copper layer count"
    if numbers["footprints"] < 8:
        return "fewer than 8 footprints"
    if numbers["segments"] < 8:
        return "little or no reference routing"
    return None


def quality_flags(path):
    lowered = path.lower()
    flags = []
    if any(word in lowered for word in ("fixture", "testboard", "test_board", "stackup", "template", "simulation")):
        flags.append("possible_fixture_or_template")
    if any(word in lowered for word in ("backup", "/old/", "archive", "-bak", ".bak")):
        flags.append("historical_or_backup_variant")
    if any(word in lowered for word in ("panel", "kikit")):
        flags.append("possible_panel")
    return flags


def diverse_candidates(candidates):
    """Prefer useful originals and distribute limited runs across repositories."""
    groups = {}
    def priority(item):
        source = item.get("found_by", "")
        hard_package = any(word in source for word in ("BGA", "In2.Cu", "LGA", "QFN"))
        size = item.get("size", 0)
        # Enough complexity to be useful, without only picking massive panels.
        size_band = 0 if 100_000 <= size <= 10_000_000 else 1
        return (len(quality_flags(item["path"])), item["repository"] not in CURATED,
                not hard_package, size_band, -min(size, 3_000_000), item["source_id"])
    for item in sorted(candidates, key=priority):
        groups.setdefault(item["repository"], []).append(item)
    ordered = []
    for offset in range(max((len(items) for items in groups.values()), default=0)):
        for items in groups.values():
            if offset < len(items):
                ordered.append(items[offset])
    return ordered


class RateLimited(Exception):
    pass


class GitHub:
    """gh handles authentication; credentials never enter logs or manifests."""
    def __init__(self):
        self.blocked_until = 0
        self.lock = threading.Lock()

    def api(self, endpoint):
        with self.lock:
            if time.time() < self.blocked_until:
                raise RateLimited("GitHub API cooling down")
        process = subprocess.run(["gh", "api", endpoint], capture_output=True,
                                 text=True, timeout=90)
        if process.returncode:
            message = process.stderr.lower()
            if "rate limit" in message or "secondary" in message or "abuse" in message or "429" in message:
                with self.lock:
                    self.blocked_until = time.time() + 120
                raise RateLimited("GitHub rate limit; paused API requests for 120s")
            if "404" in message or "409" in message or "451" in message:
                return None
            raise RuntimeError(f"GitHub API request failed ({process.returncode}) for {endpoint}")
        return json.loads(process.stdout) if process.stdout.strip() else None

    def raw(self, repository, commit, path, maximum):
        url = "https://raw.githubusercontent.com/" + repository + "/" + commit + "/" + urllib.parse.quote(path, safe="/")
        request = urllib.request.Request(url, headers={"User-Agent": "pcb-maker-local-harvest/1"})
        try:
            with urllib.request.urlopen(request, timeout=90) as response:
                if int(response.headers.get("Content-Length", "0")) > maximum:
                    raise ValueError("file exceeds size limit")
                data = response.read(maximum + 1)
                if len(data) > maximum:
                    raise ValueError("file exceeds size limit")
                return data
        except urllib.error.HTTPError as error:
            if error.code == 404:
                return None
            raise


def load_old():
    accepted, hashes = set(), set()
    for name in ("manifest.json", "training-manifest.json"):
        path = OLD / name
        if path.exists():
            for board in json.loads(path.read_text()).get("boards", []):
                accepted.add(board["key"])
                hashes.add(board["sha256"])
    return accepted, hashes


class Harvest:
    def __init__(self, args, github=None):
        self.args = args
        self.data = args.data.resolve()
        self.data.mkdir(parents=True, exist_ok=True)
        self.github = github or GitHub()
        self.lock = threading.Lock()
        self.previous, self.hashes = load_old()
        self.outcomes = {}
        self.boards = {}
        self.candidates = {}
        self.repositories = {}
        self.repo_cache = {}
        self.repo_locks = {}
        self.hash_owner = {}
        for filename, target in (("candidates.json", self.candidates), ("repositories.json", self.repositories)):
            path = self.data / filename
            if path.exists():
                target.update(json.loads(path.read_text()))
        journal = self.data / "harvest.jsonl"
        if journal.exists():
            for line in journal.read_text().splitlines():
                try:
                    record = json.loads(line)
                except json.JSONDecodeError:
                    continue  # A killed writer may leave a partial final line.
                self.outcomes[record["source_id"]] = record
                if record.get("sha256") and record["status"] != "error":
                    self.hashes.add(record["sha256"])
                    self.hash_owner.setdefault(record["sha256"], record["source_id"])
                if record["status"] == "downloaded":
                    self.boards[record["source_id"]] = record
                    self.hashes.add(record["sha256"])
                    self.hash_owner[record["sha256"]] = record["source_id"]
        for candidate in json.loads(args.seed.read_text()) if args.seed.exists() else []:
            candidate = dict(candidate)
            candidate["source_id"] = identity(candidate["repository"], candidate["path"])
            self.candidates.setdefault(candidate["source_id"], candidate)
            self.repositories.setdefault(candidate["repository"], {})
        for repository in CURATED:
            self.repositories.setdefault(repository, {})
        self.save_inventory()
        self.publish()

    def save_inventory(self):
        atomic_json(self.data / "candidates.json", self.candidates)
        atomic_json(self.data / "repositories.json", self.repositories)

    def publish(self):
        atomic_json(self.data / "manifest.json", {"schema_version": 1,
                    "updated_at": time.time(), "boards": list(self.boards.values())})

    def record(self, record):
        with self.lock:
            record = dict(record, recorded_at=time.time())
            # Recover cleanly from interruption halfway through the final line.
            path = self.data / "harvest.jsonl"
            with path.open("a+b") as stream:
                stream.seek(0, os.SEEK_END)
                if stream.tell():
                    stream.seek(-1, os.SEEK_END)
                    if stream.read(1) != b"\n":
                        stream.seek(0, os.SEEK_END)
                        stream.write(b"\n")
                stream.seek(0, os.SEEK_END)
                stream.write((json.dumps(record) + "\n").encode())
                stream.flush()
                os.fsync(stream.fileno())
            self.outcomes[record["source_id"]] = record
            if record["status"] == "downloaded":
                self.boards[record["source_id"]] = record
                self.publish()
            print(json.dumps({"source_id": record["source_id"], "status": record["status"],
                              "reason": record.get("reason"), "accepted": len(self.boards)}), flush=True)

    def context(self, repository):
        with self.lock:
            repo_lock = self.repo_locks.setdefault(repository, threading.Lock())
        with repo_lock:
            return self._context_unlocked(repository)

    def _context_unlocked(self, repository):
        if repository in self.repo_cache:
            return self.repo_cache[repository]
        cached = self.repositories.get(repository, {})
        if cached.get("commit") and time.time() - cached.get("context_at", 0) < self.args.refresh_hours * 3600:
            self.repo_cache[repository] = cached
            return cached
        info = self.github.api(f"repos/{repository}")
        if not info:
            raise ValueError("repository unavailable")
        branch = info.get("default_branch", "main")
        head = self.github.api(f"repos/{repository}/commits/{urllib.parse.quote(branch, safe='')}")
        if not head:
            raise ValueError("repository has no readable head")
        context = dict(cached, repository=repository, commit=head["sha"],
                       tree_sha=head["commit"]["tree"]["sha"],
                       license_spdx=(info.get("license") or {}).get("spdx_id", "NOASSERTION"),
                       stars=info.get("stargazers_count", 0), fork=info.get("fork", False),
                       context_at=time.time())
        with self.lock:
            self.repositories[repository] = context
            self.repo_cache[repository] = context
        return context

    def relative(self, path):
        try:
            return str(path.relative_to(ROOT))
        except ValueError:
            return str(path)

    def fetch_repository(self, repository, candidates):
        try:
            context = self.context(repository)
        except RateLimited as error:
            print(str(error), flush=True)
            return
        except Exception as error:
            for candidate in candidates:
                self.record(dict(candidate, status="error", reason=str(error)))
            return
        for candidate in candidates:
            record = dict(candidate, **{key: context[key] for key in ("commit", "license_spdx", "stars")})
            path = candidate["path"]
            source_id = candidate["source_id"]
            name = safe_name(source_id)
            directory = self.data / "boards" / name
            source_name = PurePosixPath(path).name
            board_path = directory / source_name
            record.update(name=name, board_id=PurePosixPath(path).stem,
                          directory=self.relative(directory), board_path=self.relative(board_path),
                          source_url=f"https://github.com/{repository}/blob/{context['commit']}/{urllib.parse.quote(path, safe='/')}",
                          source_format=FORMATS.get(PurePosixPath(path).suffix.lower(), "unknown"),
                          nn_license_eligible=nn_license_eligible(context["license_spdx"]),
                          license_scope="repository metadata; per-file exceptions not checked",
                          quality_flags=quality_flags(path))
            try:
                if candidate.get("size", 0) > self.args.max_bytes:
                    self.record(dict(record, status="rejected", reason="file exceeds configured size limit"))
                    continue
                board_bytes = self.github.raw(repository, context["commit"], path, self.args.max_bytes)
                if board_bytes is None:
                    self.record(dict(record, status="rejected", reason="board absent at pinned head"))
                    continue
                directory.mkdir(parents=True, exist_ok=True)
                board_path.write_bytes(board_bytes)
                digest = hashlib.sha256(board_bytes).hexdigest()
                record.update(sha256=digest, size=len(board_bytes))
                with self.lock:
                    duplicate = digest in self.hashes
                    if not duplicate:
                        self.hashes.add(digest)
                        self.hash_owner[digest] = source_id
                    duplicate_of = self.hash_owner.get(digest, "existing benchmark/training collection")
                if duplicate:
                    self.record(dict(record, status="duplicate", duplicate_of=duplicate_of))
                    continue
                if record["source_format"] != "kicad":
                    self.record(dict(record, status="conversion_pending", reason="requires native KiCad import before triage"))
                    continue
                numbers = features(board_bytes.decode("utf-8", errors="replace"))
                record["features"] = numbers
                record.update(numbers)
                reason = admission(numbers)
                if reason:
                    self.record(dict(record, status="rejected", reason=reason))
                    continue
                project_source = str(PurePosixPath(path).with_suffix(".kicad_pro"))
                project = self.github.raw(repository, context["commit"], project_source, 5_000_000)
                record.update(project_path=None, rules_inferred=project is None)
                if project is not None:
                    project_path = board_path.with_suffix(".kicad_pro")
                    project_path.write_bytes(project)
                    record["project_path"] = self.relative(project_path)
                    record["project_sha256"] = hashlib.sha256(project).hexdigest()
                rules_source = str(PurePosixPath(path).with_suffix(".kicad_dru"))
                rules = self.github.raw(repository, context["commit"], rules_source, 1_000_000)
                if rules:
                    rules_path = board_path.with_suffix(".kicad_dru")
                    rules_path.write_bytes(rules)
                    record["custom_rules_path"] = self.relative(rules_path)
                    record["custom_rules_sha256"] = hashlib.sha256(rules).hexdigest()
                self.record(dict(record, status="downloaded"))
            except ValueError as error:
                self.record(dict(record, status="rejected", reason=str(error)))
            except Exception as error:
                # A failed sidecar download must not poison dedup on retry.
                with self.lock:
                    if record.get("sha256") and self.hash_owner.get(record["sha256"]) == source_id:
                        self.hashes.discard(record["sha256"])
                        self.hash_owner.pop(record["sha256"], None)
                self.record(dict(record, status="error", reason=str(error)))

    def fetch(self):
        groups = {}
        pending = diverse_candidates(self.candidates.values())
        taken = 0
        for candidate in pending:
            key = candidate["source_id"]
            if key in self.previous:
                continue
            outcome = self.outcomes.get(key)
            if outcome and (outcome["status"] != "error" or time.time() - outcome["recorded_at"] < self.args.retry_after):
                continue
            if PurePosixPath(candidate["path"]).suffix.lower() != ".kicad_pcb" and not self.args.non_kicad:
                continue
            if self.args.limit and taken >= self.args.limit:
                break
            groups.setdefault(candidate["repository"], []).append(candidate)
            taken += 1
        print(f"Fetching {taken} candidates in {len(groups)} repositories with {self.args.workers} workers", flush=True)
        # Four boards per task, interleaved across repositories: a large fixture
        # repository cannot monopolize one downloader for hours.
        batches = []
        for offset in range(0, max((len(boards) for boards in groups.values()), default=0), 4):
            for repository, boards in groups.items():
                if offset < len(boards):
                    batches.append((repository, boards[offset:offset + 4]))
        with concurrent.futures.ThreadPoolExecutor(max_workers=self.args.workers) as executor:
            futures = [executor.submit(self.fetch_repository, repository, boards) for repository, boards in batches]
            for future in concurrent.futures.as_completed(futures):
                future.result()
        with self.lock:
            self.save_inventory()

    def tree(self, repository, sha):
        result = self.github.api(f"repos/{repository}/git/trees/{sha}?recursive=1")
        if not result:
            return []
        if not result.get("truncated"):
            return result.get("tree", [])
        # GitHub truncates huge recursive trees. Walk subtrees instead of silently losing boards.
        entries = []
        stack = [("", sha)]
        while stack:
            prefix, subtree = stack.pop()
            result = self.github.api(f"repos/{repository}/git/trees/{subtree}")
            for entry in (result or {}).get("tree", []):
                path = prefix + entry["path"]
                if entry["type"] == "tree":
                    stack.append((path + "/", entry["sha"]))
                else:
                    entries.append(dict(entry, path=path))
        return entries

    def scan(self, repository):
        try:
            context = self.context(repository)
            entries = self.tree(repository, context["tree_sha"])
            found = []
            for entry in entries:
                path = entry["path"]
                if entry.get("type") != "blob" or PurePosixPath(path).suffix.lower() not in FORMATS:
                    continue
                key = identity(repository, path)
                found.append((key, dict(source_id=key, key=key, repository=repository, path=path,
                                        size=entry.get("size", 0), blob_sha=entry["sha"], found_by="complete-repository-tree")))
            with self.lock:
                for key, item in found:
                    self.candidates.setdefault(key, item)
                self.repositories[repository].update(scanned_at=time.time(), scanned_commit=context["commit"],
                                                      discovered_boards=len(found))
                self.save_inventory()
            print(f"Scanned {repository}: {len(found)} PCB source files", flush=True)
        except Exception as error:
            with self.lock:
                self.repositories.setdefault(repository, {}).update(scan_error=str(error), scan_error_at=time.time())
                self.save_inventory()
            print(f"Scan deferred {repository}: {error}", flush=True)

    def discover(self):
        cutoff = time.time() - self.args.refresh_hours * 3600
        repositories = [name for name, state in self.repositories.items()
                        if state.get("scanned_at", 0) < cutoff and state.get("scan_error_at", 0) < time.time() - self.args.retry_after]
        # Higher-star and curated repositories first, then every seeded repository.
        repositories.sort(key=lambda name: (name not in CURATED, -self.repositories[name].get("stars", 0), name))
        if self.args.scan_limit:
            repositories = repositories[:self.args.scan_limit]
        with concurrent.futures.ThreadPoolExecutor(max_workers=min(self.args.workers, 3)) as executor:
            for start in range(0, len(repositories), self.args.scan_batch):
                futures = [executor.submit(self.scan, repository) for repository in repositories[start:start + self.args.scan_batch]]
                for future in concurrent.futures.as_completed(futures):
                    future.result()
                if self.args.command == "run":
                    self.fetch()
                if time.time() < self.github.blocked_until:
                    break

    def search_topics(self):
        state_path = self.data / "search-state.json"
        state = json.loads(state_path.read_text()) if state_path.exists() else {}
        for topic in TOPICS:
            for page in range(1, self.args.pages + 1):
                key = f"{topic}:{page}"
                if time.time() - state.get(key, 0) < self.args.refresh_hours * 3600:
                    continue
                query = urllib.parse.urlencode({"q": f"topic:{topic} fork:false", "sort": "stars", "per_page": 100, "page": page})
                try:
                    result = self.github.api("search/repositories?" + query)
                except Exception as error:
                    print(f"Topic search deferred: {error}", flush=True)
                    return
                with self.lock:
                    for item in (result or {}).get("items", []):
                        self.repositories.setdefault(item["full_name"], {"stars": item.get("stargazers_count", 0)})
                    self.save_inventory()
                    state[key] = time.time()
                    atomic_json(state_path, state)
                if not result or len(result.get("items", [])) < 100:
                    break
                time.sleep(2.1)  # Search API is separately rate limited.


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("run", "fetch", "discover", "status"))
    parser.add_argument("--data", type=Path, default=HERE / "data")
    parser.add_argument("--seed", type=Path, default=OLD / "candidates.json")
    parser.add_argument("--workers", type=int, default=6)
    parser.add_argument("--limit", type=int, default=0, help="maximum candidates fetched per pass, 0 unlimited")
    parser.add_argument("--scan-limit", type=int, default=0)
    parser.add_argument("--scan-batch", type=int, default=30)
    parser.add_argument("--pages", type=int, default=5, help="topic search pages, up to 10")
    parser.add_argument("--max-bytes", type=int, default=40_000_000)
    parser.add_argument("--refresh-hours", type=float, default=168)
    parser.add_argument("--retry-after", type=float, default=1800)
    parser.add_argument("--non-kicad", action="store_true", help="also retain Eagle/Altium source files pending conversion")
    parser.add_argument("--watch", action="store_true")
    args = parser.parse_args()
    if not 1 <= args.workers <= 16 or not 1 <= args.pages <= 10 or args.scan_batch < 1:
        parser.error("workers must be 1..16, pages 1..10, scan-batch positive")
    # Only one writer may own a dataset. triage.py reads the atomic manifest freely.
    import fcntl
    args.data.mkdir(parents=True, exist_ok=True)
    with (args.data / ".harvest.lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            parser.error("a harvest process already owns this dataset")
        harvest = Harvest(args)
        if args.command == "status":
            from collections import Counter
            print(json.dumps({"candidates": len(harvest.candidates), "repositories": len(harvest.repositories),
                              "outcomes": dict(Counter(record["status"] for record in harvest.outcomes.values()))}, indent=2))
            return
        while True:
            if args.command in ("run", "fetch"):
                harvest.fetch()
            if args.command in ("run", "discover"):
                harvest.search_topics()
                harvest.discover()
            if not args.watch:
                break
            time.sleep(30)


if __name__ == "__main__":
    main()
