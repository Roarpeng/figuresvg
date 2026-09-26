"""VLM-enhanced pipeline: CV facts → VLM understanding → SVG generation.

Replaces the pure bottom-up tracing with:
  1. CV layer extracts FACTS (colors, regions, text) — same as before
  2. VLM (Qwen) INTERPRETS the image + facts into a scene understanding
  3. SVG Generator DRAWS from the understanding (not from pixel tracing)
  4. Verification compares the result against the original bitmap
"""

import io
import json
import os
import sys

import numpy as np
import cairosvg
from PIL import Image

from .vlm_engine import understand
from .svg_from_understanding import generate as svg_generate
from .refine import refine


def _extract_facts(image_path: str) -> dict:
    """Lightweight CV fact extraction for the VLM (not the full pipeline)."""
    img = np.asarray(Image.open(image_path).convert("RGB"))
    H, W = img.shape[:2]

    # dominant colors
    from collections import Counter
    small = img[::4, ::4]
    colors = Counter()
    for px in small.reshape(-1, 3):
        c = tuple(int(v) for v in px)
        colors[c] += 1
    top_colors = [
        {"hex": f"#{c[0]:02x}{c[1]:02x}{c[2]:02x}", "count": n}
        for c, n in colors.most_common(8) if n > 10
    ]

    # OCR
    ocr_texts = []
    try:
        sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
        from ..recognizers import ocr
        items = ocr.recognize(image_path)
        ocr_texts = [
            {"text": t.text, "bbox": t.bbox, "font_size": t.font_size}
            for t in items
        ]
    except Exception:
        pass

    # content coverage regions (rough)
    mx, mn = img.max(axis=2), img.min(axis=2)
    spread = mx - mn
    content = ((spread > 15) | (mx < 250))
    content_frac = float(content.mean())

    return {
        "image_size": [W, H],
        "dominant_colors": top_colors,
        "text_elements": ocr_texts,
        "content_coverage": round(content_frac, 3),
        "content_bbox": _content_bbox(content),
    }


def _content_bbox(mask):
    ys, xs = np.where(mask)
    if len(ys) == 0:
        return None
    return [int(xs.min()), int(ys.min()),
            int(xs.max() - xs.min()), int(ys.max() - ys.min())]


def convert(image_path: str, svg_out: str) -> dict:
    """VLM pipeline: image → understanding → SVG."""
    # 1. CV facts
    facts = _extract_facts(image_path)
    print(f"  facts: {len(facts['dominant_colors'])} colors, "
          f"{len(facts['text_elements'])} texts")

    # 2. VLM understanding
    try:
        result = understand(image_path, facts)
        print(f"  VLM: {result.get('interpretation', '?')}")
        print(f"  type: {result.get('type', '?')}, "
              f"{len(result.get('components', []))} components, "
              f"{len(result.get('arrangements', []))} arrangements")
    except Exception as e:
        print(f"  VLM error: {e}")
        return {"error": str(e)}

    # 3. CV refinement: replace VLM's approximate coords/colors with
    # exact pixel measurements
    result = refine(result, image_path)
    print(f"  refined: {len(result.get('components', []))} components snapped to CV")

    # 4. Generate SVG from understanding
    W, H = facts["image_size"]
    svg = svg_generate(result, W, H)
    with open(svg_out, "w") as f:
        f.write(svg)
    print(f"  wrote {svg_out} ({len(svg)} bytes)")

    # 4. Verify
    stats = _verify(svg, image_path, W, H)
    return {"understanding": result.get("interpretation"),
            "type": result.get("type"),
            "components": len(result.get("components", [])),
            **stats}


def _verify(svg: str, source_path: str, W: int, H: int) -> dict:
    try:
        png = cairosvg.svg2png(bytestring=svg.encode(),
                               output_width=W, output_height=H)
        ren = np.asarray(Image.open(io.BytesIO(png)).convert("RGB")).astype(int)
        src = np.asarray(Image.open(source_path).convert("RGB")).astype(int)
        diff = float(np.abs(ren - src).mean())
        coverage = float(
            ((src.min(axis=2) < 235) & (ren.min(axis=2) < 235)).sum() /
            max(1, (src.min(axis=2) < 235).sum()))
        return {"pixel_diff": round(diff, 2), "coverage": round(coverage, 4)}
    except Exception as e:
        return {"verify_error": str(e)[:80]}
