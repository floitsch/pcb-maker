# Review 2026-10-21: lowering, terminals, connectivity, pour model

Read-only review of the seven packages merged today that touch the
lowering and the connectivity model (5a99d89, ffa49a5, 837b5f6, 3223282,
642e28c, 401ab2f, ded766d), read together at 20956b3. No builds, no runs.
Line numbers are of the working tree. "Sure" means the reading of the
code is certain; "suspect" means the effect on real boards is inferred.

## 1. High: the exposed-pad heuristic is defeated by the pad's own copy (sure)

`crates/pcb-kicad/src/board_router/lower.rs:1645-1670` (`exposed_pad`),
used at `lower.rs:1015-1021` to leave the exposed pad out of
`no_via_pads`. The test "largest SMD pad, at least twice any other SMD
pad's area" runs over the footprint's pads one by one. The pour-model
package (642e28c) exists because exposed pads come as *two* SMD pads of
one number, F.Cu and B.Cu, with thermal-via pads between them (Sisu
U2.11: two `smd rect (size 1.2 2)` at the same place, MSign U1.21,
LC_PAMV4b U3.25, ZSWatch IC401.33, IotThermalCamera TU2.9). The B.Cu
copy has the same area, so `2.0 * other.area > pad.area` holds and the
function returns `None`: both copies land in `no_via_pads`, the router
may put no via in the exposed pad, and the pour-connection via of a
pour-net EP has to go beside a pad the pins enclose. The same holds for
footprints whose EP is split into several same-number SMD pieces for
paste. ded766d's own numbers show it: Sisu 6 to 23 unconnected while
Castor and k30 (no two-sided EP) pass. Fix: group `footprint_pads` by
pad number before the test (sum or max the area per number, treat the
group as one candidate and exclude its members from `others`), or run the
test on the merged terminals.

## 2. Medium: a padstack pad keeps its front shape on every layer (sure code, suspect impact)

`lower.rs:783-800` pushes the pad's own obstacle with `layers:
pad_layers` (all copper for a through-hole pad); `lower.rs:821-845` then
*adds* the padstack's inner/back shapes. Where the back shape is smaller
than the front (dennisss's 4 mm front over 3 mm back, or any padstack
with a reduced inner annulus), the model has phantom copper on those
layers. Three consequences: `verify::copper_components`
(`crates/pcb-router/src/verify.rs:478`) joins a B.Cu or inner track that
touches only the phantom copper and reports "connected" where KiCad's
connectivity does not; the terminal merge at `lower.rs:1257` tests anchor
containment against the front shape on every layer; and other nets'
tracks are kept off copper that is not there. Fix: mask the own
obstacle's layers to `pad_layers & !union(variant masks)` when any
variant exists (the front shape then stays on F.Cu and on layers the
stack does not name).

## 3. Medium: net-name facts live in thread locals that only some paths set (suspect)

`crates/pcb-kicad/src/kicad_nets.rs:99,141` (`BARE_NETS`,
`normalize_net`) and `lower.rs:118-158` (`JUMPER_NETS`, `routable_net`).
Both are set per thread by `note_net_names` / `note_jumper_nets`, called
from `lower_board`, `resolve_project_rules` and the placer's lowering.
`normalize_net` and `routable_net` are also called from
`quality.rs:359-360` (score-kicad-board), `board_stats.rs:136-308`,
`describe.rs:222`, `pour_plan.rs:60-190`, `necks.rs:713`, `pin_swap.rs`
and `geometry.rs:794` (`pours`), none of which notes the facts itself.
In a fresh process or on a thread that has not lowered this board,
`normalize_net("/A15")` yields `"A15"` and folds two nets the board
declares separately (gametank, meshsat's `/VIN_CELL` beside `VIN_CELL`),
and `routable_net` drops jumper nets. In score-kicad-board this makes
the via-in-pad check (`quality.rs:1075`) compare collapsed names, and the
pour plan can key a plane by the wrong name. The lowering itself is
consistent because `lower_board` notes both before use on its own
thread, and the ladder's workers lower and emit on one thread. Fix:
carry the two sets in a `NetFacts` value passed to the consumers, or at
least call both `note_*` functions at every public entry point that
reads a board (score, stats, describe, pour plan).

## 4. Medium: rule precedence is modelled as "raise only" (sure, by design)

`crates/pcb-kicad/src/project_rules/dru.rs:1134-1200` shadows an
earlier clearance rule by later rules that surely match (the right
reading of KiCad's last-matching-rule), but every lowered value is a
floor: `apply_custom` (`project_rules.rs:531`) and
`Resolved::new` (`crates/pcb-router/src/rules.rs:178`, `max`) never
lower a clearance below the net class, and two custom rules on one pair
take the larger, not the later. KiCad's custom rules override the
implicit net-class rule even when smaller (JLCPCB's template: a 0.127 mm
"Min. Spacing" after a 0.2 mm netclass). The router therefore never
produces a KiCad clearance error from this, but it can refuse boards a
designer made routable by lowering a pair's clearance. Also sure:
`obstacle_kind` (`rules.rs:245`) gives copper graphics and the pour
brush no pair clearance, while KiCad applies a net-conditioned clearance
rule to every copper item of that net (graphics, zone fills). Low DRC
risk on today's boards; note it in `KiCadCustomRuleReport` as
"approximated" rather than "applied".

## 5. Medium: via-owner maps depend on pad order around an exposed pad (sure, low impact)

`crates/pcb-router/src/grid.rs:767-818`. The new `beside` arm lets the
pad's own net's via touch a no-via pad from outside and marks the node
blocked with that owner. The `own_vias` arm for a pad that *may* hold
vias (the exposed pad) requires `!via_blocked[index]`. With the QFN's
pins listed before the EP (KiCad's usual order), nodes inside the EP
within a pin's via reach are first blocked with owner N by the pin, then
the EP's arm fails on `via_blocked` and falls through to owner 0; EP
first, the same nodes stay owner N. The map is deterministic per board
(the statics key hashes the obstacles) but loses legal via sites along
the EP's edge in the common order, which compounds finding 1 for
pour-net EPs. Fix: let the `own_vias` arm also accept
`via_owner[index] == owner16(net)`.

## 6. Low: terminal merge and `copper_components` agree, with two gaps (sure)

`lower.rs:1240-1330`. The union-find runs per net and per label, so the
transitive merge cannot join two nets' pads through a third net's via:
the feared case does not arise. Gaps: (a) overlap is anchor containment
only (`lower.rs:1257`); two same-number pads that overlap without
holding each other's anchor stay two terminals, which costs a track
KiCad does not need (harmless). (b) Jumper groups join across layers
without copper, as KiCad 9 does; but `jumper_groups` is keyed by
footprint ordinal and group text, and `by_jumper` joins members of
different nets only if they share a net (they cannot) - fine.
`verify.rs:505-514` re-checks `terminals.any(|t| t.pad == index)` for
every obstacle though `element_of_obstacle` already holds every terminal
pad: redundant O(obstacles x terminals) per net, not wrong.

## 7. Low: pour knockouts ignore the pad's own clearance override (suspect)

`crates/pcb-router/src/router/manual_spokes.rs:706-740`
(`isolated_knockouts`) keeps the fill `zone clearance + brush half`
from a `zone_connect 0` pad. KiCad's filler keeps the larger of the
zone clearance and the pad's (or footprint's) local clearance
(`clearance_override`, `lower.rs:737`). A pad with a 0.5 mm override in
a 0.2 mm zone leaves a 0.3 mm band of fill in the model that KiCad does
not have; a thin bridge there would be a "connected but KiCad not"
piece. Same for the thermal rings' reach. Fix: `reach =
clearance.max(obstacle.clearance_override.unwrap_or(0.0)) + half`.

## 8. Low: parser consistency (sure)

- `lower.rs:1116-1135` (the `gr_*` branch) re-implements
  `item_layer_names` and `mask_sides` inline; the three copies agree
  today but will drift. `mask_sides` accepts only `F.Mask`/`B.Mask`,
  pads also `*.Mask`/`F&B.Mask`; graphics cannot be on `*.Mask`, so no
  miss now.
- `text_box_shapes` (`geometry.rs:586`) takes `(pts ...)` or
  `start/end`; a KiCad 7 `gr_text_box` with `start/end` and `(angle
  ...)` loses its rotation. `fp_text_box` has no hidden check
  (`footprint_text_box` at `geometry.rs:322` does).
- `padstack_variants` (`lower.rs:1801`) takes `"Inner"` and copper
  names; a `(mode custom)` entry for `F.Cu` returns `None` (the pad's
  own forms), correct. A blind/buried `(via (layers ...))` is lowered on
  all layers (`lower.rs:1045`), so `copper_components` can join through
  a via on a layer it does not reach.
- `pours` (`geometry.rs:852`) reads `via_reliefs` as "`connect_pads`
  has no keyword": `(connect_pads (clearance x))` is thermal, `yes`,
  `no`, `thru_hole_only` are not; matches KiCad 10's
  `knockoutThermalReliefs`.
- `vias_tented` handles KiCad 10 `(tenting (front yes))`, KiCad 9
  `(tenting front back)` and `viasonmask`; fine.

## 9. Low: performance (sure, bounded)

`lower.rs:1409-1470` scans every obstacle per mask opening (openings x
obstacles; Sisu's 190 B.Mask polygons on a 20 000-obstacle board is
~4 M AABB tests, fine; a board with thousands of mask-only test points
would be the first to hurt). `net_ids_lookup` (`lower.rs:1715`) is a
linear name scan called per copper graphic, mask opening and pour; with
code-named graphic nets by the thousand (the chessboards) that is
graphics x nets string compares - a `BTreeMap` from `net_ids` is already
in scope. `lower.rs:1358` (`touches`) is graphics x terminals of the
net. `finish.rs` runs `copper_components` per routed track-only net,
each scanning all obstacles. None is quadratic in pads; all are at most
tens of millions of cheap operations on a 5 000-pad board.

## What I checked and found sound

- `net_name_from_form` / `item_net` / `net_codes`: a one-field unquoted
  number is a code, resolved through the table; `(net 0 "")` is no net.
- Pair clearances are filled symmetrically (`rules.rs:170-182`) and
  looked up by (class group, kind, obstacle group, kind, layer) on both
  sides, in the stamps and in `verify_with_rules`; `PAD` takes the max
  of pad and track rules for the stub, deliberately.
- Zones of equal priority ordered by lowercase uuid matches
  `ZONE::HigherPriority`.
- `hops_from_a_via` reach `via_drill + hole_to_hole` is KiCad's
  centre-to-centre bound for two equal drills.
- `copper_components` uses the pad obstacle's layers, not the
  terminal's restricted `layers`; that is KiCad's view (a pad connects on
  any layer it has copper on) and the restriction is a routing choice.
- `solid_fill_reaches`, `pieces_near`, the spoke end test and the via
  reliefs read as described in the changelog.

## Suggested order

1 (one function, measurable on Sisu/MSign fingerprints), 5 (one guard),
2 (one mask), 7, 3 (an API change), then 4 and 8 as documentation or
follow-ups.
