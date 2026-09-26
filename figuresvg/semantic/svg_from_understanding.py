"""SVG Generator from Scene Understanding: draws from the VLM's
interpretation, not from pixel tracing.

The VLM says "8 petals in radial arrangement" → we emit 8 rotated
ellipses. The VLM says "face: oval + 2 eyes + mouth" → we emit
ellipse + circles + path. The CV layer provides exact measurements
that refine the VLM's approximate positions.
"""

import math
from xml.sax.saxutils import escape


def _mm(px, dpi=300.0):
    return f"{px / dpi * 25.4:.4f}mm"


def generate(understanding: dict, W: int, H: int) -> str:
    """Scene understanding → structured SVG."""
    out = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        f'<svg width="{_mm(W)}" height="{_mm(H)}" viewBox="0 0 {W} {H}" '
        f'xmlns="http://www.w3.org/2000/svg">',
        f'<rect id="background" x="0" y="0" width="{W}" height="{H}" '
        f'fill="#ffffff"/>',
        '<g id="objects">',
    ]

    # handle arrangements (e.g., radial petals) by expanding them
    components = _expand_arrangements(understanding)

    for comp in components:
        el = _draw_component(comp)
        if el:
            out.append(el)

    out.append("</g>")
    out.append("</svg>")
    return "\n".join(out)


def _expand_arrangements(understanding: dict) -> list:
    """Expand arrangement patterns into concrete components."""
    comps = list(understanding.get("components", []))
    for arr in understanding.get("arrangements", []):
        if arr.get("type") == "radial":
            # find template components with matching roles
            center = arr.get("center", [0, 0])
            count = arr.get("count", 0)
            interval = arr.get("interval_degrees", 360 / max(1, count))
            members = [c for c in comps
                       if c.get("role") in arr.get("member_roles", [])
                       and c.get("arrangement")]
            if members and count > 0:
                template = members[0]
                # remove originals that are part of this arrangement
                comps = [c for c in comps if c not in members]
                # generate rotated copies
                for i in range(count):
                    angle = i * interval
                    copy = dict(template)
                    copy["rotation"] = angle
                    copy["id"] = f"{template['id']}_r{i}"
                    # rotate position around center
                    if "position" in copy:
                        px, py = copy["position"]
                        rad = math.radians(angle)
                        # keep same distance from center, rotate
                        dx, dy = px - center[0], py - center[1]
                        copy["position"] = [
                            center[0] + dx * math.cos(rad) - dy * math.sin(rad),
                            center[1] + dx * math.sin(rad) + dy * math.cos(rad),
                        ]
                    comps.append(copy)
    return comps


def _draw_component(comp: dict) -> str:
    """One component → one SVG element."""
    cid = comp.get("id", "shape")
    fill = comp.get("fill", "#333333")
    shape = comp.get("shape", "")
    pos = comp.get("position", [0, 0])
    size = comp.get("size", [10, 10])
    rot = comp.get("rotation", 0)

    if shape == "ellipse":
        cx, cy = pos
        w, h = size if isinstance(size, list) else (size, size)
        rx, ry = w / 2, h / 2
        tr = f' transform="rotate({rot} {cx} {cy})"' if rot else ""
        return f'<ellipse id="{cid}" cx="{cx}" cy="{cy}" rx="{rx}" ry="{ry}" fill="{fill}"{tr}/>'

    if shape == "circle":
        cx, cy = pos
        r = size.get("r", 10) if isinstance(size, dict) else \
            (size[0] / 2 if isinstance(size, list) else size / 2)
        return f'<circle id="{cid}" cx="{cx}" cy="{cy}" r="{r}" fill="{fill}"/>'

    if shape == "rect":
        cx, cy = pos
        if isinstance(size, list) and len(size) == 2:
            w, h = size
        elif isinstance(size, list) and len(size) == 4:
            x1, y1, x2, y2 = size
            w, h = x2 - x1, y2 - y1
            cx, cy = (x1 + x2) / 2, (y1 + y2) / 2
        else:
            w = h = size if isinstance(size, (int, float)) else 10
        x, y = cx - w / 2, cy - h / 2
        tr = f' transform="rotate({rot} {cx} {cy})"' if rot else ""
        return f'<rect id="{cid}" x="{x}" y="{y}" width="{w}" height="{h}" fill="{fill}"{tr}/>'

    if shape == "line":
        if isinstance(size, list) and len(size) == 4:
            x1, y1, x2, y2 = size
        elif isinstance(size, list) and len(size) == 2:
            w, h = size
            x1, y1, x2, y2 = pos[0], pos[1], pos[0] + w, pos[1] + h
        else:
            x1, y1, x2, y2 = 0, 0, 10, 10
        return f'<line id="{cid}" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{fill}" stroke-width="3"/>'

    if shape == "text":
        text = comp.get("text", "")
        fs = comp.get("font_size", 20)
        x, y = pos
        ff = comp.get("font_family", "sans-serif")
        return (f'<text id="{cid}" x="{x}" y="{y}" font-size="{fs}" '
                f'font-family="{ff}" fill="{fill}">'
                f'<tspan>{escape(text)}</tspan></text>')

    if shape == "polygon":
        pts = comp.get("points", [])
        if not pts:
            return ""
        pts_str = " ".join(f"{p[0]},{p[1]}" for p in pts)
        return f'<polygon id="{cid}" points="{pts_str}" fill="{fill}"/>'

    if shape == "path":
        d = comp.get("d", "")
        if d:
            return f'<path id="{cid}" d="{d}" fill="{fill}"/>'

    return ""
