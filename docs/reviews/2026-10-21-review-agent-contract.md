# Review: the agent contract (2026-10-21)

This is a read-only review of main at 20956b3, written from the side of the agent that uses the tool. It covers docs/agents.md, README.md, docs/constraints.md (free dimension included), docs/config-reference.md, the user-facing parts of router.md and placer.md, src/main.rs, src/agent_cli.rs, src/agent_cli/free_dimension.rs, crates/pcb-kicad/src/verdict.rs, /home/flo/tmp/fence-report.md and benchmarks/fence/README.md. I built nothing and ran nothing. "Sure" means I traced it in the code. "Suspect" means it needs a run to confirm.

## High

**H1. A router config file drops some of the project's rules (sure).** agents.md:267-268 says "Pass a JSON file instead of `auto` ... The rules still come from the project". ROUTE_HELP (agent_cli.rs:1096-1098) says the same. But `board_router_config` (main.rs:747-787) copies only some of what `resolve_project_rules` sets. It skips `solder_mask_to_copper_mm` and `solder_mask_bridge_severity` (project_rules.rs:514-519), and the router's lowering reads both (board_router/lower.rs:451-457). A project with a mask-to-copper clearance therefore routes without it as soon as the agent passes `{"seeds": 4}` or `{"via_in_pad": true}`. A project that rates bridges as warnings gets the strict mask rule instead. There is a second gap. If the file sets `default_rules` or `connection_rules`, the project copy is skipped completely: edge clearance falls to 0.0, and hole clearances, `project_rules` and `net_rules` drop to their defaults. That contradicts config-reference.md:47 ("a file's keys replace what `auto` would give only where a file sets them"). Fix: start from the resolved project config and overlay the file's keys.

**H2. A missing or failing kicad-cli costs the whole budget and is then reported as our bug (sure).** Nothing checks kicad-cli at startup. The first call is the final verification (board_router/route.rs:708-711). The version warning is likewise deferred to "before the first KiCad check" (native_report.rs:263-273). A route or layout without kicad-cli on the PATH therefore spends its full budget and then exits 4 with "failed to run kicad-cli: No such file..." (native_report.rs:504). Its verdict says "an internal error ... report the error with the input" (verdict.rs:1067; agents.md:391). The import has the same flaw. When kicad-cli is missing, or `kicad-cli sch export netlist` fails on a bad schematic (netlist_import.rs:563-573), the error is not marked `input_error` and exits 4. The import help promises "2 on usage or input errors (the schematic or netlist unreadable)" (agent_cli.rs:1185). agents.md never states that KiCad 10 and kicad-cli on the PATH are required; only README.md:73 and `--help` (main.rs:155) do. An old KiCad only prints a stderr warning, which never reaches the verdict. Fix: check `kicad-cli version` before the run, exit 2 with "install KiCad 10 / put kicad-cli on PATH / PCB_SKIP_NATIVE", and put the version warning into `next_actions`.

**H3. The import finds KiCad's stock libraries only on Linux (sure on the code; the macOS effect is suspect).** `footprint_libraries` (netlist_import.rs:213-226) looks in `$KICAD10_FOOTPRINT_DIR`, then `$KICAD9_FOOTPRINT_DIR`, then `/usr/share/kicad/footprints`, and reads the user table only from `~/.config/kicad/{10.0,9.0}`. The snapshot is also built for macos-aarch64. There KiCad keeps its footprints in the app bundle and the user table under `~/Library/Preferences/kicad`. A macOS agent would see every stock footprint reported `missing` (exit 1) with the advice to edit `fp-lib-table`, which does not help. `KICAD10_FOOTPRINT_DIR` is not documented anywhere in docs/.

## Medium

**M1. The keepout warning is missing in the usual flow (sure).** constraints.md:72 says the input check warns of a keepout over a held part "before the run (the log and the verdict's next_actions)". `keepout_warnings` returns nothing when the constraints have an `outline` (placement_constraints.rs:1394-1396), and the import always writes `"outline": {}` (agents.md:32-33). In that case the warning ends up only in board-placer.json's `constraint_warnings` (board_placer/place.rs:88-93). More generally, no `constraint_warnings` entry ever reaches the verdict. This is exactly what fence issue 15 asked for.

**M2. The free dimension's contract has three gaps (sure).**
- With `accept: "pass"`, a clean size is chosen, but the run still exits 1 with `status: incomplete`. The exit code is the chosen run's (agent_cli/free_dimension.rs:449-450). `pass` also judges by `new_findings`, the per-type rule that agents.md:427 and :457 call "for comparisons only" (free_dimension.rs:1062-1068). Either judge `pass` by `new_findings_strict` without the constraints, or have the search set the exit code.
- `probe_seconds` means two things. It is a router setting (75 s of negotiation per rung probe; config-reference.md) and a `free_dimension` key. The not-clean next action says "give the probes more work (probe_seconds, total_seconds)" without saying where (agent_cli/free_dimension.rs:411). Put into router.json, it is accepted silently and does something else. Name the key `free_dimension.probe_seconds`.
- When no size is feasible, the run returns a usage error before the output directory exists (agent_cli/free_dimension.rs:166-172). No `verdict.json` and no `free-dimension.json` are written, though constraints.md:296-297 says the infeasible sizes are listed there.

**M3. Some next_actions do not say what to do (sure).**
- "new DRC errors (X): see drc.json" (verdict.rs:832-838) is the usual incomplete line after opens, and it names no step. The fence agent spent two runs on `items_not_allowed` with only this. A table from type to action would help: `items_not_allowed` → keepout or rule area, `copper_edge_clearance` → edge `max_mm` or flush, `solder_mask_bridge` → the project's mask rules, `courtyards_overlap` → `tight_bodies`.
- When KiCad finds unconnected items that the router does not, the action is "see diagnostics in the report" (verdict.rs:798-819), and there may be no diagnostics to see.
- "kicad-cli timed out after N s" (native_report.rs:515) exits 4 without naming `PCB_KICAD_TIMEOUT`.
- `SILKSCREEN_INCOMPLETE` (agent_cli.rs:1030) gives no path and no next step.

**M4. Docs contradict the code (sure).**
- config-reference.md:356 and :358 call `move_seconds` and `total_seconds` a "Wall-clock budget". This comes from the stale doc comments at board_layout.rs:35 and :40. agents.md:359-363 says seconds of work.
- config-reference.md:69 puts `automatic_planes`' supply plane "on the second" inner layer. The code uses the last (pour_plan.rs:85), as agents.md:288-289 says.
- router.md:1782 says "Existing tracks are obstacles". They are removed (board_router/attempt.rs:19), as ROUTE_HELP says.
- placer.md:636 says "Silkscreen labels are not placed", but quality.md:125-126 says the layout moves labels and board texts. placer.md:633 says "No alignment constraints yet", but `row` exists.
- Copper-to-edge clearance on edge parts is described three ways. constraints.md:68 says the router and DRC enforce it. agents.md:593-595 says DRC reports it. config-reference's `copper_edge_clearance_mm` says the placer keeps it, which is the code since d1f18f7.

**M5. The layout moves board-level texts, and agents.md does not say so (sure).** Only quality.md:125-126 mentions it, and there is no switch to keep input text in place. Fence issue 14 is a code problem, but it is also a documentation gap: the agent learned about it from the result.

**M6. `pin_swaps` in layout mode is ambiguous (suspect).** agents.md:304-307 lists `pin_swaps` under "Router options in layout mode", while pin-swap.md:18 puts it in the layout config. Both structs accept the key. A router-side `pin_swaps` reaches the layout's final ladder through `route_kicad_board` (board_layout.rs:1845, board_router/route.rs:111), where it resolves against the placed copy. It may be applied twice or fail to find its file.

## Low

- **L1. Undocumented cosmetic types (sure).** `footprint_type_mismatch` and `nonmirrored_text` are cosmetic (verdict.rs:42-50), but agents.md:408-410 does not list them.
- **L2. Undocumented verdict fields (sure).** `command`, `wall_stretch`, `outputs.placement_report` and `outputs.free_dimension` are missing from the agents.md table.
- **L3. Result-changing switches not echoed (sure).** `RESULT_ENVIRONMENT` (verdict.rs:440-456) leaves out bbdb205's switches, which change results (`PCB_ROUTER_NO_SHARED_STATICS`, `PCB_LADDER_*`).
- **L4. `overhang` ignores unknown keys (sure).** It is a `serde_json::Value` (placement_constraints.rs:384), contrary to "Unknown keys ... are errors" (agents.md:207). The fence README's proposed format (`feature`, `beyond_outline`) passes silently.
- **L5. `drc-kicad-board` does not match the verdict's check (sure).** It has no `--reference` and writes no verdict. The `router_complete_unverified` row (agents.md:388) sends agents there, so the check they run is stricter than the one the verdict uses.
- **L6. Two different fullness thresholds (suspect).** PLACE_HELP's "a side over 90 % full" (agent_cli.rs:1168) disagrees with the 70 % hints (board_placer/place.rs:353).
- **L7. Gaps in agents.md section 3 (sure).** It does not say the constraints path is relative to the project directory (constraints.md:13 does). Its `echo ... > layout.json` overwrites the import's file when run in the project directory. agents.md:591 points to `benchmarks/corpus/run.py --shrink`, which runs only corpus entries.
- **L8. The free-dimension cookbook row fails on an imported board (sure).** It fails on the import's `"outline": {}` with exit 2. The message is clear, but the row should say that the outline needs a width and a height.
- **L9. Unknown commands get no pointer to help (sure).** An unknown top-level command exits 2 without mentioning `--help` (main.rs:718).
- **L10. `plane: true` on a 2-layer board (suspect).** It would bar all tracks from B.Cu, and agents.md:276-282 does not warn about this.

## Verdict consistency (question 2)

- **Agrees:** route, layout and place share one `KiCadVerdict`, and their exit codes 0-4 map identically (verdict.rs:72-81). The free dimension reads the chosen run's verdict and rewrites the paths. Error verdicts are written only once an output directory exists, as documented.
- **Breaks:** the free dimension's `accept: pass` (M2). `drc-kicad-board`, which has its own line format, a different rule and no JSON verdict (L5). The commands that wrap `crate::run` (import, score, describe, export), where an error that is the input's fault but not marked `input_error` exits 4 instead of 2 (H2 is the visible case).
- **Next steps:** most `next_actions` name a setting or step: the `skipped` messages, `room_action`, starved thermals, unmodelled rules, keepouts. The exceptions are listed in M3.

## The cookbook taken literally (question 3)

1. On a machine without KiCad 10 (or on macOS), agents.md never states the requirement. The import fails with exit 4 or reports every footprint missing (H2, H3).
2. "Pass a JSON file instead of auto ... the rules still come from the project" is not fully true (H1).
3. The free-dimension row on an imported board: exit 2 for `"outline": {}` (L8). With `accept: pass` the agent gets exit 1 for a clean board (M2).
4. Keepouts over fixed parts give no warning when there is an outline (M1).
5. Any `pin_swaps` placement under "router options in layout mode" (M6).

The command names and options in agents.md all exist. The library-free DRC config (agents.md:529-542) matches the code as far as I read it.

## Fence items that are documentation problems (question 4)

| Item | State in code | What the docs say | Problem |
|---|---|---|---|
| 13 (report 1): vias in pads | fixed in ded766d (`via_in_pad`) | README:212 says "Open" | stale issue log |
| 15 (report 3): keepouts judging fixed parts | fixed | README:214 says "Open" | stale log; the warning gap is M1 |
| 16 (report 4): `gr_rect` outline | fixed | README:215 says "Open" | stale log |
| 17: refine budget tied to the layout, the ladder stopping silently | fixed (config-reference.md:103; `skipped` explains the stop) | README:216 says "Open" | stale log; pour quality in the verdict is still code work |
| F / 3: `swap_candidates` and `PA15` | describe.rs:49-53 matches `PA*` | — | stale; it needs pads' `pinfunction` (from a schematic), which agents.md:166-169 does not mention and a generated board may lack |
| G / 6: no snapshot release | .github/workflows/snapshot.yml exists | README says "Open" | suspect, not checked online |
| 12: edge copper clearance | the docs disagree (M4) | — | likely fixed by d1f18f7; suspect, since the issue postdates it |
| report 5: plane layer setting, decoupling `U2:2 null` | fixed (`add_pours.plane`, regulator bulk caps) | documented | — |

Code problems remain in 14 (with the doc gap M5), 5/E, 8/C, A, B and 17's pour verdict.

## Help and error messages (question 5)

These are covered above: the exit-4 messages that send the agent to "report it" for KiCad problems (H2, M3), the import's exit-2 promise (H2), the doubly-named `probe_seconds` hint (M2), the silkscreen message (M3) and the unknown-command message (L9). The other usage lines name the usage and the help command, and `IMPORT_INCOMPLETE` is a model of what the others should say.
