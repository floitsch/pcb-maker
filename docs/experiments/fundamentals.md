# Fundamentals investigation (2026-11-23 to 11-25): what to merge

On 2026-11-23 Florian asked for a review of fundamentally different approaches
beyond lattice A*, PathFinder and annealing. That review is
[fundamental-approaches.md](fundamental-approaches.md). Its top suggestions
were then tested in sibling worktrees, one package per branch. This file is
the index: what each package found, what this branch brings to main, and what
is left to do.

**Branch `fundamentals`** (worktree `../pcb-maker-fundamentals`), from main
14321b39. It merges six of the seven package branches and adds four commits:
- the noise study's write-up and scripts, without its code;
- the router retry's doc updates;
- this index;
- the `placer.md` correction, last and separate.

Every branch merged into main without conflicts on 2026-11-25.

## Headline

1. **Strict passes are an endgame problem; the open count is a packing
   problem.** On the 2026-11-18 sweep, 22 of 38 failures had 1 to 7 open.
   - The leftovers are mostly single-net misses and model walls (dead pads,
     pockets, pour islands), not joint conflicts that would need an exact
     solver (window census).
   - Some single-net misses were never tried, because of a gating bug that
     the router retry fixes.
2. **Placement: the objective accepts the designer's arrangement of active
   parts, but our search and refinement walk away from it.** Refinement
   chases the decoupling relations at 50 per mm. On Glasgow the designer's
   own placement goes from 19 open to 53 after the legaliser and 93 after the
   refinement (objective terms).
3. **The routed first route is mostly noise for comparing placements.** The
   placer × router seed interaction is about 72 % of its variance (noise
   sources).
   - The 5-pair A/B rule stays: it already treats seeds as independent draws.
   - The race's single probe per placement is mostly routing luck.

## The packages

| Package (branch, head) | Question | Finding | What this branch brings |
| --- | --- | --- | --- |
| Endgame tier, E1 (`fund-endgame-tier`, 05dda1d9) | A cheap benchmark for endgame changes? | Yes: 38 frozen near-miss boards, about 12 min at 3×2, deterministic in unconnected. It reproduces endgame changes and is blind to negotiation changes. [endgame-tier.md](endgame-tier.md) | Everything (Python only): `run.py --tier endgame`, `--endgame-release`, `endgame_freeze.py`, `endgame_report.py`, a `docs/benchmarks.md` row |
| Router retry (`fund-single-net-retry`, 50813c6f) | Why did the endgame miss closes that a single-net search finds? | KiCad's library findings hit their 199 cap and blocked the KiCad-judged repair on 17 of 34 layouts. Route mode had no open repair, and a layout repaired at most 1 net. [single-net-retry.md](single-net-retry.md) | Everything (Rust). Results below |
| Window census, A (`fund-window`, 15556e91) | Would an exact (SAT) window solver close the residue? | No go: 14 of 117 open items SAT, and 13 of those needed no other net moved. [window-census.md](window-census.md) | Everything: lab command `census-kicad-window`, `Router::window_census`, `Router::footprint_of` (additive, research only), `experiments/window-census/` |
| Cut census and skeleton gate, B and C (`fund-cuts`, 6fc15944, inside `fund-objective-terms`) | Are designer gaps sized to track quanta? Does the skeleton objective prefer the designer? | Gap thesis killed. Ours packs about twice as many gaps below one track. The skeleton objective ranks the designer's skeleton worse. [cut-census.md](cut-census.md) | Everything: a `skeleton_objective` trace line under `PCB_PLACER_TRACE`, `experiments/cut-census/` |
| Objective terms (`fund-objective-terms`, 1d3d8b55) | Which objective term misjudges the designer? | None to add: candidate terms C1 (pin room) and C2 (`constraint_weight` 10) are both dropped. The objective prefers the designer's active parts with our passives on Glasgow and the 4in1, not on the 4in1-mini. Refinement walks away from them. [objective-terms.md](objective-terms.md) | Everything (Python and docs) |
| Evaluation representations (`fund-eval-representations`, 8a664fd0) | What helps an agent see why a connection is open, or which placement routes better? | Per-open-item cards (annotated window and evidence, judged by KiCad): 12 of 12 correct at a quarter of the time. On 12 held-out cases ([cards-heldout.md](cards-heldout.md)) that does not hold: the generator's own verdict was right 4 of 12, subjects with cards 8 of 12, subjects with raw outputs 8 of 12; the cards are cheaper (47k against 69k tokens, 49 s against 195 s). Every placement representation is at chance, and pre-routing measures mislead. [evaluation-representations.md](evaluation-representations.md) | Everything (Python, cases, subject prompts, results; 0.34 MB) |
| Noise sources, E3 and E2 (`fund-noise`, d6340d12) | Is the A/B noise placer or router? Do counter-based random numbers keep pairs paired? | Router × placer interaction is about 72 %. Pairing removes no variance with either RNG, and the counter RNG left 3 of 8 protoesp placements illegal (cause unknown). [noise-sources.md](noise-sources.md) | **Write-up and `experiments/noise-sources/` only.** The RNG prototype and placebo hooks (`PCB_PLACER_COUNTER_RANDOM`, `PCB_PLACER_PLACEBO`) stay on `fund-noise`: the verdict is not to adopt them, and main prunes switches |

### What changes behaviour on main

Only the router retry (crates `pcb-kicad-route` and `pcb-kicad`):
- **`open_repair.rs`:** `COPPER_INDEPENDENT_TYPES` (`lib_footprint_issues`,
  `lib_footprint_mismatch`, `text_height`, `text_thickness`) are exempt from
  `report_capped` and from `adds_findings`.
- **`board_router/route_open_repair.rs`:** route mode runs the open repair
  after its ladder, the necks and the kept-copper trial.
- **`board_layout/ladder.rs` and `additive_repair.rs`:** a layout's final
  ladder and its neck repair leave `additive_repair::reserve` for the repair.
- **Net count:** the repair tries up to 4 nets.

All other Rust is additive and off the agent path: a lab command, two router
methods used only by it, and one trace line printed only under
`PCB_PLACER_TRACE`.

The retry's measurements, on its base 262adf59:
- **Endgame tier:** 99 → 79 open and 5 strict, against main's 99 → 81 and 3.
- **Census boards:** 116 → 107 open, 4 boards complete.
- **Designer route mode:** DSP-ADAU1452 and split72_right complete.
- **The 11-24 sweep's 17 near misses:** 58 → 58. The cap no longer refuses,
  but the searches find no local path.
- **Guards:** identical to the 11-24 nightly. Fingerprints are byte-identical
  boards, with 16 extra fingerprint lines in Sisu's `route.log` from the
  repair's searches; a log diff will flag them, and that is expected. The
  expansion tier is identical, because the repair does not run without KiCad.
  The short tier is identical (0 better, 12 same, 0 worse).

### Before merging

- **The checks pass on this branch** (2026-11-25):
  - all of `scripts/check.sh`'s steps: every crate's tests, the CLI and
    fixture tests, the agent walkthrough with KiCad, the Python suites,
    clippy with warnings as errors, and the release build;
  - `cargo fmt --check` and `check_links.py` (0 broken).
  - Two `test_tier_estimates.py` tests fail only when `TMPDIR` lies inside
    the repo (the path comes back relative); with `TMPDIR` elsewhere they
    pass.
- **Re-check on today's main.** Main moved 38 to 41 commits since the
  packages branched, so run the short tier and the endgame tier on this branch
  against main before merging. The guards above were run on the packages'
  bases, not on today's main.
- **Pick a merge.** Merge the whole branch, or pick:
  - `git merge fund-single-net-retry fund-endgame-tier` for the behaviour
    change and the tier;
  - the research branches for the record.
- **Decide on the `placer.md` commit.** It is the branch's last commit
  ("placer.md: the objective accepts the designer's active arrangement ...")
  and touches only `docs/placer.md`, applying the wording that
  objective-terms.md proposes. Revert it if the wording isn't agreed. Two
  other lines in that section are now in tension with the study but are not
  edited:
  - "The legaliser and the refinement keep the arrangement they are given":
    on Glasgow the refinement moves the active parts 1.1 → 3.4 mm and costs
    40 open.
  - "recover a median 0.92 of the designer's first-route advantage": 0.49
    today, because our seeds route better now.
- **Delete the untracked copy.** `docs/fundamental-approaches.md` in the main
  checkout is an untracked copy of the note; with this branch merged it lives
  here, and the copy can go.

## Data

Run outputs stay outside the tree, in the shared untracked `build/`:
- **Frozen endgame inputs:** `build/endgame-tier/v1`. Back them up; the tier
  cannot be rebuilt bit-identically once the sweep's stalled boards are gone.
  `endgame_freeze.py` says how they were made.
- **Package scratch:** `build/scratch/fund-*` (runs, chain scripts, logs).
  - `fund-noise/runs` is 123 cells, about 15 CPU hours.
  - `fund-objective/recon` holds the 36 pinned first routes.
- **Window census:** it needs CaDiCaL (`/usr/bin/cadical`), and one census run
  can reach 17 GB, so cap it at 8 GB as its write-up says.

## Follow-ups, roughly by value

1. **Placement bet: stop moving the active arrangement to chase the
   relations.**
   - **Where:** in the refinement, and in the skeleton phase's legaliser and
     refinement, which move the active parts 5 mm before any passive exists.
   - **Options:** anchor the active parts once the skeleton is set, or cap the
     relations' pull on them.
   - **Judging it:** several placer seeds per board, because of the noise
     finding.
   - **Evidence:** objective-terms.md section 4.
   - **Done (2026-11-26), and it loses:** [keep-arrangement.md](keep-arrangement.md).
     On our own placements the legaliser and the refinement do move the
     active parts, but holding them through the refinement routes worse
     (the short tier 9 to 21 unconnected items, 25 sweep boards 2173 to
     2449, three seeds each), and an anneal that leaves the legaliser
     little to move routes no better. No placer change came of it.
2. **Per-open-item evidence in `verdict.json`**
   (evaluation-representations.md). The held-out test
   ([cards-heldout.md](cards-heldout.md)) found the cards' generated
   verdict right on 4 of 12, so do not port it; the picture of the window
   and the reach data are the useful part (subjects were right where they
   overrode the verdict from them), and they cost an agent less than the
   raw outputs. Build them on the retry's single-net KiCad-judged search.
   The misleading outputs the same study found are being handled in
   [misleading-outputs.md](misleading-outputs.md).
3. **Three router defects** found by the evaluation study:
   - the neck rule ignores same-net copper (confirmed by the retry);
   - an escape is dropped beside duplicate pads (not checked);
   - same-number pads on two layers are merged (not checked).
4. **The race:** test n placements × m router seeds at an equal probe budget
   (noise-sources.md). Replicating router seeds is plausible but not shown.
   - **Done (2026-11-27), and nothing wins:** [race-seeds.md](race-seeds.md).
     On a matrix of every candidate routed to the end under three router
     seeds (8 boards, 69 layouts) every allocation with router seeds ends
     at +4 to +15 % of today's unconnected items; the probe's count does
     not rank one placement's router seeds (Spearman -0.07). The race stays.
5. **What leaves tonight's near misses open, if not single-net misses?** The
   retry found no local path on the 11-24 sweep's 17. Look at the model walls
   on those boards, the window census's classes, before any joint solver.
   Main since records the router's own reason per open terminal
   (`routed.nets[]`'s `open`, [agents.md](../agents.md)), and the nightly
   has a census of open causes (`scripts/nightly-causes.py`).
6. **Unexplained:** the counter RNG's illegal placements (only on
   `fund-noise`), and why the 2026-11-24 nightly's layouts took about 6 h of
   wall time for about 25 min of CPU. Load and the memory ledger are likely,
   but neither was diagnosed.
