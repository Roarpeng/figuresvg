"""SVG generator (pure code): Scene Graph -> editable SVG.

Text is REAL <text>; geometry keeps its primitive types; everything gets a
stable id inside semantic groups so editors can select/move/restyle whole
objects.
"""

from xml.sax.saxutils import escape

from .scene import Scene

HEADER = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" '
    'viewBox="0 0 {w} {h}">\n'
)


def _attrs(e) -> str:
    a = ['id="%s"' % e.id]
    if e.fill:
        a.append('fill="%s"' % e.fill)
    if e.stroke:
        a.append('stroke="%s"' % e.stroke)
        if e.stroke_width:
            a.append('stroke-width="%s"' % e.stroke_width)
    return " ".join(a)


def _el(e) -> str:
    base = _attrs(e)
    t = e.type
    if t == "text":
        x = e.bbox[0] + (e.dx or 0)
        dy = e.baseline_dy or 0
        y = int(e.bbox[1] + e.bbox[3] * 0.78) + dy + (e.dy or 0)
        weight = (' font-weight="%s"' % e.font_weight) if e.font_weight else ""
        return (
            '<text %s x="%s" y="%s" font-size="%s" font-family="%s"%s>%s</text>'
            % (base, x, y, e.font_size, e.font_family or "sans-serif",
               weight, escape(e.text or ""))
        )
    if t == "rect":
        return '<rect %s x="%s" y="%s" width="%s" height="%s"/>' % (
            base,
            e.x if e.x is not None else e.bbox[0],
            e.y if e.y is not None else e.bbox[1],
            e.w if e.w is not None else e.bbox[2],
            e.h if e.h is not None else e.bbox[3],
        )
    if t == "circle":
        return '<circle %s cx="%s" cy="%s" r="%s"/>' % (base, e.cx, e.cy, e.r)
    if t == "line":
        return '<line %s x1="%s" y1="%s" x2="%s" y2="%s"/>' % (
            base, e.x1, e.y1, e.x2, e.y2)
    if t == "polygon":
        return '<polygon %s points="%s"/>' % (base, e.points)
    if t == "polyline":
        return '<polyline %s points="%s" fill="none"/>' % (base, e.points)
    if t == "path":
        extra = ' transform="%s"' % e.transform if e.transform else ""
        return '<path %s%s d="%s"/>' % (base, extra, e.d)
    # unknown: bbox rect fallback
    return '<rect %s x="%s" y="%s" width="%s" height="%s"/>' % (
        base, e.bbox[0], e.bbox[1], e.bbox[2], e.bbox[3])


def generate(scene: Scene) -> str:
    parts = [HEADER.format(w=scene.width, h=scene.height)]
    parts.append(
        '<rect id="background" width="%s" height="%s" fill="%s"/>\n'
        % (scene.width, scene.height, scene.background)
    )
    # shapes largest-first (painting order), texts on top in one group
    shapes = sorted(
        [e for e in scene.elements if e.type != "text"],
        key=lambda e: -(e.bbox[2] * e.bbox[3]),
    )
    texts = [e for e in scene.elements if e.type == "text"]
    parts.append('<g id="shapes">\n')
    parts.extend(_el(e) + "\n" for e in shapes)
    parts.append("</g>\n")
    parts.append('<g id="texts">\n')
    plain = [e for e in texts if not any("legend-label" in t for t in e.tags)]
    for e in plain:
        parts.append(_el(e) + "\n")
    parts.append("</g>\n")
    # legend groups: swatch shape + label text under one editable node
    from collections import defaultdict

    by_group = defaultdict(lambda: {"sw": [], "tx": []})
    for e in scene.elements:
        if any(t.startswith("legend-") for t in e.tags):
            gid = next((t for t in e.tags if t.startswith("legend_")), None)
            if gid:
                (by_group[gid]["tx"] if e.type == "text"
                 else by_group[gid]["sw"]).append(e)
    for gid, g in by_group.items():
        parts.append(f'<g id="{gid}">\n')
        for e in sorted(g["sw"], key=lambda x: x.bbox[0]):
            parts.append(_el(e) + "\n")
        for e in sorted(g["tx"], key=lambda x: x.bbox[0]):
            parts.append(_el(e) + "\n")
        parts.append("</g>\n")
    if scene.relations:
        parts.append("<!-- relations\n")
        for r in scene.relations:
            parts.append("     %s [%s] %s\n" % (r.frm, r.type, r.to))
        parts.append("-->\n")
    parts.append("</svg>\n")
    return "".join(parts)
