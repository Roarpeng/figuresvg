"""Closed-loop text correction (TypeSafe-decided strategy C + V4):

Render the current SVG, re-OCR the RENDER, and measure exactly where each
text actually landed. The position/size delta between the intended box
(source OCR) and the re-recognized box (render OCR) is applied back as a
correction. Iterated 2x; converges to sub-pixel placement without any new
recognition technology -- the verification signal IS the correction.
"""

import io

import cairosvg
import numpy as np
from PIL import Image

from . import svggen


def _render(svg, w, h):
    png = cairosvg.svg2png(bytestring=svg.encode(), output_width=w,
                           output_height=h)
    return Image.open(io.BytesIO(png)).convert("RGB")


def _iou(a, b):
    ax, ay, aw, ah = a
    bx, by, bw, bh = b
    ix = max(0, min(ax + aw, bx + bw) - max(ax, bx))
    iy = max(0, min(ay + ah, by + bh) - max(ay, by))
    inter = ix * iy
    union = aw * ah + bw * bh - inter
    return inter / union if union > 0 else 0.0


def correct(scene, render_path, iterations=2):
    """render_path: temp PNG path for re-OCR rounds. Mutates text elements
    (dx/dy offsets, font_size scale). Returns loop stats."""
    from .recognizers import ocr

    texts = [e for e in scene.elements if e.type == "text"]
    if not texts:
        return {"loop_rounds": 0}
    stats = {"loop_rounds": iterations, "loop_shift_px": [], "loop_scale": []}
    w, h = scene.width, scene.height

    for rnd in range(iterations):
        svg = svggen.generate(scene)
        img = _render(svg, w, h)
        img.save(render_path)
        retext = ocr.recognize(render_path)
        used = set()
        moved = 0
        matched_this_round = 0
        for t in texts:
            # STRICT matching: same string only. A mismatched pairing
            # corrupts positions (divergence observed with IoU fallback).
            best, bi = 0.0, None
            for i, r in enumerate(retext):
                if i in used or r.text != t.text:
                    continue
                v = _iou(t.bbox, r.bbox)
                if v > best:
                    best, bi = v, i
            if bi is None:
                continue
            used.add(bi)
            matched_this_round += 1
            r = retext[bi]
            # intended placement (source box) vs actual (render box);
            # clamp per-round corrections -- large deltas mean a bad
            # pairing or a scaling bug, not a nudge
            dx = float(np.clip((t.bbox[0] - r.bbox[0]) + (t.dx or 0), -40, 40))
            dy = float(np.clip((t.bbox[1] - r.bbox[1]) + (t.dy or 0), -40, 40))
            scale = float(np.clip(t.bbox[3] / max(4, r.bbox[3]), 0.9, 1.1))
            t.dx = dx
            t.dy = dy
            t.font_size = round((t.font_size or 10) * (0.5 + 0.5 * scale), 1)
            moved += 1
            stats["loop_shift_px"].append(max(abs(dx), abs(dy)))
            stats["loop_scale"].append(scale)
        stats[f"round{rnd}_match"] = round(matched_this_round / len(texts), 3)
        if moved == 0:
            break
    sp = stats["loop_shift_px"]
    stats["loop_mean_shift"] = round(float(np.mean(sp)), 2) if sp else 0.0
    stats["loop_max_shift"] = round(float(np.max(sp)), 2) if sp else 0.0
    stats["loop_shift_px"] = len(sp)  # count only, keep stats light
    stats["loop_scale"] = round(float(np.mean(stats["loop_scale"])), 3) if \
        stats["loop_scale"] else 1.0
    return stats
