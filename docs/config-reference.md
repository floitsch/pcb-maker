# Configuration reference

Generated from the config structs by `cargo run -p config-reference`
(tools/config-reference); `cargo test --test config_reference` fails
while this file is stale. Do not edit it by hand: the descriptions are
the fields' doc comments, the defaults the structs' own (a settings
file of `{}` read through serde; the core configs' `Default`).

- *Docs*: the key is named (in code) in [router.md](router.md) or
  [placer.md](placer.md), where the settings are explained with
  their measurements.
- *internal*: `#[serde(skip)]`, set by the code (the ladder, the
  layout), never read from a file.
- The core tables are not files: their *Set from* column names the
  settings the KiCad layer copies into each field (*the KiCad layer*:
  from the board or a computed value; *-*: the default stands there).
- The *Router report* tables describe an output, `board-router.json`'s
  `rules`, not a setting.

The environment variables are indexed in the hand-over
([handover.md](handover.md#switches)).

## Summary

| Table | Fields | File keys | Internal | With a doc comment | Named in router.md or placer.md |
|---|---|---|---|---|---|
| [Router settings](#router-settings) | 87 | 75 | 12 | 87 | 32 |
| [Router settings](#router-settings) | 87 | 75 | 12 | 87 | 34 |
| [Router settings: topological](#router-settings-topological) | 8 | 8 | 0 | 8 | 0 |
| [Router settings: pin_swap](#router-settings-pin_swap) | 5 | 5 | 0 | 5 | 0 |
| [Router settings: add_pours entries](#router-settings-add_pours-entries) | 5 | 5 | 0 | 5 | 4 |
| [Router settings: net_classes entries](#router-settings-net_classes-entries) | 6 | 6 | 0 | 6 | 1 |
| [Router settings: connection rules](#router-settings-connection-rules) | 4 | 4 | 0 | 4 | 0 |
| [Router report: rules](#router-report-rules) | 5 | - | 0 | 5 | - |
| [Router report: rules from the project](#router-report-rules-from-the-project) | 9 | - | 2 | 9 | - |
| [Router report: custom_rules entries](#router-report-custom_rules-entries) | 4 | - | 0 | 3 | - |
| [Router core](#router-core) | 65 | - | 0 | 65 | - |
| [Placer settings](#placer-settings) | 47 | 43 | 4 | 47 | 17 |
| [Layout settings](#layout-settings) | 17 | 17 | 0 | 17 | 7 |
| [Layout settings: swappable entries](#layout-settings-swappable-entries) | 3 | 3 | 0 | 3 | 0 |
| [Layout settings: free_dimension](#layout-settings-free_dimension) | 11 | 11 | 0 | 11 | 2 |
| [Layout settings: silkscreen](#layout-settings-silkscreen) | 1 | 1 | 0 | 1 | 1 |
| [Placer core](#placer-core) | 8 | - | 0 | 8 | - |
| [Placer core: global](#placer-core-global) | 12 | - | 0 | 12 | - |
| [Placer core: anneal](#placer-core-anneal) | 8 | - | 0 | 8 | - |

## Router settings

The router config file of `route-kicad-board` and `layout-kicad-board` (`auto` resolves it from the project's `.kicad_pro`; a file's keys replace what `auto` would give only where a file sets them, `connection_rules` and `default_rules` together as the classes; docs/router.md). Unknown keys are rejected. `unset` options take the default the doc comment names, mostly the router core's (the next table). Source: `KiCadBoardRouterConfig` in [crates/pcb-kicad/src/board_router/config.rs](../crates/pcb-kicad/src/board_router/config.rs).

| Key | Type | Default | Docs | Description |
|---|---|---|---|---|
| `deadline_seconds` | `Option<f64>` | unset | router.md | Wall-clock seconds from the start of the command by which it returns the best board it has: no attempt starts after it and a running one stops (an agent waits for the answer). Off by default: the budgets count work, so a busy machine routes the same way. |
| `deadline` (internal) | `Option<std::time::Instant>` | unset (internal) | - | `deadline_seconds` as a time, set when routing starts (or by a caller, such as the layout, with a deadline of its own). |
| `connection_rules` | `BTreeMap<String, KiCadConnectionRoutingRules>` | `{}` | router.md | Resolved per-connection geometry, keyed by normalized net name. |
| `default_rules` | `Option<KiCadConnectionRoutingRules>` | unset | router.md | Geometry for connections without an explicit entry. |
| `edge_clearance_mm` | `f64` | `0.0` | router.md | Copper-to-edge clearance from the board outline. |
| `hole_clearance_mm` | `f64` | `0.25` | router.md | Clearance between copper and a hole. |
| `hole_to_hole_clearance_mm` | `f64` | `0.25` | - | Clearance between two holes. |
| `via_cost_mm` | `Option<f64>` | unset | - | Router `via_cost`: a via's cost in millimetres of track (default 8). |
| `against_direction_cost` | `Option<f64>` | unset | - | Router `against_direction`: the cost factor of a step across a layer's preferred axis (default 1.6). |
| `maximum_iterations` | `Option<usize>` | unset | - | Router `max_iterations`: negotiation iterations at most (default 80). |
| `frame_directory` | `Option<PathBuf>` | unset | router.md | Directory receiving the routing after every negotiation iteration as `attempt-NN/frame-NNNN.kicad_pcb` (one attempt per ladder rung), for animations and diagnostics. |
| `grid_pitches_mm` | `Option<Vec<f64>>` | unset | - | Router `pitches`: the candidate lattice pitches, the one aligning best with the pads wins (default 0.127, 0.125, 0.1). |
| `heuristic_weight` | `Option<f64>` | unset | - | Above 1, negotiation searches trade optimality for speed; the cleanup pass always searches exactly. |
| `present_factor` | `Option<f64>` | unset | - | Negotiation's starting price of shared lattice cells. |
| `present_growth` | `Option<f64>` | unset | - | Growth of that price per iteration. |
| `stall_at_cap` | `Option<usize>` | unset | - | Iterations negotiation waits for fewer conflicts once the price is at its cap. |
| `stall_drop` | `Option<f64>` | unset | - | Router `stall_drop`: at the price cap, the share of conflicted nets that must go before negotiation counts it as progress. |
| `global_routing` | `Option<bool>` | unset | - | Plan nets on the tile graph first (global routing). |
| `automatic_planes` | `Option<bool>` | unset | - | On a board with four or more copper layers and no copper pours, ground becomes a plane on the first inner layer and the supply rail with the most pads one on the last (default true; ignored when `add_pours` is given). |
| `mask_only_pad_openings` | `Option<bool>` | unset | - | A pad with a mask opening and no copper (a mask-only test point) opens the mask like a mask graphic: copper of a second net may not go under it (default true). hackrf-one's two solder mask bridges. |
| `via_reduction_present` | `Option<f64>` | unset | - | Price of sharing when a via reduction round starts. |
| `exclusive_planes` | `Option<bool>` | unset | router.md | Keep other nets' tracks off inner-layer planes entirely. |
| `exclusive_layers` | `Option<usize>` | unset | - | With exclusive planes: how many of a pour net's inner plane layers stay exclusive, the ones nearest the outer layers first (the stackups designers use: one solid ground layer under the top on four layers, In1 and In4 on six; ESC mini's GND covers In1-In4 and keeping all four left the signals two layers). The ladder sets it from the layer count when not given; `None` outside the ladder keeps every covered inner layer. |
| `pour_islands` | `Option<bool>` | unset | - | Route pour-net pads the signals cut off from their pour during the negotiation (default on; see `pcb_router::Config::pour_islands`). |
| `kicad_spokes` | `Option<bool>` | unset | router.md | Thermal spokes as KiCad draws and counts them (default on; see `pcb_router::Config::kicad_spokes`). |
| `manual_spokes` | `Option<bool>` | unset | router.md | Straight tracks from pads short of KiCad's two spokes into their pour (default on; see `pcb_router::Config::manual_spokes`). |
| `neck_reach_mm` | `Option<f64>` | unset | - | Reach of the neck width around narrow pads (default 1.5 mm; 0 off). |
| `plane_cut_cost` | `Option<f64>` | unset | router.md | Cost factor of a track (or via) cutting a plane (default 3). |
| `pour_integrity` | `Option<f64>` | unset | router.md | How much a two-layer pour's integrity counts (default 0: off; boards of other layer counts are not affected): the router prices a cut by what it severs of a pour (a corridor that joins the fill's pieces costs more than a nick in a wide area; `pcb_router::Config::pour_integrity`), also for pour nets routed as tracks, and the ladder ranks equally open attempts by the pours' integrity (`pours`: the share outside the largest piece and the piece count) before their vias and copper. |
| `present_cap` | `Option<f64>` | unset | - | Router `present_cap`: the highest price of sharing (default 1e4). |
| `neck_width_mm` | `Option<f64>` | unset | - | Narrowest track the board allows (neck-downs out of small pads). |
| `min_connection_mm` | `Option<f64>` | unset | router.md | The board setup's minimum connection width (`rules.min_connection`) as a floor on every track width the router chooses: class widths, the narrow-signal width, neck-downs, stubs and spokes. Set from the project when KiCad's connection_width check reports at error severity, and at warning severity with `PCB_MIN_CONNECTION_FLOOR=warning` (`project_rules:: connection_width_floor`, `apply_width_floor`); `None`: no floor. |
| `connection_width_severity` | `Option<String>` | unset | - | The project's severity of connection_width findings (`error`, `warning`, `ignore`), for the rules summary. |
| `cleanup_via_cost_mm` | `Option<f64>` | unset | - | Router `cleanup_via_cost`: the via cost while cleaning up (default 25). |
| `cleanup_passes` | `Option<usize>` | unset | - | Router `cleanup_passes`: passes of per-net shortening once the board is conflict free (default 4). |
| `bend_cost_mm` | `Option<f64>` | unset | - | Router `bend_cost`: the cost per 45 degrees of direction change, in millimetres (default 0.1). |
| `pin_row_cost_mm` | `Option<f64>` | unset | router.md | Router `pin_row_cost`: the cost of another net's step between two adjacent SMD pads of a part's pin row, in millimetres (default `PCB_ROUTER_PIN_ROW_COST`, else 2; 0: off). |
| `via_reduction_rounds` | `Option<usize>` | unset | - | Router `via_reduction_rounds`: via reduction rounds after the board is complete (default 3). |
| `strict_via_budget` | `Option<bool>` | unset | - | Router `strict_via_budget` (default on): the via reduction keeps its budget within a round. |
| `run_work` (internal) | `Option<(f64, f64)>` | unset (internal) | - | The run's work budget and what is left of it as the attempt starts (router `run_work`; set by the ladder, not read from a file). |
| `deadline_work` (internal) | `Option<f64>` | unset (internal) | - | The work (seconds of an idle machine) left before the run's nominal end, as the ladder starts (set by the layout; none in route mode). The ladder's decisions of whether another attempt fits are taken in it, not in the wall clock, so that a busy machine decides the same way; the wall-clock deadline stays as the last guard. |
| `router_deadline_work` (internal) | `Option<f64>` | unset (internal) | - | The work at which a router of this attempt stops as at the deadline (router `deadline_work`, its own work counted from zero; set by the ladder from `deadline_work`, not read from a file). |
| `limits_count_all` (internal) | `bool` | `false` (internal) | - | The router's work deadline and negotiation limit count all its work (router `limits_count_all`: its search, pour analyses and build); set by route mode's ladder for its first attempt and the rung probes, not read from a file. |
| `kicad_check_top` (internal) | `Option<usize>` | unset (internal) | - | KiCad checks only the attempts among this many best by the router's count (`CheckGate`; set by the layout for its final ladder, not read from a file). |
| `kicad_kept` (internal) | `Option<(RouterMeasure, (usize, usize))>` | unset (internal) | - | The caller's kept board: the router's measures (open count, vias, copper) and its checked (open count, starved thermals); the ladder's attempts are ranked against it for KiCad's checks (`CheckGate`; set by the layout, not read from a file). |
| `kicad_check_work` (internal) | `Option<f64>` | unset (internal) | - | The work one KiCad check of an attempt counts, in seconds of an idle machine, modelled from the board alone (`check_work`): the ladder adds it to its work for every check it runs, cached or not, so that its end in work does not depend on the DRC cache. Set by the layout, not read from a file; none in route mode. |
| `seeds_side_by_side` (internal) | `Option<usize>` | unset (internal) | - | How many seeds of one attempt route at once (memory; the number of seeds, and so the result, does not depend on it). |
| `cleanup_seconds` (internal) | `Option<f64>` | unset (internal) | - | A cap on the clean-up's work budget (router `cleanup_seconds`; set by the layout for a narrow-signal attempt run alone, whose negotiation needs the work more than its length and vias do). |
| `seeds` | `Option<usize>` | unset | router.md, placer.md | Route every attempt this many ways at once (the deterministic order plus perturbed orders, in parallel) and keep the best: fewest open connections, then vias, then copper. `None`/1: one way. |
| `first_seed` | `Option<u64>` | unset | - | The first of the `seeds` (0: the deterministic order). |
| `retry_seeds` | `Option<usize>` | unset | - | An attempt that leaves connections open is routed again this many perturbed ways at once (default 4; 0: never). Many failures depend on the order: Starling fails unseeded and completes for 5 of 8 seeds. |
| `plane_skeleton` | `Option<bool>` | unset | - | Route pour nets first as a fixed tree on their plane layer. |
| `skeleton_bias` | `Option<f64>` | unset | - | Router `skeleton_bias`: the cost factor of a skeleton step off the plane layer (default 6). |
| `refine_pitches_mm` | `Option<Vec<f64>>` | unset | - | Finer lattice pitches tried, in order, when the routing with the regular pitch leaves connections open. `None` means 0.075 then 0.05. |
| `refine_budget_seconds` | `Option<f64>` | unset | router.md | A finer lattice is only tried when the routing time projected from the previous attempt stays below this. Default: 240 s, a fifth of `ladder_budget_seconds` where that is more, and in a layout 0.4 of the work left before its end where that is more (a layout of 3000 s tries the finer lattice an attempt of up to about 900 s needs). Other ways of connecting the pours are tried whatever the time while connections are open. |
| `ladder_budget_seconds` | `Option<f64>` | unset | router.md | No further attempt of any kind starts after the ladder has run this long (default 1200 s; in a layout the work left before its end, at most this where it is given): ColdFire from its schematic spent 50 min in three pour rungs with their seed retries. |
| `narrow_reserve_share` | `Option<f64>` | unset | router.md | With a work deadline (the layout's ladder) and a project whose final narrow-signal or minimum-clearance step applies, this share of the ladder's first attempt's work is kept back for that step from the start (`narrow_step_reserve`; 0: none, the step gets what the rungs leave). |
| `memory_mb` | `Option<f64>` | unset | router.md | A limit on this process's memory in megabytes (resident), beside `PCB_MAKER_MEMORY_MB` and the limits the process can read itself (RLIMIT_AS, the cgroup's memory.max): the ladder runs as many routers side by side as fit the tightest (`MemoryLimits`). For a harness that kills a run above a resident size it does not set as a limit (the intake's 3 GiB). |
| `work_rate` | `Option<f64>` | unset | router.md | The search expansions a second of work stands for, pinned instead of measured at the start (`work_rate` in a verdict: for reproducing a run on another machine with the rate it recorded; docs/router.md, "Determinism and the wall clock"). `PCB_WORK_RATE` wins over it. |
| `placer_work_rate` | `Option<f64>` | unset | router.md | With `work_rate`: the placement units a second stands for (default: the reference's, scaled as `work_rate` scales the reference's search rate). |
| `negotiation_seconds` | `Option<f64>` | unset | router.md | Seconds one negotiation may run (default 900 s). The ladder sets it per attempt from what is left of its budget, so that a hopeless first rung leaves time for the next (video's exclusive-plane rung ran 1180 s with 222 open, and no other rung ran). |
| `probe_ladder` | `Option<bool>` | unset | - | Large multilayer boards: probe every rung briefly and continue the best (default on). |
| `probe_seconds` | `Option<f64>` | unset | router.md | Seconds of negotiation per probe (default 75). |
| `escape_stub_mm` | `Option<f64>` | unset | router.md | Fixed escape stubs for fine-pitch pad rows, this long in mm (router `escape_stub_mm`; off by default). |
| `fixed_plane_stubs` | `Option<bool>` | unset | router.md | Connect every pour-net pad to its plane by a fixed stub and via before the signals route (router `fixed_plane_stubs`). |
| `trouble_repair` | `Option<bool>` | unset | router.md | The trouble-makers' repair after the hard repair (router `trouble_repair`; docs/experiments/human-level-routing.md): unset, the router's default; `PCB_ROUTER_TROUBLE` overrides this. |
| `thermal_guard_cost` | `Option<f64>` | unset | router.md | Router `thermal_guard_cost`. |
| `narrow_signal_mm` | `Option<f64>` | unset | - | The project's smallest predefined track width (its width menu), which designers use for signals where the class width is a power width (ohdsp's DSP board: class 0.5 mm, 0.135 mm in the menu and on most tracks). |
| `use_narrow_signals` | `Option<bool>` | unset | - | Route signal nets at `narrow_signal_mm` (the ladder tries it when connections stay open at the class widths). |
| `relaxed_clearance_mm` | `Option<f64>` | unset | - | The board's minimum clearance when the project rates clearance findings below errors (a designer who accepted the net classes' clearances as warnings: the A13 module's DDR fan-out keeps 0.149 against classes of 0.2). The ladder's last step routes at it when connections stay open. |
| `solder_mask_to_copper_mm` | `Option<f64>` | unset | router.md | The project's solder-mask-to-copper clearance (`rules. solder_mask_to_copper_clearance`, KiCad's default 0): KiCad's DRC finds a solder mask aperture bridging copper of another net within it plus the aperture's mask expansion (USB_Cable_Tester: a track 0.094 mm from a battery holder's F.Mask polygon over a GND fill, at 0.05 + 0.05 mm). |
| `solder_mask_bridge_severity` | `Option<String>` | unset | router.md | The project's severity of solder_mask_bridge findings (`error`, `warning`, `ignore`; KiCad's default is an error). Below an error the designer accepted bridges, and the router keeps its looser mask rules of before 2026-10-15 (OpenESC 4in1-mini, warnings: the exact rule took its bridges 29 to 2 and cost 52 to 61 unconnected). |
| `solid_starved_thermals` | `Option<bool>` | unset | router.md | Pads KiCad finds short of thermal spokes (where only one fits, as on fine-pitch pins, or whose spokes reach only an isolated island of fill) get a solid zone connection in the result, and the board is verified again (one more KiCad check, charged as work). On unless false (since 2026-10-21: four boards of the v18 sweep failed on starved thermals alone, which this passes; docs/changelog.md). False keeps the designer's thermal reliefs where KiCad finds them starved: reliefs are there for soldering. |
| `via_in_pad` | `Option<bool>` | unset | router.md | Allow vias inside SMD pads (default false): a board whose vias are filled and capped (via-in-pad plating) may put them there. Off, every via keeps its annular ring out of every SMD pad but an exposed pad (a QFN's thermal pad, which holds thermal vias as designers place them): another net's vias keep their clearance beyond the pad's copper, the pad's own net's may touch it from outside. Open vias in a reflowed pad take its solder down the hole (score-kicad-board's "vias in pads" counts them). Through-hole pads hold no via either way. |
| `existing_tracks` | `KiCadExistingTracks` | `"keep"` | router.md | What route-kicad-board does with the board's own tracks and vias (`keep`, the default, or `strip`). Kept, they stay exactly as they are and other nets keep clear of them; pads they join count as connected, so the router routes only what they leave open and never rips one up (a designer's or an agent's pre-routed nets). `strip` (the command's `--strip`) removes them first: the board is routed from nothing. Copper zones stay either way. A layout strips them always (its placement moves the parts). |
| `skip_native_verification` | `bool` | `false` | - | Skip the final native KiCad verification (for timing the router). |
| `allow_below_project_minimum` | `bool` | `false` | - | Route with widths and clearances this file asks for below the project's (a net class's, `default_rules`', `connection_rules`', the neck and narrow-signal widths, `relaxed_clearance_mm`): without it such a file is refused before any work (exit 2) with each value, the project's minimum and its source; with it the run goes on and the verdict warns (`rules.overrides` in board-router.json). KiCad's DRC still holds the board to the project's rules. |
| `engine` | `Option<String>` | unset | - | The routing engine: `lattice` (default, negotiated congestion on a lattice) or `topological` (TopoR-style: triangulation, crossings, layer assignment, any-angle tracks with arcs; pour nets as tracks). |
| `topological_rounds` | `Option<usize>` | unset | - | Rip-up rounds of the topological engine (default 30). |
| `topological` | `KiCadTopologicalConfig` | its own table | - | Weights of the topological engine (defaults in `pcb_topo::Config`). |
| `tighten` | `bool` | `false` | - | Pull the lattice router's tracks tight afterwards (any-angle tracks with arcs, as the topological engine draws them), wherever the result stays legal. |
| `pours` | `KiCadPourMode` | `"auto"` | router.md | How nets with copper pours are connected. |
| `add_pours` | `Vec<KiCadPourRequest>` | `[]` | router.md | Copper pours to add before routing, such as a ground plane on the bottom: each covers the board outline on its layers. |
| `net_classes` | `Vec<crate::net_classes::KiCadNetClassRequest>` | `[]` | - | Net classes to add: track width, clearance and vias for named nets, ahead of the project's own classes and written into its `.kicad_pro`. |
| `pin_swaps` | `Option<PathBuf>` | unset | placer.md | Interchangeable pins (`pin-swaps.json`; relative to the source directory). The nets on them are permuted before routing, in the board and the schematic. |
| `pin_swap` | `KiCadPinSwapConfig` | its own table | - | The pin-swap estimate's settings, for `pin_swaps`. |
| `project_rules` (internal) | `Option<crate::project_rules::KiCadProjectRulesReport>` | unset (internal) | - | What the project's rules gave the router beyond its classes (`project_rules`; reported in `rules`, not read from a file). |
| `net_rules` (internal) | `Option<std::sync::Arc<pcb_router::NetRules>>` | unset (internal) | - | Router `net_rules` (`project_rules`; not read from a file). |

## Router settings: topological

The `topological` object of the router settings (the `engine: "topological"` engine). Source: `KiCadTopologicalConfig` in [crates/pcb-kicad/src/board_router/config.rs](../crates/pcb-kicad/src/board_router/config.rs).

| Key | Type | Default | Docs | Description |
|---|---|---|---|---|
| `topological.crossing` | `Option<f64>` | unset | - | Millimetres one crossing costs while routing. |
| `topological.via` | `Option<f64>` | unset | - | Millimetres a via costs while routing and assigning layers. |
| `topological.improve_via` | `Option<f64>` | unset | - | Millimetres a via costs while improving a legal board. |
| `topological.improve_passes` | `Option<usize>` | unset | - | Passes over all wires while improving a legal board. |
| `topological.spacing` | `Option<f64>` | unset | - | Spacing of via sites in open areas. |
| `topological.seconds` | `Option<f64>` | unset | - | Wall-clock budget of the rip-up rounds. |
| `topological.layered_start` | `Option<bool>` | unset | - | Route the first time layer by layer rather than TopoR's way. |
| `topological.against` | `Option<f64>` | unset | - | Factor on length across a layer's preferred axis (1: none). |

## Router settings: pin_swap

The `pin_swap` object of the router and the layout settings (the pin-swap estimate, docs/pin-swap.md). Source: `KiCadPinSwapConfig` in [crates/pcb-kicad/src/pin_swap.rs](../crates/pcb-kicad/src/pin_swap.rs).

| Key | Type | Default | Docs | Description |
|---|---|---|---|---|
| `pin_swap.crossing_cost_mm` | `f64` | `3.0` | - | Millimetres of ratsnest length one crossing is worth. |
| `pin_swap.maximum_net_pads` | `usize` | `12` | - | Nets with more pads than this (power, ground) are left out of the estimate: they are poured or routed wide anyway. |
| `pin_swap.body_crossing_cost_mm` | `f64` | `10.0` | - | Millimetres charged when a ratsnest line runs through the pad field of a surface-mount part with at least four pads (a detour around it). |
| `pin_swap.moves_per_unit` | `usize` | `2000` | - | Annealing moves per unit. |
| `pin_swap.seed` | `u64` | `0` | - | Seed of the annealing's moves. |

## Router settings: add_pours entries

One entry of `add_pours`. Source: `KiCadPourRequest` in [crates/pcb-kicad/src/board_router/pour_plan.rs](../crates/pcb-kicad/src/board_router/pour_plan.rs).

| Key | Type | Default | Docs | Description |
|---|---|---|---|---|
| `add_pours[].net` | `String` | required | router.md | The net, as in the board (`GND` or `/GND`). |
| `add_pours[].layers` | `Vec<String>` | required | router.md | Copper layers, such as `B.Cu`. |
| `add_pours[].clearance_mm` | `Option<f64>` | optional | - | Clearance between the pour and other nets (default 0.3 mm). |
| `add_pours[].plane` | `bool` | optional | router.md | A plane (default false): no track of any net on these layers, only vias through them, so the pour stays whole on every rung of the ladder (exclusive_planes holds only on some). A rule area keeping tracks off each layer (`plane <net> <layer>`) is written beside the zone, also where the board has the zone already, so KiCad checks it too. |
| `add_pours[].keep_whole` | `Vec<KiCadKeepWhole>` | optional | router.md | Regions where the pour stays whole on these layers (default none): other nets' tracks on the layer and other nets' vias keep out, the pour's own net may go there. Each is a box or the boxes of named parts; a rule area named `keep whole <net> <layer>` marks each (everything allowed for KiCad, which has no rule by net; the router lowers it by its name). |

## Router settings: net_classes entries

One entry of `net_classes`. Source: `KiCadNetClassRequest` in [crates/pcb-kicad/src/net_classes.rs](../crates/pcb-kicad/src/net_classes.rs).

| Key | Type | Default | Docs | Description |
|---|---|---|---|---|
| `net_classes[].name` | `String` | required | placer.md | The class's name in the `.kicad_pro` (not empty, not `Default`). |
| `net_classes[].nets` | `Vec<String>` | required | - | Net names or KiCad wildcard patterns (`*`, `?`), with or without the sheet path's leading `/`. |
| `net_classes[].track_width_mm` | `Option<f64>` | optional | - | Track width of the class's nets (unset: what each net had). |
| `net_classes[].clearance_mm` | `Option<f64>` | optional | - | Clearance of the class's nets (unset: what each net had). |
| `net_classes[].via_diameter_mm` | `Option<f64>` | optional | - | Via diameter of the class's nets (unset: what each net had). |
| `net_classes[].via_drill_mm` | `Option<f64>` | optional | - | Via drill of the class's nets (unset: what each net had). |

## Router settings: connection rules

The value of `default_rules` and of each `connection_rules` entry. Source: `KiCadConnectionRoutingRules` in [crates/pcb-kicad/src/connection_rules.rs](../crates/pcb-kicad/src/connection_rules.rs).

| Key | Type | Default | Docs | Description |
|---|---|---|---|---|
| `connection_rules.<net>.trace_width_mm` | `f64` | required | - | Track width of the connection's net (raised to the width floor; the narrow-signal width replaces it on the attempts that use it). |
| `connection_rules.<net>.clearance_mm` | `f64` | required | - | Copper clearance the net's tracks and vias keep from other nets. |
| `connection_rules.<net>.via_size_mm` | `f64` | required | - | Diameter of the net's vias. |
| `connection_rules.<net>.via_drill_mm` | `f64` | required | - | Drill of the net's vias. |

## Router report: rules

Not a setting: the `rules` object of the router's report (`board-router.json`), what the router made of the project's rules. Source: `KiCadRulesSummary` in [crates/pcb-kicad/src/board_router/result.rs](../crates/pcb-kicad/src/board_router/result.rs).

| Key | Type | Description |
|---|---|---|
| `rules.width_floor_mm` | `Option<f64>` | The minimum connection width applied as a floor on every track width (`KiCadBoardRouterConfig::min_connection_mm`); absent when none was. |
| `rules.connection_width_severity` | `Option<String>` | The project's severity of KiCad's connection_width findings, when its board setup has a minimum connection width. |
| `rules.narrowest_track_mm` | `f64` | The narrowest track width of the nets' classes as routed. |
| `rules.neck_width_mm` | `f64` | The neck-down width out of small pads. |
| `project` (flattened: the next table's keys) | `crate::project_rules::KiCadProjectRulesReport` | KiCad's defaults taken and the custom rules read, kept or skipped. |

## Router report: rules from the project

The fields `project` flattens into `rules`; each is left out when empty. Source: `KiCadProjectRulesReport` in [crates/pcb-kicad/src/project_rules.rs](../crates/pcb-kicad/src/project_rules.rs).

| Key | Type | Description |
|---|---|---|
| `rules.kicad_defaults` | `Vec<String>` | Board setup minimums the project does not state, taken at KiCad's defaults (`min_track_width 0.2 mm`). |
| `rules.custom_rules_file` | `Option<String>` | The custom rules file read (`<project>.kicad_dru`). |
| `rules.custom_rules_error` | `Option<String>` | Why the custom rules file could not be read, when it could not. |
| `rules.custom_rules` | `Vec<KiCadCustomRuleReport>` | Every constraint of every custom rule and what the router made of it (`dru::KiCadCustomRuleReport::status`). |
| `rules.unmodelled` | `Vec<String>` | The project's routing rules the router does not keep, one line each; KiCad still checks them, so their findings are left to the designer (the verdict's next_actions name them). |
| `rules.minimums` | `Option<KiCadProjectMinimums>` | The board's minimum track width and clearance, with where each comes from (the project's board setup or KiCad's default). |
| `rules.overrides` | `Vec<KiCadRuleOverride>` | Widths and clearances a router file asked for below the project's (`allow_below_project_minimum`): the run used them; KiCad's DRC holds the board to the project's. |
| `rules.net_classes` | `BTreeMap<String, Vec<String>>` | Per net (as the router names it): its classes, the highest priority first ("Default" for a net in none). |
| `rules.class_values` | `BTreeMap<String, [Option<f64>; 2]>` | Per class of the project: its track width and clearance as the project states them (before the board's minimums). |

## Router report: custom_rules entries

One constraint of one rule of the project's `.kicad_dru`. Source: `KiCadCustomRuleReport` in [crates/pcb-kicad/src/project_rules/dru.rs](../crates/pcb-kicad/src/project_rules/dru.rs).

| Key | Type | Description |
|---|---|---|
| `rules.custom_rules[].rule` | `String` | The rule's name in the .kicad_dru. |
| `rules.custom_rules[].constraint` | `String` | The constraint (`clearance`, `track_width`, ...). |
| `rules.custom_rules[].status` | `String` | `applied` (the router keeps it as KiCad does), `approximated` (kept more widely than KiCad asks: every layer, every item type, both nets of a pair), `no_effect` (the router's rules already keep at least as much, or nothing it routes matches), `not_routing` (not a rule on the copper the router lays: silk, text, courtyards, pads), `unmodelled` (a routing rule the router cannot keep; KiCad still checks it), `skipped` (its severity or the project's makes its findings no errors, or no layer of the board has it). |
| `rules.custom_rules[].detail` | `String` | (no doc comment) |

## Router core

`pcb_router::Config`, the router itself: not a file. The KiCad router builds it from the router settings in `core_config` (the column names the settings each field is copied from there); the ladder and the layout change more fields per attempt. Source: `Config` in [crates/pcb-router/src/router/config.rs](../crates/pcb-router/src/router/config.rs).

| Field | Type | Default | Set from (`core_config`) | Description |
|---|---|---|---|---|
| `pitches` | `Vec<f64>` | `[0.127, 0.125, 0.1]` | `grid_pitches_mm` | Candidate lattice pitches; the one aligning best with the pads wins. |
| `via_cost` | `f64` | `8.0` | `via_cost_mm` | Cost of a via, in millimetres of trace. |
| `bend_cost` | `f64` | `0.1` | `bend_cost_mm` | Cost per 45 degrees of direction change, in millimetres. |
| `against_direction` | `f64` | `1.6` | `against_direction_cost` | Cost factor for running across a layer's preferred axis. Layers alternate between horizontal and vertical preference. |
| `present_factor` | `f64` | `0.5` | `present_factor` | The price of sharing a lattice node with another net (the present congestion factor) in the negotiation's first iteration; a reroute starts at the price the negotiation reached, at least this. |
| `present_growth` | `f64` | `1.5` | `present_growth` | Factor on the price of sharing after every negotiation iteration, up to `present_cap`. |
| `present_cap` | `f64` | `10000.0` | `present_cap` | Upper bound of the present-congestion factor. Beyond it contested nodes act as walls the heuristic knows nothing about, and searches flood; history cost keeps separating the nets instead. |
| `fixed_plane_stubs` | `bool` | `false` | `fixed_plane_stubs` | Connect pour nets' pads to their planes first (the pour nets negotiating among themselves) and keep those stubs and vias fixed through negotiation. Off: on ColdFire the pre-pass never settles and fixed stubs left clearance violations. |
| `exclusive_planes` | `bool` | `false` | `exclusive_planes` | Keep other nets' tracks off exclusive planes entirely (else they pay `plane_cut_cost` there, as designers do cut planes locally). |
| `neck_reach` | `f64` | `1.5` | `neck_reach_mm` | Within this distance of a net's own narrow pads (narrower than its track plus clearance) the net routes at the board's neck width: the fan-out of a fine-pitch part. 0 turns it off. |
| `global_routing` | `bool` | `false` | `global_routing` | Plan every net on the tile graph first and confine its lattice search to that plan's corridor; conflicts teach the plan. |
| `stall_at_cap` | `usize` | `12` | `stall_at_cap` | Iterations without fewer conflicted nets that negotiation waits once the price of sharing is at its cap. |
| `stall_drop` | `f64` | `0.0` | `stall_drop` | At the price cap, how much fewer conflicted nets count as progress (a share of the last low; at least two nets when above zero). Zero, the default, turns it off: any new low resets the stall count, as below the cap. That can let a board crawl down one net every few iterations for hundreds of seconds (Sisu: 27-32 conflicted from iteration 6 to 38), so the layout's in-place router (its first route and the moves, which compare placements) and its placement race set 0.1 unless the router settings give `stall_drop`; the final ladder and the first route's resume keep the default. |
| `stall_patience` | `usize` | `25` | - | Iterations without fewer conflicted nets after which negotiation stops below the price cap (`stall_at_cap` at it). |
| `abandon_hopeless` | `bool` | `true` | - | Stop negotiating when a quarter of the nets have no path at all after the first iteration (see `Router::hopeless`). |
| `pour_islands` | `bool` | `true` | `pour_islands` | During negotiation, check every iteration which pads of a pour net the signals have cut off from the pour's main piece, and route those to it like off-pour pads (a via into the plane, mostly), so that the supply nets negotiate with the signals instead of being repaired after them, when the room is gone (ESC mini: 41 of GND's pads stranded after the stitching). Pads whose piece rejoins lose their stub again (a sticky first version left the early iterations' stubs in the signals' way). Connect rungs with it: ESC mini 22 open against 27, Hub 123 against 135; tracks mode unchanged; it costs iterations. |
| `spoke_clearance` | `bool` | `true` | - | A pad's thermal spoke counts only where its strip keeps the clearance from other nets' copper all the way to the fill, as KiCad's does (`analyze_pours_full`). |
| `kicad_spokes` | `bool` | `true` | `kicad_spokes` | Thermal spokes as KiCad draws and counts them (`kicad_spokes`): the spoke's end at the gap and half the minimum width, its width no more than the pad's, a piece that joins two pads is no island, and a pad's own track into the fill counts as a spoke. |
| `manual_spokes` | `bool` | `true` | `manual_spokes` | A pad that the spoke model finds short of KiCad's two spokes gets a straight track into the main piece of its pour, which KiCad's DRC counts as a spoke (`add_manual_spokes`, after the stitching). |
| `island_stubs` | `bool` | `false` (`true` with `PCB_ROUTER_ISLAND_STUBS`) | - | A pad whose spokes on a layer reach only fill pieces nothing else of the net touches (KiCad's "spokes connected to isolated island") and that neither a straight manual spoke nor a stitching via from the island to the net's main piece on another layer joins to a kept piece gets a straight track of the pad on the island's layer into the island (`add_manual_spokes`): KiCad's connectivity joins the track to the piece it overlaps, the piece has two items and is no island, and the DRC counts its spokes and the track. It changes no connection (the island was the pad's already); k30 and mackerel-30's tracks rungs, whose inner planes the signals cut into islands around through-hole pads. Off since 2026-10-21 (Florian: it only silences KiCad's isolated-island finding; the island vias stay on); `PCB_ROUTER_ISLAND_STUBS=1` turns it on. |
| `spokeless_tracks` | `bool` | `true` (`false` with `PCB_ROUTER_NO_SPOKELESS_TRACKS`) | - | A pad the spoke model finds with no spoke at all into a pour it lies in, and no track of its own into the fill, gets one straight track into kept fill all the same (`add_manual_spokes`): the model is least sure where it sees none (the spoke ends lie in the narrowest places), KiCad's DRC counts the track only while the pad is short, and a pad KiCad also gives no spoke is no finding with or without it. The expansion intake: 22 of 85 new starved thermals on 40 boards were pads the model saw without a spoke and KiCad with one (`PCB_ROUTER_NO_SPOKELESS_TRACKS` turns it off). |
| `apart_trees` | `bool` | `true` (`false` with `PCB_ROUTER_NO_APART_TREES`) | - | Terminals of a net that no path reaches from its tree (a board-wide search failed, `NetState::unreachable`) are joined among themselves where they reach each other: a tree of their own, which the net's count still calls unconnected (KiCad finds one unconnected item between the trees, not one per terminal). jiran-ble-lite's two halves share one GND and are joined only through a frame no track crosses: the right half's 20 GND pads stayed apart, also from each other (KiCad 20 unconnected items, the designer's own board 1). `PCB_ROUTER_NO_APART_TREES` turns it off. |
| `lost_spoke_repair` | `bool` | `true` (`false` with `PCB_ROUTER_NO_LOST_SPOKES`) | - | After the clean-up, a pad the spoke model finds starved whose failed spokes other nets' copper cuts (in the spoke's strip, or where its end's fill would be) has those nets routed again around the strips, kept when every net involved stays complete and the pour nets' starved spokes go down (`free_lost_spokes`; `PCB_ROUTER_NO_LOST_SPOKES` turns it off). |
| `trouble_repair` | `bool` | `true` | `trouble_repair` | After the hard repair, each net still open (pour nets aside), most open first, is routed through other nets' copper at a price; the nets it then overlaps lose the branches it crosses and are routed again hard, and the exchange is kept when the open terminals of all of them together drop; not in a layout's move trial (`repair_trouble`, router/trouble.rs; docs/experiments/human-level-routing.md). `PCB_ROUTER_TROUBLE` overrides it (`0` off, `1` on). |
| `pour_aware_polish` | `bool` | `true` | - | The clean-up keeps a change only if no pour net has more pads off its main piece after it. |
| `pour_aware_via_reduction` | `bool` | `true` | - | The same for a via reduction round. |
| `pour_whole_rip_up` | `bool` | `true` | - | Whether a pour net stuck in conflict is ripped up whole now and then like the other nets (`rip_up_whole`). |
| `escape_stub_mm` | `f64` | `0.0` | `escape_stub_mm` | Fine-pitch pads (narrower than the track plus its clearance, on one layer, in a row of their kind) get a straight stub outward fixed before negotiation, up to this long in mm (0, the default, turns it off). The stubs keep every pad's only way out open: nothing else may run along the row inside the escape zone. Tried on Tiny Tapeout (82/108 either way) and ColdFire (205 vs 208 of 209): no gain yet. |
| `history_increment` | `f64` | `0.3` | - | History cost added to a lattice node (and its tile) for every net still in conflict there after an iteration; also the amplitude of the noise a `seed` puts into the history. |
| `max_iterations` | `usize` | `80` | `maximum_iterations` | Negotiation iterations at most (each reroutes the conflicted nets). |
| `window_margin` | `f64` | `10.0` | - | Minimum search window margin around a net's terminals. |
| `cleanup_passes` | `usize` | `4` | `cleanup_passes` | Passes of per-net shortening after the board is conflict free. |
| `straighten` | `bool` | `true` (`false` with `PCB_ROUTER_NO_STRAIGHTEN`) | - | After the via reduction and the thermal spoke repairs, take stair-steps and jogs out of the signal nets' paths where the clearance allows (`straighten`: shortest octilinear paths between a run's corners with fewer bends; never longer, no via moves). |
| `bus_bundles` | `bool` | `false` (`true` with `PCB_ROUTER_BUS_BUNDLES`) | - | Route buses as bundles (an experiment, off; docs/experiments/bus-bundles.md): four or more nets joining the same footprints route first, in pad order, inside one corridor planned for the bus's width, side by side; a member that cannot follow routes as any other net. `PCB_ROUTER_BUS_BUNDLES` turns it on. |
| `bus_lane_cost` | `f64` | `1.5` | - | With `bus_bundles`: the factor on a bus member's trace step that does not lie beside a neighbouring member's copper (`PCB_ROUTER_BUS_LANE`, 1.5). |
| `cleanup_via_cost` | `f64` | `25.0` | `cleanup_via_cost_mm` | Via cost while cleaning up. The existing route is always a fallback there, so a high value removes vias without risking completion. |
| `plane_via_cost` | `f64` | `2.0` | - | Via cost when connecting to a copper pour: a short stub and a via is the preferred connection, not a long track to another pad. |
| `corridors` | `bool` | `true` | - | Plan every connection on a coarse tile graph first and search the lattice only inside that corridor (widening it on failure). |
| `plane_cut_cost` | `f64` | `3.0` | `plane_cut_cost` | Cost factor for a trace step on a node covered by another net's pour, and for a via through one: cutting a plane is expensive. |
| `pour_integrity` | `f64` | `0.0` | `pour_integrity` | Weight of what a cut severs (default 0: off; two-layer boards only; 1 on the fence lost the 46 mm board's completeness): a step or via on a node of another net's pour fill where the fill is a narrow corridor that joins its pieces pays `plane_cut_cost` times `1 + 20 * weight * severed`, `severed` being the share of the pour the corridor's cut would split off from its largest part (`Router::refresh_sever`, recomputed from the fill model before every batch). It also lets the pour of a net routed as tracks price the cut (its fill, as KiCad would pour it, is `covered`). |
| `plane_skeleton` | `bool` | `false` | `plane_skeleton` | Route every pour net first as a tree on its best-covered layer and keep that tree fixed: the pour then only adds copper, and signal routing cannot strand a pad on an island. |
| `skeleton_bias` | `f64` | `6.0` | `skeleton_bias` | Cost multiplier for skeleton steps off the plane layer. |
| `via_reduction_rounds` | `usize` | `3` | `via_reduction_rounds` | After the board is complete, renegotiate the nets that have vias with the via cost multiplied by `via_reduction_factor`, this many times, keeping a round only if nothing opens and vias go down. |
| `via_reduction_factor` | `f64` | `2.0` | - | Factor on the via cost per via reduction round: round k (from 0) routes at the via cost times this to the power k + 1. |
| `via_reduction_weight` | `f64` | `1.0` | - | Heuristic weight while reducing vias (searches there are long same-layer detours; a little greed keeps them cheap). |
| `via_reduction_present` | `f64` | `0.5` | `via_reduction_present` | Price of sharing when a via reduction round starts. |
| `via_reduction_budget` | `f64` | `3.0` | - | Via reduction stops once it has used this many times the main negotiation's time (at least 60 s). |
| `via_reduction_seconds` | `f64` | `300.0` | `negotiation_seconds` | Seconds via reduction may take at most, whatever the negotiation took (default 300 s; the KiCad ladder sets it from what is left of its budget: a 210 x 170 mm board routed clean in 130 s and then spent the runner's remaining 770 s polishing). |
| `strict_via_budget` | `bool` | `true` | `strict_via_budget` | The via reduction keeps its budget within a round too: a round whose cost (projected from the previous round) exceeds what is left does not start, and a round's negotiation and clean-up get only what is left. Without it a round started with budget left ran to its end (k30-SBC: a 60 s budget, one round of 182 s of work and a clean-up after it; CyberKeeb2040: 186-206 s for 87-117 s). |
| `run_work` | `Option<(f64, f64)>` | unset | `run_work` | The whole run's work budget (the ladder's or the layout's, in work seconds) and what was left of it when this router started (its own work since counted against it). With more than half of the run's budget unspent when the via reduction starts, it may use up to a third of what is left, and at most 300 s (k30-SBC's round of 182 s of work, the one that took it from 414 to 354 vias, fits by right then); with less, its own budget applies strictly. |
| `via_reduction_keeps_spokes` | `bool` | `false` | - | A via-reduction round is kept only if it starves no more thermal pads (`starved_pads`) than before. Off: the router's count is not KiCad's (mackerel-08: 0 against 5); on the quick tier it took starved thermals from 13 to 10 but cost k30-SBC its 55-via round and 5 more starved pads in KiCad's count. |
| `cleanup_seconds` | `f64` | `180.0` | `cleanup_seconds` | Seconds the clean-up may take at most: no batch starts later than this, nor later than the negotiation took (at least 60 s). A 130 x 90 mm six-layer board spent 470 s in clean-up. |
| `thermal_guard_cost` | `f64` | `10.0` | `thermal_guard_cost` | Cost factor for another net's step on the ring around a pad that joins its pour by thermal spokes (copper there starves the spokes). |
| `pin_row_cost` | `f64` | `2.0` | `pin_row_cost_mm` | Cost in millimetres of trace of a step into a pin-row gap (`Board::pin_row_gaps`) by a net other than the gap's two pads': a track snaking between a part's adjacent SMD pins pays it once or twice per gap it crosses. 0 turns it off (no gap marks built); `PCB_ROUTER_PIN_ROW_COST` sets it (default 2: from 3 mm on the expansion tier loses to the thermal guards, docs/router.md). |
| `negotiation_seconds` | `f64` | `900.0` | `negotiation_seconds` | A negotiation that has not converged after this many seconds is handed to the resolution step as it is. |
| `negotiation_expansions` | `u64` | `0` | `negotiation_seconds` | Search expansions one negotiation may spend (0: no limit but the seconds). Unlike seconds, independent of the machine's load. |
| `parallel` | `bool` | `true` | - | Route spatially disjoint nets of one iteration in parallel. |
| `heuristic_weight` | `f64` | `1.0` | `heuristic_weight` | Values above 1 trade path optimality for search speed. |
| `seed` | `Option<u64>` | unset | - | Perturbs the net order and seeds the history with noise of one history increment, so the same board can be routed several ways (`None`: the deterministic order, or `PCB_ROUTER_SEED`'s). |
| `verbose` | `bool` | `false` | the KiCad layer | Print progress to stderr: one line per negotiation iteration and what the clean-up, the via reduction and the spoke repairs did (the KiCad router always sets it). |
| `deadline` | `Option<std::time::Instant>` | unset | `deadline` | A wall-clock time by which routing returns what it has, however much work its budgets still allow: every budget counts as spent from then on (an agent waits for the result). |
| `deadline_work` | `Option<f64>` | unset | `router_deadline_work` | The work (this router's own, in seconds of an idle machine) at which routing returns what it has, as at `deadline`: the caller's deadline counted in work, so that where a run stops does not depend on how busy the machine is (the layout sets it from its own budget; `deadline` stays as the guard against load). |
| `net_rules` | `Option<std::sync::Arc<crate::rules::NetRules>>` | unset | `net_rules` | Rules beyond the net classes (`crate::rules`): per-net width floors, clearances between sets of nets, per item type and per layer, and the same-net hole-to-hole spacing. `None` (or empty rules): the classes alone, as before. |
| `share_static_maps` | `bool` | `true` | - | Share the static maps (the outline, every obstacle's nodes, the tight steps of every class) with the other routers built from the same board, lattice and classes (`hold_static_maps`): a router then builds only its own maps; what it counts is the same either way (`build_work`, with the shared parts keyed in `shared_build` for the ladder to charge once). False builds them anew and keys nothing (every router's build charged whole). |
| `limits_count_all` | `bool` | `false` | `limits_count_all` | The work deadline (`deadline_work`) and the negotiation's work limit (`negotiation_expansions`, in seconds at the reference's rate) count this router's pour analyses and fill probes beside its search, the deadline its build too: all of its work as route mode's ladder charges it. False: its search alone. The ladder sets it where it has no proportion of search to go by (its first attempt, the rung probes). |

## Placer settings

The placer config file of `place-kicad-board`, and the `placer` object of the layout settings. Unknown keys are rejected; a missing key takes its default. Source: `KiCadBoardPlacerConfig` in [crates/pcb-kicad/src/board_placer/config.rs](../crates/pcb-kicad/src/board_placer/config.rs).

| Key | Type | Default | Docs | Description |
|---|---|---|---|---|
| `spacing_mm` | `f64` | `0.5` | - | Minimum gap between two courtyards on colliding sides. |
| `grid_mm` | `f64` | `0.635` | - | Footprint origins are snapped to multiples of this. |
| `keep_rotation` | `bool` | `false` | - | Movable parts keep the angle the board gives them; otherwise each may take any quarter turn of it (and square angles too when it stands at an odd one). |
| `fixed` | `Vec<String>` | `[]` | router.md, placer.md | References that must keep their pose, in addition to locked footprints, footprints without nets, and footprints on the board edge. |
| `fixed_patterns` | `Vec<String>` | `[]` | placer.md | Glob patterns (`*` and `?`) of references that stay fixed. |
| `edge_keep_mm` | `f64` | `3.0` | - | Through-hole parts with at least four pins (connectors, headers) whose body comes within this distance of the outline stay where the designer put them: at the edge, where they are reachable. |
| `free` | `Vec<String>` | `[]` | placer.md | References that may move even though a default rule would fix them. |
| `whitespace_fill` | `f64` | `0.0` | - | Share of the whitespace taken by filler charge: 1 packs the parts as tightly as halos allow, 0 spreads them over the whole board. 0 by default: the spare room then lies between the parts, where routing needs it (routed in route mode, Sisu's placement left 11 open in KiCad at 0 and 20 at 0.6; OpenESC mini 62-70 against 80-86 over two seeds; wirelength within 3 %). |
| `track_pitch_mm` | `f64` | `0.65` | - | Every footprint keeps a halo of `pins / pins_per_halo_track` tracks (at `track_pitch_mm`, capped at `maximum_halo_mm`) free around its courtyard so its pins can escape. |
| `pins_per_halo_track` | `f64` | `8.0` | - | Pins per track of halo (see `track_pitch_mm`); 0 gives no part a pin-count halo. |
| `halo_scale` | `f64` | `1.0` | - | Factor on the routing halos the pin counts ask for (not on `halo_overrides_mm`): less room for escapes, shorter wires. |
| `maximum_halo_mm` | `f64` | `4.0` | - | The largest pin-count halo, in mm, before `halo_scale`. |
| `halo_overrides_mm` | `BTreeMap<String, f64>` | `{}` | - | Per-reference halos replacing the pin-count rule (routing feedback). |
| `maximum_utilization` | `f64` | `0.6` | - | Share of the free board that bodies, halos and spacing may claim. |
| `edge_margin_mm` | `f64` | `0.5` | - | Distance movable courtyards keep from the board edge (at least the copper-to-edge clearance, since pads may reach the courtyard). |
| `seed` | `u64` | `1` | - | The global placement's seed for the first of the `placement_seeds`; seed k of the race uses this plus k. |
| `deadline` (internal) | `Option<std::time::Instant>` | unset (internal) | - | Wall-clock time by which placement returns its best result (set by the layout from its budget; placement of a 404-part panel ran past 25 minutes). |
| `work_seconds` (internal) | `Option<f64>` | unset (internal) | - | Placement work each seed may do, in seconds of an idle machine (see `core::work_per_second`): set by the layout from its budget, so that a busy or a slower machine places the same way. |
| `overlap_far_side_pads` | `bool` | `false` | - | Where the project lets courtyards overlap, a part may reach over other parts' pads on its side at the last placement levels, keeping only copper apart (as katia's designer put connectors partly over hot-swap socket pads). Off by default: such pads may carry a body. |
| `constraints` | `Option<KiCadConstraintsSource>` | unset | placer.md | Placement constraints: a `constraints.json` path (relative to the source directory) or the constraints inline. |
| `constraint_weight` | `f64` | `50.0` | - | Millimetres of wirelength one millimetre of a missed near or relative constraint costs. |
| `copper_clearance_mm` | `Option<f64>` | unset | - | The largest copper clearance of the board's rules: bodies are never placed closer than this (pads may sit on a body's edge). Read from the project when not given. |
| `hole_clearance_mm` | `Option<f64>` | unset | router.md | The board's copper-to-hole clearance: a non-plated hole keeps it from other parts' copper. Read from the project when not given. |
| `copper_edge_clearance_mm` | `Option<f64>` | unset | - | The board's copper-to-edge clearance: pads of parts held at an edge keep it. Read from the project when not given. |
| `tight_bodies` | `Option<bool>` | unset | - | When nothing else fits, bodies may shrink to their fabrication outline and pads (courtyards overlap). Read from the project when not given: allowed unless KiCad's DRC treats a courtyard overlap as an error. |
| `holes_in_courtyards_error` | `Option<bool>` | unset | - | Whether the project makes a hole inside another part's courtyard an error (`pth_inside_courtyard` or `npth_inside_courtyard`); `None`: read it from the project. Where it does not, a part's holes and pads on the other side keep only the copper clearance from that side's pads (leads through the board still keep off bodies). |
| `auto_decoupling` | `bool` | `true` | - | Decoupling capacitors (small capacitors between a supply rail and ground) are pulled to the supply pins of the ICs on their rail, unless a constraint already names them. |
| `decoupling_mm` | `f64` | `2.5` | - | How close (body to pin) a decoupling capacitor up to 4.7 uF is pulled to its IC's supply pin (2.5 mm). At 2.5 mm a third of the capacitors ended within 3 mm of their pin where designers put 60 % there (quick tier); 1 mm gave 25-78 % on five boards with the same routes (not measured on the tier). |
| `keep_on_pieces` | `bool` | `true` | placer.md | On a board of several pieces (two boards side by side), the parts a net joins to fixed parts on one piece stay on that piece. On: zpn_devboard passes (its parts had spread over an empty breakaway, 129 open after the first route); nothing changes on the quick tier. |
| `keep_copies` | `bool` | `true` | placer.md | On a panel of copies joined by tabs (one outline piece), each copy's movable parts stay in its copy's region (see `keep_copies_apart`): the copies are told apart by their nets. On: boards whose nets form one group do not change. |
| `solid_pin_rows` | `bool` | `true` | - | A footprint without a courtyard whose pads stand in rows of pins (three or more at most 2.6 mm apart, holding three quarters of its pads: a QFP, an SOIC, a header, a card slot) blocks with the whole box of its pads, as a body there would; other courtyard-less footprints (a key switch, a hot-swap socket) block with their pads alone. On: hackrf's header and SOIC sat inside a turned LQFP-144's pad ring, homebrew_486's second ISA slot between the first's rows. |
| `mask_openings` | `bool` | `true` | placer.md | A footprint's solder-mask openings on its own side (graphics on F.Mask or B.Mask) keep other footprints' pads and bodies off also where the footprint blocks with its courtyard's shape rather than its whole body (the body's box holds them; a courtyard-less part blocks with them already): SNSP-CPU-01's J2 (a 12 x 5 courtyard around four pins, a 22 x 32 mm F.Mask polygon beside it) held T1 and C61, and KiCad reported solder_mask_bridge for each of their pads. They keep the bodies' spacing (at least the copper clearance) or KiCad's mask margin where that is larger (`mask_opening_growth`). On. |
| `solder_mask_to_copper_mm` | `Option<f64>` | unset | router.md | The project's `solder_mask_to_copper_clearance`: covered copper keeps it from a mask opening. Read from the project when not given. |
| `shared_halos` | `bool` | `false` | placer.md | Neighbours keep the larger of their routing halos between them, not the sum (`core::Problem::shared_halos`). |
| `anneal_far_side_gaps` | `bool` | `false` | placer.md | The annealer keeps the legality test's gaps across the board (`core::anneal::AnnealConfig::legal_far_side`). Off by default. |
| `routing_room` | `bool` | `false` | placer.md | Routing room from congestion (`core::room`): after a placement with the pin-count halos, each part's halo is scaled by the routing demand (RUDY, nets a pour carries left out) around it relative to the board's median, within the same halo area, and the parts anneal again from there. Off by default. |
| `routing_room_share` | `f64` | `1.0` | placer.md | Share of the pin-count halos' area the routing-room halos take. |
| `routing_room_floor` | `f64` | `0.5` | - | Parts with less demand than this share of the median get no halo. |
| `routing_room_most` | `f64` | `2.0` | - | The largest factor on a part's pin-count halo. |
| `routing_room_all_parts` | `bool` | `true` | placer.md | Every part's pin-count halo is scaled by its demand, also where the level left it none (`core::room::RoutingRoom::all_parts`); false: only the halos the level kept are redistributed. |
| `placement_seeds` | `usize` | `3` | placer.md | Placements tried with different seeds (in parallel); the best is kept: fewest illegal parts, least missed constraints, least wire. |
| `routing_demand` | `f64` | `0.0` | - | Routing demand as charge in the global placement: every net's estimated wire area (half perimeter times the track pitch, over its signal layers) spreads over its bounding box and pushes parts apart where the wires will run, scaled by this (0: off). |
| `orientation_search` | `bool` | `true` | placer.md | The placement's orientation search starts from a canonical pose (`core::orient`): every movable part that may turn at the first of its angle options (in a fixed order), so that the placement is the same whether the board holds 0 degrees (an import from a schematic) or the designer's angles (a stack). On; off: the parts start at the angles the board holds. |
| `keep_board_marks` | `bool` | `true` | placer.md | The board's own texts (header pin names, a title) and silkscreen graphics (a logo, a frame around a group) stay where the input put them: each is a placement obstacle on its side with a keepout's semantics (texts on F.SilkS, B.SilkS, F.Cu and B.Cu by their box from font size, thickness and justification; graphics on the silkscreen by their strokes, in clusters, leaving out a cluster that spans most of the board; a part meets a silkscreen mark without its routing halo; a mark on a movable part at its input position belongs to that part and is none: `attached_marks` in `board-placer.json`), and a part that finds no room off them at a level lies on them (`parts_on_marks` in `board-placer.json`); the layout routes around copper texts instead of moving them afterwards, and the polish moves only the parts' reference and value labels. Movable parts whose silkscreen or pads lie on a mark are reported (`labels.overlaps` in `board-layout.json`, the verdict's next_actions); fixed parts on a mark get a warning. On; off: the behaviour before 2026-10-21 (the marks are no obstacles, the polish moves them off the parts). |
| `attached_marks` (internal) | `Option<Vec<(usize, usize)>>` | unset (internal) | - | The board's marks that belong to a movable part, taken from the input (`attach_marks`: the mark's place among the board's marks, the part); set by the layout for the lowerings of the placed board, where the parts no longer lie where the input had them. |
| `global_guidance` | `Option<KiCadGlobalGuidance>` | unset | - | The global route inside the placer's search (an experiment, off): the anneal re-evaluates the global route of its placement and prefers moves away from overflowing boundaries, proposing the parts whose nets cross them more often; the coupled loop moves those parts first (docs/global-route.md, "Phase 2"). |
| `guidance_router` (internal) | `Option<KiCadBoardRouterConfig>` | unset (internal) | - | The router's configuration the guidance lowers with (set by the layout; the default otherwise). |

## Layout settings

The layout config file of `layout-kicad-board`. Unknown keys are rejected; a missing key takes its default. Source: `KiCadBoardLayoutConfig` in [crates/pcb-kicad/src/board_layout.rs](../crates/pcb-kicad/src/board_layout.rs).

| Key | Type | Default | Docs | Description |
|---|---|---|---|---|
| `moves` | `usize` | `40` | - | Footprint moves to try after the first full route. |
| `patience` | `usize` | `12` | - | Stop after this many consecutive moves without improvement. |
| `steps_mm` | `Vec<f64>` | `[0.5, 1.0, 2.0, 4.0]` | - | Move distances tried, in millimetres, in the four axis directions. |
| `move_seconds` | `f64` | `600.0` | - | Budget of the move phase, in seconds of work (search expansions and placement units at the run's work rates; docs/router.md, "Determinism and the wall clock"). |
| `polish_seconds` | `f64` | `60.0` | - | Once every connection is routed, moves only polish (fewer vias, less copper): they stop after this many seconds of the move phase. |
| `total_seconds` | `f64` | `1500.0` | router.md | Budget of the whole layout, in seconds of work (as `move_seconds`; the wall clock only guards it: `wall_guard`): the placement race stops at 40 % of it, the moves at 65 %, and the final ladder gets what is left. Every route stops by it (then only the native check runs). An agent waits for this; the best board so far is what it gets. |
| `wall_guard` | `f64` | `2.0` | router.md | How far past `total_seconds` (counted in work) the wall clock may run before the layout's phases are cut short, as a factor of it (default 2: the layout stops at `total_seconds` of work; the wall clock only guards against a machine far busier or slower than the work rates say: `wall_guard_seconds`). A load margin, not a budget. |
| `placer` | `KiCadBoardPlacerConfig` | its own table | router.md, placer.md | The settings of the layout's placement (as place-kicad-board's placer config; its constraints relative to the source directory). |
| `pin_swaps` | `Option<PathBuf>` | unset | placer.md | Interchangeable pins (`pin-swaps.json`; relative to the source directory). The nets on them are permuted after placement. |
| `swappable` | `Vec<KiCadSwappable>` | `[]` | - | Interchangeable pins, the short way: per part, the pins (pad functions such as `GPIO*` or pad numbers, globs) whose nets may be permuted. Used when `pin_swaps` is not given. |
| `pin_swap` | `KiCadPinSwapConfig` | its own table | - | The pin-swap estimate's settings, for `pin_swaps` and `swappable`. |
| `congestion_model` | `Option<PathBuf>` | unset | placer.md | The placement race's pre-ranking (`docs/congestion-model.md`): the placer makes `congestion_candidates` placements (seeds), they are ranked by `congestion_score`, and the race probes only the best `congestion_probes`. `congestion_score`: - `rudy` (default): RUDY routing demand above the tiles' free   capacity; no model needed (quick tier: KiCad unconnected 98 -> 64   against the plain race of three seeds); - `none`: the plain race (the placer's `placement_seeds`, all   probed); - `global`: the coarse global route's usage above half the tile   boundaries' capacity (`pcb-global`, docs/global-route.md; an   experiment); - with `congestion_model` (an ONNX file, an experiment): `open`   (its predicted unfinished nets), `overflow` (its predicted   overflow summed over the board), or `open+rudy` (the sum of a   placement's ranks by `open` and by RUDY). |
| `congestion_candidates` | `usize` | `16` | - | With a pre-ranking score, the placer makes at least this many placements to rank (its `placement_seeds` when that is more). |
| `congestion_probes` | `usize` | `3` | - | With a pre-ranking score, how many of the best-ranked placements the race routes (at least one). |
| `congestion_score` | `String` | `"rudy"` | placer.md | How the race ranks the placements before probing them (the values above, at `congestion_model`). |
| `free_dimension` | `Option<crate::free_dimension::KiCadFreeDimension>` | unset | - | One edge of a rectangular outline the layout may move, to find the smallest board that comes out clean (docs/constraints.md, "A free dimension"): `layout-kicad-board` then lays the board out at several sizes, `total_seconds` being the whole search's budget. |
| `silkscreen` | `KiCadSilkscreenConfig` | its own table | placer.md | The footprints' Reference and Value fields on the silkscreen, placed after routing where KiCad's silkscreen checks find nothing. |

## Layout settings: swappable entries

One entry of `swappable`. Source: `KiCadSwappable` in [crates/pcb-kicad/src/board_layout.rs](../crates/pcb-kicad/src/board_layout.rs).

| Key | Type | Default | Docs | Description |
|---|---|---|---|---|
| `swappable[].part` | `String` | required | - | The part's reference (`U1`). |
| `swappable[].pins` | `Vec<String>` | required | - | Pad functions (`GPIO*`, `IO?`) or pad numbers, as globs. |
| `swappable[].except` | `Vec<String>` | optional | - | Pads to leave alone (strapping pins, a UART the bootloader uses). |

## Layout settings: free_dimension

The `free_dimension` object of the layout settings: one edge of a rectangular outline that `layout-kicad-board` moves to find the smallest clean board (docs/constraints.md, "A free dimension"). Source: `KiCadFreeDimension` in [crates/pcb-kicad/src/free_dimension.rs](../crates/pcb-kicad/src/free_dimension.rs).

| Key | Type | Default | Docs | Description |
|---|---|---|---|---|
| `free_dimension.edge` | `String` | required | - | The edge of the rectangular outline that moves: `bottom`, `top`, `left` or `right`. The opposite edge stays where it is. |
| `free_dimension.min` | `Option<f64>` | optional | router.md | The smallest extent of the board along the free axis (its height for `top` and `bottom`, its width for `left` and `right`), in millimetres; with `max` and `step`. |
| `free_dimension.max` | `Option<f64>` | optional | - | The largest extent along the free axis, in millimetres. |
| `free_dimension.step` | `Option<f64>` | optional | - | The sizes tried: `min`, `min + step`, ... up to `max` (and `max` itself). |
| `free_dimension.sizes` | `Vec<f64>` | optional | - | The sizes tried, instead of a range and a step. |
| `free_dimension.prefer_below` | `Vec<f64>` | optional | - | Price thresholds (JLCPCB's 100 mm): a size at or below one is preferred over any larger size, and is tried itself when it lies in the range. |
| `free_dimension.accept` | `Option<String>` | optional | - | What counts as clean: `pass_strict` (default: the layout's verdict is complete, exit 0: KiCad finds no unconnected item and no error that counts, every part placed, every constraint kept) or `pass` (electrically good: KiCad finds no unconnected item and no new error by the count rule, every part placed; missed soft constraints allowed). |
| `free_dimension.band_mm` | `Option<f64>` | optional | - | What moves with the edge: everything within this distance of it (footprints by their origin, held parts also by a body at the edge, board texts and other rigid items by their box, drawn shapes and zones point by point, arcs by their ends, constraint boxes side by side). Default 0.5 mm: what lies on the edge. |
| `free_dimension.probe_seconds` | `Option<f64>` | optional | router.md | The work budget of one probe (default a third of `confirm_seconds`). |
| `free_dimension.confirm_seconds` | `Option<f64>` | optional | - | The work kept for the full runs after the probes (default a third of the layout's `total_seconds`, which is the whole search's budget): probes start only while it stays. The full runs (a smaller size retried, the confirmation, the closest size) get everything left. |
| `free_dimension.parallel` | `Option<usize>` | optional | - | Probes run side by side (default: a quarter of the threads, at most 4, at most one per 3 GB of available memory). Each gets this share of the threads and of the memory (`PCB_MAKER_MEMORY_MB`). |

## Layout settings: silkscreen

The `silkscreen` object of the layout settings: the footprints' Reference and Value fields, placed after routing where KiCad's silkscreen checks find nothing (docs/placer.md, "The fields on the silkscreen"). Source: `KiCadSilkscreenConfig` in [crates/pcb-kicad/src/board_layout.rs](../crates/pcb-kicad/src/board_layout.rs).

| Key | Type | Default | Docs | Description |
|---|---|---|---|---|
| `silkscreen.hide_when_blocked` | `bool` | `false` | placer.md | Hide a field no spot around (or inside) its part is clean for (default false: it stays at the spot with the fewest findings, and the verdict's `silkscreen` fact lists it). |

## Placer core

`pcb_placer::Config` (its defaults are `Config::new()`'s): not a file. The KiCad placer builds it from the placer settings in `place_kicad_board`. Source: `Config` in [crates/pcb-placer/src/lib.rs](../crates/pcb-placer/src/lib.rs).

| Field | Type | Default | Set from (`place_kicad_board`) | Description |
|---|---|---|---|---|
| `global` | `GlobalConfig` | its own table | - | The global (electrostatic) placement's settings. |
| `anneal` | `anneal::AnnealConfig` | its own table | - | The annealer's settings. |
| `refine_passes` | `usize` | `8` | - | Passes of the wirelength-reducing local search on the legal placement (`legal::refine`), also after each relation repair. |
| `maximum_utilization` | `f64` | `0.6` | `maximum_utilization` | Halos shrink until bodies plus margins fit into this share of the free board area. |
| `overlap_far_side_pads` | `bool` | `false` | `overlap_far_side_pads` | Where courtyards may overlap (the tight levels), let a part's body reach over other parts' pads on its side, keeping only copper apart (katia's connectors partly over the switches' hot-swap socket pads, as its designer placed them). Off by default: a socket is a body. |
| `work_limit` | `Option<u64>` | unset | `work_seconds` | Placement work (see `work`) after which no further level or anneal stage starts; the best placement so far stands. |
| `routing_room` | `Option<room::RoutingRoom>` | unset | the KiCad layer | Routing room from congestion (`room`): after placing with the pin-count halos, each part's halo is rescaled by the routing demand around it, within the same halo area, and the placement is annealed again from there. Off (`None`) by default. |
| `orientation_search` | `bool` | `true` | `orientation_search` | The orientation search starts from a canonical pose (`orient`): angle options in a fixed order, every movable part that may turn at the first of them, so that the placement does not depend on the angles the board held (0 degrees from an import, the designer's in a stack). Off: the parts start at the angles they hold, options in the order given (the placer before 2026-10-15). |

## Placer core: global

`pcb_placer::GlobalConfig`, the global (electrostatic) placement. Source: `GlobalConfig` in [crates/pcb-placer/src/global.rs](../crates/pcb-placer/src/global.rs).

| Field | Type | Default | Set from (`place_kicad_board`) | Description |
|---|---|---|---|---|
| `global.bins` | `usize` | `64` | - | Bins per side of the density grid the charge is spread on. |
| `global.whitespace_fill` | `f64` | `0.0` | `whitespace_fill` | Fraction of the whitespace (free area not needed by bodies and their halos) that is occupied by filler charge. 1 lets wirelength pull the parts into a compact cluster; 0 spreads them over the whole board. |
| `global.stop_overflow` | `f64` | `0.04` | - | The density overflow at which the global placement stops (after its first 20 iterations). |
| `global.max_iterations` | `usize` | `1500` | - | Iterations of the global placement at most. |
| `global.lambda_growth` | `f64` | `1.03` | - | Growth of the density weight per iteration. |
| `global.rotation_interval` | `usize` | `15` | - | Iterations between discrete rotation updates. |
| `global.rotation_net_limit` | `usize` | `24` | - | Nets with more pins are ignored when choosing rotations. |
| `global.frame_interval` | `usize` | `5` | - | Every this many iterations the poses are kept as a frame (for animations); 0 keeps none but the last. |
| `global.seed` | `u64` | `1` | `seed` | Seed of the start's noise: the parts start near the fixed parts' pin centroid (or the board's centre), the fillers anywhere. |
| `global.routing_demand` | `f64` | `0.0` | `routing_demand` | Routing demand as charge: every net's wire area (its half perimeter times `track_pitch`, over `routing_layers`) spreads over its bounding box and counts like body area there, so parts leave room where the wires will run. 0 turns it off; 1 counts the wires once. |
| `global.routing_layers` | `f64` | `2.0` | the KiCad layer | Signal layers the routing demand spreads over (at least 1). |
| `global.track_pitch` | `f64` | `0.65` | `track_pitch_mm` | Track pitch of the routing demand's wire area, in mm. |

## Placer core: anneal

`pcb_placer::anneal::AnnealConfig`, the annealer. Source: `AnnealConfig` in [crates/pcb-placer/src/anneal.rs](../crates/pcb-placer/src/anneal.rs).

| Field | Type | Default | Set from (`place_kicad_board`) | Description |
|---|---|---|---|---|
| `anneal.stages` | `usize` | `160` | - | Stages of the annealing schedule: the overlap penalty grows and the move radius shrinks from stage to stage. |
| `anneal.moves_per_part` | `usize` | `40` | - | Moves tried per movable part in each stage. |
| `anneal.initial_radius` | `f64` | `0.06` | - | Initial move radius as a fraction of the board's larger side. |
| `anneal.seed` | `u64` | `7` | - | Seed of the annealer's moves. |
| `anneal.deadline` | `Option<std::time::Instant>` | unset | `deadline` | No further stage starts after this wall-clock time. |
| `anneal.stop_at_work` | `Option<u64>` | unset | - | No further stage starts once this thread's `crate::work()` reaches this count. |
| `anneal.legal_far_side` | `bool` | `false` | `anneal_far_side_gaps` | Parts across the board meet at the legality test's gaps (`State::far_overlap`): holes the full spacing from bodies, far-side copper the copper clearance from pads. Without it holes meet bodies at half the spacing and far-side copper not at all, and the legalizer moves what the annealer could not see (Castor: 33 of 69 conflicted parts). Off: flat on the quick tier, worse on video and OpenESC 4in1 (2026-10-14). |
| `anneal.guidance` | `Option<crate::guidance::Guidance>` | unset | - | The global route inside the anneal (`crate::guidance`, an experiment): off (`None`) by default. |
