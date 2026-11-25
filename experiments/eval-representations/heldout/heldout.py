# Copyright (C) 2026 Florian Loitsch.
"""Thin path wrapper around the study's unchanged scripts for the held-out cases.

heldout.py gen        runs whyopen.py per case exactly as build_cases.py does for the
                      dossier (--verify) and card formats; stdout/stderr to files, nothing printed
                      but the exit codes.
heldout.py subjects   builds the card and raw prompts with build_subjects.build (unchanged).
"""
import json, os, subprocess, sys, time
EXP = "/home/flo/programming/pcb-maker-cards-heldout/experiments/eval-representations"
sys.path.insert(0, EXP)
import build_subjects as bs  # noqa: E402

N = "/home/flo/programming/pcb-maker/build/nightly/"
HERE = os.path.dirname(os.path.abspath(__file__))
CASES = os.path.join(HERE, "cases")
SUBJ = "/home/flo/programming/pcb-maker/build/scratch/ch-subjects"
picks = json.load(open(os.path.join(HERE, "picks.json")))


def result_dir(p):
    for sub in ("layout/result", "route"):
        d = os.path.join(N, p["tier"], p["name"], sub)
        if os.path.exists(os.path.join(d, "drc.json")):
            return d


def gen():
    for i, p in enumerate(picks):
        d = result_dir(p)
        board = [f for f in os.listdir(d) if f.endswith(".kicad_pcb")][0]
        base = [sys.executable, os.path.join(EXP, "whyopen.py"), os.path.join(d, board), "--drc", os.path.join(d, "drc.json"),
                "--router", os.path.join(d, "board-router.json"), "--item", str(p["item"])]
        cdir = os.path.join(CASES, f"h{i}")
        os.makedirs(cdir, exist_ok=True)
        json.dump({"result_dir": d, "board": board}, open(os.path.join(cdir, "source.json"), "w"))
        for name, extra in (("dossier", ["--formats", "dossier", "--verify"]), ("card", ["--formats", "png"])):
            t = time.time()
            r = subprocess.run(base + extra + ["--out", os.path.join(cdir, name)], capture_output=True, text=True)
            open(os.path.join(cdir, name + ".stdout"), "w").write(r.stdout)
            open(os.path.join(cdir, name + ".stderr"), "w").write(r.stderr)
            print(f"h{i} {name} exit {r.returncode} {time.time() - t:.0f} s", flush=True)


def ends_from_drc(p):
    it = json.load(open(os.path.join(result_dir(p), "drc.json")))["unconnected_items"][p["item"]]["items"]
    return {k: {"what": x["description"], "at_mm": [x["pos"]["x"], x["pos"]["y"]]} for k, x in zip("AB", it)}


def subjects():
    bs.BINARIES["2026-11-24/sweep"] = bs.BINARIES["2026-11-24/designer"]
    # paths only: the raw arm's scratch TMPDIR (/tmp is usable again; keep the line, point it at ours)
    bs.MATERIAL["raw"] = bs.MATERIAL["raw"].replace("tmp-fund-eval", "tmp-cards-heldout")
    manifest = []
    for i, p in enumerate(picks):
        case = {"id": f"h{i}", "tier": p["tier"], "item": p["item"]}
        cdir = os.path.join(CASES, case["id"])
        dos = os.path.join(cdir, "dossier", f"item{p['item']}.json")
        for rep in ("card", "raw"):
            sdir = os.path.join(SUBJ, f"h{i}-{rep}")
            if not os.path.exists(dos):
                if rep == "card":
                    print(f"h{i}: no dossier, no card subject")
                    continue
                # the raw arm needs only the ends: take them from KiCad's report
                os.makedirs(os.path.join(cdir, "dossier"), exist_ok=True)
                net = it_net = json.load(open(os.path.join(result_dir(p), "drc.json")))["unconnected_items"][p["item"]]["items"][0]["description"].split("[")[1].split("]")[0]
                json.dump({"net": net, "ends": ends_from_drc(p)}, open(dos + ".raw-only", "w"))
                os.replace(dos + ".raw-only", dos)
            if rep == "card" and not [f for f in os.listdir(os.path.join(cdir, "card")) if f.endswith(".png")]:
                print(f"h{i}: no picture, no card subject")
                continue
            bs.build(case, rep, CASES, sdir)
            manifest.append({"subject": f"h{i}-{rep}", "prompt": os.path.join(sdir, "prompt.md")})
    json.dump(manifest, open(os.path.join(SUBJ, "manifest.json"), "w"), indent=1)
    print(len(manifest), "subjects")


if __name__ == "__main__":
    {"gen": gen, "subjects": subjects}[sys.argv[1]]()
