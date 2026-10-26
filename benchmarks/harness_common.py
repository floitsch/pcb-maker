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


# The machine's memory ledger (crates/pcb-kicad/src/ledger.rs: the same
# file and rules). Every route, layout and KiCad check of a binary since
# x1590 reserves its memory in it before it allocates, and the harnesses
# admit a board against it: the sum of the live reservations and the
# board's expected size against the machine, so that runs started side by
# side (other agents' tiers, sweeps, triage) see what the others will take.
# Until 2026-10-22 each harness judged by MemAvailable alone and runs
# admitted together grew into each other (two OOM rounds).
LEDGER_MARGIN_MB = 6144.0

# The drain (crates/pcb-kicad/src/ledger.rs, `drain_allows`, since
# 2026-10-25): how long the first waiter waits before newer requests are
# held to what leaves it its room (an entry's `drain_after`, run.py's
# --ledger-drain-after), and the nightly's priority (run.py's
# --ledger-priority nightly: first among the waiters, drained for at once,
# never passed).
DEFAULT_DRAIN_AFTER_SECONDS = 600.0
PRIORITY_NIGHTLY = "nightly"

# The drain's feasibility cap (ledger.rs, SMALL_REQUEST_MB, since
# 2026-10-26): while the waiter cannot fit before a run larger than this
# ends (`blocked_by_large`), requests of at most this many megabytes are
# not held back by the drain.
SMALL_REQUEST_MB = 4096.0

# The drain only for a waiter the ledger could start (ledger.rs,
# `drain_state`, since 2026-10-26): how long a waiter's wait may hold newer
# requests to a drain (an entry's `drain_max`, run.py's --ledger-drain-max),
# after how long without a rewrite (`refreshed`) a waiting entry is stale,
# and the floor assumed for an older writer's entry that names none.
DEFAULT_DRAIN_MAX_SECONDS = 7200.0
STALE_AFTER_SECONDS = 120.0
DEFAULT_FLOOR_MB = 10240.0

# Short requests never wait behind a drain (ledger.rs, `short_request`,
# `short_allows`, since 2026-10-26): a request of at most SHORT_OWN_MB of
# its own beside at most one KiCad check (KICAD_CHECK_MB, kicad-cli's 2.2 GB
# on any board) that is declared short goes once the memory for it is there,
# whatever the order and the drain say; its reservation (`short`) counts as
# gone in the order's and the drain's sums for the others.
SHORT_OWN_MB = 2048.0
KICAD_CHECK_MB = 2.2 * 1024


def ledger_path(environ=None):
    """The ledger file PCB_MAKER_LEDGER names (default
    $XDG_RUNTIME_DIR/pcb-maker-ledger.json, else /tmp/...), None for
    `off`."""
    environ = os.environ if environ is None else environ
    value = (environ.get("PCB_MAKER_LEDGER") or "").strip()
    if value == "off":
        return None
    if value:
        return Path(value)
    runtime = environ.get("XDG_RUNTIME_DIR")
    directory = Path(runtime) if runtime and Path(runtime).is_dir() else Path("/tmp")
    return directory / "pcb-maker-ledger.json"


def _positive_mb(environ, name, allow_zero=False):
    try:
        value = float((environ.get(name) or "").strip())
    except ValueError:
        return None
    return value if value == value and value != float("inf") and (value > 0 or allow_zero and value == 0) \
        else None


def memory_total_mb():
    """MemTotal in MB, None when /proc/meminfo cannot be read."""
    try:
        with open("/proc/meminfo") as meminfo:
            for line in meminfo:
                if line.startswith("MemTotal:"):
                    return int(line.split()[1]) / 1024
    except (OSError, ValueError, IndexError):
        pass
    return None


def ledger_capacity_mb(environ=None):
    """What the reservations may take together: PCB_MAKER_LEDGER_TOTAL_MB,
    else MemTotal less PCB_MAKER_LEDGER_MARGIN_MB (default 6144). None when
    unknown (no waiting on the ledger then)."""
    environ = os.environ if environ is None else environ
    total = _positive_mb(environ, "PCB_MAKER_LEDGER_TOTAL_MB")
    if total is not None:
        return total
    margin = _positive_mb(environ, "PCB_MAKER_LEDGER_MARGIN_MB", allow_zero=True)
    machine = memory_total_mb()
    return None if machine is None else max(0.0, machine - (LEDGER_MARGIN_MB if margin is None else margin))


def _stat_fields(pid):
    try:
        with open(f"/proc/{pid}/stat") as stat:
            return stat.read().rsplit(")", 1)[1].split()
    except (OSError, IndexError):
        return None


def start_ticks(pid):
    """A process's start time in clock ticks after boot, None if unknown."""
    fields = _stat_fields(pid)
    try:
        return int(fields[19]) if fields else None
    except (IndexError, ValueError):
        return None


def reservation_alive(reservation):
    """Whether a reservation's process lives (not a zombie) and is the one
    that reserved (its start time, where recorded)."""
    fields = _stat_fields(reservation.get("pid", -1))
    if not fields or fields[0] in ("Z", "X", "x"):
        return False
    ticks = reservation.get("start_ticks")
    try:
        return ticks is None or int(fields[19]) == ticks
    except (IndexError, ValueError):
        return False


def resident_mb(pid):
    """A process's resident size in MB (0 when gone)."""
    try:
        with open(f"/proc/{pid}/status") as status:
            for line in status:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024
    except (OSError, ValueError, IndexError):
        pass
    return 0.0


class Ledger:
    """The machine's memory ledger (`path`), or one of this process alone
    (`path` None: PCB_MAKER_LEDGER=off). Every update takes an exclusive
    flock on `<path>.lock`; reservations of dead pids are dropped on every
    read."""

    def __init__(self, path):
        self.path = None if path is None else Path(path)
        self._memory = []
        self._threads = __import__("threading").Lock()

    @classmethod
    def from_env(cls, environ=None):
        return cls(ledger_path(environ))

    def update(self, change):
        """Applies `change(reservations)` (a list it may change in place)
        to the live reservations under the lock; returns its result."""
        import fcntl
        with self._threads:
            if self.path is None:
                self._memory[:] = [r for r in self._memory if reservation_alive(r)]
                return change(self._memory)
            self.path.parent.mkdir(parents=True, exist_ok=True)
            with open(str(self.path) + ".lock", "a") as lock:
                fcntl.flock(lock, fcntl.LOCK_EX)
                stored = (read_json(self.path) or {}).get("reservations", []) if self.path.exists() else []
                if not isinstance(stored, list):
                    stored = []
                reservations = [dict(r) for r in stored if isinstance(r, dict) and reservation_alive(r)]
                outcome = change(reservations)
                if reservations != stored:
                    atomic_json(self.path, {"reservations": reservations}, newline=True)
                return outcome

    def live(self):
        return self.update(lambda reservations: [dict(r) for r in reservations])

    def reserve(self, megabytes, label, pid=None):
        """Adds a reservation of `megabytes` for `pid` (default: this
        process); returns its key for `move` and `release`."""
        pid = pid or os.getpid()
        entry = {"pid": pid, "started": time.time(), "start_ticks": start_ticks(pid),
                 "megabytes": float(megabytes), "label": label}
        if entry["start_ticks"] is None:
            del entry["start_ticks"]
        self.update(lambda reservations: reservations.append(entry))
        return (entry["pid"], entry["started"], label)

    def move(self, key, pid):
        """Hands reservation `key` to process `pid` (the binary it was made
        for): it lives as long as that process. A binary that reserved for
        itself already makes it superfluous. Returns the new key."""
        def change(reservations):
            mine = [r for r in reservations if (r["pid"], r["started"], r["label"]) == key]
            if not mine:
                return None
            if any(r["pid"] == pid for r in reservations):
                reservations.remove(mine[0])
                return None
            mine[0]["pid"] = pid
            ticks = start_ticks(pid)
            if ticks is None:
                mine[0].pop("start_ticks", None)
            else:
                mine[0]["start_ticks"] = ticks
            return (pid, mine[0]["started"], key[2])
        return self.update(change)

    def release(self, key):
        """Removes reservation `key` (gone already: nothing)."""
        if key is None:
            return
        self.update(lambda reservations: reservations.__setitem__(
            slice(None), [r for r in reservations if (r["pid"], r["started"], r["label"]) != key]))


def growth_mb(reservations, resident=resident_mb, own_pid=None):
    """What the reservations are still expected to grow by: each one's
    megabytes less its process's resident size (at least 0; one of this
    process counts whole)."""
    own_pid = own_pid or os.getpid()
    return sum(max(0.0, r.get("megabytes", 0.0) - (0.0 if r.get("pid") == own_pid else resident(r["pid"])))
               for r in reservations if r.get("megabytes", 0.0) > 0)


def memory_room(reservations, megabytes, min_free_mb=0.0, capacity_mb=None, available_mb=None,
                resident=resident_mb, own_pid=None, alone=True):
    """The one admission rule of the harnesses: whether `megabytes` more
    fit, and the numbers for the log. Both must hold, unless nothing is
    reserved (then anything goes alone):
    - the ledger: the live reservations and `megabytes` within the
      capacity (ledger_capacity_mb);
    - the floor: MemAvailable, less what the reserving processes are still
      expected to grow by (their reservation less their resident size; a
      reservation of this harness itself counts whole), leaves
      `min_free_mb` after `megabytes`.
    `capacity_mb` and `available_mb` None: unknown, that rule holds.
    `alone` False: the floor holds also when nothing is reserved (triage
    fails closed)."""
    own_pid = own_pid or os.getpid()
    # Waiting requests (megabytes 0, waiting_mb set) hold nothing.
    holders = [r for r in reservations if r.get("megabytes", 0.0) > 0]
    waiting = len(reservations) - len(holders)
    reserved = sum(r.get("megabytes", 0.0) for r in reservations)
    growth = growth_mb(reservations, resident, own_pid)
    ledger_ok = capacity_mb is None or reserved + megabytes <= capacity_mb
    floor_ok = available_mb is None or available_mb - growth - megabytes >= min_free_mb
    fits = (ledger_ok and floor_ok) or (alone and not holders) or (not alone and not holders and floor_ok)
    text = (f"{reserved / 1024:.1f} GB reserved by {len(holders)} run(s)"
            + ("" if capacity_mb is None else f" of {capacity_mb / 1024:.1f} GB")
            + ("" if not waiting else f", {waiting} waiting")
            + ("" if available_mb is None else
               f", {available_mb / 1024:.1f} GB available ({growth / 1024:.1f} GB still to grow)")
            + f", {megabytes / 1024:.1f} GB expected, {min_free_mb / 1024:.1f} GB to keep")
    return fits, text


def _waiting_since(reservation):
    return reservation.get("waiting_since", reservation.get("started", 0.0))


def _held_since(reservation):
    return reservation.get("held_since", reservation.get("started", 0.0))


def _rank(priority):
    return 0 if priority == PRIORITY_NIGHTLY else 1


def _order_key(reservation):
    """A waiter's place: the nightly's first, then by when its wait began,
    then by pid."""
    return (_rank(reservation.get("priority")), _waiting_since(reservation), reservation.get("pid", 0))


def _drain_after(reservation):
    if reservation.get("priority") == PRIORITY_NIGHTLY:
        return 0.0
    value = reservation.get("drain_after")
    return float(value) if isinstance(value, (int, float)) and value >= 0 else DEFAULT_DRAIN_AFTER_SECONDS


def _drain_max(reservation):
    value = reservation.get("drain_max")
    return float(value) if isinstance(value, (int, float)) and value >= 0 else DEFAULT_DRAIN_MAX_SECONDS


def assumed_floor_mb(reservation):
    """The floor a waiter's harness keeps beside it, for whether the ledger
    could start it (ledger.rs, `Reservation::assumed_floor_mb`): `floor_mb`;
    for an older writer's entry (no `refreshed`) without one,
    DEFAULT_FLOOR_MB; else 0."""
    floor = reservation.get("floor_mb")
    if isinstance(floor, (int, float)):
        return max(0.0, float(floor))
    return DEFAULT_FLOOR_MB if reservation.get("refreshed") is None else 0.0


def oldest_waiter(reservations):
    """The waiting request the others let go first (the nightly's, then the
    one that has waited longest, ties: the lower pid), None when nothing
    waits."""
    waiters = [r for r in reservations if r.get("waiting_mb") is not None]
    return min(waiters, key=_order_key) if waiters else None


def order_allows(others, megabytes, since, capacity_mb, own_pid=None, priority=None):
    """Ordered admission (crates/pcb-kicad/src/ledger.rs, `order_allows`:
    the same rule): whether a new request of `megabytes`, waiting since
    `since` (now when it has not waited), may be admitted beside `others`
    (the live reservations but its own) without passing the request that
    has waited longest. No older waiter, or no capacity known: yes. While
    the oldest fits now, only with room for both; while it cannot fit (or
    only its harness's floor holds it back, `waits_on_floor`), only with
    room for it beside the reservations that began holding after it began
    waiting and this request: once the runs it waits on end, it fits.
    Until 2026-10-24 a large request waited while every smaller one that
    came later took the memory as it freed (a designer-tier run: 3 rows in
    75 minutes beside another agent's stream of small layouts). The first
    waiter is the nightly's where one waits (`priority`: this request's)."""
    if capacity_mb is None:
        return True
    own_pid = own_pid or os.getpid()
    oldest = oldest_waiter(others)
    if oldest is None or _order_key(oldest) >= (_rank(priority), since, own_pid):
        return True
    wants = float(oldest["waiting_mb"])
    # A short request's reservation counts as gone (it ends within seconds
    # and has not passed the oldest; the capacity counts it, memory_room).
    rest = [r for r in others if r is not oldest and not r.get("short")]
    reserved = sum(r.get("megabytes", 0.0) for r in rest)
    fits_now = reserved + wants <= capacity_mb or not any(r.get("megabytes", 0.0) > 0 for r in rest)
    if fits_now and not oldest.get("waits_on_floor"):
        return reserved + megabytes + wants <= capacity_mb
    newer = sum(r.get("megabytes", 0.0) for r in rest
                if r.get("megabytes", 0.0) > 0 and _held_since(r) >= _waiting_since(oldest))
    return newer + megabytes + wants <= capacity_mb


def drained_for(others, since, now, own_pid=None, priority=None, capacity_mb=None, available_mb=None):
    """The waiter a new request (waiting since `since`, `priority`) is held
    to a drain for at `now` (crates/pcb-kicad/src/ledger.rs, `drained_for`):
    the first waiter, when it comes before the request and the ledger
    drains for it (drain_state: it has waited its `drain_after`, default
    DEFAULT_DRAIN_AFTER_SECONDS, the nightly's at once, and the ledger
    could start it). `capacity_mb`, `available_mb` (MemAvailable less
    growth_mb): None, unknown, unbounded. None: no drain."""
    own_pid = own_pid or os.getpid()
    waiter = oldest_waiter(others)
    if waiter is None or _order_key(waiter) >= (_rank(priority), since, own_pid):
        return None
    return waiter if drain_state(waiter, others, now, capacity_mb, available_mb)[0] == "drains" else None


def drain_state(waiter, others, now, capacity_mb=None, available_mb=None):
    """Whether the ledger drains for `waiter` beside `others` at `now`
    (crates/pcb-kicad/src/ledger.rs, `drain_state`: the same rule, since
    2026-10-26), and why not: ("not yet", None) before its `drain_after`;
    ("stale", seconds) when its entry was last rewritten (`refreshed`) more
    than STALE_AFTER_SECONDS ago (absent: unknown, live); ("past bound",
    seconds) when its wait is longer than its `drain_max` (default
    DEFAULT_DRAIN_MAX_SECONDS); ("cannot start", room) when, with every
    reservation that began holding since its wait began gone, its room (the
    capacity less the older reservations, and `available_mb` with the newer
    ones' megabytes back less its floor, assumed_floor_mb) falls short of
    what it waits for; else ("drains", None). Not drained for, it is passed
    by ordered admission alone and keeps its place. Until then a drain for
    an old harness's waiter of 24.7 GB with a floor of 10 GB that could not
    start held every request of more than 4 GB back for hours."""
    waited = now - _waiting_since(waiter)
    if waited < _drain_after(waiter):
        return "not yet", None
    refreshed = waiter.get("refreshed")
    if isinstance(refreshed, (int, float)) and now - refreshed > STALE_AFTER_SECONDS:
        return "stale", now - refreshed
    if waited > _drain_max(waiter):
        return "past bound", _drain_max(waiter)
    rest = [r for r in others if r is not waiter]
    newer = sum(r.get("megabytes", 0.0) for r in rest
                if r.get("megabytes", 0.0) > 0 and (r.get("short") or _held_since(r) >= _waiting_since(waiter)))
    room = min(_waiter_room(waiter, rest, capacity_mb, available_mb, freed_mb=newer,
                            floor_mb=assumed_floor_mb(waiter)))
    if room < float(waiter["waiting_mb"]):
        return "cannot start", room
    return "drains", None


def describe_drain(state):
    """What a drain_state means, for the waiting lines (ledger.rs,
    `Drain::describe`)."""
    kind, value = state
    if kind == "drains":
        return "the ledger drains for it"
    if kind == "stale":
        return f"not drained for: its entry was last rewritten {value:.0f} s ago (stale)"
    if kind == "past bound":
        return f"not drained for: it has waited past the drain's bound of {value:.0f} s"
    if kind == "cannot start":
        return (f"not drained for: with every run since its wait began gone its room would be {value:.0f} MB "
                "(the capacity, and the memory available less its floor), too little to start it")
    return "not drained for yet"


def drain_allows(waiter, others, megabytes, capacity_mb, available_mb, expected_end=None, small_mb=SMALL_REQUEST_MB):
    """The drain (crates/pcb-kicad/src/ledger.rs, `drain_allows`: the same
    rule, since 2026-10-25): whether a new request of `megabytes`, expected
    to end at `expected_end` (None: unknown), may go beside `others` while
    the ledger drains for `waiter`. Only if after it the waiter's room
    still covers what it waits for: the capacity less every other
    reservation, and `available_mb` (MemAvailable less growth_mb; None:
    unknown) less the waiter's `floor_mb` (one that holds memory counts it
    back); or, except for the nightly's waiter, if it ends no later than
    every run that holds memory (each with an `expected_end`). Until then
    the nightly's designer boards (17-27 GB, a floor of 10 GB) waited from
    11:00 to past 20:00 while five agents' 1-8 GB routes kept the running
    set from draining (katia: 33034 s). Or (the feasibility cap, since
    2026-10-26) if it is small (at most SMALL_REQUEST_MB) and the waiter
    cannot fit before a larger run ends anyway (blocked_by_large): then
    holding it back gains the waiter nothing, and a waiter of 23-28 GB
    behind a 28 GB run held every other agent's request for hours.
    `small_mb`: the bound (0: no cap, the rule of 2026-10-25). A short
    request's reservation (`short`) counts as gone (it ends within seconds)."""
    wants = float(waiter["waiting_mb"])
    rest = [r for r in others if r is not waiter]
    if min(_waiter_room(waiter, rest, capacity_mb, available_mb, megabytes, freed_mb=_short_held_mb(rest))) >= wants:
        return True
    if megabytes <= small_mb and blocked_by_large(waiter, others, capacity_mb, available_mb, small_mb):
        return True
    if waiter.get("priority") == PRIORITY_NIGHTLY or expected_end is None:
        return False
    holders = [r for r in rest if r.get("megabytes", 0.0) > 0 and not r.get("short")]
    return bool(holders) and all(isinstance(r.get("expected_end"), (int, float)) and expected_end <= r["expected_end"]
                                 for r in holders)


def _waiter_room(waiter, rest, capacity_mb, available_mb, megabytes=0.0, freed_mb=0.0, floor_mb=None):
    """The drain's two rooms for `waiter` beside `rest` after a request of
    `megabytes`, with `freed_mb` of the reservations gone: the capacity
    less the reservations, and `available_mb` (None: unknown, unbounded)
    less the waiter's floor (`floor_mb`, default its entry's; what the
    waiter holds counts back)."""
    inf = float("inf")
    if floor_mb is None:
        floor_mb = max(0.0, float(waiter.get("floor_mb") or 0.0))
    reserved = sum(r.get("megabytes", 0.0) for r in rest) - freed_mb
    ledger_room = inf if capacity_mb is None else capacity_mb - reserved - megabytes
    floor_room = inf if available_mb is None else (
        available_mb + freed_mb - megabytes - floor_mb + waiter.get("megabytes", 0.0))
    return ledger_room, floor_room


def blocked_by_large(waiter, others, capacity_mb, available_mb, small_mb=SMALL_REQUEST_MB):
    """Whether `waiter` cannot fit before a run larger than
    SMALL_REQUEST_MB ends (crates/pcb-kicad/src/ledger.rs,
    `blocked_by_large`: the same rule): such a run holds memory, and with
    every smaller run gone the waiter's room (_waiter_room: the capacity,
    and the memory available, each with the smaller runs' megabytes back)
    still does not cover what it waits for. Then the small runs do not
    delay it: the drain lets them go (drain_allows), and holds back only
    the large ones until that run ends; then it drains in full. `small_mb`:
    the bound."""
    rest = [r for r in others if r is not waiter]
    holders = [r for r in rest if r.get("megabytes", 0.0) > 0 and not r.get("short")]
    if not any(r["megabytes"] > small_mb for r in holders):
        return False
    small = sum(r["megabytes"] for r in holders if r["megabytes"] <= small_mb) + _short_held_mb(rest)
    return min(_waiter_room(waiter, rest, capacity_mb, available_mb, freed_mb=small)) < float(waiter["waiting_mb"])


def _short_held_mb(reservations):
    """What the short requests' reservations (`short`) among
    `reservations` hold: counted as gone in the drain's rooms."""
    return sum(r.get("megabytes", 0.0) for r in reservations if r.get("short") and r.get("megabytes", 0.0) > 0)


def short_request(own_mb, children_mb=0.0):
    """Whether a request declared short of `own_mb` for its process and
    `children_mb` for its children is short (ledger.rs, `short_request`):
    at most SHORT_OWN_MB of its own beside at most one KiCad check."""
    return own_mb <= SHORT_OWN_MB and children_mb <= KICAD_CHECK_MB


def _is_waiting_entry(reservation, pid, label, since):
    return (since is not None and reservation.get("pid") == pid and reservation.get("label") == label
            and reservation.get("waiting_mb") is not None and _waiting_since(reservation) == since)


def try_admit(ledger, megabytes, label, min_free_mb=0.0, since=None, expected_seconds=None, drain_after=None,
              priority=None, drain_max=None, short=False):
    """Reserves `megabytes` under `label` for this process when they fit
    (memory_room against the ledger, its capacity and MemAvailable) and no
    older waiting request is passed (order_allows), atomically with the
    check (under the ledger's lock). Returns the reservation's key (None:
    no room) and the numbers for the log. `since`: when this request began
    waiting; one that does not fit then stays in the ledger as a waiting
    entry (megabytes 0, `waiting_mb`, `waiting_since`; `pcb-maker ledger`
    lists it) that newer requests let go first. None: one try, now,
    nothing left behind. Held to the drain for a starved waiter
    (drained_for, drain_allows) as well; its entries carry what the drain
    reads: `expected_end` (now + `expected_seconds`, when known), the
    floor (`floor_mb`), `drain_after`, `drain_max` and when it was written
    (`refreshed`) while it waits, and `priority` (PRIORITY_NIGHTLY or
    None). `short` (since 2026-10-26; ledger.rs, `short_allows`): a request
    that holds its memory for seconds; of at most SHORT_OWN_MB +
    KICAD_CHECK_MB it goes whenever memory_room holds (the capacity, and MemAvailable
    less growth leaving its own `min_free_mb`), whatever the order and the
    drain say, and its reservation is marked `short`."""
    pid = os.getpid()

    def change(reservations):
        now = time.time()
        reservations[:] = [r for r in reservations if not _is_waiting_entry(r, pid, label, since)]
        available = memory_available_bytes()
        available_mb = None if available is None else available / (1024 * 1024)
        capacity = ledger_capacity_mb()
        fits, text = memory_room(reservations, megabytes, min_free_mb, capacity, available_mb)
        begun = now if since is None else since
        ordered = order_allows(reservations, megabytes, begun, capacity, pid, priority)
        expected_end = None if expected_seconds is None else now + float(expected_seconds)
        oldest = oldest_waiter(reservations)
        seen = None if available_mb is None or oldest is None else \
            available_mb - growth_mb(reservations, own_pid=pid)
        drained = drained_for(reservations, begun, now, pid, priority, capacity, seen)
        if drained is not None and ordered:
            ordered = drain_allows(drained, reservations, megabytes, capacity, seen, expected_end)
        # (A harness's request is one number: its process's and its
        # children's together.)
        is_short = bool(short) and megabytes <= SHORT_OWN_MB + KICAD_CHECK_MB
        ticks = start_ticks(pid)
        if fits and (ordered or is_short):
            entry = {"pid": pid, "started": now, "start_ticks": ticks, "megabytes": float(megabytes),
                     "label": label, "held_since": now}
            if ticks is None:
                del entry["start_ticks"]
            if is_short:
                entry["short"] = True
            if expected_end is not None:
                entry["expected_end"] = expected_end
            if priority:
                entry["priority"] = priority
            reservations.append(entry)
            return (entry["pid"], entry["started"], label), text
        if fits:
            state = drain_state(oldest, reservations, now, capacity, seen)
            text += (f"; behind an older request: {oldest.get('label', '?')} (waiting "
                     f"{max(0.0, now - _waiting_since(oldest)):.0f} s for {oldest['waiting_mb'] / 1024:.1f} GB"
                     + ("" if state[0] == "not yet" else f"; {describe_drain(state)}") + ")")
        if since is not None:
            entry = {"pid": pid, "started": since, "start_ticks": ticks, "megabytes": 0.0, "label": label,
                     "waiting_mb": float(megabytes), "waiting_since": since, "refreshed": now}
            if ticks is None:
                del entry["start_ticks"]
            if min_free_mb > 0:
                entry["floor_mb"] = float(min_free_mb)
            # (JSON has neither infinity nor NaN, and a binary that cannot
            # read the file sees an empty ledger: such a bound is left out.)
            if drain_after is not None and float(drain_after) < float("inf"):
                entry["drain_after"] = float(drain_after)
            if drain_max is not None and float(drain_max) < float("inf"):
                entry["drain_max"] = float(drain_max)
            if priority:
                entry["priority"] = priority
            # Only the floor holds it back (the ledger has its room): newer
            # requests need not wait for a floor they cannot see.
            if not fits and memory_room(reservations, megabytes, min_free_mb, capacity, None)[0]:
                entry["waits_on_floor"] = True
            reservations.append(entry)
        return None, text
    return ledger.update(change)


def withdraw(ledger, label, since):
    """Removes this process's waiting entry for `label` since `since`."""
    pid = os.getpid()
    ledger.update(lambda reservations: reservations.__setitem__(
        slice(None), [r for r in reservations if not _is_waiting_entry(r, pid, label, since)]))


def admit(ledger, megabytes, label, min_free_mb=0.0, poll_seconds=15.0, log=None, stop=None, expected_seconds=None,
          drain_after=None, priority=None, drain_max=None, short=False):
    """Waits until `megabytes` fit and reserves them (try_admit), in the
    ledger as a waiting request meanwhile (newer requests do not pass it:
    order_allows; once it has waited `drain_after` seconds, default
    DEFAULT_DRAIN_AFTER_SECONDS, the ledger drains for it: drain_allows,
    while it could start and for at most `drain_max` seconds of its wait,
    default DEFAULT_DRAIN_MAX_SECONDS: drain_state). `short`: a request
    that holds its memory for seconds (try_admit).
    Returns the reservation's key, None when `stop()` said to give up.
    `log` gets the waiting lines."""
    started = time.monotonic()
    since = time.time()
    said = None
    key = None
    try:
        while True:
            key, text = try_admit(ledger, megabytes, label, min_free_mb, since=since,
                                  expected_seconds=expected_seconds, drain_after=drain_after, priority=priority,
                                  drain_max=drain_max, short=short)
            waited = time.monotonic() - started
            if key is not None:
                if said is not None and log:
                    log(f"{label}: started after waiting {waited:.0f} s ({text})")
                return key
            if said is None and log:
                log(f"{label}: waiting for memory ({text})")
            said = text
            if stop and stop():
                return None
            time.sleep(poll_seconds)
    finally:
        if key is None:
            withdraw(ledger, label, since)
