# Copyright (C) 2026 Florian Loitsch.
"""Writes the test subjects' material and prompts for the why-open cases.

Usage: build_subjects.py CASES_DIR SUBJECTS_DIR ROUND
  ROUND r1: raw, dossier, plain, ascii, each case in three of the four
  ROUND r2: card (the annotated picture with the dossier), every case

CASES_DIR is build_cases.py's output. Per subject a directory with only its
material and prompt.md; SUBJECTS_DIR/<round>/manifest.json lists them. The
prompts are also copied to subjects/<round>/ beside this script.
"""
import json
import os
import shutil
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BINARIES = {
    "2026-11-18/sweep": "/home/flo/programming/pcb-maker/build/bin/pcb-maker-nightly-2026-11-18-3b27f2d0eb93",
    "2026-11-24/designer": "/home/flo/programming/pcb-maker/build/bin/pcb-maker-nightly-2026-11-24-96e891b27162",
}

HEAD = """You are a test subject in an experiment on how best to show PCB routing problems to an AI agent. Answer one question about one case, from the material named below and nothing else. Work alone: read nothing outside the material directory (in particular nothing else under /home/flo/programming), do not search the web, do not start other agents. There is no trick; if the material does not let you decide, say so in your notes and give your best answer.

## Background

pcb-maker is an automatic placer and router for KiCad boards. After it routed this board, KiCad's design rule check still reports one "unconnected item": two pieces of copper of one net that should be joined and are not. The router routes every net with the track width and clearance of its net class, may neck a track down to a narrower width for a short stretch at a pad, uses through vias, and treats copper pours (zones) as refilled around new tracks, so another net's pour is not an obstacle.

## The case

Board `{board}`, net `{net}`. KiCad reports a missing connection between

- end A: {a_desc} at ({ax:.3f}, {ay:.3f}) mm
- end B: {b_desc} at ({bx:.3f}, {by:.3f}) mm

(KiCad coordinates: x grows to the right, y grows downwards.)

## The question

Why is this connection still open? Pick the one best cause:

- **A, single-net miss.** A legal track joining the two ends (the class's width, at most a short neck, vias allowed) exists right now with every other net's copper left as it is. The router did not find or keep it. Fault: `tool`.
- **B, walled by other nets' routed copper.** No such track exists with the present copper, but one would if tracks or vias of some other nets were moved. Fault: `congestion`. Name those nets.
- **C, walled by the input.** An end cannot be reached under this net's rules even on a board stripped of every other net's tracks and vias: neighbouring pads, a footprint's own copper graphics or keepouts, holes, the board edge, or a net-class track too wide for the pad. Fault: `input`. Name what walls it.
- **D, pour island.** An end is a piece of the net's own copper pour that the fill left separated from the rest. Fault: `pour`.
- **E, not local.** The ends are far apart (more than 20 mm): a long connection that was never routed, not a local obstacle. Fault: `congestion`.

Also say what single change would most likely close it.
"""

TAIL = """
## Budget

At most {budget} tool calls in total (reading a file or a picture is one call, so is each command). Count them.

## Answer

End your reply with exactly one JSON object and nothing after it:

```json
{{"cause": "A|B|C|D|E", "in_the_way": ["nets, pads or objects"], "fault": "tool|input|congestion|pour", "fix": "one sentence", "confidence": 0, "tool_calls": 0, "notes": "what in the material decided it; what was missing, hard to read or misleading"}}
```

`confidence` is your probability in per cent that `cause` is right.
"""

MATERIAL = {
    "raw": """## Material: the run's own outputs

Directory `{dir}` holds what pcb-maker and KiCad wrote for this run, as an agent finds it today:

{listing}

You may read these files, run `{binary} inspect-routing-run {dir}` (it names the open nets' pads, rules and the hot spots, as JSON on stdout), and run your own Python (`python3`, KiCad 10's `pcbnew` module is importable; set `TMPDIR=/home/flo/programming/pcb-maker/build/scratch/tmp-fund-eval` first, /tmp is full). Write scratch files only into `{dir}/scratch/`. The board file is large: do not read it whole.
""",
    "dossier": """## Material: a dossier

`{dir}/dossier.json` is a diagnosis record a prototype tool wrote for this one item: it rasterised the copper in a window around the item and searched for a path for the item's net (first with every other net's copper kept, then with the other nets' tracks and vias removed), and, where it found a single-net path, wrote it as copper and asked KiCad's DRC. Its `cause` field is that tool's own conclusion in its own words; the tool is a prototype and can be wrong. Read this file only.
""",
    "plain": """## Material: a picture

{pictures} a rendering of the copper in a window around the item, one panel per copper layer (all seen from the top), with a legend. Read the picture(s) only. No analysis was run for it: it shows the copper as it is.
""",
    "ascii": """## Material: a text map

{maps} the copper in a window around the item drawn as characters, one block per copper layer (all seen from the top), with a legend and a pad list. Read the file(s) only. No analysis was run for it: it shows the copper as it is.
""",
    "card": """## Material: a picture and a dossier

{pictures} a rendering of the copper in a window around the item, one panel per copper layer (all seen from the top), with a legend. The darker band around other copper is where this net's track would be too close; the cyan line, if any, is the cheapest path a prototype tool's search found (it may cross other nets' copper: then it is not a legal path, it shows where the wall is thinnest).

`{dir}/dossier.json` is the same tool's diagnosis record for this item: it searched for a path for the item's net (first with every other net's copper kept, then with the other nets' tracks and vias removed), and, where it found a single-net path, wrote it as copper and asked KiCad's DRC. Its `cause` field is the tool's own conclusion; the tool is a prototype and can be wrong. Read only these files.
""",
}
BUDGET = {"raw": 20, "dossier": 3, "plain": 3, "ascii": 3, "card": 4}
R1 = ["raw", "dossier", "plain", "ascii"]


def copy_all(src, dst, exts):
    out = []
    for f in sorted(os.listdir(src)):
        if f.endswith(exts):
            shutil.copy(os.path.join(src, f), dst)
            out.append(f)
    return out


def build(case, rep, cases_dir, sdir):
    cdir = os.path.join(cases_dir, case["id"])
    src = json.load(open(os.path.join(cdir, "source.json")))
    doss = json.load(open(os.path.join(cdir, "dossier", f"item{case['item']}.json")))
    os.makedirs(sdir, exist_ok=True)
    ends = doss["ends"]
    head = HEAD.format(
        board=src["board"], net=doss["net"],
        a_desc=ends["A"]["what"], ax=ends["A"]["at_mm"][0], ay=ends["A"]["at_mm"][1],
        b_desc=ends["B"]["what"], bx=ends["B"]["at_mm"][0], by=ends["B"]["at_mm"][1],
    )
    if rep == "raw":
        rd = src["result_dir"]
        names = []
        for f in sorted(os.listdir(rd)):
            p = os.path.join(rd, f)
            if os.path.isfile(p):
                shutil.copy(p, sdir)
                names.append(f)
        for log in ("route.log", "layout.log"):
            for up in (os.path.dirname(rd), os.path.dirname(os.path.dirname(rd))):
                p = os.path.join(up, log)
                if os.path.exists(p) and log not in names:
                    shutil.copy(p, sdir)
                    names.append(log)
        os.makedirs(os.path.join(sdir, "scratch"), exist_ok=True)
        listing = "\n".join(f"- `{f}` ({os.path.getsize(os.path.join(sdir, f)) // 1024} kB)" for f in names)
        body = MATERIAL["raw"].format(dir=sdir, listing=listing, binary=BINARIES[case["tier"]])
    elif rep == "dossier":
        shutil.copy(os.path.join(cdir, "dossier", f"item{case['item']}.json"), os.path.join(sdir, "dossier.json"))
        body = MATERIAL["dossier"].format(dir=sdir)
    elif rep in ("plain", "card"):
        pics = copy_all(os.path.join(cdir, rep), sdir, ".png")
        text = ", ".join(f"`{os.path.join(sdir, p)}`" for p in pics) + (" are" if len(pics) > 1 else " is")
        if len(pics) > 1:
            text += " (one window per end, the ends being far apart)"
        if rep == "card":
            shutil.copy(os.path.join(cdir, "dossier", f"item{case['item']}.json"), os.path.join(sdir, "dossier.json"))
        body = MATERIAL[rep].format(dir=sdir, pictures=text)
    elif rep == "ascii":
        maps = copy_all(os.path.join(cdir, "ascii"), sdir, ".txt")
        text = ", ".join(f"`{os.path.join(sdir, p)}`" for p in maps) + (" are" if len(maps) > 1 else " is")
        if len(maps) > 1:
            text += " (one window per end, the ends being far apart)"
        body = MATERIAL["ascii"].format(maps=text)
    prompt = head + "\n" + body + TAIL.format(budget=BUDGET[rep])
    open(os.path.join(sdir, "prompt.md"), "w").write(prompt)
    return prompt


def main():
    cases_dir, subjects_dir, rnd = sys.argv[1:4]
    spec = json.load(open(os.path.join(HERE, "cases.json")))
    manifest = []
    pdir = os.path.join(HERE, "subjects", rnd)
    os.makedirs(pdir, exist_ok=True)
    for k, case in enumerate(spec["cases"]):
        reps = [r for i, r in enumerate(R1) if i != k % 4] if rnd == "r1" else ["card"]
        for rep in reps:
            sid = f"{case['id']}-{rep}"
            sdir = os.path.join(os.path.abspath(subjects_dir), rnd, sid)
            prompt = build(case, rep, cases_dir, sdir)
            open(os.path.join(pdir, sid + ".md"), "w").write(prompt)
            manifest.append({"subject": sid, "case": case["id"], "representation": rep, "prompt": os.path.join(sdir, "prompt.md")})
    json.dump(manifest, open(os.path.join(os.path.abspath(subjects_dir), rnd, "manifest.json"), "w"), indent=1)
    print(len(manifest), "subjects")


if __name__ == "__main__":
    main()
