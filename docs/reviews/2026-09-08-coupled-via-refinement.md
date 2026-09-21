# Acting on a diagnosed via opportunity

Follow-ups: [automatic discovery](2026-09-08-via-discovery.md) now selects
targets without supplied coordinates and fixes the original-board fallback.
[Future demand and net ordering](2026-09-08-routing-demand-and-order.md)
subsequently produced a complete three-via ECC83 layout from empty copper.
The measurements below retain the earlier targeted-refinement evidence.

A user identified avoidable via excursions on the 70%-area ECC83 board. The
program now turns a successful foreign-copper counterfactual into a complete
repair transaction, rather than stopping at an explanation of the blocker.

The production replay reduces the board from **10 to 8 vias**, with stored
track length increasing from **386.307773 to 395.962172 mm**. Both boards pass
native ERC, design DRC, schematic parity and connectivity checks. Both retain
seven separately classified footprint-library metadata warnings. Placement,
pad inventory, net assignments and project rules are unchanged. Only
`Net-(P4-PM)` and `Net-(P3-P1)` copper changes.

This is a tradeoff: 9.654399 mm more track for two fewer vias. The configured
whole-board objective is stored track length plus 5 mm per via, so the score
improves from 436.307773 to 435.962172 mm. This does not claim global optimality,
better electrical behavior, or a universally correct exchange rate.

## New program action

```sh
pcb-maker refine-kicad-vias SOURCE_DIRECTORY BOARD_ID CANDIDATE_JSON OUTPUT_DIRECTORY CONFIG_JSON
```

The config contains `via_actions`, `reroute`, and `maximum_coupled_trials`.
The first two use the existing via-action and grid-routing configuration
schemas. The command is a targeted refinement operation: the caller identifies
a via in an immutable source route candidate. It does not yet scan every via
after every whole-board route.

1. Run the independent removal/local-reroute portfolio with native admission
   and automatic previews. A complete source candidate is required.
2. Collect successful bounded blocker cuts containing only foreign copper.
   Removing footprints, pads or rule areas is never an actionable cut here.
3. Recompute the local target route with that copper provisionally yielded.
   Prefer the direct merged chord when exact geometric checks show it is
   clear and shorter than the grid path under the same yielded-net set.
4. Copy the complete project, apply the target route and remove the yielded
   nets' copper. Verify and render this explicitly incomplete intermediate.
5. Reroute every yielded net against the actual resulting board. Verify and
   render the final transaction, including failed routing attempts.
6. Independently compare fixed board structure and project bytes. Compare
   **whole-board** quality, including the cost of displaced nets. Select only
   a complete preserving improvement; otherwise retain the complete source.

The source is never edited. The retained `result` is a project copy with its
native reports, labelled preview, proposal candidates and diagnostic evidence.
The initial implementation scores straight-track boards without copper zones;
it rejects unsupported arc/pour inputs instead of understating their cost.

## What the experiments exposed

Independent local search cannot remove the selected socket excursion while
`Net-(P3-P1)` remains fixed. At both 0.25 and 0.10 mm resolutions, suppressing
that net opens the target route. The completed transaction restores it with
zero new vias, using a through-hole pad to reach the other copper layer.
This is a coupled reroute, not a minimal-displacement trace shove.

The first automatic version kept a slightly longer grid path and correctly
rejected the resulting whole board on the configured score. Adding the
exact-clear direct chord reproduces the better prototype automatically.
The two resolution trials currently repeat the same direct target; deduplicating
complete repair proposals is a remaining efficiency improvement.

The alternative back-layer merge exposes `Net-(U1A-G)` as a sufficient blocker
at 0.10 mm. Restoring that net produces a native-complete candidate with 9 vias
and 406.022239 mm of track, which the objective rejects. The original complete
board is retained. The middle-board target produces no actionable sufficient
cut within the configured search and also retains the original.

The short passage between R3 pad 2 and P8's mounting pad is physically too
narrow under the unchanged rules: 1.429106 mm between copper edges versus
1.600000 mm for a 0.8 mm trace with 0.4 mm clearance on each side. This excludes
that specific passage; it is not a proof that every wider detour is impossible.

A separate integration bug made the action lookup reject rounded displayed
coordinates (`104.57` versus `104.57000000000001`). Via removal, relocation and
relocation-direction lookup now match integer-nanometre physical positions.
The topology regression uses a non-identical floating-point coordinate, and
the final production replay uses rounded target coordinates.

## Labels and evidence

Native SVG verification previews now show component references and stable
`V-` UUID prefixes while keeping component bodies and markings hidden.
`preview-labels.json` maps each label to its full UUID, net and native position.
For supported polygonal outlines, an explicit absolute-coordinate crop keeps
labels aligned with the copper. Other outlines retain the previous preview
without labels. The optional `render_inspection_layers.py` creates separate
front and mirrored-back copper views with upright labels.

All 110 KiCad library tests pass. Three production controls exercise successful
refinement, a complete but too-costly alternative, and no actionable repair.
The independent report checks native component/pad readback, project bytes,
changed-net inventory, unique label mappings and all SVG XML. Browser playback
was not tested.

- [Labelled before/after and clearance detail](../../build/ecc83-via-opportunities-2026-09-08/index.html)
- [Independent native readback audit](../../build/ecc83-via-opportunities-2026-09-08/audit.json)
- [Selected automatic transaction](../../build/ecc83-via-opportunities-2026-09-08/final-front/refinement.json)
- [Back-layer alternative](../../build/ecc83-via-opportunities-2026-09-08/final-back/refinement.json)
- [No-improvement control](../../build/ecc83-via-opportunities-2026-09-08/refined-middle/refinement.json)
- [Reproduction configuration](../../build/ecc83-via-opportunities-2026-09-08/refine-front.json)
