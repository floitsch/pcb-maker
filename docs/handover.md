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
rows identical to x722, 22 pass, CPU geomean 0.97. The waves' count follows
the cores the process may use, so a run pinned to fewer cores can end
elsewhere than an unpinned one. Left: the clean-up's pour checks are not
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
0xCB U3.45 thermal vias inside the exposed pad (10 misses; modelling them
made 15 false alarms); seams between overlapping same-net zones of
different priority (Telemetry, 5); marginal fine-pitch clearance within a
few microns; hatched fill lying over thermal pads without relief (not
understood); per-track DRC subtleties with islands.

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
