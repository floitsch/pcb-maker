# Copyright (C) 2026 Toit contributors.
"""Custom-pad translation and shape-change controls; no routed-layout claim."""
import argparse
from decimal import Decimal
import json
from pathlib import Path
import re
import shutil
import sys

from run import command, digest, read, write


def native(operation, board, output):
    from area_probe import load_pcbnew
    from audit_native_placement import inventory
    k=load_pcbnew();b=k.LoadBoard(str(board))
    if operation=='snapshot':
        write(output,inventory(b))
    else:
        assert operation=='translate'
        fp=next(f for f in b.GetFootprints() if f.GetReference()=='JP1')
        p=fp.GetPosition();fp.SetPosition(k.VECTOR2I(p.x+1000000,p.y-1000000))
        k.SaveBoard(str(board),b)


def controls(source, binary, output):
    output.mkdir(exist_ok=False)
    shutil.copy2(__file__,output/Path(__file__).name)
    name=source.name;stem=source.stem;sha=digest(source)
    base=output/'source';shutil.copytree(source.parent,base)
    assert command([binary,'strip-kicad-copper',base/name,base/name,output/'strip.json'],output/'strip.log')['exit_code']==0
    def snapshot(directory):
        result=command([sys.executable,__file__,'--native','snapshot',directory/name,directory/'inventory.json'],directory/'inventory.log')
        assert result['exit_code']==0
        rendered=command(['kicad-cli','pcb','export','svg','--mode-single',
            '--layers','F.Cu,B.Cu,Edge.Cuts','--page-size-mode','2','--exclude-drawing-sheet',
            '--output',directory/'inspection-combined.svg',directory/name],directory/'render.log')
        assert rendered['exit_code']==0
        return read(directory/'inventory.json')
    before=snapshot(base)
    moved=output/'translated';shutil.copytree(base,moved)
    assert command([sys.executable,__file__,'--native','translate',moved/name,moved/'unused'],output/'translate.log')['exit_code']==0
    shutil.copy2(source.with_suffix('.kicad_pro'),(moved/name).with_suffix('.kicad_pro'))
    after=snapshot(moved)
    assert before==after, 'Translation changed native pad-local geometry'
    changed=output/'changed-shape';shutil.copytree(moved,changed)
    text=(changed/name).read_text()
    pattern=r'(\(primitives\s+\(gr_poly\s+\(pts\s+\(xy\s+)([-+0-9.eE]+)'
    rewritten,count=re.subn(pattern,lambda m:m[1]+str(Decimal(m[2])+Decimal('.1')),text,count=1)
    assert count==1
    (changed/name).write_text(rewritten)
    corrupt=snapshot(changed)
    assert corrupt!=after
    assert {ref for ref in after if after[ref]!=corrupt[ref]}=={'JP1'}
    def without_custom(value):
        value=json.loads(json.dumps(value))
        for fp in value.values():
            for pad in fp['pads']:pad.pop('custom_primitives',None)
        return value
    assert without_custom(corrupt)==without_custom(after), 'Control changed nominal geometry fields'
    assert digest(source)==sha
    write(output/'report.json',dict(status='passed',source_sha256=sha,
        unchanged_translation_accepted=True,custom_shape_change_detected=True,
        nominal_pad_fields_identical_in_negative_control=True,source_unchanged=True,
        scope=__doc__))
    (output/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Custom-pad inventory controls</title>'
        '<style>body{font:17px system-ui;margin:2rem}img{max-width:100%}</style>'
        '<h1>Translation preserves custom-pad shape; shape changes are detected</h1>'
        '<p>These cold-board controls exercise the geometry inventory only. No routing or layout admission is claimed.</p>'
        +''.join(f'<h2>{case}</h2><img src="{case}/inspection-combined.svg">' for case in ['source','translated','changed-shape'])
        +'<p><a href="report.json">Control assertions</a></p>')


if __name__=='__main__':
    if len(sys.argv)==5 and sys.argv[1]=='--native':
        native(sys.argv[2],Path(sys.argv[3]),Path(sys.argv[4]))
    else:
        parser=argparse.ArgumentParser(description=__doc__)
        for name in ['source','binary','output']:parser.add_argument(name,type=Path)
        args=parser.parse_args();controls(args.source.resolve(),args.binary.resolve(),args.output.resolve())
