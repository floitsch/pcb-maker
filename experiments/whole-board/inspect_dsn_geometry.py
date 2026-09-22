# Copyright (C) 2026 Toit contributors.
"""Inspect the pinned external router's cold input and mark its violations.

This is a read-only geometry diagnostic, not a native design review or a proof
of full DSN equivalence. Coordinates are overlaid on an existing native combined
preview; the source board is never changed.
"""
import argparse
import html
import json
from pathlib import Path
import shutil
import subprocess
import xml.etree.ElementTree as ET

from compare_native_router import JAR_SHA256
from render_inspection_layers import render, tag
from run import command, digest, read, write


def inspect(dsn, native_directory, board_id, output, jar, java_bin, outside_outline=False):
    output.mkdir(exist_ok=False)
    assert digest(jar) == JAR_SHA256
    helper = Path(__file__).with_name('InspectDsn.java')
    shutil.copy2(helper,output/helper.name)
    shutil.copy2(__file__,output/Path(__file__).name)
    mode_helper = Path(__file__).with_name("NativeOutlineMode.java")
    shutil.copy2(mode_helper, output/mode_helper.name)
    helper=output/helper.name
    processes = []
    for name, argv in [
        ('compile',[java_bin/'javac','-cp',jar,'-d',output,helper,output/mode_helper.name]),
        ('inspect',[java_bin/'java','-Djava.awt.headless=true','-cp',f'{jar}:{output}',
                    'InspectDsn',dsn,output/'raw.json',str(outside_outline).lower()])]:
        result = command(argv,output/(name+'.log'))
        processes.append(dict(stage=name,**result))
        assert result['exit_code']==0
    raw = read(output/'raw.json')
    assert raw['load_result']=='OK' and raw['unit']=='um' and raw['resolution']==10
    # Freerouting emits the same item pair from both ends. Keep layer-specific
    # findings and spatial regions, but collapse mirrored entries before assigning labels.
    unique = {}
    for row in raw['violations']:
        key = (*sorted([row['first']['id'],row['second']['id']]),row['layer'],tuple(row['dsn_bounds']))
        if key in unique:
            assert all(row[k]==unique[key][k] for k in ['dsn_bounds','expected_clearance_dsn','actual_clearance_dsn'])
        else:
            unique[key] = row
    violations = list(unique.values())
    project = native_directory/(board_id+'.kicad_pro')
    board = native_directory/(board_id+'.kicad_pcb')
    board_hash = digest(board)
    native_edge = (read(project).get('board',{}).get('design_settings',{}).get('rules',{}).get('min_copper_edge_clearance',0.5))
    render(native_directory,board_id)
    svg = ET.parse(native_directory/'inspection-combined.svg').getroot()
    bx,by,bw,bh = map(float,svg.attrib['viewBox'].split())
    svg.set('viewBox',f'{bx-2} {by-2} {bw+4} {bh+4}')
    overlay = ET.SubElement(svg,tag('g'),{'id':'external-clearance-findings'})
    for ordinal, row in enumerate(violations):
        row['label'] = f'E{ordinal+1}'
        x0,y0,x1,y1 = row['dsn_bounds']
        x,y,w,h = x0/1000,-y1/1000,(x1-x0)/1000,(y1-y0)/1000
        row['native_bounds_mm'] = [x,y,w,h]
        item_names = [f'{v.get("component",v["kind"])}.{v.get("pin","")}' for v in [row['first'],row['second']]]
        description = f'{row["label"]}: {" / ".join(item_names)}; layer {row["layer"]}; expected {row["expected_clearance_dsn"]/1000:g} mm, actual {row["actual_clearance_dsn"]/1000:g} mm'
        row['description'] = description
        mark = ET.SubElement(overlay,tag('rect'),{'x':str(x-.2),'y':str(y-.2),
            'width':str(w+.4),'height':str(h+.4),'fill':'#f59e0b','fill-opacity':'.3','stroke':'#c2410c','stroke-width':'.15'})
        ET.SubElement(mark,tag('title')).text=description
        # The layer-specific labels share a hotspot but remain separately readable.
        label = ET.SubElement(overlay,tag('text'),{'x':str(x+.4),'y':str(y+row['layer']*1.1),
            'font-family':'sans-serif','font-size':'.9','fill':'#9a3412','stroke':'white',
            'stroke-width':'.1','paint-order':'stroke fill'})
        label.text = row['label']
    view = output/'inspection.svg'
    ET.ElementTree(svg).write(view,encoding='unicode')
    subprocess.run(['rsvg-convert','--background-color','white','--width','1400','--output',str(view.with_suffix('.png')),str(view)],check=True)
    assert digest(board)==board_hash
    edge = [v for v in violations if 'BoardOutline' in [v['first']['kind'],v['second']['kind']]]
    # Inspect the loaded rule matrix even on a board with no current findings.
    # Line mode adds outline half-width; outside-area mode uses the exact
    # boundary of the exterior polygon and adds only matrix clearance.
    effective_edges = []
    for border in raw['board_edges']:
        for cls in border['classes']:
            effective_edges.append(dict(clearance_class=cls['name'],
                effective_edge_per_layer_mm=[(v+(0 if border['outside_area'] else border['half_width_dsn']))/1000 for v in cls['clearance_per_layer_dsn']]))
    report = dict(scope=__doc__,dsn_sha256=digest(dsn),source_board_sha256=board_hash,
        source_project_sha256=digest(project),jar_sha256=digest(jar),helper_sha256=digest(helper),
        processes=processes,native_edge_clearance_mm=native_edge,outside_outline=outside_outline,
        routing_layers=raw['routing_layers'],
        signal_layer_count=sum(layer['is_signal'] for layer in raw['routing_layers']),
        raw_violation_count=len(raw['violations']),unique_violation_count=len(violations),
        edge_violation_count=len(edge),violations=violations,
        external_edge_rules=raw['board_edges'],effective_edge_rules=effective_edges,
        edge_requirement_mismatch_observed=any(abs(v-native_edge)>1e-9 for c in effective_edges for v in c['effective_edge_per_layer_mm']),
        source_unchanged=True)
    write(output/'report.json',report)
    rows=''.join('<li>'+html.escape(v['description'])+'</li>' for v in violations)
    (output/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Exchange geometry inspection</title>'
        '<style>body{font:17px system-ui;margin:2rem}img{max-width:95vw}</style><h1>Exchange geometry inspection</h1>'
        f'<p>{len(violations)} unique layer-specific findings. Native edge requirement: {native_edge:g} mm. '
        f'Observed edge-rule mismatch: {report["edge_requirement_mismatch_observed"]}.</p>'
        f'<p>Loaded signal routing layers: {report["signal_layer_count"]} of {len(report["routing_layers"])} copper layers.</p>'
        '<p>These are external-router findings on its cold input. They are not native KiCad violations. '
        'This diagnostic does not establish complete exchange equivalence or placement infeasibility.</p>'
        '<img src="inspection.svg"><ol>'+rows+'</ol><a href="report.json">Evidence</a>')
    return report


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('dsn',type=Path)
    parser.add_argument('native_directory',type=Path)
    parser.add_argument('board_id')
    parser.add_argument('output',type=Path)
    parser.add_argument('--jar',type=Path,default=Path('/tmp/freerouting-2.2.4.jar'))
    parser.add_argument('--java-bin',type=Path,default=Path('/usr/lib/jvm/java-26-openjdk/bin'))
    parser.add_argument('--outside-outline',action='store_true')
    a=parser.parse_args()
    r=inspect(a.dsn.resolve(),a.native_directory.resolve(),a.board_id,a.output.resolve(),a.jar.resolve(),a.java_bin.resolve(),a.outside_outline)
    print(json.dumps({k:r[k] for k in ['unique_violation_count','edge_violation_count','edge_requirement_mismatch_observed']}))
