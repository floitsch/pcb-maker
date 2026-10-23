# Routes and placements closer to humans (2026-10-22)

Florian's brief: "investigate alternative ways to route/place that get us
closer to (or surpass) humans. There shouldn't be any reason humans can do
this better than we can." Five ideas came with it; this page says for each
what exists, what the failure data says about its potential, and what it
would cost, then reports the prototype chosen (the trouble-makers'
repair, `PCB_ROUTER_TROUBLE`, off) with its numbers and a recommendation.
Branch `human-level-routing`; snapshots x1580 (on bda6ffa) and x1581 (on
ec60853).

## The failure data

The five gap boards of the v18 sweep (docs/benchmarks.md, "Layout sweep
v18"), what is open, from the routers' own reports (`board-router.json`:
unconnected terminals per net) of the designer tier (x1315, the designer's
placement, 1500 s) and the v18 layouts (x1315, our placement, its final
board). A *bus net* here is one of at least four nets with three to twelve
pads that join exactly the same set of footprints (address and data lines
of memories and their controller; `exp1-bus.py` in the session, a grouping
by footprint set from the board file).

| Board | designer placement: open terminals (on bus nets) | our placement: open terminals (on bus nets) | what is open on ours |
| --- | --- | --- | --- |
| video | 0 (complete) | 290 (233) | TVRAM0-31 (10 pads each, 9 of 10 open), MXA, DQ, CAS |
| SNSP-CPU-01 | 28 (2) | 149 (33) | D0-7, PA0-7 (8 pads, 7 open), CA, GND, VCC |
| OpenESC 4in1 | 0 (complete) | 157 (0) | GND, motor phases (13 pads, 12 open), +3V3, +10V |
| Glasgow | 70 (0) | 212 (9) | VIOB (30 of 32 open), GND, +3V3, +5V |
| MegaDrive | 93 (70) | 222 (143) | VCCQ (37 of 54), VA0-15, VD0-15 (6-9 pads, nearly all open) |

Three things stand out.

- **Open nets are open whole.** A bus net that fails ends with nine of
  ten pads unconnected: the hard repair after the negotiation
  (`resolve_remaining`) rips up every conflicted branch and routes the
  nets again in the routing order (pour nets, then nets of up to eight
  pads, by span), with all other copper as walls. The ten-pad bus nets
  come last and find the room taken by the short nets routed before them.
- **The negotiation stops while it still converges.** MegaDrive on the
  designer's placement (x1315, route.log): each of the ladder's three
  attempts negotiates 10-13 iterations of about 20 s and stops at its
  share (a third of an even split of the budget), conflicted nets falling
  247 to 169 at about ten an iteration when it stops; the ladder ends at
  1007 s of its 1575 s bound. Half of an iteration's search is board-wide
  fallback searches after a corridor missed (279M of 598M expansions).
- **Buses carry the open count** where the gap is largest: 80 % of
  video's open terminals on our placement, 64 % of MegaDrive's (75 % on
  the designer's), 22 % of SNSP-CPU-01's (its D and PA nets join varying
  sets and do not group by this rule). OpenESC and Glasgow are open on
  supply and motor nets, not buses.

## The five ideas

**(1) Find the trouble-makers and concentrate on them.**
*Exists:* the negotiation itself reroutes only the nets in conflict, and
only their conflicted branches (PathFinder with history); the hard repair
rips up the conflicted nets and reinserts them against fixed copper; and
`force_connect` routes a net still open straight through other copper and
lets the displaced nets renegotiate, kept only when every net involved
ends complete. What does not exist is a repair that *trades*: an open bus
net of ten pads is never allowed to cost a short net one pad.
*Data:* the open nets are whole multi-pad nets, routed last by the
order; force_connect's all-complete rule refuses nearly every exchange
on a saturated board (MegaDrive: 38 forced, 39 nets still open). Addresses
the routing share of every gap board, most on MegaDrive, SNSP-CPU-01 and
Glasgow, whose designer placements leave 93, 28 and 70 terminals open.
*Cost:* small (a repair phase, a few hundred lines; no new data
structure). **Chosen**, as the trouble-makers' repair below.

**(2) Move parts without losing the traces.**
*Exists:* `Router::update` keeps every net whose pads did not move and
whose copper is still legal and rips up only the rest; the coupled loop
nudges congested parts and reroutes what the nudge touched
(docs/coupling.md). *Data:* on the gap boards the loop does not run: a
first route more than 5 % open (4 layers with pours) or 2 % (where the
narrow step applies) goes straight to the final ladder ("far from
complete"); a bounded round of moves there was measured on 2026-10-21
and lost (DSP-ADAU1452: each trial 224-285 open against 100, 89 s a
trial). The incremental machinery is there; what is missing is a move
worth making on a board 300 connections short, which is a placement
question. *Cost:* the mechanism exists; a better move generator is the
placement problem itself. Not chosen.

**(3) Zero-width first: count how many fit through a region.**
*Exists twice:* the router's tile planner (16 x 16-node tiles with
per-layer capacity and fill, corridors for the lattice search,
docs/router.md "Two-level search") and `pcb-global` (1.5 mm tiles,
boundary capacities in tracks, negotiated, 0.3 s a board;
docs/global-route.md). *Data:* as a routability measure it orders our
placement against the designer's right in 0.79 of the pairs (RUDY 0.52),
but not the placer's seeds of one board (0.33 within-board Spearman,
RUDY's level), and as the race's pre-ranker it made dense boards worse
(1096 to 1140 unconnected). Its strength is structure: video and
OpenESC, whose designer placements route completely. That is a use in
the placer's search (an anneal term or moves of the parts whose nets
cross the fullest boundaries), not in routing. *Cost:* medium to large
(placer integration, eleven dense layouts at full budget to judge).
Not chosen this round; the best placement lever on the list.

**(4) Group similar nets: route buses as bundles.**
*Exists:* nothing in the router (`bus_rows`, a placer option lining up
identical footprints that share a bus pin for pin, got video from 315 to
259 open and was removed on 2026-10-16 for costing mackerel-08). *Data:*
the largest share of the gap: buses hold 80 % of video's open terminals,
64-75 % of MegaDrive's on either placement. A bundle router (a river
route of the group along one planned path, split where it cannot pass)
is what a designer does on these boards. *Cost:* large: a new search
mode (bundle width in the tile planner, a member order per bundle, the
split), weeks, and its value depends on a placement that lines the bus
up (video's eight SIMM sockets). Not chosen; the repair below is a
cheap probe of the same failure (bus nets routed last and starved).

**(5) Pin swapping.**
*Exists:* `swap-kicad-pins` and `pin_swaps`/`swappable` in the layout
config (docs/pin-swap.md); on the fence boards it cut crossings 15-30 %
but finished no more nets than the hand assignment. *Data:* the gap
boards' open nets are memory buses, motor phases and supplies; swapping
memory address or data lines is legal electrically, but the benchmark
judges against the designer's netlist (KiCad's parity), video and
MegaDrive have hierarchical schematics the swap does not rewrite, and
nothing in the data says the stall is an assignment problem rather than
room. *Cost:* small to try where allowed, but it is not applicable to the
gap boards without changing the task. Not chosen.

**Ideas of our own the data points at.**

- **Keep negotiating the board that converges.** A designer keeps
  working the one board; the ladder splits its budget into fresh
  attempts (another pour mode, a seed round) whose negotiations each stop
  at a third of an even share while their conflicts still fall
  (MegaDrive above). Measured below as a configuration only (one long
  negotiation).
- **Escape first, then buses, then the rest.** Designers fan out BGAs
  and QFPs before routing, and route the buses before the short nets.
  Our order puts the ten-pad buses after every short net; open problem
  2 (one-spoke pads on fine-pitch rows) wants the escape directions
  decided before negotiation too. The repair below is the after-the-fact
  form of "buses first".

## The prototype: the trouble-makers' repair

`crates/pcb-router/src/router/trouble.rs`, called from `finish` right
after the hard repair (`resolve_remaining`), so in every attempt of the
ladder, the layout's first route and its final ladder alike. Off unless
`PCB_ROUTER_TROUBLE` is set; with it off no code path changes
(fingerprints below).

For each net still open (pour nets left out), most open terminals
first, in rounds until a round closes nothing or the work is spent:

1. Route the net again from what it has, at a price for sharing
   (`PCB_ROUTER_TROUBLE_PRESENT`, 20: a node of another net costs 21
   times its length), through other nets' copper where that is cheaper
   than the way around.
2. The nets it now overlaps are its victims (and, where the overlap is
   one-sided, a net of another class whose clearance zone reaches
   further, the nets whose stamps cover its conflicts). Refused when they
   are more than `PCB_ROUTER_TROUBLE_VICTIMS` (12), when one is a pour
   net, or when fixed copper is in the way.
3. The victims lose the branches the net crosses (a one-sided blocker
   all of its own) and are routed again hard, with all other copper as
   walls, the trouble-maker's new path included.
4. Kept when the open terminals of the net and its victims together drop
   and none of them is in conflict; otherwise every one of them is
   restored exactly (their branches, connections and stamps).

A net refused is tried again only after an exchange was kept since its
refusal. The work is the attempt's: at most half its negotiation's
(`PCB_ROUTER_TROUBLE_SHARE`, 0.5, at least 30 s), charged in the
resolve phase, inside every work bound and deadline the router has.
`PCB_ROUTER_TROUBLE_DEPTH=2` lets a victim that came out worse push
others aside in turn (not the trouble-maker); `PCB_ROUTER_TROUBLE_WHY`
prints every exchange's outcome.

What it is, against what was there: `force_connect` (the hard repair's
last step) also routes an open net through others and lets them move,
but keeps the result only if every net involved ends complete, and lets
the displaced nets renegotiate softly among themselves; on a saturated
board almost nothing passes that test. The trouble-makers' repair trades:
a bus net of ten pads may cost a short net one pad. That is the human
move the data asked for: the bus goes in first, the short nets find
another way.

## Measurements

Route mode on the five gap boards (run.py, one search thread a board,
the pinned work rates, the ledger's admission, beside other agents'
runs): on the designer's placement (`--tier designer`, 1500 s of work)
and on the v18 layouts' final placements (`--route-from`, the merged
`build/exp1-v18-placements`: v18 for video and OpenESC, v18b for the
other three; the route-mode budget, 1200 s). Two pairs, each the same
binary's main against the branch: x1560 (bda6ffa) against x1580 (its
first version), and x1543 (main ec60853, the routing of every main since
up to x1603) against x1582 (the version described above; on ec60853).
Each cell: KiCad's unconnected items (the router's open terminals) /
new DRC errors (strict) / vias / work seconds. Logs
`build/exp1-{designer,v18route}-<binary>.log`.

**The designer's placement:**

| Board | x1543 | x1582, `PCB_ROUTER_TROUBLE=1` |
| --- | --- | --- |
| video | 0 (0) / 0 / 603 / 681 | 0 (0) / 0 / 603 / 681 |
| SNSP-CPU-01 | 6 (13) / 0 / 1367 / 1206 | 4 (7) / 0 / 1339 / 1108 |
| OpenESC 4in1 | 0 (0) / 0 / 111 / 281 | 0 (0) / 0 / 111 / 281 |
| Glasgow | 55 (58) / 0 / 234 / 1458 | 49 (50) / 0 / 240 / 1575 |
| MegaDrive | 75 (100) / 0 / 1234 / 1122 | 59 (79) / 0 / 1366 / 1434 |
| sum | 136 | 112 (-18 %) |

**The v18 placements:**

| Board | x1543 | x1582, `PCB_ROUTER_TROUBLE=1` |
| --- | --- | --- |
| video | 235 (334) / 0 / 938 / 1217 | 214 (290) / 0 / 1212 / 1269 |
| SNSP-CPU-01 | 158 (177) / 0 / 1223 / 737 | 140 (146) / 0 / 1283 / 919 |
| OpenESC 4in1 | 112 (160) / 0 / 141 / 1010 | 104 (117) / 0 / 140 / 1189 |
| Glasgow | 159 (167) / 0 / 245 / 1231 | 148 (155) / 0 / 247 / 1260 |
| MegaDrive | 177 (223) / 1 / 1361 / 1078 | 135 (165) / 1 / 1504 / 904 |
| sum | 841 | 741 (-12 %) |

The first pair (x1560 against x1580) says the same: designer placements
134 to 112, v18 placements 825 to 738 (video 219 to 211, SNSP-CPU-01 158
to 140, OpenESC 112 to 104, Glasgow 159 to 148, MegaDrive 177 to 135;
designer: SNSP-CPU-01 6 to 4, Glasgow 53 to 49, MegaDrive 75 to 59).
Every board that leaves anything open gains, on both placements, in
both pairs; none loses; the boards complete without the repair (video
and OpenESC on the designer's placement) are unchanged, the repair never
running there. MegaDrive's one strict error (a copper edge clearance)
is on both sides. No row was cut.

**Layouts** (the whole flow at the task budget, two threads a board;
x1592, main 313b920, routing as x1543, against x1582 with the switch;
the placements are byte-identical on both sides, the repair touching
only routes; logs `build/exp1-layout-*.log`):

| Board | x1592 | x1582, `PCB_ROUTER_TROUBLE=1` |
| --- | --- | --- |
| video | 199 (328) / 0 / 1505 / 1474 | 185 (308) / 0 / 1558 / 1354 |
| SNSP-CPU-01 | 147 (160) / 0 / 1692 / 1435 | 128 (146) / 0 / 1802 / 1453 |
| OpenESC 4in1 | 127 (187) / 0 / 135 / 1398 | 120 (154) / 0 / 141 / 1500 |
| Glasgow | 191 (214) / 0 / 319 / 1390 | 190 (213) / 0 / 319 / 1503 |
| MegaDrive | 215 (268) / 0 / 1829 / 1563 | 154 (167) / 0 / 2024 / 1573 |
| sum | 879 | 777 (-12 %) |

**Fingerprints, the switch off:** the routed boards of Castor, k30-SBC,
jetson and Sisu (route mode on the v16 placements at a quarter of the
budget, as the fingerprint runs) byte-identical, x1543 against x1581 and
x1582, x1573 against x1583 (after the rebase on 55f6d48) and x1603
against x1584 (on the router-memory merge). With
the switch on, at that budget: jetson 20 to 17 unconnected (597 to 607
vias), Sisu 25 to 23 (375 to 379), Castor and k30 (complete) identical;
x1584 with the switch reproduces x1582 there exactly.

**What the exchanges are.** Per run, summed over the ladder's attempts
(`trouble:` lines; x1582, v18 placements): MegaDrive 34 of 144 exchanges
kept, 143 terminals closed in 154 s of work; SNSP-CPU-01 48 of 480, 138
in 173 s; OpenESC 26 of 607, 88 in 183 s; video 9 of 21, 44 in 167 s
(one attempt: video's exchanges are expensive); Glasgow 10 of 173, 21
in 165 s. On MegaDrive's designer placement the kept exchanges are bus
nets (VA9-12, VD5-14, VDDF: 7 to 17 terminals open closed to 0-3 by
displacing 3-9 short nets), the refusals mostly more than 12 victims
(the supplies VCC, VCCQ), a pour net in the way (GND), or a trade that
closes as many as it opens.

**Costs.** Vias rise where the repair works (MegaDrive +11 % on every
placement, video +29 % on the v18 placement and +4 % in the layout,
SNSP-CPU-01 +5-7 %): displaced nets are routed
again hard and take vias around the bus that went in; the via reduction
runs after the repair as before. The repair takes up to half of an
attempt's negotiation in work, inside the ladder's bound; where the
ladder used to leave part of its bound unused (Glasgow's designer route
1458 s of 1575, MegaDrive 1122 s) the repair's work is extra; where the
bound binds, it comes out of the later attempts (MegaDrive's v18 route
904 s against 1078: its second attempt started with less). Wall time
beyond the work: finding one-sided blockers scans every net's stamps
(uncharged, milliseconds an exchange).

## Also measured: one long negotiation (configuration only)

The data's own suggestion, "keep negotiating the board that converges":
the designer placements of MegaDrive, SNSP-CPU-01 and Glasgow routed by
x1543 with `{"ladder_budget_seconds": 1500, "negotiation_seconds": 900}`
(`build/exp1-longneg-x1543`). MegaDrive's first attempt negotiated 47
iterations: conflicted nets 248, 168 at iteration 10 (where the default
stops), then 141 at 12 and a plateau of 118-150 at the price cap for 34
more iterations; KiCad 85 unconnected against 75 by default.
SNSP-CPU-01 7 against 6, Glasgow 53 against 55. The negotiation does not
converge further on these boards; it stalls at the price cap with a
hundred nets in conflict. **No-go**: the time is better spent after it,
on trades.

## Recommendation: go, as the next default candidate (iterate first)

Update 2026-10-23: on by default (`trouble_repair`), skipped only in a
layout's move trials; the tiers and the layouts in docs/changelog.md,
"Committed on 2026-10-23 (trouble repair default)".

The trouble-makers' repair is the first routing change in this round
that closes connections on every gap board it can touch, on both
placements, in route mode and in the full layout, with no new errors
and identical results where it does not run: -12 % to -18 % of KiCad's
unconnected items on the five gap boards, most on the bus boards
(MegaDrive -21 to -28 %, SNSP-CPU-01 -11 to -33 %). It is small (one
file, a hook in `finish`), deterministic, and inside every work bound.
It is not a fix of the gap: video on our placement still leaves 185-214
open, the designer's placement none; the placement half of the gap is
untouched.

Before it becomes a default:

1. The short tier and the expansion tier at 30 s with
   `PCB_ROUTER_TROUBLE=1`, against the same binary: the boards the five
   do not cover (complete boards should be unchanged, as Castor and k30
   are; starved thermals, mask bridges and vias on the nearly complete
   ones are the risk: a displaced net may now cut a spoke that the
   thermal repair after it has to win back).
2. Then the layout sweep (the strict passes and the via ratio).
3. Tuning only after that, on the short tier: the victims' limit (12;
   VCC and VCCQ want more), the price (20), the share (0.5).

Next steps the data asks for, in order of expected gain per week:

- **Supply nets as trouble-makers.** The refusals on MegaDrive and
  Glasgow are supply nets (VCC, VCCQ, +3V3, VIOB) with more than twelve
  victims or a pour in the way; Glasgow gains least (21 terminals). A
  supply trade wants its pour analysed again after the exchange (the
  stitching and the pour refresh exist), and victims grouped by area.
- **The placement half: the global route in the placer's search**
  (idea 3), where it is strong (structure: video and OpenESC route
  completely on the designer's placement and leave 185-214 and 104-127
  open on ours).
- **Bus bundles** (idea 4), after a placement that lines the bus up:
  the repair shows bus nets go in when short nets make room; a bundle
  router would lay them out together in the first place.
- Not now: pin swapping on these boards (not applicable to the task),
  more negotiation (measured: it stalls), moves on boards far from
  complete (measured on 2026-10-21: they lose).

Snapshots: x1580 (bda6ffa plus the first version), x1581 (ec60853 plus
the first version), x1582 (the measured version on ec60853), x1583 (on
55f6d48), x1584 (on main 9b403c8, the router-memory merge). The
session's analysis scripts (open nets by net class, bus grouping, the
tables) are not committed; the tables above are their output.
