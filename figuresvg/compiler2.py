"""SVG compiler v2: Scene Graph @2 -> Origin/Inkscape-style structured SVG.

Target format (per Relative_gh31.svg + path3.svg):
  - root: viewBox + physical width/height in mm (300 DPI default)
  - <defs> with linearGradients for ramp objects
  - <g id="layer1_objects"> shapes, <g id="layer2_texts"> texts,
    semantic groups (legend_N / errbar_N / ticks_N)
  - one element per object, style attribute, crispEdges on straight shapes
  - texts: tspan, font stack, rotate transform, text-anchor
"""

from xml.sax.saxutils import escape

DPI = 300.0


def _mm(px: float) -> str:
    return f"{px / DPI * 25.4:.4f}mm"


def _style(e, fill_override=None) -> str:
    parts = []
    fill = fill_override or e.get("fill")
    if fill:
        parts.append(f"fill:{fill}")
    if e.get("stroke"):
        parts.append(f"stroke:{e['stroke']}")
        if e.get("stroke_width"):
            parts.append(f"stroke-width:{e['stroke_width']}")
    if e.get("opacity"):
        parts.append(f"fill-opacity:{e['opacity']}")
    return ";".join(parts)


def _shape_el(e) -> str:
    eid = e["id"]
    st = _style(e)
    crisp = ' shape-rendering="crispEdges"' if e["type"] in ("rect", "polygon") else ""
    t = e["type"]
    if t == "rect":
        return (f'<rect id="{eid}" x="{e["x"]}" y="{e["y"]}" '
                f'width="{e["w"]}" height="{e["h"]}" style="{st}"{crisp}/>')
    if t == "circle":
        return (f'<circle id="{eid}" cx="{e["cx"]}" cy="{e["cy"]}" '
                f'r="{e["r"]}" style="{st}"/>')
    if t == "line":
        return (f'<line id="{eid}" x1="{e["x1"]}" y1="{e["y1"]}" '
                f'x2="{e["x2"]}" y2="{e["y2"]}" style="{st}"/>')
    if t == "polygon":
        return f'<polygon id="{eid}" points="{e["points"]}" style="{st}"{crisp}/>'
    if t == "polyline":
        return (f'<polyline id="{eid}" points="{e["points"]}" '
                f'style="{st};fill:none"/>')
    if t == "path":
        tr = f' transform="{e["transform"]}"' if e.get("transform") else ""
        return f'<path id="{eid}" d="{e["d"]}" style="{st}"{tr}/>'
    # unknown fallback
    b = e["bbox"]
    return (f'<rect id="{eid}" x="{b[0]}" y="{b[1]}" width="{b[2]}" '
            f'height="{b[3]}" style="{st}"/>')


def _text_el(e) -> str:
    # rotation: elements may carry "rotate" (degrees) + rotate center
    tr = ""
    if e.get("rotate"):
        cx = e["bbox"][0] + e["bbox"][2] / 2
        cy = e["bbox"][1] + e["bbox"][3] / 2
        tr = f' transform="rotate({e["rotate"]},{cx:.1f},{cy:.1f})"'
    style = (
        f"font-size:{e.get('font_size', 12)}px;"
        f"font-family:{e.get('font_family') or 'sans-serif'};"
    )
    if e.get("font_weight") and e["font_weight"] != "normal":
        style += f"font-weight:{e['font_weight']};"
    if e.get("fill"):
        style += f"fill:{e['fill']};"
    anchor = f' text-anchor="{e["text_anchor"]}"' if e.get("text_anchor") else ""
    x = e["bbox"][0] + (e.get("dx") or 0)
    y = int(e["bbox"][1] + e["bbox"][3] * 0.78) + (e.get("baseline_dy") or 0) + (e.get("dy") or 0)
    return (f'<text id="{e["id"]}" x="{x}" y="{y}"{anchor}{tr} '
            f'style="{style}"><tspan>{escape(e.get("text") or "")}</tspan></text>')


def generate(scene_json: dict, texts: list = None) -> str:
    """scene_json: engine scene@2 dict; texts: OCR text element dicts."""
    w = scene_json["canvas"]["width"]
    h = scene_json["canvas"]["height"]
    out = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        f'<svg width="{_mm(w)}" height="{_mm(h)}" '
        f'viewBox="0 0 {w} {h}" xmlns="http://www.w3.org/2000/svg">',
    ]
    defs = scene_json.get("defs", [])
    if defs:
        out.append("<defs>")
        for g in defs:
            gid = g["id"]
            (x1, x2, y1, y2) = ("0%","100%","0%","0%") if g.get("axis") == "x" else ("0%","0%","0%","100%")
            out.append(
                f'<linearGradient id="{gid}" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}">'
                f'<stop offset="0" stop-color="{g["from"]}"/>'
                f'<stop offset="1" stop-color="{g["to"]}"/></linearGradient>')
        out.append("</defs>")
    out.append(f'<rect id="background" x="0" y="0" width="{w}" '
               f'height="{h}" fill="#ffffff"/>')
    # objects: gradient refs replace fill
    els = scene_json["elements"]
    out.append('<g id="layer1_objects">')
    grouped = set()
    for e in els:
        for tag in (e.get("tags") or []):
            if tag.startswith("errbar_"):
                grouped.add(e["id"])
    for e in sorted(els, key=lambda e: -(e["bbox"][2] * e["bbox"][3])):
        if e["id"] in grouped:
            continue
        if e.get("gradient"):
            e = dict(e, fill=f'url(#{e["gradient"]["id"]})')
        out.append(_shape_el(e))
    out.append("</g>")
    # error-bar groups: stems+caps under one editable node
    from collections import defaultdict

    ebg = defaultdict(list)
    for e in els:
        for tag in (e.get("tags") or []):
            if tag.startswith("errbar_"):
                ebg[tag].append(e)
    for gid, es in ebg.items():
        out.append(f'<g id="{gid}">')
        out.extend(_shape_el(e) for e in sorted(es, key=lambda x: x["bbox"][1]))
        out.append("</g>")
    if texts:
        out.append('<g id="layer2_texts">')
        # semantic groups (legend labels etc.) first
        plain = [t for t in texts if not any(x.startswith("legend") for x in t.get("tags", []))]
        for t in plain:
            out.append(_text_el(t))
        out.append("</g>")
        from collections import defaultdict

        lg = defaultdict(list)
        for t in texts:
            gid = next((x for x in t.get("tags", []) if x.startswith("legend")), None)
            if gid:
                lg[gid].append(t)
        for gid, ts in lg.items():
            out.append(f'<g id="{gid}">')
            out.extend(_text_el(t) for t in ts)
            out.append("</g>")
    out.append("</svg>")
    return "\n".join(out)
