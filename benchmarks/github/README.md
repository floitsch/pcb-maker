# Boards harvested from GitHub

The other benchmark sets are old or small: PCBench is KiCad-5 era and has no
schematics, the corpus is 23 boards. This set is modern (KiCad 7+) open-source
boards found on GitHub, chosen for what makes routing hard: inner layers,
BGAs, QFNs and LGAs, fine pads, many footprints.

`harvest.py` does it in three steps (`gh auth login` first):

```sh
benchmarks/github/harvest.py search          # code and topic searches -> candidates.json
benchmarks/github/harvest.py fetch --limit 120   # download, judge, manifest.json
benchmarks/github/harvest.py boards          # boards.json for the runners
```

- **search** runs GitHub code searches for `.kicad_pcb` files by generator
  version and a package kind (`Package_BGA`, `In2.Cu`, ...), plus the trees of
  repositories under the `kicad`/`pcb-design`/... topics for boards over
  300 KB (code search does not index files over 384 KB). Forks are skipped.
- **fetch** takes the board and its `.kicad_pro` at the repository's head
  commit into `benchmarks/real/external/github/<owner>__<repo>__<stem>/`
  (gitignored; `origin.json` names the source). A board counts when it is a
  finished design: KiCad 6+ format, an outline, 2-8 copper layers, 25-900
  footprints, tracks on it. Duplicates (same file in another repository) are
  dropped. `manifest.json` (committed) records origin, commit, licence, stars
  and the designer's numbers per board, and why each rejected candidate was
  rejected.
- **boards** writes `boards.json` in the corpus runner's format, hardest
  first.

The files are used for benchmarking only and are not redistributed; the
manifest records each repository's licence.

## Schematics (`harvest_schematics.py`)

The boards above come without their schematics. `harvest_schematics.py`
fetches, at each board's commit, what an agent starting from the schematic
has: the root sheet `<board_id>.kicad_sch` and every sheet it names, the
project file and custom rules, the library tables, the symbol libraries the
tables keep in the repository and, of the footprint libraries in the
repository (Git submodules included), the footprints the symbols name.
Libraries a table names by a designer's absolute path, or that lie outside
the project folder, are looked for in the repository by folder and file
name and recorded as `library_fixes` (the schematic tier adds them to the
project's `fp-lib-table` when the import reports them missing, as an agent
would). Files go to `benchmarks/real/external/github-sch/<board>/repo/`
under their repository paths, with `schematic.json` beside; the committed
`schematics.json` has the same per board (sheets, libraries, footprints
found, unresolved libraries, size, licence) and why boards were skipped.

```sh
benchmarks/github/harvest_schematics.py [--only NAME...] [--refresh]
```

A board is skipped without a schematic of its name at the commit (panels,
a renamed project), with a KiCad 5 `.sch`, a sheet missing or Git LFS
pointers, or when its licence does not allow copies: no licence file at all
(all rights reserved), or a licence text without a grant. GitHub's licence
detection decides where it names one; otherwise the licence files at the
root and along the board's folder are read (CERN OHL, GPL, Creative
Commons, Hippocratic, ... texts GitHub calls `NOASSERTION`).

2026-10-15: 79 of the 109 boards have a usable schematic (241 sheets,
1171 files, 95 MB). Skipped: 17 without a licence file (the six ISS-PCB
boards, seven of hackclub's OnBoard, MegaDrive, Neotron-Pico, SUMEC, ATAT1800,
vd-rd's module), 10 without a schematic of the board's name (the PolyKybd
molecules and the OpenESC, OpenRX and 0xCB panels, krishveercard), 2 with a
sheet missing at the commit (the Hub, poly_corne_split42_right), 1 KiCad 5
schematic (Qfwfq). Of the 79, 50 have every named footprint in KiCad's
libraries or the repository; 22 needed `library_fixes`; 19 use submodules.
The schematic tier (benchmarks/agent-tasks/README.md, "The schematic tier")
runs the agent's flow on 12 of them.

## Running

Route mode (the designer's placement, tracks and vias stripped, pours kept):

```sh
benchmarks/corpus/run.py build/github-route --corpus benchmarks/github/boards.json --skip-layout
```

Layout mode as agent tasks (parts unplaced, connectors the designer put at an
edge constrained to that edge, overhanging and shield-like parts fixed):

```sh
benchmarks/agent-tasks/from_pcbench.py build/github-tasks-v7 --corpus benchmarks/github/boards.json
flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/github-layout-v17 \
  --tasks build/github-tasks-v7/tasks.json --binary build/bin/pcb-maker-xNNN --jobs 8 --threads 2
```

The current task list is v7 (`build/github-tasks-v7`: v6 with uuids on
the entries of repeated references, six panels); the quick and short
tiers still default to v6, and a comparison needs one list on both
sides. One benchmark at a time on the machine, under the lock
(benchmarks/agent-tasks/README.md, "How to measure").

Freerouting against our cold route, headless (never start the jar without
`-de`/`-do`: it opens its GUI):

```sh
benchmarks/github/freerouting.py build/freerouting --ours
```

Results go to [docs/benchmarks.md](../../docs/benchmarks.md); the
current ones are summarised in
[docs/handover.md](../../docs/handover.md#benchmarks-that-count).
