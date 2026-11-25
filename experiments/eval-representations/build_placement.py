# Copyright (C) 2026 Florian Loitsch.
"""Writes the test subjects' material and prompts for the placement cases.

Usage: build_placement.py SUBJECTS_DIR ROUND   (ROUND p1: raw, metrics, picture; p2: both)
"""
import glob
import json
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

HEAD = """You are a test subject in an experiment on how best to show PCB placements to an AI agent. Answer one question about one case, from the material named below and nothing else. Work alone: read nothing outside the material directory (in particular nothing else under /home/flo/programming), do not search the web, do not start other agents. There is no trick; if the material does not let you decide, say so in your notes and give your best answer.

## The case

Two placements, P and Q, of the same board (the same parts, nets, outline, layers and rules; only the parts' positions, sides and rotations differ). Neither is routed. The same automatic router (pcb-maker, a negotiated-congestion router on a lattice, through vias, with the same work budget) then routed both: on one of them it left several times as many connections open as on the other.

## The question

Which placement routed better (fewer open connections), P or Q? And what about the other one makes it harder to route: name the parts or regions you would change first.
"""

TAIL = """
## Budget

At most {budget} tool calls in total (reading a file or a picture is one call, so is each command). Count them.

## Answer

End your reply with exactly one JSON object and nothing after it:

```json
{{"better": "P|Q", "confidence": 50, "worse_because": ["parts or regions of the worse placement, most important first"], "why": "one or two sentences", "tool_calls": 0, "notes": "what in the material decided it; what was missing, hard to read or misleading"}}
```

`confidence` is your probability in per cent that `better` is right (50 means a coin flip).
"""

MATERIAL = {
    "raw": """## Material: the two boards

`{dir}/P/` and `{dir}/Q/` each hold one placement as a KiCad 10 project (`.kicad_pcb`, `.kicad_pro`). You may read the files and run your own Python (`python3`, KiCad 10's `pcbnew` module is importable; set `TMPDIR=/home/flo/programming/pcb-maker/build/scratch/tmp-fund-eval` first, /tmp is full). Write scratch files only into `{dir}/scratch/`. The board files are large: do not read them whole.
""",
    "metrics": """## Material: measures

`{dir}/P.json` and `{dir}/Q.json` hold, per placement, measures a prototype tool computed without routing: the ratsnest (a minimum spanning tree per net over its pads; nets with a pour left out) with its length and crossings, a coarse map of the ratsnest's demand against the room for tracks with its fullest cells and the parts there, and per part of ten pads or more how much of the ring around it other parts' pads close. Read these two files only.
""",
    "picture": """## Material: pictures

`{dir}/P.png` and `{dir}/Q.png` show each placement from the top: the parts' pad boxes (orange: front, blue: back) with their references, the pads, the ratsnest (thin white lines: a minimum spanning tree per net; nets with a pour left out), and in red the cells where the ratsnest's demand passes half the room for tracks. Read these two pictures only.
""",
    "both": """## Material: pictures and measures

`{dir}/P.png` and `{dir}/Q.png` show each placement from the top: the parts' pad boxes (orange: front, blue: back) with their references, the pads, the ratsnest (thin white lines: a minimum spanning tree per net; nets with a pour left out), and in red the cells where the ratsnest's demand passes half the room for tracks.

`{dir}/P.json` and `{dir}/Q.json` hold, per placement, measures a prototype tool computed without routing: the ratsnest's length and crossings, the map of demand against room with its fullest cells and the parts there, and per part of ten pads or more how much of the ring around it other parts' pads close. Read only these four files.
""",
}
BUDGET = {"raw": 20, "metrics": 3, "picture": 3, "both": 5}


def main():
    subjects_dir, rnd = os.path.abspath(sys.argv[1]), sys.argv[2]
    spec = json.load(open(os.path.join(HERE, "placement_cases.json")))
    reps = ["raw", "metrics", "picture"] if rnd == "p1" else ["both"]
    pdir = os.path.join(HERE, "subjects", rnd)
    os.makedirs(pdir, exist_ok=True)
    manifest = []
    for case in spec["cases"]:
        src = {
            case["designer"]: os.path.join(spec["nightly"], "designer", case["name"], "placed"),
            "Q" if case["designer"] == "P" else "P": os.path.join(spec["nightly"], "sweep", case["name"], "layout", "placed"),
        }
        work = os.path.join(subjects_dir, "placement-work", case["id"])
        os.makedirs(work, exist_ok=True)
        for letter, d in src.items():
            board = glob.glob(os.path.join(d, "*.kicad_pcb"))[0]
            if not os.path.exists(os.path.join(work, letter + ".json")):
                subprocess.run([sys.executable, os.path.join(HERE, "placeview.py"), board, "--out", os.path.join(work, letter), "--label", letter], check=True, capture_output=True)
        for rep in reps:
            sid = f"{case['id']}-{rep}"
            sdir = os.path.join(subjects_dir, rnd, sid)
            os.makedirs(sdir, exist_ok=True)
            if rep == "raw":
                for letter, d in src.items():
                    os.makedirs(os.path.join(sdir, letter), exist_ok=True)
                    for f in glob.glob(os.path.join(d, "*.kicad_pcb")) + glob.glob(os.path.join(d, "*.kicad_pro")):
                        shutil.copy(f, os.path.join(sdir, letter, "board" + os.path.splitext(f)[1]))
                os.makedirs(os.path.join(sdir, "scratch"), exist_ok=True)
            for ext, want in ((".json", ("metrics", "both")), (".png", ("picture", "both"))):
                if rep in want:
                    for letter in "PQ":
                        shutil.copy(os.path.join(work, letter + ext), sdir)
            prompt = HEAD + "\n" + MATERIAL[rep].format(dir=sdir) + TAIL.format(budget=BUDGET[rep])
            open(os.path.join(sdir, "prompt.md"), "w").write(prompt)
            open(os.path.join(pdir, sid + ".md"), "w").write(prompt)
            manifest.append({"subject": sid, "case": case["id"], "representation": rep, "prompt": os.path.join(sdir, "prompt.md")})
    json.dump(manifest, open(os.path.join(subjects_dir, rnd, "manifest.json"), "w"), indent=1)
    print(len(manifest), "subjects")


if __name__ == "__main__":
    main()
