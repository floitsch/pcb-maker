# Copyright (C) 2026 Florian Loitsch.
"""The window census of the nightly's near-miss boards
(docs/experiments/window-census.md).

Per KiCad unconnected item and window half-size: census-kicad-window cuts
the copper of the window's nets back to the window's edge and dumps the
router's lattice; solve.py decides the joint reroute (SAT, UNSAT with a
core over the nets, timeout); a SAT answer's copper is written into the
cut board and checked with drc-kicad-board.

    run.py items.json outdir [--jobs 6] [--limit 600] [--half 2 4]
"""
import argparse, concurrent.futures as cf, json, math, os, re, shutil, subprocess, sys, time, uuid

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import solve as S

BIN = os.environ.get("PCB_MAKER", "/home/flo/programming/pcb-maker/build/scratch/fund-window/bin/pcb-maker-fw4")
MAX_WINDOW = 20.0  # mm across: longer items are not local


def env():
    e = dict(os.environ)
    e["RAYON_NUM_THREADS"] = "1"
    e.setdefault("TMPDIR", "/home/flo/programming/pcb-maker/build/scratch/tmp-fund-window")
    return e


def wait_for_room():
    while True:
        avail = 0
        for line in open("/proc/meminfo"):
            if line.startswith("MemAvailable:"):
                avail = int(line.split()[1]) / 1e6
        hour = time.localtime().tm_hour
        if avail >= 16 and not (2 <= hour < 7):
            return
        time.sleep(60)


def net_of(description):
    m = re.search(r"\[(.*)\]", description)
    return m.group(1) if m else None


def board_settings(result_dir):
    d = json.load(open(os.path.join(result_dir, "board-router.json")))
    pitch, lines, name = d.get("grid_pitch_mm"), None, None
    for a in d.get("attempts") or []:
        if a.get("selected"):
            pitch = a.get("grid_pitch_mm", pitch)
            lines = a.get("pin_lines")
            name = a.get("name")
    if lines is None and isinstance(d.get("pin_lines"), str) and d["pin_lines"].startswith("on"):
        lines = True
    return pitch, lines, name


PAIR = re.compile(r"^(.*?)(_?)(\+|-|P|N)$")


def pair_nets(board_text):
    names = set(re.findall(r'\(net (?:\d+ )?"((?:[^"\\]|\\.)*)"\)', board_text))
    out = set()
    for n in names:
        m = PAIR.match(n)
        if not m:
            continue
        other = {"+": "-", "-": "+", "P": "N", "N": "P"}[m.group(3)]
        if m.group(1) + m.group(2) + other in names and m.group(1):
            out.add(n)
    return out


def kicad_escape(text):
    return text.replace("\\", "\\\\").replace('"', '\\"')


def write_solution(census, copper, out_path):
    """The cut board with the solution's copper."""
    text = open(census["clipped_board"]).read().rstrip()
    assert text.endswith(")")
    layers = census["layers"]
    items = []

    def segment(a, b, width, layer, net):
        if abs(a[0] - b[0]) < 1e-6 and abs(a[1] - b[1]) < 1e-6:
            return
        items.append(f'  (segment (start {a[0]:.6f} {a[1]:.6f}) (end {b[0]:.6f} {b[1]:.6f}) '
                     f'(width {width:.6f}) (layer "{layers[layer]}") (net "{kicad_escape(net)}") '
                     f'(uuid "{uuid.uuid4()}"))')

    for net, sol in zip(census["nets"], copper):
        nodes, widths = net["nodes"], net["widths"]
        for (u, v) in sol["segments"]:
            segment(nodes[u][1:], nodes[v][1:], min(widths[u], widths[v]), nodes[u][0], net["raw"])
        for vi in sol["vias"]:
            x, y, _ = net["vias"][vi]
            items.append(f'  (via (at {x:.6f} {y:.6f}) (size {net["via"][0]:.6f}) (drill {net["via"][1]:.6f}) '
                         f'(layers "{layers[0]}" "{layers[-1]}") (net "{kicad_escape(net["raw"])}") '
                         f'(uuid "{uuid.uuid4()}"))')
        done = set()
        for (_, node, esc) in sol["ways"]:
            if esc < 0 or esc in done:
                continue
            done.add(esc)
            _, enode, ewidth, poly = net["escapes"][esc]
            for a, b in zip(poly, poly[1:]):
                segment(a, b, ewidth, nodes[enode][0], net["raw"])
    with open(out_path, "w") as f:
        f.write(text[:-1].rstrip() + "\n" + "\n".join(items) + "\n)\n")
    return len(items)


def drc(project_dir, board_id, report):
    r = subprocess.run([BIN, "drc-kicad-board", project_dir, board_id, report],
                       capture_output=True, text=True, env=env(), timeout=1800)
    if not os.path.exists(report):
        return None, r.stdout[-400:] + r.stderr[-400:]
    return summarize(json.load(open(report))), None


def summarize(d):
    viol = {}
    for v in d.get("violations", []):
        if v.get("severity") == "error":
            viol[v["type"]] = viol.get(v["type"], 0) + 1
    unconnected = [sorted(net_of(i["description"]) or "" for i in u["items"]) for u in d.get("unconnected_items", [])]
    return {"unconnected": len(d.get("unconnected_items", [])), "unconnected_nets": unconnected, "violations": viol}


def project_copy(result_dir, dest):
    os.makedirs(dest, exist_ok=True)
    for f in os.listdir(result_dir):
        if f.endswith((".kicad_pro", ".kicad_dru")):
            shutil.copy(os.path.join(result_dir, f), dest)


def census_run(board, cfg, work, tag):
    cfg_path = os.path.join(work, f"{tag}.cfg.json")
    json.dump(cfg, open(cfg_path, "w"))
    clip_dir = os.path.join(work, f"{tag}-clip")
    project_copy(os.path.dirname(board), clip_dir)
    clipped = os.path.join(clip_dir, os.path.basename(board))
    out = os.path.join(work, f"{tag}.census.json")
    t = time.time()
    r = subprocess.run([BIN, "census-kicad-window", board, cfg_path, clipped, out],
                       capture_output=True, text=True, env=env(), timeout=3600)
    if r.returncode != 0 or not os.path.exists(out):
        return None, (r.stderr or r.stdout)[-600:], time.time() - t
    return json.load(open(out)), None, time.time() - t


def run_job(job):
    """One item at one half-size on `job['board']`."""
    wait_for_room()
    work = job["work"]
    os.makedirs(work, exist_ok=True)
    res = {k: job[k] for k in ("tier", "name", "item", "half", "net", "length")}
    started = time.time()
    board = job["board"]
    ends = job["ends"]
    h = job["half"]
    window = [min(e[0] for e in ends) - h, min(e[1] for e in ends) - h,
              max(e[0] for e in ends) + h, max(e[1] for e in ends) + h]
    res["window"] = window
    if max(window[2] - window[0], window[3] - window[1]) > MAX_WINDOW:
        res["status"] = "long"
        return res
    text = open(board).read()
    pairs = sorted(n for n in pair_nets(text) if n != job["net"])
    cfg = {"window": window, "items": [{"net": job["net"], "ends": ends}],
           "pitch": job["pitch"], "pin_lines": job["pin_lines"],
           "slack_steps": job["slack"], "slack_ratio": 0.5,
           "other_slack_steps": job["other_slack"], "other_slack_ratio": 0.25,
           "fixed_nets": pairs}
    unreproducible = []
    census = None
    for attempt in range(6):
        census, err, sec = census_run(board, cfg, work, "census")
        res["census_seconds"] = round(sec, 1)
        if census is None:
            res["status"] = "census_error"
            res["error"] = err
            return res
        bad = [n["raw"] for n in census["nets"] if n["separated"] and not n["target"]]
        if not bad:
            break
        unreproducible += bad
        cfg["fixed_nets"] = cfg["fixed_nets"] + bad
    res["fixed_in_window"] = census.get("fixed_in_window", [])
    res["unreproducible"] = unreproducible
    res["nets"] = [n["raw"] for n in census["nets"]]
    res["window_nets"] = len(census["nets"])
    res["layers"] = len(census["layers"])
    res["warped"] = census["warped"]
    res["removed"] = census["removed"]
    targets = [n for n in census["nets"] if n["target"]]
    if not targets:
        res["status"] = "no_target"
        return res
    res["target_sides"] = targets[0]["target_sides"]
    if any(n["separated"] for n in targets):
        res["status"] = "target_walled"
        # Walled by copper at all, or by pads, graphics and rule areas?
        cfg3 = dict(cfg)
        cfg3["strip_all"] = True
        cfg3["fixed_nets"] = []
        c3, err3, _ = census_run(board, cfg3, work, "census-bare")
        if c3 is not None:
            res["walled_without_copper"] = any(n["separated"] for n in c3["nets"] if n["target"])
        # Is it the pairs' copper (kept as routed) that walls it?
        if pairs:
            cfg2 = dict(cfg)
            cfg2["fixed_nets"] = unreproducible
            c2, err2, _ = census_run(board, cfg2, work, "census-pairs-free")
            if c2 is not None:
                res["walled_with_pairs_free"] = any(n["separated"] for n in c2["nets"] if n["target"])
        return res
    # Stages: the target alone (every other net's copper kept), then the
    # nearest k nets freed too, then every net. A SAT answer at any stage
    # is a solution in the window; UNSAT is a certificate only with every
    # net free (and then without the detour budget).
    others = [n for n in census["nets"] if not n["target"]]
    a, b = ends[0], ends[-1]

    def distance(net):
        best = float("inf")
        for u in net["hint_nodes"]:
            p = net["nodes"][u][1:]
            dx, dy = b[0] - a[0], b[1] - a[1]
            t = 0.0 if dx == dy == 0 else max(0.0, min(1.0, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy)))
            best = min(best, math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy))
        return best

    ranked = sorted(others, key=distance)
    stages = sorted({k for k in (0, 2, 5) if k < len(ranked)})
    res["stages"] = []
    sol = None
    for k in stages:
        cfg_k = dict(cfg)
        cfg_k["fixed_nets"] = cfg["fixed_nets"] + [n["raw"] for n in ranked[k:]]
        ck, errk, _ = census_run(board, cfg_k, work, f"census-k{k}")
        if ck is None or any(n["separated"] for n in ck["nets"]):
            res["stages"].append({"free": k, "status": "separated" if ck else "census_error"})
            continue
        sk = S.solve(ck, work, job["stage_limit"], 0, cores=False)
        res["stages"].append({"free": k, "status": sk["status"], "seconds": sk["seconds"],
                              "variables": sk["variables"], "nets": [n["raw"] for n in ranked[:k]]})
        if sk["status"] == "sat":
            sol, census = sk, ck
            res["status"] = "sat"
            res["moved"] = k
            res["moved_nets"] = [n["raw"] for n in ranked[:k]]
            break
    if sol is None:
        sol = S.solve(census, work, job["limit"], job["core_limit"], cores=not job.get("no_core"))
        for key in ("status", "seconds", "build_seconds", "variables", "clauses", "core", "core_unknown",
                    "without_targets"):
            if key in sol:
                res[key] = sol[key]
        if sol["status"] == "sat":
            res["moved"] = len(ranked)
            res["moved_nets"] = [n["raw"] for n in ranked]
    if sol["status"] == "unsat" and not job.get("no_core"):
        # The budget's UNSAT is no certificate: the whole region, no budget.
        cfg4 = dict(cfg)
        cfg4.update({"slack_steps": 1e6, "other_slack_steps": 1e6})
        c4, err4, _ = census_run(board, cfg4, work, "census-full")
        if c4 is not None and not any(n["separated"] for n in c4["nets"]):
            full = S.solve(c4, work, job["limit"], job["core_limit"])
            res["full"] = {k: full[k] for k in ("status", "seconds", "variables", "clauses", "core",
                                                "core_unknown", "without_targets") if k in full}
            if full["status"] == "sat":
                sol, census = full, c4
                res["moved"] = len(ranked)
        elif c4 is not None:
            res["full"] = {"status": "separated"}
    if sol["status"] == "sat":
        sol_dir = os.path.join(work, "solution")
        project_copy(os.path.dirname(board), sol_dir)
        out_board = os.path.join(sol_dir, os.path.basename(board))
        res["written_items"] = write_solution(census, sol["copper"], out_board)
        board_id = os.path.basename(board)[:-len(".kicad_pcb")]
        summary, err = drc(sol_dir, board_id, os.path.join(work, "solution-drc.json"))
        res["drc"] = summary
        if err:
            res["drc_error"] = err
    res["total_seconds"] = round(time.time() - started, 1)
    for f in os.listdir(work):
        if f.endswith(".census.json"):
            os.remove(os.path.join(work, f))
    return res


def baseline(board, work):
    """KiCad's verdict of the routed board as drc-kicad-board gives it."""
    os.makedirs(work, exist_ok=True)
    path = os.path.join(work, "baseline-drc.json")
    if os.path.exists(path):
        return summarize(json.load(open(path)))
    d = os.path.join(work, "baseline")
    project_copy(os.path.dirname(board), d)
    shutil.copy(board, d)
    board_id = os.path.basename(board)[:-len(".kicad_pcb")]
    summary, err = drc(d, board_id, path)
    return summary


def main():
    p = argparse.ArgumentParser()
    p.add_argument("items")
    p.add_argument("outdir")
    p.add_argument("--jobs", type=int, default=6)
    p.add_argument("--limit", type=float, default=600)
    p.add_argument("--core-limit", type=float, default=120)
    p.add_argument("--stage-limit", type=float, default=120)
    p.add_argument("--half", type=float, nargs="+", default=[2.0, 4.0])
    p.add_argument("--slack", type=float, default=20)
    p.add_argument("--other-slack", type=float, default=10)
    p.add_argument("--only", default=None, help="regex on board names")
    a = p.parse_args()
    boards = json.load(open(a.items))
    jobs = []
    for b in boards:
        if a.only and not re.search(a.only, b["name"]):
            continue
        rdir = os.path.dirname(b["board"])
        pitch, lines, attempt = board_settings(rdir)
        for i, ends in enumerate(b["items"]):
            nets = {net_of(e[2]) for e in ends}
            net = nets.pop() if len(nets) == 1 else None
            pts = [[e[0], e[1]] for e in ends]
            length = math.hypot(pts[0][0] - pts[-1][0], pts[0][1] - pts[-1][1])
            for h in a.half:
                tier = b["tier"].replace("/", "-")
                jobs.append({"tier": b["tier"], "name": b["name"], "board": b["board"], "item": i,
                             "half": h, "net": net, "ends": pts, "length": round(length, 2),
                             "descriptions": [e[2] for e in ends], "pitch": pitch, "pin_lines": lines,
                             "attempt": attempt, "limit": a.limit, "core_limit": a.core_limit,
                             "slack": a.slack, "other_slack": a.other_slack, "stage_limit": a.stage_limit,
                             "work": os.path.join(a.outdir, tier, b["name"], f"item{i}-h{h:g}")})
    os.makedirs(a.outdir, exist_ok=True)
    done_path = os.path.join(a.outdir, "results.jsonl")
    done = set()
    if os.path.exists(done_path):
        for line in open(done_path):
            r = json.loads(line)
            done.add((r["tier"], r["name"], r["item"], r["half"]))
    todo = []
    for j in jobs:
        if (j["tier"], j["name"], j["item"], j["half"]) in done:
            continue
        if any("Zone [" in d for d in j["descriptions"]):
            r = {k: j[k] for k in ("tier", "name", "item", "half", "net", "length")}
            r["status"] = "zone_item"
            r["descriptions"] = j["descriptions"]
            open(done_path, "a").write(json.dumps(r) + "\n")
            continue
        if j["net"] is None:
            r = {k: j[k] for k in ("tier", "name", "item", "half", "net", "length")}
            r["status"] = "mixed_nets"
            open(done_path, "a").write(json.dumps(r) + "\n")
            continue
        todo.append(j)
    print(len(jobs), "jobs,", len(todo), "to do", flush=True)
    with cf.ProcessPoolExecutor(max_workers=a.jobs) as pool:
        futures = {pool.submit(run_job, j): j for j in todo}
        for fut in cf.as_completed(futures):
            j = futures[fut]
            try:
                r = fut.result()
            except Exception as e:  # keep going; record the failure
                r = {k: j[k] for k in ("tier", "name", "item", "half", "net", "length")}
                r["status"] = "error"
                r["error"] = repr(e)[-600:]
            r["descriptions"] = j["descriptions"]
            r["attempt"] = j["attempt"]
            open(done_path, "a").write(json.dumps(r) + "\n")
            print(r["name"][:40], r["item"], r["half"], r["status"], r.get("seconds"), r.get("core"),
                  r.get("drc", {}).get("unconnected") if r.get("drc") else "", flush=True)


if __name__ == "__main__":
    main()
