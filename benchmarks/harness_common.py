# Copyright (C) 2026 Toit contributors.
"""What the benchmark harnesses share: file digests, atomic JSON, safe
names, KiCad's DRC counting filter, the machine's available memory and
children run in a process group of their own (killed as a group, waited
for until the group is gone).

Used by benchmarks/agent-tasks (run.py, compare.py, quality.py) and
benchmarks/expansion (harvest.py, triage.py, import_pending.py). Import it
with the benchmarks directory on the path:

    sys.path.append(str(Path(__file__).resolve().parents[1]))
    import harness_common
"""

import hashlib
import json
import os
import re
import signal
import subprocess
import tempfile
import time
from pathlib import Path

# KiCad finding types that are not copper errors: silkscreen and text,
# library and courtyard bookkeeping, isolated copper islands.
COSMETIC = re.compile(r"^(silk_|text_|lib_footprint|footprint_type_mismatch|missing_courtyard|isolated_copper|nonmirrored_text)")


def counts_as_error(violation, missing_severity=None):
    """Whether a KiCad DRC violation counts as an error: error severity
    (`missing_severity` stands in for a report without one), not
    excluded, not cosmetic (COSMETIC)."""
    return (violation.get("severity", missing_severity) == "error" and not violation.get("excluded")
            and not COSMETIC.match(violation.get("type", "")))


def error_counts(report, missing_severity=None):
    """The errors that count of a DRC report, per type."""
    counts = {}
    for violation in report.get("violations", []):
        if counts_as_error(violation, missing_severity):
            kind = violation.get("type", "unknown")
            counts[kind] = counts.get(kind, 0) + 1
    return counts


def file_sha256(path):
    """The SHA-256 of a file, read in 1 MB chunks."""
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def atomic_json(path, value, indent=2, sort_keys=False, newline=False, make_parents=False):
    """Writes `value` as JSON to `path` through a temporary file in the
    same directory, synced and renamed over it: a reader sees the old file
    or the new one, never a part."""
    path = Path(path)
    if make_parents:
        path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, prefix=path.name + ".", suffix=".tmp",
                                     delete=False) as stream:
        temporary = Path(stream.name)
        try:
            json.dump(value, stream, indent=indent, sort_keys=sort_keys)
            if newline:
                stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    temporary.replace(path)


def atomic_text(path, text):
    """Writes `text` to `path` as atomic_json writes JSON."""
    path = Path(path)
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, prefix=path.name + ".", suffix=".tmp",
                                     delete=False) as stream:
        temporary = Path(stream.name)
        stream.write(text)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


def read_json(path):
    """A JSON file's value, or None when it cannot be read or parsed."""
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError):
        return None


def safe_name(source_id, limit=100):
    """A file name for a source id: its readable part (at most `limit`
    characters) and 12 hex digits of its SHA-256."""
    readable = re.sub(r"[^A-Za-z0-9_.-]+", "_", source_id)[:limit]
    return readable + "__" + hashlib.sha256(source_id.encode()).hexdigest()[:12]


def memory_available_bytes():
    """The machine's MemAvailable in bytes, None when /proc/meminfo cannot
    be read. The harnesses' memory gates all start from it."""
    try:
        with open("/proc/meminfo") as meminfo:
            for line in meminfo:
                if line.startswith("MemAvailable:"):
                    return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        pass
    return None


def group_members(group):
    """The pids of the live (not zombie) processes in process group
    `group`, from /proc."""
    members = []
    try:
        entries = os.listdir("/proc")
    except OSError:
        return members
    for entry in entries:
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/stat") as stat:
                # The command may hold spaces and parentheses: the fields
                # after the last ")" are fixed (state, ppid, pgrp, ...).
                fields = stat.read().rsplit(")", 1)[1].split()
        except (OSError, IndexError):
            continue
        if len(fields) > 2 and fields[0] != "Z" and int(fields[2]) == group:
            members.append(int(entry))
    return members


def kill_group(group, sig=signal.SIGKILL):
    """Sends `sig` to every process of group `group` (a child started with
    start_new_session=True: its pid). Quiet when the group is gone."""
    try:
        os.killpg(group, sig)
    except (ProcessLookupError, PermissionError):
        pass


def end_group(group, wait_seconds=60.0, poll_seconds=0.2):
    """Kills what is left of process group `group` (a timed-out child's
    kicad-cli, a stray child of a run that ended) and waits until none of
    it is alive: init reaps the orphans. Returns the pids that were left,
    empty when the group had ended on its own."""
    left = group_members(group)
    if not left:
        return []
    kill_group(group)
    deadline = time.monotonic() + wait_seconds
    while group_members(group) and time.monotonic() < deadline:
        time.sleep(poll_seconds)
    return left


def run_group(command, timeout=None, **options):
    """subprocess.run for a child that may start children of its own (the
    binary's kicad-cli): the child gets a process group of its own; on a
    timeout the whole group is killed and waited for before
    subprocess.TimeoutExpired is raised, and whatever the child left
    running when it ended is killed too. `options` are subprocess.Popen's
    (capture_output, text, cwd, stdout, stderr, env, check)."""
    check = options.pop("check", False)
    if options.pop("capture_output", False):
        options["stdout"] = subprocess.PIPE
        options["stderr"] = subprocess.PIPE
    process = subprocess.Popen(command, start_new_session=True, **options)
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        kill_group(process.pid)
        process.communicate()
        end_group(process.pid)
        raise
    except BaseException:
        kill_group(process.pid)
        process.wait()
        end_group(process.pid)
        raise
    end_group(process.pid)
    completed = subprocess.CompletedProcess(process.args, process.returncode, stdout, stderr)
    if check:
        completed.check_returncode()
    return completed
