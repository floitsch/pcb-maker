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

**Layout sweep v15** (x723, tasks v6, 6 jobs, 17910 s, 2026-10-14): **45 of
109 pass**, 3829 open; against v14 better 20 / same 63 / worse 16; the
worse rows are the crowded boards (see docs/benchmarks.md).
**Layout sweep v14** (x510, tasks v6, 10 jobs, 12668 s): **34 of 109
pass**, 3065 open in all; against v13 better 19 / same 51 / worse 24. The
baseline for what comes next (`build/github-layout-v14.log`; tasks
`build/github-tasks-v6`). Timeouts: link, katia, MegaDrive, the 0xCB
panel; no legal placement: the molecule panels and the OpenESC 4in1 panel.

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
- Speed, round three (2026-10-13; experiments in the worktree
  `pcb-maker-speed2`, diff `build/speed2/speed2-experiments.diff`, logs
  `build/speed2`, tiers `build/quick30-x333-L12.2.2.log`,
  `build/quick30-x334-viastop.log` against `build/quick30-x510.log`), all
  negative:
  - Chronic nets searched near their last route (`PCB_ROUTER_LOCAL=12,2,2`:
    a net rerouted 12 times or more searches its corridor cut to two tiles
    around its previous route, tree and targets first, every other
    reroute). Measured first: nets with 18+ reroutes find their path within
    one tile of the previous route in 97-98 % of searches yet expand 72-78 %
    of their nodes outside it. bench-search reaches lower conflict counts at
    equal work (Sisu 17 at 350M expansions against 28; jetson 9 at 550M
    against 25) and stops at 58 % / 82 % of the expansions. But the tier is
    neutral (19/30 both, unconnected 30 to 24, CPU -5.5 % against x510 and
    +9 % against x508: noise), Sisu in route mode -10 % CPU (pinned pair,
    swapped: 0.894, 0.915) ends 34 open against 27, jetson identical (2
    open) at -3 %. Route mode rarely reaches the chronic regime; where it
    does, the full searches are what finds the detours.
  - Not re-searching a conflict-free net: already so (a net's branches are
    tested for conflicts at its turn, after the nets before it moved; a
    speculative search that read a changed block is redone).
  - A cheaper first iteration: iteration 0 is 1-2 % of the negotiation's
    expansions on Sisu and jetson, the first three 3-7 %; not built.
  - On the tier the negotiation is not the cost: the via reduction is (its
    lines sum to 66 % of the v14 tier layouts' routing time, the clean-up's
    to 27 %; 80 % of its rounds reverted, 72 %
    because the round found no fewer vias, and after such a round the next
    is kept 9-11 % of the time). Stopping at such a round
    (`PCB_ROUTER_VIA_STOP`) cut the tier's via-reduction time 13 % and its
    CPU 1.7 % (noise), +18 vias: a layout spends its work budget anyway
    (more move trials), so a cheaper phase shows up as more trials, not as
    less CPU. Speed on the tier means quality per work, and a phase that
    ends by its budget gains nothing from being cheaper.
- The quick tier is 30 boards now (`benchmarks/agent-tasks/quick.json`,
  README): real layouts in about 1100 s and 3.7 GB; about 35 min at 8
  jobs.

## Committed on 2026-10-13 (the placement-gap agent, x644)

Five boards that pass with the designer's placement and not with ours
(zpn_devboard, video, MokyaLora, hackrf-one, homebrew_486; logs
`build/gap-*`), each run pinned with ours against the designer's placement.
Wirelength is not the gap: our HPWL is within 0.8-1.4x of the designer's on
all five, and on four the loss is a modelling or task bug.

- **MokyaLora** passes now: a movable footprint's own rule areas (a U.FL's
  keepout) now move and turn with it, in the writer and in `run.py`'s
  stacking (`move_zones`). About 15 of the 109 tasks change through
  `run.py`, so baselines made before this differ on those boards (three
  tier boards remeasured: `build/quick30-x510-zonetasks.log`).
- **zpn_devboard** passes now (two runs, 0 open, 291-299 vias against the
  designer's 136): `keep_on_pieces`, on by default, keeps the parts a net
  joins to fixed parts on one piece on that piece; before, most of the
  board had moved onto an empty breakaway (129 open after the first route).
  Nothing changes on the tier.
- **hackrf-one** loses its 6-9 `items_not_allowed`: a movable part's keepout
  that forbids pads keeps the far side's pads out; a board rule area on
  inner layers only that forbids pads keeps pads out on both sides (a
  through-hole pad crosses them). It still fails on two solder-mask
  bridges from mask-only test points the router does not treat as
  obstacles (the designer's placement routed by us shows the same two).
- **homebrew_486** is not a gap (plus or minus one unconnected run to run);
  its v14 pass is flattered: a courtyard-less footprint blocks only with
  its pads, so the second ISA slot interleaved the first 2.54 mm apart,
  and on hackrf a header and an SOIC sat inside a turned LQFP's pad ring.
- **video** is the one real objective gap: eight SIMM sockets in one column
  at the designer's 8.89 mm pitch line the bus up; we scatter them.
  `bus_rows` (identical footprints sharing a bus pin for pin placed as a
  row; through-hole only; off) gets video from 315 to 259 open at the
  designer's pitch and 499 with the automatic one, made mackerel-08 worse
  (+6 starved, +21 vias, +1100 CPU s) and homebrew_486's SRAM rows lost two
  connections; left off. Video needs more than aligned sockets.
- Rows whose members block with shaped boxes now block with every member's
  boxes (before only the first part's; likely the J9/U16 overlap on 486).

Quick tier x642 against x510: 19/30 both, unconnected 30 to 29, other
copper errors 8 to 3, starved 12 to 18 and +21 vias all from the bus rows
on mackerel-08 (now off); 25 boards byte-identical. Next on this thread:
courtyard-less ICs and connectors should block the inside of their pad
box (not a blanket rule: ULK puts 42 diodes inside its switches' pad
boxes).

## Planted boards (2026-10-13)

Boards routable by construction, three ways (`experiments/planted/`,
`docs/experiments/planted-boards.md`): maze (unrealistic), grow (realistic
at the easy level, cheap, but every one of 48 boards is easy for us: ours
shorter with fewer vias on 45), derived (blockers in a routed real board;
the only set that exposed failures, all starved thermals on complete
routes). Planted tile occupancy is a weak congestion label (Spearman
0.05-0.23 against our routing). No network; grow needs real difficulty
(pours on 2 layers, BGAs, buses) before its results are worth training on.
## Committed on 2026-10-13 evening (the bodies agent, x652)

Census of the 109 boards: 3800 footprints with pads and no courtyard
(2591 movable) on 92 boards; they blocked only with their pads, so hackrf's
LQFP-144 held 32 parts and homebrew_486's ISA slots interleaved. A Fab-layer
body rule fails (hackrf's library and the ISA16 draw none; where a Fab box
exists, 42 of ULK's switches hold a diode on purpose). The pad-pattern rule
`solid_pin_rows` (on): a footprint with 4 or more pads, three quarters of
them in rows or columns of 3 or more at most 2.6 mm apart, blocks with its
whole pad box plus copper and mask graphics; 625 footprints are solid, and
only 17 hold a designer's part (16 fixed-on-fixed). Keyboard switches,
hot-swap sockets and stabilisers stay hollow. `mask_only_pad_openings`
(router, on): a pad with a mask layer and no copper is a mask opening of
the net under it (hackrf's test points: its two solder-mask bridges on the
designer's placement become a pass). Copper-edge floor of 0.02 mm in the
placer (KiCad flags contact at zero clearance). Quick tier x651 against
x644: 19/30 both, unconnected 29 both, 21 boards byte-identical; OpenFC,
USB_Keypad and WLED change because parts no longer sit inside a QFN-80, a
USB-C receptacle or a LED pad box. hackrf: 0 open, one starved thermal
left (P5 VCC on the inner pour layer), but 1.6x the routed length now that
its parts are legal (crowded placement, problem 4); homebrew_486 passes
with its slots apart. Latent: the copper-pad aperture rule reads
`solder_mask_margin`, a key KiCad never writes (`pad_to_mask_clearance`);
noted in a comment. Logs `build/quick30-x644.log` (the reference),
`build/quick30-x651.log`, `build/bodies-*`; scripts `experiments/bodies/`.

## Committed on 2026-10-14 (the fill agent, x693 / x710)

Starved thermals were not optimistic fill misses: in a census of 44 routed
boards (4274 thermal pad-layers, 171 DRC starved findings;
`experiments/pour-fidelity/{spokes,census,classify,causes}.py`, `spokes.py`
counts spokes in pcbnew exactly as the DRC does, `dump-pour-map` writes
`<net>.spokes`) the old model predicted 77 of the findings but raised 866
false alarms, so the router could not tell real starved pads from false
ones. `kicad_spokes` (on; `PCB_ROUTER_OLD_SPOKES` for A/B) copies KiCad
10.0.7's zone_filler and zone-connection DRC: spoke end at gap + half the
minimum width, spoke width min(spoke, pad minor), landing test off the
lattice with exact distances (`router/fill.rs` FillProbe), clearance from
the DRC ring, facing spokes of neighbouring pads meet, the island rule
("single" = exactly one item touches the piece), and the DRC's rule that a
track leaving the pad into kept fill counts as a spoke. New census: 116
predicted, 36 missed, 87 false alarms. `manual_spokes` (on;
`PCB_ROUTER_NO_MANUAL_SPOKES`): after stitching, a pad our model finds
short of two spokes gets straight tracks of its own class into a piece
KiCad keeps (the DRC counts them as spokes); also for pour nets routed as
tracks. Quick tier x651 to x693: 19 to 21 pass (Castor, spell_tome,
rp2040zero), starved 12 to 2, unconnected 29 to 22, vias 2859 to 2838, CPU
ratio geomean 0.87 under different load (no slowdown). USB_Cable_Tester
lost its pass: GND fill under U1's F.Mask polygon (two solder-mask
bridges), a routing change from the spoke model, not the repair; the pour
under a mask opening of another net is the gap the bodies agent noted.
Starved-only boards in route mode: Castor 20 to 5, PixelWave 30 to 1, Hub
15 to 4, rosco 9 to 3, k30 21 to 14, mackerel-30 21 to 19. The rest:
inner-layer through-hole pads whose spokes reach only island pieces (k30,
mackerel-30; needs stitching of single pieces in tracks mode), island-rule
edge cases, pads with repeated numbers and custom spoke templates (19 in
the census, unmodelled). Cell-level fill agrees to 1-3 % but our map is in
far more pieces than KiCad keeps (Castor F.Cu 128 against 17); untouched.
Logs `build/quick30-x693.log`, `build/fill-starved`, `build/fill-census`.
## Committed on 2026-10-14 (the legaliser agent, x701 / x711): the premise was wrong

The legaliser does not lose 30-90 % of the global placement's wirelength.
Measured with `PCB_PLACER_TRACE` (per seed the weighted HPWL after global
placement, annealing, legalisation and refinement, plus the designer's for
the same movable set) on the 30 tier boards and nine gap boards: the
global placement is 30-90 % longer than the final one and the annealer
removes that (annealed/global median 0.83); legalisation costs 4 % (median;
Castor 25 %, MIDAS-MK2 28 % before refinement, where the nearest-spot
legaliser moves 100-120 parts because larger parts placed first take
their spots or a fixed through-hole part's far side is met); refinement
wins most of it back (final/annealed median 1.00). Final/designer median
0.85; above the designer only BAGEL 1.77, zpn 1.39, MIDAS-MK2 1.34, MK2.1
1.28, hackrf 1.19, Castor 1.14. Video (0.96), OpenESC (0.61-0.73) and SNSP
(0.85-0.93) are below the designers: their failures are not wirelength.
The gap to the designers on crowded boards is the routing halos: no halos
gives -22 % HPWL, and designers' own placements are illegal at our halo
level for nearly every part (they put courtyards against each other). Four
legalise-or-place-better variants (reflow only conflicted parts, push apart
a step at a time, filler whitespace, four times the annealer's moves) do
nothing beyond seed noise (about ±10 % per board); removed, numbers in
docs/placer.md. Two options built, both off: `anneal_far_side_gaps` (the
annealer sees the legality test's hole and far-side gaps; Castor's invisible
conflicts 33 to 0, parts moved by legalisation 119 to 66; tier flat, video
and OpenESC worse, SNSP better) and `shared_halos` (neighbours keep the
larger halo, not the sum; six gap boards HPWL -6 %; tier 19 to 20 passes,
unconnected 29 to 24, vias -8.8 %, but zpn loses its pass and hackrf +57 %
vias). Placement with defaults is byte-identical to x652. Lesson: a
placement change flips crowded boards by ±75 connections at random, so
seed-repeated pairs are needed before any placement default changes. Next
lever: routing room from congestion (a halo only where RUDY or the first
route says a part's escapes need it) instead of the pin-count rule; less
halo everywhere has now lost routing twice (x507 halo scale, shared halos).
Scripts `experiments/placement-loss/`; logs `build/legal-*`,
`build/quick30-x700*.log`.

## Committed on 2026-10-14 (the island agent, x722 / x723)

The islands are real in KiCad's fill, not our fragmentation
(`experiments/pour-fidelity/islands.py`): signal tracks and vias on the
plane layers cut them off in the tracks rung. KiCad experiments (kicad-cli
on mackerel-30 C50.2): a pad's own track ending inside the island cures
the finding, as does a track crossing it; a plated pad's fill stops at the
drill, so a track leaving a round through-hole pad between its diagonal
spokes touches none of them (the fill agent's "own track touches every
spoke piece" was wrong there). Changes, all on: `track_meets_spokes` (a
pad's own track joins a spoke's piece only where its first run overlaps the
spoke's strip outside the drill; census found 116 to 124, missed 36 to 28,
false alarms 87 to 90; `PCB_ROUTER_OLD_TRACKED`), `island_vias` (a short
pad whose manual spoke reaches no kept fill gets a stitching via from its
island to the net's main piece on another layer, also in tracks mode;
`PCB_ROUTER_NO_ISLAND_VIAS`), `island_stubs` (failing a via, a straight
track of the pad's own into its island, slivers included;
`PCB_ROUTER_NO_ISLAND_STUBS`). The stub changes no connection: the island
already hangs off the pad; it only satisfies KiCad's two-items rule. Its
broader form (stub every island-only pad) was rejected: 18 pads KiCad
passes for every 1 fixed. USB_Cable_Tester's bridges were not a pour
problem: KiCad exposes copper within `solder_mask_to_copper_clearance`
plus the aperture's expansion (0.1 mm there; our model had 0.075); now read
from the project (`solder_mask_to_copper_mm`) and used as the opening
clearance, always on. A pour keepout under openings exposing another net
was measured and not built: 1 real case in 128 results against 475 false
firings (`mask_bridges.py`). Pairs x710 to x722 (route mode, v14
placements): k30 starved 14 to 1 (+10 island vias), mackerel-30 19 to 3,
Castor 5 to 4, Hub 4 to 4, USB_Cable_Tester bridges 3 to 0; CPU within
±4 % paired. Quick tier x693 to x722: 21 to 22 pass (mackerel-08), 29 of
30 same, unconnected 22 to 22; USB_Cable_Tester's race picked another seed
whose /SHLD connect branch ends where KiCad leaves no fill (pour optimism,
one track_dangling). Remaining starved pads explained pad by pad in the
agent's report: spoke landing and piece fidelity (28 missed, 90 false
alarms), repeated pad numbers, a fine-pitch spoke-clearance case (k30
U14.12), slivers with no manual path. Reference tier log now
`build/quick30-x722.log`.

## Committed on 2026-10-14 (the routing-room agent, x737 / x738): measured, built, off

Halo granted against halo used on 35 routed boards
(`experiments/routing-room/halo_use.py`, `analyse.py`; a part's band is
its body grown 1 mm; a 25 s probe's conflict and history maps from
`export-congestion-sample`): pin count does not predict where routing goes
(Spearman about 0 per board), RUDY in the band does (0.4-0.84, median 0.6)
and the probe's history likewise; 7-34 % of halo area is used and 0-74 %
idle (zpn 33 %, keyboar_ 48 %, 0xcb-static 74 %); keeping the half of the
halo area with the highest RUDY keeps 58-79 % of the routing inside halos
on crowded boards, the half by pin count about 50 %. `routing_room` (off;
`routing_room_share` 1.0, `_floor` 0.5, `_most` 2.0, `_all_parts` on;
`crates/pcb-placer/src/room.rs`): per-part demand from RUDY over each
net's spanning tree, halo = pin-count halo times min(2, k demand/median),
k by bisection so the total stays at share times the level's area; after
the normal placement a re-anneal with small moves, legalise, refine; a seed
whose pass fails keeps pin-count halos and sorts after the others. Seed
pairs (seeds 1, 101, 201, pinned): the option never fires on video,
OpenESC and hackrf (their kept level has no halos; hackrf walks down to
halo 0 because one part finds no room); on BAGEL, MK2.1, MK2, Castor and
zpn passes over 15 runs off 11, on 11 (BAGEL and MK2.1 win, where the level
had stripped small parts' halos; MK2, Castor and zpn lose 5-9 % HPWL as
busy clusters spread; zpn gains 1-3 mask bridges); CPU geomean 1.15x (the
second anneal on every seed). Share 0.5 gives HPWL -8 % on all five without
the routing collapse of the uniform x507 scale, but two passes lost to
single errors (within seed noise). Quick tier x722 to x737 with the option:
22 to 24 pass, both single-error flips (USB_Cable_Tester's track_dangling,
eternal-keypad's shorting_items), vias -3.7 %, 8 boards identical. Verdict:
implementation sound, idea as built a wash; left off. Found on the way:
video's routing is not deterministic for an identical placement (392, 314,
325, 262 unconnected for seed 1). Logs `build/room-*`,
`build/quick30-x737.log`.

## Triage of the v15 regressions (2026-10-14, no code changed)

A read-only triage of the 16 boards v15 (x723) lost against v14 (x510), by
pinned route-mode pairs on the same placement, layout repeats, and layout
with `solid_pin_rows` off. Outputs in `build/triage/` and `build/triage-*/`.

- The router is not the cause. On the same placement x510 and x723 give
  identical tracks-mode results (counts, vias, length; x723 reports fewer
  starved thermals), connect mode differs by a few connections either way.
  CPU x723/x510 0.98-1.06. Every x723 layout repeat reproduced its v15 row
  exactly unless the wall deadline fired.
- Ladder budget cliff (module 8 to 81 open, jetson 9 to 25, vaio_re): the
  last ladder step, signals at the project's narrowest predefined width plus
  the relaxed clearance, is what nearly completes these boards (module 126
  to 17 open, jetson 48 to 6 in route mode). In layout mode the first route
  eats the budget and the step's guards (`no_time_for_another`,
  `spent < 1.5 * ladder_budget`) refuse it. Assigned: reserve the step's
  cost or run it as an early rung.
- Placement perturbation (RP2040_base 100 to 173, Hub, SNSP, glasgow): a
  rule change shifts the legaliser's path and the race picks another seed;
  v14's RP2040 placement is legal under `solid_pin_rows`, the rule only
  perturbs (off: 100 open again). Seed-repeated pairs are the only sound
  judge of such changes.
- Wall clock not booked as work (0xCB panel, glasgow, vaio_re): the
  legalise phases book 0.10-0.15 of their wall time as work (panel 504-966 s
  wall for 34 s of work), so the layout deadline fires and the result
  depends on machine load (vaio_re 63 or 151 open for one placement).
  Assigned.
- Tabbed panels (0xCB panel, OpenESC 4in1, molecule panels): one outline
  piece, so `keep_on_pieces` never fires and 15-22 parts of one copy land
  on another copy; the router then reports 49 of 188 nets without a path.
  Assigned: confine each net group to its own fixed parts' region.
- Far-side courtyards (woagboard 26 courtyard overlaps; koeg-board and
  Quanta75 may share the cause): `legal.rs` `clear_of` checks far-side
  rects only between parts on opposite sides; KiCad checks two back-mounted
  parts' front courtyards against each other. Assigned.
- protoesp and vaio_re lost because v14's placements put parts inside
  module and connector bodies that `solid_pin_rows` now blocks (honest).
- None of v15's 45 passes relies on a bug: `inside_bodies.py` finds only
  fixed designer parts inside solid boxes, the designer-budgeted errors are
  internal to footprints. Caveat: qfwfq's hole_clearance sits at KiCad's
  199 cap, like the designer's.

## Committed on 2026-10-14 (the determinism agent, x746 / x747)

Video's 262-392 unconnected for one placement was the layout's wall-clock
deadline (1.1 x 1500 s), not a race: in the four room-pair runs the
negotiation is identical to its last iteration and the runs part where the
deadline cut the hard repair (48, 70, 72, 98 nets rerouted). Video's work
lagged its wall clock (placement: 193 s of work, 630-1385 s of wall clock;
its seeds run in waves, each as long as its longest seed), so the deadline,
not the work, ended its layout under any load. Other wall-clock decisions
found: the via reduction's allowance in a layout (a third of the wall clock
left: Castor's final ladder 196 s on x738, 260 s of work now), the default
900 s negotiation of the attempts the ladder repeats from a probed rung
(wall clock only: video's narrow-signal step, 364 against 339 unconnected
on two runs), the pour nets' one-minute pre-negotiation in route mode, and
the probe ladder's check of the work left only between waves (the number
of rungs judged followed free memory). Route mode (Castor, Sisu: three runs
each) and the small boards' layouts were already deterministic.

Changed: the layout's routers stop at its end in work (router
`deadline_work`, armed before every phase; the end is `total_seconds`, the
wall-clock deadline 10 % later is the load margin); the waves of placement
seeds count toward that end (`waves_work_seconds`); the first route's
negotiation gets at most half of what is left; the via-reduction allowance,
the default negotiation budget and the pour pre-negotiation are in work;
probes are judged one after the other whatever ran side by side; every
wall-clock cut is logged (`wall guard: ...`) and reported (`cut` in the
reports and in run.py's rows, CUT in compare.py); `PCB_ROUTER_NO_WALL_GUARD`
drops the layout's deadline too. Video (seed 1, four threads): three runs
on eight logical CPUs 300/300/300 unconnected, 1362 vias, one fingerprint,
1355-1475 s; on four pinned CPUs it still meets the deadline (placement
alone 980-1385 s) and says so; without the guard there four runs of x744
and x746 give 318 and one fingerprint. Quick tier x746 (load 7-30): all 30
rows identical to x722, 22 pass, CPU geomean 0.97. The waves' count followed
the cores the process may use, so a run pinned to fewer cores could end
elsewhere than an unpinned one (fixed by the placement pool, below). Left: the clean-up's pour checks are not
work (a route-mode budget guard fired once under load 56, on a losing
rung). See docs/router.md, "Determinism and the wall clock".

## Committed on 2026-10-14 (fill round two, x768 / x769)

The census of 44 boards (`experiments/pour-fidelity/census.py`, KiCad's
starved-thermal findings against our pour model) before and after, same
tooling (the tooling now reads standard layer names; rosco's In1.Cu is
called "Route2"):

| | x760 (= x738 model) | x768 |
|---|---|---|
| DRC starved findings predicted | 129 | 146 |
| missed | 28 | 13 |
| false alarms | 90 | 27 |
| starved pads not modelled | 14 | 12 |

What changed, all on by default, each with the census step it bought (env
switches for A/B): track ends found anywhere across a trimmed spoke; spokes
meet at any angle; pad clearance overrides replace the zone clearance in the
fill, also when smaller (`Board::fill_clearance`, `PCB_ROUTER_OLD_FILL_CLEARANCE`);
sliver ends; spokes in the pad's own frame (`Board::pad_spokes`,
`PCB_ROUTER_OLD_SPOKE_FRAMES`; the routing guards keep the box axes, frames
there cost Hub 29 to 35 unconnected); repeated pad numbers checked on layers
where they are copper of their own (`Board::merged_pads`); no relief cuts a
spoke in the DRC track-end test; cut-off spokes bring no item; hatched zones
(`Plane::hatch`: spoke end half a web out, a ring around thermal pads,
KiCad's hole grid); coincident pads share tracks; foreign spokes across a
pad's ring count (`PCB_ROUTER_NO_FOREIGN_SPOKES`); tracks and vias are items
of every piece their copper overlaps (`PCB_ROUTER_OLD_TRACK_PIECES`); 3 micron
tolerance on brush clearances; joined outlines for a pad's overlapping spokes
and lattice-cut necks (`PCB_ROUTER_NO_NECK_JOINS`). Router fixes: `reroute()`
refreshes the pours after negotiation; `trim_dangling_plane_ends` cuts a
plane tail whose fill was taken back (USB_Cable_Tester /SHLD: a layout move
rerouted signals over the plane end, not fill optimism).

Route-mode pairs against x738 (pinned, same time): Castor starved 4 to 3,
k30 1 to 0, mackerel-30 3 to 2, Hub 4 to 4 with 29 unconnected (x761 and
x765 had 35 from spoke frames in the guards, fixed in x767); CPU 0.94 to
1.05. Quick tier `build/quick30-x768.log` against x746: pass 22 to 23
(USB_Cable_Tester), starved 7 to 6, vias 2847 to 2814, unconnected 22 to 25
(reCamera 4 to 9, its ladder went another way; MIDAS 11 to 9). Eleven router
tests, one per class. `experiments/pour-fidelity/render.py` draws KiCad's
fill against our map around a pad.

Fragmentation measured on 7 boards: our extra pieces are lattice sampling of
necks; PixelWave's 1212 single-node blobs hold no spoke and no item; of 23
island spokes in outlines our map split, 4 pads had a wrong verdict and the
neck and spoke-overlap joins fixed 3. Remaining disagreements by cause:
0xCB U3.45 thermal vias inside the exposed pad (10 misses, modelled since in
"exposed-pad thermal vias"; modelling them
made 15 false alarms); seams between overlapping same-net zones of
different priority (Telemetry, 5; modelled since in "zone seams"); marginal fine-pitch clearance within a
few microns; hatched fill lying over thermal pads without relief (not
understood); per-track DRC subtleties with islands.

## Committed on 2026-10-14 (CLI exit codes)

A benchmark agent found `route-kicad-board` exiting 1 on most harvested
boards with 0 open connections: the CLI failed on any error-severity DRC
finding, the designer's own included (USB_Cable_Tester on its v15
placement, x782: 54/54 routed, KiCad 0 unconnected, one `courtyards_overlap`
H4/J2 that the designer's board has too; Castor: 126/126, KiCad 0
unconnected, 4 `solder_mask_bridge` and 3 `starved_thermal`, exactly the
designer's board's, plus a cosmetic `footprint_type_mismatch`). The
layout's exit code used the router's count instead (pads joined through a
pour's fill counted open), and every error, a missing file and a panic
alike, exited 1 (a panic 101).

The contract now (`pcb-maker --help`, docs/agents.md section 4):

| Exit | Status | When |
| --- | --- | --- |
| 0 | complete | KiCad: no unconnected item, no error beyond the reference board's per type (cosmetic types and warnings never count); layouts and placements also place every part and keep every constraint |
| 1 | incomplete | finished, result written, but not complete |
| 2 | usage_error | arguments, missing or invalid inputs, unknown keys or parts (`pcb_kicad::input_error` marks the library's input-stage errors), output directory exists (now checked before the run, not after an hour of routing) |
| 3 | cut | incomplete and `cut` non-empty (a complete result with a cut is 0) |
| 4 | internal_error | other errors and panics (one stderr line; the panic hook keeps the default report only with `RUST_BACKTRACE`) |

The rule is run.py's: KiCad decides, not the router's count; the
reference's errors are subtracted per type (`pcb_kicad::verdict`, unit
tests there). The reference is `--reference <project-dir>`: route mode
defaults to its input as given (for a harvested board, the designer's
routed board, which is run.py's `designer_findings`), layout mode to none
(its input is stacked parts); it is checked only when the result has errors
that count (one more KiCad DRC, `reference-drc.json`). Schematic parity is
reported, not judged (run.py does not judge it either; the old route check
did).

Every route, layout and placement writes `<out>/verdict.json` (also on an
error once the output directory exists, and for a layout whose parts found
no place, which writes no board-layout.json, so that scripts' "report
missing" test still means "no layout") and the same as `verdict` in its
report: status, exit code, one-line reason, router and KiCad open counts,
new and pre-existing findings by type, cosmetic and warning counts, cut,
work and wall seconds, absolute output paths, skipped ladder steps and
next actions. board-router.json gained `work_seconds` (the ladder's work)
and `skipped` (the narrow-signal and minimum-clearance steps, finer
lattices and further rungs that did not run while connections were open,
with why). run.py records `exit` and `verdict` per row and takes a missing
report's error from verdict.json; its pass rule is unchanged. freerouting.py
and the corpus and PCBench runners read only the reports; they are
unchanged (their route runs now pay the reference DRC on boards whose
result has errors: about 10-50 s of wall clock, not in `routing_seconds`).

Found on the way: every route-mode run that skipped the probe ladder
reported `cut: ["ladder"]` (x746 to x782): the ladder's `spent` noted a
cut whenever the wall clock over 4 was ahead of the work, which at the
ladder's start (0 s of work) is always. The guard now takes over only when
the wall clock is ahead by more than a twentieth of the ladder's budget
(60 s of work by default); below that the shares are the work's alone.
USB_Cable_Tester routes the same board (86 vias, 2292.4 mm).

Verified: USB_Cable_Tester route on its v15 placement, exit 1 (x782) to 0
(x790), the courtyard overlap pre-existing; Castor (wntrblm mainboard, v15 placement) exit 1
to 0 with the same board as x782 (126/126, 225 vias, 5186.2 mm), its 4
solder_mask_bridge and 3 starved_thermal pre-existing in the input as in
the designer's board, 419 s wall for 399 s of work, the reference DRC
included; the tiny
three-pad fixture in route, place and layout mode (complete, unplaced parts,
a missed `near`), usage errors and a forced panic
(`tests/cli_exit_codes.rs`, `PCB_MAKER_TEST_PANIC`).

Left for agents to work around: a process killed by a signal (a harness's
timeout, the OOM killer) has no verdict; `import-kicad-netlist` now exits 1
when footprints are missing; research commands keep 0/1/2 (+4 on a panic)
without the incomplete/internal distinction. Tests: pcb-kicad, pcb-router,
pcb-placer and tests/cli_exit_codes.rs pass. Three tests of the root
crate's tests/kicad_ladder.rs fail, before and after this change (the
research ladder, code this change does not touch; a 2026-09-30 test build
fails the first one too): native_one_connection_transaction_commits_or_exactly_rolls_back,
native_shared_tree_insertion_reuses_copper_without_a_via_cluster and
trace_tension_can_move_a_real_rung_one_footprint_without_breaking_kicad.
A closed stdout (`pcb-maker --help | head`) ends quietly with exit 0 (or
the verdict's code when a run already had one).

## Committed on 2026-10-14 (benchmark tooling: short tier, route mode, splicing)

Florian's feedback: we spent most of the day waiting for CPU, because every
agent ended with a serialised full-budget 30-board tier and paired every
measurement. `benchmarks/agent-tasks/README.md` "How to measure" now states
the discipline: the short tier (`--tier short`, 12 flip-prone boards from
`flippers.py` over x644..x782, a third of the budget via `--budget-scale`,
about 600 s at 6 jobs; WLED_Matrix gets wall-cut above load 45 at that
budget) for iteration; the 30-board tier at full budget only as the merge
gate; quality judged by one run per side (the layout is deterministic unless
`cut`); pinned simultaneous pairs only for CPU claims; router questions in
route mode on fixed placements (`--route-from <sweep dir>`, verified
identical to manual routes on two boards); incremental sweeps with
`--splice-from <log>` (rows marked `spliced`, compare.py shows SPL); an
objectives line (unconnected, new errors, vias, length, CPU, work, cuts,
first-route open terminals) in the summary and summary.json.

## Committed on 2026-10-14 (DRC cache)

KiCad's DRC (`kicad-cli pcb drc`, 6-40 s of wall clock, about 20 s of CPU
and 2.2 GB per call) is now cached on disk (`crates/pcb-kicad/src/drc_cache.rs`,
hooked into `native_report::run_kicad_report`, so every DRC of a route,
layout, verify-kicad-rung and the reference check goes through it; ERC is
about 1 s and is not cached). Key: SHA-256 (sha2 is in the tree) of the
kicad-cli version, the DRC arguments, the board's file name (the report's
`source`), every KiCad design file under the board's directory by relative
path and bytes (`.kicad_pcb`, `.kicad_pro`, `.kicad_dru`, `.kicad_sch`,
`.kicad_sym`, `.kicad_mod`, library tables; hidden entries skipped), the
global `fp-lib-table`/`sym-lib-table` and the `KICAD*` environment.
`kicad_common.json` is left out on purpose: kicad-cli rewrites it on every
run. KiCad's installed libraries are not hashed (they feed only the
`lib_footprint_*` warnings, which never count). Value: KiCad's report
bytes; a hit is validated like a fresh report and published with a rename.
Entries are `<key>.json` in `$PCB_MAKER_CACHE_DIR/drc`, else
`$XDG_CACHE_HOME/pcb-maker/drc`, else `~/.cache/pcb-maker/drc`;
`PCB_MAKER_NO_CACHE=1` turns it off; temporary file plus rename; least
recently used out beyond 2000 entries or 2 GB (a hit touches its entry);
any cache failure falls back to running KiCad. Visible as `drc_cache:
{calls, hits}` in verdict.json (and the reports' `verdict`) and on the
`board check:` stderr line, which also gives the lowering and lattice
totals so far. Snapshot x801.

Correctness (verify-kicad-rung, fresh with `PCB_MAKER_NO_CACHE=1`, then a
miss, then a hit, each in its own copy of the project): on Castor's routed
board, spell_tome, fence-esp32 (with schematic parity), jetson and Hub the
hit's drc.json is byte-identical to the miss's and verification.json is
identical across all three. Fresh against miss (two fresh runs) differs on
jetson and Hub beyond the date: KiCad's DRC is not byte-deterministic (the
order of findings, and which item pair names a missing connection); the
counts per type agree. One byte in the board (a footprint coordinate) and
one byte in the `.kicad_pro` each missed and added an entry on all five.
Unit tests: key derivation (every input changes it, unrelated and hidden
files do not, length-prefixed fields), atomic writes under eight
concurrent writers, eviction; an integration test with a fake kicad-cli
(two DRCs of one board: one KiCad run, one entry, identical bytes; a
changed byte or argument misses; a corrupt entry is dropped).

Savings, x801. Per call (verify-kicad-rung): Castor 5.8 s wall / 21.8 s CPU
to 1.1 s / 1.1 s (the rest is the preview SVG export, uncached), jetson
17-23 s to 6 s. Castor route mode on its v15 placement: 2 DRC calls (the
result and the reference), cold 557 s wall / 598 s CPU, warm 489 s / 578 s,
same board and drc.json; the board check's KiCad 38.3 s to 0.7 s. Short
tier (4 jobs, 2 threads, `build/drccache/short-{cold,warm}.log`): 54 DRC
calls, warm 53 hits (WLED_Matrix's cold run was cut by the deadline, the
warm one not, so its final board differs); 11 of 12 results identical;
tier wall 873 to 683 s, CPU 7008 to 5167 s (load-sensitive), and the
layout's peak memory drops from 2.1-2.3 GB to its own (0.4-1.4 GB) where
every DRC hits. run.py's designer DRC calls kicad-cli itself and is not
cached.

Within one run (cold cache) almost no DRC is on a byte-identical board:
spell_tome layout 1 call, Castor layout 2, Castor route 2, jetson route 5
(4 attempts and the reference), 0 hits each; the short tier 54 calls, 1
hit (reCamera) and 2 identical boards checked side by side (51 entries).
v15 layouts make 0-19 DRC calls (median 2). The cache pays across runs:
reruns, A/B pairs on the same placement, the reference board.

Follow-up (x802): `pcb-maker drc-kicad-board <project-dir> <board-id>
<report.json> [--no-refill]` runs the cached DRC on the board and its
rules as the reference check does (`pcb_kicad::kicad_drc_report`; exit 0
no error that counts and nothing unconnected, 1 otherwise, 2 input, 4
KiCad failed; `tests/cli_exit_codes.rs` with a fake kicad-cli). run.py's
designer check and freerouting.py's designer and Freerouting-result checks
use it when `<binary> --help` lists it (probed once per binary; older
snapshots keep calling kicad-cli). Its report has every severity; the
designer counts (errors only, not excluded, not cosmetic) equal the old
`--severity-error` counts on qfwfq (546, at KiCad's 199 cap), WLED_Matrix,
USB_Keypad, Castor, 1337-v4.0 and MIDAS-MK1.1. run.py rows gain
`designer_drc_seconds`. Short-tier splice of spell_tome and Blue_Line
twice: the designer DRC 7.2 s to 0.4 s and 4.6 s to 0.4 s, the layouts'
own DRC hits too (CPU 100.5 to 91.9 s and 53.8 to 44.5 s), same results.

Lowering and lattice building (the second target): measured, not cached.
jetson route 7 lowerings 1.3 s and 7 router builds 15.4 s (summed over
threads) of 1558 s wall; Castor route 4 builds 4.3 s of 557 s; Castor
layout 22 lowerings 0.1 s, 4 builds 2.6 s of 904 s. Under 1.1 % everywhere,
below the 5 % bar.

## Committed on 2026-10-14 (exposed-pad thermal vias)

The largest class left by fill round two: 0xCB-1337's U3.45, ten
starved-thermal findings on six census boards the model had no row for
(thermal vias of the QFN's exposed pad, pads of its number merged into it,
which the model checked only where they are copper of their own). What
KiCad 10 does (zone_filler.cpp, drc_test_provider_zone_connections.cpp;
KiCad's filled polygons drawn per outline): every thermal pad
gets its relief, a hatched zone's ring (pad + gap to pad + gap + spoke
width, clipped by the zone outline only, then by other nets' clearances)
and four spokes from its centre, kept where their ends land in that fill
or meet another spoke, a pad inside a larger same-net pad included; the
thermal pads' plated holes are cut from the fill at the end. The DRC checks
each pad by its own ring half a gap out (crossings with fill outlines; an
outline over the larger pad holds both pads, no island) and its own
connected tracks (KiCad's connectivity: any track whose copper overlaps the
via, the exposed pad's tracks crossing it too). 0xCB's GND zone is hatched:
each via has a ring 0.4 to 0.7 mm and four spokes into it; the vias at the
exposed pad's corners lose their ring and outer spokes to the QFN pins'
clearances and keep the inward spoke: one, starved. In a solid zone the
larger pad's relief takes the vias' spokes and the DRC sees the larger
pad's own spokes crossing a via's ring (mackerel's mounting holes on In1:
2 on the spoke axes, 0 off them; solid there, so not in the census).

Modelled (`inner_pads`, on, `PCB_ROUTER_NO_INNER_PADS`): merged pads get
spoke records on every layer, reliefs and rings (`thermal_ring_pads`);
spokes of pads of one terminal meet; a landed spoke of a pad inside its
terminal's pad counts; foreign spokes across such a pad's ring are sampled
from their centre and count twice when they run through it; a merged pad's
DRC tracks are those overlapping its copper; no fill in a thermal pad's
plated hole. The earlier attempt's 15 false alarms came from the missing
rings (the vias' spokes found no fill), terminal-wide track ownership and
the track to the centre via's hole. Separately (`track_end_tolerance`, on,
`PCB_ROUTER_OLD_TRACK_ENDS`): the DRC's track-end test keeps other nets'
clearances to 0.5 microns instead of the spokes' 3 (`TRACK_END_TOLERANCE`);
four of the ten have a track from the corner via ending 1.5 microns outside
a sliver of fill between pins U3.23 and U3.24. Half a micron is KiCad's
DRC epsilon (`BOARD_DESIGN_SETTINGS::GetDRCEpsilon`, ADVANCED_CFG
`m_DRCEpsilon` 0.0005 mm, 500 nm read back in pcbnew), which its clearance
providers subtract from every clearance (`sub_e`); the zone-connection
check's point test uses none and the filler prunes with 1 micron, so the
number is KiCad's clearance tolerance, not that check's. Exact 12 missed /
29 false alarms, 0.5 microns 12 / 27, 1 micron 16 / 27, 3 microns 17 / 27.

Census, same tooling and KiCad cache (x801 = x820 = x768 before; x824
after, x823 before the rebase onto the DRC cache, identical):

| | x801 | x824 | x823 without inner_pads | x823 without track_end_tolerance |
|---|---|---|---|---|
| DRC starved predicted | 146 | 157 | 147 | 152 |
| missed | 13 | 12 | 12 | 17 |
| false alarms | 27 | 27 | 27 | 27 |
| starved pads not modelled | 12 | 2 | 12 | 2 |

By cause (`causes.py`, missed incl. not modelled): missing 12 to 2, tracks
4 to 3 (PixelWave's D45.4, the tolerance), the rest unchanged; false alarms
unchanged by cause. Every thermal pad inside a pad in the census (0xCB's 78
on six boards, Hub's U16.9_1/_2, Quanta75's U1.31; `pads_in_pads.py`,
mackerel's and PixelWave's are solid): 85 rows, 10 starved found, 75 agree,
none missed or false. Three router tests (hatched rings, the spoke through
a via's ring, merged pads on all layers).

Route mode on 0xCB's v15 placement, x801 against x824: identical boards
(47/47, 47 vias, 1952 mm, KiCad unconnected 0), starved_thermal 2 both (the
two corner vias, which the model now predicts), annular_width 13 (the
footprint's vias) both. The router does not repair them: `manual_spokes`
skips merged pads (no nodes of their own). A repair would be a short track
from the corner via inward into its own ring, which the DRC counts; not
done. Logs `build/census/x801.txt`, `x824*.txt` in the fill3 worktree;
`build/fill3-x801.log`, `build/fill3-x824.log`.

## Committed on 2026-10-14 (placer fixes, x792)

Three placer defects from the v15 triage (`build/triage*`).

**1. The work clock under-counted the placer** (`placer phase` lines now
print the thread's CPU time; the ratio is work over CPU). The legaliser
booked 3-15 % of its CPU time on the 0xCB panels, the annealer 25-50 %
on boards with many relations, against 100 % on keyboar_, so the
layout's wall guard fired and results followed the load. Profiled with
callgrind (instructions per counted unit; about 42 on keyboar_'s anneal,
the reference) and fixed by counting what is done (details and the table
in docs/placer.md): cutout polygons per edge visited (the panels' slots
have thousands of points), box lists and pairs as the annealer counts
them, the other part's box and the linked lookup per same-side test,
relation anchors (glasgow's capacitors, 23 supply pins each), pin
positions (3 units), held parts' pad offsets, edge and region scans.
Exact speed-ups too: one pass over a cutout with a box prefilter (the
pcb-panel's legalisation 224 s to 28 s of CPU a pass), pads' edge offsets
only for parts an edge holds. Ratios, pairs on the two threads of a core
(before / after, whole placement): 0xCB pcb-panel 0.11 / 0.95, 0xCB panel
0.11 / 0.77, glasgow 0.40 / 1.08, vaio_re 0.72 / 0.98, MIDAS-MK2 0.52 /
1.07, Castor 0.68 / 0.98, urchin 0.86 / 1.08, keyboar_ 1.02 / 1.02;
phases now 0.63-1.6 (refine and repair on the panel up to 2.2, global
1.5-2.3 as before) where they were 0.03-1.5. A seed does exactly what it
did: only where budgets stop moves. The layout's placement step on the
tier's 30 prepared tasks (16 seeds, x738 against x780): 29 identical;
MIDAS-MK1.1 keeps 5 seeds instead of 12 (its seeds booked 0.45 of their
CPU time, now 1.05: twelve were 2.4 times the budget), wirelength 2119 to
2255 mm, the same 11 unconnected in the tier. Rebased on the determinism
agent's work (x781 = x747 + this): the waves' work now counts each wave's
slowest seed at its real cost too. Layouts, one run a side (x791 =
main against x792 = main + this, four threads each, load 16-49): 0xCB
pcb-panel, x791 never finished placing (1800 s timeout, cut); x792 placed
in 329 s (170 s of work, 321 s more in waves), routed (177 open after the
first route) and ended on kicad-cli's 900 s zone refill (open problem 7,
no KiCad count). 0xCB panel, x791 placed in 1248 s for 34 s of work and
was cut twice (router deadline, no final ladder), 136/188 routed, 213
unconnected; x792 placed in 222 s, cut once (a router deadline, 14 s of
its work done, load 30), 152/188, 86 unconnected. glasgow, both
identical (112/226, 263 unconnected, no cut), placement 755 to 544 s of
wall. vaio_re, no cut either side, 163 to 138 unconnected (8 seeds
kept instead of 11).

**2. Tabbed panels** (`keep_copies`, on): copies told apart by their
nets (KiKit's `Board_N-` names) and repeated references, each with fixed
parts apart from the others', keep their movable parts in their copy's
region (cut halfway between the copies' fixed parts). The 0xCB
pcb-panel and panel (four copies each) qualify; no tier board does. A
repeated reference in a constraint now names its parts in turn (the
molecules' J3 had a left, a top and a bottom edge at once).
Parts whose nearest fixed part belongs to another copy, the 0xCB panel:
13-20 in v14, v15 and on x791, 0 on x792; pcb-panel on x792: 2 (C47 and
C1, next to a cut line). The PolyKybd molecules share their net names across
copies (one net group), so the copy rule cannot see them; with the
reference fix 5x4_wave places legally, the other four still fail: the
designer stacked alternative connectors (J3 on J5, J4 on J6, pad on pad)
on the one spot per tile the fixed switches leave, which our rules
cannot reproduce. OpenESC's 4in1 panel is one net group too (its error
is crowding); the 20x20 4in1-mini is a 4-in-1 board, not a panel.

**3. Far-side courtyards of parts on one side** (`far_courtyard`, always
on): the legality test and the annealer keep one part's courtyard on the
other side from the other part's far side (holes, pads, courtyard).
woagboard's back-mounted choc switches: 28 courtyard overlaps in the
placement to 0 (KiCad DRC on the placed board). Not the cause elsewhere:
koeg-board's 15 are the designer's own (fixed switches against netless
mounting holes, both where the designer put them; the designer's board
does not load in kicad-cli, exit 3, so the runner cannot subtract them);
Quanta75's 45-66 were the runner's: its mounting holes are all "M2.5",
some on GND, so the GND ones were stacked and then held at the stack by
the netless ones' "M2.5" in `fixed`. run.py now leaves a netted
footprint in place when a netless one shares its reference (Quanta75
both 0 overlaps placed; it also changes WLED_Matrix's LDO1 and Hub's
REF**). Layouts on x792 (one run each): woagboard 26 courtyard
overlaps and 25 unconnected on x791 to 0 and 8 (one items_not_allowed;
both runs cut by the wall clock at load 20-50); Quanta75 Stamp passes
(0 overlaps, 162/162; v15 45 overlaps), Quanta75 Bare 0 overlaps (v15
66), 126 unconnected, 8 starved thermals (cut). The layout's verdict
could name the parts a repeated reference or a copy region held when a
placement fails (`unplaced`, `next_actions`); not done.

Quick tier x782 (x769 + this) against x768, both on the tier's own task
list (github-tasks-v5): 29 of 30 rows identical (routed, vias, length,
KiCad findings); WLED_Matrix 4 to 5 unconnected, from run.py's change
(its netted LDO1 now stays where the designer put it); 23 passes both,
unconnected 25 to 26, vias 2814 to 2824, CPU geomean 0.94, no cut rows.
The same binary's placements on github-tasks-v6 (the 30 tier boards
prepared once, x738 against x780): 29 identical, MIDAS-MK1.1 as above.
Logs `build/quick30-x782.log` (v5), `build/quick30-x781.log` (v6, 23 of
30; framework_mobo passes on v6's constraints, which fix M1).

## Committed on 2026-10-14 (ladder reservation)

The layout's ladder now keeps work for its last step, the best attempt
again with signals at the project's narrowest predefined width (plus the
minimum clearance when the project rates clearance findings as warnings),
on projects where that step applies (`narrow_step_applies`: 43 of the 109
v6 tasks have a narrower menu width or a relaxed clearance). In layout
mode the ladder's work end is about its budget, so the regular rungs used
to spend everything and the step never ran (module, DSP, Hub, vaio_re in
v15). Once the ladder's first attempt has run with connections open,
`narrow_reserve_share` (0.75) of that attempt's work, at least 120 s and
at most 0.4 of the ladder's work and 600 s, is held back: every later
rung, the seed retries and the seed rounds check the work left without it.
The step runs when half the reservation and at least 180 s are left (the
floor: on jetson, with 70 s left, a 60 s negotiation took 232 s and was
useless), negotiates a third of what is left with its router's work
deadline at what is left (it had no work budget when it varied a probed
rung, so it ran to the wall clock: DSP 619 open in v15), its work counts
in `work_seconds`, and `skipped` says why it did not run. Route mode and
projects without the step are unchanged (no reservation: the code paths
are gated on it). A limit on how many rungs run side by side was built
and removed: the rungs are judged in order either way, so it only
serialised waves and cost wall clock (Telemetry).

Layout runs, one per side, pinned (A 0-7,16-23, B 8-15,24-31, 4 threads
each); unconnected is KiCad's count, errors KiCad's; "step" = whether the
narrow step ran (router's open count after it):

| board | base / ours | unconnected | errors | vias | CPU ours/base | step base / ours |
| --- | --- | --- | --- | --- | --- | --- |
| A13 module | x747 / x772 | 81 -> 18 | 1 / 1 (pre-existing) | 414 / 404 | 0.86 | no / yes (37) |
| vaio_re | x769 / x773 | 163 -> 73 | 0 / 0 | 604 / 572 | 1.02 | no / yes (165) |
| DSP-ADAU1452 | x791 / x774 | 129 -> 108 | 0 / 5 starved thermals | 188 / 185 | 1.07 | no / yes (230) |
| Hub | x791 / x774 | 30 / 30 | 0 / 0 | 187 / 187 | 0.90 | no / yes (176, not kept) |
| MokyaLora (passes) | x791 / x774 | 0 / 0 | 0 / 0 | 334 / 334 | 0.96 | yes / yes (bounded: 157 s) |
| TelemetryOnboard | x791 / x774 | 1 -> 0 (pass) | 0 / 0 | 102 / 108 | 0.93 | no / yes (0); both wall-cut |
| jetson | x769 / x773 | 25 / 25, identical | 1 / 1 | 782 / 782 | 0.96 | no ladder (first route 1283 of 1500 s) |
| ghoul | x791 / x775 | 9 / 9, identical | 1 / 1 | 604 / 604 | 1.00 | step blocked: wall cut in placement |
| OpenFC (no step) | x791 / x775 | 1 / 1, identical fingerprints | 0 / 0 | 159 / 159 | 1.07 | - |
| protoesp (no step) | x791 / x775 | 92 / 92, identical | 2 / 2 | 295 / 295 | 1.00 | - |

The bases moved during the work (determinism, fill round two, exit codes);
each pair has both sides on the same base. Earlier pairs on x738 (before
the determinism change): module 81 -> 13, vaio_re 101 -> 21, Hub 35 -> 29,
DSP and koeg unchanged. Jetson cannot be helped by the ladder: its first
route takes the layout's budget (in route mode the step takes it from 48
to 6 open); a narrow first route when the project has the step is the
lever there, untested. Narrow is not a better default for the ladder's
first attempt: on designer placements (route-v12) the step beat the whole
class-width ladder on 9 of 16 boards (DSP 114 to 12, jetson 29 to 8,
vaio_re 36 to 16, Telemetry 8 to 0) and lost on 5 by 2-28 open, and in
layout v15 it lost on koeg (164 against 134) and Explorer (8 against 4).
The narrow step's DRC risk: the widths are the project's own menu (at or
above its minimum width), so KiCad raises no width finding; DSP's 5 new
errors are starved thermals of the narrow attempt's pours. Quick tier
x776 (this change on b2f5fbf, run on github-tasks-v6): no tier board
reserved or ran the step (none prints the reservation line). Its first
comparison, against build/quick30-x768.log, was not one: x768 ran on the
tier's default list then, github-tasks-v5, and v5 and v6 differ in the
constraints of 12 of the 30 boards (v6 fixes more parts: Castor J2 at the
designer's place instead of an edge constraint, BAGEL U102, USB_Keypad
and WLED_Matrix M1-M4, ...). Exactly those 12 rows changed (BAGEL 319 to
360 vias, Castor 230 to 249, USB_Keypad 81 to 100, WLED_Matrix's
connection_width 2 to 11; framework_mobo_lefthalf passes on v6); the 18
boards with the same constraints gave identical rows (1738 vias, 56520 mm
both). Castor swapped: x776 on v5 gives x768's row (230 vias, 4746.5 mm),
x768 on v6 gives x776's (249, 5601.1), same ladder and via-reduction lines
(`build/via-bisect/`). No commit between x768 and b2f5fbf changed routing:
the `spent` change of the exit codes only acts without a deadline (route
mode), the DRC cache only adds counters. The reservation's tier
comparison is x776 against a v6 base, build/quick30-x783.log (pending:
not finished when this was written). Logs `build/ladder-reserve/`
(summary.py there), `build/quick30-x776.log`.

## Committed on 2026-10-15 (clean-up work accounting, x814)

The determinism agent left the clean-up's pour checks out of the work:
the pour analyses that keep a batch from stranding pour pads ran on wall
clock (Sisu: 21-47 s of pour checks in a 61-66 s budget; search work alone
covered 52-80 % of the clean-up's wall time), so a busy machine
stretched the clean-up and the route-mode guard (4 x the budget) could stop
it. Now every pour analysis counts as work in the phase budgets
(`budget_work` behind `clock` and `spent`: resolve, clean-up, via
reduction, thermal spokes): 41 ns per lattice node inside the net's pour,
38 ns per stamp of the net, 47 ns per item the fill probe indexes or looks
at (the spoke tests; `FillProbe` counts them). Calibration (route mode, one
thread, `PCB_ROUTER_POUR_TRACE`, seven boards): per board the model gives
0.89-1.07 of the analyses' wall time at load 17-22, 0.70-1.10 at 21-25
(x814, Castor and reCamera lowest); a lattice-only model spread 0.5-1.65
(the spoke tests are a third to two thirds of the time on reCamera and
Castor). At one thread the clean-up's counted work is now 0.98 of its wall
time on Castor (0.64 before). The verbose log has a `profile work:` line
(wall, search work, pour wall and work per phase) and the clean-up's last
line gives its work, its budget and the pour share. What stays uncounted
is in docs/router.md, "Determinism and the wall clock" (the negotiation's
stamping, the ladder's and the layout's tallies, `deadline_work`).

Route mode on v15 placements, x777 (main) against x814 side by side, two
jobs, load 29-35, one run each:

| | before | after |
|---|---|---|
| Sisu result | 7 unconnected, 408 vias, 5924.8 mm | identical (the winning attempt, signals at 0.1 mm on pours=tracks, has no pour checks) |
| Sisu clean-ups on the pour rungs | 404 / 308 / 234 analyses | 404 / 290 / 220; work 60.1 of 63, 69.3 of 66, 60.9 of 61 s in 80.9 / 80.5 / 61.8 s wall (work/wall 0.74-0.99) |
| Castor result | 0 unconnected, 3 starved (allowed 3, pass), 225 vias, 5186.2 mm | 0 unconnected, 4 starved (fail by one), 209 vias, 5156.1 mm |
| Castor clean-ups | first one finished its passes | each ends by work (60.1 of 60 s in 82.7 s wall, 60.3 of 60 in 59.6, 68.8 of 71 in 60.9) |

No wall guard fired on either side. Castor's change is the budget meaning
less work than before: the first clean-up stops earlier, the via
reduction starts elsewhere (16 vias fewer) and one pad more is starved;
a flip of the kind the short tier holds, not a fault of the accounting.

Short tier x814 (load 9 at the start, 7 at the end) against
build/bench-short/short-x783.log (both tasks v6): 12 of 12 the same by
compare.py (better 0, same 12, worse 0; unconnected 42 to 42, passes 6 to
6), 11 rows identical in vias, length and work. The one that moved,
1337-v4.0 (passes on both): vias 64 to 76, length 1594 to 1611 mm, work
169 to 130 s. Its first polish's via reduction (strict, 60 s) now counts
the pour analyses of its renegotiations and clean-ups, so after two rounds
it stops ("round 2 would take about 24s of the 15s left of 60s") where it
ran three (71 to 69 vias); the second polish starts from that board. Tests:
`cargo test --release -p pcb-router -p pcb-kicad` pass (new:
`pour_analyses_count_as_budget_work`). Logs: build/cleanup-x777,
build/cleanup-x814 (route mode), build/short-x814.log, calibration
build/cleanup-cal3-x813 and build/cleanup-val-x814.

Recalibrated 2026-10-15 (x817) after the router-overhead round (x872)
made the pour analysis about twice as fast (the old constants booked
Castor's clean-up 9.7 s of pour work for 5.2 s of pour checks, Sisu's
25.6 for 16.3): `POUR_NODE_NANOS` 41 to 18, `POUR_STAMP_NANOS` 38 to 26,
`PROBE_ITEM_NANOS` 47 to 42, fitted over three calibration runs at one
thread (load 15-31; build/cleanup-cal-x816, -x816b; the trace now also
prints the pour layers' nodes, which did not help the fit). Per board
0.71-1.08 of the analyses' wall time by the median, the big boards
(Castor, Sisu, reCamera, k30) 0.85-1.08; one board's ratio moves 0.6-1.2
between runs with the load, so 0.9-1.1 per board is beyond what one
calibration run can show. Route mode, x872 against x817 side by side (two
jobs, load 15-18): Sisu identical (7 unconnected, 408 vias, 5924.8 mm);
Castor back to its pre-accounting board (0 unconnected, 3 starved, pass,
225 vias, 5185.8 mm against x872's 209 vias, 5157.5 mm, also a pass now).
The clean-ups' pour work against their pour checks' wall time (four
threads): Castor 5.8 / 5.5 / 5.9 s for 5.1 / 4.3 / 4.9 s (x872: 9.7 / 9.0 /
9.6), Sisu 12.5 / 10.4 / 10.8 for 16.4 / 12.1 / 8.8 (x872: 25.6 / 20.1 /
19.0); each clean-up ends by work, no wall guard. Short tier x817 (load 17
to 10) against build/short-x880.log (tasks v6): all 12 rows identical
(vias, length, work, KiCad), 1337-v4.0 included.

Next: the ladder's and the layout's work tallies still count search
expansions only; counting the pour work there too (a `work()` in
`RoutingResult` instead of `expansions`) would make `deadline_work` and the
ladder's budget consistent with the phase budgets, at the price of shifting
more rows (every pour board's attempts), so it wants its own measurement.
## Committed on 2026-10-14 (inner-pad spokes)

The exposed-pad model (above) found 0xCB-1337's two starved corner vias
but the router could not repair them: `manual_spokes` starts its tracks
from a pad's lattice nodes, and a via merged into the exposed pad has none.
`add_inner_spokes` (`inner_spokes`, on, `PCB_ROUTER_NO_INNER_SPOKES`; after
the manual spokes) gives such a via, where the spoke model finds it short
on a layer inside the exposed pad, a straight track of the net's width from
its centre into fill over the pad (in a hatched zone its own ring), its
copper wholly inside the exposed pad's, the shortest of sixteen directions,
kept only if the model then counts the via whole. KiCad's DRC counts a
track connected to the pad that starts in its ring and ends in fill as a
spoke; inside the exposed pad's copper the track keeps every clearance the
pad keeps. The tracks are exact copper (`NetState::inner_spokes`), counted
by the DRC track rule in `kicad_spokes`. Checked with kicad-cli on the
x825 board plus the two tracks before building it in: starved 2 to 0, no
other finding.

One trap: the tracks rung lowers pours with `connect` false, and the fill
probe read a zone's hatch only from connecting planes, so in that rung the
vias' rings did not exist and the model saw them unconnected (no finding,
no repair), while the ladder picked that rung's board. While
`add_inner_spokes` runs the probe now reads the hatch of any plane
(`Router::hatch_any_plane`); reading it in the tracks rung everywhere
changed 0xCB's tracks-rung routing (46 vias, MX3.S1 left an isolated
island), so it stays scoped. The tracks rung's fill model ignoring hatched
zones is a gap of its own.

Route mode on 0xCB's v15 placement (tasks v6), x825 against x828: the
same board plus two segments of 0.42 mm (length 1952.3 to 1953.2 mm),
starved_thermal 2 to 0, no new finding (annular_width 13 is the
footprint's vias), CPU 160 to 138 s. Census of 44 boards: x828 identical
to x825 (157 predicted, 12 missed, 27 false alarms, 2 not modelled). Short
tier (tasks v6) x829 against build/bench-short/short-x783.log: 11 boards
byte-identical, 0xCB-1337 the same board plus the two segments, its
starved_thermal 2 to 0 (it passed before on the designer's 3); passes 6 to
6, unconnected 42 to 42, vias 1165 to 1165, length 22013 to 22014 mm, CPU
4920 to 4951 s. An earlier tier against short-x791 was tasks v5 (the
list mix-up) and is void; WLED_Matrix on it was identical between x825
and x828. Router test:
`a_starved_thermal_via_inside_an_exposed_pad_gets_a_track_into_its_ring`.
Logs `build/fill3b-x8{25,27,28}.log`, `build/short-x829.log`.

## Committed on 2026-10-14 (placement pool)

The placement race ran its seeds in waves as wide as the machine's cores
(`available_parallelism`, 32 here whatever `RAYON_NUM_THREADS` said), each
wave as long as its slowest seed, and the waves' work counted toward the
layout's work end (`waves_work_seconds`). Now (`race_seeds` in
board_placer.rs) the seeds run in a pool of the process's search threads
(`RAYON_NUM_THREADS`), a seed starting as a thread frees; they are judged
in seed order as one after the other (seed k kept while the seeds before it
used less than the budget, three at least; ties to the earlier seed), as
the probe ladder judges its probes. The batches they start in are sized
from the kept seeds' work alone (the first holds the three, each later one
as many as the budget left holds at their mean work), so the seeds run and
the seed kept do not depend on the threads. A seed the budget would not
keep no longer runs: the waves ran one more seed after the budget was
spent and counted it (glasgow 260 s, the 0xCB pcb-panel 171 s, MIDAS-MK1.1
167 s of work for nothing). The clock counts the kept seeds as the
reference machine (eight threads, the calibration of
`EXPANSIONS_PER_SECOND`) runs them per batch (`pool_work_seconds`,
replacing `waves_work_seconds`); `board-placer.json` adds `seeds_run`,
`seed_kept` and `seeds_work_seconds` (their sum), and the layout's
`layout: placed` line prints them.

Six boards, one layout a side (x783 = main before, x830 = x783's tree plus
this; four threads, tasks v6, `PCB_ROUTER_NO_WALL_GUARD=1` on both so the
work alone decides, load 18-48; `build/pool/six-x7xx`). Wall and work in
seconds; work is the longest seed; the clock is what the layout counted:

| Board | placement wall | work | clock (waves / pool) | sum of kept seeds | kept / run | seed kept | final board |
| --- | --- | --- | --- | --- | --- | --- | --- |
| video | 421 / 457 | 439 | 567 / 567 | 1026 | 10 / 11 | 2 | identical, 263/371, 1238 vias |
| 0xCB pcb-panel | 471 / 314 | 170 | 491 / 320 | 652 | 4 / 4 | 4 | identical; both end on kicad-cli's 900 s zone refill (exit 4) |
| glasgow | 501 / 305 | 334 | 593 / 334 | 816 | 3 / 3 | 1 | identical, 112/226, 201 vias |
| Castor | 29 / 54 | 9.8 | 19 / 28 | 145 | 16 / 16 | 1 | identical, pass |
| USB_Keypad | 11 / 23 | 5.1 | 9.8 / 13.5 | 71 | 16 / 16 | 14 | identical |
| urchin | 8.8 / 13.8 | 4.7 | 8.4 / 10.7 | 55 | 16 / 16 | 6 | identical, pass |

The placements (poses) are identical on all six, so the kept seed is
x783's too; the result boards are byte-identical. glasgow's first route
negotiated with 467 s of work allowed instead of 337 s and took another
path (its fingerprints differ until the final ladder), but ended on the
same board. The placement's wall time follows the threads now: x783 ran up
to 13 seeds at once per board whatever the search threads, x830 four in
the tier, so boards of many short seeds place 1.5-3 times longer in wall
time (Castor 29 to 54 s, the same CPU); boards that ran a wasted seed are
faster. Placement alone with `RAYON_NUM_THREADS` 4, 8 and 16 on x830
(`build/pool/rayon`): seeds run, kept seed, clock and poses identical on
all six boards (walls 413/316/402 s video, 43/25/16 Castor, 308/286/274
the pcb-panel, 272/291/294 glasgow, 16/12/9 USB_Keypad, 10/7.5/6.4
urchin; the machine was loaded).

Short tier (x831 = main 6b3ced6 plus this, `build/short-x831.log`, against
`build/bench-short/short-x783.log`, both tasks v6): all 12 boards same,
every placement and every result board byte-identical; unconnected 42,
errors 6, vias 1165, length 22013 mm on both; work 2339 to 2428 s, CPU
4920 to 5127 s. The clock moved on every board (16-seed boards 1-5 s
more: the waves were 32 wide, the reference pool 8; less where a wave
waited on one slow seed or ran a wasted one: WLED_Matrix 177 to 113 s,
MIDAS-MK1.1 300 to 133 s, reCamera 75 to 52 s), and the work
left went into routing that did not win: WLED_Matrix's ladder ran its
two fine-pitch rungs (23 and 83 open against the kept 15; row work 282 to
358 s), MIDAS-MK1.1's first route negotiated 137 s instead of 53 s (334 to
347 s), reCamera's probe ladder ran where x783 had "deadline too near"
(its rungs lost). CPU: the pool runs the same seeds or fewer, so the
extra CPU is that routing; one pinned pair on reCamera
(`PCB_MAKER_NO_CACHE=1`, `build/pool/pair-x7xx`, load 40-58): 937 to 1061
s, the ladder's 4 probes and 3 rungs, identical row (55/56, 72 vias).
Tests: `placement_seeds_are_judged_in_seed_order_whatever_the_threads`
(threads 1-16, seeds finishing in reverse order, the sequential reference).

## Committed on 2026-10-15 (hatched zones in the tracks rung)

The tracks rung lowers its pours with `connect` false, and the fill probe
read a zone's hatch (and the outline edges) only from connecting pours, so
there hatched zones were modelled solid: the spoke verdicts, manual
spokes, island vias and stubs (only `add_inner_spokes` read the hatch,
through `hatch_any_plane`). The probe now reads every zone of the net as
KiCad fills it (`FillProbe::filled_planes`; `tracks_rung_fill`, on,
`PCB_ROUTER_OLD_TRACKS_FILL`): hatch, rings, hole grid and outline in
every rung. In the tracks rung the probe is used only after the routing
(the manual-spoke pass and its island repairs; the via reduction's starved
count is the corridor model), so the routing itself never saw the hatch.

Why reading the hatch alone hurt 0xCB (x860, route mode on v15: tracks
rung 46 vias, KiCad "2 spokes connected to isolated island" at MX3.S1 on
B.Cu): not the island rule and not the routing. MX3.S1 (a 2 mm PTH pad)
has two F.Cu tracks; KiCad's DRC counts a track connected to the pad that
starts in its ring and ends in this layer's fill, and one of them,
112.07 to 118.37 mm, ends in B.Cu fill. Our model tests a pad's straight
lattice run to its turn, which ran past that end (a junction: another GND
branch leaves it there, and `materialize` cuts the segment) to 98.77 mm,
outside the fill; the other run ends at 131.07 mm, 12 microns inside a
hatch hole (pcbnew agrees: not in fill). With the solid model the second
end was "in fill", so the model counted one track, found the pad short
with two island spokes, and added an island via; with the hatch it counted
no track, and with no track KiCad's rule lets the island spokes stand in
(spokes_short 0: no repair). KiCad counted the first track (1 counted, the
islands then ignored): starved. Fix: the DRC track rule cuts the run at
the first junction as the emitter does (`junction_nodes`;
`track_end_junctions`, on, `PCB_ROUTER_OLD_TRACK_RUNS`). The census
cannot see either change (it lowers with the pours connected and adopts
the board's exact segments). Router tests:
`the_tracks_rung_reads_a_hatched_zone_as_hatched`,
`a_track_ends_for_the_drc_at_a_junction`.

Hatched boards (`(fill ... (mode hatch`): 0xCB-1337 1337-v4.0, pcb, panel,
pcb-panel; Business-Cards Batch_1, USB_Keypad, WLED_Matrix; capybully;
le_chiffre stm32_chiffre_36keys and stm32_hotswap_chiffre; Qfwfq; the A13
module. Route mode on the v15 placements (tasks v6, `--jobs 1 --threads
4`), x832 (= main's router) against x861, `build/hatch/route-x8{32,61}.log`:
pcb-panel has no v15 placement and panel times out at 1800 s on both
sides; on the other 10 the same rung wins everywhere, unconnected 518 to
518, errors 14 to 14, starved thermals the same per board (Batch_1 7,
the rest 0), vias 1428 to 1427 (USB_Keypad 100 to 99), length 37025 to
37022 mm. 0xCB 1337-v4.0: tracks rung, 0 open, 0 starved, 47 vias on both,
MX3.S1's island via kept, and two manual spokes (B.Cu, four segments) the
hatched model no longer needs are gone (1953.2 to 1949.2 mm); capybully
-1.1 mm (two manual spokes fewer), Batch_1 +2.3 mm. Census of 44 boards:
x832, x860 and x862 identical (157 predicted, 12 missed, 27 false alarms,
2 not modelled), `build/hatch/census-x8*.txt`.

Short tier (tasks v6) x862 (this, rebased on main 9276fba) against
x833h (main's x833, run now as the base: short-x829 predates the work
accounting and the placement pool, which move 0xCB 1337-v4.0 to 76 vias
and 1611.6 mm on main itself), `build/short-x833h.log`,
`build/short-x862.log`: 10 of 12 boards byte-identical (placement and
result); USB_Keypad 100 to 99 vias, WLED_Matrix 2457.8 to 2459.2 mm (its
tracks rung adds 11 manual spokes, not 8; the model on x833h's board had
2 false alarms there), both 0 starved in KiCad on both sides; unconnected
42 to 42, errors 6 to 6, passes 6 to 6, vias 1177 to 1176, length 22030
to 22032 mm; OpenFC cut by the wall clock on both sides.

## Committed on 2026-10-15 (first-route budget)

Jetson ended layout mode at 25 unconnected with no ladder at all. Where
its work went, x833 (four threads, `profile work:` lines, work seconds):

| phase | work |
| --- | --- |
| placement (longest seed; the pool's 79 s count toward the end only) | 52 |
| race: the two other placements' probes | 150 |
| first route: kept probe 75 + negotiation 478 (cap 556) | 553 |
| first route: hard repair 40, stitching and pour analyses 6 | 46 |
| move C33 (rejected, 39 open): negotiation 84, repair 37, stitching 3 | 124 |
| move C33 turned (kept, 30 open): negotiation 120, repair 31, stitching 2 | 152 |
| resume (82 s, 30 to 25 open): negotiation 83, repair 27, stitching 2 | 112 |
| clean-up, via reduction, spokes | 0 (polish needs 240 s, 218 left) |
| ladder | none (needs 240 s, 218 left) |

1203 s of work plus 79 s of pool: 1282 of 1500 (x832 before the pool:
1222 plus 46 of waves, wall-cut in the polish). The first route's
negotiation was not the problem (it stalled at 478 of 556 s); the moves
and the resume were: jetson's 31 open sat under the "many open" limit (5 %
of its terminals), so it was treated as nearly complete. On the boards
over that limit (module, DSP-ADAU1452, vaio_re) the ladder ran but its
probes and continued tracks rung took 900-1000 s, the reserved
narrow-signal step got 135-156 s or nothing, and no class-width attempt
beat the first route (x833: module 82, DSP 121, vaio_re 138).

Changed (crates/pcb-kicad/src/board_layout.rs): where the narrow-signal
step applies (`narrow_step_applies`), a board on four layers with pours
counts as far from complete above 2 % of its terminals (at least 10;
`far_open_limit`) as well as above the old 5 %; such a board skips the
moves, the resume and the polish, and instead of the ladder gets one
attempt of the step alone (`narrow_step_config`: signals at the narrowest
menu width, the minimum clearance where clearance findings are warnings;
the first route's pour mode, no finer pitches) with the work left: the
clean-up capped at a tenth of it (30-120 s, new `cleanup_seconds` in the
router config, not read from files), the negotiation three quarters of
the rest (60-900 s; `direct_narrow_budget`), its work deadline the
layout's end; the layout keeps it when KiCad finds it better.
`PCB_LAYOUT_OLD_FIRST_ROUTE_BUDGET` restores the old split. Unit tests
for the limits and the split. Boards without the step, and boards near
completion, take the old paths.

Layout runs, one per side, four threads each, unpinned, load 22-41;
before x833, after x855 (x855 is this on x862; x833 and x862 have the same
budget code, x862 changes hatched zones; the jetson, module, DSP and
vaio_re rows of x855 equal those of the x851/x854 runs built on x833).
Unconnected and errors are KiCad's (new errors against the designer's):

| board | unconnected | errors | vias | before: steps | after: steps |
| --- | --- | --- | --- | --- | --- |
| jetson | 25 -> 18 | 1 clearance -> 8 hole_clearance | 772 -> 710 | moves, resume, no polish, no ladder | narrow attempt (408 s negotiation, router 36 open) |
| A13 module | 82 -> 17 | 0 -> 1 track_dangling | 424 -> 490 | ladder, step skipped (154 s left of 319 kept) | narrow attempt (stalled at 103 s), a seed round |
| DSP-ADAU1452 | 121 -> 82 | 5 starved -> 0 | 186 -> 225 | ladder, step at 156 s: 258 | narrow attempt 169, four seeds 207 |
| vaio_re | 138 -> 35 | 0 / 0 | 551 -> 686 | ladder, step at 135 s: 530 | narrow attempt 76, one seed |
| Castor | 0 / 0 | 0 / 0 | 249 -> 167 | first route, moves, tracks rung | same; 167 is x862's (hatched zones; x862 alone gives 167) |
| keyboar_ | 0 / 0 | 0 / 0 | 39 / 39 | first route, moves | same |

No row was cut. Jetson's 8 hole_clearance findings are one via of
3V3_SYS 0.2438 mm from J2's PTH pad 1 hole (0.25 required; KiCad lists
it 8 times): a router hole-clearance rounding to look at, not the narrow
width. Two more boards with the step, on x833 against x854 with the same
rule (`PCB_LAYOUT_DIRECT_NARROW=always`, an experiment switch not kept):
Hub 30 -> 30, 187 -> 170 vias, 1378 -> 577 s of work; koeg 12 -> 7
unconnected, shorting_items 2 -> 5 (15 courtyard overlaps on both, both
runs wall-cut under load).

Variants measured on the way (jetson unless said): the narrow attempt with
half of what is left and the default 180 s clean-up (x850): 23, its
clean-up took 186 of 604 s; three quarters after a capped clean-up
(x851): 18, the negotiation still falling (21 nets in conflict); 0.85
(x852): 21, a plateau at 21 and another end; the step alone only when the
ladder could not afford probes plus two attempts of the first route's cost
(x851): module took it (17), DSP and vaio_re took the ladder and stayed at
121 and 138, which is why the rule now has no such threshold. A cap on the
first route itself was not needed on jetson (its negotiation stalled
under its cap) and would risk boards that complete in it.

Short tier x855 against x862 (same tasks, v6): all 12 rows identical, 42
unconnected, 6 errors, 1176 vias; against build/bench-short/short-x783.log
only 1337-v4.0's vias differ (64 -> 76, the recorded router change). No
short-tier board has the step, so none takes the new path. Logs in the
worktree's build/ (pcb-maker-jetson: six-x833, six-x851, a-x855, b-x855,
always-x854, extra-x833, extra-x854-always, jetson-x832, jetson-x852,
castor-x862), short tier build/short-x855.log.

Left: the narrow attempt does not use all the work on every board (Hub
ended at 577 of 1500 s): a class-width ladder after it with what is left;
a mode choice for the attempt (the first route's pour mode now; tracks
probed better on module, koeg and DSP); the via hole-clearance rounding
(not rounding: "hole clearance" below).

## Committed on 2026-10-15 (hole clearance)

Jetson's 8 hole_clearance findings (x855 layout) are one 3V3_SYS via at
(133.219, 122.186), 0.769 from the centre of J2's PTH pad 1 (GND; 0.75
copper, 0.55 drill). KiCad measures the via's copper (0.25 radius) to the
pad's hole (0.275 radius): 0.769 - 0.25 - 0.275 = 0.2438 where the
project's `min_hole_clearance` is 0.25; it lists the pair 8 times. To the
pad's copper the via kept 0.144 (0.125 needed), hole to hole 0.419 (0.25).
Not lattice rounding (the via sits on a 0.127 node, the pad off the
lattice; 0.775 needed, 0.75 kept) and not the narrow width: since b6e3530
a routed pad's plated hole was a hole obstacle of its net that blocked
other nets' tracks but not their vias ("vias stay with the pad copper
rule"), so vias kept only clearance plus copper, which with a 0.1 ring
and 0.125 clearance is 0.025 short of the hole clearance. The verifier
skipped it too (`blocks_vias` false), hence 0 internal violations.

Fix (crates/pcb-kicad/src/board_router.rs, crates/pcb-router/src/grid.rs):
the plated hole blocks vias; the static via map exempts the pad's own net
through `via_owner` where only the hole reaches (its copper still blocks
the own net as before). KiCad does not test a net's own copper against its
holes (every track to a THT pad ends on its drill; none of the 735
hole_clearance findings with a via or track in recent drc.json files pairs
items of one net). The verifier now checks other nets' vias against these
holes. Unit test `other_nets_vias_keep_the_hole_clearance_from_a_plated_hole`
(J2's geometry: lowering, static map for both nets, verifier 0.2439
against 0.25); it fails on x855 at the lowering and without the grid
change at the own-net exemption.

How common: the only finding of this kind in our routers' output. Of 3269
drc.json files of the last six days under build/ and the worktrees
(references and Freerouting left out), hole_clearance findings with a via,
track or zone are this via alone (jetson's x850, x851, x852 and x855
layouts). build/github-layout-v15 (100 boards) and v16 (39 so far): 20 and
9 boards with hole_clearance, every one footprint-intrinsic (pad to NPTH
of the same part, pad holes within J2, MX switch pins), none new against
the designer (v15's rows: no new hole_clearance).

Jetson in route mode, the a-x855 placement, the layout's narrow attempt
(`use_narrow_signals`, pours connect, no refine pitches, no retry seeds,
four threads; clean-up at its default 180 s, the 60 s cap is not a file
setting), three negotiation budgets per side since the result is chaotic
(identical up to iteration 48 at 405 s, where the via first differs):

| negotiation | x855 unconnected | x855 via findings | x880 unconnected | x880 via findings |
| --- | --- | --- | --- | --- |
| 380 s | 18 | 8 | 17 | 0 |
| 405 s | 11 | 8 | 16 | 0 |
| 430 s | 19 | 0 | 15 | 0 |

Sum 48 against 48; the other findings equal on all six (clearance 2,
courtyards_overlap 1, starved_thermal 4, the designer's 36
hole_clearance). Short tier x880 against build/short-x855.log (tasks v6):
all 12 rows identical (42 unconnected, 6 errors, 1176 vias, 22032 mm,
work 2390 s); none of its boards had a via in that ring.
Logs: build/repro in the hole-clearance worktree, build/short-x880.log.

## Freerouting refresh on x738 (2026-10-15, no code changed)

docs/benchmarks.md "Freerouting against our cold route, x738". Same picture
as the 2026-10-09 round (47 / 6 fewer unconnected, 51 / 8 clean where both
finish, 0.65x Freerouting's CPU), no board flipped sides. Our cold route
regressed on 11 boards since x136/x155: SNSP-CPU-01 55 to 178 open (all
GND; 362 of 363 connections "routed" while KiCad finds 178 unconnected
terminals: our connectivity and KiCad's disagree) and MegaDrive 95 to 137
repeat on a calm machine; katia hits the 1800 s wall at 14.9 GB. Assigned
to a cold-route regression agent (x900+). Fairness: Freerouting timeouts
above load 45 rerun, eight boards rerun Freerouting-only, 4 GB heap shown
harmless by A/B, keyboar_ is a parser hang (backtick in a part number), not
a timeout.

## Committed on 2026-10-15 (router overhead, x870 / x871 / x872)

Speed round four, unlike the three before it, changed no search: it looked
for CPU that buys no search and removed it with routes byte-identical.
Profiled route mode on the v15 placements of Castor, jetson-nano-baseboard
and k30-SBC (x862's code with debug symbols, four threads as the pairs run
them; `perf` is not installed and samply needs `perf_event_paranoid` 1, so
gperftools' `libprofiler.so` by `LD_PRELOAD` with `CPUPROFILE_USE_PID`,
200 Hz, libunwind stacks; tools and profiles in the worktree
`pcb-maker-speed`, `build/prof`). Share of the run's CPU (kicad-cli's DRC
included), Castor / jetson / k30:

| group | Castor | jetson | k30 |
|---|---|---|---|
| A* search proper (expansions, key heap, occupancy reads, heuristic) | 69.8 | 80.5 | 77.6 |
| the search's neck class lookup (`class_at` per step) | 2.2 | 1.2 | 0.0 |
| tile planner (`plan_corridor`: Dijkstra, `BinaryHeap`) | 3.0 | 5.9 | 8.8 |
| stamping and conflict bookkeeping | 6.8 | 3.0 | 4.1 |
| pour refresh and fill probes, negotiation | 3.3 | 0.5 | 3.5 |
| pour checks in clean-up and via reduction | 5.9 | 1.5 | 0.0 |
| lowering and lattices | 0.6 | 0.5 | 0.1 |
| KiCad (kicad-cli DRC) | 3.1 | 3.9 | 2.8 |
| allocation, memset, memcpy | 2.2 | 1.4 | 1.4 |
| SipHash | 1.8 | 0.4 | 0.8 |
| other | 1.3 | 1.1 | 0.8 |

Top symbols by self time (inlined callees included), Castor / jetson /
k30: `Router::search` 65.8 / 77.2 / 72.7 (its own lines: `KeyHeap::pop`
9.7 / 12.5 / 10.6, `occupancy_seen` 8.1 / 10.7 / 9.9), the search's
heuristic closure 4.6 / 3.6 / 5.6, `stamp`'s apply closure 4.9 / 1.8 /
2.9, `plan_corridor`'s entry-cost closure 1.4 / 2.8 / 3.9, `route_net`
(the planner inlined) 1.0 / 1.9 / 2.8, `PourMap::build` 2.6 / 0.6 / 1.0,
the planner's `BinaryHeap::pop` 0.7 / 1.4 / 2.2, `class_at` 2.6 / 1.4 /
0.0, `RandomState::hash_one` 1.9 / 0.4 / 0.8, `unstamp` 1.5 / 0.8 / 0.8,
`OwnStamps::contains` 1.8 / 0.4 / 0.9, SipHash `write` 1.1 / 0.3 / 0.4,
`Shape::distance_to_point` 1.1 / 0.0 / 0.2, `FillProbe::blocker` 0.6 /
0.0 / 0.1. The search proper was left alone (cheaper expansions were
exhausted in round two); the rest, with the share of the code each change
touched (Castor / jetson / k30):

- The pour analysis's own-stamp bitsets (`OwnStamps`) are found by a
  table indexed by map, not a SipHash map, asked once per node of the
  pour (4.0 / 0.9 / 1.7, most of it hashing).
- The search's neck class comes from the step's coordinates
  (`zone_class`), not `class_at` with a division per neighbour (1.4 /
  0.7 / 0).
- The tile planner computes each (layer, tile)'s entry cost once per plan
  (it was computed for each of up to eleven relaxations) and pops from
  the four-way `KeyHeap` with (cost bits, state) as one word, the order
  of the `(u32, u32)` pairs it popped before (3.0 / 5.9 / 8.7).
- A restamp builds the new list first (marks only) and then changes the
  counts only of stamps that went or came; the kept branches' stamps were
  decremented and incremented again on every reroute. The list, its
  order and the final counts and tile claims are as before (6.7 / 2.8 /
  4.0; `PCB_ROUTER_FINGERPRINT_ALL`, which hashes every occupancy map,
  identical). The diagonal footprint is mirrored in place (`turned`
  allocated a vector per diagonal node).
- `Board::fill_clearance` read `PCB_ROUTER_OLD_FILL_CLEARANCE` from the
  environment for every item a fill probe tests; now once (0.6 on
  Castor).
- The fill probe's index and hatch-hole memo hash with `FastHasher`
  (rustc's FxHash; iteration order unused) (0.4 / 0.1 / 0.2).
- `PourMap::build` looks its layer's maps up once, not per neighbour
  (2.1 / 0.5 / 0.8 touched).

After (a profile of all changes but the restamp, scaled to the same
search samples): Castor's run 11 % less CPU (own stamps and SipHash 3.5
points, planner 1.8, `class_at` 1.4, pour labelling and probes 1.5), k30's
9 % (planner 5.5, pour 1.8). Nothing in
the work accounting changed: the negotiation's limit is expansions and the
pour analyses count by their model (41 ns a node, 38 ns a stamp, 47 ns a
probe item). The model now overstates the analyses' wall time (Castor's
clean-up: 9.8 s of pour work in 6.7 s of wall, was 10.9 s wall), so the
budgets buy less pour checking than they could; recalibrating would move
results and wants its own measurement.

CPU, pinned simultaneous pairs in route mode (`--jobs 1 --threads 4`,
`PCB_MAKER_NO_CACHE=1` both sides, `cpu_seconds` of the rows; the base is
always main's code without this change; scripts `build/pairs/pair.sh`,
`run-pairs.sh`, `rows.py` in the worktree, rows under `build/pairs/r1`
to `r4`). The first side named ran on 0-7,16-23 except in r2:

| board | r1 x862 / x870 | r2 x870 / x862 (swapped) | r3 x855 / x871 | r4 x881 / x872 |
|---|---|---|---|---|
| Castor | 0.864 (511.0 / 441.3) | 0.915 (507.7 / 464.6) | 0.858 (561.8 / 482.2) | 0.862 (552.6 / 476.3) |
| k30-SBC | 0.888 (612.1 / 543.5) | 0.940 (524.8 / 493.4) | 0.939 (494.8 / 464.6) | 0.894 (644.0 / 576.0) |
| jetson | 0.895 (2462.6 / 2205.0) | 0.919 (1938.6 / 1780.7) | 0.878 (2236.4 / 1963.6) | 0.840 (2151.0 / 1807.3) |
| spell_tome | 0.960 (22.4 / 21.5) | | 0.995 (18.8 / 18.7) | 0.967 (18.0 / 17.4) |
| urchin | 0.958 (43.1 / 41.3) | | 0.891 (43.2 / 38.5) | 0.941 (37.6 / 35.4) |
| geomean | 0.912 | 0.925 (three boards) | 0.911 | 0.900 |

Ratios are new / old CPU. Over the four rounds (geomeans): Castor 0.87, k30 0.92,
jetson 0.88, the small boards 0.95 (KiCad's DRC more than routing:
spell_tome is 1.2 s of work in 18-22 s of CPU); the spread between rounds
is the machine's load (19-37 beside other agents). Every pair: the same
board file (md5), vias, length, work seconds and verdict; with
`PCB_ROUTER_FINGERPRINT=1` (r2, r4) the same fingerprints on every board
(jetson in r2 the same multiset: its side-by-side probes print in the
order they finish). During the work, `PCB_ROUTER_FINGERPRINT_ALL` (every
occupancy map hashed) at a fifth of the budget, two threads: Castor and
k30 identical.

Short tier (tasks v6) x871 against `build/short-x855.log`: all 12 rows
identical apart from CPU, wall, load and memory (vias, length, work,
unconnected, errors, first-route open terminals; compare.py: same 12,
unconnected 42 to 42, passes 6 to 6, cut 0 on both). The tier's CPU (5064
to 3185 s) is not a claim: unpaired, and the two runs saw different
loads. Rebased on the hole-clearance fix, x872 against
`build/short-x880.log`: again all 12 rows identical (compare.py same 12;
CPU 5216 to 4982 s, unpaired).

Next: the search proper is 70-80 % of route mode; outside it the largest
items left are the restamp's list building (the marks pass over every
branch on every reroute) and the clean-up's pour analyses (pour labelling
and the fill probe's distance tests).

## Committed on 2026-10-15 (after the narrow attempt)

The narrow-signal attempt run alone (first-route budget, above) did not
use the layout's work on every board: Hub ended at 577 of 1500 s, vaio_re
at 1005. What was left after it, in work (1500 less the layout's work and
the placement pool's excess), and what to run with it:

| board | left after the attempt (x855) |
| --- | --- |
| jetson (x881) | 114 s |
| A13 module | 40 s |
| DSP-ADAU1452 | 228 s |
| vaio_re | 410 s |
| Hub | 916 s |
| koeg | 53 s |

Changed (crates/pcb-kicad/src/board_layout.rs): after that attempt, with
at least 240 s left (`NARROW_CONTINUATION_FLOOR`), connections open and
the wall clock inside the layout's guard (`narrow_continuation`), the
attempt runs once more in the other pour mode (as tracks after connected,
and the other way round), negotiation and clean-up split from what is left
as for the attempt (`direct_narrow_budget`), that work its deadline; the
layout keeps the better board as before (open items, then starved
thermals). The work of the attempt and of this one now counts in the
layout's `work_seconds` whether kept or not (it used to count only a kept
board's; on the ordinary ladder fallback it still does). `skipped` says
when it did not run (with too little work: jetson, module, DSP, koeg).
`PCB_LAYOUT_NARROW_CONTINUATION` (stop, tracks, seeds, ladder) and
`PCB_LAYOUT_NARROW_CONTINUATION_FLOOR` are experiment switches; stop is
x855's behaviour. Unit tests for the decision and the seed count.

Candidates, one layout run each, four threads, unpinned, load 13-34, no
row cut. Unconnected and errors are KiCad's (new errors against the
designer's), then vias and the layout's work; in brackets the router's
open count of the extra attempt, kept or not. Before: x855 (jetson x881,
the hole-clearance fix; module, DSP and vaio_re rows from the first-route
agent's x855 runs, deterministic, same tasks file); candidates on x890
(this code on 1c8f17f, switches by environment); after: x892 (this code
with the default on 2b3b787, routes identical to x855 there).

| board | before (stop) | P: probe both pour modes for the attempt | T: other pour mode | S: fresh seeds | L: class-width ladder |
| --- | --- | --- | --- | --- | --- |
| jetson | 17 / 1 starved / 670 / 1307 | - | not run (114 s left); x892 identical | - | - |
| A13 module | 17 / 1 track_dangling / 490 / 1330 | 21 / 1 items_not_allowed / 409 / 1127 | not run (40 s); x892 identical | - | - |
| DSP-ADAU1452 | 82 / 0 / 225 / 1210 | 126 / 0 / 189 / 864 | not run (228 s); x892 identical; floor 180: 82 / 0 / 225 / 1441 (223, not kept) | floor 180: 82 / 0 / 225 / 1347 (one seed, 229) | floor 180: 82 / 0 / 225 / 1380 (339) |
| vaio_re | 35 / 0 / 686 / 1005 | not run | **21** / 0 / 637 / 1338 (55, kept); x892 the same | 35 / 0 / 686 / 1210 (one seed, 121) | 35 / 0 / 686 / 1310 (551) |
| Hub | 30 / 0 / 170 / 577 | 39 / 11 starved / 179 / 875 | 30 / 0 / 170 / 818 (244, not kept); x892 the same | 28 / 0 / 172 / 891 (4 seeds, 111, kept) | 30 / 0 / 187 / 1404 (114, kept: KiCad the same, 17 more vias) |
| koeg | 7 / 20 (15 courtyards, 5 shorting) / 427 / 1335 | not run | not run (53 s); x892 identical | - | - |

Steps (x892): jetson the attempt alone; module the attempt and a round of
3 seeds; DSP the attempt and 4 seeds; vaio_re the attempt, one seed, then
the tracks attempt; Hub the attempt with its 4 seed retries, then the
tracks attempt with its 4; koeg the attempt and 3 seeds.

What was learned:

- The other pour mode is the only continuation that gained: vaio_re 35 to
  21 unconnected with 49 fewer vias. On Hub it spent 240 s for a board it
  did not keep (Hub's leftover would otherwise go unused: CPU, not
  quality), on DSP (floor lowered to 180) 230 s for nothing.
- Fresh seeds: Hub 30 to 28 (two vias more, 314 s); on vaio_re one seed
  fit (255 s an attempt) and lost by 45 open; on DSP one seed lost too.
  One perturbed seed with less negotiation is a worse attempt, not a
  variation of the same one.
- The class-width ladder with 230 to 900 s: never better in KiCad's
  count (vaio_re: four probes and the continued rung in 306 s of the 410,
  551 open; Hub: 826 s for the same 30 and 17 more vias). The ladder needs the budget the first-route
  agent took from it, which is why the attempt beats it.
- Probing both pour modes (75 s each side by side, the better continued
  with the attempt's negotiation less the probes): tracks led every probe
  (module 60 against 123 unfinished nets, DSP 104 against 247, Hub 17
  against 78) and the continued tracks attempt then lost on all three
  (module 21 against 17 unconnected, DSP 126 against 82, Hub 39 and 11
  starved thermals against 30). A probe's unfinished count does not
  compare the modes: in connect mode it counts pour pads the pour does not
  reach yet, in tracks mode the pour nets are ordinary nets that finish
  late. The probe code was removed again; tracks as the second attempt,
  judged by KiCad, is the safe form of the mode choice.

Short tier x893 (this on 670454c) against build/short-x855.log: all 12
rows identical (42 unconnected, 6 errors, 1176 vias, 22032 mm, 2390 s of
work; CPU 5064 to 3924 s is main's router-overhead round). No short-tier
board has the narrow-signal step, so none takes this path.
Logs in the worktree's build/ (pcb-maker-narrow2: <board>-x855, -P,
-T, -S, -L, -T180, -S180, -L180, -x892, jetson-x881; rows2.py prints the
rows with the steps), short tier build/short-x893.log.

Left: on boards with little left (module 40 s, koeg 53 s, jetson 114 s,
DSP 228 s) nothing runs; a tracks attempt first instead of connect where
tracks wins (vaio_re) needs a predictor the probe is not. The ordinary
ladder fallback's work is still counted only when kept.

## Layout sweep v16 (2026-10-15, x862)

docs/benchmarks.md "Layout sweep v16": 44 of 109 pass (v15 45), unconnected
3868 to 3527, new errors 289 to 82, better 13 / same 58 / worse 13, 17 rows
cut by the wall clock at load 16 to 35, which says a phase's wall runs well
ahead of its counted work on a quiet machine (KiCad DRC inside the layout
is not counted; the pool's charge is a reference 8-thread pool; candidates).
New timeouts ULK and poly_kybd_split72_left, the 0xCB pcb-panel ends on
kicad-cli's 900 s zone refill, SmartSpin2k and OpenESC panels stop with
parts the placer refuses. A triage agent (x930+) has the cut rows, the
timeouts, the worse rows and the panels; its section "Triage of v16"
follows. x862 predates the first-route budget (x855), hole clearance
(x881), router overhead (x872), pour recalibration (x817) and the second
narrow attempt (x893); the next sweep carries them.
## Committed on 2026-10-15 (agent walkthrough)

An agent's whole flow on two real boards, using only README.md,
docs/agents.md, docs/constraints.md and `--help`, from the KiCad project
with its schematic to a board `drc-kicad-board` passes. The harvested
GitHub boards carry no schematic (`benchmarks/real/external/github` holds
the board and project only), so the schematics and project libraries were
fetched at the boards' commits (origin.json): spell_tome_bottom (25 parts,
18 nets, 2 layers, its libraries in the project) and urchin (a split
keyboard, both halves on one board: 80 parts, 68 nets, KiCad 6 schematic
and libraries, MIT).

The flow (RAYON_NUM_THREADS=4):

```sh
pcb-maker import-kicad-netlist dl/spell_tome/spell_tome_bottom.kicad_sch tome spell_tome_bottom
pcb-maker describe-kicad-board tome spell_tome_bottom > board.json
# constraints.json: outline 70 x 100, U1 on the back, SW2 flush on the right
# edge opening outwards, J1 at the top, the four buttons in a region,
# L1 below U2 (relative), C2 near U2 pin 6
pcb-maker place-kicad-board tome spell_tome_bottom place1 tome/layout.json
pcb-maker layout-kicad-board tome spell_tome_bottom lay1 auto tome/layout.json
pcb-maker drc-kicad-board lay1/result spell_tome_bottom drc.json
pcb-maker export-kicad-fab lay1/result spell_tome_bottom fab
```

spell_tome: exit 0 in 41 s (73 s of work), 18/18, 9 vias, 586 mm, 8 of 8
constraints; drc-kicad-board 0 errors, 0 unconnected. urchin with 100
constraints (two 5 x 3 key grids as `row`s, a region per half, each of the
34 diodes `near` its key, thumb keys `relative` below the grid, the power
switches on the top edge, display and reset near the controller): exit 0
in 29 s (20 s of work), 68/68, 39 vias, 1897 mm, 100 of 100. With
`"total_seconds": 20` both finish in 12-13 s, still complete. Failure
paths tried: a 22 x 24 mm spell_tome (exit 1, `unplaced` U1, J1, J2 and
the hints in `next_actions`; 30 x 30 mm already routes clean), footprint
libraries removed (exit 1, `missing`), a netlist exported outside the
project, a file that is no netlist.

Friction found, and what was done:

- **Bug: a turned row was re-lowered the wrong way.** Layout mode lowers
  its own placement again for the report and the moves; `apply_rows` built
  the row from the leader's angle alone, so a row the placer had turned
  half way ran the other way: urchin's right half reported `region SW18`
  missed by 61 mm and `relative SW33 SW18` by 95 mm (exit 1 on a board
  that was fine), and the members would have been pulled there by the
  followers' sync. The row's frame is now the leader's angle or that angle
  turned by a half (a quarter with `turns`) turn, whichever matches where
  the members lie; a fresh stack matches all alike and keeps the leader's.
- **`near` could not name a key of a row** ("SW2 is carried by its row"):
  a keyboard's diode at its switch could not be said. `part_of` and
  `pin_of` may now name any row member: a new placer anchor
  `Anchor::Area(leader, member, centre, half)` is the member's body (or
  pad, half size 0) in the leader's frame, and the report names the member.
- **Bug: KiCad 6 library footprints kept `fp_text reference`.** KiCad
  reads it over the Reference property the import sets, so every copy of a
  footprint carried the library's reference (`Bat+0`): 181 schematic
  parity issues on urchin (73 duplicate, 73 missing footprints, 35 net
  conflicts). The import converts `fp_text reference` and `fp_text value`
  to properties (place, layer, font, hidden kept): 6 left, the controller's
  no-connect pins below.
- **276 false warnings "fixed parts X and Y overlap"** on every placement
  of a fresh import: the check ran before the constraints, when the stack
  was fixed by the default rules (parts off the board are on the edge);
  `move_all` frees them. It runs after the constraints now.
- **The layout's last line said `native complete=false`** above a verdict
  `complete` (KiCad's raw count includes the cosmetic silkscreen
  warnings). It now prints KiCad's unconnected count and its findings of
  any severity, "the verdict judges them"; agents.md says `routed.native`
  is raw and the verdict decides.
- **`describe-kicad-board` gave a fresh import a 3.8 x 0.8 mm outline**:
  SW2's slots on Edge.Cuts. `outline.from_footprints` is true when the
  board draws no Edge.Cuts of its own.
- **`place-kicad-board` needed a placer.json** that agents.md told the
  agent to cut out of layout.json; it now takes the layout config (its
  `placer` object) as well.
- **Missing footprints said only "not found"**: now where it was looked
  for (no library of that name, or the library's folder without that
  file), and the stderr line says to add the library and import again
  into the same directory (constraints.json and layout.json are kept; the
  help said "existing files kept", which read as "the board too").
- **A netlist exported outside the project** missed the project's
  libraries (9 of 25 footprints on spell_tome): the import also looks in
  the folder of the schematic the netlist names as its source.
- **A file that is no netlist imported as an empty board, exit 0**: now an
  input error (exit 2) that says how to export one.
- **`place-kicad-board` on a board without room did not end** ("checks
  constraints in seconds"): urchin on 80 x 40 mm ran over 12 minutes on
  two threads before it was killed; one part in a region too small for it
  took 55 s. The placer's relaxation levels anneal and legalize again and
  again, and the command had no work budget unless
  `PCB_PLACER_WORK_SECONDS` was set (with 10 s it still took 60 s of work:
  the budget is checked between levels only, and the first level's
  legalize, legalize-first and repair passes took 11, 10 and 8 s). The
  command now takes the layout's placement share by default (600 s of
  work, the same budget the layout would give it), and a `region` smaller
  than its part in every orientation it may take is an input error at
  once (exit 2, "region for Display1 cannot hold it: the box is 3.00 x
  3.00 mm, the part ...").
- **Help and docs**: layout's help said the unplaced case leaves a
  board-layout.json with only the verdict (there is none; verdict.json has
  it); `total_seconds` was only in `--help` (now in agents.md with what it
  does to the phases); the row rules (constraints on the first part hold
  the whole row) and two keyboard lines in the cookbook; import's help says
  what comes next and its exit codes; `unmatched_pins` on KiCad's
  `unconnected-(...)` nets are harmless but show as parity `net_conflict`.

Left as they are, small: every command's help repeats the agent exit-code
table, also for `describe-kicad-board` and `import-kicad-netlist` whose
codes differ; the import report's paths are relative where the verdict's
are absolute; `back`, `front` and `opening_outwards` are kept but not
listed in `constraints[]`; `drc-kicad-board` counts library findings
(`lib_footprint_issues`) as cosmetic that the layout's DRC does not run.

`tests/agent_walkthrough.rs` runs the documented flow on urchin
(`crates/pcb-kicad/tests/fixtures/urchin`: the schematic, project, library
table and the seven footprints used, with the MIT licence): import,
describe, a placement with a region too small for its part (exit 2 at
once), a placement that keeps all 100 constraints (a diode
reported against its key), the layout at `"total_seconds": 30` with
`PCB_ROUTER_NO_WALL_GUARD=1` (exit 0 and the verdict's fields), and
`drc-kicad-board` (exit 0). It is in the root package:
`cargo test --release -p pcb-maker --test agent_walkthrough` (14-28 s at a load of 20-25).

The row fix changes layouts whose row the placer turns (keyboards, LED
bars written as `row`); boards without rows place as before. The region
check and the placement default touch only `place-kicad-board` and
regions too small to hold their part. Snapshot: `build/bin/pcb-maker-x920`.
No benchmark tier was run.

## Committed on 2026-10-15 (zone seams)

The census's five TelemetryOnboard misses (J1/J2 SH3/SH4, D12.1, all on
F.Cu): its GND has four zones, a whole-board one of priority 0 (F, B), one
of priority 1 (F, B, In1, also the whole board) and two 4 mm bands along
the round edge, priority 10 (left, top) and 11 (right, bottom), drawn with
arcs. What KiCad 10.0.7 does (zone_filler.cpp, zone.cpp
BuildSmoothedPoly, drc_test_provider_zone_connections.cpp; the 10.0.7 tag,
not the master checkout in benchmarks/real/external/_downloads): each zone
is filled on its own over its outline and those of the same-net zones it
collides with (so reliefs and spokes are the same up to the seam), trimmed
to its own outline, and the outlines of the net's zones of a strictly
higher priority are cut out of it last (`subtractHigherPriorityZones`;
equal priorities overlap). The fills meet at the higher zone's outline
with no clearance between them. The DRC runs per zone and layer: a pad's
spokes are the crossings of its ring with that zone's fill, islands are
that zone's, tracks count where they end in that zone's fill, and a zone
whose fill does not cross the ring is no connection. The USB shield pads
(1.55 x 1.5 mm, 1 mm from the band's inner edge at y = 45.5) have their
ring 0.015 mm inside the band: three spokes in the priority-1 zone, one
(+y) in the band, "actual 1" for zone 11 (10 for J2.SH3). D12.1 sits in
the left band with its +x spoke crossing into the priority-1 zone: one.

Modelled (`zone_seams`, on, `PCB_ROUTER_NO_ZONE_SEAMS`): `Plane::above`
holds the outlines of the net's higher-priority zones on the layer; where
a net has two or more zones on a layer, `kicad_spokes` counts per zone the
pad's spokes whose ring crossing lies in the zone's part (sampled across
the spoke: a spoke the seam splits counts in both), its island spokes,
other pads' crossings and its tracks ending there, by the DRC's rule; the
worst zone's shortfall (`SpokeRecord::seam_short`, `seam_zone`) joins
`spokes_short`, the dump (`seam_short`) and spokes.py; a manual spoke for
it must end in that zone's part. Two fixes it needed: zone outlines drawn
with arcs are followed (`rule_area_polygon_points` dropped `arc`, so the
bands were polygons through their other corners; `PCB_KICAD_OLD_ZONE_ARCS`;
in the corpus only Telemetry's zones and a keepout of oro-os link have
arcs), and D12.1's +x spoke, which the model blocked: it ends in a strip
exactly the minimum width (0.2 mm) between its relief and D12.2's 0.2 mm
clearance, whose brush centres lie on one line 2 microns short of the
spoke's end (KiCad's end test is 5 microns past gap and half width); the
clearance test now runs from the ring to gap plus half width
(`spoke_clear_to_fill`, on, `PCB_ROUTER_OLD_SPOKE_CLEAR`). Not modelled:
per-zone fill settings (the layer's fill takes the last zone's clearance
and minimum width: Telemetry's bands keep 0.25, the priority-1 zone the
class's 0.2) and islands the seam alone makes (island verdicts stay the
net's one fill's).

Census (44 boards, KiCad side from the fill3 cache), x817 against x912
(x910 before the rebase, identical), and each change switched off:

| | x817 | x912 | no zone_seams | old spoke clear | old zone arcs |
|---|---|---|---|---|---|
| DRC starved predicted | 157 | 162 | 157 | 161 | 157 |
| missed | 12 | 7 | 12 | 8 | 12 |
| false alarms | 27 | 26 | 26 | 27 | 26 |
| starved pads not modelled | 2 | 2 | 2 | 2 | 2 |

Telemetry 7 found / 5 missed to 12 / 0 (D12.1 now as an island sliver in
zone 1, the strip the lattice misses: right verdict, KiCad counts it as a
plain spoke). The false alarm gone is Quanta75's FB2.1 (B.Cu, the same
end-of-spoke case); capybully's U1.8 changes one spoke from blocked to
none, verdict unchanged. Hub (several equal-priority GND zones per layer)
gets `seam` rows only on pads already short. No new false alarm.
Overlapping same-net zones of different priority (`experiments/
pour-fidelity/seams.py`, every census board and every v15 source and
result): only Telemetry in the census; in the corpus also OpenESC-20x20
4in1-mini (GND In2, priority 0 under 30) and SNSP-CPU-01 (GND B.Cu, 2
under 4), neither with a thermal pad near the seam.

Route mode on the v15 placements (tasks v6, `--jobs 1 --threads 4`, one
run per side), x817 against x910, `build/seams-x8{17,10}.log`:
Telemetry 108/108, 0 unconnected, 88 vias both, KiCad starved_thermal 5
(J1.SH3, J2.SH3/SH4, J1.SH4, D12.1, each "actual 1" in a band or the
priority-1 zone) to 0, length 3549.8 to 3559.4 mm (the manual spokes into
the short zones), other findings identical (the designer's 50 clearance,
22 solder_mask_bridge); 4in1-mini and SNSP-CPU-01 identical (53 and 259
unconnected, SNSP's 13 solder_mask_bridge and 2 starved on both).

Short tier x912 (rebased on 140d0bf) against build/short-x880.log (tasks
v6): all 12 boards byte-identical (no short-tier board has two zones of a
net on a layer, so only `spoke_clear_to_fill` could move them), 42
unconnected, 6 errors, 1176 vias, 22032 mm both; work 2390 to 2384 s
(framework_mobo 225.5 to 220.3 s, the same board: the shorter clearance
test asks the fill probe less). Tests:
`each_zone_counts_the_spokes_in_its_own_part` (router: a pad with three
spokes in the lower zone and one in the upper is short 1 in the upper, by
priority or overlapping at equal priority, 0 as one fill; the manual spoke
ends in the upper part) and
`a_zone_of_a_higher_priority_is_cut_from_the_nets_lower_zone_and_arcs_are_followed`
(lowering). Logs: build/census-seams (census x817.txt, x912.txt, the switched-off
runs try2-*.txt, seams.py's JSON), build/seams-x817.log,
build/seams-x910.log, build/short-x912.log.

## Open problems, ranked

1. **Fill fidelity, what is left** (2026-10-14 section above): island-only
   inner-layer pads in tracks mode (stitch single pieces or add a via),
   the island-rule and track-rule edge cases, pads with repeated numbers
   and custom spoke templates, piece fragmentation against KiCad's kept
   outlines, and a pour of one net filling under a mask opening that
   exposes another net's pad (USB_Cable_Tester).
2. **One-spoke pads on fine-pitch rows** (k30's U10 pad 8, 1.77 x 0.50 mm:
   the two corridors along the row are closed by the neighbours, and the
   neighbours' exit tracks run through the two left). A hard block failed;
   the repair (`free_thermal_spokes`, budgeted, only when polishing) moves
   2 of 10 nets. Probably needs the exit direction of the neighbouring
   pads decided before negotiation (a fan-out step), not a cost.
3. **The ladder's judge.** Measured on 21 boards with every rung
   continued to its final (2026-10-13, current code; `build/ladder-judge-v2`
   our v14 placements of the tier's eight 4-layer boards, jetson and katia;
   `build/ladder-judge-merged` nine designer placements from
   `build/ladder-judge2` (x510) plus Hub rerun; `build/ladder-judge-v2x`
   MIDAS-MK1; the stats line now prints `nets` (conflicted or incomplete)
   and `stranded` pour pads; `experiments/congestion/ladder_judge.py <dirs>`).
   Right by final open count: unfinished (today's judge) 16 of 21,
   conflicted 14, conflicted-or-incomplete 14, incomplete 14, open after a
   30 s continuation 17; by (open, starved) 14 / 12 / 12 / 12 / 15. Open
   above the best rung, summed: unfinished +166 (Hub's designer placement
   alone +121), conflicted +53, the 30 s continuation +20. Conflicted wins
   Hub only by the tie rule (17 against 17 conflicted nets); on our MIDAS-MK2
   and BAGEL the pour-pad term is what picks right (connect rungs: 6 and 20
   stranded pads, ending 3 open and 1 starved against tracks' 0). No change.
   The probe statistics of the 26 other v14 layouts that ran the probe
   ladder (`build/ladder-judge-v14p`, `analyse.py` there) show a class no
   count fixes: Hub, DSP-ADAU1452, koeg-board, zpn_devboard (old placement)
   probe tracks far ahead in every count (conflicted too: 15 against 19,
   136 against 137, 6 against 68, 0 against 51) while a fresh connect
   attempt with less budget ends far better (126 against 268 open, 338
   against 641, 147 against 204, 195 against 236); the 75 s probe is too
   short there, and the regular rungs that follow recover it. If anything,
   the 30 s continuation per probed rung (up to 120 s more of work) is the
   judge to try. Still open from before: the ladder stops at the first
   complete rung whatever its starved thermals (Castor, ours: all four
   rungs probe 0 unfinished, rung 0 ends with 20 starved against tracks'
   12); rank by (open, starved) before vias, and let a complete-but-starved
   rung be rivalled.
4. **Crowded placement** (OpenESC, SNSP, video, hackrf, BAGEL): the three
   directions are built as options and none wins; `replicate_channels`'
   rigid macros never get seated. The legaliser is not the lever (4 %,
   see the 2026-10-14 section); the routing halos are worth 22 % of
   wirelength on crowded boards but removing them loses routing. Next:
   routing room from congestion rather than pin count; bus-aligned rows
   for video; seed-repeated pairs as the judge.
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
8. **The agent flow's gaps** (the 2026-10-15 walkthrough), ranked by what
   they save an agent against what they cost:
   1. *A fast no.* A placement without room costs a minute (one part
      without a place on urchin) to many (a side 95 % full): every
      relaxation level anneals and legalizes again, and the work budget is
      checked only between levels (a 10 s budget took 60 s of work). Check
      it inside `legalize`, `legalize_first`, `evict_for` and the repair
      pass, and stop at once when even the tight bodies need more than a
      side's area (`overfull` at the last level) or a hard constraint's
      box is smaller than its part (regions do this now, in
      `region_holds`; an `edge` with `max_mm` and a row longer than the
      outline are the same test). Small to medium: a day, with the
      placement-loss tasks to check that feasible boards place as before.
   2. *Room as numbers.* A placement without room says so in words only
      (`next_actions`: "the back side is 81 % full"). Put `room` in the
      verdict: the outline used, utilization per side, and a suggested
      outline (the current one scaled by the root of utilization over
      0.7, at its aspect), so that an agent enlarges in one step. Small:
      half a day (placement facts, verdict field, the walkthrough test).
   3. *Report what is kept.* `back`, `front`, `rotation` and
      `opening_outwards` are enforced but not in `constraints[]`, and the
      verdict counts `schematic_parity_issues` without types (an agent
      opens drc.json to learn that urchin's 6 are no-connect pins). Add
      them as statuses and a per-type parity count, with a `next_actions`
      line when only `unconnected-(...)` pins are involved. Small: half a
      day.
   4. *Starter constraints for keyboards and arrays.* The import writes
      `move_all`, `outline: {}` and the plug-in connectors; urchin needed
      100 hand-written entries. Recognise a key matrix from the netlist
      (switches whose second pad meets a diode, the diodes' other side on
      row nets, the switches' first pad on column nets): one `row` per
      matrix with `columns` from the column nets, a `near` of 3 mm per
      diode to its switch (row members are allowed now), split halves by
      their controllers. Medium: one to two days with the corpus's
      keyboards (urchin, ErgoSNM, bumwings, chiffre, ULK) as tests.
   5. *Outlines that are not rectangles.* `outline` takes a width and a
      height; spell_tome is book-shaped and urchin two halves joined by
      mouse bites. Accept `"polygon": [[x, y], ...]` and `"pieces": [{...}]`
      written as Edge.Cuts (the placer already follows any outline and
      keeps parts on their piece). Medium: a day, mostly the outline
      sizing loop, which scales width and height today.
   6. *A router starter config.* A board from a netlist has the
      project's net classes or the default 0.2 mm for every net. The
      import could write `router.json` beside `layout.json` with a power
      class for rails (by name and by `power_in` pin type) and, on two
      layers, `add_pours` GND on B.Cu, listed in the report for the agent
      to keep or drop. Small to medium: a day with the from-schematic
      benchmark to check it does not cost routability.
   7. *Schematics in the harvested corpus.* `harvest.py` fetches the
      board and project only, so the from-schematic benchmark runs on the
      KiCad demos alone. Fetch the schematic sheets and the project's
      libraries at the same commit (the walkthrough did it by hand for
      two boards). Small for the fetch, medium for a tier on top.

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
- One task list per benchmark comparison: the tiers default to
  github-tasks-v6 since 2026-10-14 (v5 before); a log against one on the
  other list compares inputs, not binaries (x768 v5 against x776 v6 looked
  like 174 more vias). compare.py's first line names both lists and says
  TASK LISTS DIFFER; rows on other prepared constraints are marked INPUT.
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
