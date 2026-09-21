# Repairing remaining native connections

`bridge-kicad-open-connections` discovers disconnected copper islands with
KiCad's connectivity engine and tries bounded bridges between existing pads.
It preserves the existing placement and copper. Each admitted bridge must
reduce native-reported opens, preserve the original design, and pass the native
DRC/ERC/parity checks. The last verified board remains selected after failure.

```sh
pcb-maker bridge-kicad-open-connections source board-id bridge.json output
```

Example `bridge.json`:

```json
{
  "routing": { "resolution_mm": 0.25, "max_expansions": 2000000 },
  "maximum_attempts": 4,
  "maximum_pair_candidates": 8
}
```

The program compiles widths, clearances, through-via dimensions and edge/drill
rules from the native project. These replace physical dimensions supplied in
`routing`; the remaining fields control search. Unknown custom design rules
require separate support. Python with `pcbnew`, KiCad CLI, and the existing
verification/render dependencies must be available.

Candidates are ranked by pad-center distance with stable identity tie-breaking.
The shortlist contains pairs from distinct native islands, not guessed missing
straight-line traces. The router retains all other pads and copper as context.
Coincident front/back SMD pads remain separate electrical contacts; a plated pad
can join layers. Padless islands and ambiguous pad locators remain explicit
limitations. Filled zones, unsupported padstacks, copper graphics and geometry
without exact routing support are refused.

`output/island-bridges.json` is the atomic selection record. Its
`selected_directory` names the retained native project. Exit zero requires full
native completion; partial progress remains nonzero. Baseline, proposal, island
and verification renders are saved automatically with combined copper and no
component bodies. The baseline receives full native verification. Candidates
reuse its schematic ERC report only after checking unchanged ERC input hashes;
each candidate still runs native PCB DRC and schematic parity and captures a
preview. Separate exact design-input and append-only copper checks remain.

This command appends copper. It does not move traces, rip up nets, change
placement, or choose a different board outline. The attempt and shortlist bounds
limit the search; exhausting either is not proof that no bridge exists.
