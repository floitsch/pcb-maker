# Copyright (C) 2026 Toit contributors.
"""Read-only sampler of the machine's memory beside the ledger (ledger6's
measurement): every `interval` seconds one JSON line with MemTotal,
MemAvailable, swap, the ledger's entries with each holder's RSS (its own and
its process tree's), and the memory outside the ledger. Takes no lock: it
reads the ledger file as it is (writers rename atomically)."""

import json
import os
import sys
import time


def meminfo():
    values = {}
    with open("/proc/meminfo") as f:
        for line in f:
            name, rest = line.split(":", 1)
            values[name] = int(rest.split()[0]) / 1024
    return values


def processes():
    """pid -> (ppid, rss_mb, comm)."""
    table = {}
    for name in os.listdir("/proc"):
        if not name.isdigit():
            continue
        try:
            with open(f"/proc/{name}/stat") as f:
                text = f.read()
            comm = text[text.index("(") + 1:text.rindex(")")]
            fields = text.rsplit(")", 1)[1].split()
            ppid = int(fields[1])
            # resident pages (stat field 24 = rss) times page size
            rss = int(fields[21]) * 4096 / (1024 * 1024)
            table[int(name)] = (ppid, rss, comm)
        except (OSError, ValueError, IndexError):
            pass
    return table


def main():
    path = sys.argv[1]
    interval = float(sys.argv[2]) if len(sys.argv) > 2 else 10.0
    until = time.time() + float(sys.argv[3]) if len(sys.argv) > 3 else float("inf")
    ledger = os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/tmp"), "pcb-maker-ledger.json")
    with open(path, "a") as out:
        while time.time() < until:
            started = time.monotonic()
            now = time.time()
            m = meminfo()
            table = processes()
            children = {}
            for pid, (ppid, _, _) in table.items():
                children.setdefault(ppid, []).append(pid)
            try:
                with open(ledger) as f:
                    entries = json.load(f).get("reservations", [])
            except (OSError, ValueError):
                entries = []
            counted = set()
            rows = []
            # holders first, by held_since, so that a tree is counted once
            for r in sorted(entries, key=lambda r: -(r.get("megabytes") or 0)):
                pid = r.get("pid")
                mb = r.get("megabytes") or 0.0
                own = table.get(pid, (0, 0.0, ""))[1]
                tree = 0.0
                if mb > 0 and pid in table:
                    stack = [pid]
                    while stack:
                        p = stack.pop()
                        if p in counted:
                            continue
                        counted.add(p)
                        tree += table.get(p, (0, 0.0, ""))[1]
                        stack.extend(children.get(p, []))
                rows.append({k: r.get(k) for k in ("pid", "megabytes", "waiting_mb", "floor_mb", "held_since",
                                                    "waiting_since", "short", "priority", "expected_end",
                                                    "waits_on_floor", "label") if r.get(k) is not None}
                            | {"rss": round(own, 1), "tree": round(tree, 1)})
            ledger_rss = sum(row["tree"] for row in rows)
            used = m["MemTotal"] - m["MemAvailable"]
            # the largest outside process groups by command, for the record
            by_comm = {}
            for pid, (_, rss, comm) in table.items():
                if pid not in counted:
                    by_comm[comm] = by_comm.get(comm, 0.0) + rss
            top = sorted(by_comm.items(), key=lambda c: -c[1])[:6]
            sample = {"t": round(now, 1), "total": round(m["MemTotal"]), "available": round(m["MemAvailable"]),
                      "free": round(m["MemFree"]), "swap_used": round(m["SwapTotal"] - m["SwapFree"]),
                      "reserved": round(sum(r.get("megabytes") or 0 for r in entries)),
                      "ledger_rss": round(ledger_rss), "outside": round(used - ledger_rss),
                      "top": [[c, round(v)] for c, v in top], "entries": rows}
            out.write(json.dumps(sample) + "\n")
            out.flush()
            time.sleep(max(0.0, interval - (time.monotonic() - started)))


if __name__ == "__main__":
    main()
