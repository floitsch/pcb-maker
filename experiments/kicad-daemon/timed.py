# Copyright (C) 2026 Toit contributors.
"""Runs a command under a pty, timestamps each output line, reports wall,
user, sys and peak RSS (wait4)."""
import os, pty, sys, time, json

def run(argv, cwd=None):
    start = time.monotonic()
    pid, fd = pty.fork()
    if pid == 0:
        if cwd:
            os.chdir(cwd)
        os.execvp(argv[0], argv)
    lines, buf = [], b""
    while True:
        try:
            chunk = os.read(fd, 4096)
        except OSError:
            break
        if not chunk:
            break
        now = time.monotonic() - start
        buf += chunk
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            lines.append((round(now, 3), line.decode(errors="replace").rstrip("\r")))
    _, status, usage = os.wait4(pid, 0)
    wall = time.monotonic() - start
    return {"wall": round(wall, 3), "user": usage.ru_utime, "sys": usage.ru_stime,
            "rss_mb": round(usage.ru_maxrss / 1024), "status": status, "lines": lines}

if __name__ == "__main__":
    r = run(sys.argv[1:])
    for t, l in r["lines"]:
        print(f"{t:8.3f}  {l}")
    print(json.dumps({k: v for k, v in r.items() if k != "lines"}))
