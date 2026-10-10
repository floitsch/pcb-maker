# Hand-over (state as of 2026-10-09, binary x155)

What the tool does today, what the benchmarks say, what is known to be
wrong, and where everything is. The day-by-day history lives in git; this
file holds only what the next session needs.

## The flow in one paragraph

`layout-kicad-board` places the free footprints (ePlace-style global
placement, annealing legalisation, three seeds raced by a quick route),
routes the board with the lattice router (PathFinder negotiation; pours as
fill the pads join through thermal spokes, with stranded pads routed as
tracks and islands stitched with vias), resumes the first route briefly,
polishes without renegotiating, and - when connections stay open - runs the
attempt ladder (probe every pour mode for 75 s, continue the best; then the
regular rungs, finer pitches, seeds, signals at the project's narrowest
width). Every budget is in work (search expansions), with a wall-clock guard.
KiCad's ERC/DRC/connectivity decide pass or fail. `route-kicad-board` is the
same without placement. See `docs/router.md`, `docs/placer.md`,
`docs/constraints.md`, `docs/agents.md`.

## Benchmarks and where they stand

**Layout sweep v11** (`benchmarks/agent-tasks/run.py` on
`build/github-tasks-v5/tasks.json`, binary x136 with killed or panicked
boards rerun on x137-x146; 109 boards harvested from GitHub, each reduced to
stacked parts plus a `constraints.json` derived from the designer's board,
1800 s each): **38 of 109 pass** (KiCad: 0 unconnected, no copper error,
constraints met). Against the x112 baseline on the 29 boards both have:
better on 14, same on 13, worse on 2 (the two OpenESC/OpenRX panels), 1598
to 1231 unconnected, passes 5 to 5. Reproduce the table with

    benchmarks/agent-tasks/compare.py build/github-layout-v8-x112.log build/github-layout-v11.log \
      --splice build/github-layout-v11-x138.log build/github-layout-v11-x139.log \
               build/github-layout-v11-x140.log build/github-layout-v11-x141.log \
               build/github-layout-v11-x146-jiran.log --markdown

The full table is in `docs/benchmarks.md`.

**A caveat on every layout sweep up to v12 (found 2026-10-09 by the
congestion-model agent):** `run.py`'s unplace only read KiCad 10 net
syntax, so on the 89 pre-KiCad-10 tasks every footprint counted as netless
and stayed where the designer put it; only 22 of the 109 tasks were really
placed (the v12 logs' "placed in" times show which). Fixed in run.py;
v13 is the first sweep that places all of them, and its numbers will be
lower. Route-mode conclusions (pour model, connect rungs, Freerouting
comparison) are unaffected.

**Layout sweep v13** (x503, 12 jobs, load 25-40, 14525 s): **31 of 109
pass** with every board placed from stacked parts; 3586 open in all. The
honest layout baseline from now on (v12's 41 was mostly routing). 7
placement-illegal rows (5 molecule panels, framework M1, ghoul), 6
timeouts (katia, PixelWave, SmartSpin2k, MegaDrive, two 0xCB panels).
v14 should run on the 16-seed race (x504+) once the placement budget for
the seed count is in. Table in `docs/benchmarks.md`.

**The first real-layout baseline, quick tier** (`build/quick-v13-base.log`,
x165 with the fixed run.py, 37 boards, 12 jobs, 3617 s): **18 of 37 pass**
(25 when the designer's placement was kept). New failure classes now
visible: four boards fail before routing because the placer finds no legal
spot for a part (framework_mobo_lefthalf M1; laptop power C8/R49/C22;
ErgoSNM KEY31/KEY33; urchin SW31/SW23); the SmartSpin2k panel at 270/585;
WLED_Matrix 26 and PixelWave 30 unconnected with shorts, bridges and 50
keepout violations; MIDAS-MK1 15, BAGEL 7. Starved thermals only:
mackerel-08 8, Castor 9, 1337-v4.0 3, bumwings R64 1. The tier's stored
times and peaks (quick.json) are from designer-placement runs and must be
redone (one board peaked at 5.7 GB against 3.7 assumed).

**Layout sweep v12** (x165 = 1c7a557, 14 jobs, 10375 s in all): **41 of
109 pass**; against v11 better 12 / same 77 / worse 12, unconnected 4722 to
4771 (SNSP +90 and Quanta75 +25 are the known costs of the mask-body and
plate rules; A13 +42: x141, x146 and x155 all give 162/223 with 103 under the
sweep's load and the same race as v11, x165 picks another seed (279
against 288 unfinished after the probe) and ends worse again; the board
is sensitive to the race's pick and to load, not to one code step). Seven
error rows: Telemetry's degenerate fill-keepout rule area (new lowering
bug), the `refresh_pour` -> `find` stale-index panic on SUMEC, coco2,
bumwings xiao_sd and Qfwfq, PolyKybd right at the 1800 s limit under
load, the 0xCB panel's kicad-cli timeout. Table in `docs/benchmarks.md`.
Compare: `compare.py build/github-layout-v11-spliced.log build/github-layout-v12.log`.

**Freerouting against our cold route** (`benchmarks/github/freerouting.py`,
headless, 1500 s each, both on the designer's placement with all copper
removed; `freerouting_compare.py`): all 109 boards. Freerouting finishes 70,
times out on 34 and fails on 5; where it finishes we have fewer unconnected
on 49 boards, it on 6; unconnected in all 913 against 206; clean boards 8
against 50. Our rows mix x136 (40 boards) and x155 (69); rerun the 40 on a
current binary when the comparison is quoted as final. Logs:
`build/github-freerouting{,-2,-3}.log`, `build/github-ours-cold{,-2,-3}.log`
(`-all.log` is their concatenation).

**Route sweep v16** (`benchmarks/corpus/run.py --skip-layout`, x136, 2700 s)
has 2 rows (jetson 338/340 with 2 unconnected, link 246/251 with 8) and was
stopped; restart with `build/chain-v11.sh`'s last line.

Rerun lists kept: `build/github-layout-v11*.log`, `build/github-freerouting*.log`,
`build/github-ours-cold.log`; binaries `build/bin/pcb-maker-x112` ... `x155`.

## Not targets

- `thpoll83__PolyKybd__poly_kb_molecule_*` (5 tasks): tile panels with
  duplicated references (SW1 sixteen times, 576 footprints on 29 nets); the
  designer's own board has 266 unconnected. Kept in the runner, ignored.
- The OpenESC 4in1 panel, OpenRX panel, krishveercard, SUMEC (48 pads no
  track of their class can enter): the board's rules, not the flow.
- `bitshiftcrazy__d20_pcb`: 39 parts, a back-side keepout that allows
  footprints yet KiCad flags 27 of our pads inside it; 6 of 14 nets have no
  path. Not understood.

## What was learned this week, condensed

- **A knockout text is its plate** (the glyphs' box grown by the stroke),
  not its glyphs; a copper text with a twin on its mask layer is
  exposed-copper artwork and stays put, for the mover and for the layout's
  in-place route alike. Quanta75: shorts 0, the bridges left are the
  designer's J8 inside the logo. Routing less (146/172 against 169) is the
  price of the plate as an obstacle.
- **A footprint's own mask graphics are part of its body** for the placer
  (SNSP-CPU-01's J2: a 22 x 32 mm opening, 63 pads of other parts under it,
  each a bridge). The cost is real on that crowded board (333/363 with 116
  unconnected to 309/363 with 290; the race ends at 418 unfinished nets
  instead of 314), measured by bisecting (x143 reproduces x136 exactly).
  Kept: pads under a mask opening are bridges, and the back side stays free.
- **The ladder's memory** (x146): a finer pitch grows as the square of the
  pitch ratio and the seeds run side by side; jiran-ble-lite (3299 x 1048
  nodes at 0.1 mm, 13.8M at 0.05 mm, 7 GB a router) took 31 GB. The ladder
  now projects memory from the process's peak after the first attempt
  (`memory_mb`, `router_mb`): a finer pitch must fit 80 % of what is
  available, seeds half of it. jiran: 104/105, KiCad 20 unconnected (its GND
  has no pour; half its 40 GND pads are not reached on 2 layers; open).
- **KiCad's island rule** (x147-x155): a fill piece joined to the net by
  thermal spokes alone is an island; KiCad removes it and does not count the
  spoke ("spokes connected to isolated island"). Measured in pcbnew on
  k30-SBC: a via inside the piece cures it. Changes, all default:
  - `analyze_pours_full`: a piece gets copper of the net first (a track or
    via node, a solid pad); a thermal link joins a pad to a piece with such
    an item, never the piece to the pad;
  - every item-less piece a spoke reaches, for a pad short of two good
    spokes on that layer, gets a stitching via when one fits;
  - the net's own tracks are fill wherever fill may lie, inside a pad's
    thermal ring too (a plane stub ending in the ring read as "stranded, 1
    branch, 0 pieces" and was rerouted and stitched for nothing);
  - `canonicalize_copper_net_names` falls back to the bare name: copper is
    written as `(net "/GND")`, and since the net-name registry (x140) kept
    the slash when a bare GND exists, KiCad made a net "/GND" of our items
    (Hub: 636 items, 36 clearance errors);
  - `junction_hosts` skips a one-node branch between two junctions (the Hub
    panicked).
  Results: Castor 126/126 clean, islands 4 to 2 (x155; one run gave 5
  starved thermals in all, another 18: the winning rung decides); Sisu
  186/192 with 13 (x136: 185 with 26); Hub 146/159 with 17 (as x136); k30
  173/173 with 10 (as x136, its 8 one-spoke pads untouched).
- **Hard thermal guards do not work and cost routes.** Closing a pad's last
  two spoke corridors to other nets (`hard_thermal_guards`, off) left k30's
  one-spoke pads as they were and cost Castor six nets and Sisu fifty
  connections.
- **The finding under all of it: our fill is 10-40x more fragmented than
  KiCad's.** k30's GND fill is 63 + 33 + 29 outlines in KiCad (F, B, In2)
  and 1100-1500 pieces in our pour map; +5V on In1 24 against 338. The
  brush-centre model on a 0.1 mm lattice loses every neck near the minimum
  width (0.254 mm is 2.5 cells), and clearance rings around other nets' vias
  on a plane layer meet where KiCad's do not. The two-way pad links of the
  old model hid this; the one-way links show it as stranded pads, reroutes
  and stitching vias. k30's +5V: "38 stranded terminals" while KiCad finds
  every +5V pad connected.

## Committed on 2026-10-09 evening (b67da0a), measured by the pour agent

- Spoke clearance (a spoke counts only when its strip clears other
  nets' copper), pour-aware clean-up and via reduction (a change that
  strands a pour pad is rejected), stale plane stubs rerouted,
  deterministic stitching, the pour-refresh panic fixed at its root, circle
  rule areas lowered. k30 forced to connect: 41 open (before) to 0 open
  with 14 starved thermals, beating the tracks rung's 16. Castor's connect
  rung in route mode: 126/126 with 18 starved; Hub 144/159 with 20.
- Measured and left off: stitching vias during negotiation (Sisu 63 to 68
  open: the stitches crowd the signals), main-piece hysteresis (the main
  piece does not flip, it collapses when a stuck pour net is ripped up
  whole), not ripping pour nets up whole (worse), hard thermal guards.
- Cost: the pour-aware check's own-stamp lookup is a bitset now (k30
  forced-connect checks 179 s to 78 s, Hub 207 s to 130 s, identical
  results); skipping far pour nets never fires on boards whose pours cover
  the board.
- Same-copper comparison, route mode on v12's placed boards, x165 against
  HEAD: k30 identical (tracks wins both), Castor 184 to 176 vias at 18
  starved, Hub 18 to 20 unconnected, Sisu 20 to 13 unconnected. The
  probe ladder picks the tracks rung by its 75 s probe even where the
  connected rung's final result is better (k30 forced connect: 0 open,
  14 starved against tracks' 16): the ladder's judge has the race's
  noisy-probe problem.
- Real-layout rows on x204 (fixed run.py): k30 173/173 with 3 starved
  (365 vias to 287), Qfwfq 64/64 clean, Telemetry 107/108, Castor 125/126
  with one unconnected, one short and one bridge (the short is under
  investigation), Hub 132/159 with 40 (placed for the first time), Sisu
  183/192 with 12 (unchanged: it was always placed).

## Committed on 2026-10-10 (the memory and congestion agents)

- Rungs side by side with fixed work budgets, honest via-reduction budgets,
  a 4-ary key heap (9-13 % less search CPU), polish-again after a trial's
  polish, and the layout deciding by work everywhere: a board's layout now
  reproduces exactly run to run (CyberKeeb 3 of 3, Sisu byte-identical).
- The placement race pre-ranks 16 placer seeds by RUDY (routing demand over
  tile capacity) and probes the best three: quick tier KiCad unconnected
  98 to 64 (6 boards better, 2 worse), laptop power finds a legal seed.
  Default from the next commit. The neural predictor ranks race candidates
  about as well as RUDY (its label, a single 75 s probe, agrees with
  itself only at Spearman 0.6 between router seeds) and stays an
  experiment behind `congestion_model`; see docs/congestion-model.md.
- Placer correctness (the placement agent, in 3b890fa and 93b83f5): a
  fixed part's copper and mask graphics on its other side are obstacles
  there (Castor's logo: the short is gone); parts at odd angles are tested
  as turned rectangles and may turn square (ErgoSNM places: 58/58 clean);
  a footprint with a courtyard on each side gets a body per side (urchin
  places: 66/68); courtyards may touch where the pads inside keep the
  copper clearance, as a last resort (laptop power places on every seed,
  but RUDY then prefers a touching seed: 9 to 18 unconnected, a ranking
  fix is under way); rule areas inside footprints and pads-only rule
  areas are lowered (PixelWave 41 keepout findings to 0, 343/343 routed;
  d20 27 to 0); eviction 7x faster. framework_mobo's M1 has no legal spot
  at the designer's outline (its pads overhang the polygon): the task
  generator should test overhang against the polygon. SmartSpin2k: 16
  seeds cost 1218 s of placement on 4 cores (a placement budget for the
  seed count is under way). Measured: the legaliser's wirelength loss is
  mostly won back by refinement (final within 10 % of the global
  placement on most tier boards); the 1.4-1.9x against the designers is
  the routing halos, whether less halo still routes is the next experiment.
- Placer round 2 (a609a75): a touching-courtyard placement ranks last;
  the race's seed count follows the placer's 40 % share of the work (the
  placer's work clock was undercounting legalisation 20-fold; fixed);
  courtyards that fill under 75 % of their box block with their shape
  (ghoul places, urchin passes). Quick tier x617 against x500 (everything
  since x500 together): passes 16 to 20, KiCad unconnected 67 to 29,
  placement failures 4 to 1 (framework M1, the board's). The task
  generator now tests overhang against the outline polygon: 35 of 109
  tasks gain fixed parts (mounting holes in rounded corners, parts in
  notches); tasks v6 for v14. Open: `halo_scale` experiment (does less
  routing halo still route), the placer's model being stricter than
  KiCad at mounting-hole rings and pad boxes (45 designer poses illegal on
  ghoul), SmartSpin2k (a 585-connection panel whose first route alone
  needs 1000 s; not a target).
- Halo scale measured on the 30-board tier (x507): 0.75 and 0.5 cut the
  placer's wirelength only 1.3 and 4 % (halos are already shrunk where
  boards are crowded) and win nothing clearly (passes 19 / 19 / 17);
  1.0 stays. The 1.4-1.9x gap to the designers' wirelength is not the
  halos. The race's judge: the 75 s probe is the best pre-routing
  predictor of a layout's final result (Spearman 0.55 on 92 finals; two
  probes averaged no better; RUDY 0.30, wirelength 0.23, the network
  0.13), so the network is a closed negative as a ranker and
  `probe_seeds: 2` is not built.
- Speed is structural now: cheaper expansions are exhausted (five
  candidates under 5 %). The waste measurement (`PCB_ROUTER_WASTE=1`,
  bench-search) found chronically rerouted nets searching corridors up to
  10 tiles wide for narrow paths, and on Sisu and jetson half the
  expansions after iteration 20 gaining nothing. Committed (3c1f81d): the
  corridor margin grows every sixth reroute instead of every third: quick
  tier 10 % less CPU, unconnected 32 to 30, starved 20 to 15, vias -6 %,
  no pass lost; SNSP route mode 171 to 98 unconnected at +6 % vias. A
  stall stop (N=12 iterations without a conflict improvement; 3-11 % less
  CPU, boards nearly unchanged) is queued for a tier pair. Capping the
  margin fails (SNSP needs the wide corridors).
- Committed aa4e9e8: the negotiation ends twelve iterations after the
  price cap without a conflict improvement (tier -8 % CPU, -18 % board
  time, no pass lost). The 30-board tier's clean baseline on x508: 19 of
  30 pass, 23 unconnected, 12 starved thermals, 31.5 min of wall at 8
  jobs (`build/quick30-x508.log`).
- The quick tier is 30 boards now (`benchmarks/agent-tasks/quick.json`,
  README): real layouts in about 1100 s and 3.7 GB; about 35 min at 8
  jobs.

## Open problems, ranked

1. **Fill fidelity.** Measure our free mask against KiCad's fill polygons
   (`GetFilledPolysList` in pcbnew) where they differ; then either test the
   brush at sub-cell offsets for necks, or take KiCad's own fill as the pour
   map. Everything downstream of the pour model (stranded pads, stitching,
   the connect rungs' quality, starved thermals) depends on it.
2. **One-spoke pads on fine-pitch rows** (k30's U10 pad 8, 1.77 x 0.50 mm:
   the two corridors along the row are closed by the neighbours, and the
   neighbours' exit tracks run through the two left). A hard block failed;
   the repair (`free_thermal_spokes`, budgeted, only when polishing) moves
   2 of 10 nets. Probably needs the exit direction of the neighbouring
   pads decided before negotiation (a fan-out step), not a cost.
3. **The ladder's judge.** Measured 2026-10-11 (x406 = HEAD 916bfbf plus
   env hooks `PCB_LADDER_FORCE_RUNG`, `PCB_LADDER_STATS`,
   `PCB_LADDER_SHORT_RESUME`; `build/ladder-judge`,
   `experiments/congestion/ladder_judge.py`): every probe-ladder rung of
   k30, Castor, Hub and Sisu continued to its final in route mode. By
   (open, starved, vias) the probe picked the best rung on 3 of 4: k30
   tracks (0 open, 24 starved, 359 vias; connect 0/24/630, so the earlier
   "connect is better" is not reproduced on HEAD), Sisu tracks (35 open
   against 41-184), Castor connect (0 open, 8 starved). It failed on Hub:
   tracks probed 20 unfinished against 97-103 for the connect rungs, but
   ended 244 open against 127. The probe's count adds each stranded
   pour pad of the connect rungs (Hub: 82 of its 97), which the
   continuation then connects. The conflicted nets alone (no pour-pad
   term) pick the best rung by final open count on all 4 (Hub rung 0: 127
   open, 19 starved against the best 15). Incomplete nets also do on 4 of
   4, open after a 30 s continuation on 3 of 4. Four boards are not
   enough to change the judge; the next step is the same measurement on
   more 4-layer boards, comparing the probe count with and without the
   pour-pad term. Still open from before: the ladder stops at the first
   complete rung whatever its starved thermals (Castor x155: "0 open, 18
   starved" kept); rank by (open, starved) before vias, and let a
   complete-but-starved rung be rivalled.
4. **Crowded placement** (OpenESC, SNSP): the three directions are built
   as options and none wins; `replicate_channels`' rigid macros never get
   seated. The legalizer loses 30-90 % of the global placement's
   wirelength on every board; that is the lever.
5. **Starved thermals as the only failure** on complete boards (rosco 5,
   Castor 2-15, mackerel-30 15, k30 10, mackerel-08 5, Hub 5, PixelWave 1,
   BAGEL 1). `solid_starved_thermals` (opt-in: a starved pad gets a solid
   zone connection) would pass them; Florian has not decided whether it
   should be the default.
6. **Our open count against KiCad's.** The router counts connections per
   net, KiCad per missing pair; and the pour model's optimism (above)
   adds. The pass rule trusts KiCad; the router's own count is for steering
   only.
7. jiran-ble-lite's GND (no pour, 2 layers, 20 of 40 pads unreached); d20's
   keepout pads; the 0xCB panel's zone refill timeout in kicad-cli.

## Where things are

| What | Where |
| --- | --- |
| Sweep logs and outputs | `build/github-layout-v11*` (layout), `build/github-freerouting*`, `build/github-ours-cold*`, `build/github-route-v16*` |
| Snapshot binaries | `build/bin/pcb-maker-xNN` (x155 = HEAD); `cargo build --release` first, `cargo test` leaves `target/release` stale |
| Sweep chain | `build/chain-v11.sh` (layout sweep, then the route sweep) |
| Tasks | `build/github-tasks-v5/` (from `benchmarks/agent-tasks/from_pcbench.py --corpus benchmarks/github/boards.json`) |
| Harvested boards | `benchmarks/real/external/github/` (not in git; `benchmarks/github/manifest.json` has origin and licence) |
| Agent memory | `~/.claude/projects/-home-flo-programming-pcb-maker/memory/` |
| Untracked in the repo root | `pcb_pr_agent_brief.md`, `target-bisect/` (not from these sessions) |

## Gotchas

- Never run `cargo fmt`: the repo is not rustfmt-clean.
- `pgrep -f` and `pkill -f` match their own shell: a waiter on `pgrep`
  never ends, `pkill` kills the session. Kill by PID.
- No backticks in `git commit -m`.
- Benchmarks use a copied binary; run nothing heavy beside a sweep whose
  timings matter; pin experiments with `taskset`.
- The machine (Ryzen 9 5950X, 32 threads, 62 GB, 32 GB swap): the runners
  wait for free memory (`--min-free-gb 12`) and the ladder projects its
  own; three layout jobs plus a Freerouting and a cold-route sweep fit.
- KiCad 10: pinfunction is `NAME_NUMBER`; pad and text angles are absolute;
  copper items reference nets by name, and an unknown name becomes a new net
  unless a touching pad supplies one; DRC caps a violation type at 499 (199
  for some); its silkscreen DRC sometimes names the wrong pad.
- Freerouting's jar opens its GUI without `-de`/`-do` (`--help` too): always
  `-Djava.awt.headless=true -de ... -do ...`, or use `freerouting.py`.
