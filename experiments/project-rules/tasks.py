#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Writes the route-mode task list of the project-rules measurement
(docs/changelog.md, "project rules at scale"): 20 intake boards from
drc_failure_boards.csv whose only new findings are clearance,
track_width, copper_edge_clearance or hole_to_hole, with a licence
run.py fetches.

    python3 -I experiments/project-rules/tasks.py [--data DIR] > benchmarks/agent-tasks/rules-tasks.json
    flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/rules-x1225 \\
      --tasks benchmarks/agent-tasks/rules-tasks.json --jobs 8 --threads 1 \\
      --binary build/bin/pcb-maker-x1225 > build/rules-x1225.log
(with --harvest-data when the harvest is elsewhere).
"""
import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DATA = ROOT / "benchmarks/expansion/data"

# By the rule behind the intake's new findings (the routed board's DRC):
BOARDS = [
    # clearance: netclass patterns that are regular expressions (HIGH_V)
    "dennisss_dacha_pkg_vision_mocap_boards_compute_r4_board-latest.kicad_pcb__7a70d8e325c4",
    "dennisss_dacha_pkg_vision_mocap_boards_compute_r5_board-latest.kicad_pcb__99c989b4409c",
    # clearance: .kicad_dru rules (on areas and parts; a jumper's pins)
    "wenming-ma_kisurf_qa_data_pcbnew_issue24211_issue24211.kicad_pcb__e0a6f9234f57",
    "ChengminLi_AI-PCB-layout_examples_emc-flyback-demo_design_emc_flyback.kicad_pcb__a49eeca027d3",
    # clearance: net class clearances KiCad finds closer (controls: no rule change)
    "meshsat_meshsat-fieldkit_v2_release_revA_boards_meshsat-pcb-d-revA-D7_pcb-d-aprs.kicad_pcb__d9040b05ec37",
    "Nicholas1023_name-card_src_name-card.kicad_pcb__27ad2e2795a4",
    "CIRCUITSTATE_AQEYE-SB-D1_AQEYE-SB-D1.kicad_pcb__005cd33b69d7",
    # copper_edge_clearance: a .kicad_dru rule, the board setup
    "openenergymonitor_emonWiFi_pcb_emonWiFi.kicad_pcb__1270854846eb",
    "michpro_PY32F002_ESSOP10_minimal_board_PY32F002_ESSOP10_minimal_board.kicad_pcb__8347392ecf4b",
    "ThunderFly-aerospace_TFRPM01_hw_sch_pcb_TFRPM01.kicad_pcb__89a89260c944",
    "buganini_Kikakuka_samples_gerber_gerber.kicad_pcb__b9afe2eb0096",
    "siderakb_facade_main_board_2u_shift_facade60_main.kicad_pcb__cdaad4916f59",
    # hole_to_hole: the board setup
    "Squantor_templates_kicad_nuclone_devboard_small_4_layer_nuclone_devboard_small_4_layer.kicad_pcb__d9b9bd17a116",
    # track_width: KiCad's default minimum (no min_track_width in the project)
    "baxter-barlow_kicad-assistant_qa_data_pcbnew_pns_regressions_boards_pic_programmer.kicad_pcb__2289eafe2e8f",
    "kanengomibako_Modules_10VBlocker_10VBlocker_RevB.kicad_pcb__ca8d7824e84f",
    "Boardoza_Boardoza_HP303B_Barometric_Pressure_Sensor_Breakout_Board_hardware_files_B-HP303B-BRK-R1.0.__6b0e7a637b06",
    "brunostjohn_flatmaxxing_testdir_valid_board.kicad_pcb__88861994eb60",
    "KiCad_kicad-source-mirror_qa_data_pcbnew_issue25101.kicad_pcb__ef1a4cc3de60",
    "Alan-Jowett_sonde_hw_carrier-board_output_sonde-carrier.kicad_pcb__438a19534cf8",
    "TheusHen_EEGFrontier_PCB_EEGFrontier.kicad_pcb__4c9ee2dbd4b6",
]


def task(record):
    board_id = record["board_id"]
    files = {f"{board_id}.kicad_pcb": record["sha256"]}
    if record.get("project_sha256"):
        files[f"{board_id}.kicad_pro"] = record["project_sha256"]
    if record.get("custom_rules_sha256"):
        files[f"{board_id}.kicad_dru"] = record["custom_rules_sha256"]
    name = record["name"].split(".kicad_pcb")[0][:80]
    return {
        "name": name,
        "mode": "route",
        "directory": f"build/rules-tasks/{name}/source",
        "board_id": board_id,
        "designer_budget": True,
        "budget_seconds": 30,
        "router": {"seeds": 1, "skip_native_verification": True},
        "source": {
            "repository": record["repository"],
            "commit": record["commit"],
            "path": record["path"],
            "url": record["source_url"],
            "license": record["license_spdx"],
            "harvest_name": record["name"],
            "files": files,
        },
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, default=DATA, help="the harvest's data directory")
    data = parser.parse_args().data
    records = {record["name"]: record for record in json.loads((data / "manifest.json").read_text())["boards"]}
    tasks = [task(records[name]) for name in BOARDS]
    print(json.dumps({
        "comment": "Route-mode tasks of the project-rules measurement (experiments/project-rules/tasks.py): "
                   "the intake's settings, the designer's placement, 30 s of work.",
        "tasks": tasks,
    }, indent=1))


if __name__ == "__main__":
    main()
