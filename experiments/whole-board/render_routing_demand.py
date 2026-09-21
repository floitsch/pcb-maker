# Copyright (C) 2026 Toit contributors.
"""Render the advisory trace-demand field stored in a native route candidate.

This plots forecast cost before physical obstacle masking; it is not a map of
legal routing space. Both layers use the same color scale and board coordinates.
"""
import argparse
import base64
from io import BytesIO
import json
from pathlib import Path
import xml.etree.ElementTree as ET
import numpy as np
from PIL import Image


def render(candidate_path, background, output_prefix):
    candidate = json.loads(candidate_path.read_text())
    config = candidate['config']
    demand = config.get('routing_demand')
    if not demand:
        return
    width, height = candidate['grid_size']
    origin = candidate['grid_origin']
    step = config['resolution_mm']
    yy, xx = np.mgrid[0:height, 0:width]
    x = origin[0] + xx*step
    y = origin[1] + yy*step
    field = np.zeros((2, height, width))
    for alternative in demand['alternatives']:
        if alternative['connection'].removeprefix('/') == candidate['connection'].removeprefix('/'):
            continue
        one = np.zeros_like(field)
        for path in alternative['paths']:
            for a, b in zip(path, path[1:]):
                ax, ay = a['at']; bx, by = b['at']
                changing = a['layer'] != b['layer']
                radius = alternative['via_size_mm'] / 2 if changing else alternative['trace_width_mm'] / 2
                core = radius + config['trace_width_mm']/2 + max(config['clearance_mm'], alternative['clearance_mm'])
                squared = (bx-ax)**2 + (by-ay)**2
                t = np.clip(((x-ax)*(bx-ax)+(y-ay)*(by-ay))/squared, 0, 1) if squared else 0
                distance = np.hypot(x-(ax+t*(bx-ax)), y-(ay+t*(by-ay)))
                value = np.maximum(0, 1-np.maximum(0, distance-core)/demand['shoulder_mm'])
                layers = [0, 1] if changing else [0 if a['layer']=='F.Cu' else 1]
                for layer in layers:
                    np.maximum(one[layer], value, out=one[layer])
        field += one * alternative['weight'] * demand['strength']
    NS = 'http://www.w3.org/2000/svg'
    tag = lambda name: '{'+NS+'}'+name
    for layer, side in enumerate(['front', 'back']):
        svg = ET.parse(background).getroot()
        rgba = np.zeros((height, width, 4), dtype=np.uint8)
        rgba[:, :, :3] = [232, 87, 26]
        rgba[:, :, 3] = (np.clip(field[layer]/3.0, 0, 1)*220).astype(np.uint8)
        stream = BytesIO()
        Image.fromarray(rgba).save(stream, format='PNG')
        overlay = ET.Element(tag('image'), {
            'x': str(origin[0]-step/2), 'y': str(origin[1]-step/2),
            'width': str(width*step), 'height': str(height*step),
            'preserveAspectRatio': 'none', 'style': 'image-rendering:pixelated',
            'href': 'data:image/png;base64,'+base64.b64encode(stream.getvalue()).decode(),
        })
        svg.insert(0, overlay)
        destination = Path(str(output_prefix)+f'-{side}.svg')
        ET.ElementTree(svg).write(destination, encoding='unicode')
        ET.parse(destination)
    Path(str(output_prefix)+'.json').write_text(json.dumps(dict(
        candidate=str(candidate_path), connection=candidate['connection'], peak_demand=field.max(axis=(1, 2)).tolist(),
        color_scale='Orange alpha increases linearly to demand=3; same scale on both layers.',
        scope='Advisory trace cost before physical obstacle masking. Excludes own net. Via footprint cost is separate.'), indent=2)+'\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('background', type=Path)
    parser.add_argument('output_prefix', type=Path)
    args = parser.parse_args()
    render(args.candidate, args.background, args.output_prefix)
