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
