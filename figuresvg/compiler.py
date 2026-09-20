"""SVG compiler v3: pixel-engine backbone + verified editable-text surgery.

The pixel engine's output IS the fidelity backbone (mean diff ~1). For each
OCR text we attempt a surgical swap: remove the traced glyph paths inside
the text's box from the backbone and insert a real <text>. The swap is
accepted only if the box's render deviation from the source stays under
TEXT_ACCEPT_DIFF -- the bitmap-comparison gate. Rejected texts keep their
traced glyphs (with data-ocr metadata) and the figure stays 1:1.

Scene primitives (rect/circle/line/...) are NOT swapped into the backbone:
they ship in the Scene Graph JSON for tooling, because replacing traced
regions with simplified primitives measurably degrades fidelity (the
c.jpeg regression). The semantic layer stays data, not pixels.
"""

import io
import re

import cairosvg
import numpy as np
from PIL import Image

TEXT_ACCEPT_DIFF = 12.0
PAD = 6


def _render(svg: str, w: int, h: int) -> np.ndarray:
    png = cairosvg.svg2png(bytestring=svg.encode(), output_width=w,
                           output_height=h)
    return np.asarray(Image.open(io.BytesIO(png)).convert("RGB")).astype(np.int32)


def _box_diff(a: np.ndarray, b: np.ndarray, bbox) -> float:
    x, y, w, h = bbox
    x0, y0 = max(0, x - PAD), max(0, y - PAD)
    x1, y1 = min(a.shape[1], x + w + PAD), min(a.shape[0], y + h + PAD)
    if x1 <= x0 or y1 <= y0:
        return 0.0
    return float(np.abs(a[y0:y1, x0:x1] - b[y0:y1, x0:x1]).mean())


# ---- backbone path indexing -------------------------------------------

_NUM = re.compile(r"-?\d+\.?\d*")


def _layer_groups(base_svg: str):
    """Yield (group_start, group_end, translate(x,y), [path spans])."""
    out = []
    for m in re.finditer(
        r'<g fill="[^"]*"(?: transform="translate\(([-\d.]+),([-\d.]+)\)")?>',
        base_svg,
    ):
        start = m.start()
        end = base_svg.find("</g>", start)
        tx = float(m.group(1)) if m.group(1) else 0.0
        ty = float(m.group(2)) if m.group(2) else 0.0
        sub = base_svg[start:end]
        # spans are RELATIVE to the group substring: searching the whole
        # document with a relative offset cuts the wrong text for any group
        # that is not at the document top
        paths = [(pm.start(), sub.find("/>", pm.start()) + 2)
                 for pm in re.finditer(r"<path ", sub)]
        out.append((start, end, tx, ty, paths))
    return out


def _path_bbox(d: str):
    nums = [float(v) for v in _NUM.findall(d)]
    xs, ys = nums[0::2], nums[1::2]
    if not xs:
        return None
    return min(xs), min(ys), max(xs), max(ys)


def _text_element(t) -> str:
    from xml.sax.saxutils import escape

    x = t.bbox[0]
    y = int(t.bbox[1] + t.bbox[3] * 0.78)
    return (
        '<text id="%s" data-ocr="1" x="%s" y="%s" font-size="%s" '
        'font-family="%s" fill="%s">%s</text>'
        % (t.id, x, y, t.font_size, t.font_family or "sans-serif",
           t.fill or "#000000", escape(t.text or ""))
    )


def compile_svg(base_svg: str, texts: list, w: int, h: int,
                source_path: str) -> tuple[str, dict]:
    """Return (final_svg, stats)."""
    src = np.asarray(Image.open(source_path).convert("RGB")).astype(np.int32)
    stats = {"texts_total": len(texts), "texts_editable": 0,
             "texts_traced": 0}

    if not texts:
        return base_svg, stats

    groups = _layer_groups(base_svg)
    # removal spans (absolute offsets in base_svg) per text
    removals = {}
    for t in texts:
        spans = []
        bx0, by0 = t.bbox[0] - PAD, t.bbox[1] - PAD
        bx1, by1 = t.bbox[0] + t.bbox[2] + PAD, t.bbox[1] + t.bbox[3] + PAD
        for (gs, ge, tx, ty, paths) in groups:
            for (ps, pe) in paths:
                d = re.search(r' d="([^"]+)"',
                              base_svg[gs + ps:gs + pe] if ps < ge - gs else "")
                if not d:
                    continue
                bb = _path_bbox(d.group(1))
                if bb is None:
                    continue
                x0, y0 = bb[0] + tx, bb[1] + ty
                x1, y1 = bb[2] + tx, bb[3] + ty
                # glyph path inside the text box? (clamp both axes: a
                # disjoint box on both axes would otherwise give a
                # positive product of two negatives)
                ix = max(0.0, min(x1, bx1) - max(x0, bx0))
                iy = max(0.0, min(y1, by1) - max(y0, by0))
                ov = ix * iy
                area = max(1e-6, (x1 - x0) * (y1 - y0))
                if ov / area >= 0.6 and ov >= 8:
                    spans.append((gs + ps, gs + pe))
        removals[t.id] = spans

    base_render = _render(base_svg, w, h)
    # trial: all removable sets removed + all texts inserted, one render
    trial = _apply(base_svg, texts, removals, accept_all=True)
    trial_render = _render(trial, w, h)

    accepted = []
    for t in texts:
        db = _box_diff(trial_render, src, t.bbox)
        if db <= TEXT_ACCEPT_DIFF and removals[t.id]:
            accepted.append(t)
            stats["texts_editable"] += 1
        else:
            stats["texts_traced"] += 1
    final = _apply(base_svg, accepted, removals, accept_all=False)
    stats["final_mean_diff"] = round(
        float(np.abs(_render(final, w, h).astype(np.int32) - src).mean()), 3)
    return final, stats


def _apply(base_svg: str, texts: list, removals: dict, accept_all: bool):
    """Remove glyph spans of `texts`, insert their <text> before </svg>."""
    spans = []
    for t in texts:
        spans.extend(removals.get(t.id, []))
    if not spans:
        insert0 = "".join(_text_element(t) + "\n" for t in texts)
        return base_svg.replace("</svg>", "<g id=\"texts\">\n" + insert0 + "</g>\n</svg>")
    # merge overlapping intervals: the same path can match several adjacent
    # OCR boxes; double removal with stale offsets corrupts the document
    spans = sorted(set(spans))
    merged = [list(spans[0])]
    for s, e in spans[1:]:
        if s < merged[-1][1]:
            merged[-1][1] = max(merged[-1][1], e)
        else:
            merged.append([s, e])
    out = base_svg
    for s, e in reversed(merged):
        out = out[:s] + out[e:]
    insert = "".join(_text_element(t) + "\n" for t in texts)
    return out.replace("</svg>", "<g id=\"texts\">\n" + insert + "</g>\n</svg>")
