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

## Running

Route mode (the designer's placement, tracks and vias stripped, pours kept):

```sh
benchmarks/corpus/run.py build/github-route --corpus benchmarks/github/boards.json --skip-layout
```

Layout mode as agent tasks (parts unplaced, connectors the designer put at an
edge constrained to that edge, overhanging and shield-like parts fixed):

```sh
benchmarks/agent-tasks/from_pcbench.py build/github-tasks --corpus benchmarks/github/boards.json
benchmarks/agent-tasks/run.py build/github-layout --tasks build/github-tasks/tasks.json
```

Results go to [docs/benchmarks.md](../../docs/benchmarks.md).
