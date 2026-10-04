# TODO

Open items that are not yet scheduled. Remove an entry when it is done or
turns out to be moot; record the outcome in the relevant doc.

The ranked work list from the failure analysis of 2026-09-25 is in
[failure-analysis.md](failure-analysis.md#work-list); it covers most of the
router and placer items.

## Benchmarks

- **Check the Freerouting setup on Multichannel.** The head-to-head in
  [benchmarks.md](benchmarks.md#head-to-head-with-freerouting-and-tscircuit-2026-09-22)
  reports 180 open for Freerouting on Multichannel, attributed to "via class
  below board minimum". That is more unconnected items than the board has
  nets (79) and looks like a broken setup rather than a routing failure.
  Verify the DSN export (via padstacks, class rules, board minimums) and the
  session import before the number is quoted anywhere, including the README.
  Artifacts: `build/corpus-fr7/multichannel/freerouting*`.
- **Breadboard fence issues.** [benchmarks/fence/README.md](../benchmarks/fence/README.md#issue-log)
  lists what went wrong on those boards, each with a benchmark: keepout zones
  inside footprints are ignored by the router (1), signals get routed on a
  `power` plane layer (9), no placement constraints (2, 11; proposed
  `constraints.json`), pin swapping (3; implemented, see
  [pin-swap.md](pin-swap.md), but on the fence it does not yet beat the
  hand-made assignment), no warning for overlapping fixed parts (5), and
  `free` vs `fixed_patterns` / overhanging footprints (10). The fence boards
  work around 1, 2, 3 and 9 in their inputs; the README says how to take each
  workaround out to test a fix.

## Router

- **The attempt ladder stops at an expensive clean rung.** On Interf-U with
  seeded runs ([GPU survey](reviews/2026-09-26-gpu-survey.md#1-seed-spread-of-the-existing-router)),
  the plane-skeleton rung comes out clean for 3 of 7 seeds with 68–82 vias
  and the ladder stops, while the pour-nets-as-tracks rung gives 24–31 vias
  whenever it runs. Continue to the next rung when a clean result has far
  more vias than the best unclean one, or rank clean rungs by vias.
- **Seeded portfolio.** Best of 7–15 seeds (`PCB_ROUTER_SEED`) saved vias on
  every board tried (PIC 2 → 0, Multichannel 18 → 14, Interf-U 28 → 24).
  Idle cores could run seeds in parallel and keep the best board.

- **Fan-out phase for dense parts (Tiny Tapeout's QFN-56, ColdFire's
  LQFP-100, BGAs).** Exact clearance (done, [router.md](router.md)) made
  Tiny Tapeout's pad escapes legal but not its completion (82/108): the
  dump of the QFN's right side shows every pad escaping outward on the top
  layer and then nets turning to run *along* the pad row 1-2 mm out,
  fencing the other escapes; 45 nets sit at the present cap for 30
  iterations. Negotiation cannot discover the structured pattern the
  situation needs: escapes straight out to vias in staggered rows (at
  0.4 mm pitch with 0.62 mm vias and 0.2 mm clearance, three rows, each
  via passing two 0.16 mm neck tracks between it and its row neighbour),
  so that every net leaves the dense area on an inner or the far layer.
  Design: after `prepare_net`, cluster narrow pads (those with neck zones)
  into rows by proximity and orientation, take the outward direction from
  the cluster's centroid, assign via rows along each row (`k mod m`, m the
  smallest count with `m * pitch >= via + clearance + (m - 1) * (neck +
  clearance)`), place each via at the first row whose spot is free of
  statics and of the other fan-out vias, and emit pad -> stub -> via as a
  fixed branch (`NetState.fixed`, like plane skeletons) so negotiation
  starts from the via on every layer. Skip pads whose net's other terminals
  all lie within ~3 mm on the outward side (decoupling, crystals). Same-net
  neighbours in a row (ColdFire's alternating GND/+3.3V pairs) share one
  via through a short bus along the row at the stub ends; that is the pour
  net pad tree below. Measure on ColdFire and whatever the GitHub harvest
  brings with BGAs; keep it off where no cluster exists.
  Caveat found on Tiny Tapeout: at 0.4 mm pitch with 0.62 mm vias no
  stagger fits (a via takes 1.18 mm of lateral room and gives back 0.4;
  the tracks between two row vias would need 0.36 mm each and have less
  than that). The designer fanned out 17 of the 56 pads with vias and took
  the rest straight to nearby parts on the top layer, which is what the
  placement was made for; so on that board the win is a negotiation that
  keeps escapes straight (no net running along a pad row within the
  escape zone), not a via pattern. A fan-out phase pays on 0.5 mm pitch
  and up (ColdFire's LQFP: 0.5 mm, 0.8 mm vias, m = 5) and on BGAs.
- **Pour nets forming trees among their pads (ColdFire GND).** GND and
  +3.3V alternate on the LQFP-100 at 0.5 mm; one 0.8 mm via per pad can
  never fit, and the designer joins the power pads with tracks along the
  rows to 1 GND and 3 +3.3V vias. `connect_to_plane` can join a pad to
  another pad's copper already; the hard reroute of stranded pads fails
  because the ways out are taken by then. Route the pour nets' pad trees
  before the signals (as `fixed_plane_stubs` tries, but as trees, not one
  via per pad), or let a stranded pad reach the nearest own copper at neck
  width.
- **Routability-aware placement (Brushless_ESC).** Heuristic placement
  changes (rails by their own name, inductor pulls) swing the first route
  between 5 and 18 open on this board. The placer has no routability term;
  add congestion (pin density per tile) and crossing estimates to the
  global objective, or race more seeds when the first route leaves opens.

- **Ladder: probe the rungs, then continue the best.** How the ladder's
  1200 s are shared decides big boards, and every fixed rule fails some
  board (2026-10-03, harvested boards):
  - half of what is left to each non-last rung: the laptop motherboard's
    first rung negotiates 600 s and leaves 3 connections open; zpn_devboard
    spends 1300 s in its first two rungs and the rungs that complete it
    never run;
  - an even split (current): zpn_devboard and MIDAS-MK2 complete, ColdFire
    unchanged and faster, but the laptop motherboard leaves 21 open and
    OpenRX 69 (from 3 and 50);
  - an even split that lets a converging negotiation run on (a tenth fewer
    conflicts in five iterations, up to half of what is left): worse on
    both (early iterations do not converge by that measure; the extension
    ate the budget of zpn's completing rung). Reverted.
  Done (2026-10-03): the probe ladder (`probe_ladder`, `probe_seconds`)
  for 4+ layer boards with 120+ connections; see router.md. Open: rank
  probes by trend as well as count, and continue the second best when
  the leader stalls with time left.

- **Budgets in work, not seconds.** Negotiation, clean-up, repair and the
  ladder are budgeted in wall-clock seconds, so a board routed while the
  machine is busy gets less work done and can come out much worse
  (OpenESC 4in1, 2026-10-03: 150/152 during a sweep, 121/152 when a test
  ran next to the sweep). Results are not reproducible across loads or
  machines. Budget by work instead (search expansions, iterations, nets
  rerouted), calibrated so that an idle machine takes about the time the
  seconds give now; keep a wall-clock limit only as a last guard.

- **Neck down where the class width does not fit.** Designers often
  leave the default class wide and draw most tracks narrower (ohdsp's DSP
  board: class 0.5 mm, 840 of 1992 segments at 0.135 mm). Routed at the
  class width the board leaves 30 connections open; at the designer's
  widths (benchmarks/github/guide.py) it routes 190/190. For boards from a
  netlist there is no designer copper to read: route at the class width
  but let a net neck down to the board's minimum (or the project's
  predefined widths) where the class width does not fit, at a cost, as
  neck zones do near narrow pads today. Tried (2026-10-04) and reverted:
  necking a net down everywhere once it is blocked at its class width or
  in conflict for 16 iterations. The DSP board stayed at 158/190 (its
  trouble is congestion spread over many nets, which rarely makes one net
  blocked or stuck), and Interf-U got 3 more vias. What would work better:
  narrower widths as a cost the search can choose per step (neck cells as
  an alternative class on every node, priced by how much narrower), or a
  ladder rung that routes signal nets at the project's smallest
  predefined track width. Done (2026-10-04) as the last ladder step:
  signal nets at the project's smallest predefined width when connections
  stay open (DSP board 158 -> 189 of 190). Open: a per-step width choice
  in the search, and boards whose project lists no narrower width.

- **Pour reach as KiCad's spokes, not a radius.** The router counts a pad
  as joined to its pour when a pour node lies within 0.8 mm of a pad node
  (`analyze_pours`, `piece_near`). KiCad joins it only through thermal
  spokes: straight out from the pad (along its axes, or at the
  thermal_bridge_angle), as wide as the bridge width, across the thermal
  gap into fill. In a fine-pitch row the fill reaches the pads diagonally
  or not at all, so the router believes pads joined that KiCad leaves
  unconnected (ohdsp's DSP board: 15 GNDD pads of U201). Tried and
  reverted (2026-10-04): feeding KiCad's unconnected pads back as
  "route these by tracks"; they then joined the same phantom fill (14
  left). The fix is in the model: test the spoke rectangles against the
  pour map, and count a pad on its pour only with at least one spoke (two
  for no starved_thermal). Done (2026-10-04) for thermal pours, solid
  pours keep the touch test: DSP board 15 -> 6 unconnected. Open: per-pad
  zone_connect overrides, the spoke angle from thermal_bridge_angle, two
  spokes for no starved thermal.

## Layout

- **Moves that cannot close the last opens cost the whole budget.** On the
  esp-usb-corner task (four layers, 107 connections) the first route left 8
  open. 15 moves of about 40 s each spent the 600 s budget without closing
  them, and route mode's ladder then finished the board in 276 s: 1119 s
  in all. Moves now stop when six trials in a row close no open connection
  (894 s). Each trial still costs 30-60 s there (150-230 s on Interf-U):
  the incremental reroute is the cost to attack.

- **Interf-U layout: 110/110 again (2026-09-29, commit 5fdbd3e).** The
  placement kept for the least wire (seed 1) ended 109/110. The placement
  race now switches to seed 2's (5 open after the first route instead of
  6), and two moves close the rest: 110/110, 32 vias, 4669 mm. KiCad
  reports 3 starved thermals (the designer's board has 2). First-route
  opens are a weak predictor (seed 3: 22 open, yet it also completes), so
  racing further (after moves) may pay on other boards.

## Placer

- **Placement for bus-heavy through-hole boards.** Interf-U from its
  schematic (2026-09-29): a quick route leaves 89 connections open at the
  designer's area, and only 3.9 times the parts' area (191 x 128 mm against
  the designer's 116 x 108) routes. Our router completes the designer's
  placement in 75 s, so the placement is what loses: aligned rows of chips
  with the bus running straight between them are not something the
  wirelength objective finds.
- **Packing along an edge.** FRM16 (a 137 x 15 mm strip, five 12 mm
  connectors on the top edge and an 18 x 12 mm relay) and similar boards
  leave connectors unplaced although the designer's placement exists: the
  legalizer places greedily and does not pack a crowded edge.
- **Card edges.** Parts held at an edge may now be placed by their copper
  (a PCIe tab with its key notch), centred in the tab. MAVRIC's outermost
  finger stays unreachable on the back layer although the designer runs a
  0.4 mm track there; not understood yet.
- **Alignment beyond rows.** `row` places parts in a line at a pitch as
  one macro body, or a grid with `columns`. Alignment without a fixed
  pitch, rows of parts turned differently and nested groups (a switch with
  its diode) are not there yet.
- **Parts as wide as the board.** On raspberry_pi_pullup (a 4.8 mm wide
  board) the parts' copper reaches both edges; the placer's grid cannot
  hit the 0.002 mm of room. Snapping to both edges at once would.
- **Crowded single sides.** FogDrive, HaveSome and tiny_tapeout leave a
  part unplaced with a side 78-93 % full. The hint tells the agent to move
  parts to the other side; the placer does not choose sides by itself.
- **Legalization loses 30-90 % of the global placement's wirelength.**
  Examples: PIC 678 → 1306 mm, hierarchy 493 → 907-1169 mm, Interf-U
  3206 → 4077 mm; measured 2026-09-26/28, graph-first experiment on branch
  `experiment/graph-first-placement`.
  - **Not the anneal's random walk alone.** A minimum-displacement overlap
    relaxation in its place (tried 2026-09-28, reverted) ends no better
    (hierarchy 974, Interf-U 5301).
  - **The global placement's wirelength is partly fictitious.** At the 4 %
    bin-overflow stop, parts still overlap physically: 64 bins smear small
    parts over about 1.5 mm.
  - **Exact pairwise overlap force: tried 2026-09-29, reverted.** This was
    the exact overlap of body pairs as an extra force in the late global
    iterations, which keep going until the pairs are nearly apart. Final
    wirelength did not improve consistently:
    - improved: hierarchy −11 % (weight 10), PIC −18 % (10), Interf-U
      −11 % (0.1);
    - worse: multichannel +14 % and sonde +18 % (10);
    - no weight is best on every board, and the changes are within the
      seed spread.
  - **Open candidates.** Bins sized from the median part; the survey's
    CP-SAT NoOverlap2D / sparrow legalizer
    ([algorithm survey](reviews/2026-09-26-algorithm-survey.md), section 5).
    Meanwhile three placement seeds (the best one kept) absorb part of the
    spread.
- **LNS endgame: tried, no gain (2026-09-28).** This was the survey's
  large-neighbourhood search: rip up an open net with every net in its
  window, reinsert with the open net first in a shuffled order, and keep
  the round if fewer terminals stay open.
  - Result on four PCBench "open" boards: rounds were kept, but no board
    got closer to complete.
  - Why: each ends with two or three nets that compete for the same gap,
    and only one fits on the lattice, whatever the order.
  - The designer fitted both, so the limit is geometric (lattice
    resolution or topology), not negotiation.
  - Reverted; the survey's exact window solving or a finer local lattice
    would be the next step for these.
