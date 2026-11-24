# Outputs that misled agents, fixed (2026-11-24)

The eval study of 2026-11-24 (`docs/experiments/evaluation-representations.md`
on branch `fund-eval-representations`) found places where the tool's
outputs misled its subjects. Each was checked on main (e38be883) and fixed
or explained. Diagnostics only: no routing result changes (Identity).

**1. board-placer.json against the written board.** Confirmed on the
sweep of 2026-11-18, fixed on main since 84c4c3eb (2026-11-20).
USB_Keypad's `result/board-placer.json` described the placer's kept seed
(`seed 16 kept`), the board held the race's (`placement 1 kept`): 70 of 98
parts elsewhere, and 57 of 106 sweep boards differed. Since 84c4c3eb
`board_layout/placement_evidence.rs` writes the selected placement's report
(`selected_placement`; the original in `initial-placement/`); on the
nightly of 2026-11-24 0 of 107 sweep boards differ, in `placed/` and
`result/`. New test `a_layouts_placement_report_describes_the_board_it_wrote`:
F1 at seed 7 keeps pool placement 7, and every row must stand where the
board has it; with the report's write removed it fails (`placed: C1 at
[12.065, 14.605, 90.0] in board-placer.json, on the board [12.7, 17.78,
270.0]`).

**2. Hot spots far from the open item.** Confirmed. `inspect-routing-run`
listed a net's hot spots within 3 mm of any of its pads; USB_Keypad's
+3.3V has pads everywhere, and the five listed lay about 16 mm from the
open item. Now each net has `open_at` (the ends of KiCad's items) and
`hot_spots_near` holds those within 3 mm of an end (zones' items left out:
KiCad places them at the zone's first corner), nearest first, with
`to_open_item_mm`; top-level hot spots get `nearest_open_item_mm`. On the
study's USB_Keypad run no hot spot is listed for +3.3V (the nearest lies
8.3 mm away). The verdict's hot spot action leads with KiCad's open link
(the one nearest the hot spot) and gives the hot spot's distance to it;
beyond 5 mm, or without a link beyond 5 mm from every item, the hot spot
is said to be where nets fought and its parts are not offered (SUMEC's and
WLED's worst hot spots, 20 and 22 mm from the nearest open item, led their
open connections). Tests: `hot_spots_near_an_open_net_are_those_near_its_open_item`,
`the_open_link_named_is_the_one_nearest_the_hot_spot`,
`a_hot_spot_far_from_every_open_item_is_no_lever`, the jetson test.

**3. Dead-pad wording.** Confirmed: "pads no track of their class can
enter" and "the escape was not kept: ... its pocket closed" read as walls
of the input. `dead_pads[]` now carry `access`: `no_legal_access` (no
point of the pad keeps the clearances for the net's narrowest track),
`off_lattice` (no node within 2 mm: SUMEC's C19 and R12 lie beside the
outline; their wall was empty), `no_lattice_node` (a track end fits, no
node within 2 mm admits the class's width), `stubs_blocked`,
`start_blocked` (free from the pad's best point, not from where the
router starts), `escape_not_kept`. The verdict says only the first two are
walls by the router's rules. Tests: the three `a_dead_pad_*` tests in
`router/corridor_tests.rs`.

**4. keep_whole on a file of three boards.** Confirmed (Hub). The verdict
finds Edge.Cuts loops that do not touch (`board_outlines`: count, boxes,
KiCad's items whose ends lie on two of them). Hub's verdict now opens with
`3 separate board outlines ... 2 of KiCad's unconnected items join copper
on two of them, which no track or pour can join`, and every pour action
starts with the outline note. Empty part names no longer reach the
keep_whole entry (Hub said `"parts": ["U10", "U5", "", ""]`; they come
from `pour_plan.rs`, another package's). Test:
`three_outlines_and_the_items_between_them_are_counted`.

**5. "no track leaves, no via allowed".** Partly confirmed: board-router.json
had it as `vias_in_pads.closed`, under a name about vias, and the verdict
dropped its action whenever `via_in_pad_retries` was computed, also empty:
the laptop keyboard's U2 (KiCad names AC9, not all three closed RESET
pads, so no retry groups them). Now each closed entry has `why`, the
verdict has `closed_pads` (with `open_nets`), the open connections' line
names it, and the action is dropped only for a footprint a retry names. On
the study's keyboard run all three appear, the U2 action included. Test:
`open_bga_balls_without_a_via_in_pad_name_the_opt_in`, extended.

**6. Router defects.** (a) Confirmed, documented in docs/router.md ("What
limits the router"): SUMEC's IC2.11 and IC3.11 (`no_lattice_node`, a 2 mm
class); KiCad accepts a 0.4 mm track to the net's own pad C15.2, which
needs a pre-route merge of the two terminals: a new mechanism with its own
measurement, not built. (b) Verified, not "found and not kept": WLED's
LDO1.3 and LDO1.2 (designer tier of 2026-11-24 too) lie under a second
SOT-23 without nets that covers their centres, a plain pad's stubs start
at its centre, and the old wall measured its free stubs from the pad's
best point. They read `start_blocked ... blocked there by LDO1.2 (no net,
...)` now. Starting stubs from a pad's free part was measured on
2026-11-07 (branch pad-access-starts: WLED 2 to 0 open with a pour neck,
designer tier 75 to 75 strict) and not taken; documented. (c) Fixed on
main before this package by the pour-fidelity merge (59bd64a2):
capybully had 3 unconnected on the sweep of 2026-11-18, 0 on the sweep and
the designer tier of 2026-11-24, both strict passes. Not touched.

**7. Placement measures shown to agents.** An audit, nothing removed or
added. Relabel ("does not predict the routed result"): board-placer.json's
`wirelength_*_mm` and `global_overflow` with place-kicad-board's stdout
line; board-layout.json's `congestion` (RUDY scores, ratsnest crossings)
and layout.log's pre-ranking lines; ablate-kicad-placement's "signals a
placer can read before routing"; `rear_routing_candidates`, whose action
ties a crowded front to open connections. Keep: the race's routed probes
(`race.probe`, `placement_race`, `outline_sizing[].open`), `room` and
`utilization` (a fit measure), `pad_exits`, score-kicad-board's measures,
the "density is a hypothesis" lines.

## Identity

main's binary at the base (e38be883) against this branch's, side by side:
the fingerprints (route mode from build/github-layout-v16, 1 x 4, scale
0.25, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`) 4 of 4 boards
identical, every phase line and the routed board's sha256; the expansion
tier 46 of 46 identical (phase lines, routed board, row; 571 unconnected,
28 strict on both), with the branch's first and its final binary (7a959247).
No router fix is in the branch.

On the expansion tier `access` reads 8 `no_legal_access`, 6
`start_blocked` (LC_PAMV4b's J16.11: its centre lies in a rule area), 1
`no_lattice_node` and 1 `escape_not_kept` (Barklind's J27.9 and J27.7, a
3.5 mm class). The study's four boards (designer tier, scale 0.595) leave
the nightly's unconnected items (Hub 3, SUMEC 6, WLED 2, keyboard 2).
