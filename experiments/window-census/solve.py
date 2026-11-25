# Copyright (C) 2026 Florian Loitsch.
"""Solves one census window: SAT (with the copper), UNSAT (with a core
over the nets, by deletion) or timeout. Usage:

    solve.py census.json result.json [--limit 600] [--core-limit 120]
"""
import argparse, json, os, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import model as M


def run_cadical(cnf_path, limit):
    start = time.time()
    def cap():
        import resource
        resource.setrlimit(resource.RLIMIT_AS, (8 << 30, 8 << 30))
    r = subprocess.run(["cadical", "-q", "--phase=0", "-t", str(int(limit)), cnf_path],
                       capture_output=True, text=True, preexec_fn=cap)
    seconds = time.time() - start
    status = "timeout"
    values = set()
    for line in r.stdout.splitlines():
        if line.startswith("s SATISFIABLE"):
            status = "sat"
        elif line.startswith("s UNSATISFIABLE"):
            status = "unsat"
        elif line.startswith("v "):
            values.update(int(x) for x in line[2:].split() if int(x) > 0)
    return status, values, seconds


def solve(census, work, limit, core_limit, cores=True):
    nets = census["nets"]
    result = {"nets": [n["name"] for n in nets],
              "targets": [n["name"] for n in nets if n["target"]]}
    separated = [(n["name"], n["target"]) for n in nets if n["separated"]]
    result["separated"] = separated
    # Too big for this census (the worker's memory): a few hundred bytes a
    # clause in Python.
    stamps = sum(len(e[2]) + len(e[3]) for n in nets for e in n["elements"])
    nodes = sum(len(c[3]) for n in nets for c in n["connections"])
    result["estimated_clauses"] = stamps + 25 * nodes
    if result["estimated_clauses"] > 12_000_000:
        result["status"] = "too_big"
        return result
    t = time.time()
    cnf, info = M.build(census)
    result["build_seconds"] = round(time.time() - t, 1)
    result["variables"] = cnf.count
    result["clauses"] = len(cnf.clauses)
    result["conflict_clauses"] = info["conflict_clauses"]
    path = os.path.join(work, f"window-{os.getpid()}.cnf")
    sels = [info["nets"][i]["sel"] for i in range(len(nets))]
    cnf.write(path, units=sels)
    status, values, seconds = run_cadical(path, limit)
    result["status"] = status
    result["seconds"] = round(seconds, 1)
    result["hinted"] = len(cnf.hint)
    if status == "sat":
        result["copper"] = M.decode(census, info, cnf.true_vars(values))
    elif status == "unsat" and cores:
        # Deletion over the nets: targets last, so that the core keeps them
        # when it can.
        order = [i for i in range(len(nets)) if not nets[i]["target"]] + \
                [i for i in range(len(nets)) if nets[i]["target"]]
        keep = set(range(len(nets)))
        unknown = []
        for i in order:
            trial = keep - {i}
            if not trial:
                continue
            cnf.write(path, units=[sels[j] for j in sorted(trial)])
            s, _, sec = run_cadical(path, core_limit)
            if s == "unsat":
                keep = trial
            elif s == "timeout":
                unknown.append(nets[i]["name"])
        result["core"] = [nets[i]["name"] for i in sorted(keep)]
        result["core_unknown"] = unknown
        # Without the targets: do the other nets fit alone?
        others = [j for j in range(len(nets)) if not nets[j]["target"]]
        if others and len(others) < len(nets):
            cnf.write(path, units=[sels[j] for j in others])
            s, _, sec = run_cadical(path, core_limit)
            result["without_targets"] = s
    try:
        os.remove(path)
    except OSError:
        pass
    return result


def main():
    p = argparse.ArgumentParser()
    p.add_argument("census")
    p.add_argument("out")
    p.add_argument("--limit", type=float, default=600)
    p.add_argument("--core-limit", type=float, default=120)
    a = p.parse_args()
    census = json.load(open(a.census))
    work = os.path.dirname(os.path.abspath(a.out))
    result = solve(census, work, a.limit, a.core_limit)
    json.dump(result, open(a.out, "w"))
    print(json.dumps({k: v for k, v in result.items() if k != "copper"}))


if __name__ == "__main__":
    main()
