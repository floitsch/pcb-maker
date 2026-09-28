# PCBench / PCBWorld D3: open-source boards as a routing benchmark

About 680 real open-source KiCad boards, prepared exactly the way PCBWorld's
D3 benchmark prepares them. Results are therefore comparable with published
numbers:

- [PCBench](https://github.com/PCBench/PCBench), 1 183 boards from GitHub,
  with Freerouting and PcbRouter baselines;
- [PCBWorld](https://github.com/LGAI-Research/PCBWorld) (KDD 2026 workshop),
  which curated 679 of them into D3, split by size into D3-A/B/C, and reports
  Clean Pass rates for Freerouting, OrthoRoute, KiCadRoutingTools, RL and
  LLM agents.

No board content is committed here: `fetch.sh` downloads and prepares
everything under `benchmarks/real/external/` (gitignored). The committed
`manifest.json` holds only numbers and names.

## Prepare

```sh
benchmarks/pcbench/fetch.sh            # ~1.5 h on 8 cores; --limit N for a trial
```

The script:

1. Clones PCBench @ `dec3be75` and PCBWorld @ `b3d62f5`.
2. Runs PCBWorld's preparation chain (BSD-3) with the local KiCad (9 or 10):
   - KiCad 5 → current format;
   - `.kicad_pro` derived from the legacy setup;
   - D3's rule patches (some checks KiCad 9 added are ignored; hole-to-hole
     floored at the Default clearance);
   - DRC filter: the designer's routed board must pass;
   - the *guide* board: each net's designer width folded into net classes.
3. Writes `manifest.json` (`manifest.py`).

With KiCad 10.0.6 the chain reproduces D3 up to `d3_boards_missing_here` in
the manifest. PCBWorld used KiCad 9.0.8.

## Run

```sh
benchmarks/pcbench/run.py build/pcbench-d3test --subset d3-test --jobs 4
benchmarks/pcbench/run.py build/pcbench-smoke --subset smoke
benchmarks/pcbench/run.py build/pcbench-all --subset all --jobs 4 --timeout 1800
```

- **Subsets.**
  - `smoke`: 12 boards, 4 per tier.
  - `d3-test`: PCBWorld's test boards, 99 D3-A + 10 D3-B + 10 D3-C.
  - `d3a-test`, `d3b-test`, `d3c-test`: one tier's test boards.
  - `d3a`, `d3b`, `d3c`: a whole tier.
  - `d3`: every D3 board.
  - `all`: every board that passed the filter.
- **Modes.** `--mode route` (default) routes the designer's placement;
  `layout` places and routes; `both` does both.
- **Resuming.** Rows are appended to `results.jsonl` as boards finish, and
  a re-run skips finished boards.

**Clean** is PCBWorld's Clean Pass (CP): every connection routed, and native
KiCad DRC (the board's own D3 rules, error severity) reports no violation and
no unconnected item.

## Comparing with published numbers

- **Boards.**
  - PCBench's table covers all 1 183 boards on the KiCad 5 files.
  - PCBWorld's Table 3 covers only the test splits. It reports @5: best of
    five rollouts, Freerouting as the mean over 4 seeds.
  - pcb-maker is deterministic, so its numbers are @1.
- **Published Clean Pass rates** (PCBWorld Table 3):

  | Method | D3-A (99 boards) | D3-B (10 boards) |
  | --- | --- | --- |
  | Freerouting | 0.80 | 0.78 |
  | KiCadRoutingTools | 0.74 | 0.20 |
  | Best RL | 0.94 | 0.45 |
  | Best LLM | 0.65 | 0.00 |

  D3-C was not evaluated in the paper.
- **Freerouting success** from PCBench's table is listed per tier in
  `summary.md`. It comes from their run on the unconverted boards, so it is a
  reference, not a matched comparison. A matched run uses the corpus runner's
  `--freerouting` harness.

## Results

### 2026-09-28, route mode, all 617 boards

Commit 7b8fc4d, 4 boards at a time with 2 threads each, 900 s per board
(`build/pcbench-all-v9`; two outline boards re-run after the last fix).

| Tier | Boards | pcb-maker clean | PCBench Freerouting success | pcb-maker vias / designer | copper / designer | median s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| D3-A | 97 | 97 (100 %) | 93 (96 %) | 0.49 | 0.89 | 6.4 |
| D3-B | 267 | 263 (99 %) | 242 (91 %) | 0.43 | 0.92 | 9.1 |
| D3-C | 248 | 236 (95 %) | 187 (75 %) | 0.59 | 0.94 | 37.3 |
| not in D3 | 5 | 5 | 4 | | | |
| all | 617 | **601 (97 %)** | 526 (85 %) | 0.56 | 0.93 | 12.3 |

- **PCBWorld's test splits** (94 + 9 + 9 boards reproduced here): all
  clean.
- **Published Clean Pass on the same splits:**
  - D3-A: Freerouting 0.80, KiCadRoutingTools 0.74, best RL 0.94, best
    LLM 0.65.
  - D3-B: Freerouting 0.78, best RL 0.45.
- **The 16 boards that are not clean** (`triage.py`):
  - 14 leave connections open.
  - glasgow reaches the 900 s limit.
  - navelino-leaf's outline has junctions.
  - No board has a model mismatch or a copper DRC error.
- **Progress over the day.** The first full run (commit 30df46e) had
  586/617 clean, with 31 failures: 4 errors, 3 model mismatches, 7 DRC
  errors and 17 open.

### Bugs these boards exposed (all fixed)

The first sweeps found real bugs in how pcb-maker reads boards:

- **Nets and holes.**
  - Numeric net labels under a sheet path (`/1`) were dropped as
    single-pin placeholders.
  - Slotted mechanical holes were discs of their long side.
- **Pad shapes.**
  - Trapezoidal pads were rectangles.
  - SMD pads on every copper layer were used as vias.
  - Custom pads without primitives were rejected.
- **Copper text.** Its boxes were up to 1.75 times too large. The estimate
  now comes from glyph advances measured on KiCad's own plots.
- **Clearances.**
  - Pad clearance overrides did not replace the net class the way KiCad
    does.
  - Negative overrides (KiCad 4) were rejected.
- **Outlines.** Micrometre gaps, full circles written as arcs, and
  outlines made of 0.01 mm segments.

The benchmark also led to router changes:

- Attempts that leave connections open are retried with four seeds.
  Starling completes; Freerouting fails on it.
- The ladder ranks clean attempts by vias.
- `board-router.json` now says why a board is incomplete.

### What is left

- **Structurally hard boards.**
  - Congestion at fine-pitch parts: T962A, stm32_mech_keyboard.
  - Dense 4-layer boards: Own-Mailbox, glasgow (Freerouting fails on
    them too).
  - Large boards that hit the 900 s limit: dropbot, glasgow.
- **A D3 artifact.** BB-PWR-8113 needs copper exactly on the board edge,
  which D3's rule patch (edge clearance 0) allows and pcb-maker does not.
- **An unusual outline.** navelino-leaf's outline has junctions where more
  than two segments meet.
