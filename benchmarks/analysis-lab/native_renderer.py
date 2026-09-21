#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Analysis views of native KiCad geometry; the native board remains authority."""
import argparse
import json
import math
import subprocess
from pathlib import Path


def xy_value(p):
    return [p['x'],p['y']] if isinstance(p,dict) else p


def bounds_value(bounds):
    if isinstance(bounds,dict):
        return xy_value(bounds['min'])+xy_value(bounds['max'])
    return bounds


def render(board, output, selected_net=None, bounds=None, labels=False):
    from PIL import Image,ImageDraw,ImageFont
    bounds=bounds_value(bounds or board['bounds'])
    x0,y0,x1,y1=bounds
    w,h=1000,850
    image=Image.new('RGB',(w*2,h),'white')
    font_path=subprocess.check_output(['fc-match','-f','%{file}','sans'],text=True)
    font=ImageFont.truetype(font_path,13)
    title=ImageFont.truetype(font_path,20)
    scale=min((w-100)/(x1-x0),(h-150)/(y1-y0))
    layers=[l['name'] for l in board.get('layers',[])] or ['F.Cu','B.Cu']
    if len(layers)!=2:
        raise ValueError('This inspection renderer currently requires exactly two exported copper layers')
    for panel,layer in enumerate(layers):
        d=ImageDraw.Draw(image)
        ox,oy=panel*w+60,70
        def point(p):
            x,y=xy_value(p)
            return (ox+(x-x0)*scale,oy+(y-y0)*scale)
        d.text((panel*w+20,10),f"{board.get('id','native board')} | {layer} | net {selected_net or 'all'}",fill='black',font=title)
        d.text((panel*w+20,38),'Common board coordinates in mm; y increases downward. Outline frame is view extent.',fill='#555555',font=font)
        tick=max(1,5*math.ceil(max(x1-x0,y1-y0)/100))
        for x in range(math.ceil(x0/tick)*tick,math.floor(x1/tick)*tick+1,tick):
            a,b=point([x,y0]),point([x,y1])
            d.line([a,b],fill='#eeeeee')
            d.text((a[0]-8,a[1]-20),str(x),fill='#555555',font=font)
        for y in range(math.ceil(y0/tick)*tick,math.floor(y1/tick)*tick+1,tick):
            a,b=point([x0,y]),point([x1,y])
            d.line([a,b],fill='#eeeeee')
            d.text((a[0]-40,a[1]-7),str(y),fill='#555555',font=font)
        d.rectangle([point([x0,y0]),point([x1,y1])],outline='#555555')
        ink=Image.new('RGBA',image.size,(0,0,0,0))
        d=ImageDraw.Draw(ink)
        def active(obj):
            return selected_net is None or obj.get('net')==selected_net
        for track in board.get('tracks',[]):
            if track['layer']!=layer:
                continue
            color=('#b9362e' if panel==0 else '#315dbc') if active(track) else '#c6c6c6'
            d.line([point(track['start']),point(track['end'])],fill=color,width=max(1,round(track['width']*scale)))
            if labels and active(track):
                a,b=point(track['start']),point(track['end'])
                d.text(((a[0]+b[0])/2+3,(a[1]+b[1])/2+3),track['id'],fill=color,font=font)
        for pad in board.get('pads',[]):
            if layer not in pad['layers'] and '*.Cu' not in pad['layers']:
                continue
            sx,sy=xy_value(pad['size'])
            angle=math.radians(pad.get('rotation_degrees',0))
            center=xy_value(pad['at'])
            # KiCad angles rotate local pad axes opposite the common y-down view.
            def local(x,y):
                return point([center[0]+x*math.cos(angle)+y*math.sin(angle),
                              center[1]-x*math.sin(angle)+y*math.cos(angle)])
            shape=str(pad['shape']).lower()
            if 'circle' in shape:
                points=[local(sx/2*math.cos(t*math.pi/24),sy/2*math.sin(t*math.pi/24)) for t in range(48)]
            elif 'oval' in shape:
                radius=min(sx,sy)/2
                dx,dy=max(0,(sx-sy)/2),max(0,(sy-sx)/2)
                points=[]
                for t in range(64):
                    theta=t*math.pi/32
                    points.append(local(radius*math.cos(theta)+(dx if math.cos(theta)>=0 else -dx),
                                        radius*math.sin(theta)+(dy if math.sin(theta)>=0 else -dy)))
            elif 'roundrect' in shape:
                radius=pad['roundrect_radius']
                points=[]
                for cx,cy,angle0 in [(sx/2-radius,sy/2-radius,0),(-sx/2+radius,sy/2-radius,90),
                                     (-sx/2+radius,-sy/2+radius,180),(sx/2-radius,-sy/2+radius,270)]:
                    for step in range(9):
                        theta=math.radians(angle0+step*90/8)
                        points.append(local(cx+radius*math.cos(theta),cy+radius*math.sin(theta)))
            else:
                points=[local(-sx/2,-sy/2),local(sx/2,-sy/2),local(sx/2,sy/2),local(-sx/2,sy/2)]
            d.polygon(points,fill='#69a380' if active(pad) else '#d4ded6',outline='#46634c')
            drill=xy_value(pad.get('drill',[0,0]))
            if drill[0]>0 and drill[1]>0:
                hole=[local(drill[0]/2*math.cos(t*math.pi/24),drill[1]/2*math.sin(t*math.pi/24)) for t in range(48)]
                d.polygon(hole,fill='white',outline='#666666')
            if labels and active(pad):
                px,py=point(pad['at'])
                d.text((px+5,py+5),pad['id'],fill='#16482b',font=font)
        for via in board.get('vias',[]):
            if layer not in via['layers'] and '*.Cu' not in via['layers']:
                continue
            px,py=point(via['at'])
            r=via['diameter']*scale/2
            d.ellipse((px-r,py-r,px+r,py+r),fill='#f4cc54' if active(via) else '#dddddd',outline='black',width=1)
            r=via['drill']*scale/2
            d.ellipse((px-r,py-r,px+r,py+r),fill='white')
            if active(via):
                d.text((px+5,py-17),via['id'],fill='black',font=font)
        box=(round(ox),round(oy),round(ox+(x1-x0)*scale)+1,round(oy+(y1-y0)*scale)+1)
        clipped=ink.crop(box)
        image.paste(clipped,box,clipped)
        d=ImageDraw.Draw(image)
        d.text((panel*w+20,h-50),'Red/blue: layer copper   Green: pads   Gold: vias   Muted copper: other nets',fill='black',font=font)
        d.text((panel*w+20,h-27),'Geometry inspection only. Native PCB/project and KiCad DRC govern repair acceptance.',fill='black',font=font)
    Path(output).parent.mkdir(parents=True,exist_ok=True)
    image.save(output)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('board')
    parser.add_argument('--output',required=True)
    parser.add_argument('--net')
    parser.add_argument('--bounds',nargs=4,type=float)
    parser.add_argument('--labels',action='store_true')
    args=parser.parse_args()
    render(json.loads(Path(args.board).read_text()),args.output,args.net,args.bounds,args.labels)
    print(args.output)


if __name__=='__main__':
    main()
