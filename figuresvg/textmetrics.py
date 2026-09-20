"""Text metric optimization (bitmap-driven, per user's step 4):

Every text stays a REAL editable <text> element. The bitmap comparison
does NOT fall back to outlines -- it drives a per-text search over
font-size / baseline / weight candidates, compositing each candidate over
the geometry-only render inside the text's box, and keeps whichever
matches the source bitmap best. Targeted modification, not reversion.
"""

import io
import re

import cairosvg
import numpy as np
from PIL import Image

PAD = 8
FS_SCALES = (0.75, 0.85, 1.0, 1.15)
DY_CHOICES = (-3, -1, 0, 2)
WEIGHTS = ("normal", "bold")


def _render_text(text, fs, dy, weight, color, bw, bh, family="sans-serif") -> Image.Image:
    """Transparent-background render of one <text> candidate."""
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{bw}" height="{bh}">'
        f'<text x="{PAD}" y="{int(bh * 0.78) + dy}" font-size="{fs:.1f}" '
        f'font-family="{family}" font-weight="{weight}" fill="{color}">'
        f'{_esc(text)}</text></svg>'
    )
    png = cairosvg.svg2png(bytestring=svg.encode(), output_width=bw,
                           output_height=bh)
    return Image.open(io.BytesIO(png)).convert("RGBA")


def _esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def optimize(scene, source_path: str) -> dict:
    """Optimize font metrics of every text element against the bitmap."""
    from . import svggen

    src = np.asarray(Image.open(source_path).convert("RGB")).astype(np.int32)
    h, w = src.shape[:2]

    texts = [e for e in scene.elements if e.type == "text"]
    if not texts:
        return {"texts": 0}

    # geometry-only render (background under the text)
    scene_no_text = _without_texts(scene)
    bg_png = cairosvg.svg2png(bytestring=svggen.generate(scene_no_text).encode(),
                              output_width=w, output_height=h)
    bg = Image.open(io.BytesIO(bg_png)).convert("RGBA")

    improved = 0
    for t in texts:
        x, y, bw, bh = t.bbox
        x0, y0 = max(0, x - PAD), max(0, y - PAD)
        x1, y1 = min(w, x + bw + PAD), min(h, y + bh + PAD)
        if x1 <= x0 or y1 <= y0:
            continue
        target = src[y0:y1, x0:x1]
        bg_crop = bg.crop((x0, y0, x1, y1))
        base_fs = t.font_size or (bh / 0.72)
        best, best_diff = None, 1e9
        for fss in FS_SCALES:
            for dy in DY_CHOICES:
                for wt in WEIGHTS:
                    fs = base_fs * fss
                    cand = _render_text(t.text or "", fs, dy, wt,
                                        t.fill or "#000000",
                                        x1 - x0, y1 - y0,
                                        t.font_family or "sans-serif")
                    comp = bg_crop.copy()
                    comp.alpha_composite(cand)
                    diff = float(np.abs(
                        np.asarray(comp.convert("RGB")).astype(np.int32)
                        - target).mean())
                    if diff < best_diff:
                        best_diff = diff
                        best = (fs, dy, wt)
        if best:
            t.font_size = round(best[0], 1)
            t.baseline_dy = best[1]
            t.font_weight = best[2]
            t.box_diff = round(best_diff, 2)
            improved += 1
    return {"texts": len(texts), "metric_optimized": improved}


def _without_texts(scene):
    import copy

    sc = copy.deepcopy(scene)
    sc.elements = [e for e in sc.elements if e.type != "text"]
    return sc
