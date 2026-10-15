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

## Triage of v16 (2026-10-15, x931 / x932)

The v16 sweep (x862, tasks v6, five jobs of four threads beside other
agents, load 16-35) against v15 (x723). The fix is on branch
v16-triage: x931 = x862's tree (068de8b) + fix, so that x931 against the
v16 rows changes only the fix; x932 = main f7addd8 + fix (tests, short
tier). Outputs were in the triage agent's scratchpad (not kept).

**1. The 17 cut rows.** Per row from the layout logs (seconds; "search"
and "pour" sum every phase's `profile work:` line, probes side by side
included, so they are thread sums, not wall; the KiCad column sums the
`board check` lines; lattice builds are the last `so far N lattices` count,
also summed over routers built side by side; the placement charge is its
work plus the pool's excess, `layout: placed`):

| board | wall | work | placement wall / charge | KiCad DRC (calls, hits, s) | lattice builds (n, s) | search wall / work | pour wall / work | cut |
|---|---|---|---|---|---|---|---|---|
| motherboard | 1702.5 | 909.5 | 262 / 122 | 2, 1, 16 | 7, 9.1 | 4159 / 2662 | 258 / 262 | deadline |
| link | 1709.0 | 1108.7 | 389 / 368 | 1, 1, 3 | 3, 22.3 | 3545 / 2353 | 280 / 304 | deadline |
| poly_kybd_split72_right | 1722.9 | 1232.9 | 131 / 192 | 1, 1, 2 | 3, 4.0 | 4120 / 2686 | 178 / 92 | deadline |
| poly_kybd_split72_left | 1800.7 | - | 194 / 210 | 0, 0, 0 | -, - | 3298 / 2084 | 146 / 80 | timeout |
| poly_corne_split42_right | 1784.9 | 1149.7 | 179 / 208 | 1, 0, 28 | 3, 5.9 | 3539 / 2193 | 191 / 103 | deadline,layout |
| poly_corne_split42_left | 1669.6 | 1168.4 | 96 / 125 | 1, 1, 1 | 3, 3.1 | 4182 / 2840 | 182 / 102 | deadline,layout |
| koeg-board-pcb | 1699.0 | 409.7 | 139 / 183 | 6, 5, 37 | 11, 40.1 | 1964 / 1050 | 42 / 54 | deadline |
| katia | 1671.9 | 585.6 | 200 / 323 | 1, 0, 17 | 3, 704.6 | 507 / 221 | 62 / 31 | layout |
| capybully | 1696.1 | 710.9 | 85 / 64 | 2, 0, 73 | 7, 179.1 | 1210 / 918 | 172 / 131 | deadline |
| Batch_1 | 1680.2 | 1325.7 | 370 / 387 | 4, 0, 225 | 6, 74.1 | 3432 / 3901 | 755 / 200 | deadline |
| Quanta75_BareRP2040_JLCPCBAo | 1721.3 | 1045.2 | 194 / 124 | 1, 0, 28 | 3, 42.8 | 2918 / 2175 | 243 / 243 | deadline,layout |
| woagboard | 1759.1 | 1201.7 | 89 / 76 | 11, 0, 115 | 19, 39.0 | 1833 / 1363 | 579 / 566 | deadline |
| panel | 1710.7 | 507.8 | 219 / 167 | 7, 0, 306 | 12, 567.3 | 1127 / 1765 | 640 / 293 | deadline |
| MegaDrive | 1800.7 | - | 130 / 253 | 0, 0, 0 | -, - | 4690 / 3537 | 89 / 85 | timeout |
| ghoul | 1700.7 | 1162.9 | 260 / 410 | 8, 0, 321 | 13, 44.9 | 1048 / 937 | 511 / 524 | deadline |
| ULK_sl_PG1316s | 1758.4 | 92.1 | 3 / 4 | 7, 0, 196 | 12, 1099.6 | 310 / 390 | 276 / 66 | deadline |
| ULK | 1800.6 | ~95 (at routed) | 1 / 2 | 8, 0, 201 | 13, 1201.4 | 335 / 406 | 313 / 63 | timeout |

No single phase explains them; four holes, by size:

- **Lattice builds of boards with a large cutout** (ULK, ULK_sl, katia,
  the 0xCB panel; 570-1200 s of the run). Not work, and not
  interruptible. Profiled (gdb samples, a per-obstacle trace): all of it in
  `StaticMaps::build`, `Shape::distance_to_point` of one board cutout
  measured for every node of its box edge by edge: ULK's 454-corner
  cutout over 16867 mm2, 15 s a class, four classes, 60-190 s a lattice
  (0.075 mm rungs the most); katia's 228-corner cutout 15 s a class; the
  panel's slots 0.7-2.7 s each. **Fixed** (`PolygonField` in grid.rs, a
  polygon of 16 corners or more): each edge updates only the nodes within
  reach of its box, the inside comes from each row's sorted crossings with
  `point_in_polygon`'s test; the distances are bit for bit
  `distance_to_point`'s below the reach (unit test against it on concave
  polygons with lattice-aligned corners), so the maps and the routes do
  not change. ULK's lattice 72 s to 1.2 s; route mode on v16's ULK
  placement, both measures in one binary (`PCB_ROUTER_OLD_POLYGON_DISTANCES`
  for the old): the 32 fingerprints the old side reached are all in the new
  side's (rungs side by side print in another order).
  Layouts on x931, one run each, four threads, load 6-21 (v16: 13-27):

  | board | v16 (x862) | x931 |
  | --- | --- | --- |
  | ULK | timeout at 1800 s, no row (v15: 66/70, 4 unconnected, 1 starved) | 1480 s, no cut, 66/70, 4 unconnected, no new error, 64 vias |
  | katia | 1672 s, cut (no final ladder), 139/162, 104 | 1382 s, no cut, the same board (139/162, 104, 401 vias); the final ladder ran and lost |
  | 0xCB panel | 1711 s, cut (deadline), 152/188, 86, +5 starved | 957 s, no cut, the same board (283 vias, 4594 mm); one more ladder rung ran and lost |
  | ULK_sl_PG1316s | 1758 s, cut, 68/70, 2 | 1710 s, still cut, the same board: its ladder now ran nine rungs (1200 s budget used) and seven uncached KiCad checks (22 s each), the last rung's seed retry was cut |

  The DRC cache hit the first route's and the early rungs' checks on
  ULK, ULK_sl and the panel: the boards were byte-identical to v16's.

- **KiCad DRC inside the layout is not work** (ghoul 8 calls 321 s, the
  panel 7 / 306 s, Batch_1 4 / 225 s, ULK 200 s, woagboard 11 / 115 s,
  capybully 73 s; an uncached check 10-50 s). A deterministic charge needs
  a model, and the obvious ones fail: over 57 v16 boards the median
  uncached check time does not follow the board's pads, tracks and vias
  or its file size (correlation 0.02 each); hatched zones (the panel,
  Batch_1, WLED_Matrix, capybully) and the load dominate. Counting the
  measured wall would make the layout follow the load. Not changed;
  options below.
- **The search's work rate under the sweep's load.** On the boards that
  spend the whole budget (the four PolyKybd halves, link, motherboard,
  MegaDrive, Quanta75 Bare) negotiation and repair run 1.3-1.6 times
  their counted work in wall time: `EXPANSIONS_PER_SECOND` is an idle
  machine's, and five jobs of four threads already exceed the 5950X's 16
  physical cores, so with other agents (load 16-35 on 32 logical CPUs)
  most search threads share a core. The 10 % margin cannot hold there;
  this is the sweep's configuration as much as the accounting.
- **Pour analyses on hatched and big pour boards** run 2-4 times their
  modelled work (Batch_1 755 s of wall for 200 s of work, the panel 640 /
  293, PolyKybd 180 / 95; x862 still has the 41 ns constants, main
  recalibrated to 18 ns after the overhead round, so recheck on main).
- Smaller: the placement pool's charge (kept seeds at the reference eight
  threads) below the four-thread wall on motherboard (262 s of wall, 122
  charged) and Quanta75 Bare (194 / 124); above it on most boards. A
  layout's `work_seconds` in the row leaves out a final ladder it did not
  keep (koeg: 410 s reported, its unkept ladder ran 1100 s more of wall),
  so the rows understate the work that ran.

After the wall guard fires, the layout still runs what it does not
interrupt (lattice builds, repairs and stitching of the current attempt,
KiCad checks, labels and the final DRC), and the harness kills at 1.2x
(1800 s): that is how three of the cut rows became timeouts (ULK,
poly_kybd_split72_left, MegaDrive).

**2. New timeouts.** ULK: the lattice builds above (13 builds, 1201 s;
the first route needed 95 s of work); v15 finished in 1688 s because its
deadline was then the wall clock, which stopped the ladder; fixed by the
polygon field (row above). poly_kybd_split72_left: the first route's
negotiation ran 1046 s of wall for 677 s of search work (the load), the
wall guard cut it at 862 s of its work, and the two repair passes after
the cut (each about 60 s of negotiation, 300 s of hard repair and
stitching in all) ran past 1800 s; v15 (x723, wall-clock deadline) ended
at 396/420. Same class as the cut rows, no lattice or DRC share. 0xCB
pcb-panel: not a layout problem; the placement (306 s) and the first route
ran, then kicad-cli's zone refill timed out at 900 s (open problem 7,
`internal error: kicad-cli timed out`); v15 timed out at 1800 s before
getting there.

**3. The 13 worse rows** (compare.py counts unconnected only; PixelWave's
lost pass, one new error, is not among them). Placement against v15's
(poses of every footprint compared; v15's rows have no
`first_unconnected_terminals`), then the mechanism:

| board | v15 -> v16 (KiCad unconnected) | placement | route mode on v16's placement, x723 / x862 | x862 on v15's placement | verdict |
| --- | --- | --- | --- | --- | --- |
| Telemetry | 0 pass -> 1 | identical | 0 / 0 (88 vias both) | - | layout flow: the ladder kept 120 s for the narrow step, the step needs 180 s (`narrow_step_minimum`), 153 s were left, so it did not run (v15 won with it: 0 open; route mode's step: 0 open). Constants of 18a9a77: the reserve's floor (120 s) is below the step's minimum (180 s). Regression, on main too (not a far-open board) |
| ATAT1800 | 53 -> 54 | identical | - | - | noise: first route 67 -> 68 open, one attempt each, new errors 7 -> 4 |
| Explorer | 1 -> 2 | 2 parts moved | 3 / 3 | - | noise: first route 9 -> 3 open, the narrow attempt 8 open both |
| eurorack-pmod | 20 -> 21 | 3 parts moved | 22 / 17 | - | noise: router not worse; first route 19 -> 26, the polish kept the unpolished routes |
| protoesp | 83 -> 92 | 1 part moved, identical first route (88 open, 319 vias) | - | - | layout work: the first route and its resume booked 1246 s of work in 881 s of wall (parallel batches count each thread's expansions), so no ladder ran (v15, wall-clock deadline: four attempts). 30b14fb (x747) |
| module | 81 -> 82 | different | - | - | narrow-step reserve too small (154 s left of 319 kept); fixed on main by 1c8f17f / 974fa2b (x892: 17) |
| DSP-ADAU1452 | 105 -> 121 (+59e -> 0) | different | - | - | same; main x892: 82 |
| vaio_re | 101 -> 138 | different | - | - | same; main x892: 21 |
| BAGEL-MK1.1 | 0 pass -> 1 | different | 2 / 2 (identical) | 2 | noise in the layout path: router and placement give the same count in route mode |
| hackrf | 0 +1e -> 2 | different (205 of 421 moved, 0.635 mm steps) | 13 / 13 | 1 | placement |
| reCamera | 4 -> 9 | different | 9 / 9 | 4 | placement |
| WLED_Matrix | 4 +13e -> 6 +1e | different | 6 / 6 | 4 | placement (and errors 13 -> 1) |
| 4in1-mini | 38 -> 71 | different | 74 / 79 | 53 | placement (router 74 / 79 is noise) |

The router is not the cause on any of the eight boards routed in pairs
(x723 and x862 on v16's placement, route mode, four threads, one run
each: identical counts and vias but for eurorack and 4in1-mini, which
differ by 5 either way). Where the placement is the cause, the race ran
fewer seeds: hackrf 16 to 10, WLED_Matrix 14 to 11, 4in1-mini 9 to 5,
BAGEL-MK1.1 9 to 6. The placer's work now counts what the legaliser and
annealer do (93f40bd, x792) and the pool runs only the seeds the budget
keeps (8435fff), so the same placement budget holds fewer seeds; the
kept seed's wirelength is the same on these boards, the poses are not.
Suspects: 93f40bd and 8435fff; seed-repeated pairs would settle it (one
seed's result is not a placement's quality). Not regressions of the
code paths, but the budget now buys fewer seeds.

**4. Panels.** The rows changed wording more than behaviour: v15's
"placement is not legal" and v16's "incomplete: N parts without a legal
place" are the CLI exit-code commit's verdicts for the same failure.

- PolyKybd molecules (4x4, 4x5, 5x2_shifted, 7x5_wave_left; 5x4_wave
  places since x792's in-turn naming and routes 2/28): the parts named are the back connectors
  J5 and J6 (J9 on 7x5). The designer stacks them on the front J3 and J4
  (same spot, other side, SMD pads only, no courtyard) and KiCad finds
  no courtyard or other overlap there (DRC of the designer's 4x4: only
  starved thermals and cosmetic findings). What refuses them is the edge
  constraints of repeated references, named in turn since x792 (the k-th
  entry for J5 binds the k-th J5 in board order). The task's entries are
  not in board order, and a corner part's two edges are two entries that
  now bind two parts: placing the 4x4 with every part except J5 and J6
  fixed where the designer put them (`move_all` off) misses 17 edge
  constraints on the designer's own fixed J1, J3 and J4 (18.5, 37.6 and
  56.6 mm off: whole tile pitches) and leaves J5, J6, J2 and SW1
  without a legal place. So the rule is wrong for these tasks, not the
  geometry: a repeated reference's edge entries should be matched to the
  instances that can keep them (per entry, any instance; or the task
  generator writes them in board order with a corner's edges in one
  entry).
- SmartSpin2k_Panelized: `keep_copies` confines each KiKit copy's
  movable parts to its region (no edge constraints in the task, so the
  "region" of the hint is the copy's). The rule agrees with the designer:
  placing from the designer's poses (`move_all` off, x862) ends complete,
  every part placed and every constraint kept. From the prepared stack
  (every movable part at one point, inside one copy's region) 57 parts
  find no legal place: the search, not the rule. Start each confined part
  inside its own copy's region (its copy's fixed parts' centre) instead
  of at the common stack.
- OpenESC 4in1-panel: one net group (no copies), the front side 101 %
  full by the placer's bodies: crowding, as in v15 ("not legal" then).

**Ranked, what to assign next:**

1. Merge the polygon field (this branch; byte-identical maps, ULK no
   longer times out, katia and the panel no longer cut). Short tier x932
   against build/short-x912.log (main f7addd8 without the fix): all 12
   rows identical (42 unconnected, 6 errors, 1176 vias, 22032 mm, 2384 s
   of work); build/short-x932.log. Tests of pcb-router and pcb-kicad pass.
2. The sweep's configuration against the 1.1 margin: at most 16 search
   threads in all on this machine (four jobs of four, or three while
   other agents run), or a harness timeout and wall guard that scale
   with the measured load; otherwise every board that spends its budget
   is cut (8 of the 17).
3. Stop after the wall guard: once it fires, skip the remaining repairs,
   stitching passes and ladder rungs and go to the final check (three
   timeouts came from work after the guard).
4. KiCad checks in the layout: judge ladder rungs by the router's count
   and check only the candidates that could win, or charge a check from
   a per-board model of zone fill (hatched zones dominate; pads and tracks
   do not predict it), or store the check's seconds in the DRC cache and
   charge them on a hit as on a miss once known.
5. Repeated-reference edge constraints (molecules): match entries to
   instances; then the four molecule panels can place.
6. Keep_copies start positions (SmartSpin2k).
7. Recheck the pour work model on hatched boards on main (2-4x under).
   Done: "Committed on 2026-10-15 (hatched pour work)".
8. The narrow-step reserve's floor (18a9a77): `narrow_step_reserve`
   keeps at least 120 s, `narrow_step_minimum` wants at least 180 s, so
   a board whose first attempt is short keeps a reserve the step can
   never use (Telemetry lost its pass to this; route mode's step
   completes it). Raise the floor to 180 s, or let the step run on what
   was kept. Done: "Committed on 2026-10-15 (narrow reserve floor)".
9. Placement seeds per budget (hackrf, reCamera, WLED_Matrix,
   4in1-mini): seed-repeated pairs of x723 and main's placer to see
   whether fewer seeds is the loss; if so, the placement budget in the
   new work units needs raising. Measured: not the loss ("Placement seeds
   after honest work").

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

## Committed on 2026-10-15 (cold-route regression, x900-x903)

The two regressions of the Freerouting refresh, cold route (designer
placement, no copper), route mode, 4 threads, one run each (deterministic;
`build/cold/run.sh` and `table.py` in the worktree pcb-maker-cold).

Bisect, KiCad unconnected items (b399ec85 and b3c1f81d built from 399ec85
and 3c1f81d in a scratch worktree):

| binary | x155 | 399ec85 | 3c1f81d | x325 | x510 | x644 | x738 | x881 | x902 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| SNSP-CPU-01 | 55 | 55 | 178 | 178 | 178 | 178 | 178 | 178 | 5 |
| MegaDrive | (95) | 95 | 137 | | | | 137 | 137 | 70 |

Both flip at 3c1f81d (2026-10-10, a rerouted net's corridor grows a tile
every sixth reroute, not every third). Mechanism (`PCB_ROUTER_DEBUG`, new
`route_net ...: search failed` and `resolve ...` lines):

- SNSP: GND (199 pads, four walled in at J3 and P6) failed a board-wide
  search in iterations 0 and 1 with those four open, so it kept to its
  corridors (`hopeless_searches` = 2, b72e61a); in iteration 2 a corridor
  missed and route_net stopped the whole net at 48 pads, frozen as
  blocked (21-48 pads for the rest), while the signals took the room.
  The repair ripped it up whole and its hard reroute from one pad reached
  21. Router and KiCad agreed all along (GND partial, 178 terminals): no
  counting bug, the 362/363 is per net. The narrower corridors only made
  the miss happen. Fix: pads a board-wide search did not reach are left
  out of the net's later searches in that negotiation
  (`unreachable_terminals`, `PCB_ROUTER_OLD_HOPELESS`).
- MegaDrive: narrower corridors miss more, the windowed fallbacks cost
  (open searches 228M expansions against 14M), the work budget ends
  negotiation at iteration 15 with 145 nets in conflict instead of 19
  with 106, and the repair ripped 77 nets up whole; 19 ended with no
  copper at all. Not a bug in 3c1f81d (it measured well on the quick
  tier), but the repair's whole rip-up is: it now removes only the
  conflicted branches (ignoring the negotiation's every-eighth whole
  rip-up) and keeps an incomplete net's tree (`resolve_keeps_partial`,
  `PCB_ROUTER_OLD_RESOLVE`).

Cold route, KiCad unconnected (x881 = main before; x900 = the pad rule
plus keeping incomplete trees; x901 + conflicted branches kept; x902 +
the stuck reset, = x903 before the rebase onto 670454c):

| board | x881 | x900 | x901 | x902 |
| --- | --- | --- | --- | --- |
| SNSP-CPU-01 | 178 | 5 | | 5 |
| MegaDrive | 137 | 137 | 147 | 70 |
| vaio_re | 8 | 41 | 5 | 5 |
| mackerel-30 | 0 | 0 | | 0 |
| DSP-ADAU1452 | 0 | | 0 | 1 |

vaio on x900: the tracks rung went 634 to 132 open and became the best,
and its narrow attempt ripped +3V3E up whole (3 of 43 pads): the reason
the repair change is part of this. DSP on x902: every rung as good or
better, but the ladder had time for the 0.05 mm rung (50 open) and left
the narrow-signal step 52 s of work instead of 142 (2 open against 0;
82 to 60 vias). SNSP CPU 816 to 283 s (one attempt, no seed rounds).

Short tier x903 against build/short-x817.log (tasks v6, current main):
unconnected 42 to 35 (MIDAS 26 to 20, WLED 8 to 6, reCamera 4 to 5),
passes 6 to 6, vias 1176 to 1129, new errors 6 to 12 (WLED_Matrix
connection_width 3 to 9: KiCad's GND fill necks at the RGB LEDs' pad 3,
on both sides, more of them on the new layout), work 2390 to 2677 s
(reCamera 65 to 366: its layout took another path; WLED 358 to 469),
CPU 4437 to 4865 s (not a pinned pair). Unit tests:
`a_walled_in_pad_does_not_stop_the_rest_of_its_net` (shows the old net
ending at the corridor miss), `the_hard_repair_keeps_the_branches_out_of_conflict`.

Quick tier (30 boards, tasks v6, 6 jobs), x817 (main at 670454c)
against x903 (this change on it): build/quick30-x817.log,
build/quick30-x903.log.

- x817: 24/30 pass, unconnected 27, new errors 2, vias 2913, length
  98215 mm, CPU 13555 s, work 9313 s, cut 0, first-route open 52; load
  median 17.9 (8.3-41.7).
- x903: 25/30 pass, unconnected 21, new errors 2, vias 2867, length
  99235 mm, CPU 13190 s, work 11453 s, cut 0, first-route open 47; load
  median 10.9 (1.7-34.7).

Better 3, same 27, worse 0; no row cut on either side; 23 rows
identical. The changed rows:

- OpenFC: 1 to 0 open, passes. Its first route 2 to 1 open; the ladder's
  regular rungs ran quicker, so the 0.075 mm pitch fit the refinement
  budget (x817: projected 296 s, over 240) and its re-seeded tracks rung
  finished (0 open, 169 vias against 159). Work 563 to 2143 s, CPU 464 to
  1802 s: the finer rungs really ran.
- MIDAS-MK1.1: 9 to 8 open, vias 240 to 197, length 3391 to 4464 mm (GND
  as tracks: the ladder's tracks rung now beat the first route, which
  x817 kept). Work 396 to 1322 s is accounting: the layout's work counts
  the ladder's only when its board is kept (src/agent_cli.rs), x817's
  ladder ran as long and was dropped; CPU 1572 to 1425.
- reCamera: 9 to 5 open, first route 10 to 8, work 891 to 610.
- WLED_Matrix: 6 to 6 open, vias 138 to 143, first route 10 to 8; errors
  the same (copper_edge_clearance 1; its connection_width necks are in
  the short tier's layout, not this one).
- USB_Keypad: 2 to 2, vias 99 to 78, work 513 to 445.
- urchin (work 97.3 to 93.8) and framework_mobo_lefthalf (391.8 to 392.0):
  same boards, the searches differ by a little.

New errors 2 on both (WLED copper_edge_clearance, eternal-keypad's), so
none came or went. Work +2140 s is OpenFC +1580 and MIDAS +926 (the
accounting above), less reCamera, Keypad, WLED.

The five cold boards on one binary pair, x817 against x903, KiCad
unconnected / vias / CPU s:

| board | x817 | x903 |
| --- | --- | --- |
| SNSP-CPU-01 | 178 / 1220 / 775 | 5 / 1456 / 302 |
| MegaDrive | 137 / 1148 / 615 | 70 / 1324 / 506 |
| vaio_re | 8 / 421 / 1142 | 5 / 369 / 961 |
| mackerel-30 | 0 / 46 / 574 | 0 / 46 / 555 |
| DSP-ADAU1452 | 0 / 81 / 1587 | 1 / 60 / 1450 |

(x903 identical to x902 on all five; x817 the same counts as x881 and
x738. x940, this change rebased onto f7addd8's zone seams: the same
counts, vias and lengths as x903 on all five.)

WLED_Matrix's connection_width findings (short tier): KiCad's own fill,
not ours (the routed board has no filled_polygon; the tracks rung leaves
GND's zone to KiCad's refill), necks of the GND zone on F.Cu at the
LEDs' pad 3 where other nets' tracks pass close. Not more GND as tracks:
GND track length 531 to 496 mm (F.Cu 409 to 423, B.Cu 122 to 73). The
x903 result is the tracks rung at the 0.075 mm pitch (10 open, the
regular pitch 14), x817's at the regular pitch (15; its 0.075 rung 23):
on the finer lattice other nets' tracks hug the LED pads, 25 to 31 of
the 64 LED GND pads with a foreign F.Cu track within 0.35 mm.

Not done: the walled-in pads could still be joined among themselves (a
forest of trees in the hard search: J3.5-J3.6, P6.5-P6.6 would save 2 of
SNSP's 5); a hard search that fails from the first pad's tree still ends
the net; the corridor schedule itself (3c1f81d) was not revisited.

## Committed on 2026-10-15 (placement room)

The first three of the walkthrough's automation gaps (open problem 8):

- **A fast no.** Before placement (after the constraints are applied),
  a user's `row` that fits the outline's box in no orientation it may take
  and the parts held at one edge on one side that need more length than it
  has (single file: only parts too deep to stand two behind each other
  within their reach, each by its shortest tight body, the two that may
  overhang the ends by their copper) are input errors (exit 2), said
  together with the region errors (`placement_room::outline_holds`; not
  for an automatic outline, which the layout grows). In the placer,
  `hopeless` (computed on the last, loosest level before the levels run):
  the movable tight bodies need more area than a side's outline, or they
  cannot lie side by side in strips (`strips_hold`: bodies at least `t`
  across meet a line at most `floor(height / t)` times, so their least
  extents along it sum to at most that times the width). Both are exact,
  so a board that can be placed never takes this way; a hopeless one skips
  every level: one hurried legalisation from the global placement,
  `overfull` in board-placer.json and a hint. The work budget is checked
  inside the passes (`out_of_work`, a thread-local stop the running
  `place` sets): past it a part without a spot in `legalize` searches 31
  rings (15 mm of a 0.5 mm grid) instead of the board, `evict_for`,
  `refine`, `repair_relations` and `align_orientations` stop, the
  stuck-first retry is skipped.
  Urchin (walkthrough fixture) on 80 x 40 mm, four threads, load 20-35:
  with the walkthrough's constraints (rows, regions) x921 exit 1 after
  742 s (the "over 12 minutes"), now exit 2 in 0.04 s; move_all only on
  60 x 40 mm exit 1 in 3 s. Move_all only on 80 x 40 mm is not hopeless
  by either test (the KiCad 6 switches have no courtyard: their pads alone
  block, and pads could interleave), so it still takes the levels: x921
  398 s, x948 353 s (loads differ; the seeds' work 756 s against the
  600 s budget before, 604 s now: the passes stop at the budget).
- **Room as numbers** (`placement_room::room`, `room` in board-placer.json
  and in the verdict of place, layout and of an input error a constraint
  of the outline raised): per side the usable area (outline less the edge
  band and fixed bodies), the movable bodies with spacing, utilization,
  the area missing at a target of 0.7, the smallest outline at the target
  (area at the current aspect, then large parts side by side in strips as
  full as the target, choosing among sizes within a tenth of the least
  area the one nearest the current aspect; every part and row; edge sets)
  and, when parts did not fit, `suggested_outline_mm` with a next action.
  The urchin example (agents.md) suggests 118 x 107.5 mm; placed there,
  every part finds room (two near/relative entries written for halves
  side by side miss). An error verdict with room is written into a fresh
  output directory, and a directory holding only such a verdict is taken
  again by the retry.
- **What is kept.** `back`, `front`, `rotation` and `opening_outwards` are
  in `constraints[]` (`Constraints::kept`, `report_kept`; not in the
  placer's ranking, which sees only `report`). The verdict has
  `schematic_parity_by_type`, and a next action that says there is
  nothing to do when every finding is "No pad found for pin" (urchin's
  6 net_conflict, the controller's no-connect pins).

Identity (x948 = this on 140d0bf, against x921): place-kicad-board with
the short tier's layout.json and constraints (tasks v6) writes
byte-identical boards on spell_tome_bottom, USB_Cable_Tester, urchin,
eternal-keypad, OpenFC and Blue_Line; the short tier (tasks v6, 2 jobs x 2
threads, `build/short-x948.log`) against build/short-x921.log: every
layout's placement (placed/board-placer.json poses) and result board
byte-identical, all rows identical (compare.py same 12, unconnected 42
to 42, passes 6 to 6, vias, length, work, first-route open terminals)
except the `constraints` count on MIDAS-MK1.1 (3 to 6), OpenFC (25 to
50) and reCamera (1 to 2): their tasks' `rotation` entries are listed
now; nothing missed changed.
Snapshot `build/bin/pcb-maker-x948` (x940 was taken by another agent);
after the rebase on ad4362f, x949.
## Committed on 2026-10-15 (connection width)

KiCad's `connection_width` (drc_test_provider_connection_width.cpp, 10.0.7,
fetched from the 10.0.7 tag): runs only with a board-setup
`min_connection` (KiCad's default 0) or a custom rule, never from a
netclass. Per net and copper layer it unites zone fills (teardrops
included), tracks, flashed vias and flashed pads, fractures the union and
pairs each concave vertex with the nearest non-neighbouring vertex of the
same outline closer than the minimum less 2 x (DRC epsilon 0.0005 + max
error + extra fill margin 0.0005) = 0.115 mm for 0.127, seen from inside
at that vertex, with the outline bulging past the limit in x and y on both
sides. Vertex based and local: it also flags narrow gaps between two
parts of one outline ("gap" below). Position is not in the JSON.
`experiments/pour-fidelity/necks.py` reproduces it vertex for vertex on
kicad-cli's own fill (`--refill-zones --save-board`) and agrees with
KiCad on every board with findings (OpenESC's 199 are KiCad's cap).
Severity is the project's: error only on three corpus boards (CDFER
WLED_Matrix 0.127, USB_Keypad 0.127, USB_Cable_Tester 0.1); 15 others
have it as a warning.

Why our pour model misses them: kicad-cli's DRC with `--refill-zones`
first rebuilds teardrops for every pad with teardrops enabled (WLED: 3
zones become 636), and the necks are KiCad filler leftovers its
min-thickness pruning does not reach, all in hatched zones: the thermal
rings (added after pruning), the minimum-width border the iterative
refill restores around other nets' teardrop knockouts, pad corners
against hatch holes. Census (necks.py; build/necks/designer.txt,
results.txt, route.json, gaps.json):

| board | KiCad | kinds |
|---|---|---|
| designer WLED_Matrix | 4 | fill between pads / by a track (2 near other nets' teardrops), 2 gaps |
| designer USB_Keypad | 9 | fill near teardrops 5, thermal ring 2, by a track 2; 2 gaps |
| our WLED, v16 / short-x903 / short-x921 / short-x941 | 4 / 13 / 7 / 13 | GND F.Cu at the RGB LEDs' pad 3: pad copper or relief against hatch holes and the LED's own pad 1, a ring, fill near teardrops |
| our USB_Keypad, v16 / x903 / x921 / route x941 | 3 / 4 / 3 / 7 | GND F.Cu fill near teardrops, 3 of route's 7 are gaps |
| warnings: 0xCB pcb (ours) | 14 | thermal rings against other nets' copper |
| warnings: OpenESC 4in1, 4in1-mini (ours) | 199 cap (855, 1090) | our own tracks narrower than 0.16 |
| warnings: reCamera, MegaDrive, BAGEL, OpenESC designer | 1-30 | own tracks, a via, fill between pads |

Not modelled in the pour: it would need KiCad's teardrop generator and
its hatched iterative refill (polygon booleans the router does not have).
Built instead: a repair with KiCad's fill (`necks.rs`, on,
`PCB_KICAD_NO_NECK_REPAIR`; docs/router.md "KiCad's connection width").
When the final DRC has connection_width errors, kicad-cli refills and
saves a copy, the net's copper (fill outlines, pads, tracks, vias) is
rasterised at 4 microns, and saddles of the distance map (a pixel
narrower than the limit whose wider neighbours form two arcs, leading to
copper wider than the limit both ways within 0.5 mm) are the necks; on
the boards above it finds exactly KiCad's passages (7/7, 12/13 with one
duplicate pair, 3/3, 4/4). A track of the net, minimum wide, runs along
each into wide copper where it keeps class, hole, edge and other-fill
clearances (shifted or shortened if not); the board is checked again and
the tracks kept while connection_width drops and nothing else (error
types, unconnected) rises, by halves if all together fail (at most six
DRCs). Gaps are not repaired: the space's saddles were mostly gaps KiCad
does not report (Keypad: 20 candidates, 7 to 6 instead of 7 to 3). A
track that removes a Keypad neck sometimes adds a zone-zone unconnected
item in KiCad (not understood; the halving drops it). Cost: two to six
kicad-cli runs, 45-90 s wall, only on boards with such errors; a hook in
route_kicad_board's end and in the layout's end (a `necks.json` marker
stops a second try); `pcb-maker repair-kicad-necks <dir> <id>` runs it on
a routed directory; `board-router.json` gets `neck_repair`.

Numbers (x950; the commit's binary x951 is the same code plus comments):

- Route mode on the v16 placements, tasks v6, x941 against x950
  (build/necks-route-x9{41,50}.log): WLED_Matrix connection_width 4 to 0
  (5 tracks), USB_Keypad 7 to 3 (4 tracks; the 3 left are gaps),
  USB_Cable_Tester none; unconnected, vias, other errors identical.
- Short tier (flock, tasks v6) x941 against x950 (build/short-x941.log,
  build/short-x950.log): new errors 12 to 3, unconnected 35 to 35, vias
  1129 to 1129, passes 6 to 6, work 2671 to 2671 s, cut 0. Changed rows:
  WLED_Matrix connection_width 13 to 1 (12 tracks; new errors +11 to +2,
  the copper_edge_clearance it had), USB_Keypad 4 to 2 (2 tracks kept,
  under the designer's 9 either way). Every other row identical; the
  rows' length is the router's (the repair tracks, under a millimetre a
  board, are not in it). CPU WLED 609 to 637 s, Keypad 517 to 591 s (not
  a pinned pair).
- Starved-thermal census (census.py, fill3 KiCad cache, 44 boards) with
  x950: 162 predicted / 7 missed / 26 false alarms / 2 not modelled, as
  x912 (the router is untouched).
- Unit tests `a_thin_bridge_between_two_wide_parts_is_a_neck_a_wide_one_and_a_spike_are_not`,
  `findings_are_read_from_kicads_report`, `a_repair_track_keeps_its_clearances`.

Open: gaps (fill them along the slot, with KiCad's "same outline"
condition); the Keypad unconnected item; the WLED neck the repair leaves
(0.070 at a repair track, x950 short tier).

## Placement seeds after honest work (2026-10-15, x934 / x980, no code changed)

Question from the v16 triage (its item 9): since the placer's work is
counted honestly (93f40bd) and the pool runs only the seeds the budget keeps
(8435fff), the race runs fewer seeds (hackrf 16 to 10, WLED_Matrix 14 to
11, 4in1-mini 9 to 5, MIDAS-MK1.1 12 to 5 kept). Is the placement budget too
small? Answer: **no; noise or honest cost, nothing shipped.**

No setting exists for this: the layout sets the placer's `work_seconds` to
0.4 of `total_seconds` itself (`--placer` cannot change it) and the race's
first batch is `min(placement_seeds, 3)`. x980 = 1223878 plus two fields used
only for this measurement and not kept: the layout's `placement_share`
(0.4) and the placer's `minimum_seeds` (3), and a `--layout` JSON option in
run.py. Layout mode, tasks v6, four threads, three jobs, load 8-31, one run
per configuration (the layout is deterministic, no row cut):
(a) x934 as is; (b) `placement_share` 1.6 (2400 s of seed work, room for
16 seeds on all six); (c) `minimum_seeds` 16 (all 16 seeds in one batch,
whatever the budget); (d) as (c) with `congestion_probes` 6 (the race
probes six placements instead of three); and the designer's placement
(tracks stripped) in route mode on x934 as the ceiling. Logs and outputs
`build/seeds/{a-x934,b-x980,c-x980,d-x980,designer-x934}`.

KiCad unconnected (+ new errors); v15 and v16 are the sweeps' rows:

| board | v15 | v16 | (a) | (b) | (c) | (d) | designer, route mode |
| --- | --- | --- | --- | --- | --- | --- | --- |
| hackrf | 0 +1e | 2 | 2 | 2 | 2 | 0 +1e | 0 +1e |
| reCamera | 4 | 9 | 5 | 5 | 5 | 5 | 4 |
| WLED_Matrix | 4 +13e | 6 +1e | 6 +1e | 6 +1e | 6 +1e | 6 +1e | 1 +4e |
| 4in1-mini | 38 | 71 | 54 | 48 | 48 | 56 | 5 |
| Castor | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| MIDAS-MK1.1 | 11 | 9 | 8 | 8 | 8 | 8 | 8 |
| sum | 57 | 97 | 75 | 69 | 69 | 75 | 18 |

Vias 1416 / 1422 / 1422 / 1310 (a, b, c, d); CPU 7139 / 9811 / 8546 / 8251 s.

The placement per configuration (seeds kept of run, the placer's kept
seed; the clock's charge = the kept seeds' work as the reference pool of
eight threads runs them per batch; routing work left = 1500 s less the
charge; placement wall on four threads):

| board | (a) seeds, charge, left, wall | (c) | (b) | sum of seed work (a / 16 seeds) |
| --- | --- | --- | --- | --- |
| hackrf | 10/10 #4, 146, 1354, 214 | 16/16 #4, 153, 1347, 191 | 16/16, 196, 1304, 225 | 646 / 1048 |
| reCamera | 16/16 #1, 78, 1422, 60 | 54, 1446, 64 | 78, 1422, 82 | 402 |
| WLED_Matrix | 11/11 #2, 121, 1379, 168 | 122, 1378, 176 | 171, 1329, 308 | 603 / 877 |
| 4in1-mini | 5/5 #3, 297, 1203, 162 | 16/16 #16, 311, 1189, 400 | 443, 1057, 413 | 691 / 2100 |
| Castor | 16/16 #1, 28, 1472, 27 | 19, 1481, 39 | 28, 1472, 38 | 145 |
| MIDAS-MK1.1 | 5/7 #4, 309, 1191, 199 | 16/16 #11, 307, 1193, 341 | 406, 1094, 515 | 630 / 2082 |

What it says, per board:

- **The clock barely charges the extra seeds.** The charge is the
  reference pool's makespan per batch; (a) runs a batch of three and then
  one more batch, two rounds of the eight-thread pool, and (c)'s single
  batch of 16 is also two rounds. So 16 seeds cost the routing 0-14 s of
  its budget (less on reCamera and Castor, whose 3 + 13 batches become one),
  but 20 % more CPU over the six boards (+1400 s, 4in1-mini and MIDAS
  +1050 and +500) and twice the placement wall on four threads where the
  seeds are long (4in1-mini 162 to 400 s, MIDAS 199 to 341 s). (b) runs
  the same 16 seeds in 3 + 13 and charges up to 130 s more (4in1-mini 443,
  MIDAS 406) for the same boards as (c): the routing did not use it.
- **hackrf: not the seeds, the probe.** v15's final placement (RUDY 12.51,
  the best score) is one of (a)'s ten seeds too, and the race probes it
  first in every configuration (scores [12.51, 19.06, 19.6] in v15, v16,
  (a) and (c): the same three placements). v15's probes left [16, 16, 23]
  nets unfinished and it kept that placement (0 +1e); x862's and x934's
  probes leave [36, 72, 10] and keep the third (2). The probe is a 75 s
  route by today's router, so the router change since x723 moved the race,
  not the placer. With six probes (d) the fourth placement (RUDY 20.15,
  probe 0 unfinished) wins and ends 0 +1e, 603 vias against 701.
- **MIDAS-MK1.1:** 16 seeds bring v15's best-scored placement back (RUDY
  1.57, absent with five seeds); its probe leaves 24 unfinished against the
  kept one's 17, so the race keeps the same placement and the board is the
  same (8, 197 vias). v15's final placement is the placer's kept seed then
  and now (5 parts differ).
- **WLED_Matrix:** the extra seeds only change the third probe (4.29
  instead of 4.89, 119 unfinished); the same placement wins, identical
  board. v16's poses equal (a)'s; v15's (138 parts differ) are not among
  today's seeds.
- **reCamera, Castor:** 16 seeds already in (a); identical results in all
  configurations. reCamera's v16 row (9) is 5 on main: the router, not
  the placement (v16's triage counted it as placement).
- **4in1-mini:** seed 16 (beyond (a)'s five) scores best by RUDY and is
  kept: 54 to 48. With six probes (d) a placement whose probe left fewer
  nets unfinished (101 against 109) wins the race and ends worse (56):
  the probe's order does not hold for the final route when the counts are
  this close. v15's 38 came from placements today's placer no longer makes
  (every configuration differs from v15 in 188 of 191 parts; x792's far-side
  courtyards changed the seeds, Castor's sixteen seeds differ from v15's in
  162 parts too). The designer's placement routes to 5: the gap on this
  board is the placement itself, not the number of seeds.

Over the six boards: (b) and (c) 75 to 69 unconnected, all of it one board
(4in1-mini 54 to 48), with (d) giving 6 back on the same board; the
legaliser agent measured a placement change to flip crowded boards like
this one by far more, so one seed's difference on one board is not a
signal. More placement budget (b) costs routing budget (up to 130 s) and
CPU for nothing measurable; a floor of 16 seeds (c) is nearly free in the
clock but costs 20 % CPU and placement wall. Neither is clear enough for a
default, and the v16 "worse" rows were not seed counts: hackrf is the
probe race under the new router, reCamera the router, WLED_Matrix and
MIDAS unchanged with 16 seeds, 4in1-mini a placement gap that seeds do not
close (designer 5, ours 48-56). Short tier not run (no default changed).

Worth knowing for later: the race's probes (RUDY's best three, 75 s each)
pick the final placement by a margin smaller than their own noise on
hackrf and 4in1-mini (probe 36 for a placement that routes to 0-1; probe
101 for the one that ends 8 worse than the probe-109 one). The designer's
ceiling (18 unconnected over the six against 69-75) is in 4in1-mini (5 vs
48-56) and hackrf's vias (277 vs 603-701).

## Committed on 2026-10-15 (narrow reserve floor)

The layout ladder's reservation for the narrow-signal step
(`narrow_step_reserve`, "ladder reservation" above) was not a floor: a
rung, a finer pitch or a seed round started when the work left beyond
the reserve covered the last attempt's work, a finer pitch costs about
the square of the pitch ratio more, a rung's work deadline was the
pitch's whole work left (the reserve included), and the step then needed
only half of the reserve. Worse, the reserve's own floor (120 s) was
below the step's minimum (`narrow_step_minimum`, 180 s), so a board with
a short first attempt kept a reserve the step could never use:
TelemetryOnboard (v16 triage) kept 120 s, its three 0.05 mm rungs ran
(none better than the 0.075 mm tracks rung), 105 s were left, the step did not run and the
board ended 1 unconnected (v15 passed with the step).

Changed (crates/pcb-kicad/src/board_router.rs), only where the ladder
keeps a reserve (a work deadline, i.e. a layout, and a project where the
step applies):

- The reserve is at least `NARROW_RESERVE_FLOOR` (240 s: the step's
  180 s minimum and its clean-up at the negotiation's 60 s floor), at
  most the cap as before (0.4 of the work left, 600 s), and none when
  the cap is under the step's minimum (`narrow_step_reserve_floored`).
- A further rung starts only if its projected work leaves the whole
  reserve (`fits_before_reserve`; the last attempt's work, times the
  square of the pitch ratio on a finer lattice, `projected_attempt_work`,
  at least 120 s). The seed retries and seed rounds were already checked
  against the whole reserve.
- Work deadlines end where the reserve begins (`deadline_before_reserve`):
  a rung's at the pitch's work left less the reserve, each seed's at its
  share (seeds side by side count once each) of the work left less the
  reserve. A rung that stops at the deadline only does so where it would
  otherwise have eaten into the reserve.
- The step negotiates at least what the ladder's first attempt negotiated
  (its budget: the probe and its continuation, or the first rung's),
  bounded by the reserve and half of what is left
  (`narrow_step_negotiation_floored`).
- A board the ladder completes (a clean skeleton rung that goes on to
  the next rung) releases the reserve.
- The step still runs with half the reserve and at least 180 s left (a
  rung can still overrun its projection inside its deadline).

`PCB_LADDER_OLD_RESERVE` restores the old rules. Route mode keeps no
reserve and is unchanged, as are projects without the step and the
direct narrow-signal attempt of far-open boards (its configuration is
already narrow, so the step does not apply to it).

Layout runs, tasks v6, four threads, one run per side, unpinned; before
x941 (main at ad4362f), after x960 (this on 1223878; the polygon field
of 6aa79de changes no map; x961 is the commit rebased onto 6099e13,
whose neck repair acts only on boards with connection_width errors, none
of these). KiCad's unconnected and new errors, vias,
the layout's work (s); "same board" means the routed .kicad_pcb is byte
for byte the same:

| board | path | before | after |
| --- | --- | --- | --- |
| TelemetryOnboard | ladder, step applies | 1 / 0 / 102 / 1379, step skipped (105 s left of 120 kept) | **0 / 0 / 99 / 1196, passes**: 240 s kept, the 0.05 mm rungs did not start, the step ran (139 s of work, router 0 open) |
| MokyaLora | ladder, step applies | 0 / 0 / 333 / 1365, passes | same board, 1206 s: 240 s kept, the one-seed round of the best rung (1 open, about 160 s of work) did not start |
| A13 module | direct narrow attempt | 17 / 3 / 488 / 1653 | same board |
| vaio_re | direct narrow attempt | 15 / 0 / 716 / 1263 | same board |
| DSP-ADAU1452 | direct narrow attempt | 65 / 0 / 220 / 1276 | same board |
| Castor | no step | 0 / 0 / 167 / 924 | same board |
| keyboar_ | no step | 0 / 0 / 39 / 275 | same board |

No row was cut. module, vaio_re and DSP take the direct narrow-signal
attempt since x855 and never reach a ladder with a reserve, so this change
cannot touch them (their x941 numbers differ from x892's in the "after
the narrow attempt" table through the commits between, not this one).

DSP-ADAU1452 cold route (designer placement, no copper, route mode,
1200 s ladder, 4 threads): x940 1 unconnected, x960 the same board
(same attempts, 60 vias). It was not a reserve problem: route mode keeps
no reserve, and the step was not short of work. Its log (x903, x940,
x960): the best attempt before the step is the 0.075 mm tracks rung (20
open, 115 vias, ahead of the connect rung's 20 open and 135 vias on
vias); the step varies it, its negotiation reaches 1 net in conflict at
iteration 10, sits there to the price cap and stops on the stall at the
cap (`stall_at_cap` 12) after 25 iterations, with 16 s of negotiation
work of a budget of at least 60 s; the repair leaves 2 open. x817's step varied
the 0.075 mm connect rung (22 open against tracks' 24), completed in its
negotiation, and 96 of its 142 s of work were the via reduction of a
complete board. The 0.05 mm rung came after the best attempt was chosen
and only spent ladder work. What would help there is a step that tries
the other pour mode or the runner-up attempt when it stalls, not more
work.

Short tier x960 against build/short-x941.log (tasks v6): all 12 rows
identical (vias, length, work, KiCad's findings; 35 unconnected, 12 new
errors, 1129 vias, 2671 s of work; CPU 3913 to 3349 s is load). No
short-tier board keeps a reserve (no "kept for the narrow-signal step"
line in any layout log).

Unit tests: `the_floored_reserve_is_enough_for_the_step_to_run`,
`no_attempt_starts_into_the_reserve`,
`attempts_stop_where_the_reserve_begins`,
`the_step_negotiates_at_least_what_the_first_attempt_did_within_the_reserve`.
Logs: worktree pcb-maker-reserve2, build/runs (rows.py prints the rows
with the ladder's reserve, rungs and step lines; cold-x960 the cold
route), build/short-x960.log.
## Committed on 2026-10-15 (panel fixes)

The two panel defects of the v16 triage (part 4). x1000 = this on
9ce95b1; x1001 = the same after the rebase on 6099e13 (necks only).

**1. Entries for a repeated reference name their footprint by uuid.**
The task generator writes an edge and rotation entry only for the copies
the designer put at an edge, in board order; x792's in-turn rule bound the
n-th entry to the n-th footprint of that reference, so the entries landed
on other copies. Now an `edge`, `rotation` or `place` entry may carry
`uuid` (the footprint's `uuid`, `tstamp` in KiCad 5-7 files;
`describe-kicad-board` lists it): it binds that footprint, a uuid no
footprint of the reference has is an error, and entries without one keep
the in-turn rule (hand-written constraints; docs/constraints.md).
`from_pcbench.py` writes `uuid` for every entry of a repeated reference.
**Tasks v7** (`build/github-tasks-v7`, the existing generator with
`--corpus` on the absolute-path copy of boards.json, x1000's
describe-kicad-board): tasks.json byte-identical to v6, 103 constraint
files identical, six differ only by uuids (the five molecules and
OpenESC's 4in1-panel, one entry). Binaries before x1000 reject v7's six
files (unknown field `uuid`); the other 103 are v6's.
Judge, every part fixed at the designer's spots (`move_all` off, the
task's constraints, place-kicad-board): missed constraints x990 on v6 /
x1000 on v7: 4x4 29 / 0 (17 of them on J1, J3, J4, the triage's count;
the rest on J5, J6), 4x5 31 / 0, 5x2_shifted 14 / 0, 5x4_wave 10 / 0,
7x5_wave_left 19 / 0. With J5, J6 and J9 free (the rest fixed): 5x4_wave
and 7x5_wave_left complete; 4x4 (J5, J6), 4x5 (J5, J6) and 5x2 (J5)
still find no legal place for some back connectors. That is geometry, not
binding: at the designer's poses (`PCB_PLACER_DESIGNER`) neighbouring
tiles' J5 and J6 lie 0.03 mm apart on the back (centres 2.03 mm, pad
bodies 2 mm wide) and the edge tiles' J5 0.016 mm from the board edge
(the copper-to-edge floor is 0.02 mm).

**2. keep_copies regions follow the copies' layout, and copies start in
their own region.** SmartSpin2k's 57 unplaced parts were the rule's, not
only the start's: its fixed parts sit at the top of each copy (2.3 mm
from the board's top edge, 40.5 mm from its bottom), so the cut halfway
between two copies' fixed parts ran through a copy's own parts: 162 of
the designer's 630 movable parts lay outside their copy's region
(the 3 x 3 copies got rows 50.7, 65.3 and 88.9 mm high). The cut now
moves from halfway by half the difference of the board's margins beyond
all copies' fixed parts, clamped between the two copies (each copy's
region lies around its fixed parts as the board lies around all of
them): 0 of 630 outside, rows of 67.6, 65.3 and 72 mm. The 0xCB pcb-panel's
regions do not change (0.1 mm display), the 0xCB panel's cut moves from
82.7 to 82.0 (the board's middle). In addition each confined part starts
global placement at its region's centre (`Constraints::starts`, read by
`global_place`; parts without one start as before). place-kicad-board on
v16's prepared SmartSpin2k (4 threads): x990 57 parts without a legal place; the shifted
regions alone complete (final wirelength 14217 mm); shifted regions and
starts complete (12219 mm). Only boards where keep_copies fires change:
by a replica of its test over the 109 tasks of v7, SmartSpin2k and the
two 0xCB panels.

**Layouts, one run each** (four threads, jobs of two, load 12-30; tasks
v6 for x990, v7 for x1000; placed = movable parts with a legal place):

| board | before | x1000 |
| --- | --- | --- |
| SmartSpin2k_Panelized | x990: 57 parts without a legal place (exit 1, 185 s) | 630/630 placed, 568/585 routed, KiCad 19 unconnected, 0 new errors (the designer's 199 invalid_outline subtracted), 1036 vias, 1218 s of work, not cut, 924 s |
| poly_kb_molecule_4x4 | v16: 4 unplaced, 8 illegal (J5, J6, SW1) | 157/160 (J5, J6 twice), no constraint missed |
| poly_kb_molecule_4x5 | v16: 6 unplaced, 17 illegal | 190/196 (J3, J5 three times, J6 twice, SW1) |
| poly_kb_molecule_5x2_shifted | v16: 2 unplaced (J5, J6) | 129/130 (J5) |
| poly_kb_molecule_5x4_wave | v16: placed, 2/28 routed, 499 unconnected, 5 starved | placed, 3/28 routed, 499 unconnected, 6 starved_thermal, 501 vias, 937 s |
| poly_kb_molecule_7x5_wave_left | v16: 1 unplaced (J9) | 234/235 (J9) |
| OpenESC 4in1-panel (information) | v16: 207 parts (load 54) | 199 parts without a legal place: crowding (one net group, no copies) |

The molecules now place the designer's fixed parts' edges right, but
from the stack the back connectors still need the designer's 0.03 mm
gaps (above); the next step there is pad-level spacing for same-net
pads of neighbouring parts on one side, not constraints. 5x4_wave's 499
unconnected are its routing (25 of 28 connections open in both).

**Identity.** place-kicad-board with the short tier's prepared sources
and layout.json (build/short-x941, tasks v6), x990 against x1000:
byte-identical boards on spell_tome_bottom, USB_Cable_Tester, urchin,
eternal-keypad, OpenFC and Blue_Line. Short tier on x1000 (tasks v6, 2
jobs x 4 threads, under the lock, `build/short-x1000.log`) against
build/short-x941.log: compare.py same 12, unconnected 35 to 35, passes 6
to 6, new errors 12, vias 1129, length 22253 mm, work 2671 s, first-route
open terminals 61; every layout's placed board and result board
byte-identical. The only row field that differs is `constraints` on
MIDAS-MK1.1 (3 to 6), OpenFC (25 to 50) and reCamera (1 to 2): x941
(ad4362f) predates the placement room commit, which lists the tasks'
`rotation` entries (as x948 against x921).

Unit tests: `a_repeated_reference_entry_with_a_uuid_names_that_footprint`
(placement_constraints), the shifted cut and the starts in
`copies_of_a_tabbed_panel_keep_their_parts` (board_placer),
`global_placement_starts_parts_where_a_start_says` (pcb-placer tests).
Logs: build/panels-x1000.log, build/panels-ss-x990.log,
build/panels-ss-x1000.log. Snapshots x1000, x1001.

## Committed on 2026-10-15 (minimum connection width)

KiCad's board setup has `min_connection` (minimum connection width) and
the project a severity for connection_width findings; the router chose
widths from the classes, the narrow-signal step's menu width and the neck
width without regard to it. Now `project_rules.rs` reads both
(`min_connection_mm`, `connection_width_severity` in the router config;
KiCad's default severity is a warning, board_design_settings.cpp) and,
where it applies, floors every track width: the class widths, the
narrow-signal width (the menu's smallest width raised to the floor), the
neck width; `route_kicad_board` and the lowering raise them again
(`apply_width_floor`, `width_floor`), so a configuration file's rules and
requested net classes keep it; stubs, escapes, manual and inner spokes
take their net's class or neck width; the neck repair's tracks already
were the minimum rounded up to a micron. main.rs's file merge copies the
floor from the project. `board-router.json` has a new `rules` object:
`width_floor_mm`, `connection_width_severity`, `narrowest_track_mm`,
`neck_width_mm` (WLED: 0.127 / error / 0.127 / 0.127).

The choice: the floor applies at error severity only, by default
(`PCB_MIN_CONNECTION_FLOOR=warning` adds warnings, `off` none). Measured
with warnings floored too (x1020), it cost routing on the only boards
where it matters (below). Census of the 109 v6 tasks
(`experiments/min-connection/census.py`, mirroring project_rules as of
x952; `emitted.py` counts the routed tracks below the minimum in a run):
18 have a `min_connection`, all with an explicit severity (3 error, 15
warning); a router width below the minimum only on two, both warnings:

| board | min_connection | severity | narrowest class | narrow step | neck | below the minimum |
| --- | ---: | --- | ---: | ---: | ---: | --- |
| OpenESC-20x20 4in1-mini | 0.16 | warning | 0.09 | - | 0.1 | Default 0.09 (every track); neck 0.1 |
| OpenESC-30x30 4in1 | 0.16 | warning | 0.09 | - | 0.1 | Default 0.09 (every track); neck 0.1 |
| OpenESC-30x30 4in1-panel | 0.16 | warning | 0.16 | - | 0.16 | - (no v16 result) |
| USB_Keypad, WLED_Matrix | 0.127 | error | 0.127 | - | 0.127 | - |
| USB_Cable_Tester | 0.1 | error | 0.1 | - | 0.1 | - |
| MokyaLora | 0.1 | warning | 0.1199 | 0.1199 | 0.1 | - |
| Sisu_SSE-9 | 0.1 | warning | 0.1 | 0.1 | 0.1 | - |
| reCamera | 0.1016 | warning | 0.1016 | - | 0.1016 | - |
| OpenFC, OpenRX panel | 0.09 | warning | 0.09 | - | 0.1 | - |
| 0xCB-1337 pcb, pcb-panel | 0.09 | warning | 0.1 | 0.2 | 0.1 | - |
| BAGEL-MK1, MK1.1-Avocado | 0.1 | warning | 0.127 | - | 0.1 | - |
| OpenAirScope | 0.1 | warning | 0.2 | - | 0.14 | - |
| MegaDrive | 0.1 | warning | 0.25 | - | 0.1 | - |
| SUMEC_MK_IV | 0.01 | warning | 0.25 | 0.25 | 0.1 | - |

On the v16 layouts only the two OpenESC boards have tracks below the
minimum (all 2530 and 2483 of them at 0.09; the designer's are 0.16 and
up, mostly 0.2). Route mode on the v16 placements, tasks v6, one run per
side, 2 jobs x 2 threads (build/minconn-route-x952, -x1020, -x1021;
connection_width counted apart, KiCad's report caps it at 199, necks.py
counts all; no other errors on any side):

| board | side | unconnected | connection_width | vias | length | work |
| --- | --- | ---: | --- | ---: | ---: | ---: |
| 4in1-mini | x952 (no floor) | 52 | 199 warnings (1007 by necks.py) | 157 | 1770 | 835 |
| 4in1-mini | x1020 (floor 0.16) | 89 | 0 | 126 | 1508 | 1172 |
| 4in1 | x952 (no floor) | 104 | 199 warnings (709) | 123 | 2073 | 903 |
| 4in1 | x1020 (floor 0.16) | 121 | 0 | 124 | 2003 | 1111 |

0.16 tracks at the class's 0.09 clearance need a lattice pitch of 0.25
against 0.18: 37 and 17 more open items for the warnings gone, so the
floor is not shipped there. x1021 (the commit, rebased on 851a473, error
severity only) gives x952's rows exactly on both. The error-severity
boards (WLED, Keypad, Cable_Tester) already route at or above their
minimum, so the floor changes nothing in their routing today; it guards
other class or narrow-step widths (a requested net class, the narrow step,
a neck) from going below.

Short tier (flock, tasks v6) x1020 (on 6099e13) against
build/short-x941.log: unconnected 35 to 35, new errors 12 to 3, vias 1129
to 1129, length 22253 mm both, passes 6 to 6, work 2671 s both, cut 0.
The changed rows are x950's neck repair, not this: WLED_Matrix
connection_width 13 to 1 (+11 to +2 new errors), USB_Keypad 4 to 2;
against build/short-x950.log all 12 rows are identical (routed, vias,
length, KiCad's counts, work, first-route open terminals). No short-tier
board has a width below its minimum (reCamera's and OpenFC's classes are
at it), so the warning choice does not move them either. Logs
build/short-x1020.log, build/minconn-route-x{952,1020,1021}.log.

Unit tests: `the_connection_width_severity_decides_the_floor`,
`the_minimum_connection_floors_the_project_widths` (project_rules),
`the_minimum_connection_floors_classes_the_narrow_width_and_the_neck`,
`a_routed_board_has_no_track_narrower_than_the_minimum_connection`
(board_router; lowers and routes a board with a forced neck-down).

Open: OpenESC's warnings (700-1000 necks of our own 0.09 tracks) remain;
a middle way would be the floor where it costs nothing (a last pass
widening tracks to 0.16 where the clearances allow, as the neck repair
does), not built.
## Committed on 2026-10-15 (outline polygons)

Gap 5 of the walkthrough (open problem 8): `outline` in constraints.json
took only a rectangle. Snapshots: x1040 = main at 851a473 (the base),
x1041 = this.

**What an agent can write** (docs/constraints.md, "Outlines that are not
rectangles"; agents.md's cookbook):
- `"polygon": [[x, y], ...]` (mm, closed implicitly, its points' box
  moved to `x`, `y`), checked: three corners, an area, no side crossing
  another (input errors, exit 2);
- `"corner_radius"` on the rectangle, the polygon, a piece or a cutout:
  every corner (inner ones too) an arc, the radius cut where a side is
  too short (a 30 x 4 mm outline with radius 5 is a slot with round ends);
- `"cutouts"`: boxes or polygons relative to the outline's `x`, `y`
  (`"origin": "absolute"`), inside their outline and apart from each
  other;
- `"pieces": [outline, ...]`, each with its own offset, radius and
  cutouts; they may not overlap or touch. No tabs are drawn.
A rectangle without radius or cutouts takes the old path (one gr_rect);
the automatic size and `shrink` stay rectangles (`area_factor`, `aspect`,
`shrink` with a polygon or pieces are errors; cutouts need a size).

**Writing** (`apply_shaped_outline`, placement_constraints.rs): every
contour as `gr_line` and `gr_arc` (start, mid, end; nanometre-rounded so
the pieces chain), stroke 0.05 mm, replacing the board's Edge.Cuts
graphics as before. `outline::board_loops` reads them back (the router's
and the placer's boundary, unchanged), and now also keeps the pieces'
own loops (`piece_loops`) and which cutouts the board's own graphics draw
(`board_cutouts`, as opposed to a footprint's slot).

**The placer.**
- *Edges on a polygon* (`Constraints::sides`, set by the lowering when
  the constraint's outline or one of its pieces is a polygon): each
  segment of the pieces' loops faces the side its outward normal points
  along most (`outline_sides`; 45 degrees counts as top or bottom). An
  edge constraint holds the part at the nearest segment facing its edge,
  by box-to-segment distance (`side_lines`): the line is the segment's
  innermost coordinate over the body's stretch (a slanted side: where the
  body's corner meets it), and a body not alongside the segment misses by
  the gap (`hard_violation` adds it). `edge_frame` takes the body's half
  extent for this; every caller had it. `any` measures the same way
  (`side_distance` in `choose_any_edges`). Rectangles, rounded ones, ones
  with cutouts and pieces that are all rectangles keep the box (or the
  piece boxes): nothing changes for them. On urchin's L (below)
  SW_POWER1 "top" goes to the step at y = 30; against the box it would
  have had no place in the right half.
- *Cutouts keep bodies off* (`Constraints::solid_cutouts`): cutouts the
  board's own graphics draw when the constraints give an outline (the
  outline constraint replaced all others) are tested against bodies by
  their polygon in `clear_of` (`body_meets_solid_cutout`; touching is
  allowed) and penalised by their box in the anneal; a footprint's slot
  still keeps only copper off (a connector's pegs).
- *Room*: the usable area subtracts the solid cutouts' area (the polygon's
  own area was already used); `outline_mm` and the suggestions stay boxes.
- With an outline from the constraints, a part whose centre lies off a
  polygon (in an L's notch) counts as off the board (placed, not kept).
- Pieces feed `keep_on_pieces` and the per-piece edge frames as a board
  drawn with several pieces did.

**Test boards** (urchin, the walkthrough's fixture and constraints, 30 s
of work, four threads, `PCB_ROUTER_NO_WALL_GUARD=1`): spell_tome's
schematic is not in the repo (its licence is NOASSERTION, and its
artwork footprints are 4 MB), so the test uses urchin (MIT).
- L-shaped polygon, 230 x 125 mm box less a 115 x 30 mm notch (the right
  half 30 mm lower), each half's region in its part: exit 0 in 15 s
  (18 s of work), 68/68, KiCad 0 unconnected, 0 new errors, every
  constraint kept; SW_POWER1 (edge top) at y = 30.02, the step.
- 230 x 95 mm with 5 mm round corners, a 10 x 55 mm slot between the
  halves and a triangular window: exit 0 in 23 s (30 s of work), 68/68,
  0 unconnected, 0 new errors, no body in the slot; four gr_arc.
- drc-kicad-board on both: exit 0, no `invalid_outline`, no
  `copper_edge_clearance` (only the fixture's library findings, and two
  silk_edge_clearance warnings on the rounded board).
- `"edge": "any"` on the L: chosen by the nearest facing side, kept.

**Tests**: `outline_shapes_parse_and_mistakes_are_errors`,
`outline_shapes_are_written_as_edge_cuts_lines_and_arcs`
(placement_constraints), `a_shaped_outline_gives_the_placer_its_sides_pieces_and_solid_cutouts`
(board_placer), `a_part_held_at_the_top_of_an_l_keeps_to_its_step`,
`a_cutout_the_user_cut_keeps_bodies_off` (pcb-placer tests), and
`the_keyboard_lays_out_on_an_l_and_on_a_board_with_round_corners_and_cutouts`
in tests/agent_walkthrough.rs (both layouts above, about 35 s).

**Identity** (rectangles: the tasks give no `outline`, so no new path
runs). place-kicad-board with the short tier's prepared sources and
layout.json (build/short-x1000, tasks v6, 120 s of placement work, four
threads), x1040 against x1041: byte-identical boards on all 12 short-tier
boards. Short tier on x1041 (tasks v6, 2 jobs x 4 threads, under the
lock, `build/short-x1041.log`) against build/short-x1000.log: compare.py
same 12, unconnected 35 to 35, passes 6 to 6, vias 1129, length 22253 mm,
work 2671 s, first-route open terminals 61, every row field identical
but WLED_Matrix's `native` (connection_width 9 to 0: new errors 12 to 3
in all); every layout's placed board byte-identical, every result board
too except USB_Keypad's and WLED_Matrix's, which carry the neck repair's
added tracks (2 and 12, their layout logs' "connection width" lines).
That is main's neck repair (70279ba, after x1000's base 9ce95b1), not
this change: x1000 predates it, as x941 to x950 measured (12 to 3).

## Committed on 2026-10-15 (schematic tier)

The agent's real use case as a benchmark (open problem 8, item 7): the
harvested boards' schematics, and a tier that starts from them.

**Harvest.** `benchmarks/github/harvest_schematics.py` fetches, at each
board's origin commit, the root sheet and every sheet it names, the
project file and `.kicad_dru`, the library tables, the symbol libraries the
tables keep in the repository and the footprints the symbols name from the
repository's footprint libraries (Git submodules followed: 19 boards keep
their libraries in one). Libraries a table names by a designer's absolute
path (`/home/seb/dev/...`), or that lie outside the project folder, are
found in the repository by path suffix, folder or file name and recorded as
`library_fixes` (22 boards). Files go to
`benchmarks/real/external/github-sch/<board>/repo/` (gitignored), the
per-board record to `schematic.json` there and to the committed
`benchmarks/github/schematics.json`. Licences: GitHub's SPDX id where it
has one; `NOASSERTION` and none are decided by reading the licence files
at the root and along the board's folder (CERN OHL, AGPL and Hippocratic
texts GitHub does not name were allowed; no licence file at all means all
rights reserved and the board is skipped). **79 of the 109 boards have a
usable schematic** (241 sheets, 1171 files, 95 MB). Skipped: 17 without a
licence file (ISS-PCB's six, seven OnBoard boards, MegaDrive, Neotron-Pico,
SUMEC, ATAT1800, vd-rd's module), 10 without a schematic of the board's
name (PolyKybd's five molecules, the OpenESC, OpenRX and two 0xCB panels,
krishveercard, whose sheet is misspelt), 2 with a sheet missing at the
commit (the Hub, poly_corne_split42_right), 1 KiCad 5 schematic (Qfwfq).
50 of the 79 have every named footprint in KiCad's libraries or the
repository.

**Imports of all 79** (x1002, import and constraints only): 76 imported,
3 failed (osprey, hackrf, rosco_m68k: KiCad 5 arcs, below); 33 complete
after the library step (2 of them only through it), 43 with footprints
missing: personal libraries that are not in the repository (Castor's
`winterbloom` 160, DSP-ADAU1452 371, jetson's `antmicro-footprints` 121,
reCamera's `Capacitor`/`Resistor` 92, OpenMuscle's `FlexSensorFootprint`
60, ligra's `otter` 33, ...), hackrf's KiCad 5 `.mod` library (395),
footprints a KiCad library no longer has (OpenESC's `C_0402_WIDE`,
Blue_Line's `CONN3_S3B-PH_JST`), symbols without a footprint (framework
mobo's RN1). Two panels (Batch_1, SmartSpin2k) import empty: their
schematic is not the panel's.

**The tier** (`run.py --tier schematic`, `benchmarks/agent-tasks/schematic.py`,
`schematic.json`; README.md, "The schematic tier"): per board, import the
copied project with the designer's layer count, add the found libraries and
import again when footprints are missing, move tasks v7's constraints onto
the new footprints (matched by symbol path, else by reference; `fixed`,
netless and locked parts as `place` at the designer's spot; every part on
the designer's footprint layer with `back`/`front`), draw the designer's
Edge.Cuts, copy the netless board-only footprints (fixed, `board_only`),
`layout-kicad-board` at a third of the budget, `drc-kicad-board`. Pass:
the layout rule and no footprint missing; parity counted. Rows have
`stopped` (import, constraints, placement, routing, drc) and `why`.

First run on x1002 (4 jobs x 2 threads, load 9-21, `build/sch-x1002.log`):
4/12 pass in 1034 s wall; stopped at routing 6, drc 2. Passed:
spell_tome, 0xcb-static (after the library step), slab-pcb-left,
homebrew_486, each as in the layout tier. The other rows are in the README.

**Fixed in the branch** (x1030 = this branch's binary):
- *KiCad 5 library footprints*: `(module ...)` files were imported under
  that head; pcb-maker reads only `footprint`, so the parts stayed at the
  import's point, unplaced and unrouted, without a word (coco2's 16
  footprints: 87 unconnected and 199 shorts; OpenFC's SD socket and a
  sensor: 17 unconnected). The import writes `footprint`. Arcs given by
  centre and angle (`fp_arc (start c) (end s) (angle a)`) became a board
  KiCad 10 cannot load (kicad-cli exit 3, no message; the import's pad check
  failed with exit 4 on osprey, hackrf and rosco_m68k); they are converted
  to start, mid and end. Tests `kicad_5_arcs_become_three_points`,
  `kicad_5_library_footprints_become_footprints_of_today`.
- *Libraries outside the project were copied outside the output*: a
  `${KIPRJMOD}/../lib` entry was copied to `<out>/../lib` (the survey
  found four library folders written beside its output directories). Only
  libraries inside the project are copied now.
- *`back` reported missed on a board that kept it*: urchin's key sockets
  are front footprints with their pads on B.Cu; `back` flips them to B.Cu
  as asked, but the kept check compared the part's side by its pads
  (front) with "back": all 34 were reported missed (exit 1). The side to
  keep is now the one the flipped footprint's pads give, as lowered.
  `describe-kicad-board` lists `layer` (the footprint's own) beside `side`
  (where its pads are), so an agent sees which one `back` sets. Test in
  `constraints_kept_by_construction_are_reported`.

Rerun on x1030 of the boards the fixes touch (coco2, OpenFC, 0xcb-static,
spliced into the x1002 log, `build/sch-x1030-three.log`, after a
harvest refresh): **5/12 pass**. coco2 173/173 routed, KiCad 0 unconnected,
no new error, 614 vias, passed (x1002: 87 unconnected, 199 shorts; the
layout tier's v16 row 173/173 pass); OpenFC 80/82, KiCad 2 unconnected,
clearance 3 and annular_width 4 (x1002: 17 unconnected; layout tier 81/82,
1); 0xcb-static passed again, now with its library found in the project
folder on the first import (the refreshed harvest fetched the copy nearest
the project), 43 footprints substituted by name, which showed one more
gap: parity 44 (`footprint_symbol_mismatch` 43).
- *A substituted footprint kept its old name in KiCad 6 schematics*: the
  import rewrote the symbol's `Footprint` property in the copied
  schematic, but a KiCad 6 file repeats it per instance
  (`symbol_instances`, `(footprint "MX_Only:...")`), and that one wins.
  Both are rewritten now (x1031): the imported 0xcb-static's parity check
  (kicad-cli `--schematic-parity`) goes from 44 findings to the 1
  no-connect pin.

Unit tests (pcb-kicad netlist_import, placement_constraints, describe) and
`tests/agent_walkthrough.rs` pass. Snapshots `build/bin/pcb-maker-x1030`
(the rerun) and `x1031` (with the schematic rewrite); `x1032` is the
branch rebased on 8c20715 (main's x1042).
No layout tier was run on x1030: the import fixes change only boards with
KiCad 5 footprints, which no layout task has (their boards come from the
designer, written by KiCad); the side check changes the report (and so the
exit code and the pass rule) only where `back`/`front` name a footprint
whose pads are on the other side, which no task does.

## Committed on 2026-10-15 (wall guard stop, fewer layout checks)

Three items of the v16 triage ("Ranked": 2, 3, 4) and the work of an
unkept ladder (smaller hole 4). Binaries: x972 (the change on 9ce95b1, for
the layout pairs against x934), x976 (rebased on bd0babc: neck repair,
reserve floor; short tier), x977 (rebased on 8a1fd2c, the commit's code).

**Stop after the wall guard** (`WallStop`, `LayoutStep` in board_layout.rs).
Once the layout's wall-clock deadline passes, only the steps that make the
result run: labels, KiCad's check of the board kept, the report. Skipped,
each noted as a `layout` cut and logged once (`layout: past the deadline,
... skipped`): the placement race, move trials (checked before every
trial, not only every move: v16's poly_kybd_split72_left ran three trials
of 60-200 s past the guard and was killed), the resume, the polish, the
final ladder, the attempt after the narrow-signal attempt and the neck
repair (up to six KiCad checks; also skipped in the ladder past its
deadline, and its checks are counted apart in the log). In the router, a
negotiation that resumes an earlier one (resume, trial, via reduction)
starts no iteration past the deadline, and the pour stitching leaves
stranded pads instead of their hard reroute; a fresh router still
negotiates once. Without a cut nothing changes (every skip needs the wall
clock past the deadline). Checked on poly_kybd_split72_left, tasks v6,
4 threads pinned to two cores (`taskset`, beside other agents, load 14-27):
full budget x970 ended at 1719 s (harness 1800 s) with a result, 29
unconnected, 981 vias, cut deadline+layout: the guard fired in the
narrow-signal attempt's forced repair, the attempt finished and was
checked (it could win and won), the continuation was skipped, `layout:
done` at 1688 s. At `--budget-scale 0.3` (timeout 750 s, deadline 495 s)
x970 and x934 both ended at 520-525 s (cut in the first route, 368 and
483 open after it, so no moves or ladder to skip). Unit tests: a fake step
list (`past_the_wall_guard_only_the_finishing_steps_run`,
`once_fired_the_guard_stays_fired`).

**Fewer KiCad checks in the ladder** (`CheckGate` in board_router.rs;
docs/router.md "The attempt ladder"). Attempts route without KiCad's check
and are checked only when they could still win. The ladder's open count is
the router's (unconnected terminals and internal violations) plus KiCad's
unconnected items, so the router's count is a lower bound with no slack:
an attempt whose router count is above the best checked open count (or
equal, with no starved thermal on the best and no fewer vias and copper)
cannot win. The ladder's first board and any board the router finishes
are always checked; a seed retry against an unchecked rung prefers the
checked board (exact otherwise). Route mode uses this rule alone (no
decision of the ladder changes). The layout's final ladder (and the
narrow-signal attempt and its continuation) also checks only attempts
among the two best by the router's measures (open count, vias, copper),
the layout's own board counted first (`KICAD_CHECK_TOP`); a first version
ranked by the open count alone and lost MokyaLora's 333-via narrow step
(339 kept), fixed by the vias tie-break. Past the deadline an attempt is
checked only if it could beat both the ladder's best and the layout's
board (`kicad_kept`), and an unchecked ladder board is never kept. Log:
`board check: KiCad skipped, ...` per attempt, `layout: done (...); KiCad
checks N (from the DRC cache, of the neck repair), M skipped`. Unit tests
`check_gate_tests` (5).

**Unkept ladder work counts**: the layout's `routed.work_seconds` is the
ladder's work (and the continuation's) whether its board is kept or not,
as the narrow path did since 974fa2b (koeg in v16 reported 410 s with
1100 s more run). Rows' `work_seconds` rise where a ladder ran and lost;
results do not change.

**run.py threads**: the default with several jobs is the physical cores
(from /sys, else lscpu) over the jobs, at most 4, at least 1 (was the
logical CPUs: 5 jobs got 4 threads on 16 cores, now 3; 6 jobs got 4, now
2). An explicit `--threads` is kept; jobs x threads above the physical
cores is warned about. `summary.json` records `jobs`, `threads`,
`physical_cores`. A layout is deterministic per thread count: compare at
the same `--threads` (give `--threads 4` against older short and quick
tier logs).

Layout pairs, tasks v6, 4 threads, one run per side, PCB_MAKER_NO_CACHE=1,
x934 against x972 side by side, load 9-39 (rows in build/guard-ab-x934.log
and build/guard-ab-x972.log; the poly_kybd runs in build/guard-poly-*.log). KiCad checks are the DRC calls
of the layout (seconds of kicad-cli in brackets):

| board | unconnected | errors | vias | work s | wall s | cut | KiCad checks |
|---|---|---|---|---|---|---|---|
| ghoul | 6 / 6 | 0 / 0 | 385 / 385 | 1208 / 1208 | 1088 / 1079 | no / no | 7 (188 s) / 4 (88 s) |
| 0xCB panel | 109 / 109 | 0 / 0 | 231 / 231 | 362 / 362 | 1298 / 1052 | no / no | 10 (331 s) / 4 (163 s) |
| Batch_1 | 491 / 491 | 0 / 0 | 384 / 384 | 1156 / 1156 | 1262 / 1377 | no / no | 5 (203 s) / 2 (108 s) |
| ULK | 4 / 4 | 0 / 0 | 130 / 130 | 98 / 98 | 1679 / 1572 | deadline / no | 13 (370 s) / 7 (205 s) |
| woagboard | 5 / 5 | 1 / 1 | 185 / 185 | 1131 / 1540 | 1665 / 1641 | deadline / no | 12 (127 s) / 8 (70 s) |
| MokyaLora (pass) | 0 / 0 | 0 / 0 | 333 / 333 | 1365 / 1365 | 944 / 812 | no / no | 7 (61 s) / 3 (22 s) |
| zpn_devboard (pass) | 0 / 0 | 0 / 0 | 417 / 417 | 522 / 522 | 595 / 466 | no / no | 1 / 1 |
| Castor mainboard (pass) | 0 / 0 | 0 / 0 | 167 / 167 | 924 / 924 | 829 / 721 | no / no | 2 / 2 |

All eight boards identical (unconnected 615, 1 new error, 2232 vias,
47510 mm both); KiCad checks 57 to 31, kicad-cli 1297 to 676 s; CPU 14495
to 12643 s. Cut 2 to 0: on x934 the wall clock cut woagboard's and ULK's
last rung (both boards unchanged; woagboard's counted work 1131 against
1540 s is where the cut stopped that rung, ULK's 98 s leaves out its
unkept ladder, the accounting fixed above came later). Wall clock follows
the load (Batch_1 ran beside load 22-26 on x972, 9-23 on x934); the
checks saved are 100-170 s a board on the big ones.

Short tier (flock, tasks v6, `--threads 4 --jobs 1`, DRC cache warm), x976
against build/short-x1020.log (x1020 = x950's rows; x1022 changes none):
all 12 rows identical (unconnected 35, new errors 3, 1129 vias, 22253 mm,
passes 6, cut 0). Work 2671 to 3236 s: framework_mobo 220 to 292 and OpenFC
140 to 634, their unkept ladders now counted; every other row's work the
same. KiCad checks 62 to 41 (rung checks 53 to 32; the neck repair's 7 on
USB_Keypad and 2 on WLED_Matrix both sides): USB_Keypad 20 to 14,
reCamera 14 to 4, WLED_Matrix 11 to 9, OpenFC 6 to 3, the rest unchanged.
Wall 1702 to 1946 s summed over the rows (the cache made the checks
0.5 s each, so the saving is nil there; jobs 1 beside other agents' tiers
against x1020's run). build/short-x976.log.

Not done: the uncached KiCad check's time is still not work (the gate cuts
the number of checks, not their charge); the ladder's first board is always
checked (it is the ladder's measure), even where it cannot beat the
layout's board.

## Committed on 2026-10-15 (hatched pour work)

Question: on main's constants (18 / 26 / 42 ns), do pour analyses on
hatched boards still run far from their modelled work? Route mode on the
v16 placements, tasks v6, one thread, `PCB_ROUTER_POUR_TRACE`, x952 (main
6099e13), all eight boards at once (load 12-24); "model / wall" is the
traced analyses' modelled work over their wall time, the profile column
sums the `profile work:` lines' pour wall and work:

| board | zones | analyses | wall s | model s | model / wall | profile pour wall / work s |
|---|---|---|---|---|---|---|
| Batch_1 | hatched | 442 | 282.5 | 105.4 | 0.37 | 167.3 / 71.0 |
| WLED_Matrix | hatched | 723 | 44.3 | 61.5 | 1.39 | 38.7 / 53.5 |
| USB_Keypad | hatched | 571 | 38.2 | 53.5 | 1.40 | 33.7 / 46.8 |
| Qfwfq | hatched | 309 | 20.9 | 19.2 | 0.92 | 20.5 / 19.0 |
| 1337-v4.0 | hatched | 452 | 14.4 | 10.2 | 0.71 | 14.1 / 10.0 |
| A13 module | hatched | 2250 | 31.2 | 41.1 | 1.32 | 26.1 / 34.8 |
| Castor | solid | 499 | 34.6 | 33.7 | 0.97 | 29.1 / 28.1 |
| Sisu | solid | 1154 | 50.2 | 51.1 | 1.02 | 40.3 / 43.3 |

The controls sit inside 0.8-1.2, the hatched boards outside on both sides
(a repeat with the trace build, x1011, load 13-32: Batch_1 0.34, WLED
1.60, Keypad 1.83, A13 1.76, 1337 0.81, Castor 0.99, Sisu 1.05). Batch_1
is still about 3 times under on main (the v16 triage's 755 / 200 s was a
four-thread layout on x862's constants). What the items-only model
misses, from trace counters per analysis (thread-local: route mode runs
two ladder rungs side by side even at one search thread, so process-wide
counters mixed them, as x1010's did):

- The probe's queries (a point tested against its index, `blocker` and
  `slack`). A query costs about a microsecond whatever its items (the 9-25
  bucket lookups, the `edges` vector, `inside`'s nearest-node and polygon
  test); a hatched probe asks many more of them, with few items each in
  open fill: a hatch hole judged (`hole_kept`, memoised per probe; 1.8-2.0
  k on WLED and Keypad) samples 64 points with `fill_at` and `near_edge`,
  up to some 2000 queries, and the ring test (`in_ring`) up to 33 per pad
  of the net (WLED 24 M queries, 400 k ring tests). Per
  item the probe's wall time was 21-34 ns on WLED and Keypad (many items
  per query) and 51-81 ns on 1337, Qfwfq and Castor; with a query term
  it is one model.
- Polygon vertices. Batch_1's queries cost 5.5 microseconds and its items
  220 ns: its board cutouts are `Hole` obstacles of up to 134 vertices
  (x1012 dump), and every shape test near them walks the polygon twice;
  15.9 G vertices in 442 analyses. Not the hatch itself, but the hatch
  multiplies the queries near them. The ring test's own pads are 0.8 G of
  those (x1014), `inside`'s polygon fallback 0.1 G: not it.
- So the hole grid and the rings cost through their queries, not as a
  term of their own; the per-zone seam counting (`in_zone_part` on
  4-point outlines) is small. The
  lattice part (fill mask, labelling, stamps) was over-counted instead:
  A13's analyses spend 17.6 s outside `kicad_spokes` for 41.1 s of node
  and stamp work (486 k stamps at 26 ns), Sisu 38.2 for 51.1, which the
  items-only fit balanced against `kicad_spokes`' uncounted rest.

Change (`fill::FillProbe` counts `queries` and the `vertices` its shape
tests go through, `Router::probe_nanos`): `PROBE_QUERY_NANOS` 850,
`PROBE_VERTEX_NANOS` 9, `PROBE_ITEM_NANOS` 42 to 9, `POUR_NODE_NANOS` 18
to 23, `POUR_STAMP_NANOS` 26 to 6. Fitted (least squares of the log
ratio, every board one weight) on the eight boards and the four other
boards of the x816 calibration (reCamera, k30-SBC, USB_Cable_Tester,
spell_tome; x1013, route mode at budget scale 0.3, load 10-17): per board
model / wall Batch_1 1.03, WLED 0.97, Keypad 0.97, Qfwfq 1.01, 1337 0.91,
A13 1.01, Castor 1.02, Sisu 0.89, reCamera 1.12, k30 1.05,
USB_Cable_Tester 1.00, spell_tome 1.41 (1.1 s of analyses; it was 1.11
with the old constants). The pour trace now prints `probe queries`,
`probe vertices`, `probe work` and whether the net has a hatched zone.
Test: `a_fill_probe_counts_its_queries_and_polygon_vertices`;
`pour_analyses_count_as_budget_work` takes the probe's whole work.

Judge, route mode, the same eight boards one run per side, x952 against
x1016 (6099e13 with this commit), one thread; the after side at load
22-24:

| board | result (both sides) | model / wall before, after | clean-ups: work (pour) of budget in wall s, before; after |
|---|---|---|---|
| Batch_1 | cut (budget guard), 499 open, 345 vias, 5180.1 mm | 0.37, 0.87 | 57.2 (19.5) of 293 in 71.5; 73.3 (35.6) of 293 in 58.7 |
| WLED_Matrix | 6 open, 143 vias, 2139.8 mm | 1.39, 0.86 | 55.1 (27.3) of 240 in 37.5; 47.9 (20.1) of 240 in 46.0 |
| USB_Keypad | 2 open, 79 vias, 1555.7 mm | 1.40, 0.86 | 93.7 (30.6) of 360 in 61.9; 83.1 (19.9) of 360 in 66.7 |
| Qfwfq | 0 open, 62 vias, 3787.2 mm | 0.92, 0.81 | 83.9 (10.8) of 240 in 72.3; 85.1 (11.8) of 240 in 87.7 |
| 1337-v4.0 | 0 open, 66 vias, 1578.7 mm | 0.71, 0.83 | 41.9 (5.4) of 240 in 35.7; 43.9 (7.5) of 240 in 43.9 |
| A13 module | 14 open, 411 vias, 6015.1 mm | 1.32, 0.96 | 99.0 (27.9) of 196 in 68.9; 86.1 (15.3) of 196 in 50.3 |
| Castor | 0 open, 262 vias, 5593.9 mm | 0.97, 0.86 | 156.8 (21.0) of 209 in 132.5; 157.8 (21.9) of 209 in 146.1 |
| Sisu | 6 open, 388 vias, 5897.0 mm | 1.02, 0.90 | 170.1 (36.1) of 189 in 131.3; 165.9 (27.8) of 189 in 107.4 |

Every board's result is identical; the analyses' counts too but Sisu's
(1154 to 1156, its work 1455.3 to 1459.5 s): the clean-ups here end on
their passes, not on their budgets, so the counted work moves (Batch_1's
pour share 19.5 to 35.6 s, the hatched 1.3-1.4 boards' down by a quarter to a half)
without moving a decision. The after side's ratios sit 0.81-0.96 at the
higher load (the spread, 0.15, is what matters: before 0.37-1.40).
Batch_1's cut is the same on both sides and not the pour work: "budget
guard: 0 s of wall clock for a budget of 0 s of work", a phase with a
budget under half a second whose first analysis runs past four times it
before the work notices (one Batch_1 analysis is about a second of wall).
Its route-mode result (107/204 routed, 499 open) is that board's v16
placement, not this change.

Short tier (tasks v6, `--jobs 2 --threads 4`, under the lock, load 15-17)
x1015 (8c20715 with this commit) against build/short-x1041.log (main's
code): 12 of 12 rows identical (vias, length, work, KiCad; unconnected 35
to 35, errors 3 to 3, passes 6 to 6, work 2671 s on both). No clean-up
budget on the tier's hatched boards (1337-v4.0, USB_Keypad, WLED_Matrix)
ends by work. Against build/short-x941.log the rows differ by main's own
changes since (neck repair, placement work), not this one.
`cargo test --release -p pcb-router -p pcb-kicad` passes. Logs:
build/hatchwork-* (x952), build/hwtrace-*, build/tl-*, build/poly-*,
build/vx-*, build/val-*, build/ring-batch1 (traces), build/after-*
(x1016), build/short-x1015.log.

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
   Items 1-3 were done on 2026-10-15 (placement room, above); left of
   item 1: a crowded side (90-99 %) that no exact test calls hopeless
   still runs the levels up to the budget.
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
   5. *Outlines that are not rectangles.* Done on 2026-10-15 (outline
      polygons, above): polygon, round corners, cutouts, pieces. Left:
      tabs or mouse bites between pieces, an automatic size for a
      polygon (scaling it), and `room` suggesting a scale for a polygon
      rather than a rectangle.
   6. *A router starter config.* A board from a netlist has the
      project's net classes or the default 0.2 mm for every net. The
      import could write `router.json` beside `layout.json` with a power
      class for rails (by name and by `power_in` pin type) and, on two
      layers, `add_pours` GND on B.Cu, listed in the report for the agent
      to keep or drop. Small to medium: a day with the from-schematic
      benchmark to check it does not cost routability.
   7. *Schematics in the harvested corpus.* Done on 2026-10-15
      (schematic tier, above; its friction is item 9).
9. **The schematic tier's friction** (2026-10-15, above), ranked by
   boards affected; each with a proposal:
   1. *Personal footprint libraries are not in the repository*: 43 of 76
      imports miss footprints after the library step; in about 30 the
      library is nowhere in the repository (Castor 160, DSP-ADAU1452 371,
      jetson 121, reCamera 92, OpenMuscle 60, USB_Cable_Tester 57, ...).
      The designer's board has every one of them. Proposal:
      `import-kicad-netlist --footprints-from <board.kicad_pcb>`: a missing
      footprint is taken from that board (by the symbol's path, else by
      footprint id), brought back to library form as KiCad's "Export
      footprints to library" does (position and rotation removed from
      pads and texts, a back footprint flipped to the front). An agent
      handed an old board, or a colleague's, then imports completely.
      Medium: a day; the inverse transform is the work, the flip exists.
   2. *Library drift*: the import takes today's footprints; 22 of the 79
      boards have parts whose body differs by more than a fifth from the
      designer's (KiCad 10's ESP32-S2-MINI-1 and ESP32-S3-WROOM-1 with
      their antenna clearance in the courtyard, 45 x 35 mm against
      16 x 21; SOT-323, SOT-23 and crystal courtyards; bumwings' updated
      hotswap sockets on 54-64 keys). laptop power cannot be placed
      (left out of the tier), bumwings and urchin lose a pass. Proposal:
      the same option with "prefer the board's footprint" for footprints
      whose library version differs, and a `drifted` list in the import
      report (library body against the board's) so the agent knows.
      Small on top of item 1.
   3. *Libraries outside the project or named by absolute paths* (22
      boards): the import says "no library X", an agent has to find the
      folder. The tier does what an agent would (adds `library_fixes` to
      the fp-lib-table and imports again: 0xcb-static 43 missing to 0,
      Glasgow 24 to 2, WLED_Matrix 135 to 1). Proposal: the import's
      `missing` line names candidates: a folder `<library>.pretty`, or one
      holding the footprint's file, under the project's ancestors up to
      the repository root (`.git`), and the longest existing suffix of an
      absolute table path. Not used silently: two board revisions often
      keep libraries of one name (0xCB's rev3.0 and rev5.0). Small: half a
      day.
   4. *Board-only parts* (the board has them, the schematic does not):
      mounting holes, logos and connector shells without nets are copied
      by the tier; parts with nets are lost (0xcb-static's 11 alternative
      switch footprints for other layouts, MX37_1 and the like;
      USB_Keypad's four mounting holes plated on GND). An agent needs
      mounting holes without drawing them into the schematic. Proposal: an
      `add` entry in constraints.json, `{"add": [{"footprint":
      "MountingHole:MountingHole_3.2mm_M3", "x": 3.5, "y": 3.5}]}`, written
      to the board `board_only` and fixed. Small to medium: a day.
   5. *KiCad 5 libraries*: `module` files and centre-angle arcs fixed
      (above); a legacy `.mod` library (hackrf's 395 footprints) is still
      unread. Proposal: a table entry of type `Legacy` is converted with
      `kicad-cli fp upgrade` into a temporary `.pretty` before the
      lookup. Small.
   6. *A board KiCad cannot load says nothing*: kicad-cli exits 3 with
      only Debug lines; the import and the layout report "kicad-cli failed
      with exit status: 3: <a Debug line>" (an Edge.Cuts circle written end
      before centre, a KiCad 5 arc). Proposal: on exit 3 say "KiCad could
      not load the board" and name the first item pcb-maker's own reader
      finds out of KiCad 10's order or form, where it can. Small.
   7. *Panels and boards without a schematic of their own* (10 boards,
      plus two panels whose schematic is a single copy): not targets.

## Where things are

| What | Where |
| --- | --- |
| Sweep logs and outputs | `build/github-layout-v11*` (layout), `build/github-freerouting*`, `build/github-ours-cold*`, `build/github-route-v16*` |
| Snapshot binaries | `build/bin/pcb-maker-xNN` (x155 = HEAD); `cargo build --release` first, `cargo test` leaves `target/release` stale |
| Sweep chain | `build/chain-v11.sh` (layout sweep, then the route sweep) |
| Tasks | `build/github-tasks-v5/` (from `benchmarks/agent-tasks/from_pcbench.py --corpus benchmarks/github/boards.json`) |
| Harvested boards | `benchmarks/real/external/github/` (not in git; `benchmarks/github/manifest.json` has origin and licence); their schematics in `benchmarks/real/external/github-sch/` (`benchmarks/github/schematics.json`) |
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
