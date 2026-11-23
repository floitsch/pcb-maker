# Architecture

A map of the code as it is: the crates and what each owns, the stages of
`layout-kicad-board` and `route-kicad-board` in the order they run, the
data each stage hands to the next, and the file that owns each stage.
Paths are relative to the repository root; a bare file name in the
tables is relative to the `src/` of the crate that holds its module
(below). How each stage decides what it
does is in [router.md](router.md) and [placer.md](placer.md); the settings
are in [config-reference.md](config-reference.md); the state and the open
problems in [handover.md](handover.md).

## The crates

| Crate | Owns | Depends on |
| --- | --- | --- |
| `src/` (the `pcb-maker` binary) | The command line. `agent_cli.rs` is the agent contract: `AGENT_COMMANDS`, the input checks, the exit codes, `verdict.json`; `agent_cli/free_dimension.rs` runs a layout at several sizes of one edge as child processes. `main.rs` dispatches through its `COMMANDS` table and prints the help (`ENVIRONMENT_HELP`); `kicad_commands.rs` and `lab_commands.rs` hold the research and utility commands it runs. | pcb-kicad, pcb-kicad-import, pcb-kicad-lab |
| `crates/pcb-kicad-lab` | The measurement tools the research commands run (below). | pcb-kicad and the crates below it |
| `crates/pcb-kicad` | The top of the KiCad side: `layout-kicad-board` (`board_layout`, `congestion_rank`, `free_dimension`), the verdict (`verdict`) and the other commands' reports and outputs (`describe`, `class_view`, `run_diagnosis`, `silkscreen`, `fab_export`, `fab_release`). Its `lib.rs` re-exports what the binary uses of the crates below, so `src/` reads one API. | pcb-kicad-place, pcb-kicad-route, pcb-kicad-check, pcb-kicad-file |
| `crates/pcb-kicad-import` | `import-kicad-netlist` and the starters it writes. Beside pcb-kicad: neither uses the other. | pcb-kicad-place, pcb-kicad-route, pcb-kicad-check, pcb-kicad-file |
| `crates/pcb-kicad-place` | A board placed: the lowering to the placer's problem, `place_kicad_board` and its seed race, placement constraints, panels, sub-boards, labels. | pcb-kicad-route, pcb-kicad-check, pcb-kicad-file, pcb-placer, pcb-router, pcb-global |
| `crates/pcb-kicad-route` | A board routed: the project's rules, the lowering to the router's problem, `route_kicad_board` and its ladder, pairs, lengths, fan-outs, pours, necks, pin swaps, and `quality`; `settings`, how the settings files are layered (below, "Settings"). | pcb-kicad-check, pcb-kicad-file, pcb-router, pcb-placer, pcb-topo |
| `crates/pcb-kicad-check` | KiCad's own checks: kicad-cli runs with a library-free configuration, the DRC cache, the reports read, a routed result's verification; and the machine: the memory ledger, `/proc` memory, the work rate. | pcb-kicad-file |
| `crates/pcb-kicad-file` | KiCad's files as data: the S-expression tree, canonical order, nets, pad and outline geometry, the outline's pieces, copper written back, statistics, file utilities, the exit codes and the errors that are the input's. | pcb-router, pcb-placer (types only) |
| `crates/pcb-router` | A format-free, in-memory whole-board router: `Board` (nets, terminals, obstacles, planes, rules, via spans) in, `RoutingResult` out. The lattice and its static maps, the pour model, the search, PathFinder negotiation, the repairs, the polish and an exact copper verifier (`verify`). `Router` keeps its state between calls (`new`, `run`, `probe`, `resume`, `update`, `reroute`), which the layout's race and moves use. | none |
| `crates/pcb-placer` | A format-free placer: `Problem` (components, pins, nets, constraints) in, `Placement` (poses, legality, the constraints' status) out, through `place`. | none |
| `crates/pcb-global` | A coarse tile-graph global router (capacities per tile boundary, PathFinder on tiles), the placer's measure: the measured escape demand (`Incremental`, on by default) and two research commands use it. | pcb-router |
| `crates/pcb-topo` | A TopoR-style topological router (a constrained Delaunay mesh, crossings resolved topologically, taut geometry) and `tighten`, which pulls any router's copper taut. Only with the router settings `tuning.engine: "topological"` or `tuning.tighten: true`. | pcb-router |
| `tools/config-reference` | Generates [config-reference.md](config-reference.md) and the environment index at the end of [handover.md](handover.md) from the settings structs and every `PCB_*` name in the sources; `tests/config_reference.rs` fails while either is stale. | pcb-kicad, pcb-placer, pcb-router |

pcb-router and pcb-placer know nothing of KiCad or of each other; the
`pcb-kicad*` crates join them. Each `pcb-kicad*` crate's `lib.rs` says
what it holds and imports, at its root, the modules of the crates below
that it uses, so a module's `crate::kicad_sexpr::...` path names a module
of pcb-kicad-file. Items are `pub` only where a crate above uses them.

## The KiCad side, crate by crate

The seven were one crate, `pcb-kicad`, until 2026-11-11. The cut follows
the module graph: every dependency points down this list, and the
compiler holds it.

- **pcb-kicad-file** (`crates/pcb-kicad-file/src/`). `kicad_sexpr` (the
  S-expression tree, parser and writer), `canonical_order` (footprints by
  reference, their items sorted, so two saves of one design lay out
  alike), `kicad_nets`, `geometry`, `copper_geometry`, `roundrect_pad`,
  `electrical_terminals`, `local_clearance`, `outline` (Edge.Cuts loops
  and cutouts), `outline_pieces` (the outline's pieces without its tabs:
  the sub-boards of a panel, the router's net split, KiCad's check judged
  per sub-board), `copper_emit` (tracks, vias and pad vias written back)
  with `route_graph`, `board_stats`, `flip`, `part_motion`,
  `check_repairs`, `connection_rules`, `corridors_fact`, `fs_util`, and
  `footprint_bodies` (a footprint's body, courtyard and pad boxes, its
  pose written back), and `exit` (the exit codes and `input_error`,
  which every reader of an input marks its errors with).
- **pcb-kicad-check** (`crates/pcb-kicad-check/src/`). `native_report`
  (running kicad-cli and reading its reports), `kicad_config` (a
  library-free KiCad configuration), `drc_cache`, `verification` (ERC,
  DRC with schematic parity; `verify_materialized_rung`) and
  `verification_preview`; `ledger` (the machine-wide memory ledger),
  `proc_memory`, `work_rate` (budgets in work, the rate calibration).
- **pcb-kicad-route** (`crates/pcb-kicad-route/src/`).
  - The project's rules: `project_rules` and `project_rules/dru.rs`
    (`.kicad_pro` and `.kicad_dru` rules lowered to router rules),
    `net_classes`.
  - `board_router/`: the lowering (`lower/`, with `geometry.rs`,
    `pour_plan.rs`, `kept.rs`, `config.rs`), `route.rs`
    (`route_kicad_board`), `ladder.rs`, `attempt.rs`, `budget.rs`,
    `memory.rs`, `diff_pairs/`, `lengths` and `meander` (length and skew
    rules, tuning patterns), `layer_preference`, `bga_fanout`,
    `via_in_pad`, `via_spans`, the result files (`result.rs`,
    `pour_quality.rs`) and `tools.rs` (the track strip of
    `strip-kicad-tracks`; the rest measurement only). Beside it: `necks`
    (connection-width findings and their repair), `pin_swap`,
    `check_work` (the work a KiCad check is charged).
  - `quality` with `aesthetics` and `cost` (`score-kicad-board`): the
    router's lowering reads its net-name heuristics (`ground`,
    `rail_name`) and its census of vias in pads.
- **pcb-kicad-place** (`crates/pcb-kicad-place/src/`), on the router
  side's lowering (the layer table, the router config, pours, BGA array
  rooms, length rules).
  - `board_placer/`: the lowering (`lower/`, with `courtyards.rs`,
    `rules.rs`), `place/` (`place_kicad_board`) and `seed_race.rs`,
    `panel` (panels of copies), `skeleton`, `decoupling`, `edges`,
    `pair_rows`, `marks`, `exits`, `measured` (the escape demand's global
    route), `result.rs`, and `trace.rs` (measurement only). The footprint bodies it places are
    pcb-kicad-file's `footprint_bodies`, which `quality` and the router's
    pour plan read too.
  - `placement_constraints/` (constraints.json), `placement_room`,
    `sub_boards` (the placer's half of a panel of different boards),
    `labels` and `silk_fields`.
- **pcb-kicad** (`crates/pcb-kicad/src/`). `board_layout`
  (`layout_kicad_board`, its stages in `board_layout/`),
  `congestion_rank` (the race's RUDY pre-ranking), `free_dimension`;
  `verdict` (the verdict, `judge`, `route_facts`, the next actions);
  `describe`, `class_view`, `run_diagnosis` (`inspect-routing-run`),
  `silkscreen`, `fab_export`, `fab_release`.
- **pcb-kicad-import** (`crates/pcb-kicad-import/src/`). `netlist_import/`,
  `board_footprints`, `constraints_template`, `key_matrix`.
- **pcb-kicad-lab** (`crates/pcb-kicad-lab/src/`), reached from research
  commands and not from the agent commands: `coarse_probe`,
  `trouble_probe`, `placement_ablation`, `shrink`, `footprint_links`,
  `legality`, and `lowering` (the placed board as the layout routes it,
  which they share).

## `layout-kicad-board`

`agent_cli::layout` (src/agent_cli.rs) checks the project, the output
directory, the router config (a file, or `auto` for the project's rules),
the constraints and kicad-cli, calibrates the work rate, reserves memory
in the ledger, and calls `layout_kicad_board` (crates/pcb-kicad/src/board_layout.rs).
With `free_dimension` it goes to `agent_cli/free_dimension.rs` instead.
Every improving step below first asks the layout's work clock and
wall-clock guard (`WallStop`) whether it may run; a step past the
deadline is skipped and reported as cut.

| # | Stage | Function, file | Hands on |
| ---: | --- | --- | --- |
| 1 | Outline sizing (only with an automatic outline) | `size_outline`, board_layout.rs | the outline in the placer config |
| 2 | Placement | `place_kicad_board`, board_placer/place/ (below) | `placed/<id>.kicad_pcb`, `board-placer.json`, `KiCadBoardPlacerResult`: the kept poses and the other legal seeds' poses |
| 3 | Pin swaps (with `pin_swaps` or `swappable`) | `swap_kicad_pins_in_place`, pin_swap.rs | the placed board, nets permuted |
| 4 | Pours and classes added, the placed board lowered again | `add_pour_zones` (board_router/pour_plan.rs), `lower_placement` (board_placer/lower/) | the board as an `Expr`, a `pcb_placer::Problem` for the moves' legality |
| 5 | Lowering for the router | `core_config` (board_router/route.rs), `lower_for_route` (board_router/diff_pairs/plan.rs, around `lower`) | a `pcb_router::Config` and `pcb_router::Board` |
| 6 | Pre-ranking of the placements | `rank_placements`, congestion_rank.rs | the candidates in RUDY order (sixteen placements, three probed) |
| 7 | The race | `run_race`, `probe_placements` and `pick_placement` (board_layout/race.rs): `Router::new` and `Router::probe` per candidate, side by side as the ledger allows; `race_pick`; `race_lines_probe` | one `Router` (the winner's, resumed), its `RoutingResult`, `KiCadRaceReport` |
| 8 | First route (without a race) | `route_once`, board_layout/race.rs | a `Router` and its `RoutingResult` |
| 9 | Footprint moves where nets fought | `try_moves` and `try_pose` (board_layout/moves.rs): `Router::update` and `reroute` per candidate pose | the best board and router, or the old ones restored |
| 10 | Resume and polish | `Router::resume_polished`, `Router::polish` | the router's routes, cleaned up, fewer vias |
| 11 | Pairs, labels, lengths | `finish_pairs` (diff_pairs/finish.rs), `place_labels` (labels.rs), `finish_lengths` (lengths.rs) | pair copper, label poses, tuning patterns |
| 12 | Write and check | `emit_routes` (board_router/attempt.rs, with copper_emit.rs), `finish_routed_board` (board_router/route.rs): the copper verifier, the board written to `result/`, KiCad's check | `result/<id>.kicad_pcb`, `drc.json`, `board-router.json`, a `KiCadBoardRouterResult` |
| 13 | Final ladder (while connections or starved thermals stay and at least 240 s of work are left) | `route_kicad_board` on `placed/` rewritten with the final poses and no copper (the route flow below); kept when better | `result-ladder/` renamed to `result/` |
| 14 | Starved thermals made solid | `solidify_starved_thermals`, board_router/route.rs | the board, kept where KiCad finds it better |
| 15 | Neck repair | `repair_necks` (route.rs), `repair_connection_width` (necks.rs) | the board, rechecked by KiCad |
| 16 | Report | `finish_report`, board_layout/report.rs | `board-layout.json`, `KiCadBoardLayoutResult` |
| 17 | Verdict | `route_facts` and `judge` (verdict.rs), `write_verdict` (agent_cli.rs) | `verdict.json`, the exit code |

`stop_after_first_route` ends the layout after stage 8 (labels, the
check and the report still run). A four-layer board with pours that is
far from complete after the first route skips stages 9 and 10 and goes
to the final ladder's narrow-signal attempt.

## `route-kicad-board`

`agent_cli::route` handles `--strip` and turns differential pairs on by
need, then calls `route_kicad_board` (crates/pcb-kicad-route/src/board_router/route.rs).

| # | Stage | Function, file | Hands on |
| ---: | --- | --- | --- |
| 1 | A prepared copy (planned pours, net classes, `--strip`) | `strip_tracks`, `add_pour_zones`, `write_net_classes` | `<out>.poured/` |
| 2 | Pin swaps (with `pin_swaps`) | `swap_kicad_pins`, pin_swap.rs | `<out>.swapped/` |
| 3 | Parse and plan | `parse_board` (canonical_order.rs), `plan_vias_in_pads` (`plan` in via_in_pad.rs, re-exported by board_router/mod.rs), `MemoryLimits` (memory.rs) | the `Expr`, layer table, pour nets, memory limits |
| 4 | The attempt ladder | `Ladder::new` and `Ladder::run`, board_router/ladder.rs, over `KiCadRunner` (route.rs). Rungs are pour modes (`ladder_rungs`), pitches the regular one then 0.075 and 0.05 mm (`ladder_pitches`). In order: probe every rung (75 s each, three rungs or more) and continue the leader; the leader's variants first, then other rungs, finer pitches and seed rounds; last the narrow-signal and minimum-clearance steps while connections stay open. Each attempt runs isolated (a panic is recorded, not fatal); KiCad checks are gated (`CheckGate`, budget.rs). | the best attempt's directory, a `KiCadBoardRouterResult` |
| 5 | Starved thermals, neck repair | `solidify_starved_thermals`, `repair_necks` | the board, rechecked |
| 6 | Report | route.rs | `board-router.json`; `<out>.partial` renamed to `<out>` |
| 7 | Verdict | `route_facts`, `judge`, `write_verdict` | `verdict.json`, the exit code |

One attempt (board_router/attempt.rs, `route_kicad_board_once`, or the
runner's `lower`, `probe`, `resume` and `write`):

1. **Lowering.** `lower` (board_router/lower/) makes a `Lowered`
   (`pcb_router::Board` and the expressions to write back);
   `keep_existing_copper` (kept.rs) and `plan_pairs` (diff_pairs/plan.rs) add
   kept copper and planned pairs.
2. **The router.** `Router::new` (crates/pcb-router/src/router/build.rs)
   builds the lattice and the static maps (shared between routers of one
   board). `run`, `probe` and `resume` (router/driver.rs) prepare
   (`routing_order` in order.rs; escapes in escapes.rs, plane skeletons and
   stubs in fixed.rs; corridors in corridors.rs) and negotiate
   (negotiate.rs, batching.rs, give_up.rs).
3. **The finish** (router/finish.rs), in order: the hard repair
   (resolve.rs), the trouble-makers' repair (trouble.rs: an open net
   routed through others at a price, the displaced nets routed again,
   kept when fewer terminals stay open), clean-up (cleanup.rs), the
   length repair, via reduction (vias.rs), the bend round (bends.rs),
   freed thermal spokes (guards.rs), straightening (straighten.rs),
   compaction (compact.rs), per pour net the skeleton's trim, the pour
   analysis and island stitching (stitch.rs), manual spokes
   (manual_spokes.rs), and the routes made concrete (output.rs) into a
   `RoutingResult`.
4. **Writing.** `write_attempt` (attempt.rs): pair copper, meanders
   (lengths.rs), `emit_routes` with copper_emit.rs, then
   `finish_routed_board`.
5. **KiCad's check.** `verify_materialized_rung` (verification.rs) runs
   `kicad-cli sch erc` and `kicad-cli pcb drc --schematic-parity`
   through `run_kicad_report` (native_report.rs), cached on disk by
   drc_cache.rs, and writes `erc.json`, `drc.json` and
   `verification.json`.

The pour model (pour.rs, router/pour_analysis/, fill.rs, spokes/,
sever.rs, reclaim.rs) follows KiCad 10's fill and thermal spokes, so the
router knows which pads a pour joins.

## The placement

`place_kicad_board` (crates/pcb-kicad-place/src/board_placer/place/):
`prepare_board` parses the board, resolves the constraints
(placement_constraints/) and the project rules, applies the outline,
keepouts and sides, and lowers it (`lower_placement`) to a
`LoweredPlacement` holding a `pcb_placer::Problem`. Panels of copies
(`panel::detect`) and sub-boards (`sub_boards::detect_for_placer`) are
found. `race_seeds` (board_placer/seed_race.rs) runs the seeds in a
pool, each by `panel::place_copies`, `sub_boards::place_sub_boards`,
`pcb_placer::skeleton::place_skeleton` (the race over skeletons) or
`pcb_placer::place`. The placements are sorted (illegal parts, closed
pads, relaxation, missed constraints, wirelength), and the kept one is
written with `placement.html` and `board-placer.json`.

Inside `pcb_placer::place` (crates/pcb-placer/src/lib.rs), in order:

1. `fit_halos`; `orient::canonical_start` (orient.rs) with the
   orientation search; `groups::BusGroups::detect` (groups.rs).
2. `global::global_place` (global.rs): the electrostatic descent
   (ePlace), from `initial::quadratic_start` (initial.rs, a
   bound-to-bound wirelength solve of the netlist); stacks.rs glues banks
   on a wide bus into one body for the descent; rotations chosen.
3. Per relaxation level (halos, spacing, fine grid, tight bodies):
   `anneal::anneal` (anneal.rs, with the ratsnest crossings of
   crossings.rs in its cost), `facing::turn_to_face`, `legal::legalize`
   (legal.rs), parts seated on board marks.
4. The refinement (`finish` in lib.rs): `legal::refine_with`, edge
   copper centred, `repair_relations`, `align_orientations`,
   `facing::face`.
5. `exits::repair` (exits.rs, only with `pad_exits`), then the legality
   and constraints report into the `Placement`.

`skeleton.rs` places the active parts first and the passives around the
held skeleton; `constraints.rs` holds the hard and soft constraints;
`buckets.rs` the neighbour grid; `view.rs` the HTML playback.

## Other commands

- `place-kicad-board`: `agent_cli::place` builds the layout's placer
  config (`placement_check_config`, board_layout.rs) and calls
  `place_kicad_board`; KiCad checks the placed board.
- `import-kicad-netlist`: `import_kicad_netlist_options`
  (netlist_import/) exports the netlist with kicad-cli, finds the
  footprints (or takes them from a board, board_footprints.rs), and
  writes the board, `constraints.json` (key_matrix.rs and
  constraints_template.rs for the starters) and `layout.json`.
- `describe-kicad-board` (describe.rs), `fab-kicad-board`
  (fab_release.rs), `drc-kicad-board` (verdict.rs, native_report.rs),
  `inspect-routing-run` (run_diagnosis.rs), `score-kicad-board`
  (quality.rs), `strip-kicad-tracks` and `strip-kicad-copper`
  (board_router/tools.rs, copper_emit.rs).
- The research commands (the `COMMANDS` table in src/main.rs, run from
  src/kicad_commands.rs and src/lab_commands.rs) call the
  measurement-only modules above.

## Settings

Three layers. A settings file (router, placer, layout) holds at its top
level the keys an agent sets: the ones docs/agents.md, docs/constraints.md
and the skill name, the benchmark tasks set and the CLI help lists. The
machinery's own settings sit in one nested group per file, `tuning`
(`KiCadRouterTuning`, `KiCadPlacerTuning`, `KiCadLayoutTuning`): experiment
and calibration knobs no agent needs. Below both is the core's config
(`pcb_router::Config`, `pcb_placer::Config`), which no file reads: the
KiCad layer builds it per attempt. The router's group declares its `core`
section with `tuning_group!` (pcb-kicad-route `settings.rs`): each key is
an optional override of the core field of the same name and type, all set
in one step (`KiCadRouterTuning::apply` in `core_config`), so exposing a
new core knob is one line there and the compiler checks it against the
core. A top-level key that moved into the group is refused with where it
lives now (`"x" moved to "tuning.x"`, `settings_with_moved_keys!`); any
other unknown key is refused as before. config-reference.md lists the
top-level keys and the groups in separate tables.

## Off by default, still in the code

- **The topological engine** (`tuning.engine: "topological"`, pcb-topo) and
  `tuning.tighten`.
- **Pad exits** (`pad_exits`, pcb-placer exits.rs, board_placer/exits.rs,
  pcb-router router/pad_exits.rs), **pair rows** (`pair_rows`,
  board_placer/pair_rows.rs, pcb-placer facing.rs), **length minima**
  (`length_minima`), **the length reroute** (`length_reroute`), **blind
  vias** (`blind_vias`, board_router/via_spans.rs).

Removed on 2026-11-10, each lost or null under its measurement: the move
descent (`move_descent`), bus bundles (`bus_bundles`), the router's
global plan (`global_routing`) and fixed escape stubs (`escape_stub_mm`);
on 2026-11-11 the global route as the race's pre-ranker
(`tuning.congestion_score: "global"`) and global guidance (`global_guidance`);
git history has the code.

## Where things happen

| What | Where |
| --- | --- |
| Work budgets, the rate calibration, determinism | work_rate.rs; pcb-router `set_expansions_per_second`; pcb-placer `work()` |
| The wall-clock guard | pcb-router router/wall.rs; `WallStop` in board_layout.rs |
| Memory reservations | ledger.rs, proc_memory.rs; board_router/memory.rs; pcb-router mem_profile.rs |
| The project's rules and custom rules | project_rules.rs, project_rules/dru.rs; the verdict's `custom_rules` in verdict.rs |
| Exit codes, next actions, `verdict.json` | exit.rs (`EXIT_*`, `input_error`), verdict.rs (`judge`), src/agent_cli.rs |
| The input checks before a run | src/agent_cli.rs (`check_project`, `check_constraints`, `check_pours`, `check_kicad`) |
| Panels | board_placer/panel.rs (copies), outline_pieces.rs and sub_boards.rs (different boards) |
| Differential pairs | board_router/diff_pairs/; pcb-router give_up.rs |
| Length and skew rules | board_router/lengths.rs, meander.rs, layer_preference.rs |
| BGA fan-outs, vias in pads, via spans | board_router/bga_fanout.rs, via_in_pad.rs, via_spans.rs |
| Lattice lines at pin rows | pcb-router pin_lines.rs; `race_lines_probe` in board_layout.rs; the ladder's probes in ladder.rs |
| Silkscreen and labels | labels.rs, silk_fields.rs, silkscreen.rs |
| The fab release | fab_release.rs, fab_export.rs, cost.rs |
| Environment variables | grep the `PCB_*` name; the index in [handover.md](handover.md#switches) |
