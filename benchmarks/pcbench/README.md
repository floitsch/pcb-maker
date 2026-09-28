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
